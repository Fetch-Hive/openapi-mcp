//! IR execution plan to outbound HTTP. No axum, no Redis, no Loki.
//!
//! Customer URLs are dialed only through [`ssrf`] + [`dial`]. `reqwest` is
//! intentionally not a dependency of this crate.

pub mod credentials;
pub mod dial;
pub mod error;
pub mod headers;
pub mod map;
pub mod render;
pub mod ssrf;

pub use credentials::{encrypt, inject, CredentialSpec, InjectedCredential, InjectedKind};
pub use dial::validate_and_dial;
pub use error::ProxyError;
pub use map::{
    error_result, map_proxy_error, map_success, validate_schema, ResponseKind, ToolResult,
    UpstreamResponse,
};
pub use render::{render, RenderedRequest};
pub use ssrf::{pin_url, MapResolver, Pinned, Resolver, SsrfError, SsrfPolicy};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles() {}
}
