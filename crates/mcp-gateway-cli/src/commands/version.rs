use crate::exit::ExitCode;
use crate::output::Output;
use crate::CliError;

pub fn run(out: &Output) -> Result<ExitCode, CliError> {
    let version = env!("CARGO_PKG_VERSION");
    let commit = option_env!("VERGEN_GIT_SHA")
        .or(option_env!("GIT_COMMIT"))
        .unwrap_or("unknown");
    let target = format!(
        "{}-{}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::env::consts::FAMILY
    );
    let musl = cfg!(target_env = "musl");
    if out.json {
        out.json_value(&serde_json::json!({
            "version": version,
            "ir": mcp_gateway_ir::IR_VERSION,
            "mcp": "2026-07-28",
            "commit": commit,
            "target": target,
            "rustls": true,
            "musl": musl,
        }));
        return Ok(ExitCode::Ok);
    }
    out.line(&format!("mcp-gateway {version}"));
    out.line(&format!("ir {}", mcp_gateway_ir::IR_VERSION));
    out.line("mcp 2026-07-28");
    out.line(&format!("commit {commit}"));
    out.line(&format!("target {target}"));
    out.line(&format!(
        "rustls yes  musl {}",
        if musl { "yes" } else { "no" }
    ));
    Ok(ExitCode::Ok)
}
