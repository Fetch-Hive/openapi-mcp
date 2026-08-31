//! Single-tenant MCP Gateway protocol server.
//!
//! Hosted multi-tenant glue lives in the private hosted overlay, not this crate.

mod bind;
mod handler;
mod http;
mod stdio;

pub use bind::{is_loopback_bind, parse_bind, validate_http_serve, BindError};
pub use handler::{GatewayHandler, LiveExecutor, LocalGateway, UpstreamExecutor};
pub use http::{build_router, serve_http, HttpServeOptions};
pub use stdio::serve_stdio;
