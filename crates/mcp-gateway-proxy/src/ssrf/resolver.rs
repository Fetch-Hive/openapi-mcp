//! DNS resolution behind a trait so tests never hit the public internet.

use async_trait::async_trait;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

#[async_trait]
pub trait Resolver: Send + Sync {
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String>;
}

/// Fixture resolver. Classification tests must not perform real public DNS.
#[derive(Debug, Default, Clone)]
pub struct MapResolver {
    pub records: HashMap<String, Vec<IpAddr>>,
}

impl MapResolver {
    pub fn new(records: HashMap<String, Vec<IpAddr>>) -> Self {
        Self { records }
    }

    pub fn single(host: impl Into<String>, ip: IpAddr) -> Self {
        let mut records = HashMap::new();
        records.insert(host.into(), vec![ip]);
        Self { records }
    }
}

#[async_trait]
impl Resolver for MapResolver {
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        self.records
            .get(host)
            .cloned()
            .ok_or_else(|| format!("no fixture records for {host}"))
    }
}

/// Public-resolver Hickory lookup (1.1.1.1 and 8.8.8.8). Never uses the system
/// `/etc/resolv.conf`, so VPC names such as `redis.internal` do not resolve.
pub struct HickoryResolver {
    resolver: hickory_resolver::TokioResolver,
}

impl HickoryResolver {
    pub fn public(servers: &[IpAddr]) -> Result<Self, String> {
        use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
        use hickory_resolver::name_server::TokioConnectionProvider;

        let group = NameServerConfigGroup::from_ips_clear(servers, 53, true);
        let config = ResolverConfig::from_parts(None, vec![], group);
        let mut opts = ResolverOpts::default();
        opts.timeout = Duration::from_secs(2);
        opts.attempts = 1;
        opts.use_hosts_file = hickory_resolver::config::ResolveHosts::Never;
        let resolver = hickory_resolver::Resolver::builder_with_config(
            config,
            TokioConnectionProvider::default(),
        )
        .with_options(opts)
        .build();
        Ok(Self { resolver })
    }
}

#[async_trait]
impl Resolver for HickoryResolver {
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        let response = self
            .resolver
            .lookup_ip(host)
            .await
            .map_err(|e| e.to_string())?;
        Ok(response.iter().collect())
    }
}

/// System `/etc/resolv.conf` resolver. Only compiled for self-host opt-in so the
/// hosted data plane cannot accidentally resolve VPC-internal names.
#[cfg(feature = "self-host")]
pub struct SystemResolver {
    resolver: hickory_resolver::TokioResolver,
}

#[cfg(feature = "self-host")]
impl SystemResolver {
    pub fn new() -> Result<Self, String> {
        use hickory_resolver::name_server::TokioConnectionProvider;
        let resolver = hickory_resolver::Resolver::builder(TokioConnectionProvider::default())
            .map_err(|e| e.to_string())?
            .build();
        Ok(Self { resolver })
    }
}

#[cfg(not(feature = "self-host"))]
pub struct SystemResolver;

#[cfg(not(feature = "self-host"))]
impl SystemResolver {
    pub fn new() -> Result<Self, String> {
        Err("system resolver requires the self-host feature".into())
    }
}

#[cfg(feature = "self-host")]
#[async_trait]
impl Resolver for SystemResolver {
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, String> {
        let response = self
            .resolver
            .lookup_ip(host)
            .await
            .map_err(|e| e.to_string())?;
        Ok(response.iter().collect())
    }
}
