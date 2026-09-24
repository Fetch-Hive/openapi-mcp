use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::routing::{get, post};
use axum::Router;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bytes::Bytes;
use mcp_gateway_tunnel::{client_identity, run, LocalService, TunnelConfig, TunnelState};
use mcp_gateway_tunnel_proto::{
    CancelReason, EndpointKind, Frame, Limits, Welcome, CONTENT_TYPE_EVENT_STREAM, MCP_PATH,
};
use tokio::sync::{oneshot, Notify};

fn cfg(addr: std::net::SocketAddr) -> TunnelConfig {
    TunnelConfig {
        relay_url: format!("ws://{addr}/v1/tunnel"),
        local_authority: "127.0.0.1:9".into(),
        mcp_path: MCP_PATH.into(),
        auth_mode: mcp_gateway_tunnel_proto::EndpointAuthMode::Token,
        client: client_identity(),
    }
}

fn welcome() -> Welcome {
    Welcome::new(
        "abcd2345",
        "https://abcd2345.mcp.fetchhive.com/mcp",
        "a".repeat(43),
        1800,
        Some(8 * 60 * 60),
        Limits::anonymous_defaults(),
        EndpointKind::Anonymous,
    )
}

async fn listen<F, Fut>(on_upgrade: F) -> std::net::SocketAddr
where
    F: Fn(WebSocket) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let app = Router::new().route(
        "/v1/tunnel",
        get(move |ws: WebSocketUpgrade| {
            let on_upgrade = on_upgrade.clone();
            async move { ws.on_upgrade(on_upgrade) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn recv_text(ws: &mut WebSocket) -> String {
    loop {
        let msg = ws.recv().await.expect("socket").expect("frame");
        match msg {
            Message::Text(text) => return text.to_string(),
            Message::Ping(payload) => {
                ws.send(Message::Pong(payload)).await.unwrap();
            }
            Message::Close(_) => panic!("closed"),
            _ => {}
        }
    }
}

async fn send_json(ws: &mut WebSocket, value: &impl serde::Serialize) {
    let text = serde_json::to_string(value).unwrap();
    ws.send(Message::Text(text.into())).await.unwrap();
}

async fn wait_state(state: &mut tokio::sync::watch::Receiver<TunnelState>) -> TunnelState {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let snapshot = state.borrow().clone();
        if !matches!(snapshot, TunnelState::Connecting) {
            return snapshot;
        }
        if tokio::time::timeout_at(deadline, state.changed())
            .await
            .is_err()
        {
            return snapshot;
        }
    }
}

type Seen = Arc<std::sync::Mutex<Option<(String, Vec<u8>)>>>;

fn echo_app(seen: Seen) -> Router {
    Router::new()
        .route(
            "/mcp",
            post(
                |State(seen): State<Seen>, headers: axum::http::HeaderMap, body: Bytes| async move {
                    let host = headers
                        .get("host")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_owned();
                    *seen.lock().unwrap() = Some((host, body.to_vec()));
                    body
                },
            ),
        )
        .with_state(seen)
}

fn request(id: u64, body: Option<String>, complete: bool) -> Frame {
    Frame::RequestStart {
        id,
        method: "POST".into(),
        path: MCP_PATH.into(),
        query: None,
        headers: vec![("content-type".into(), "application/json".into())],
        body_complete: complete,
        body,
        client_ip: "203.0.113.9".into(),
        request_id: format!("req-{id}"),
    }
}

#[tokio::test]
async fn inline_body_is_forwarded_with_loopback_host() {
    let seen = Arc::new(std::sync::Mutex::new(None));
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let addr = listen({
        let tx = tx.clone();
        move |mut ws: WebSocket| {
            let tx = tx.clone();
            async move {
                let hello = recv_text(&mut ws).await;
                assert!(hello.contains("\"t\":\"hello\""), "{hello}");
                assert!(hello.contains("\"type\":\"anonymous\""), "{hello}");
                send_json(&mut ws, &welcome()).await;
                let payload = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
                send_json(&mut ws, &request(1, Some(STANDARD.encode(payload)), true)).await;
                let raw = recv_text(&mut ws).await;
                let _ = tx.lock().unwrap().take().unwrap().send(raw);
            }
        }
    })
    .await;
    let mut handle = run(cfg(addr), echo_app(seen.clone()));
    let state = wait_state(&mut handle.state).await;
    assert!(matches!(state, TunnelState::Connected { .. }), "{state:?}");
    let raw = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    let frame: Frame = serde_json::from_str(&raw).unwrap();
    match frame {
        Frame::ResponseStart {
            status,
            body_complete,
            body,
            ..
        } => {
            assert_eq!(status, 200);
            assert!(body_complete);
            let bytes = STANDARD.decode(body.unwrap()).unwrap();
            assert_eq!(bytes, payload_bytes());
        }
        other => panic!("{other:?}"),
    }
    let (host, body) = seen.lock().unwrap().clone().unwrap();
    assert_eq!(host, "127.0.0.1:9");
    assert_eq!(body, payload_bytes());
    let finished = tokio::time::timeout(Duration::from_secs(2), handle.requests.recv())
        .await
        .expect("request log")
        .expect("finished request");
    assert_eq!(finished.method, "POST");
    assert_eq!(finished.status, 200);
    assert_eq!(
        handle.stats.total.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    handle.shutdown.cancel();
}

fn payload_bytes() -> Vec<u8> {
    br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#.to_vec()
}

#[tokio::test]
async fn chunked_request_body_is_assembled() {
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let addr = listen({
        let tx = tx.clone();
        move |mut ws: WebSocket| {
            let tx = tx.clone();
            async move {
                let _ = recv_text(&mut ws).await;
                send_json(&mut ws, &welcome()).await;
                let payload = br#"{"hello":"world"}"#;
                send_json(&mut ws, &request(7, None, false)).await;
                send_json(
                    &mut ws,
                    &Frame::RequestBody {
                        id: 7,
                        chunk: STANDARD.encode(payload),
                        last: true,
                    },
                )
                .await;
                let raw = recv_text(&mut ws).await;
                let _ = tx.lock().unwrap().take().unwrap().send(raw);
            }
        }
    })
    .await;
    let seen = Arc::new(std::sync::Mutex::new(None));
    let handle = run(cfg(addr), echo_app(seen.clone()));
    let raw = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    let frame: Frame = serde_json::from_str(&raw).unwrap();
    let Frame::ResponseStart { body, .. } = frame else {
        panic!("{raw}");
    };
    assert_eq!(
        STANDARD.decode(body.unwrap()).unwrap(),
        br#"{"hello":"world"}"#
    );
    handle.shutdown.cancel();
}

#[derive(Clone)]
struct SseService;

impl LocalService for SseService {
    async fn call(&self, _request: http::Request<Body>) -> http::Response<Body> {
        let stream = futures_util::stream::iter([
            Ok::<Bytes, std::io::Error>(Bytes::from("data: a\n\n")),
            Ok(Bytes::from("data: b\n\n")),
        ]);
        http::Response::builder()
            .header("content-type", CONTENT_TYPE_EVENT_STREAM)
            .body(Body::from_stream(stream))
            .unwrap()
    }
}

#[tokio::test]
async fn sse_response_is_chunked() {
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let addr = listen({
        let tx = tx.clone();
        move |mut ws: WebSocket| {
            let tx = tx.clone();
            async move {
                let _ = recv_text(&mut ws).await;
                send_json(&mut ws, &welcome()).await;
                send_json(&mut ws, &request(3, Some(STANDARD.encode(b"{}")), true)).await;
                let mut frames = Vec::new();
                loop {
                    let raw = recv_text(&mut ws).await;
                    let frame: Frame = serde_json::from_str(&raw).unwrap();
                    let done = matches!(frame, Frame::ResponseBody { last: true, .. });
                    frames.push(frame);
                    if done {
                        break;
                    }
                }
                let _ = tx.lock().unwrap().take().unwrap().send(frames);
            }
        }
    })
    .await;
    let handle = run(cfg(addr), SseService);
    let frames = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        &frames[0],
        Frame::ResponseStart {
            body_complete: false,
            ..
        }
    ));
    let mut body = Vec::new();
    for frame in &frames[1..] {
        let Frame::ResponseBody { chunk, .. } = frame else {
            panic!("{frame:?}");
        };
        body.extend(STANDARD.decode(chunk).unwrap());
    }
    let text = String::from_utf8(body).unwrap();
    assert!(text.contains("data: a"), "{text}");
    assert!(text.contains("data: b"), "{text}");
    handle.shutdown.cancel();
}

#[derive(Clone)]
struct Hold {
    started: Arc<Notify>,
    dropped: Arc<AtomicBool>,
}

impl LocalService for Hold {
    async fn call(&self, _request: http::Request<Body>) -> http::Response<Body> {
        let started = self.started.clone();
        let dropped = self.dropped.clone();
        let _guard = DropFlag(dropped);
        started.notify_one();
        std::future::pending::<()>().await;
        http::Response::new(Body::empty())
    }
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn cancel_aborts_the_local_call() {
    let started = Arc::new(Notify::new());
    let dropped = Arc::new(AtomicBool::new(false));
    let addr = listen({
        let started = started.clone();
        move |mut ws: WebSocket| {
            let started = started.clone();
            async move {
                let _ = recv_text(&mut ws).await;
                send_json(&mut ws, &welcome()).await;
                send_json(&mut ws, &request(4, Some(STANDARD.encode(b"{}")), true)).await;
                started.notified().await;
                send_json(
                    &mut ws,
                    &Frame::Cancel {
                        id: 4,
                        reason: CancelReason::ClientGone,
                    },
                )
                .await;
            }
        }
    })
    .await;
    let hold = Hold {
        started: started.clone(),
        dropped: dropped.clone(),
    };
    let handle = run(cfg(addr), hold);
    tokio::time::timeout(Duration::from_secs(5), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancel should drop the local call");
    handle.shutdown.cancel();
}

#[tokio::test]
async fn answers_protocol_ping() {
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let addr = listen({
        let tx = tx.clone();
        move |mut ws: WebSocket| {
            let tx = tx.clone();
            async move {
                let _ = recv_text(&mut ws).await;
                send_json(&mut ws, &welcome()).await;
                send_json(&mut ws, &Frame::Ping { nonce: 9 }).await;
                let raw = recv_text(&mut ws).await;
                let _ = tx.lock().unwrap().take().unwrap().send(raw);
            }
        }
    })
    .await;
    let handle = run(cfg(addr), Router::new());
    let raw = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    let frame: Frame = serde_json::from_str(&raw).unwrap();
    assert_eq!(frame, Frame::Pong { nonce: 9 });
    handle.shutdown.cancel();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn goaway_reconnects_after_the_requested_delay() {
    let connects = Arc::new(AtomicUsize::new(0));
    let (second_tx, second_rx) = oneshot::channel();
    let second_tx = Arc::new(std::sync::Mutex::new(Some(second_tx)));
    let addr = listen({
        let connects = connects.clone();
        let second_tx = second_tx.clone();
        move |mut ws: WebSocket| {
            let connects = connects.clone();
            let second_tx = second_tx.clone();
            async move {
                let n = connects.fetch_add(1, Ordering::SeqCst);
                let hello = recv_text(&mut ws).await;
                send_json(&mut ws, &welcome()).await;
                if n == 0 {
                    assert!(!hello.contains("\"reclaim\""), "{hello}");
                    send_json(
                        &mut ws,
                        &Frame::GoAway {
                            reason: "deploy".into(),
                            reconnect_after_secs: 5,
                        },
                    )
                    .await;
                } else {
                    assert!(hello.contains("abcd2345"), "{hello}");
                    let _ = second_tx.lock().unwrap().take().unwrap().send(());
                    std::future::pending::<()>().await;
                }
            }
        }
    })
    .await;
    let mut handle = run(cfg(addr), Router::new());
    loop {
        if let TunnelState::Reconnecting { delay, .. } = handle.state.borrow().clone() {
            if delay == Duration::from_secs(5) {
                break;
            }
        }
        handle.state.changed().await.expect("go_away wait");
    }
    tokio::time::advance(Duration::from_secs(4)).await;
    assert_eq!(connects.load(Ordering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(2)).await;
    second_rx.await.expect("second welcome");
    assert!(connects.load(Ordering::SeqCst) >= 2);
    handle.shutdown.cancel();
}

#[tokio::test]
async fn terminal_reject_does_not_reconnect() {
    let connects = Arc::new(AtomicUsize::new(0));
    let addr = listen({
        let connects = connects.clone();
        move |mut ws: WebSocket| {
            let connects = connects.clone();
            async move {
                connects.fetch_add(1, Ordering::SeqCst);
                let _ = recv_text(&mut ws).await;
                let rejected = mcp_gateway_tunnel_proto::Rejected::new(
                    mcp_gateway_tunnel_proto::RejectCode::Unauthorized,
                    "named endpoints require login",
                    None,
                );
                send_json(&mut ws, &rejected).await;
            }
        }
    })
    .await;
    let mut handle = run(cfg(addr), Router::new());
    let state = wait_state(&mut handle.state).await;
    match state {
        TunnelState::Rejected { code, .. } => assert_eq!(code, "unauthorized"),
        other => panic!("{other:?}"),
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(connects.load(Ordering::SeqCst), 1);
    handle.shutdown.cancel();
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn refused_connect_backs_off() {
    let connects = Arc::new(AtomicUsize::new(0));
    let again = Arc::new(Notify::new());
    let addr = listen({
        let connects = connects.clone();
        let again = again.clone();
        move |mut ws: WebSocket| {
            let connects = connects.clone();
            let again = again.clone();
            async move {
                let n = connects.fetch_add(1, Ordering::SeqCst);
                let _ = recv_text(&mut ws).await;
                if n >= 1 {
                    again.notify_one();
                }
            }
        }
    })
    .await;
    let handle = run(cfg(addr), Router::new());
    loop {
        if connects.load(Ordering::SeqCst) >= 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    tokio::time::advance(Duration::from_millis(400)).await;
    assert_eq!(
        connects.load(Ordering::SeqCst),
        1,
        "backoff should still be running"
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    again.notified().await;
    assert!(connects.load(Ordering::SeqCst) >= 2);
    handle.shutdown.cancel();
}

#[derive(Clone)]
struct CountHold {
    calls: Arc<AtomicUsize>,
    started: Arc<Notify>,
}

impl LocalService for CountHold {
    async fn call(&self, _request: http::Request<Body>) -> http::Response<Body> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        std::future::pending::<()>().await;
        http::Response::new(Body::empty())
    }
}

#[tokio::test]
async fn inflight_limit_is_429_without_calling_the_service() {
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(Notify::new());
    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let addr = listen({
        let started = started.clone();
        let tx = tx.clone();
        move |mut ws: WebSocket| {
            let started = started.clone();
            let tx = tx.clone();
            async move {
                let _ = recv_text(&mut ws).await;
                let mut limited = welcome();
                limited.limits.max_inflight = 1;
                send_json(&mut ws, &limited).await;
                send_json(&mut ws, &request(1, Some(STANDARD.encode(b"{}")), true)).await;
                started.notified().await;
                send_json(&mut ws, &request(2, Some(STANDARD.encode(b"{}")), true)).await;
                let raw = recv_text(&mut ws).await;
                let _ = tx.lock().unwrap().take().unwrap().send(raw);
            }
        }
    })
    .await;
    let handle = run(
        cfg(addr),
        CountHold {
            calls: calls.clone(),
            started: started.clone(),
        },
    );
    let raw = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    let frame: Frame = serde_json::from_str(&raw).unwrap();
    match frame {
        Frame::ResponseStart { id, status, .. } => {
            assert_eq!(id, 2);
            assert_eq!(status, 429);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    handle.shutdown.cancel();
}

#[derive(Clone, Default)]
struct Proxied {
    hits: Arc<AtomicUsize>,
    auth: Arc<std::sync::Mutex<Option<String>>>,
    path: Arc<std::sync::Mutex<String>>,
}

#[tokio::test]
async fn http_upstream_through_the_relay_strips_the_gateway_token() {
    let proxied = Proxied::default();
    let state = proxied.clone();
    let app = Router::new()
        .route(
            "/custom",
            post(
                |State(state): State<Proxied>, request: axum::extract::Request| async move {
                    state.hits.fetch_add(1, Ordering::SeqCst);
                    let (parts, body) = request.into_parts();
                    let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
                    *state.path.lock().unwrap() = parts.uri.to_string();
                    *state.auth.lock().unwrap() = parts
                        .headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    bytes
                },
            ),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let upstream = mcp_gateway_tunnel::upstream::HttpUpstream::connect(
        &format!("http://{upstream_addr}/custom?x=1"),
        false,
    )
    .await
    .unwrap();
    let gate = mcp_gateway_tunnel::upstream::AuthGate::new(
        mcp_gateway_tunnel::upstream::ProxyAuth::Token,
        Some("secret".into()),
        upstream,
    );

    let (tx, rx) = oneshot::channel();
    let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
    let addr = listen({
        let tx = tx.clone();
        move |mut ws: WebSocket| {
            let tx = tx.clone();
            async move {
                let _ = recv_text(&mut ws).await;
                send_json(&mut ws, &welcome()).await;
                send_json(
                    &mut ws,
                    &Frame::RequestStart {
                        id: 1,
                        method: "POST".into(),
                        path: MCP_PATH.into(),
                        query: None,
                        headers: vec![("content-type".into(), "application/json".into())],
                        body_complete: true,
                        body: Some(STANDARD.encode(b"{\"id\":1}")),
                        client_ip: "203.0.113.9".into(),
                        request_id: "req-1".into(),
                    },
                )
                .await;
                let denied = recv_text(&mut ws).await;
                send_json(
                    &mut ws,
                    &Frame::RequestStart {
                        id: 2,
                        method: "POST".into(),
                        path: MCP_PATH.into(),
                        query: None,
                        headers: vec![
                            ("content-type".into(), "application/json".into()),
                            ("authorization".into(), "Bearer secret".into()),
                            ("cookie".into(), "a=b".into()),
                        ],
                        body_complete: true,
                        body: Some(STANDARD.encode(b"{\"id\":2}")),
                        client_ip: "203.0.113.9".into(),
                        request_id: "req-2".into(),
                    },
                )
                .await;
                let allowed = recv_text(&mut ws).await;
                let _ = tx.lock().unwrap().take().unwrap().send((denied, allowed));
            }
        }
    })
    .await;
    let handle = run(cfg(addr), gate);
    let (denied, allowed) = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .unwrap()
        .unwrap();
    let denied: Frame = serde_json::from_str(&denied).unwrap();
    match denied {
        Frame::ResponseStart {
            status,
            body_complete,
            body,
            ..
        } => {
            assert_eq!(status, 401);
            assert!(body_complete);
            let text = String::from_utf8(STANDARD.decode(body.unwrap()).unwrap()).unwrap();
            assert!(text.contains("missing authorization"), "{text}");
        }
        other => panic!("{other:?}"),
    }
    let allowed: Frame = serde_json::from_str(&allowed).unwrap();
    match allowed {
        Frame::ResponseStart { status, body, .. } => {
            assert_eq!(status, 200);
            let bytes = STANDARD.decode(body.unwrap()).unwrap();
            assert_eq!(bytes, b"{\"id\":2}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(proxied.hits.load(Ordering::SeqCst), 1);
    assert_eq!(proxied.path.lock().unwrap().as_str(), "/custom?x=1");
    assert!(proxied.auth.lock().unwrap().is_none());
    handle.shutdown.cancel();
}
