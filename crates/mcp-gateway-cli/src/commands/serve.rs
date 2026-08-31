use super::{load_cfg, spec};
use crate::cli::Globals;
use crate::config::GatewayConfig;
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::runtime::{handler_for, mcp_token};
use crate::CliError;
use mcp_gateway_server::{
    parse_bind, serve_http, serve_stdio, validate_http_serve, HttpServeOptions,
};
use tracing_subscriber::EnvFilter;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    paths: &PlatformPaths,
    globals: &Globals,
    out: &Output,
    name: String,
    stdio: bool,
    bind: Option<String>,
    path: String,
    expose: bool,
    allow_anonymous: bool,
    token_file: Option<std::path::PathBuf>,
    allow_insecure_http: bool,
    base_url: Option<String>,
    url: Option<String>,
) -> Result<ExitCode, CliError> {
    bootstrap_spec(
        paths,
        globals,
        out,
        &name,
        url,
        base_url.clone(),
        allow_insecure_http,
    )
    .await?;
    let cfg = load_cfg(paths)?;
    init_tracing(&cfg.log.level);
    let spec = cfg.spec(&name)?.clone();
    let handler = handler_for(
        globals,
        &cfg,
        &spec,
        paths,
        allow_insecure_http,
        base_url.as_deref(),
    )?;
    let tools = handler.gateway.operations().count();
    let allow_private = globals.allow_private_networks || cfg.ssrf.allow_private_networks;
    let upstream = handler.gateway.base_url.clone();
    if stdio {
        out.err_line(&format!(
            "{} {} stdio  spec={name} tools={tools} ssrf={}",
            out.bold("mcp-gateway"),
            env!("CARGO_PKG_VERSION"),
            if allow_private {
                "private-networks"
            } else {
                "public-internet"
            }
        ));
        serve_stdio(handler)
            .await
            .map_err(|e| CliError::io(e.to_string()))?;
        return Ok(ExitCode::Ok);
    }

    let port_env = std::env::var("PORT").ok();
    let (bind_raw, expose) = resolve_http_bind(
        bind.as_deref(),
        &cfg.server.bind,
        expose || cfg.server.expose,
        port_env.as_deref(),
    )?;
    let addr = parse_bind(&bind_raw).map_err(|e| CliError::usage(e.to_string()))?;
    let anon = allow_anonymous || cfg.server.allow_anonymous;
    let token = mcp_token(&cfg, token_file.as_deref())?;
    let path = if path == "/mcp" && !cfg.server.path.is_empty() {
        cfg.server.path.clone()
    } else {
        path
    };
    validate_http_serve(addr, expose, anon, token.as_deref(), &path)
        .map_err(|e| CliError::usage(e.to_string()))?;

    out.heading(&format!(
        "mcp-gateway {}  ir=1.0  mcp=2026-07-28",
        env!("CARGO_PKG_VERSION")
    ));
    out.line(&format!(
        "{} {}",
        out.bold("config:"),
        paths.config_file.display()
    ));
    out.line(&format!("{} {name}  tools={tools}", out.bold("spec:")));
    out.line(&format!("{} {upstream}", out.bold("upstream:")));
    out.line(&format!(
        "{} {}",
        out.bold("auth:"),
        if anon {
            "anonymous (loopback)"
        } else {
            "MCP bearer required (env MCP_GATEWAY_TOKEN)"
        }
    ));
    out.line(&format!(
        "{} {}",
        out.bold("ssrf:"),
        if allow_private {
            "private networks allowed (system resolver; metadata still denied)"
        } else {
            "public-internet defaults (private networks denied)"
        }
    ));
    out.line(&format!(
        "{} streamable-http  bind={addr}  path={path}",
        out.bold("transport:")
    ));
    out.line(&out.dim("────────"));
    out.line(&format!(
        "listening. paste into Cursor, Codex, or Claude Code: `mcp-gateway inspect {name} --client cursor`"
    ));
    out.line(&mcp_gateway_upsell::serve_boot_banner());
    if allow_private {
        out.err_line(
            "warning: --allow-private-networks is on; this process can reach RFC1918, ULA, and loopback.",
        );
    }

    serve_http(
        handler,
        HttpServeOptions {
            bind: addr,
            expose,
            bearer_token: token,
            allow_anonymous: anon,
            path,
        },
    )
    .await
    .map_err(|e| CliError::io(e.to_string()))?;
    Ok(ExitCode::Ok)
}

fn init_tracing(level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let fallback = if level.is_empty() { "info" } else { level };
        EnvFilter::new(fallback)
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
}

/// When `--bind` is omitted and `PORT` is set (Heroku/Render/DO), listen on
/// `0.0.0.0:$PORT` with `--expose`. Distroless images cannot expand `$PORT` in CMD.
pub(crate) fn resolve_http_bind(
    bind_flag: Option<&str>,
    cfg_bind: &str,
    expose_flag: bool,
    port_env: Option<&str>,
) -> Result<(String, bool), CliError> {
    if let Some(bind) = bind_flag.filter(|s| !s.is_empty()) {
        return Ok((bind.to_owned(), expose_flag));
    }
    if let Some(raw) = port_env.map(str::trim).filter(|s| !s.is_empty()) {
        let port: u16 = raw
            .parse()
            .map_err(|_| CliError::usage(format!("PORT must be a TCP port number, got {raw:?}")))?;
        return Ok((format!("0.0.0.0:{port}"), true));
    }
    Ok((cfg_bind.to_owned(), expose_flag))
}

fn spec_bootstrap_url(url_flag: Option<String>) -> Option<String> {
    url_flag
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("MCP_GATEWAY_SPEC_URL").ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

async fn bootstrap_spec(
    paths: &PlatformPaths,
    globals: &Globals,
    out: &Output,
    name: &str,
    url_flag: Option<String>,
    base_url: Option<String>,
    allow_insecure_http: bool,
) -> Result<(), CliError> {
    let url = spec_bootstrap_url(url_flag);
    let config_exists = paths.config_file.exists();
    if config_exists {
        let cfg = load_cfg(paths)?;
        if cfg.spec(name).is_ok() {
            return Ok(());
        }
        if url.is_none() {
            return Err(CliError::usage(format!(
                "unknown spec '{name}'\nhint: mcp-gateway add-spec --name {name} --url HTTPS_URL\n      or set MCP_GATEWAY_SPEC_URL / pass --url to serve"
            )));
        }
    } else {
        if url.is_none() {
            return Err(CliError::usage(format!(
                "no config at {}; run mcp-gateway init",
                paths.config_file.display()
            )));
        }
        spec::check_spec_url(
            url.as_deref().expect("url"),
            globals,
            &GatewayConfig::blank(),
            allow_insecure_http,
        )
        .await?;
        std::fs::create_dir_all(&paths.cache_dir)
            .map_err(|e| CliError::io(format!("create IR cache: {e}")))?;
        GatewayConfig::blank().save(&paths.config_file)?;
    }
    spec::add(
        paths,
        globals,
        out,
        name.to_owned(),
        url,
        None,
        base_url,
        allow_insecure_http,
        false,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_env_binds_all_interfaces_and_exposes() {
        let (bind, expose) =
            resolve_http_bind(None, "127.0.0.1:8787", false, Some("10000")).unwrap();
        assert_eq!(bind, "0.0.0.0:10000");
        assert!(expose);
    }

    #[test]
    fn explicit_bind_ignores_port_and_keeps_expose_off() {
        let (bind, expose) =
            resolve_http_bind(Some("0.0.0.0:8787"), "127.0.0.1:8787", false, Some("5000")).unwrap();
        assert_eq!(bind, "0.0.0.0:8787");
        assert!(!expose);
    }

    #[test]
    fn empty_port_uses_config_bind() {
        let (bind, expose) = resolve_http_bind(None, "127.0.0.1:8787", false, Some("  ")).unwrap();
        assert_eq!(bind, "127.0.0.1:8787");
        assert!(!expose);
    }

    #[test]
    fn invalid_port_is_usage() {
        let err = resolve_http_bind(None, "127.0.0.1:8787", false, Some("nope")).unwrap_err();
        assert!(err.to_string().contains("PORT"));
    }
}
