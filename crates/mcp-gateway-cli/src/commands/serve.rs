use super::{load_cfg, spec};
use crate::cli::{Globals, TunnelAuth};
use crate::config::GatewayConfig;
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::runtime::{handler_for, mcp_token};
use crate::CliError;
use mcp_gateway_server::{
    build_router, parse_bind, serve_http, serve_listener, serve_stdio, validate_http_serve,
    HttpServeOptions,
};
use mcp_gateway_tunnel::{EndpointAuthMode, TunnelState};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    paths: &PlatformPaths,
    globals: &Globals,
    out: &Output,
    name: String,
    stdio: bool,
    tunnel: bool,
    tunnel_name: Option<String>,
    tunnel_auth: TunnelAuth,
    bind: Option<String>,
    path: String,
    expose: bool,
    allow_anonymous: bool,
    token_file: Option<std::path::PathBuf>,
    allow_insecure_http: bool,
    base_url: Option<String>,
    url: Option<String>,
) -> Result<ExitCode, CliError> {
    if tunnel_name.is_some() {
        return Err(CliError::usage("persistent names are not available yet"));
    }
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
    if tunnel {
        match tunnel_auth {
            TunnelAuth::Token
                if token
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .is_none() =>
            {
                return Err(CliError::usage(
                    "tunnel with --tunnel-auth token needs MCP_GATEWAY_TOKEN or --token-file; drop --tunnel, or pass --tunnel-auth public with --allow-anonymous",
                ));
            }
            TunnelAuth::Public if !anon => {
                return Err(CliError::usage(
                    "--tunnel-auth public requires --allow-anonymous",
                ));
            }
            _ => {}
        }
    }
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
    if tunnel {
        out.line("local server is up. waiting for the tunnel URL…");
        if tunnel_auth == TunnelAuth::Public {
            out.err_line(
                "warning: this tunnel URL is reachable by anyone on the internet with no token",
            );
        }
    } else {
        out.line(&format!(
            "listening. paste into Cursor, Codex, or Claude Code: `mcp-gateway inspect {name} --client cursor`"
        ));
        out.line(&mcp_gateway_upsell::serve_boot_banner());
    }
    if allow_private {
        out.err_line(
            "warning: --allow-private-networks is on; this process can reach RFC1918, ULA, and loopback.",
        );
    }

    let opts = HttpServeOptions {
        bind: addr,
        expose,
        bearer_token: token,
        allow_anonymous: anon,
        path: path.clone(),
        extra_allowed_hosts: Vec::new(),
    };
    if !tunnel {
        serve_http(handler, opts)
            .await
            .map_err(|e| CliError::io(e.to_string()))?;
        return Ok(ExitCode::Ok);
    }

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| CliError::io(e.to_string()))?;
    let local = listener
        .local_addr()
        .map_err(|e| CliError::io(e.to_string()))?;
    let authority = loopback_authority(local);
    let listener_app =
        build_router(handler.clone(), &opts).map_err(|e| CliError::usage(e.to_string()))?;
    let mut tunnel_opts = opts;
    tunnel_opts.allow_anonymous = tunnel_auth == TunnelAuth::Public;
    tunnel_opts.extra_allowed_hosts = vec![authority.clone()];
    let tunnel_app =
        build_router(handler, &tunnel_opts).map_err(|e| CliError::usage(e.to_string()))?;
    let Some(tunnel_cfg) = prepare_tunnel(
        true,
        &mcp_gateway_tunnel::resolve_relay_url(&cfg.tunnel.relay_url),
        &authority,
        &path,
        tunnel_auth,
    ) else {
        return Err(CliError::usage("tunnel did not start"));
    };
    if out.json {
        out.json_value(&serde_json::json!({"event":"tunnel","state":"connecting"}));
    }
    let handle = mcp_gateway_tunnel::run(tunnel_cfg, tunnel_app);
    let shutdown = handle.shutdown.clone();
    let server_shutdown = shutdown.clone();
    let server = tokio::spawn(async move {
        serve_listener(listener, listener_app, async move {
            server_shutdown.cancelled().await;
        })
        .await
    });
    let mcp_gateway_tunnel::TunnelHandle {
        mut state,
        mut done,
        shutdown,
        ..
    } = handle;
    let mut interrupted = false;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                shutdown.cancel();
                interrupted = true;
                break;
            }
            changed = state.changed() => {
                if changed.is_err() {
                    break;
                }
                let snapshot = state.borrow().clone();
                emit_tunnel(out, &snapshot);
                if let TunnelState::Rejected { code, message } = snapshot {
                    shutdown.cancel();
                    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
                    return Err(reject_cli(&code, &message));
                }
            }
            _ = done.changed() => {
                let snapshot = state.borrow().clone();
                if let TunnelState::Rejected { code, message } = snapshot {
                    shutdown.cancel();
                    return Err(reject_cli(&code, &message));
                }
                break;
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while !*done.borrow() {
            if done.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
    if interrupted {
        Ok(ExitCode::Interrupted)
    } else {
        Ok(ExitCode::Ok)
    }
}

fn emit_tunnel(out: &Output, state: &TunnelState) {
    if out.json {
        let value = match state {
            TunnelState::Connecting => serde_json::json!({"event":"tunnel","state":"connecting"}),
            TunnelState::Connected { url, slug } => {
                serde_json::json!({"event":"tunnel","state":"connected","url":url,"slug":slug})
            }
            TunnelState::Reconnecting => {
                serde_json::json!({"event":"tunnel","state":"reconnecting"})
            }
            TunnelState::Rejected { code, message } => {
                serde_json::json!({"event":"tunnel","state":"rejected","code":code,"message":message})
            }
            TunnelState::Stopped => serde_json::json!({"event":"tunnel","state":"stopped"}),
        };
        out.json_value(&value);
        return;
    }
    match state {
        TunnelState::Connected { url, .. } => {
            out.line(&format!("{} {url}", out.bold("tunnel:")));
            out.line(
                &out.dim(
                    "this URL is anonymous and is released 30 minutes after the CLI disconnects",
                ),
            );
        }
        TunnelState::Reconnecting => out.line(&out.dim("reconnecting tunnel…")),
        TunnelState::Rejected { code, message } => {
            out.err_line(&format!("tunnel rejected ({code}): {message}"));
        }
        TunnelState::Connecting | TunnelState::Stopped => {}
    }
}

fn reject_cli(code: &str, message: &str) -> CliError {
    let text = format!("tunnel rejected ({code}): {message}");
    match code {
        "unauthorized" | "plan_limit" => CliError::policy(text),
        "version_unsupported" | "name_taken" | "name_invalid" | "name_reserved" => {
            CliError::usage(text)
        }
        _ => CliError::upstream(text),
    }
}

/// `None` when tunnel mode is off, so a normal `serve` never builds a client.
pub(crate) fn prepare_tunnel(
    enabled: bool,
    relay_url: &str,
    local_authority: &str,
    mcp_path: &str,
    auth: TunnelAuth,
) -> Option<mcp_gateway_tunnel::TunnelConfig> {
    if !enabled {
        return None;
    }
    Some(mcp_gateway_tunnel::TunnelConfig {
        relay_url: relay_url.to_owned(),
        local_authority: local_authority.to_owned(),
        mcp_path: mcp_path.to_owned(),
        auth_mode: match auth {
            TunnelAuth::Token => EndpointAuthMode::Token,
            TunnelAuth::Public => EndpointAuthMode::Public,
        },
        client: mcp_gateway_tunnel::client_identity(),
    })
}

fn loopback_authority(addr: SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => format!("127.0.0.1:{}", addr.port()),
        IpAddr::V6(ip) if ip.is_unspecified() => format!("[::1]:{}", addr.port()),
        IpAddr::V6(ip) => format!("[{ip}]:{}", addr.port()),
        _ => addr.to_string(),
    }
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

    #[test]
    fn plain_serve_does_not_build_a_tunnel() {
        assert!(prepare_tunnel(
            false,
            "wss://connect.mcp.fetchhive.com/v1/tunnel",
            "127.0.0.1:9",
            "/mcp",
            TunnelAuth::Token
        )
        .is_none());
    }
}
