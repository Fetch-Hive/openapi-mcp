//! Ordered URL classification. No I/O.

use super::cidrs::{ipv4_denied, ipv6_denied};
use super::resolver::Resolver;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use thiserror::Error;
use url::Url;

const ALLOWED_PORTS: &[u16] = &[80, 443, 8443];
const MAX_HOST_LEN: usize = 253;

const HOST_EXACT_DENY: &[&str] = &[
    "localhost",
    "localhost.localdomain",
    "ip6-localhost",
    "ip6-loopback",
    "metadata.google.internal",
    "metadata.goog",
    "instance-data",
    "instance-data.ec2.internal",
    "kubernetes",
    "kubernetes.default",
    "kubernetes.default.svc",
    "kubernetes.default.svc.cluster.local",
    "internal",
    "intranet",
    "corp",
    "private",
];

const HOST_SUFFIX_DENY: &[&str] = &[
    ".localhost",
    ".local",
    ".internal",
    ".corp",
    ".home",
    ".lan",
    ".invalid",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pinned {
    pub hostname: String,
    pub scheme: String,
    pub port: u16,
    pub addr: SocketAddr,
}

#[derive(Debug, Clone)]
pub struct SsrfPolicy {
    allow_insecure_http: bool,
    max_hops: u8,
    /// Test-only: treat loopback as allowed so a local echo server can be dialed.
    /// Production always leaves this false.
    allow_loopback: bool,
    /// Self-host opt-in: skip RFC1918 / ULA / CGNAT denials. Honored only when
    /// the `self-host` Cargo feature is enabled; classification ignores the
    /// stored flag otherwise.
    allow_private_networks: bool,
    /// Hidden self-host opt-in: also allow cloud-metadata CIDRs. Honored only
    /// with the `self-host` Cargo feature.
    allow_metadata: bool,
}

impl Default for SsrfPolicy {
    fn default() -> Self {
        Self {
            allow_insecure_http: false,
            max_hops: 3,
            allow_loopback: false,
            allow_private_networks: false,
            allow_metadata: false,
        }
    }
}

impl SsrfPolicy {
    pub fn allow_insecure_http(&self) -> bool {
        self.allow_insecure_http
    }

    pub fn max_hops(&self) -> u8 {
        self.max_hops
    }

    pub fn allow_loopback(&self) -> bool {
        self.allow_loopback
    }

    /// RFC1918/ULA/CGNAT opt-in. Always `false` without `self-host`.
    pub fn allow_private_networks(&self) -> bool {
        #[cfg(feature = "self-host")]
        {
            self.allow_private_networks
        }
        #[cfg(not(feature = "self-host"))]
        {
            false
        }
    }

    /// Cloud-metadata opt-in. Always `false` without `self-host`.
    pub fn allow_metadata(&self) -> bool {
        #[cfg(feature = "self-host")]
        {
            self.allow_metadata
        }
        #[cfg(not(feature = "self-host"))]
        {
            false
        }
    }

    pub fn with_allow_insecure_http(mut self, enabled: bool) -> Self {
        self.allow_insecure_http = enabled;
        self
    }

    pub fn with_max_hops(mut self, hops: u8) -> Self {
        self.max_hops = hops;
        self
    }

    pub fn with_allow_loopback(mut self, enabled: bool) -> Self {
        self.allow_loopback = enabled;
        self
    }

    /// Enable RFC1918/ULA/CGNAT. No-op unless the `self-host` feature is enabled.
    pub fn with_private_networks(mut self, enabled: bool) -> Self {
        #[cfg(feature = "self-host")]
        {
            self.allow_private_networks = enabled;
        }
        #[cfg(not(feature = "self-host"))]
        {
            let _ = enabled;
            self.allow_private_networks = false;
        }
        self
    }

    pub fn with_metadata(mut self, enabled: bool) -> Self {
        #[cfg(feature = "self-host")]
        {
            self.allow_metadata = enabled;
        }
        #[cfg(not(feature = "self-host"))]
        {
            let _ = enabled;
            self.allow_metadata = false;
        }
        self
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(super) fn force_private_networks_field(&mut self, enabled: bool) {
        self.allow_private_networks = enabled;
    }
}

fn private_networks_allowed(policy: &SsrfPolicy) -> bool {
    policy.allow_private_networks()
}

fn metadata_allowed(policy: &SsrfPolicy) -> bool {
    policy.allow_metadata()
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SsrfError {
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("scheme not allowed: {0}")]
    SchemeForbidden(String),
    #[error("userinfo is forbidden")]
    UserinfoForbidden,
    #[error("port not allowed: {0}")]
    PortForbidden(u16),
    #[error("hostname not allowed: {0}")]
    HostnameDenied(String),
    #[error("DNS resolution failed for {0}")]
    DnsFailed(String),
    #[error("resolved address is not allowed")]
    ResolvedBlocked,
    #[error("address is not allowed")]
    AddressBlocked,
}

impl SsrfError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::TooManyRedirects => "too_many_redirects",
            Self::InvalidUrl(_) => "invalid_url",
            Self::SchemeForbidden(_) => "scheme_forbidden",
            Self::UserinfoForbidden => "userinfo_forbidden",
            Self::PortForbidden(_) => "port_forbidden",
            Self::HostnameDenied(_) => "hostname_denied",
            Self::DnsFailed(_) => "dns_failed",
            Self::ResolvedBlocked => "resolved_blocked",
            Self::AddressBlocked => "address_blocked",
        }
    }
}

/// Classify `url` and pin a single allowed `SocketAddr`. DNS is used only for
/// name hosts via `resolver`. IP literals never call DNS.
pub async fn pin_url(
    url: &Url,
    hop: u8,
    policy: &SsrfPolicy,
    resolver: &dyn Resolver,
) -> Result<Pinned, SsrfError> {
    if hop > policy.max_hops() {
        return Err(SsrfError::TooManyRedirects);
    }
    match url.scheme() {
        "https" => {}
        "http" if policy.allow_insecure_http() => {}
        other => return Err(SsrfError::SchemeForbidden(other.to_owned())),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SsrfError::UserinfoForbidden);
    }

    let port = url
        .port()
        .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
    if !ALLOWED_PORTS.contains(&port) && !policy.allow_loopback() {
        return Err(SsrfError::PortForbidden(port));
    }

    let hostname = normalize_hostname(url)?;
    if hostname_denied(&hostname, policy) {
        return Err(SsrfError::HostnameDenied(hostname));
    }

    let addrs = if let Ok(ip) = hostname.parse::<IpAddr>() {
        vec![ip]
    } else {
        let resolved = resolver
            .resolve(&hostname)
            .await
            .map_err(|e| SsrfError::DnsFailed(format!("{hostname}: {e}")))?;
        if resolved.is_empty() {
            return Err(SsrfError::DnsFailed(hostname));
        }
        resolved
    };

    let mut allowed: Vec<IpAddr> = Vec::new();
    for addr in &addrs {
        if is_blocked_ip_with_policy(*addr, policy) {
            return Err(SsrfError::ResolvedBlocked);
        }
        allowed.push(*addr);
    }
    if allowed.is_empty() {
        return Err(SsrfError::ResolvedBlocked);
    }

    let chosen = pick_addr(&allowed);
    Ok(Pinned {
        hostname,
        scheme: url.scheme().to_owned(),
        port,
        addr: SocketAddr::new(chosen, port),
    })
}

fn normalize_hostname(url: &Url) -> Result<String, SsrfError> {
    let host = url
        .host()
        .ok_or_else(|| SsrfError::InvalidUrl("missing host".into()))?;
    match host {
        url::Host::Ipv4(v4) => Ok(v4.to_string()),
        url::Host::Ipv6(v6) => Ok(v6.to_string()),
        url::Host::Domain(domain) => {
            let ascii = idna::domain_to_ascii(domain)
                .map_err(|e| SsrfError::InvalidUrl(format!("idna: {e}")))?;
            let round = idna::domain_to_unicode(&ascii).0;
            let original_lower = domain.to_ascii_lowercase();
            let round_ascii = idna::domain_to_ascii(&round)
                .map_err(|e| SsrfError::InvalidUrl(format!("idna round-trip: {e}")))?;
            if round_ascii != ascii && original_lower != ascii {
                return Err(SsrfError::InvalidUrl("idna round-trip mismatch".into()));
            }
            let mut host = ascii.to_ascii_lowercase();
            if let Some(stripped) = host.strip_suffix('.') {
                if stripped.ends_with('.') {
                    return Err(SsrfError::InvalidUrl("double trailing dot".into()));
                }
                host = stripped.to_owned();
            }
            if host.is_empty() {
                return Err(SsrfError::InvalidUrl("empty host".into()));
            }
            if host.len() > MAX_HOST_LEN {
                return Err(SsrfError::InvalidUrl("host too long".into()));
            }
            Ok(host)
        }
    }
}

const METADATA_HOSTS: &[&str] = &[
    "metadata.google.internal",
    "metadata.goog",
    "instance-data",
    "instance-data.ec2.internal",
];

fn hostname_denied(host: &str, policy: &SsrfPolicy) -> bool {
    if METADATA_HOSTS.contains(&host) && !metadata_allowed(policy) {
        return true;
    }
    if private_networks_allowed(policy) {
        if host == "localhost"
            || host == "localhost.localdomain"
            || host == "ip6-localhost"
            || host == "ip6-loopback"
        {
            return !policy.allow_loopback();
        }
        return false;
    }
    if HOST_EXACT_DENY.contains(&host) {
        return true;
    }
    HOST_SUFFIX_DENY.iter().any(|sfx| host.ends_with(sfx))
}

pub fn is_blocked_ip(ip: IpAddr) -> bool {
    is_blocked_ip_with_policy(ip, &SsrfPolicy::default())
}

fn is_blocked_ip_with_policy(ip: IpAddr, policy: &SsrfPolicy) -> bool {
    match unwrap_ip(ip) {
        IpAddr::V4(v4) => {
            if policy.allow_loopback() && v4.is_loopback() {
                return false;
            }
            if super::cidrs::is_metadata_v4(v4) && !metadata_allowed(policy) {
                return true;
            }
            if private_networks_allowed(policy) && super::cidrs::is_self_host_private_v4(v4) {
                return false;
            }
            ipv4_denied(v4)
        }
        IpAddr::V6(v6) => {
            if policy.allow_loopback() && v6.is_loopback() {
                return false;
            }
            if super::cidrs::is_metadata_v6(v6) && !metadata_allowed(policy) {
                return true;
            }
            if private_networks_allowed(policy) && super::cidrs::is_self_host_private_v6(v6) {
                return false;
            }
            ipv6_denied(v6)
        }
    }
}

/// Unwrap transition embeddings until stable, then return the canonical form.
pub fn unwrap_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => unwrap_v6(v6),
    }
}

fn unwrap_v6(v6: Ipv6Addr) -> IpAddr {
    let segs = v6.segments();

    if let Some(v4) = v6.to_ipv4_mapped() {
        return IpAddr::V4(v4);
    }

    // Deprecated IPv4-compatible ::/96 except :: and ::1.
    if segs[0] == 0
        && segs[1] == 0
        && segs[2] == 0
        && segs[3] == 0
        && segs[4] == 0
        && segs[5] == 0
        && v6 != Ipv6Addr::UNSPECIFIED
        && v6 != Ipv6Addr::LOCALHOST
    {
        return IpAddr::V4(Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            (segs[6] & 0xff) as u8,
            (segs[7] >> 8) as u8,
            (segs[7] & 0xff) as u8,
        ));
    }

    // NAT64 well-known 64:ff9b::/96 — unwrap embedded IPv4.
    if segs[0] == 0x64
        && segs[1] == 0xff9b
        && segs[2] == 0
        && segs[3] == 0
        && segs[4] == 0
        && segs[5] == 0
    {
        return IpAddr::V4(Ipv4Addr::new(
            (segs[6] >> 8) as u8,
            (segs[6] & 0xff) as u8,
            (segs[7] >> 8) as u8,
            (segs[7] & 0xff) as u8,
        ));
    }

    // 6to4 2002::/16 — IPv4 in bits 16–47.
    if segs[0] == 0x2002 {
        return IpAddr::V4(Ipv4Addr::new(
            (segs[1] >> 8) as u8,
            (segs[1] & 0xff) as u8,
            (segs[2] >> 8) as u8,
            (segs[2] & 0xff) as u8,
        ));
    }

    IpAddr::V6(v6)
}

fn pick_addr(addrs: &[IpAddr]) -> IpAddr {
    addrs
        .iter()
        .copied()
        .find(IpAddr::is_ipv6)
        .unwrap_or(addrs[0])
}
