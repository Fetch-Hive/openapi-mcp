use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use mcp_gateway_tunnel_proto::{
    is_hop_by_hop_header, is_stripped_request_header, Frame, Reclaim, Welcome, CLOSE_NORMAL,
    CONTENT_TYPE_EVENT_STREAM, MAX_FRAME_BYTES, MCP_PATH,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    tungstenite::{
        protocol::{frame::coding::CloseCode, CloseFrame},
        Message,
    },
    MaybeTlsStream, WebSocketStream,
};
use tokio_util::sync::CancellationToken;

use crate::stats::Stats;
use crate::{FinishedRequest, LocalService, TunnelConfig};

/// Largest raw slice whose standard base64 form fits in one frame.
const RAW_CHUNK: usize = (MAX_FRAME_BYTES / 4) * 3;

pub(crate) struct SessionStop {
    pub after: Option<std::time::Duration>,
    pub reclaim: Option<Reclaim>,
    pub fresh: bool,
    pub terminal: Option<(String, String)>,
    pub shutdown: bool,
}

struct ReqParts {
    id: u64,
    method: String,
    path: String,
    query: Option<String>,
    headers: Vec<(String, String)>,
    body_complete: bool,
    body: Option<String>,
}

pub(crate) async fn run<S: LocalService>(
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    welcome: Welcome,
    cfg: TunnelConfig,
    service: S,
    shutdown: CancellationToken,
    stats: Arc<Stats>,
    requests: mpsc::UnboundedSender<FinishedRequest>,
) -> SessionStop {
    let reclaim = Some(Reclaim {
        slug: welcome.slug.clone(),
        credential: welcome.credential.clone(),
    });
    let limits = welcome.limits;
    let (mut write, mut read) = socket.split();
    let (out_tx, mut out_rx) = mpsc::channel::<Out>(32);
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<u64>();
    let mut bodies: HashMap<u64, mpsc::UnboundedSender<Bytes>> = HashMap::new();
    let mut cancels: HashMap<u64, CancellationToken> = HashMap::new();
    let mut go_away: Option<u32> = None;
    let mut terminal_reason: Option<String> = None;

    loop {
        if let Some(after) = go_away {
            if stats.inflight.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                if let Some(reason) = terminal_reason.take() {
                    return stop_terminal(reason);
                }
                return stop_reconnect(
                    reclaim,
                    Some(std::time::Duration::from_secs(u64::from(after))),
                );
            }
        }
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                let _ = write.send(Message::Close(Some(CloseFrame {
                    code: CloseCode::from(CLOSE_NORMAL),
                    reason: "".into(),
                }))).await;
                return SessionStop { after: None, reclaim, fresh: false, terminal: None, shutdown: true };
            }
            msg = out_rx.recv() => {
                let Some(msg) = msg else { continue };
                if write.send(msg.into_message()).await.is_err() {
                    return stop_reconnect(reclaim, None);
                }
            }
            incoming = read.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(frame) = serde_json::from_str::<Frame>(&text) else { continue };
                        match frame {
                            Frame::RequestStart { id, method, path, query, headers, body_complete, body, .. } => {
                                if go_away.is_some() {
                                    continue;
                                }
                                let parts = ReqParts { id, method, path, query, headers, body_complete, body };
                                accept_request(parts, &cfg, &service, &stats, limits.max_inflight, &out_tx, &done_tx, &requests, &mut bodies, &mut cancels);
                            }
                            Frame::RequestBody { id, chunk, last } => {
                                if let Some(tx) = bodies.get(&id) {
                                    if let Ok(bytes) = STANDARD.decode(chunk) {
                                        let _ = tx.send(Bytes::from(bytes));
                                    } else if let Some(cancel) = cancels.get(&id) {
                                        cancel.cancel();
                                    }
                                    if last {
                                        bodies.remove(&id);
                                    }
                                }
                            }
                            Frame::Cancel { id, .. } => {
                                if let Some(cancel) = cancels.remove(&id) {
                                    cancel.cancel();
                                }
                                bodies.remove(&id);
                            }
                            Frame::Ping { nonce } => {
                                let _ = out_tx.try_send(Out::Text(frame_json(&Frame::Pong { nonce })));
                            }
                            Frame::GoAway { reason, reconnect_after_secs } => {
                                if terminal_go_away(&reason) {
                                    terminal_reason = Some(reason);
                                    go_away = Some(0);
                                } else {
                                    go_away = Some(reconnect_after_secs);
                                }
                            }
                            Frame::Pong { .. } | Frame::ResponseStart { .. } | Frame::ResponseBody { .. } | Frame::Stats { .. } => {}
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = out_tx.try_send(Out::Pong(payload.to_vec()));
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        if let Some(reason) = terminal_reason.take() {
                            return stop_terminal(reason);
                        }
                        return stop_reconnect(reclaim, None);
                    }
                    Some(Ok(_)) => {}
                }
            }
            Some(id) = done_rx.recv() => {
                cancels.remove(&id);
                bodies.remove(&id);
            }
        }
    }
}

/// Named replace and delete must not dial again. A reclaim of the stale
/// credential falls through to authorize and would take the name back.
fn terminal_go_away(reason: &str) -> bool {
    matches!(
        reason,
        "replaced by a newer connection" | "endpoint deleted"
    )
}

fn stop_terminal(reason: String) -> SessionStop {
    let (code, message) = match reason.as_str() {
        "endpoint deleted" => (
            "deleted",
            "endpoint was deleted; run tunnels list".to_owned(),
        ),
        _ => ("replaced", reason),
    };
    SessionStop {
        after: None,
        reclaim: None,
        fresh: false,
        terminal: Some((code.to_owned(), message)),
        shutdown: false,
    }
}

fn stop_reconnect(reclaim: Option<Reclaim>, after: Option<std::time::Duration>) -> SessionStop {
    SessionStop {
        after,
        reclaim,
        fresh: false,
        terminal: None,
        shutdown: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn accept_request<S: LocalService>(
    parts: ReqParts,
    cfg: &TunnelConfig,
    service: &S,
    stats: &Arc<Stats>,
    max_inflight: u32,
    out_tx: &mpsc::Sender<Out>,
    done_tx: &mpsc::UnboundedSender<u64>,
    requests: &mpsc::UnboundedSender<FinishedRequest>,
    bodies: &mut HashMap<u64, mpsc::UnboundedSender<Bytes>>,
    cancels: &mut HashMap<u64, CancellationToken>,
) {
    if !stats.begin_request(max_inflight) {
        let _ = requests.send(finished(&parts, 429, Duration::ZERO));
        let _ = out_tx.try_send(Out::Text(frame_json(&too_busy(parts.id))));
        return;
    }
    let cancel = CancellationToken::new();
    cancels.insert(parts.id, cancel.clone());
    let (body, body_tx) = request_body(&parts);
    if let Some(tx) = body_tx {
        bodies.insert(parts.id, tx);
    }
    let request = match build_request(cfg, &parts, body) {
        Ok(request) => request,
        Err(()) => {
            stats.end_request();
            cancels.remove(&parts.id);
            let _ = requests.send(finished(&parts, 400, Duration::ZERO));
            let _ = out_tx.try_send(Out::Text(frame_json(&error_response(
                parts.id,
                400,
                "bad request",
            ))));
            return;
        }
    };
    let service = service.clone();
    let out_tx = out_tx.clone();
    let done_tx = done_tx.clone();
    let stats = stats.clone();
    let requests = requests.clone();
    let method = parts.method.clone();
    let path = parts.path.clone();
    let rpc = rpc_label(&parts);
    let started = Instant::now();
    let id = parts.id;
    tokio::spawn(async move {
        let guard = InflightGuard {
            stats,
            done: done_tx,
            id,
        };
        let (status, duration) = tokio::select! {
            _ = cancel.cancelled() => (0u16, started.elapsed()),
            response = service.call(request) => {
                let status = response.status().as_u16();
                if !cancel.is_cancelled() {
                    let _ = write_response(id, response, &out_tx, &cancel).await;
                }
                (status, started.elapsed())
            }
        };
        drop(guard);
        let _ = requests.send(FinishedRequest {
            method,
            path,
            rpc,
            status,
            duration,
        });
    });
}

fn finished(parts: &ReqParts, status: u16, duration: Duration) -> FinishedRequest {
    FinishedRequest {
        method: parts.method.clone(),
        path: parts.path.clone(),
        rpc: rpc_label(parts),
        status,
        duration,
    }
}

/// Decoded JSON-RPC body kept for the request-line label. Larger bodies,
/// and any body that arrived in more than one frame, are not parsed.
const RPC_BODY_CAP: usize = 64 * 1024;
/// Each of `method` and `params.name` is cut here. The cut form ends with `…`.
const RPC_PART_CAP: usize = 80;

/// Label for the status line. `None` when the body was streamed, bigger
/// than [`RPC_BODY_CAP`] decoded bytes, or not a JSON-RPC call whose
/// `method` passes [`is_safe_label`].
fn rpc_label(parts: &ReqParts) -> Option<String> {
    if !parts.body_complete {
        return None;
    }
    let raw = parts.body.as_deref()?;
    if raw.len() > rpc_b64_cap(RPC_BODY_CAP) {
        return None;
    }
    let bytes = STANDARD.decode(raw).ok()?;
    if bytes.len() > RPC_BODY_CAP {
        return None;
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    label_from_value(&value)
}

fn rpc_b64_cap(decoded_cap: usize) -> usize {
    decoded_cap.div_ceil(3) * 4
}

fn label_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Array(items) => {
            let first = label_one(items.first()?)?;
            let extra = items.len() - 1;
            if extra == 0 {
                Some(first)
            } else {
                Some(format!("{first} +{extra}"))
            }
        }
        other => label_one(other),
    }
}

fn label_one(value: &serde_json::Value) -> Option<String> {
    let method = value.get("method").and_then(|item| item.as_str())?;
    if !is_safe_label(method) {
        return None;
    }
    let method = bound_part(method);
    if method == "tools/call" {
        let name = value
            .pointer("/params/name")
            .and_then(|item| item.as_str())
            .filter(|name| is_safe_label(name))
            .map(bound_part);
        return Some(match name {
            Some(name) => format!("tools/call {name}"),
            None => "tools/call".to_owned(),
        });
    }
    Some(method)
}

/// Printable text. Control characters, tabs, newlines, and bidi or
/// zero-width characters are refused. A normal space is allowed.
fn is_safe_label(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|c| {
            if c.is_control() {
                return false;
            }
            if c.is_whitespace() {
                return c == ' ';
            }
            !matches!(
                c,
                '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
                    | '\u{feff}'
            )
        })
}

fn bound_part(text: &str) -> String {
    if text.chars().count() <= RPC_PART_CAP {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(RPC_PART_CAP).collect();
    cut.push('…');
    cut
}

struct InflightGuard {
    stats: Arc<Stats>,
    done: mpsc::UnboundedSender<u64>,
    id: u64,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.stats.end_request();
        let _ = self.done.send(self.id);
    }
}

fn request_body(parts: &ReqParts) -> (Body, Option<mpsc::UnboundedSender<Bytes>>) {
    if parts.body_complete {
        let bytes = parts
            .body
            .as_deref()
            .and_then(|raw| STANDARD.decode(raw).ok())
            .unwrap_or_default();
        return (Body::from(bytes), None);
    }
    let (tx, rx) = mpsc::unbounded_channel();
    if let Some(raw) = parts.body.as_deref() {
        if let Ok(bytes) = STANDARD.decode(raw) {
            let _ = tx.send(Bytes::from(bytes));
        }
    }
    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv()
            .await
            .map(|chunk| (Ok::<Bytes, std::io::Error>(chunk), rx))
    });
    (Body::from_stream(stream), Some(tx))
}

fn build_request(
    cfg: &TunnelConfig,
    parts: &ReqParts,
    body: Body,
) -> Result<http::Request<Body>, ()> {
    let path = if parts.path == MCP_PATH {
        cfg.mcp_path.clone()
    } else {
        parts.path.clone()
    };
    let uri = match parts.query.as_deref() {
        Some(query) if !query.is_empty() => format!("{path}?{query}"),
        _ => path,
    };
    let method = http::Method::from_bytes(parts.method.as_bytes()).map_err(|_| ())?;
    let mut builder = http::Request::builder().method(method).uri(uri);
    for (name, value) in &parts.headers {
        if name.eq_ignore_ascii_case("host") || name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        if is_stripped_request_header(name) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder = builder.header("host", &cfg.local_authority);
    builder.body(body).map_err(|_| ())
}

async fn write_response(
    id: u64,
    response: http::Response<Body>,
    out_tx: &mpsc::Sender<Out>,
    cancel: &CancellationToken,
) -> Result<(), ()> {
    let (parts, mut body) = response.into_parts();
    let sse = parts
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with(CONTENT_TYPE_EVENT_STREAM));
    if sse {
        send_frame(
            out_tx,
            &response_start(id, parts.status.as_u16(), &parts, false, None),
        )
        .await?;
        stream_body(id, &mut body, out_tx, cancel).await?;
        return Ok(());
    }
    let mut buf = Vec::new();
    while let Some(frame) = body.frame().await {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let frame = frame.map_err(|_| ())?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        if buf.len() + data.len() > RAW_CHUNK {
            send_frame(
                out_tx,
                &response_start(id, parts.status.as_u16(), &parts, false, None),
            )
            .await?;
            send_chunks(id, &buf, false, out_tx).await?;
            send_chunks(id, &data, false, out_tx).await?;
            stream_body(id, &mut body, out_tx, cancel).await?;
            return Ok(());
        }
        buf.extend_from_slice(&data);
    }
    let encoded = if buf.is_empty() {
        None
    } else {
        Some(STANDARD.encode(&buf))
    };
    send_frame(
        out_tx,
        &response_start(id, parts.status.as_u16(), &parts, true, encoded),
    )
    .await
}

async fn stream_body(
    id: u64,
    body: &mut Body,
    out_tx: &mpsc::Sender<Out>,
    cancel: &CancellationToken,
) -> Result<(), ()> {
    let mut pending: Vec<u8> = Vec::new();
    while let Some(frame) = body.frame().await {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let frame = frame.map_err(|_| ())?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        pending.extend_from_slice(&data);
        while pending.len() >= RAW_CHUNK {
            let rest = pending.split_off(RAW_CHUNK);
            let full = std::mem::replace(&mut pending, rest);
            send_frame(
                out_tx,
                &Frame::ResponseBody {
                    id,
                    chunk: STANDARD.encode(&full),
                    last: false,
                },
            )
            .await?;
        }
    }
    send_frame(
        out_tx,
        &Frame::ResponseBody {
            id,
            chunk: STANDARD.encode(&pending),
            last: true,
        },
    )
    .await
}

async fn send_chunks(
    id: u64,
    bytes: &[u8],
    last: bool,
    out_tx: &mpsc::Sender<Out>,
) -> Result<(), ()> {
    if bytes.is_empty() {
        if last {
            return send_frame(
                out_tx,
                &Frame::ResponseBody {
                    id,
                    chunk: String::new(),
                    last: true,
                },
            )
            .await;
        }
        return Ok(());
    }
    let mut offset = 0;
    while offset < bytes.len() {
        let end = (offset + RAW_CHUNK).min(bytes.len());
        let is_last = last && end == bytes.len();
        send_frame(
            out_tx,
            &Frame::ResponseBody {
                id,
                chunk: STANDARD.encode(&bytes[offset..end]),
                last: is_last,
            },
        )
        .await?;
        offset = end;
    }
    Ok(())
}

fn response_start(
    id: u64,
    status: u16,
    parts: &http::response::Parts,
    body_complete: bool,
    body: Option<String>,
) -> Frame {
    Frame::ResponseStart {
        id,
        status,
        headers: response_headers(parts, body_complete),
        body_complete,
        body,
    }
}

fn response_headers(parts: &http::response::Parts, body_complete: bool) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    for (name, value) in &parts.headers {
        if is_hop_by_hop_header(name.as_str()) {
            continue;
        }
        if !body_complete && name.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        if let Ok(text) = value.to_str() {
            headers.push((name.to_string(), text.to_string()));
        }
    }
    headers
}

fn too_busy(id: u64) -> Frame {
    error_response(id, 429, "too many in-flight requests")
}

fn error_response(id: u64, status: u16, message: &str) -> Frame {
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":-32000,\"message\":{}}},\"id\":null}}",
        serde_json::to_string(message).unwrap_or_else(|_| "\"error\"".into())
    );
    Frame::ResponseStart {
        id,
        status,
        headers: vec![
            ("content-type".into(), "application/json".into()),
            ("retry-after".into(), "1".into()),
        ],
        body_complete: true,
        body: Some(STANDARD.encode(body.as_bytes())),
    }
}

fn frame_json(frame: &Frame) -> String {
    serde_json::to_string(frame).unwrap_or_else(|_| "{}".into())
}

async fn send_frame(out_tx: &mpsc::Sender<Out>, frame: &Frame) -> Result<(), ()> {
    out_tx
        .send(Out::Text(frame_json(frame)))
        .await
        .map_err(|_| ())
}

enum Out {
    Text(String),
    Pong(Vec<u8>),
}

impl Out {
    fn into_message(self) -> Message {
        match self {
            Self::Text(text) => Message::Text(text.into()),
            Self::Pong(payload) => Message::Pong(payload.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(body: Option<&str>, complete: bool) -> ReqParts {
        ReqParts {
            id: 1,
            method: "POST".into(),
            path: "/mcp".into(),
            query: None,
            headers: Vec::new(),
            body_complete: complete,
            body: body.map(|text| STANDARD.encode(text)),
        }
    }

    #[test]
    fn tools_call_label_includes_the_tool_name() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_issues","arguments":{}}}"#;
        assert_eq!(
            rpc_label(&parts(Some(body), true)).as_deref(),
            Some("tools/call list_issues")
        );
    }

    #[test]
    fn other_methods_keep_the_method_alone() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        assert_eq!(
            rpc_label(&parts(Some(body), true)).as_deref(),
            Some("tools/list")
        );
    }

    #[test]
    fn batch_shows_the_first_call_and_the_rest_count() {
        let body = r#"[{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_issues"}},{"jsonrpc":"2.0","id":2,"method":"tools/list"},{"jsonrpc":"2.0","id":3,"method":"ping"}]"#;
        assert_eq!(
            rpc_label(&parts(Some(body), true)).as_deref(),
            Some("tools/call list_issues +2")
        );
    }

    #[test]
    fn streamed_or_unsafe_or_huge_bodies_have_no_label() {
        let call =
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_issues"}}"#;
        assert_eq!(rpc_label(&parts(Some(call), false)), None);
        assert_eq!(
            rpc_label(&parts(
                Some(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call\n"}"#),
                true
            )),
            None
        );
        let named =
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bad\nname"}}"#;
        assert_eq!(
            rpc_label(&parts(Some(named), true)).as_deref(),
            Some("tools/call")
        );
        let mut huge = String::from(r#"{"jsonrpc":"2.0","id":1,"method":"ping","pad":""#);
        huge.push_str(&"x".repeat(RPC_BODY_CAP));
        huge.push_str(r#""}"#);
        assert_eq!(rpc_label(&parts(Some(&huge), true)), None);
        assert_eq!(rpc_label(&parts(Some("not json"), true)), None);
    }

    #[test]
    fn long_method_is_cut_at_eighty_characters() {
        let method = "a".repeat(90);
        let body = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}"}}"#);
        let label = rpc_label(&parts(Some(&body), true)).unwrap();
        assert_eq!(label.chars().count(), RPC_PART_CAP + 1);
        assert!(label.ends_with('…'));
    }
}
