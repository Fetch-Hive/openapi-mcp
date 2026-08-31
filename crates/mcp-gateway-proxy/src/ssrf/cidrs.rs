//! IANA special-purpose CIDR tables plus cloud-metadata extras.
//!
//! Sources (Phase 2 spec §4.8.2–4.8.4):
//! IANA IPv4 Special-Purpose Address Registry (2025-10-09),
//! IANA IPv6 Special-Purpose Address Registry (2025-10-09),
//! plus Azure Wire Server, Aliyun metadata, and AWS IMDSv6.

use ipnet::{Ipv4Net, Ipv6Net};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

/// (CIDR, sample address used by the mandatory per-row test, reason).
pub const IPV4_DENY_ROWS: &[(&str, &str, &str)] = &[
    ("0.0.0.0/8", "0.0.0.0", "this network"),
    ("0.0.0.0/32", "0.0.0.0", "this host on this network"),
    ("10.0.0.0/8", "10.0.0.0", "RFC 1918 private"),
    ("100.64.0.0/10", "100.64.0.0", "CGNAT / Tailscale"),
    ("127.0.0.0/8", "127.0.0.0", "loopback"),
    (
        "169.254.0.0/16",
        "169.254.0.0",
        "link-local / cloud metadata",
    ),
    ("172.16.0.0/12", "172.16.0.0", "RFC 1918 private"),
    ("192.0.0.0/24", "192.0.0.0", "IETF protocol assignments"),
    ("192.0.2.0/24", "192.0.2.0", "TEST-NET-1"),
    ("192.88.99.0/24", "192.88.99.0", "deprecated 6to4 relay"),
    ("192.168.0.0/16", "192.168.0.0", "RFC 1918 private"),
    ("198.18.0.0/15", "198.18.0.0", "benchmarking"),
    ("198.51.100.0/24", "198.51.100.0", "TEST-NET-2"),
    ("203.0.113.0/24", "203.0.113.0", "TEST-NET-3"),
    ("224.0.0.0/4", "224.0.0.0", "multicast"),
    ("240.0.0.0/4", "240.0.0.0", "reserved"),
    ("255.255.255.255/32", "255.255.255.255", "limited broadcast"),
    ("168.63.129.16/32", "168.63.129.16", "Azure Wire Server"),
    ("100.100.100.200/32", "100.100.100.200", "Aliyun metadata"),
];

pub const IPV6_DENY_ROWS: &[(&str, &str, &str)] = &[
    ("::1/128", "::1", "loopback"),
    ("::/128", "::", "unspecified"),
    ("::ffff:0:0/96", "::ffff:0:0", "IPv4-mapped (unwrap)"),
    ("64:ff9b::/96", "64:ff9b::", "NAT64 well-known (unwrap)"),
    ("64:ff9b:1::/48", "64:ff9b:1::", "NAT64 local-use"),
    ("100::/64", "100::", "discard-only"),
    ("100:0:0:1::/64", "100:0:0:1::", "dummy IPv6 prefix"),
    ("2001::/23", "2001::", "IETF protocol assignments"),
    ("2001::/32", "2001::", "Teredo"),
    ("2001:2::/48", "2001:2::", "benchmarking"),
    ("2001:db8::/32", "2001:db8::", "documentation"),
    ("2002::/16", "2002::", "6to4 (unwrap)"),
    ("3fff::/20", "3fff::", "documentation"),
    ("5f00::/16", "5f00::", "SRv6 SIDs"),
    ("fc00::/7", "fc00::", "unique-local"),
    ("fe80::/10", "fe80::", "link-local unicast"),
    ("ff00::/8", "ff00::", "multicast"),
    ("fec0::/10", "fec0::", "deprecated site-local"),
    ("fd00:ec2::254/128", "fd00:ec2::254", "AWS IMDSv6"),
];

fn v4_nets() -> &'static [Ipv4Net] {
    static NETS: OnceLock<Vec<Ipv4Net>> = OnceLock::new();
    NETS.get_or_init(|| {
        IPV4_DENY_ROWS
            .iter()
            .map(|(cidr, _, _)| cidr.parse::<Ipv4Net>().expect("valid IPv4 CIDR"))
            .collect()
    })
}

fn v6_nets() -> &'static [Ipv6Net] {
    static NETS: OnceLock<Vec<Ipv6Net>> = OnceLock::new();
    NETS.get_or_init(|| {
        IPV6_DENY_ROWS
            .iter()
            .map(|(cidr, _, _)| cidr.parse::<Ipv6Net>().expect("valid IPv6 CIDR"))
            .collect()
    })
}

pub fn ipv4_denied(addr: Ipv4Addr) -> bool {
    v4_nets().iter().any(|net| net.contains(&addr))
}

pub fn ipv6_denied(addr: Ipv6Addr) -> bool {
    v6_nets().iter().any(|net| net.contains(&addr))
}

pub fn is_metadata_v4(addr: Ipv4Addr) -> bool {
    let o = addr.octets();
    (o[0] == 169 && o[1] == 254 && o[2] == 169 && o[3] == 254)
        || addr == Ipv4Addr::new(168, 63, 129, 16)
        || addr == Ipv4Addr::new(100, 100, 100, 200)
}

pub fn is_metadata_v6(addr: Ipv6Addr) -> bool {
    addr == "fd00:ec2::254".parse::<Ipv6Addr>().expect("imdsv6")
}

pub fn is_self_host_private_v4(addr: Ipv4Addr) -> bool {
    addr.is_private() || {
        let o = addr.octets();
        o[0] == 100 && o[1] >= 64 && o[1] <= 127
    }
}

pub fn is_self_host_private_v6(addr: Ipv6Addr) -> bool {
    let segs = addr.segments();
    (segs[0] & 0xfe00) == 0xfc00
}
