use crate::cli::{ClientKind, Globals};
use crate::clients;
use crate::commands::load_cfg;
use crate::config::{GatewayConfig, SpecEntry};
use crate::exit::ExitCode;
use crate::ir_cache;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::runtime::{resolver, ssrf_policy, upstream_base_url};
use crate::CliError;
use mcp_gateway_compile::{compile_with, CompileOptions, SafetyOpts, SpecSource};
use mcp_gateway_proxy::ssrf::pin_url;
use std::path::PathBuf;
use url::Url;

#[allow(clippy::too_many_arguments)]
pub async fn add(
    paths: &PlatformPaths,
    globals: &Globals,
    out: &Output,
    name: String,
    url: Option<String>,
    file: Option<PathBuf>,
    base_url: Option<String>,
    insecure_http: bool,
    force: bool,
) -> Result<ExitCode, CliError> {
    let mut cfg = load_cfg(paths)?;
    if cfg.specs.iter().any(|s| s.name == name) && !force {
        return Err(CliError::usage(format!(
            "spec {name} already exists; pass --force to replace"
        )));
    }
    if let Some(base) = base_url.as_deref() {
        if !mcp_gateway_compile::is_absolute_http_url(base) {
            return Err(CliError::usage(
                "--base-url must be an absolute http(s) URL (host + scheme)",
            ));
        }
    }
    let source = match (url.as_deref(), file.as_deref()) {
        (Some(u), None) => {
            check_spec_url(u, globals, &cfg, insecure_http).await?;
            out.line(&format!("Fetching {u} (HTTPS, 10 MiB cap, 15s)"));
            out.verbose("spec URL passed SSRF pin");
            SpecSource::Url(u.to_owned())
        }
        (None, Some(p)) => SpecSource::File(p.to_path_buf()),
        _ => {
            return Err(CliError::usage(
                "exactly one of --url or --file is required",
            ))
        }
    };
    let allow_private = globals.allow_private_networks || cfg.ssrf.allow_private_networks;
    let bundle = compile_with(
        source,
        CompileOptions {
            max_ops: None,
            safety: SafetyOpts { allow_private },
        },
    )
    .map_err(|e| {
        if matches!(e, mcp_gateway_compile::CompileError::Safety(_)) {
            CliError::policy(e.to_string())
        } else {
            CliError::usage(e.to_string())
        }
    })?;
    out.line(&format!(
        "Parsed OpenAPI {}, normalised to 3.1 view",
        bundle.api.gateway.spec_version
    ));
    out.line(&format!(
        "Compiled IR {} in {}ms",
        bundle.api.ir_version, bundle.report.compile_ms
    ));
    out.line(&format!(
        "  operations: {} compiled, {} skipped",
        bundle.report.operations_compiled, bundle.report.operations_skipped
    ));
    if !bundle.report.warnings.is_empty() {
        let codes: Vec<_> = bundle
            .report
            .warnings
            .iter()
            .map(|w| format!("{:?}", w.code))
            .collect();
        out.warn(&format!(
            "warnings: {}  ({})",
            bundle.report.warnings.len(),
            codes.join(", ")
        ));
    }
    let cache = if cfg.cache.dir.is_empty() {
        paths.cache_dir.clone()
    } else {
        PathBuf::from(&cfg.cache.dir)
    };
    let (ir_path, sha) = ir_cache::write_bundle(&cache, &name, &bundle)?;
    let pin = sha.chars().take(12).collect::<String>();
    cfg.specs.retain(|s| s.name != name);
    cfg.specs.push(SpecEntry {
        name: name.clone(),
        url,
        file: file.map(|p| p.display().to_string()),
        base_url: base_url.clone(),
        ir_pin: Some(pin.clone()),
        enabled_tools: vec![],
        disabled_tools: vec![],
        upstream: None,
    });
    cfg.save(&paths.config_file)?;
    if out.json {
        out.json_value(&serde_json::json!({
            "name": name,
            "operations_compiled": bundle.report.operations_compiled,
            "ir_sha256": sha,
            "ir_path": ir_path,
        }));
        return Ok(ExitCode::Ok);
    }
    out.ok(&format!("Wrote spec [{name}] to config"));
    out.line(&format!("Cached IR sha256:{pin}… at {}", ir_path.display()));
    let upstream_preview = base_url
        .as_deref()
        .or_else(|| bundle.api.servers.first().map(|s| s.url_template.as_str()))
        .unwrap_or("(none)");
    out.line(&format!("upstream: {upstream_preview}"));
    let tool_names: Vec<&str> = bundle
        .api
        .operations
        .iter()
        .map(|op| op.tool.name.as_str())
        .collect();
    out.line(&format!("tools: {}", preview_tool_names(&tool_names, 8)));
    out.line(&format!(
        "      mcp-gateway inspect {name} lists every compiled name"
    ));
    out.line(&format!(
        "Next: mcp-gateway auth add {name} --type bearer --from-env PETSTORE_TOKEN"
    ));
    out.line(&format!("      mcp-gateway serve {name}"));
    Ok(ExitCode::Ok)
}

fn preview_tool_names(names: &[&str], max: usize) -> String {
    if names.is_empty() {
        return "(none)".into();
    }
    if names.len() <= max {
        return names.join(", ");
    }
    format!("{}, …", names[..max].join(", "))
}

pub(crate) async fn check_spec_url(
    raw: &str,
    globals: &Globals,
    cfg: &GatewayConfig,
    insecure_http: bool,
) -> Result<(), CliError> {
    let url = Url::parse(raw).map_err(|e| CliError::usage(format!("invalid URL: {e}")))?;
    let policy = ssrf_policy(globals, cfg, insecure_http);
    let resolver = resolver(&policy)?;
    pin_url(&url, 0, &policy, resolver.as_ref())
        .await
        .map_err(|e| {
            CliError::policy(format!(
                "spec URL rejected ({})\n  host: {}\n  reason: {}\nhint: this is the same SSRF policy as Fetch Hive Cloud.\n      if you intend to compile a spec on your private network, pass\n      --allow-private-networks (prints a warning; see mcp-gateway doctor)",
                e.error_code(),
                url.host_str().unwrap_or_default(),
                e
            ))
        })?;
    Ok(())
}

pub fn list(paths: &PlatformPaths, out: &Output) -> Result<ExitCode, CliError> {
    let cfg = load_cfg(paths)?;
    if cfg.specs.is_empty() {
        out.line("No specs. mcp-gateway add-spec --name petstore --url https://…");
        return Ok(ExitCode::Ok);
    }
    if out.json {
        out.json_value(&cfg.specs);
        return Ok(ExitCode::Ok);
    }
    out.line(&out.bold("NAME       SPECS  TOOLS  AUTH     LAST SERVE"));
    for spec in &cfg.specs {
        let tools = ir_cache::find_cached(&paths.cache_dir, &spec.name)
            .and_then(|p| ir_cache::load_bundle(&p).ok())
            .map(|b| b.api.operations.len())
            .unwrap_or(0);
        let auth = spec
            .upstream
            .as_ref()
            .map(|u| u.kind.as_str())
            .unwrap_or("none");
        out.line(&format!(
            "{:<10} {:<6} {:<6} {:<8} never",
            spec.name, 1, tools, auth
        ));
    }
    Ok(ExitCode::Ok)
}

pub fn inspect(
    paths: &PlatformPaths,
    _globals: &Globals,
    out: &Output,
    name: Option<String>,
    tool: Option<String>,
    client: Option<ClientKind>,
) -> Result<ExitCode, CliError> {
    let name = match name.as_deref() {
        None | Some("config") => {
            out.line(&format!("config: {}", paths.config_file.display()));
            return Ok(ExitCode::Ok);
        }
        Some(n) => n,
    };
    let cfg = load_cfg(paths)?;
    let spec = cfg.spec(name)?;
    if let Some(kind) = client {
        let snippet = clients::snippet(kind, name, &cfg.server.bind, &cfg.server.path);
        if out.json {
            out.json_value(&snippet);
        } else {
            out.line(&serde_json::to_string_pretty(&snippet).unwrap_or_default());
        }
        return Ok(ExitCode::Ok);
    }
    let (bundle, _) = crate::runtime::load_bundle_for_spec(paths, &cfg, spec)?;
    if let Some(tool_name) = tool {
        let op = bundle
            .api
            .operations
            .iter()
            .find(|o| o.tool.name == tool_name)
            .ok_or_else(|| CliError::usage(format!("unknown tool {tool_name}")))?;
        let auth = spec
            .upstream
            .as_ref()
            .map(|u| {
                let src = u
                    .token
                    .as_ref()
                    .map(|t| t.display())
                    .unwrap_or_else(|| "none".into());
                let present = u.token.as_ref().map(|t| t.present()).unwrap_or(false);
                format!(
                    "{} ({src}, present={})",
                    u.kind,
                    if present { "yes" } else { "no" }
                )
            })
            .unwrap_or_else(|| "none".into());
        if out.json {
            out.json_value(&serde_json::json!({
                "tool": op.tool.name,
                "id": op.id,
                "http": format!("{} {}", op.source.method, op.source.path_template),
                "auth": auth,
                "annotations": op.tool.annotations,
                "inputSchema": op.tool.input_schema,
            }));
            return Ok(ExitCode::Ok);
        }
        out.line(&format!("tool: {}", op.tool.name));
        out.line(&format!("id:   {}", op.id));
        out.line(&format!(
            "http: {} {}",
            op.source.method, op.source.path_template
        ));
        out.line(&format!("auth: {auth}"));
        out.line(&format!(
            "annotations: readOnlyHint={} destructiveHint={:?} idempotentHint={:?} openWorldHint={}",
            op.tool.annotations.read_only_hint,
            op.tool.annotations.destructive_hint,
            op.tool.annotations.idempotent_hint,
            op.tool.annotations.open_world_hint
        ));
        out.line(&format!(
            "inputSchema: {}",
            serde_json::to_string(&op.tool.input_schema).unwrap_or_default()
        ));
        return Ok(ExitCode::Ok);
    }
    let tool_names: Vec<&str> = bundle
        .api
        .operations
        .iter()
        .map(|op| op.tool.name.as_str())
        .collect();
    if out.json {
        out.json_value(&serde_json::json!({
            "name": spec.name,
            "title": bundle.api.gateway.title,
            "upstream": upstream_base_url(&bundle, spec, None).ok(),
            "tools": bundle.api.operations.len(),
            "tool_names": tool_names,
            "warnings": bundle.report.warnings.len(),
            "ir_version": bundle.api.ir_version,
        }));
        return Ok(ExitCode::Ok);
    }
    out.line(&format!("{} {}", out.bold("spec:"), spec.name));
    out.line(&format!(
        "{} {}",
        out.bold("title:"),
        bundle.api.gateway.title
    ));
    match upstream_base_url(&bundle, spec, None) {
        Ok(u) => out.line(&format!("{} {u}", out.bold("upstream:"))),
        Err(_) => {
            let raw = bundle
                .api
                .servers
                .first()
                .map(|s| s.url_template.as_str())
                .unwrap_or("(none)");
            out.warn(&format!(
                "upstream: {raw}  (relative; pass --base-url or add-spec --url)"
            ));
        }
    }
    out.line(&format!(
        "{} {}",
        out.bold("tools:"),
        bundle.api.operations.len()
    ));
    out.line(&format!(
        "{} {}",
        out.bold("warnings:"),
        bundle.report.warnings.len()
    ));
    out.line("");
    let name_width = tool_names.iter().map(|n| n.len()).max().unwrap_or(4).max(4);
    out.line(&out.bold(&format!(
        "{:<name_width$}  {:<6}  PATH",
        "NAME",
        "METHOD",
        name_width = name_width
    )));
    for op in &bundle.api.operations {
        out.line(&format!(
            "{:<name_width$}  {:<6}  {}",
            op.tool.name,
            op.source.method,
            op.source.path_template,
            name_width = name_width
        ));
    }
    Ok(ExitCode::Ok)
}
