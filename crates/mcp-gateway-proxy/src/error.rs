use crate::ssrf::SsrfError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error(transparent)]
    Ssrf(#[from] SsrfError),
    #[error("invalid arguments: {0}")]
    Schema(String),
    #[error("reserved header")]
    ReservedHeader,
    #[error("open redirect in path substitution")]
    OpenRedirect,
    #[error("upstream request failed: {0}")]
    Upstream(String),
    #[error("upstream returned HTTP {0}")]
    UpstreamStatus(u16),
    #[error("upstream response too large")]
    TooLarge,
    #[error("timeout")]
    Timeout,
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("peer address mismatch")]
    PeerMismatch,
    #[error("redirect on POST is not followed")]
    RedirectOnPost,
    #[error("credential unwrap failed")]
    Credential,
}

impl ProxyError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::Ssrf(_) => "ssrf",
            Self::Schema(_) => "schema",
            Self::ReservedHeader => "reserved_header",
            Self::OpenRedirect => "ssrf",
            Self::Upstream(_) => "upstream",
            Self::UpstreamStatus(s) if *s >= 500 => "upstream_5xx",
            Self::UpstreamStatus(_) => "upstream_4xx",
            Self::TooLarge => "too_large",
            Self::Timeout => "timeout",
            Self::TooManyRedirects => "ssrf",
            Self::PeerMismatch => "ssrf",
            Self::RedirectOnPost => "ssrf",
            Self::Credential => "internal",
        }
    }

    pub fn customer_message(&self) -> String {
        match self {
            Self::Ssrf(_) | Self::OpenRedirect | Self::PeerMismatch | Self::TooManyRedirects => {
                "Upstream address is not allowed".into()
            }
            Self::Schema(path) => format!("Invalid arguments: {path}"),
            Self::ReservedHeader => "Reserved header".into(),
            Self::Upstream(_) | Self::Timeout => "Upstream request failed".into(),
            Self::UpstreamStatus(s) => format!("Upstream returned HTTP {s}"),
            Self::TooLarge => "Upstream response too large".into(),
            Self::RedirectOnPost => "Upstream request failed".into(),
            Self::Credential => "Upstream request failed".into(),
        }
    }
}
