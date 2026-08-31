use mcp_gateway_ir::ExecutionPlan;
use mcp_gateway_proxy::ssrf::{MapResolver, SsrfPolicy};
use mcp_gateway_proxy::{render, validate_and_dial, InjectedCredential};
use secrecy::SecretString;
use serde_json::json;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

async fn local_echo(
    status: u16,
    location: Option<&'static str>,
    body: &'static [u8],
) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let io = hyper_util::rt::TokioIo::new(stream);
        hyper::server::conn::http1::Builder::new()
            .serve_connection(
                io,
                hyper::service::service_fn(move |_req| {
                    let mut builder = hyper::Response::builder().status(status);
                    if let Some(loc) = location {
                        builder = builder.header("location", loc);
                    }
                    let resp = builder
                        .body(http_body_util::Full::new(bytes::Bytes::from_static(body)))
                        .unwrap();
                    async move { Ok::<_, Infallible>(resp) }
                }),
            )
            .await
            .ok();
    });
    addr
}

fn get_plan() -> ExecutionPlan {
    ExecutionPlan {
        method: "GET".into(),
        path_template: "/".into(),
        path_params: vec![],
        query_params: vec![],
        header_params: vec![],
        cookie_params: vec![],
        body: None,
        accept: "*/*".into(),
        timeout_ms: 5_000,
    }
}

#[tokio::test]
async fn loopback_without_flag_is_denied() {
    let req = render("http://127.0.0.1/", &get_plan(), &json!({})).unwrap();
    let err = validate_and_dial(req, &SsrfPolicy::default(), &MapResolver::default(), None)
        .await
        .unwrap_err();
    assert!(matches!(err, mcp_gateway_proxy::ProxyError::Ssrf(_)));
}

#[tokio::test]
async fn redirect_to_private_is_denied_on_second_hop() {
    let addr = local_echo(302, Some("http://169.254.169.254/"), b"").await;
    let policy = SsrfPolicy::default()
        .with_allow_insecure_http(true)
        .with_allow_loopback(true)
        .with_max_hops(3);
    let resolver = MapResolver::single("echo.test", addr.ip());
    let req = render(
        &format!("http://echo.test:{}/", addr.port()),
        &get_plan(),
        &json!({}),
    )
    .unwrap();
    let err = validate_and_dial(req, &policy, &resolver, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, mcp_gateway_proxy::ProxyError::Ssrf(_)),
        "redirect to metadata must fail SSRF, got {err:?}"
    );
}

#[tokio::test]
async fn ip_literal_metadata_is_denied_without_dns() {
    let req = render("http://169.254.169.254/", &get_plan(), &json!({})).unwrap();
    let resolver = MapResolver::default();
    let err = validate_and_dial(
        req,
        &SsrfPolicy::default().with_allow_insecure_http(true),
        &resolver,
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, mcp_gateway_proxy::ProxyError::Ssrf(_)));
}

#[tokio::test]
async fn bearer_is_injected_once_by_dialer() {
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured_task = captured.clone();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let io = hyper_util::rt::TokioIo::new(stream);
        hyper::server::conn::http1::Builder::new()
            .serve_connection(
                io,
                hyper::service::service_fn(move |req| {
                    let auth = req
                        .headers()
                        .get(http::header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .map(ToOwned::to_owned);
                    *captured_task.lock().unwrap() = auth;
                    let resp = hyper::Response::builder()
                        .status(200)
                        .body(http_body_util::Full::new(bytes::Bytes::from_static(b"{}")))
                        .unwrap();
                    async move { Ok::<_, Infallible>(resp) }
                }),
            )
            .await
            .ok();
    });
    let policy = SsrfPolicy::default()
        .with_allow_insecure_http(true)
        .with_allow_loopback(true)
        .with_max_hops(3);
    let resolver = MapResolver::single("echo.test", addr.ip());
    let req = render(
        &format!("http://echo.test:{}/", addr.port()),
        &get_plan(),
        &json!({}),
    )
    .unwrap();
    let cred = InjectedCredential {
        kind: mcp_gateway_proxy::credentials::InjectedKind::Bearer,
        secret: SecretString::from("once-only-token"),
        header_name: None,
        param_name: None,
    };
    let _ = validate_and_dial(req, &policy, &resolver, Some(&cred))
        .await
        .unwrap();
    let auth = captured.lock().unwrap().clone();
    assert_eq!(auth.as_deref(), Some("Bearer once-only-token"));
}
