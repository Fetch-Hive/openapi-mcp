use crate::cli::Globals;
use crate::commands::load_cfg;
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
) -> Result<ExitCode, CliError> {
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

    let bind_raw = bind.unwrap_or_else(|| cfg.server.bind.clone());
    let addr = parse_bind(&bind_raw).map_err(|e| CliError::usage(e.to_string()))?;
    let expose = expose || cfg.server.expose;
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
        "listening. clients: see `mcp-gateway inspect {name} --client cursor`"
    ));
    out.line(&mcp_gateway_upsell::serve_boot_banner());
    if allow_private {
        out.err_line(
            "warning: --allow-private-networks is on; this process can reach RFC1918/ULA targets.",
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
