use super::*;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::Url;

fn policy() -> SsrfPolicy {
    SsrfPolicy::default()
}

fn parse(raw: &str) -> Url {
    Url::parse(raw).expect("test URL")
}

async fn pin(raw: &str) -> Result<Pinned, SsrfError> {
    let resolver = MapResolver::default();
    pin_url(&parse(raw), 0, &policy(), &resolver).await
}

async fn pin_with(
    raw: &str,
    resolver: &MapResolver,
    policy: &SsrfPolicy,
) -> Result<Pinned, SsrfError> {
    pin_url(&parse(raw), 0, policy, resolver).await
}

#[test]
fn every_ipv4_cidr_row_is_denied() {
    for (cidr, sample, reason) in IPV4_DENY_ROWS {
        let ip: Ipv4Addr = sample.parse().unwrap_or_else(|_| panic!("sample {sample}"));
        assert!(
            is_blocked_ip(IpAddr::V4(ip)),
            "{cidr} sample {sample} ({reason}) must be denied"
        );
    }
}

#[test]
fn every_ipv6_cidr_row_is_denied_or_unwraps_to_denied() {
    for (cidr, sample, reason) in IPV6_DENY_ROWS {
        let ip: Ipv6Addr = sample.parse().unwrap_or_else(|_| panic!("sample {sample}"));
        assert!(
            is_blocked_ip(IpAddr::V6(ip)),
            "{cidr} sample {sample} ({reason}) must be denied after unwrap"
        );
    }
}

#[test]
fn ipv4_mapped_loopback_is_denied() {
    let ip: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
    assert!(is_blocked_ip(ip));
}

#[test]
fn ipv4_mapped_rfc1918_is_denied() {
    let ip: IpAddr = "::ffff:10.0.0.1".parse().unwrap();
    assert!(is_blocked_ip(ip));
}

#[test]
fn ipv4_mapped_metadata_is_denied() {
    let ip: IpAddr = "::ffff:169.254.169.254".parse().unwrap();
    assert!(is_blocked_ip(ip));
}

#[test]
fn ipv4_compatible_loopback_is_denied() {
    let ip: IpAddr = "::127.0.0.1".parse().unwrap();
    assert!(is_blocked_ip(ip));
}

#[test]
fn nat64_metadata_is_denied() {
    let ip: IpAddr = "64:ff9b::a9fe:a9fe".parse().unwrap();
    assert_eq!(unwrap_ip(ip), IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)));
    assert!(is_blocked_ip(ip));
}

#[test]
fn nat64_local_use_prefix_is_denied() {
    let ip: IpAddr = "64:ff9b:1::".parse().unwrap();
    assert!(is_blocked_ip(ip));
}

#[test]
fn sixto4_rfc1918_is_denied() {
    let ip: IpAddr = "2002:0a00:0001::1".parse().unwrap();
    assert_eq!(unwrap_ip(ip), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
    assert!(is_blocked_ip(ip));
}

#[test]
fn azure_and_aliyun_metadata_are_denied() {
    assert!(is_blocked_ip("168.63.129.16".parse().unwrap()));
    assert!(is_blocked_ip("100.100.100.200".parse().unwrap()));
}

#[tokio::test]
async fn hostname_denylist() {
    let resolver = MapResolver::single("example.com", "1.1.1.1".parse().unwrap());
    for host in [
        "localhost",
        "metadata.google.internal",
        "foo.local",
        "bar.internal",
    ] {
        let url = Url::parse(&format!("https://{host}/")).unwrap();
        let err = pin_url(&url, 0, &policy(), &resolver)
            .await
            .expect_err(host);
        assert!(
            matches!(err, SsrfError::HostnameDenied(_)),
            "{host}: {err:?}"
        );
    }
}

#[tokio::test]
async fn scheme_file_and_gopher_are_denied() {
    for raw in ["file:///etc/passwd", "gopher://example.com/"] {
        let err = pin(raw).await.expect_err(raw);
        assert!(
            matches!(err, SsrfError::SchemeForbidden(_)),
            "{raw}: {err:?}"
        );
    }
}

#[tokio::test]
async fn userinfo_is_denied() {
    let err = pin("https://user:pass@example.com/").await.unwrap_err();
    assert_eq!(err, SsrfError::UserinfoForbidden);
}

#[tokio::test]
async fn forbidden_ports_are_denied() {
    for port in [22_u16, 6379, 5432] {
        let err = pin(&format!("https://example.com:{port}/"))
            .await
            .unwrap_err();
        assert_eq!(err, SsrfError::PortForbidden(port));
    }
}

#[tokio::test]
async fn mixed_dns_any_blocked_denies_all() {
    let mut resolver = MapResolver::default();
    resolver.records.insert(
        "evil.example".into(),
        vec!["8.8.8.8".parse().unwrap(), "10.0.0.1".parse().unwrap()],
    );
    let err = pin_with("https://evil.example/", &resolver, &policy())
        .await
        .unwrap_err();
    assert_eq!(err, SsrfError::ResolvedBlocked);
}

#[tokio::test]
async fn ip_literal_metadata_skips_dns() {
    let resolver = MapResolver::default();
    let err = pin_url(
        &parse("http://169.254.169.254/"),
        0,
        &SsrfPolicy::default().with_allow_insecure_http(true),
        &resolver,
    )
    .await
    .unwrap_err();
    assert_eq!(err, SsrfError::ResolvedBlocked);
    assert!(resolver.records.is_empty());
}

#[tokio::test]
async fn public_a_record_is_allowed() {
    let resolver = MapResolver::single("dns.google", "8.8.8.8".parse().unwrap());
    let pinned = pin_with("https://dns.google/", &resolver, &policy())
        .await
        .expect("public A record must be allowed");
    assert_eq!(pinned.hostname, "dns.google");
    assert_eq!(pinned.addr.ip(), IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
    assert_eq!(pinned.port, 443);
}

#[tokio::test]
async fn http_rejected_unless_allow_insecure() {
    let err = pin("http://dns.google/").await.unwrap_err();
    assert!(matches!(err, SsrfError::SchemeForbidden(_)));
}

#[tokio::test]
async fn hop_overflow_is_denied() {
    let resolver = MapResolver::single("dns.google", "8.8.8.8".parse().unwrap());
    let err = pin_url(&parse("https://dns.google/"), 4, &policy(), &resolver)
        .await
        .unwrap_err();
    assert_eq!(err, SsrfError::TooManyRedirects);
}

#[tokio::test]
async fn loopback_literal_denied_without_test_flag() {
    let err = pin("https://127.0.0.1/").await.unwrap_err();
    assert_eq!(err, SsrfError::ResolvedBlocked);
}

#[test]
fn decimal_ipv4_tricks_are_classified_if_url_parses_them() {
    for raw in ["http://127.1/", "http://0x7f.0.0.1/"] {
        if let Ok(url) = Url::parse(raw) {
            match url.host() {
                Some(url::Host::Ipv4(v4)) => {
                    assert!(
                        is_blocked_ip(IpAddr::V4(v4)),
                        "{raw} parsed as {v4} must be denied"
                    );
                }
                Some(url::Host::Domain(d)) => {
                    if let Ok(ip) = d.parse::<IpAddr>() {
                        assert!(is_blocked_ip(ip), "{raw} domain {d} parsed as IP");
                    }
                }
                _ => {}
            }
        }
    }
}

#[test]
fn public_unicast_is_not_blocked() {
    assert!(!is_blocked_ip("1.1.1.1".parse().unwrap()));
    assert!(!is_blocked_ip("8.8.8.8".parse().unwrap()));
    assert!(!is_blocked_ip("2606:4700:4700::1111".parse().unwrap()));
}

#[test]
fn private_networks_opt_in_is_noop_without_self_host_feature() {
    let policy = SsrfPolicy::default().with_private_networks(true);
    #[cfg(not(feature = "self-host"))]
    assert!(
        !policy.allow_private_networks(),
        "hosted builds must not honor with_private_networks"
    );
    #[cfg(feature = "self-host")]
    assert!(policy.allow_private_networks());
}

#[cfg(not(feature = "self-host"))]
#[tokio::test]
async fn same_module_field_write_cannot_bypass_feature_gate() {
    let mut policy = SsrfPolicy::default().with_allow_insecure_http(true);
    policy.force_private_networks_field(true);
    let err = pin_with("https://10.0.0.1/", &MapResolver::default(), &policy)
        .await
        .unwrap_err();
    assert_eq!(err, SsrfError::ResolvedBlocked);
}

#[cfg(feature = "self-host")]
#[tokio::test]
async fn rfc1918_allowed_when_private_networks_opt_in() {
    let policy = SsrfPolicy::default().with_private_networks(true);
    let resolver = MapResolver::single("api.int.corp", "10.0.0.8".parse().unwrap());
    let pinned = pin_url(&parse("https://api.int.corp/"), 0, &policy, &resolver)
        .await
        .expect("RFC1918 must pass with self-host opt-in");
    assert_eq!(pinned.addr.ip(), IpAddr::V4(Ipv4Addr::new(10, 0, 0, 8)));
}
