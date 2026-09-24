//! Streamable HTTP upstream.
//!
//! The client uses rustls, does not read `HTTP_PROXY`, and does not follow
//! redirects (a 3xx is returned as that status). Connect budget is 5 seconds.
//! The whole request, including a `text/event-stream` body, ends at 60 seconds
//! with HTTP 504 and `upstream timeout`. A connection failure is HTTP 502 and
//! `upstream unreachable`. Both JSON-RPC error codes are `-32001` and `id` is
//! null. The reqwest error is also printed on stderr as `upstream: ...`.
//!
//! Idle pooled connections are dropped after 90 seconds. DNS is not used
//! after startup: every request dials the address pinned by [`pin_upstream`].
//! The incoming path and query are replaced by the configured URL. `Host` is
//! the upstream authority. Hop-by-hop headers, `Cookie`, and `Content-Length`
//! are dropped. `Accept`, `Content-Type`, `Mcp-Session-Id`,
//! `MCP-Protocol-Version`, `Last-Event-ID`, and `Authorization` are copied
//! when they are still on the request (the auth gate may already have removed
//! `Authorization`).

use std::future::Future;
use std::time::Duration;

use axum::body::Body;
use futures_util::StreamExt;
use mcp_gateway_tunnel_proto::is_hop_by_hop_header;
use serde_json::{json, Value};

use super::policy::{pin_upstream, PinError, PinnedUpstream};
use super::rpc_response;
use crate::client_identity;
use crate::LocalService;

const ACCEPT: &str = "application/json, text/event-stream";
const PROTOCOL: &str = "2025-06-18";
const POOL_IDLE: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy)]
pub struct UpstreamLimits {
    pub connect: Duration,
    pub request: Duration,
}

impl Default for UpstreamLimits {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            request: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub name: String,
    pub version: String,
    pub tools: usize,
    pub tools_skipped: bool,
}

impl ProbeReport {
    pub fn summary(&self) -> String {
        if self.name.is_empty() && self.tools_skipped {
            return "skipped".to_owned();
        }
        if self.tools_skipped {
            return format!("{} {}, tools/list skipped", self.name, self.version);
        }
        format!("{} {}, {} tools", self.name, self.version, self.tools)
    }
}

#[derive(Clone)]
pub struct HttpUpstream {
    client: reqwest::Client,
    url: url::Url,
    host_header: String,
}

impl HttpUpstream {
    pub async fn connect(raw: &str, allow_remote: bool) -> Result<Self, PinError> {
        Self::connect_with(raw, allow_remote, UpstreamLimits::default()).await
    }

    pub async fn connect_with(
        raw: &str,
        allow_remote: bool,
        limits: UpstreamLimits,
    ) -> Result<Self, PinError> {
        let pinned = pin_upstream(raw, allow_remote).await?;
        Self::from_pinned(pinned, limits).map_err(PinError::Resolve)
    }

    fn from_pinned(pinned: PinnedUpstream, limits: UpstreamLimits) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .connect_timeout(limits.connect)
            .timeout(limits.request)
            .pool_idle_timeout(POOL_IDLE)
            .redirect(reqwest::redirect::Policy::custom(|attempt| attempt.stop()))
            .no_proxy()
            .user_agent(client_identity())
            .resolve(&pinned.resolve_host, pinned.addr)
            .build()
            .map_err(|err| format!("http client: {err}"))?;
        Ok(Self {
            client,
            url: pinned.url,
            host_header: pinned.host_header,
        })
    }

    pub fn host_header(&self) -> &str {
        &self.host_header
    }

    /// `POST` initialize, `notifications/initialized`, then `tools/list`.
    /// `authorization` is sent only when `Some`. A failure names the HTTP
    /// status or parse error and the Streamable HTTP hint.
    pub async fn probe(&self, authorization: Option<&str>) -> Result<ProbeReport, String> {
        let (status, headers, body) = self
            .rpc(
                "initialize",
                Some(1),
                json!({
                    "protocolVersion": PROTOCOL,
                    "capabilities": {},
                    "clientInfo": {
                        "name": "mcp-gateway",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
                None,
                authorization,
            )
            .await
            .map_err(|err| probe_hint(&err))?;
        if !(200..300).contains(&status) {
            return Err(probe_hint(&format!("initialize returned HTTP {status}")));
        }
        let message = parse_rpc(header_str(&headers, reqwest::header::CONTENT_TYPE), &body)
            .map_err(|err| probe_hint(&err))?;
        if let Some(error) = message.get("error") {
            return Err(probe_hint(&format!("initialize failed: {error}")));
        }
        let result = message
            .get("result")
            .cloned()
            .ok_or_else(|| probe_hint("initialize response had no result"))?;
        let name = result
            .pointer("/serverInfo/name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let version = result
            .pointer("/serverInfo/version")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let session = header_str(&headers, "mcp-session-id").map(str::to_owned);

        let note = self
            .rpc(
                "notifications/initialized",
                None,
                json!({}),
                session.as_deref(),
                authorization,
            )
            .await;
        if let Ok((status, _, _)) = &note {
            if *status >= 400 {
                return Err(probe_hint(&format!(
                    "notifications/initialized returned HTTP {status}"
                )));
            }
        }

        let (status, headers, body) = self
            .rpc(
                "tools/list",
                Some(2),
                json!({}),
                session.as_deref(),
                authorization,
            )
            .await
            .map_err(|err| probe_hint(&err))?;
        if !(200..300).contains(&status) {
            return Err(probe_hint(&format!("tools/list returned HTTP {status}")));
        }
        let message = parse_rpc(header_str(&headers, reqwest::header::CONTENT_TYPE), &body)
            .map_err(|err| probe_hint(&err))?;
        if let Some(error) = message.get("error") {
            return Err(probe_hint(&format!("tools/list failed: {error}")));
        }
        let tools = message
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .ok_or_else(|| probe_hint("tools/list did not return a tools array"))?;
        Ok(ProbeReport {
            name,
            version,
            tools: tools.len(),
            tools_skipped: false,
        })
    }

    async fn rpc(
        &self,
        method: &str,
        id: Option<u64>,
        params: Value,
        session: Option<&str>,
        authorization: Option<&str>,
    ) -> Result<(u16, reqwest::header::HeaderMap, String), String> {
        let mut payload = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        if let Some(id) = id {
            payload["id"] = json!(id);
        } else {
            payload.as_object_mut().expect("object").remove("id");
        }
        let mut builder = self
            .client
            .post(self.url.clone())
            .header(reqwest::header::HOST, &self.host_header)
            .header(reqwest::header::ACCEPT, ACCEPT)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .json(&payload);
        if let Some(session) = session {
            builder = builder.header("mcp-session-id", session);
        }
        if let Some(authorization) = authorization {
            builder = builder.header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {authorization}"),
            );
        }
        let response = builder.send().await.map_err(|err| {
            eprintln!("upstream: {err}");
            if err.is_timeout() {
                "upstream timeout".to_owned()
            } else {
                "upstream unreachable".to_owned()
            }
        })?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.text().await.map_err(|err| err.to_string())?;
        Ok((status, headers, body))
    }
}

fn header_str(
    headers: &reqwest::header::HeaderMap,
    name: impl reqwest::header::AsHeaderName,
) -> Option<&str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

impl LocalService for HttpUpstream {
    fn call(
        &self,
        request: ::http::Request<Body>,
    ) -> impl Future<Output = ::http::Response<Body>> + Send {
        let client = self.client.clone();
        let url = self.url.clone();
        let host_header = self.host_header.clone();
        async move { forward(client, url, host_header, request).await }
    }
}

async fn forward(
    client: reqwest::Client,
    url: url::Url,
    host_header: String,
    request: ::http::Request<Body>,
) -> ::http::Response<Body> {
    let method = request.method().clone();
    let headers = request.headers().clone();
    let stream = http_body_util::BodyExt::into_data_stream(request.into_body())
        .map(|chunk| chunk.map_err(|err| std::io::Error::other(err.to_string())));
    let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
        Ok(method) => method,
        Err(_) => return rpc_response(400, -32600, "unsupported method", Value::Null),
    };
    let mut builder = client
        .request(reqwest_method, url)
        .header(reqwest::header::HOST, host_header)
        .body(reqwest::Body::wrap_stream(stream));
    for (name, value) in headers.iter() {
        if skip_request_header(name.as_str()) {
            continue;
        }
        builder = builder.header(name.as_str(), value.as_bytes());
    }
    let response = match builder.send().await {
        Ok(response) => response,
        Err(err) => {
            eprintln!("upstream: {err}");
            return if err.is_timeout() {
                rpc_response(504, -32001, "upstream timeout", Value::Null)
            } else {
                rpc_response(502, -32001, "upstream unreachable", Value::Null)
            };
        }
    };
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .bytes_stream()
        .map(|chunk| chunk.map_err(|err| std::io::Error::other(err.to_string())));
    let mut out = ::http::Response::builder().status(status.as_u16());
    for (name, value) in headers.iter() {
        if skip_response_header(name.as_str()) {
            continue;
        }
        out = out.header(name.as_str(), value.as_bytes());
    }
    out.body(Body::from_stream(bytes))
        .unwrap_or_else(|_| rpc_response(502, -32001, "upstream unreachable", Value::Null))
}

fn skip_request_header(name: &str) -> bool {
    is_hop_by_hop_header(name)
        || name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("content-length")
}

fn skip_response_header(name: &str) -> bool {
    is_hop_by_hop_header(name) || name.eq_ignore_ascii_case("content-length")
}

fn probe_hint(detail: &str) -> String {
    format!(
        "{detail}\nIs the server running? Is this path Streamable HTTP (POST with Accept: {ACCEPT})? The older SSE transport (GET /sse and POST /messages) is not proxied."
    )
}

fn parse_rpc(content_type: Option<&str>, body: &str) -> Result<Value, String> {
    let payload = if content_type.is_some_and(|value| value.contains("text/event-stream")) {
        sse_json(body)?
    } else {
        body.to_owned()
    };
    serde_json::from_str(&payload).map_err(|err| format!("response was not JSON: {err}"))
}

fn sse_json(body: &str) -> Result<String, String> {
    let mut lines = Vec::new();
    for line in body.lines() {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        let rest = rest.strip_prefix(' ').unwrap_or(rest);
        if rest.is_empty() || rest == "[DONE]" {
            continue;
        }
        lines.push(rest.to_owned());
    }
    if lines.is_empty() {
        return Err("event stream contained no data".into());
    }
    let joined = lines.join("\n");
    if serde_json::from_str::<Value>(&joined).is_ok() {
        return Ok(joined);
    }
    for line in &lines {
        if serde_json::from_str::<Value>(line).is_ok() {
            return Ok(line.clone());
        }
    }
    Err("event stream data was not JSON".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::routing::post;
    use axum::Router;
    use std::sync::{Arc, Mutex};

    #[test]
    fn probe_summary_names_the_server_and_tool_count() {
        let report = ProbeReport {
            name: "weather".into(),
            version: "1.2.0".into(),
            tools: 4,
            tools_skipped: false,
        };
        assert_eq!(report.summary(), "weather 1.2.0, 4 tools");
    }

    #[test]
    fn sse_data_line_parses_as_json() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n\n";
        let value = parse_rpc(Some("text/event-stream"), body).unwrap();
        assert_eq!(value["id"], 1);
    }

    #[derive(Clone)]
    struct Seen {
        inner: Arc<Mutex<Option<SeenRequest>>>,
    }

    struct SeenRequest {
        method: String,
        path: String,
        host: String,
        accept: String,
        authorization: Option<String>,
        cookie: Option<String>,
        body: Vec<u8>,
    }

    async fn serve(seen: Seen) -> std::net::SocketAddr {
        let app = Router::new()
            .route(
                "/custom",
                post(
                    |State(seen): State<Seen>, request: axum::extract::Request| async move {
                        let (parts, body) = request.into_parts();
                        let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
                        let header = |name: &str| {
                            parts
                                .headers
                                .get(name)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or("")
                                .to_owned()
                        };
                        *seen.inner.lock().unwrap() = Some(SeenRequest {
                            method: parts.method.to_string(),
                            path: parts.uri.to_string(),
                            host: header("host"),
                            accept: header("accept"),
                            authorization: parts
                                .headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            cookie: parts
                                .headers
                                .get("cookie")
                                .and_then(|value| value.to_str().ok())
                                .map(str::to_owned),
                            body: bytes.to_vec(),
                        });
                        bytes
                    },
                ),
            )
            .with_state(seen);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn forwards_to_the_configured_url_and_drops_cookie() {
        let seen = Seen {
            inner: Arc::new(Mutex::new(None)),
        };
        let addr = serve(seen.clone()).await;
        let upstream = HttpUpstream::connect(&format!("http://{addr}/custom?x=1"), false)
            .await
            .unwrap();
        let request = ::http::Request::builder()
            .method("POST")
            .uri("/mcp?y=2")
            .header("accept", "application/json")
            .header("cookie", "session=1")
            .header("connection", "close")
            .header("authorization", "Bearer keep")
            .header("host", "relay.example")
            .body(Body::from(Bytes::from_static(b"{\"id\":1}")))
            .unwrap();
        let response = upstream.call(request).await;
        assert_eq!(response.status(), 200);
        let got = seen.inner.lock().unwrap().take().unwrap();
        assert_eq!(got.method, "POST");
        assert_eq!(got.path, "/custom?x=1");
        assert_eq!(got.host, format!("{addr}"));
        assert_eq!(got.accept, "application/json");
        assert_eq!(got.authorization.as_deref(), Some("Bearer keep"));
        assert!(got.cookie.is_none());
        assert_eq!(got.body, b"{\"id\":1}");
    }

    #[tokio::test]
    async fn connection_refused_is_502() {
        let upstream = HttpUpstream::connect("http://127.0.0.1:1/mcp", false)
            .await
            .unwrap();
        let response = upstream
            .call(
                ::http::Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), 502);
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains("upstream unreachable"), "{text}");
    }

    #[tokio::test]
    async fn probe_reads_server_info_and_tool_count() {
        let app = Router::new().route(
            "/mcp",
            post(|body: Bytes| async move {
                let value: Value = serde_json::from_slice(&body).unwrap();
                let method = value["method"].as_str().unwrap();
                let response = if method == "initialize" {
                    json!({
                        "jsonrpc": "2.0",
                        "id": value["id"],
                        "result": {
                            "protocolVersion": "2025-06-18",
                            "serverInfo": { "name": "fixture", "version": "9" },
                            "capabilities": {}
                        }
                    })
                } else if method == "tools/list" {
                    json!({
                        "jsonrpc": "2.0",
                        "id": value["id"],
                        "result": { "tools": [{ "name": "echo" }, { "name": "ping" }] }
                    })
                } else {
                    json!({ "jsonrpc": "2.0" })
                };
                (
                    [(::http::header::CONTENT_TYPE, "application/json")],
                    serde_json::to_vec(&response).unwrap(),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let upstream = HttpUpstream::connect(&format!("http://{addr}/mcp"), false)
            .await
            .unwrap();
        let report = upstream.probe(None).await.unwrap();
        assert_eq!(report.summary(), "fixture 9, 2 tools");
    }
}
