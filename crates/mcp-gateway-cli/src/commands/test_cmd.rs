use crate::cli::Globals;
use crate::commands::load_cfg;
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::runtime::handler_for;
use crate::CliError;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    paths: &PlatformPaths,
    globals: &Globals,
    out: &Output,
    name: String,
    tool: String,
    args: String,
    timeout: u64,
    base_url: Option<String>,
) -> Result<ExitCode, CliError> {
    let cfg = load_cfg(paths)?;
    let spec = cfg.spec(&name)?.clone();
    let handler = handler_for(globals, &cfg, &spec, paths, false, base_url.as_deref())?;
    let arguments: serde_json::Value = serde_json::from_str(&args)
        .map_err(|e| CliError::usage(format!("--args is not JSON: {e}")))?;
    let op = handler
        .gateway
        .operation(&tool)
        .ok_or_else(|| CliError::usage(format!("unknown tool {tool}")))?;
    let timeout = std::time::Duration::from_secs(timeout.max(1));
    let target =
        mcp_gateway_proxy::render(&handler.gateway.base_url, &op.execution_plan, &arguments)
            .map(|r| r.url.to_string())
            .unwrap_or_else(|_| op.source.path_template.clone());
    out.line(&format!(
        "calling {} {}  (SSRF pinned, timeout {}s)",
        op.source.method,
        target,
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
        out.fail("isError: true");
        out.line(&format!("error: {}", result.text));
        if result.error_code.as_deref() == Some("upstream_5xx") {
            out.line("hint: HTTP 5xx came from the upstream API, not mcp-gateway");
        } else if result.text.contains("HTTP 401") || result.text.contains("HTTP 403") {
            out.line(&format!("hint: mcp-gateway auth list {name}"));
        }
        return Err(CliError::upstream(result.text));
    }
    out.ok(&format!("upstream ok  {ms}ms"));
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
