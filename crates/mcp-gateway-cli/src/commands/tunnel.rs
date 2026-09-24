//! `mcp-gateway tunnel` — anonymous tunnel in front of any MCP server.

use super::load_cfg;
use super::serve::{self, emit_request, emit_tunnel, print_event, reject_cli};
use super::tunnel_screen::{self, TunnelScreen};
use crate::cli::{Globals, ProxyAuth as CliAuth};
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::CliError;
use mcp_gateway_server::parse_bind;
use mcp_gateway_tunnel::upstream::{
    generate_bearer_token, serve_local, AuthGate, HttpUpstream, PinError, ProxyAuth, StdioBridge,
};
use mcp_gateway_tunnel::{
    client_identity, run as open_tunnel, EndpointAuthMode, LocalService, TunnelConfig, TunnelState,
};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::watch;

const MCP_PATH: &str = "/mcp";

#[allow(clippy::too_many_arguments)]
pub async fn run(
    paths: &PlatformPaths,
    _globals: &Globals,
    out: &Output,
    url: Option<String>,
    stdio: bool,
    command: Vec<String>,
    tunnel_auth: CliAuth,
    token: Option<String>,
    bind: Option<String>,
    name: Option<String>,
    no_probe: bool,
    allow_remote_upstream: bool,
) -> Result<ExitCode, CliError> {
    if name.is_some() {
        return Err(CliError::usage("persistent names are not available yet"));
    }
    let mode = match tunnel_auth {
        CliAuth::Token => ProxyAuth::Token,
        CliAuth::Passthrough => ProxyAuth::Passthrough,
        CliAuth::Public => ProxyAuth::Public,
    };
    let supplied = token.filter(|value| !value.is_empty());
    match (url, stdio) {
        (Some(url), false) => {
            let upstream = match HttpUpstream::connect(&url, allow_remote_upstream).await {
                Ok(upstream) => upstream,
                Err(PinError::Refused(message)) => return Err(CliError::usage(message)),
                Err(PinError::Resolve(message)) => return Err(CliError::upstream(message)),
            };
            let cfg = load_cfg(paths)?;
            serve::init_tracing(&cfg.log.level);
            let probe = if no_probe {
                None
            } else {
                Some(
                    upstream
                        .probe(probe_authorization(mode, supplied.as_deref()))
                        .await
                        .map_err(CliError::upstream)?,
                )
            };
            let (gate_token, generated) = gate_token(mode, supplied);
            let authority = upstream.host_header().to_owned();
            let gate = AuthGate::new(mode, gate_token, upstream);
            let relay = mcp_gateway_tunnel::resolve_relay_url(&cfg.tunnel.relay_url);
            let probe_line = probe
                .as_ref()
                .map(|item| item.summary())
                .unwrap_or_else(|| "skipped".to_owned());
            drive(
                out,
                mode,
                generated,
                &url,
                &authority,
                &relay,
                &[("Upstream", url.as_str()), ("Probe", probe_line.as_str())],
                gate,
                None,
                None,
            )
            .await
        }
        (None, true) if !command.is_empty() => {
            let cfg = load_cfg(paths)?;
            serve::init_tracing(&cfg.log.level);
            let bridge = StdioBridge::start(command.clone(), !no_probe)
                .await
                .map_err(CliError::upstream)?;
            let report = bridge.report().await;
            let bind = bind.unwrap_or_else(|| "127.0.0.1:8787".into());
            let addr = parse_bind(&bind).map_err(|err| CliError::usage(err.to_string()))?;
            let listener = TcpListener::bind(addr)
                .await
                .map_err(|err| CliError::io(err.to_string()))?;
            let local = listener
                .local_addr()
                .map_err(|err| CliError::io(err.to_string()))?;
            let authority = serve::loopback_authority(local);
            let local_url = tunnel_screen::local_http_url(&authority, MCP_PATH);
            let (gate_token, generated) = gate_token(mode, supplied);
            let fatal = bridge.fatal();
            let gate = AuthGate::new(mode, gate_token, bridge);
            let relay = mcp_gateway_tunnel::resolve_relay_url(&cfg.tunnel.relay_url);
            let upstream_line = format!("stdio: {}", command.join(" "));
            let probe_line = report.summary();
            drive(
                out,
                mode,
                generated,
                &local_url,
                &authority,
                &relay,
                &[
                    ("Upstream", upstream_line.as_str()),
                    ("Probe", probe_line.as_str()),
                ],
                gate,
                Some(listener),
                Some(fatal),
            )
            .await
        }
        (None, true) => Err(CliError::usage(
            "pass the stdio command after --, for example --stdio -- npx -y @modelcontextprotocol/server-filesystem /tmp",
        )),
        (None, false) | (Some(_), true) => Err(CliError::usage(
            "pass an upstream URL or --stdio -- <command>",
        )),
    }
}

fn probe_authorization(mode: ProxyAuth, token: Option<&str>) -> Option<&str> {
    match mode {
        ProxyAuth::Passthrough => token.filter(|value| !value.is_empty()),
        ProxyAuth::Token | ProxyAuth::Public => None,
    }
}

fn gate_token(mode: ProxyAuth, supplied: Option<String>) -> (Option<String>, Option<String>) {
    match mode {
        ProxyAuth::Token => match supplied {
            Some(token) => (Some(token), None),
            None => {
                let created = generate_bearer_token();
                (Some(created.clone()), Some(created))
            }
        },
        ProxyAuth::Passthrough | ProxyAuth::Public => (None, None),
    }
}

#[allow(clippy::too_many_arguments)]
async fn drive<S: LocalService>(
    out: &Output,
    mode: ProxyAuth,
    generated: Option<String>,
    local_url: &str,
    authority: &str,
    relay: &str,
    extras: &[(&str, &str)],
    service: S,
    listener: Option<TcpListener>,
    mut fatal: Option<watch::Receiver<Option<String>>>,
) -> Result<ExitCode, CliError> {
    if mode == ProxyAuth::Public && (out.json || out.quiet) {
        eprintln!("warning: this tunnel URL is reachable by anyone on the internet with no token");
    }
    if let Some(token) = &generated {
        if out.json || out.quiet {
            eprintln!("MCP_GATEWAY_TOKEN={token}");
        }
    }
    let auth_override = generated
        .as_ref()
        .map(|token| format!("bearer {token}"))
        .or_else(|| {
            (mode == ProxyAuth::Passthrough)
                .then(|| "passthrough, upstream sees the caller's Authorization".to_owned())
        });
    let config = TunnelConfig {
        relay_url: relay.to_owned(),
        local_authority: authority.to_owned(),
        mcp_path: MCP_PATH.to_owned(),
        auth_mode: match mode {
            ProxyAuth::Token => EndpointAuthMode::Token,
            ProxyAuth::Passthrough | ProxyAuth::Public => EndpointAuthMode::Public,
        },
        client: client_identity(),
    };
    if out.json {
        print_event(&serde_json::json!({"event":"tunnel","state":"connecting"}));
    }
    let handle = open_tunnel(config, service.clone());
    let shutdown = handle.shutdown.clone();
    if let Some(listener) = listener {
        let server_shutdown = shutdown.clone();
        tokio::spawn(async move {
            serve_local(listener, service, server_shutdown).await;
        });
    }
    let mcp_gateway_tunnel::TunnelHandle {
        mut state,
        mut done,
        mut requests,
        shutdown,
        stats,
        ..
    } = handle;
    let mut screen = TunnelScreen::open(
        out,
        env!("CARGO_PKG_VERSION"),
        local_url,
        mode == ProxyAuth::Public,
        auth_override.as_deref(),
        extras,
        &stats,
    );
    let mut interrupted = false;
    let mut requests_open = true;
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                shutdown.cancel();
                interrupted = true;
                break;
            }
            _ = async {
                match fatal.as_mut() {
                    Some(fatal) => {
                        let _ = fatal.changed().await;
                    }
                    None => std::future::pending::<()>().await,
                }
            } => {
                let message = fatal
                    .as_ref()
                    .and_then(|fatal| fatal.borrow().clone())
                    .unwrap_or_else(|| "stdio child stopped".to_owned());
                shutdown.cancel();
                screen.finish();
                return Err(CliError::upstream(message));
            }
            changed = state.changed() => {
                if changed.is_err() {
                    break;
                }
                let snapshot = state.borrow().clone();
                emit_tunnel(out, &snapshot);
                screen.apply_state(&snapshot, &stats);
                if let TunnelState::Rejected { code, message } = snapshot {
                    shutdown.cancel();
                    screen.finish();
                    return Err(reject_cli(&code, &message));
                }
            }
            finished = requests.recv(), if requests_open => {
                match finished {
                    Some(request) => emit_request(out, &request, &mut screen, &stats),
                    None => requests_open = false,
                }
            }
            _ = done.changed() => {
                let snapshot = state.borrow().clone();
                if let TunnelState::Rejected { code, message } = snapshot {
                    shutdown.cancel();
                    screen.finish();
                    return Err(reject_cli(&code, &message));
                }
                break;
            }
        }
    }
    screen.finish();
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while !*done.borrow() {
            if done.changed().await.is_err() {
                break;
            }
        }
    })
    .await;
    if interrupted {
        Ok(ExitCode::Interrupted)
    } else {
        Ok(ExitCode::Ok)
    }
}
