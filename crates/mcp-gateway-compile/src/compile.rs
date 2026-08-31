use crate::error::CompileError;
use crate::loader::{self, LoadedSpec, SpecSource};
use crate::lower;
use crate::names;
use crate::parse;
use crate::refs::Retriever;
use crate::safety::SafetyOpts;
use crate::server_url::resolve_server_url;
use mcp_gateway_ir::*;
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct CompileOptions {
    pub max_ops: Option<usize>,
    pub safety: SafetyOpts,
}

pub fn compile(source: SpecSource) -> Result<CompileBundle, CompileError> {
    compile_with(source, CompileOptions::default())
}

pub fn compile_with(
    source: SpecSource,
    opts: CompileOptions,
) -> Result<CompileBundle, CompileError> {
    let loaded = loader::load_with(source, opts.safety)?;
    compile_loaded(&loaded, opts)
}

pub fn compile_loaded(
    loaded: &LoadedSpec,
    opts: CompileOptions,
) -> Result<CompileBundle, CompileError> {
    let started = Instant::now();
    let (doc, spec_version) = parse::parse_to_value(&loaded.bytes, loaded.format, loaded.family)?;
    let retriever = Retriever::with_safety(opts.safety);
    let mut lowering = lower::collect(&doc, &retriever);
    if let Some(max) = opts.max_ops {
        lowering.drafts.truncate(max);
    }

    let candidates: Vec<String> = lowering
        .drafts
        .iter()
        .map(|d| d.candidate_name.clone())
        .collect();
    let keys: Vec<(String, String, String)> = lowering
        .drafts
        .iter()
        .map(|d| {
            (
                d.method.clone(),
                d.path.clone(),
                d.operation_id.clone().unwrap_or_default(),
            )
        })
        .collect();
    let names = names::uniquify(&candidates, &keys);
    for (draft, new_name) in lowering.drafts.iter().zip(names.iter()) {
        if new_name != &draft.candidate_name {
            lowering.warnings.push(lower::warn(
                WarningCode::NameCollision,
                draft.operation_id.clone(),
                Some(&draft.method),
                Some(&draft.path),
                format!("name collision; assigned {new_name}"),
            ));
        }
    }

    let mut operations = Vec::new();
    let mut extra_skipped = Vec::new();
    for (draft, name) in lowering.drafts.iter().zip(names) {
        match lower::lower_operation(&doc, draft, name, &retriever, &mut lowering.warnings) {
            Ok(op) => operations.push(op),
            Err(skip) => extra_skipped.push(skip),
        }
    }
    lowering.skipped.extend(extra_skipped);

    let compiled = operations.len() as u32;
    let enabled = operations.iter().filter(|o| o.enabled_by_default).count() as u32;
    if compiled > 128 {
        lowering.warnings.push(lower::warn(
            WarningCode::ToolExplosion,
            None,
            None,
            None,
            format!("{compiled} tools compiled"),
        ));
    } else if compiled > 64 {
        let mut w = lower::warn(
            WarningCode::ToolExplosion,
            None,
            None,
            None,
            format!("{compiled} tools compiled"),
        );
        w.severity = Severity::Info;
        lowering.warnings.push(w);
    }

    let title = doc
        .pointer("/info/title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Untitled")
        .to_owned();
    let description = doc
        .pointer("/info/description")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let document_url = match loaded.source.kind {
        SourceKind::Url => Some(loaded.source.locator.as_str()),
        SourceKind::File | SourceKind::Stdin => None,
    };
    let mut servers: Vec<Server> = doc
        .get("servers")
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().filter_map(lower::parse_server).collect())
        .unwrap_or_default();
    if servers.is_empty() && document_url.is_some() {
        servers.push(Server {
            url_template: "/".into(),
            variables: Default::default(),
            description: None,
        });
    }
    for server in &mut servers {
        server.url_template = resolve_server_url(document_url, &server.url_template);
    }

    let compile_ms = started.elapsed().as_millis() as u64;

    Ok(CompileBundle {
        api: NormalizedApi {
            ir_version: IR_VERSION.to_owned(),
            gateway: GatewayMeta {
                title: title.clone(),
                description,
                spec_version,
                source: loaded.source.clone(),
            },
            servers,
            security_schemes: lower::parse_security_schemes(&doc),
            operations,
        },
        report: AnalysisReport {
            spec_title: title,
            spec_version: loaded.spec_version.clone(),
            operations_total: lowering.operations_total,
            operations_compiled: compiled,
            operations_skipped: lowering.skipped.len() as u32,
            tools_enabled_by_default: enabled,
            compile_ms,
            warnings: lowering.warnings,
            skipped: lowering.skipped,
        },
    })
}

pub fn compile_path(path: impl AsRef<std::path::Path>) -> Result<CompileBundle, CompileError> {
    compile(SpecSource::File(path.as_ref().to_path_buf()))
}
