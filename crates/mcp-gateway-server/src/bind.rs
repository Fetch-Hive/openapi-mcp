use std::net::{IpAddr, SocketAddr};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BindError {
    #[error("invalid bind address: {0}")]
    Invalid(String),
    #[error(
        "non-loopback bind requires --expose\nhint: --expose acknowledges that the MCP endpoint will be reachable on all\n      interfaces. TLS is your reverse proxy's job. Auth remains required."
    )]
    ExposeRequired,
    #[error("--allow-anonymous is loopback-only")]
    AnonymousLoopbackOnly,
    #[error("bearer token required unless --allow-anonymous")]
    TokenRequired,
    #[error("MCP path must start with /")]
    InvalidPath,
}

pub fn parse_bind(raw: &str) -> Result<SocketAddr, BindError> {
    raw.parse::<SocketAddr>()
        .or_else(|_| {
            raw.parse::<u16>()
                .map(|port| SocketAddr::from(([127, 0, 0, 1], port)))
                .map_err(|_| BindError::Invalid(raw.to_owned()))
        })
        .map_err(|_| BindError::Invalid(raw.to_owned()))
}

pub fn is_loopback_bind(addr: SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

pub fn validate_http_serve(
    bind: SocketAddr,
    expose: bool,
    allow_anonymous: bool,
    bearer_token: Option<&str>,
    path: &str,
) -> Result<(), BindError> {
    if !path.starts_with('/') {
        return Err(BindError::InvalidPath);
    }
    if !is_loopback_bind(bind) && !expose {
        return Err(BindError::ExposeRequired);
    }
    if allow_anonymous && !is_loopback_bind(bind) {
        return Err(BindError::AnonymousLoopbackOnly);
    }
    let token = bearer_token.map(str::trim).filter(|s| !s.is_empty());
    if token.is_none() && !allow_anonymous {
        return Err(BindError::TokenRequired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_loopback() {
        let addr = parse_bind("8787").unwrap();
        assert!(is_loopback_bind(addr));
        assert_eq!(addr.port(), 8787);
    }

    #[test]
    fn all_interfaces_is_not_loopback() {
        let addr = parse_bind("0.0.0.0:8787").unwrap();
        assert!(!is_loopback_bind(addr));
    }

    #[test]
    fn expose_required_for_all_interfaces() {
        let addr = parse_bind("0.0.0.0:8787").unwrap();
        let err = validate_http_serve(addr, false, false, Some("tok"), "/mcp").unwrap_err();
        assert!(matches!(err, BindError::ExposeRequired));
    }

    #[test]
    fn empty_token_is_missing() {
        let addr = parse_bind("127.0.0.1:8787").unwrap();
        let err = validate_http_serve(addr, false, false, Some(""), "/mcp").unwrap_err();
        assert!(matches!(err, BindError::TokenRequired));
        let err = validate_http_serve(addr, false, false, Some("   "), "/mcp").unwrap_err();
        assert!(matches!(err, BindError::TokenRequired));
    }

    #[test]
    fn anonymous_rejected_off_loopback() {
        let addr = parse_bind("0.0.0.0:8787").unwrap();
        let err = validate_http_serve(addr, true, true, None, "/mcp").unwrap_err();
        assert!(matches!(err, BindError::AnonymousLoopbackOnly));
    }
}
