//! Upstream adapters for `mcp-gateway tunnel`.
//!
//! [`HttpUpstream`] forwards Streamable HTTP. [`StdioBridge`] speaks
//! newline-delimited JSON-RPC to a child process. [`AuthGate`] sits in front
//! of either one.

mod auth;
mod http;
mod policy;
mod stdio;

pub use auth::{generate_bearer_token, AuthGate, ProxyAuth};
pub use http::{HttpUpstream, ProbeReport, UpstreamLimits};
pub use policy::{address_allowed, PinError, PinnedUpstream};
pub use stdio::StdioBridge;

use axum::body::Body;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::LocalService;

/// Listen on `listener` and forward `/mcp` to `service`. Any other path is 404.
/// `GET /mcp` and `DELETE /mcp` are answered by the service.
pub async fn serve_local<S: LocalService>(
    listener: tokio::net::TcpListener,
    service: S,
    shutdown: CancellationToken,
) {
    let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let service = service.clone();
        async move {
            if request.uri().path() != "/mcp" {
                return empty_response(404);
            }
            service.call(request).await
        }
    });
    let _ = axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await;
}

pub(crate) fn rpc_response(
    status: u16,
    code: i32,
    message: &str,
    id: Value,
) -> ::http::Response<Body> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "error": { "code": code, "message": message },
        "id": id,
    });
    let mut builder = ::http::Response::builder()
        .status(status)
        .header(::http::header::CONTENT_TYPE, "application/json");
    if status == 401 {
        builder = builder.header(::http::header::WWW_AUTHENTICATE, "Bearer");
    }
    builder
        .body(Body::from(serde_json::to_vec(&body).unwrap_or_default()))
        .unwrap_or_else(|_| ::http::Response::new(Body::empty()))
}

pub(crate) fn empty_response(status: u16) -> ::http::Response<Body> {
    ::http::Response::builder()
        .status(status)
        .body(Body::empty())
        .unwrap_or_else(|_| ::http::Response::new(Body::empty()))
}

pub(crate) fn json_body(status: u16, body: &Value) -> ::http::Response<Body> {
    ::http::Response::builder()
        .status(status)
        .header(::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap_or_default()))
        .unwrap_or_else(|_| ::http::Response::new(Body::empty()))
}
