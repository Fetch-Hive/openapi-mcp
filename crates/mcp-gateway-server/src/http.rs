use crate::bind::{is_loopback_bind, validate_http_serve, BindError};
use crate::handler::GatewayHandler;
use axum::extract::Request;
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use std::net::SocketAddr;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tower::ServiceExt;
use tower_http::timeout::TimeoutLayer;

#[derive(Clone)]
pub struct HttpServeOptions {
    pub bind: SocketAddr,
    pub expose: bool,
    pub bearer_token: Option<String>,
    pub allow_anonymous: bool,
    pub path: String,
}

impl Default for HttpServeOptions {
    fn default() -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 8787)),
            expose: false,
            bearer_token: None,
            allow_anonymous: false,
            path: "/mcp".into(),
        }
    }
}

#[derive(Clone)]
struct AuthState {
    token: Option<String>,
    allow_anonymous: bool,
    loopback: bool,
    health_path: String,
}

pub fn build_router(handler: GatewayHandler, opts: &HttpServeOptions) -> Result<Router, BindError> {
    validate_http_serve(
        opts.bind,
        opts.expose,
        opts.allow_anonymous,
        opts.bearer_token.as_deref(),
        &opts.path,
    )?;

    let auth = AuthState {
        token: opts
            .bearer_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned),
        allow_anonymous: opts.allow_anonymous,
        loopback: is_loopback_bind(opts.bind),
        health_path: "/health".into(),
    };

    let mut rmcp_config = StreamableHttpServerConfig::default();
    rmcp_config.legacy_session_mode = false;
    rmcp_config.json_response = true;
    if opts.expose {
        rmcp_config.allowed_hosts.clear();
    } else {
        rmcp_config.allowed_hosts = vec![
            "localhost".into(),
            "127.0.0.1".into(),
            "[::1]".into(),
            "::1".into(),
        ];
    }
    rmcp_config.stateless_protocol_metadata_required = true;

    let mcp = StreamableHttpService::new(
        {
            let handler = handler.clone();
            move || Ok(handler.clone())
        },
        NeverSessionManager::default().into(),
        rmcp_config,
    );

    let mcp_path = opts.path.clone();
    Ok(Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(
            &mcp_path,
            post(mcp_post).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(60),
        ))
        .layer(from_fn_with_state(auth, bearer_gate))
        .with_state(mcp))
}

pub async fn serve_http(
    handler: GatewayHandler,
    opts: HttpServeOptions,
) -> Result<(), std::io::Error> {
    let app = build_router(handler, &opts).map_err(|e| {
        let kind = match e {
            BindError::ExposeRequired | BindError::AnonymousLoopbackOnly => {
                std::io::ErrorKind::PermissionDenied
            }
            BindError::TokenRequired | BindError::Invalid(_) | BindError::InvalidPath => {
                std::io::ErrorKind::InvalidInput
            }
        };
        std::io::Error::new(kind, e.to_string())
    })?;
    let listener = tokio::net::TcpListener::bind(opts.bind).await?;
    axum::serve(listener, app).await
}

async fn mcp_post(
    axum::extract::State(svc): axum::extract::State<
        StreamableHttpService<GatewayHandler, NeverSessionManager>,
    >,
    req: Request,
) -> Response {
    match svc.oneshot(req).await {
        Ok(resp) => resp.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn bearer_gate(
    axum::extract::State(auth): axum::extract::State<AuthState>,
    headers: HeaderMap,
    req: Request,
    next: Next,
) -> Response {
    if req.uri().path() == auth.health_path {
        return next.run(req).await;
    }
    if auth.allow_anonymous && auth.loopback {
        return next.run(req).await;
    }
    let Some(expected) = auth.token.as_deref().filter(|s| !s.is_empty()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match provided {
        Some(got)
            if got.len() == expected.len()
                && bool::from(got.as_bytes().ct_eq(expected.as_bytes())) =>
        {
            next.run(req).await
        }
        _ => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response(),
    }
}
