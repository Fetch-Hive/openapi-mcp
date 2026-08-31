use crate::cli::Globals;
use crate::exit::ExitCode;
use crate::ir_cache;
use crate::output::Output;
use crate::runtime::{base_url, resolver};
use crate::CliError;
use mcp_gateway_compile::SpecSource;
use mcp_gateway_ir::CompileBundle;
use mcp_gateway_proxy::{
    render, validate_and_dial, validate_schema, InjectedCredential, SsrfPolicy,
};
use secrecy::SecretString;
use std::path::PathBuf;

pub fn compile(
    out: &Output,
    spec: PathBuf,
    ir_out: Option<PathBuf>,
    report_out: Option<PathBuf>,
) -> Result<ExitCode, CliError> {
    out.verbose(&format!("source {}", spec.display()));
    let bundle = compile_source(&spec)?;
    let ir_path = ir_out.unwrap_or_else(|| PathBuf::from("ir.json"));
    let report_path = report_out.unwrap_or_else(|| PathBuf::from("report.json"));
    std::fs::write(
        &ir_path,
        serde_json::to_vec_pretty(&bundle).map_err(|e| CliError::io(e.to_string()))?,
    )
    .map_err(|e| CliError::io(e.to_string()))?;
    std::fs::write(
        &report_path,
        serde_json::to_vec_pretty(&bundle.report).map_err(|e| CliError::io(e.to_string()))?,
    )
    .map_err(|e| CliError::io(e.to_string()))?;
    out.line(&format!(
        "compiled {} ops -> {}",
        bundle.report.operations_compiled,
        ir_path.display()
    ));
    out.line(&format!("report {}", report_path.display()));
    Ok(ExitCode::Ok)
}

fn compile_source(spec: &std::path::Path) -> Result<CompileBundle, CliError> {
    let source = if spec.as_os_str() == "-" {
        SpecSource::Stdin
    } else {
        SpecSource::File(spec.to_path_buf())
    };
    mcp_gateway_compile::compile(source).map_err(|e| CliError::usage(e.to_string()))
}

pub fn list_tools(out: &Output, ir: PathBuf, tag: Option<String>) -> Result<ExitCode, CliError> {
    let bundle = ir_cache::load_bundle(&ir)?;
    for op in &bundle.api.operations {
        if let Some(tag) = &tag {
            if !op.tags.iter().any(|t| t == tag) {
                continue;
            }
        }
        out.line(&format!(
            "{}\t{}\t{} {}\tenabled={}",
            op.tool.name,
            op.tool.title,
            op.source.method,
            op.source.path_template,
            op.enabled_by_default
        ));
    }
    Ok(ExitCode::Ok)
}

#[allow(clippy::too_many_arguments)]
pub async fn call(
    _globals: &Globals,
    out: &Output,
    ir: PathBuf,
    tool_name: String,
    args: String,
    base: Option<String>,
    bearer_env: String,
    allow_disabled: bool,
) -> Result<ExitCode, CliError> {
    let bundle = ir_cache::load_bundle(&ir)?;
    let op = bundle
        .api
        .operations
        .iter()
        .find(|o| o.tool.name == tool_name)
        .ok_or_else(|| CliError::usage(format!("unknown tool {tool_name}")))?;
    if !op.enabled_by_default && !allow_disabled {
        return Err(CliError::usage(format!(
            "tool {tool_name} is disabled; pass --allow-disabled"
        )));
    }
    let arguments: serde_json::Value = serde_json::from_str(&args)
        .map_err(|e| CliError::usage(format!("--args is not JSON: {e}")))?;
    validate_schema(&op.tool.input_schema, &arguments)
        .map_err(|e| CliError::usage(format!("invalid arguments: {e}")))?;
    let url = base_url(&bundle, base.as_deref())?;
    let rendered =
        render(&url, &op.execution_plan, &arguments).map_err(|e| CliError::usage(e.to_string()))?;
    let cred = match std::env::var(&bearer_env) {
        Ok(v) if !v.is_empty() => Some(InjectedCredential::bearer(SecretString::from(v))),
        _ => None,
    };
    let policy = SsrfPolicy::default();
    let resolver = resolver(&policy)?;
    let upstream = validate_and_dial(rendered, &policy, resolver.as_ref(), cred.as_ref())
        .await
        .map_err(|e| {
            if matches!(e, mcp_gateway_proxy::ProxyError::Ssrf(_)) {
                CliError::policy(e.to_string())
            } else {
                CliError::upstream(e.to_string())
            }
        })?;
    let mapped =
        mcp_gateway_proxy::map_success(&upstream, mcp_gateway_proxy::ResponseKind::Json, None);
    let payload = serde_json::json!({
        "resultType": "complete",
        "content": [{ "type": "text", "text": mapped.text }],
        "structuredContent": mapped.structured,
        "isError": mapped.is_error,
    });
    out.json_value(&payload);
    Ok(ExitCode::Ok)
}

pub fn corpus(out: &Output, only: Option<String>) -> Result<ExitCode, CliError> {
    let manifest = std::path::Path::new("fixtures/corpus/MANIFEST.toml");
    if !manifest.exists() {
        out.line("corpus: fixtures/corpus/MANIFEST.toml not found (no-op)");
        return Ok(ExitCode::Ok);
    }
    let raw = std::fs::read_to_string(manifest).map_err(|e| CliError::io(e.to_string()))?;
    let value: toml::Value = raw
        .parse()
        .map_err(|e: toml::de::Error| CliError::usage(e.to_string()))?;
    let Some(cases) = value.get("case").and_then(|c| c.as_array()) else {
        out.line("corpus: no [[case]] entries");
        return Ok(ExitCode::Ok);
    };
    let mut ran = 0u32;
    for case in cases {
        let id = case.get("id").and_then(|v| v.as_str()).unwrap_or("-");
        if let Some(only) = &only {
            if id != only {
                continue;
            }
        }
        let spec = case.get("spec").and_then(|v| v.as_str()).unwrap_or("");
        let path = std::path::Path::new("fixtures/corpus").join(spec);
        match mcp_gateway_compile::compile(SpecSource::File(path.clone())) {
            Ok(bundle) => {
                out.line(&format!(
                    "ok {id} ops={}",
                    bundle.report.operations_compiled
                ));
                ran += 1;
            }
            Err(e) => {
                out.line(&format!("fail {id}: {e}"));
                return Err(CliError::usage(format!("corpus case {id} failed")));
            }
        }
    }
    out.line(&format!("corpus: {ran} cases"));
    Ok(ExitCode::Ok)
}
