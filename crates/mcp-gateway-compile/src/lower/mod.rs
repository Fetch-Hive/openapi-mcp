//! Per-operation IR lowering.

mod body;
mod collect;
mod media;
mod params;
mod responses;
mod schema;
mod security;
mod servers;

use crate::category;
use crate::destructive;
use crate::refs::Retriever;
use mcp_gateway_ir::*;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

pub use collect::collect;
pub use security::parse_security_schemes;
pub use servers::parse_server;

pub struct Draft {
    pub method: String,
    pub path: String,
    pub operation_id: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub deprecated: bool,
    pub op: Value,
    pub path_item: Value,
    pub class: destructive::Classification,
    pub candidate_name: String,
}

pub struct SchemaSink<'a> {
    pub properties: &'a mut Map<String, Value>,
    pub required: &'a mut Vec<String>,
    pub defs: &'a mut Map<String, Value>,
    pub had_cycle: &'a mut bool,
    pub warnings: &'a mut Vec<Warning>,
    pub blocking_codes: &'a mut Vec<WarningCode>,
}

pub struct Lowering {
    pub drafts: Vec<Draft>,
    pub warnings: Vec<Warning>,
    pub skipped: Vec<SkippedOperation>,
    pub operations_total: u32,
}

pub fn lower_operation(
    doc: &Value,
    draft: &Draft,
    tool_name: String,
    retriever: &Retriever,
    warnings: &mut Vec<Warning>,
) -> Result<Operation, SkippedOperation> {
    let mut blocking_codes: Vec<WarningCode> = Vec::new();
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut defs = Map::new();
    let mut had_cycle = false;
    let mut sink = SchemaSink {
        properties: &mut properties,
        required: &mut required,
        defs: &mut defs,
        had_cycle: &mut had_cycle,
        warnings,
        blocking_codes: &mut blocking_codes,
    };

    let params_out = params::lower_parameters(doc, draft, retriever, &mut sink);

    let (request_body, body_binding) = body::lower_body(doc, draft, retriever, &mut sink);

    if !blocking_codes.is_empty() {
        return Err(SkippedOperation {
            method: draft.method.clone(),
            path: draft.path.clone(),
            operation_id: draft.operation_id.clone(),
            codes: blocking_codes,
        });
    }

    if had_cycle {
        warnings.push(warn(
            WarningCode::RecursiveSchema,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "circular $ref left in $defs".into(),
        ));
    }

    let mut input_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    });
    if !defs.is_empty() {
        if let Value::Object(map) = &mut input_schema {
            map.insert("$defs".into(), Value::Object(defs));
        }
    }
    if input_schema.to_string().len() > 64 * 1024 {
        warnings.push(warn(
            WarningCode::LargeSchema,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "inlined input_schema exceeds 64 KiB".into(),
        ));
    }

    let responses_out = responses::lower_responses(doc, draft, retriever, warnings);
    if let Some(codes) = responses_out.blocking {
        return Err(SkippedOperation {
            method: draft.method.clone(),
            path: draft.path.clone(),
            operation_id: draft.operation_id.clone(),
            codes,
        });
    }
    if !responses_out.json_response {
        warnings.push(warn(
            WarningCode::NoJsonResponse,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "success response is not JSON".into(),
        ));
    }
    if responses_out.output_schema.is_none() {
        warnings.push(warn(
            WarningCode::MissingResponseSchema,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "no JSON 2xx schema".into(),
        ));
    }

    let servers = servers::effective_servers(doc, &draft.path_item, &draft.op);
    if servers.is_empty() {
        warnings.push(warn(
            WarningCode::NoServers,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "no effective servers".into(),
        ));
    }

    let security = security::effective_security(doc, &draft.op);
    let auth_ok = security::security_supported(doc, &security);
    if !auth_ok {
        warnings.push(warn(
            WarningCode::AuthUnsupported,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "security scheme is not bearer/apiKey header".into(),
        ));
    }
    if draft.deprecated {
        warnings.push(warn(
            WarningCode::Deprecated,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "operation is deprecated".into(),
        ));
    }
    if draft.class.destructive {
        warnings.push(warn(
            WarningCode::Destructive,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "classified as destructive".into(),
        ));
    }

    let title = draft
        .summary
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| title_case(&tool_name));
    let description = draft
        .description
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| draft.summary.clone())
        .unwrap_or_else(|| format!("{} {}", draft.method, draft.path));
    if description.len() < 24 || description == tool_name {
        warnings.push(warn(
            WarningCode::WeakDescription,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "weak description".into(),
        ));
    }

    let schema_ref_name = first_schema_name(&draft.op);
    let category = category::category(&draft.tags, &draft.path, schema_ref_name.as_deref());
    let enabled_by_default = !draft.class.destructive && !draft.deprecated && auth_ok;

    Ok(Operation {
        id: operation_id(&draft.method, &draft.path),
        tool: McpTool {
            name: tool_name,
            title,
            description,
            input_schema,
            output_schema: responses_out.output_schema,
            annotations: destructive::annotations(draft.class),
        },
        source: OperationSource {
            operation_id: draft.operation_id.clone(),
            method: draft.method.clone(),
            path_template: draft.path.clone(),
        },
        http: HttpBinding {
            method: draft.method.clone(),
            path_template: draft.path.clone(),
            effective_servers: servers,
            parameters: params_out.ir_params,
            request_body,
            responses: responses_out.responses,
        },
        security,
        tags: draft.tags.clone(),
        category,
        deprecated: draft.deprecated,
        destructive: draft.class.destructive,
        enabled_by_default,
        execution_plan: ExecutionPlan {
            method: draft.method.clone(),
            path_template: draft.path.clone(),
            path_params: params_out.path_bindings,
            query_params: params_out.query_bindings,
            header_params: params_out.header_bindings,
            cookie_params: params_out.cookie_bindings,
            body: body_binding,
            accept: "application/json".into(),
            timeout_ms: 15_000,
        },
    })
}

pub fn warn(
    code: WarningCode,
    operation_id: Option<String>,
    method: Option<&str>,
    path: Option<&str>,
    message: String,
) -> Warning {
    Warning {
        severity: code.default_severity(),
        code,
        operation_id,
        method: method.map(str::to_owned),
        path: path.map(str::to_owned),
        message,
        pointer: None,
    }
}

pub fn skip(
    skipped: &mut Vec<SkippedOperation>,
    warnings: &mut Vec<Warning>,
    method: &str,
    path: &str,
    operation_id: Option<String>,
    code: WarningCode,
    message: &str,
) {
    warnings.push(warn(
        code,
        operation_id.clone(),
        Some(method),
        Some(path),
        message.into(),
    ));
    skipped.push(SkippedOperation {
        method: method.to_owned(),
        path: path.to_owned(),
        operation_id,
        codes: vec![code],
    });
}

pub fn operation_id(method: &str, path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update(b"\n");
    hasher.update(path.as_bytes());
    let hex = hex::encode(hasher.finalize());
    format!("op_{}", &hex[..16])
}

pub fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub fn string_array(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn title_case(name: &str) -> String {
    name.split('_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut c = s.chars();
            match c.next() {
                Some(f) => format!("{}{}", f.to_ascii_uppercase(), c.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn first_schema_name(op: &Value) -> Option<String> {
    let r = op
        .pointer("/requestBody/content/application~1json/schema/$ref")
        .or_else(|| op.pointer("/responses/200/content/application~1json/schema/$ref"))
        .and_then(Value::as_str)?;
    Some(r.rsplit('/').next().unwrap_or(r).to_owned())
}

pub fn apply_bundle(
    bundled: crate::refs::BundleOutcome,
    defs: &mut Map<String, Value>,
    had_cycle: &mut bool,
) -> Result<(Value, bool), Vec<String>> {
    *had_cycle |= bundled.had_cycle;
    if !bundled.unresolved.is_empty() {
        return Err(bundled.unresolved);
    }
    let mut schema = bundled.schema;
    schema::hoist_defs(&mut schema, defs);
    let leftover = schema::flatten_allof(&mut schema);
    Ok((schema, leftover))
}

pub fn note_schema(
    schema: &Value,
    leftover_allof: bool,
    draft: &Draft,
    warnings: &mut Vec<Warning>,
) {
    if leftover_allof || schema.get("allOf").is_some() {
        warnings.push(warn(
            WarningCode::AllOfUnflattened,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "allOf retained".into(),
        ));
    }
    if schema.get("discriminator").is_some() {
        warnings.push(warn(
            WarningCode::Discriminator,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "discriminator present".into(),
        ));
    }
}
