use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use mcp_gateway_compile::load_bytes;
use mcp_gateway_ir::SourceKind;
use mcp_gateway_proxy::ssrf::MapResolver;
use mcp_gateway_proxy::SsrfPolicy;
use mcp_gateway_server::{
    build_router, validate_http_serve, BindError, GatewayHandler, HttpServeOptions, LiveExecutor,
    LocalGateway,
};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceExt;

const TOKEN: &str = "fh_mcp_live_e2e_token_value_not_empty";
const TINY: &str = include_str!("../../mcp-gateway-cli/tests/fixtures/tiny.yaml");

fn compile_tiny() -> mcp_gateway_ir::CompileBundle {
    let loaded =
        load_bytes(TINY.as_bytes().to_vec(), SourceKind::File, "tiny.yaml").expect("load tiny");
    mcp_gateway_compile::compile_loaded(&loaded, mcp_gateway_compile::CompileOptions::default())
        .expect("compile tiny")
}

async fn echo_loop(body: &'static [u8]) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let io = hyper_util::rt::TokioIo::new(stream);
            let body = body;
            hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    io,
                    hyper::service::service_fn(move |_req| {
                        let resp = hyper::Response::builder()
                            .status(200)
                            .header("content-type", "application/json")
                            .body(http_body_util::Full::new(bytes::Bytes::from_static(body)))
                            .unwrap();
                        async move { Ok::<_, Infallible>(resp) }
                    }),
                )
                .await
                .ok();
        }
    });
    addr
}

fn handler_for(base_url: String, enabled: Vec<String>, disabled: Vec<String>) -> GatewayHandler {
    let bundle = compile_tiny();
    let policy = SsrfPolicy::default()
        .with_allow_insecure_http(true)
        .with_allow_loopback(true);
    let resolver = Arc::new(MapResolver::default());
    GatewayHandler::new(
        Arc::new(LocalGateway {
            bundle: Arc::new(bundle),
            base_url,
            credential: None,
            ssrf: policy,
            enabled_tools: enabled,
            disabled_tools: disabled,
        }),
        Arc::new(LiveExecutor { resolver }),
    )
}

fn opts(expose: bool, token: Option<&str>, anon: bool) -> HttpServeOptions {
    HttpServeOptions {
        bind: "127.0.0.1:8787".parse().unwrap(),
        expose,
        bearer_token: token.map(ToOwned::to_owned),
        allow_anonymous: anon,
        path: "/mcp".into(),
    }
}

async fn oneshot(
    handler: GatewayHandler,
    serve: &HttpServeOptions,
    req: Request<Body>,
) -> axum::http::Response<Body> {
    let app = build_router(handler, serve).expect("router");
    app.oneshot(req).await.unwrap()
}

async fn body_text(resp: axum::http::Response<Body>) -> String {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn json_rpc(id: u64, method: &str, mut params: Value) -> Value {
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "e2e", "version": "0"}
    });
    match &mut params {
        Value::Object(map) => {
            map.insert("_meta".into(), meta);
        }
        Value::Null => {
            params = json!({ "_meta": meta });
        }
        other => {
            params = json!({ "value": other, "_meta": meta });
        }
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    })
}

fn mcp_post(
    token: Option<&str>,
    host: &str,
    method: &str,
    name: Option<&str>,
    body: Value,
) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("host", host)
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", method);
    if let Some(n) = name {
        b = b.header("mcp-name", n);
    }
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn health_is_ok() {
    let h = handler_for("https://example.com".into(), vec![], vec![]);
    let resp = oneshot(
        h,
        &opts(false, Some(TOKEN), false),
        Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_text(resp).await, "ok");
}

#[tokio::test]
async fn missing_and_wrong_bearer_are_401() {
    let serve = opts(false, Some(TOKEN), false);
    let h = handler_for("https://example.com".into(), vec![], vec![]);
    let resp = oneshot(
        h.clone(),
        &serve,
        mcp_post(
            None,
            "127.0.0.1",
            "tools/list",
            None,
            json_rpc(1, "tools/list", json!({})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let resp = oneshot(
        h,
        &serve,
        mcp_post(
            Some("wrong-token"),
            "127.0.0.1",
            "tools/list",
            None,
            json_rpc(1, "tools/list", json!({})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn empty_token_is_rejected_at_validate() {
    let addr = "127.0.0.1:8787".parse().unwrap();
    let err = validate_http_serve(addr, false, false, Some(""), "/mcp").unwrap_err();
    assert!(matches!(err, BindError::TokenRequired));
    let err = build_router(
        handler_for("https://example.com".into(), vec![], vec![]),
        &opts(false, Some(""), false),
    )
    .unwrap_err();
    assert!(matches!(err, BindError::TokenRequired));
}

#[tokio::test]
async fn expose_accepts_public_host_header() {
    let h = handler_for("https://example.com".into(), vec![], vec![]);
    let resp = oneshot(
        h,
        &opts(true, Some(TOKEN), false),
        mcp_post(
            Some(TOKEN),
            "vps.example.com",
            "initialize",
            None,
            json_rpc(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2026-07-28",
                    "capabilities": {},
                    "clientInfo": {"name": "e2e", "version": "0"}
                }),
            ),
        ),
    )
    .await;
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "{}",
        body_text(resp).await
    );
}

#[tokio::test]
async fn anonymous_loopback_skips_bearer() {
    let h = handler_for("https://example.com".into(), vec![], vec![]);
    let resp = oneshot(
        h,
        &opts(false, None, true),
        mcp_post(
            None,
            "127.0.0.1",
            "initialize",
            None,
            json_rpc(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2026-07-28",
                    "capabilities": {},
                    "clientInfo": {"name": "e2e", "version": "0"}
                }),
            ),
        ),
    )
    .await;
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn anonymous_non_loopback_is_error() {
    let addr = "0.0.0.0:8787".parse().unwrap();
    let err = validate_http_serve(addr, true, true, None, "/mcp").unwrap_err();
    assert!(matches!(err, BindError::AnonymousLoopbackOnly));
}

#[tokio::test]
async fn initialize_list_and_call_against_echo() {
    let addr = echo_loop(br#"[{"id":1}]"#).await;
    let base = format!("http://127.0.0.1:{}", addr.port());
    let h = handler_for(base, vec![], vec![]);
    let serve = opts(false, Some(TOKEN), false);

    let listed = h.execute_named("list_pets", json!({})).await;
    assert!(!listed.is_error, "{}", listed.text);
    assert!(listed.text.contains("id"), "{}", listed.text);

    let bad = h.execute_named("nope", json!({})).await;
    assert_eq!(bad.error_code.as_deref(), Some("unknown_tool"));

    let schema = h.execute_named("list_pets", json!({"limit": "nope"})).await;
    assert_eq!(schema.error_code.as_deref(), Some("invalid_arguments"));

    let resp = oneshot(
        h.clone(),
        &serve,
        mcp_post(
            Some(TOKEN),
            "127.0.0.1",
            "initialize",
            None,
            json_rpc(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2026-07-28",
                    "capabilities": {},
                    "clientInfo": {"name": "e2e", "version": "0"}
                }),
            ),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK, "{}", body_text(resp).await);

    let resp = oneshot(
        h.clone(),
        &serve,
        mcp_post(
            Some(TOKEN),
            "127.0.0.1",
            "tools/list",
            None,
            json_rpc(2, "tools/list", json!({})),
        ),
    )
    .await;
    let text = body_text(resp).await;
    assert!(text.contains("list_pets"), "{text}");

    let resp = oneshot(
        h,
        &serve,
        mcp_post(
            Some(TOKEN),
            "127.0.0.1",
            "tools/call",
            Some("list_pets"),
            json_rpc(
                3,
                "tools/call",
                json!({"name": "list_pets", "arguments": {}}),
            ),
        ),
    )
    .await;
    let text = body_text(resp).await;
    assert!(!text.contains("\"isError\": true"), "{text}");
}

#[tokio::test]
async fn disabled_tools_are_hidden() {
    let h = handler_for(
        "https://example.com".into(),
        vec![],
        vec!["list_pets".into()],
    );
    let result = h.execute_named("list_pets", json!({})).await;
    assert_eq!(result.error_code.as_deref(), Some("unknown_tool"));
    assert_eq!(h.gateway.operations().count(), 0);
}

#[tokio::test]
async fn enabled_tools_allowlist() {
    let h = handler_for(
        "https://example.com".into(),
        vec!["does_not_exist".into()],
        vec![],
    );
    assert_eq!(h.gateway.operations().count(), 0);
}

#[test]
fn compile_helper_sees_list_pets() {
    let bundle = compile_tiny();
    assert!(bundle
        .api
        .operations
        .iter()
        .any(|op| op.tool.name == "list_pets"));
}
