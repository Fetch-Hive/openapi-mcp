use crate::cli::Globals;
use crate::config::{GatewayConfig, SpecEntry, UpstreamAuth};
use crate::ir_cache;
use crate::paths::PlatformPaths;
use crate::secrets::SecretRef;
use crate::CliError;
use mcp_gateway_ir::CompileBundle;
use mcp_gateway_proxy::ssrf::{HickoryResolver, Resolver, SystemResolver};
use mcp_gateway_proxy::{InjectedCredential, SsrfPolicy};
use mcp_gateway_server::{GatewayHandler, LiveExecutor, LocalGateway};
use secrecy::{ExposeSecret, SecretString};
use std::net::IpAddr;
use std::sync::Arc;

pub fn ssrf_policy(globals: &Globals, cfg: &GatewayConfig, allow_insecure: bool) -> SsrfPolicy {
    let mut policy = SsrfPolicy::default()
        .with_private_networks(globals.allow_private_networks || cfg.ssrf.allow_private_networks)
        .with_metadata(globals.allow_metadata || cfg.ssrf.allow_metadata)
        .with_allow_insecure_http(allow_insecure || cfg.ssrf.allow_insecure_http);
    #[cfg(debug_assertions)]
    if std::env::var("MCP_GATEWAY_TEST_ALLOW_LOOPBACK").as_deref() == Ok("1") {
        policy = policy
            .with_allow_loopback(true)
            .with_allow_insecure_http(true);
    }
    policy
}

pub fn resolver(policy: &SsrfPolicy) -> Result<Arc<dyn Resolver>, CliError> {
    if policy.allow_private_networks() {
        let sys =
            SystemResolver::new().map_err(|e| CliError::policy(format!("system resolver: {e}")))?;
        Ok(Arc::new(sys))
    } else {
        let public: [IpAddr; 2] = [
            "1.1.1.1".parse().expect("cloudflare dns"),
            "8.8.8.8".parse().expect("google dns"),
        ];
        let h = HickoryResolver::public(&public)
            .map_err(|e| CliError::io(format!("public resolver: {e}")))?;
        Ok(Arc::new(h))
    }
}

pub fn load_bundle_for_spec(
    paths: &PlatformPaths,
    cfg: &GatewayConfig,
    spec: &SpecEntry,
) -> Result<(CompileBundle, std::path::PathBuf), CliError> {
    let cache = if cfg.cache.dir.is_empty() {
        paths.cache_dir.clone()
    } else {
        std::path::PathBuf::from(&cfg.cache.dir)
    };
    let path = ir_cache::find_cached(&cache, &spec.name).ok_or_else(|| {
        CliError::usage(format!(
            "no IR cache for {}; run mcp-gateway add-spec --name {}",
            spec.name, spec.name
        ))
    })?;
    let bundle = ir_cache::load_bundle(&path)?;
    if bundle.api.ir_version != mcp_gateway_ir::IR_VERSION {
        return Err(CliError::usage(format!(
            "IR version {} is not supported",
            bundle.api.ir_version
        )));
    }
    Ok((bundle, path))
}

pub fn base_url(bundle: &CompileBundle, override_url: Option<&str>) -> Result<String, CliError> {
    if let Some(url) = override_url {
        return Ok(url.to_owned());
    }
    let server = bundle
        .api
        .servers
        .first()
        .ok_or_else(|| CliError::usage("IR has no servers; pass --base-url"))?;
    let mut url = server.url_template.clone();
    for (name, var) in &server.variables {
        if let Some(default) = &var.default {
            url = url.replace(&format!("{{{name}}}"), default);
        }
    }
    Ok(url)
}

pub fn injected_credential(
    auth: Option<&UpstreamAuth>,
) -> Result<Option<InjectedCredential>, CliError> {
    let Some(auth) = auth else {
        return Ok(None);
    };
    Ok(Some(build_credential(auth)?))
}

pub fn build_credential(auth: &UpstreamAuth) -> Result<InjectedCredential, CliError> {
    match auth.kind.as_str() {
        "none" => Ok(InjectedCredential::none()),
        "bearer" => Ok(InjectedCredential::bearer(require_token(auth)?)),
        "basic" => {
            let user = auth
                .username
                .as_ref()
                .ok_or_else(|| CliError::usage("basic auth requires username"))?
                .resolve()?;
            let pass = auth
                .password
                .as_ref()
                .ok_or_else(|| CliError::usage("basic auth requires password"))?
                .resolve()?;
            let packed = format!("{}:{}", owned_secret(&user), owned_secret(&pass));
            Ok(InjectedCredential::basic(SecretString::from(packed)))
        }
        "api_key_header" => Ok(InjectedCredential::api_key_header(
            require_token(auth)?,
            auth.header.clone().unwrap_or_else(|| "X-API-Key".into()),
        )),
        "api_key_query" => Ok(InjectedCredential::api_key_query(
            require_token(auth)?,
            auth.query.clone().unwrap_or_else(|| "api_key".into()),
        )),
        "custom_headers" => {
            let mut items = Vec::new();
            for h in &auth.headers {
                items.push(serde_json::json!({
                    "name": h.name,
                    "value": owned_secret(&h.value.resolve()?),
                }));
            }
            Ok(InjectedCredential::custom_headers(SecretString::from(
                serde_json::to_string(&items).unwrap_or_else(|_| "[]".into()),
            )))
        }
        other => Err(CliError::usage(format!("unknown auth type {other}"))),
    }
}

fn require_token(auth: &UpstreamAuth) -> Result<SecretString, CliError> {
    auth.token
        .as_ref()
        .ok_or_else(|| CliError::usage("auth token reference is required"))?
        .resolve()
}

fn owned_secret(s: &SecretString) -> String {
    s.expose_secret().to_owned()
}

fn nonempty_secret(s: SecretString) -> Option<String> {
    let owned = owned_secret(&s);
    let trimmed = owned.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(owned)
    }
}

pub fn handler_for(
    globals: &Globals,
    cfg: &GatewayConfig,
    spec: &SpecEntry,
    paths: &PlatformPaths,
    allow_insecure: bool,
    base_override: Option<&str>,
) -> Result<GatewayHandler, CliError> {
    let (bundle, _) = load_bundle_for_spec(paths, cfg, spec)?;
    let policy = ssrf_policy(globals, cfg, allow_insecure);
    let resolver = resolver(&policy)?;
    let base = {
        #[cfg(debug_assertions)]
        {
            std::env::var("MCP_GATEWAY_TEST_BASE_URL")
                .ok()
                .or_else(|| base_override.map(ToOwned::to_owned))
        }
        #[cfg(not(debug_assertions))]
        {
            base_override.map(ToOwned::to_owned)
        }
    };
    let gateway = Arc::new(LocalGateway {
        base_url: base_url(&bundle, base.as_deref())?,
        credential: injected_credential(spec.upstream.as_ref())?,
        ssrf: policy,
        enabled_tools: spec.enabled_tools.clone(),
        disabled_tools: spec.disabled_tools.clone(),
        bundle: Arc::new(bundle),
    });
    Ok(GatewayHandler::new(
        gateway,
        Arc::new(LiveExecutor { resolver }),
    ))
}

pub fn mcp_token(
    cfg: &GatewayConfig,
    token_file: Option<&std::path::Path>,
) -> Result<Option<String>, CliError> {
    if let Some(path) = token_file {
        return Ok(nonempty_secret(
            SecretRef::file(path.display().to_string()).resolve()?,
        ));
    }
    match &cfg.server.token {
        Some(r) => match r.resolve() {
            Ok(s) => Ok(nonempty_secret(s)),
            Err(_) if cfg.server.allow_anonymous => Ok(None),
            Err(e) => Err(e),
        },
        None => Ok(None),
    }
}
