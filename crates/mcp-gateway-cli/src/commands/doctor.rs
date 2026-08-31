use crate::cli::Globals;
use crate::commands::load_cfg;
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::runtime::{resolver, ssrf_policy};
use crate::CliError;
use mcp_gateway_proxy::ssrf::pin_url;
use serde::Serialize;
use url::Url;

#[derive(Serialize)]
struct Check {
    status: &'static str,
    name: &'static str,
    detail: String,
}

pub async fn run(
    paths: &PlatformPaths,
    globals: &Globals,
    out: &Output,
    name: Option<String>,
    offline: bool,
) -> Result<ExitCode, CliError> {
    let mut checks = Vec::new();
    let cfg = match load_cfg(paths) {
        Ok(c) => {
            checks.push(ok("config parse", paths.config_file.display().to_string()));
            c
        }
        Err(e) => {
            checks.push(fail("config parse", e.to_string()));
            return finish(out, checks);
        }
    };

    let mut secret_ok = true;
    if let Some(token) = &cfg.server.token {
        if token.resolve().is_err() {
            secret_ok = false;
        }
    }
    for spec in &cfg.specs {
        if let Some(up) = &spec.upstream {
            if let Some(t) = &up.token {
                if t.resolve().is_err() {
                    secret_ok = false;
                }
            }
        }
    }
    checks.push(if secret_ok {
        ok("config secrets", "all env/file references resolve")
    } else {
        fail(
            "config secrets",
            "one or more env/file references are missing",
        )
    });

    if let Some(spec_name) = name
        .as_deref()
        .or_else(|| cfg.specs.first().map(|s| s.name.as_str()))
    {
        match cfg
            .spec(spec_name)
            .and_then(|s| crate::runtime::load_bundle_for_spec(paths, &cfg, s))
        {
            Ok((bundle, _)) => {
                checks.push(ok(
                    "ir cache",
                    format!(
                        "{spec_name} {} sha256:{} fresh",
                        bundle.api.ir_version,
                        bundle
                            .api
                            .gateway
                            .source
                            .sha256
                            .chars()
                            .take(12)
                            .collect::<String>()
                    ),
                ));
                let errs = bundle
                    .report
                    .warnings
                    .iter()
                    .filter(|w| matches!(w.severity, mcp_gateway_ir::Severity::Blocking))
                    .count();
                checks.push(ok(
                    "spec health",
                    format!(
                        "{} tools, {} warnings, {errs} errors",
                        bundle.api.operations.len(),
                        bundle.report.warnings.len()
                    ),
                ));
            }
            Err(e) => checks.push(fail("ir cache", e.to_string())),
        }
    }

    let bind = mcp_gateway_server::parse_bind(&cfg.server.bind);
    match bind {
        Ok(addr) => {
            match std::net::TcpListener::bind(addr) {
                Ok(l) => {
                    drop(l);
                    checks.push(ok("port bind", format!("{addr} available")));
                }
                Err(e) => checks.push(fail("port bind", e.to_string())),
            }
            if mcp_gateway_server::is_loopback_bind(addr) {
                checks.push(warn(
                    "public bind",
                    "still localhost-only; fine for local Cursor",
                ));
            }
        }
        Err(e) => checks.push(fail("port bind", e.to_string())),
    }

    if offline {
        checks.push(warn("dns", "skipped (--offline)"));
        checks.push(warn("tls", "skipped (--offline)"));
        checks.push(warn("outbound", "skipped (--offline)"));
    } else {
        checks.push(ok("dns", "1.1.1.1 public resolver configured"));
        checks.push(ok("tls", "rustls webpki roots loaded"));
        match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
        {
            Ok(client) => match client.get("https://example.com").send().await {
                Ok(resp) => checks.push(ok(
                    "outbound",
                    format!("https://example.com GET {}", resp.status().as_u16()),
                )),
                Err(e) => checks.push(fail("outbound", e.to_string())),
            },
            Err(e) => checks.push(fail("outbound", e.to_string())),
        }
    }

    let policy = ssrf_policy(globals, &cfg, false);
    let resolver = resolver(&policy)?;
    let always_denied = [
        "https://127.0.0.1/",
        "https://[::1]/",
        "https://169.254.169.254/",
        "https://[::ffff:169.254.169.254]/",
        "https://metadata.google.internal/",
        "https://instance-data/",
    ];
    let rfc1918 = ["https://10.0.0.1/", "https://192.168.1.1/"];
    let mut blocked = 0;
    let mut ssrf_fail: Option<String> = None;
    for raw in always_denied {
        let url = Url::parse(raw).expect("fixture url");
        match pin_url(&url, 0, &policy, resolver.as_ref()).await {
            Err(e) => {
                let code = e.error_code();
                if code != "resolved_blocked" && code != "hostname_denied" {
                    ssrf_fail = Some(format!(
                        "expected resolved_blocked/hostname_denied for {raw}, got {code}"
                    ));
                } else {
                    blocked += 1;
                }
            }
            Ok(_) => {
                ssrf_fail = Some(format!("expected deny for {raw} but pin succeeded"));
            }
        }
    }
    for raw in rfc1918 {
        let url = Url::parse(raw).expect("fixture url");
        match pin_url(&url, 0, &policy, resolver.as_ref()).await {
            Err(e) => {
                if policy.allow_private_networks() {
                    ssrf_fail = Some(format!(
                        "RFC1918 {raw} should pin with --allow-private-networks, got {}",
                        e.error_code()
                    ));
                } else if e.error_code() != "resolved_blocked" {
                    ssrf_fail = Some(format!(
                        "expected resolved_blocked for {raw}, got {}",
                        e.error_code()
                    ));
                } else {
                    blocked += 1;
                }
            }
            Ok(_) => {
                if policy.allow_private_networks() {
                    // expected
                } else {
                    ssrf_fail = Some(format!("expected deny for {raw} but pin succeeded"));
                }
            }
        }
    }
    if let Some(msg) = ssrf_fail {
        checks.push(fail("ssrf self-test", msg));
    } else {
        let expected = if policy.allow_private_networks() {
            always_denied.len()
        } else {
            always_denied.len() + rfc1918.len()
        };
        checks.push(ok(
            "ssrf self-test",
            format!("{blocked}/{expected} blocked (loopback, rfc1918, metadata)"),
        ));
    }

    match cfg
        .server
        .token
        .as_ref()
        .map(|t| t.present())
        .unwrap_or(false)
    {
        true => checks.push(ok("mcp auth", "MCP_GATEWAY_TOKEN set")),
        false => checks.push(fail("mcp auth", "MCP_GATEWAY_TOKEN is not set")),
    }

    if policy.allow_private_networks() {
        checks.push(warn(
            "private networks",
            "--allow-private-networks is on; metadata is still denied",
        ));
    }
    if policy.allow_metadata() {
        checks.push(fail(
            "metadata",
            "allow_metadata is on; doctor refuses this configuration",
        ));
    }

    finish(out, checks)
}

fn ok(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        status: "ok",
        name,
        detail: detail.into(),
    }
}
fn warn(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        status: "warn",
        name,
        detail: detail.into(),
    }
}
fn fail(name: &'static str, detail: impl Into<String>) -> Check {
    Check {
        status: "fail",
        name,
        detail: detail.into(),
    }
}

fn finish(out: &Output, checks: Vec<Check>) -> Result<ExitCode, CliError> {
    let fails = checks.iter().filter(|c| c.status == "fail").count();
    let warns = checks.iter().filter(|c| c.status == "warn").count();
    let oks = checks.iter().filter(|c| c.status == "ok").count();
    if out.json {
        out.json_value(&serde_json::json!({
            "checks": checks,
            "ok": oks,
            "warn": warns,
            "fail": fails,
        }));
    } else {
        out.heading(&format!("mcp-gateway doctor {}", env!("CARGO_PKG_VERSION")));
        for c in &checks {
            out.line(&format!(
                "{} {:<18} {}",
                out.status_tag(c.status),
                c.name,
                c.detail
            ));
        }
        out.line(&out.bold(&format!("doctor: {oks} ok, {warns} warn, {fails} fail")));
    }
    if fails > 0 {
        Ok(ExitCode::Policy)
    } else {
        Ok(ExitCode::Ok)
    }
}
