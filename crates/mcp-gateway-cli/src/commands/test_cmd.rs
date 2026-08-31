use crate::cli::Globals;
use crate::commands::load_cfg;
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::runtime::handler_for;
use crate::CliError;
use std::time::Instant;

pub async fn run(
    paths: &PlatformPaths,
    globals: &Globals,
    out: &Output,
    name: String,
    tool: String,
    args: String,
    timeout: u64,
) -> Result<ExitCode, CliError> {
    let cfg = load_cfg(paths)?;
    let spec = cfg.spec(&name)?.clone();
    let handler = handler_for(globals, &cfg, &spec, paths, false, None)?;
    let arguments: serde_json::Value = serde_json::from_str(&args)
        .map_err(|e| CliError::usage(format!("--args is not JSON: {e}")))?;
    let op = handler
        .gateway
        .operation(&tool)
        .ok_or_else(|| CliError::usage(format!("unknown tool {tool}")))?;
    let timeout = std::time::Duration::from_secs(timeout.max(1));
    out.line(&format!(
        "calling {} {}  (SSRF pinned, timeout {}s)",
        op.source.method,
        op.source.path_template,
        timeout.as_secs()
    ));
    let start = Instant::now();
    let result = match tokio::time::timeout(timeout, handler.execute_named(&tool, arguments)).await
    {
        Ok(r) => r,
        Err(_) => {
            return Err(CliError::upstream(format!(
                "tool call timed out after {}s",
                timeout.as_secs()
            )))
        }
    };
    let ms = start.elapsed().as_millis();
    if result.error_code.as_deref() == Some("ssrf") {
        return Err(CliError::policy(result.text.clone()));
    }
    if result.is_error {
        out.line("isError: true");
        out.line(&format!("error: {}", result.text));
        out.line(&format!("hint: mcp-gateway auth list {name}"));
        return Err(CliError::upstream(result.text));
    }
    out.line(&format!("upstream ok  {ms}ms"));
    out.line("isError: false");
    let preview = if result.text.len() > 1024 {
        format!("{}…", &result.text[..1024])
    } else {
        result.text.clone()
    };
    out.line(&format!("result: (truncated 1 KB) {preview}"));
    if out.json {
        out.json_value(&result);
    }
    Ok(ExitCode::Ok)
}
