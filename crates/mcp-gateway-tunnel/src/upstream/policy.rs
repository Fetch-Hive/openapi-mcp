//! Which upstream addresses `mcp-gateway tunnel` may dial.
//!
//! This is the inverse of the outbound proxy's default SSRF policy. The proxy
//! refuses loopback unless a self-host flag is on, and it only allows ports
//! 80, 443, and 8443. A tunnel into a local MCP server allows loopback and
//! RFC1918/ULA on any port, and refuses a public address unless
//! `--allow-remote-upstream` is set.
//!
//! [`mcp_gateway_proxy::ssrf::is_blocked_ip`] still refuses metadata,
//! documentation, and benchmarking ranges. Those stay refused when the flag
//! is set. Metadata is checked first, including AWS IMDSv6 `fd00:ec2::254`,
//! which is inside the ULA range and would otherwise look local.
//!
//! The check runs once, at startup, against every address the system
//! resolver returns. If any address is refused, the URL is refused. The
//! first address in that list is then pinned for the process. Later DNS
//! answers are not used.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use mcp_gateway_proxy::ssrf::{is_blocked_ip, is_metadata_v4, is_metadata_v6, unwrap_ip};
use url::Url;

#[derive(Debug)]
pub enum PinError {
    /// The operator can change the URL or pass a flag. Exit 1.
    Refused(String),
    /// The name did not resolve. Exit 4.
    Resolve(String),
}

impl std::fmt::Display for PinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(message) | Self::Resolve(message) => f.write_str(message),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PinnedUpstream {
    pub url: Url,
    pub addr: SocketAddr,
    /// Host sent as `Host` and passed to reqwest's DNS override. No brackets.
    pub resolve_host: String,
    pub host_header: String,
}

/// `Ok` for loopback, RFC1918, and IPv6 ULA. Public addresses need
/// `allow_remote`. Metadata and other denylist ranges are refused either way.
pub fn address_allowed(ip: IpAddr, allow_remote: bool) -> Result<(), String> {
    let ip = unwrap_ip(ip);
    if is_metadata(ip) {
        return Err(format!(
            "upstream address {ip} is blocked (metadata, documentation, or another non-global range)"
        ));
    }
    if is_local(ip) {
        return Ok(());
    }
    if is_blocked_ip(ip) {
        return Err(format!(
            "upstream address {ip} is blocked (metadata, documentation, or another non-global range)"
        ));
    }
    if allow_remote {
        Ok(())
    } else {
        Err(format!(
            "upstream address {ip} is public; pass --allow-remote-upstream to allow it"
        ))
    }
}

pub async fn pin_upstream(raw: &str, allow_remote: bool) -> Result<PinnedUpstream, PinError> {
    let url =
        Url::parse(raw).map_err(|err| PinError::Refused(format!("invalid upstream URL: {err}")))?;
    match url.scheme() {
        "http" | "https" => {}
        other => {
            return Err(PinError::Refused(format!(
                "upstream scheme {other} is not allowed; use http or https"
            )));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PinError::Refused(
            "upstream URL must not contain a username or password".into(),
        ));
    }
    let (resolve_host, ip_literal) = match url.host() {
        Some(url::Host::Domain(domain)) => (domain.to_owned(), None),
        Some(url::Host::Ipv4(ip)) => (ip.to_string(), Some(IpAddr::V4(ip))),
        Some(url::Host::Ipv6(ip)) => (ip.to_string(), Some(IpAddr::V6(ip))),
        None => return Err(PinError::Refused("upstream URL is missing a host".into())),
    };
    let port = url
        .port_or_known_default()
        .ok_or_else(|| PinError::Refused("upstream URL is missing a port".into()))?;
    if port == 0 {
        return Err(PinError::Refused("upstream port 0 is not allowed".into()));
    }

    let ips = if let Some(ip) = ip_literal {
        vec![ip]
    } else {
        let looked = tokio::net::lookup_host((resolve_host.as_str(), port))
            .await
            .map_err(|err| PinError::Resolve(format!("could not resolve {resolve_host}: {err}")))?;
        let ips: Vec<IpAddr> = looked.map(|addr| addr.ip()).collect();
        if ips.is_empty() {
            return Err(PinError::Resolve(format!(
                "could not resolve {resolve_host}: no addresses"
            )));
        }
        ips
    };

    for ip in &ips {
        if let Err(message) = address_allowed(*ip, allow_remote) {
            return Err(PinError::Refused(message));
        }
    }
    let chosen = ips[0];
    Ok(PinnedUpstream {
        host_header: host_header(&url, &resolve_host, port),
        resolve_host,
        addr: SocketAddr::new(chosen, port),
        url,
    })
}

fn host_header(url: &Url, host: &str, port: u16) -> String {
    let default_port = if url.scheme() == "https" { 443 } else { 80 };
    if port == default_port && url.port().is_none() {
        return host.to_owned();
    }
    if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn is_metadata(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_metadata_v4(v4),
        IpAddr::V6(v6) => is_metadata_v6(v6),
    }
}

fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || is_ula(v6),
    }
}

fn is_ula(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn local_addresses_are_allowed_and_public_ones_need_the_flag() {
        assert!(address_allowed(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), false).is_ok());
        assert!(address_allowed(IpAddr::V4(Ipv4Addr::new(10, 1, 1, 1)), false).is_ok());
        assert!(address_allowed(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 9)), false).is_ok());
        let public = address_allowed(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), false).unwrap_err();
        assert!(public.contains("--allow-remote-upstream"), "{public}");
        assert!(address_allowed(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), true).is_ok());
        let metadata =
            address_allowed(IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)), true).unwrap_err();
        assert!(metadata.contains("blocked"), "{metadata}");
        let ula: Ipv6Addr = "fd00::1".parse().unwrap();
        assert!(address_allowed(IpAddr::V6(ula), false).is_ok());
        let imds_v6: Ipv6Addr = "fd00:ec2::254".parse().unwrap();
        let imds = address_allowed(IpAddr::V6(imds_v6), true).unwrap_err();
        assert!(imds.contains("blocked"), "{imds}");
    }

    #[tokio::test]
    async fn pin_allows_loopback_on_a_high_port_and_refuses_a_public_ip() {
        let pinned = pin_upstream("http://127.0.0.1:9/mcp", false).await.unwrap();
        assert_eq!(pinned.addr.port(), 9);
        assert_eq!(pinned.host_header, "127.0.0.1:9");
        let refused = pin_upstream("http://1.1.1.1/mcp", false).await.unwrap_err();
        assert!(refused.to_string().contains("--allow-remote-upstream"));
        let userinfo = pin_upstream("http://user:secret@127.0.0.1/mcp", false)
            .await
            .unwrap_err();
        assert!(userinfo.to_string().contains("username"));
        let scheme = pin_upstream("ftp://127.0.0.1/mcp", false)
            .await
            .unwrap_err();
        assert!(scheme.to_string().contains("scheme"));
    }
}
