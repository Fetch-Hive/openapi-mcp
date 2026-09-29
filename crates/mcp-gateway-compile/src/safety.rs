//! Outbound safety for spec loading and `$ref` fetch inside this crate.
//!
//! Execute-path SSRF is implemented in `mcp-gateway-proxy` (not by depending on
//! this crate). This loader still connects by hostname after the DNS check
//! (TOCTOU remains).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use thiserror::Error;
use url::Url;

#[derive(Debug, Clone, Copy, Default)]
pub struct SafetyOpts {
    /// Skip RFC1918 / ULA / loopback denials. Cloud metadata stays denied.
    pub allow_private: bool,
    /// Allow an `http` spec URL, and only when `allow_private` is also set and
    /// the host is loopback, RFC1918, or IPv6 ULA. Public HTTP stays denied.
    /// Default is `false`, so existing callers stay HTTPS-only.
    pub allow_insecure_http: bool,
}

#[derive(Debug, Error)]
pub enum SafetyError {
    /// Full sentence. Callers match the variant; the text names the scheme and,
    /// for `http`, which opt-in or host check failed.
    #[error("{0}")]
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
    assert_scheme(&url, opts)?;
    check_url_host(&url, opts)?;
    Ok(url)
}

/// `https` is always eligible. `http` is eligible only when both opt-ins are
/// set and every resolved address is loopback, RFC1918, or IPv6 ULA.
/// Any other scheme is refused. Metadata and link-local are not decided here;
/// [`check_url_host`] still denies them after this returns.
fn assert_scheme(url: &Url, opts: SafetyOpts) -> Result<(), SafetyError> {
    match url.scheme() {
        "https" => Ok(()),
        "http" => assert_private_http(url, opts),
        other => Err(SafetyError::NotHttps(format!(
            "only https URLs are allowed, got {other}"
        ))),
    }
}

fn assert_private_http(url: &Url, opts: SafetyOpts) -> Result<(), SafetyError> {
    if let Some(missing) = missing_http_opt_in(opts) {
        return Err(SafetyError::NotHttps(format!(
            "only https URLs are allowed, got http; http needs both --insecure-http and --allow-private-networks and a private or loopback host ({missing})"
        )));
    }
    match http_target_is_private(url)? {
        true => Ok(()),
        false => Err(SafetyError::NotHttps(format!(
            "only https URLs are allowed, got http; host {} is not loopback, RFC1918, or IPv6 ULA",
            url.host_str().unwrap_or_default()
        ))),
    }
}

fn missing_http_opt_in(opts: SafetyOpts) -> Option<&'static str> {
    match (opts.allow_insecure_http, opts.allow_private) {
        (true, true) => None,
        (false, false) => Some("missing --insecure-http and --allow-private-networks"),
        (false, true) => Some("missing --insecure-http"),
        (true, false) => Some("missing --allow-private-networks"),
    }
}

/// Literal addresses are classified directly. Any other hostname is resolved,
/// and every address must be loopback, RFC1918, or IPv6 ULA. One public
/// address, or a failed lookup, refuses the URL.
fn http_target_is_private(url: &Url) -> Result<bool, SafetyError> {
    match url.host() {
        Some(url::Host::Ipv4(v4)) => Ok(is_http_private_or_loopback(IpAddr::V4(v4))),
        Some(url::Host::Ipv6(v6)) => Ok(is_http_private_or_loopback(IpAddr::V6(v6))),
        Some(url::Host::Domain(domain)) => {
            let host = domain.to_owned();
            if let Ok(ip) = host.parse::<IpAddr>() {
                return Ok(is_http_private_or_loopback(ip));
            }
            // Port is unused; `to_socket_addrs` only needs a socket pair to resolve the name.
            let addrs = (host.as_str(), 443)
                .to_socket_addrs()
                .map_err(|e| SafetyError::Dns(host.clone(), e.to_string()))?;
            let mut any = false;
            for addr in addrs {
                any = true;
                if !is_http_private_or_loopback(addr.ip()) {
                    return Ok(false);
                }
            }
            if !any {
                return Err(SafetyError::Dns(host, "no addresses".into()));
            }
            Ok(true)
        }
        None => Err(SafetyError::InvalidUrl("missing host".into())),
    }
}

/// Loopback, RFC1918, or ULA, and not an address that stays blocked when
/// private networks are allowed (link-local, metadata, multicast, unspecified).
/// Derived from [`is_blocked_ip_with`] so the two answers cannot drift.
fn is_http_private_or_loopback(ip: IpAddr) -> bool {
    let closed = SafetyOpts {
        allow_private: false,
        ..SafetyOpts::default()
    };
    let open = SafetyOpts {
        allow_private: true,
        ..SafetyOpts::default()
    };
    is_blocked_ip_with(ip, closed) && !is_blocked_ip_with(ip, open)
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
        if opts.allow_private {
            return Ok(());
        }
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
    if v4.is_link_local()
        || v4.is_multicast()
        || v4.is_broadcast()
        || v4.is_unspecified()
        || v4.octets() == [169, 254, 169, 254]
        || v4.octets()[0] == 0
    {
        return true;
    }
    if v4.is_loopback() {
        return !opts.allow_private;
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
    if v6.is_multicast() || v6.is_unspecified() || is_ipv6_link_local(v6) || is_ipv6_metadata(v6) {
        return true;
    }
    if v6.is_loopback() {
        return !opts.allow_private;
    }
    if opts.allow_private {
        return false;
    }
    v6.is_unique_local()
}

/// IPv4-mapped (`::ffff:a.b.c.d`) and deprecated IPv4-compatible (`::a.b.c.d`).
/// `::` and `::1` are not IPv4-compatible addresses; treating `::1` as
/// `0.0.0.1` would deny loopback even when `allow_private` is set.
fn embedded_ipv4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return Some(v4);
    }
    if v6.is_unspecified() || v6.is_loopback() {
        return None;
    }
    let o = v6.octets();
    if o[0..12] == [0; 12] {
        return Some(Ipv4Addr::new(o[12], o[13], o[14], o[15]));
    }
    None
}

fn is_ipv6_metadata(v6: Ipv6Addr) -> bool {
    v6.octets() == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254).octets()
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
            allow_insecure_http: false,
        };
        parse_https_url_with("https://10.0.0.1/spec.json", opts).unwrap();
        parse_https_url_with("https://127.0.0.1/spec.json", opts).unwrap();
        parse_https_url_with("https://[::1]/spec.json", opts).unwrap();
        check_host_with("localhost", opts).unwrap();
        let err = parse_https_url_with("https://169.254.169.254/spec.json", opts).unwrap_err();
        assert!(matches!(err, SafetyError::BlockedAddress(_)));
        let err = parse_https_url_with("https://[fd00:ec2::254]/spec.json", opts).unwrap_err();
        assert!(matches!(err, SafetyError::BlockedAddress(_)));
    }

    fn http_opts(private: bool, insecure: bool) -> SafetyOpts {
        SafetyOpts {
            allow_private: private,
            allow_insecure_http: insecure,
        }
    }

    #[test]
    fn http_names_the_missing_opt_in() {
        let url = "http://127.0.0.1:8000/openapi.json";
        let neither = parse_https_url(url).unwrap_err().to_string();
        assert!(neither.contains("missing --insecure-http and --allow-private-networks"));
        let private_only = parse_https_url_with(url, http_opts(true, false))
            .unwrap_err()
            .to_string();
        assert!(private_only.contains("missing --insecure-http"));
        assert!(!private_only.contains("missing --allow-private-networks"));
        let http_only = parse_https_url_with(url, http_opts(false, true))
            .unwrap_err()
            .to_string();
        assert!(http_only.contains("missing --allow-private-networks"));
        assert!(!http_only.contains("missing --insecure-http"));
    }

    #[test]
    fn http_private_loopback_allowed_only_with_both_flags() {
        let both = http_opts(true, true);
        parse_https_url_with("http://127.0.0.1:8000/openapi.json", both).unwrap();
        parse_https_url_with("http://10.0.0.5/openapi.json", both).unwrap();
        parse_https_url_with("http://[::1]:8000/openapi.json", both).unwrap();
        // `localhost` resolves on the system resolver. Every address must be
        // loopback (typically 127.0.0.1 and ::1).
        parse_https_url_with("http://localhost:8000/openapi.json", both).unwrap();
    }

    #[test]
    fn http_public_host_rejected_with_both_flags() {
        let literal =
            parse_https_url_with("http://8.8.8.8/openapi.json", http_opts(true, true)).unwrap_err();
        assert!(matches!(literal, SafetyError::NotHttps(_)));
        assert!(literal.to_string().contains("8.8.8.8"));
        let err = parse_https_url_with("http://example.com/openapi.json", http_opts(true, true))
            .unwrap_err();
        match err {
            SafetyError::NotHttps(msg) => assert!(msg.contains("example.com"), "{msg}"),
            SafetyError::Dns(_, _) => {}
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn http_metadata_rejected_with_both_flags() {
        let both = http_opts(true, true);
        let v4 = parse_https_url_with("http://169.254.169.254/spec.json", both).unwrap_err();
        assert!(matches!(v4, SafetyError::NotHttps(_)));
        assert!(v4.to_string().contains("169.254.169.254"));
        let v6 = parse_https_url_with("http://[fd00:ec2::254]/spec.json", both).unwrap_err();
        assert!(
            matches!(
                v6,
                SafetyError::NotHttps(_) | SafetyError::BlockedAddress(_)
            ),
            "{v6:?}"
        );
    }

    #[test]
    fn rejects_non_http_schemes() {
        let err = parse_https_url("file:///tmp/openapi.json").unwrap_err();
        assert!(matches!(err, SafetyError::NotHttps(_)));
        assert!(err.to_string().contains("got file"));
    }
}
