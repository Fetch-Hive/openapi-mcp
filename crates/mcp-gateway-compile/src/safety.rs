//! Outbound safety for spec loading and `$ref` fetch inside this crate.
//!
//! Execute-path SSRF is implemented in `mcp-gateway-proxy` (not by depending on
//! this crate). Phase 1 still connects by hostname after the DNS check (TOCTOU
//! remains). That is accepted for the CLI and forbidden to copy into the hosted
//! dialer.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, Copy, Default)]
pub struct SafetyOpts {
    /// Skip RFC1918 / ULA denials. Loopback and cloud metadata stay denied.
    pub allow_private: bool,
}

#[derive(Debug, Error)]
pub enum SafetyError {
    #[error("only https URLs are allowed, got {0}")]
    NotHttps(String),
    #[error("localhost is not allowed")]
    Localhost,
    #[error("blocked address: {0}")]
    BlockedAddress(String),
    #[error("could not resolve host {0}: {1}")]
    Dns(String, String),
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
}

impl SafetyError {
    pub fn exit_code(&self) -> i32 {
        3
    }
}

pub fn parse_https_url(raw: &str) -> Result<Url, SafetyError> {
    parse_https_url_with(raw, SafetyOpts::default())
}

pub fn parse_https_url_with(raw: &str, opts: SafetyOpts) -> Result<Url, SafetyError> {
    let url = Url::parse(raw).map_err(|e| SafetyError::InvalidUrl(e.to_string()))?;
    assert_https(&url)?;
    check_url_host(&url, opts)?;
    Ok(url)
}

pub fn assert_https(url: &Url) -> Result<(), SafetyError> {
    if url.scheme() != "https" {
        return Err(SafetyError::NotHttps(url.scheme().to_owned()));
    }
    Ok(())
}

fn check_url_host(url: &Url, opts: SafetyOpts) -> Result<(), SafetyError> {
    match url.host() {
        Some(url::Host::Domain(domain)) => check_host_with(domain, opts),
        Some(url::Host::Ipv4(v4)) => {
            if is_blocked_v4(v4, opts) {
                Err(SafetyError::BlockedAddress(v4.to_string()))
            } else {
                Ok(())
            }
        }
        Some(url::Host::Ipv6(v6)) => {
            if is_blocked_v6(v6, opts) {
                Err(SafetyError::BlockedAddress(v6.to_string()))
            } else {
                Ok(())
            }
        }
        None => Err(SafetyError::InvalidUrl("missing host".into())),
    }
}

pub fn check_host(host: &str) -> Result<(), SafetyError> {
    check_host_with(host, SafetyOpts::default())
}

pub fn check_host_with(host: &str, opts: SafetyOpts) -> Result<(), SafetyError> {
    let lower = host.to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return Err(SafetyError::Localhost);
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip_with(ip, opts) {
            return Err(SafetyError::BlockedAddress(host.to_owned()));
        }
    }
    Ok(())
}

/// Resolve DNS and reject if any record is private / metadata.
pub fn resolve_and_check(host: &str) -> Result<(), SafetyError> {
    resolve_and_check_with(host, SafetyOpts::default())
}

pub fn resolve_and_check_with(host: &str, opts: SafetyOpts) -> Result<(), SafetyError> {
    check_host_with(host, opts)?;
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let addrs = (host, 443)
        .to_socket_addrs()
        .map_err(|e| SafetyError::Dns(host.to_owned(), e.to_string()))?;
    let mut any = false;
    for addr in addrs {
        any = true;
        if is_blocked_ip_with(addr.ip(), opts) {
            return Err(SafetyError::BlockedAddress(addr.ip().to_string()));
        }
    }
    if !any {
        return Err(SafetyError::Dns(host.to_owned(), "no addresses".to_owned()));
    }
    Ok(())
}

pub fn is_blocked_ip(ip: IpAddr) -> bool {
    is_blocked_ip_with(ip, SafetyOpts::default())
}

pub fn is_blocked_ip_with(ip: IpAddr, opts: SafetyOpts) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4, opts),
        IpAddr::V6(v6) => is_blocked_v6(v6, opts),
    }
}

fn is_blocked_v4(v4: Ipv4Addr, opts: SafetyOpts) -> bool {
    if v4.is_loopback()
        || v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || v4.octets() == [169, 254, 169, 254]
        || v4.octets()[0] == 0
    {
        return true;
    }
    if opts.allow_private {
        return false;
    }
    v4.is_private()
}

fn is_blocked_v6(v6: Ipv6Addr, opts: SafetyOpts) -> bool {
    if let Some(v4) = embedded_ipv4(v6) {
        return is_blocked_v4(v4, opts);
    }
    if v6.is_loopback()
        || v6.is_multicast()
        || v6.is_unspecified()
        || is_ipv6_link_local(v6)
        || v6.octets() == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254).octets()
    {
        return true;
    }
    if opts.allow_private {
        return false;
    }
    v6.is_unique_local()
}

/// IPv4-mapped (`::ffff:a.b.c.d`) and deprecated IPv4-compatible (`::a.b.c.d`).
fn embedded_ipv4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    let o = v6.octets();
    if o[0..12] == [0; 12] {
        return Some(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
    }
    None
}

fn is_ipv6_link_local(v6: Ipv6Addr) -> bool {
    let segs = v6.segments();
    (segs[0] & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_http() {
        let err = parse_https_url("http://example.com/openapi.json").unwrap_err();
        assert!(matches!(err, SafetyError::NotHttps(_)));
    }

    #[test]
    fn rejects_localhost() {
        let err = parse_https_url("https://localhost/spec.json").unwrap_err();
        assert!(matches!(err, SafetyError::Localhost));
    }

    #[test]
    fn rejects_loopback_literal() {
        let err = parse_https_url("https://127.0.0.1/spec.json").unwrap_err();
        assert!(matches!(err, SafetyError::BlockedAddress(_)));
    }

    #[test]
    fn rejects_metadata_ipv4() {
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn allows_public_https() {
        parse_https_url("https://example.com/openapi.json").unwrap();
    }

    #[test]
    fn rejects_ipv4_compatible_loopback() {
        let ip: IpAddr = "::127.0.0.1".parse().unwrap();
        assert!(is_blocked_ip(ip));
        let err = parse_https_url("https://[::127.0.0.1]/spec.json").unwrap_err();
        assert!(matches!(err, SafetyError::BlockedAddress(_)));
    }

    #[test]
    fn rejects_ipv4_mapped_loopback() {
        let ip: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(is_blocked_ip(ip));
    }

    #[test]
    fn rejects_rfc1918_by_default() {
        let err = parse_https_url("https://10.0.0.1/spec.json").unwrap_err();
        assert!(matches!(err, SafetyError::BlockedAddress(_)));
    }

    #[test]
    fn allow_private_skips_rfc1918_not_metadata() {
        let opts = SafetyOpts {
            allow_private: true,
        };
        parse_https_url_with("https://10.0.0.1/spec.json", opts).unwrap();
        let err = parse_https_url_with("https://169.254.169.254/spec.json", opts).unwrap_err();
        assert!(matches!(err, SafetyError::BlockedAddress(_)));
        let err = parse_https_url_with("https://127.0.0.1/spec.json", opts).unwrap_err();
        assert!(matches!(err, SafetyError::BlockedAddress(_)));
    }
}
