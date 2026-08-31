use super::{apply_bundle, note_schema, str_field, Draft, SchemaSink};
use crate::refs::{bundle_schema, resolve_openapi_object, Retriever};
use crate::style::{parse_style, StyleSupport};
use mcp_gateway_ir::*;
use serde_json::{json, Value};

pub struct ParamsOut {
    pub ir_params: Vec<Parameter>,
    pub path_bindings: Vec<ArgBinding>,
    pub query_bindings: Vec<ArgBinding>,
    pub header_bindings: Vec<ArgBinding>,
    pub cookie_bindings: Vec<ArgBinding>,
}

pub fn lower_parameters(
    doc: &Value,
    draft: &Draft,
    retriever: &Retriever,
    sink: &mut SchemaSink<'_>,
) -> ParamsOut {
    let mut out = ParamsOut {
        ir_params: Vec::new(),
        path_bindings: Vec::new(),
        query_bindings: Vec::new(),
        header_bindings: Vec::new(),
        cookie_bindings: Vec::new(),
    };
    let params = merge_parameters(
        doc,
        &draft.path_item,
        &draft.op,
        retriever,
        sink.warnings,
        draft,
    );

    for param in &params {
        let name = str_field(param, "name").unwrap_or_default();
        let location = str_field(param, "in").unwrap_or_default();
        if name.is_empty() {
            sink.blocking_codes.push(WarningCode::UnresolvedRef);
            sink.warnings.push(super::warn(
                WarningCode::UnresolvedRef,
                draft.operation_id.clone(),
                Some(&draft.method),
                Some(&draft.path),
                "parameter is missing name (unresolved $ref?)".into(),
            ));
            continue;
        }
        let mut required_flag = param
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if location == "path" {
            required_flag = true;
        }
        let loc = match location.as_str() {
            "path" => ParamLocation::Path,
            "header" => ParamLocation::Header,
            "cookie" => ParamLocation::Cookie,
            "query" => ParamLocation::Query,
            _ => {
                let code = WarningCode::UnsupportedStyle;
                sink.warnings.push(super::warn(
                    code,
                    draft.operation_id.clone(),
                    Some(&draft.method),
                    Some(&draft.path),
                    format!("unknown parameter in={location:?} for {name}"),
                ));
                if required_flag || location.is_empty() {
                    sink.blocking_codes.push(code);
                }
                continue;
            }
        };

        let style_raw = str_field(param, "style");
        let explode = param.get("explode").and_then(Value::as_bool);
        let support = parse_style(&location, style_raw.as_deref(), explode);
        let (style, explode_flag) = match support {
            StyleSupport::Ok(s, e) => (s, e),
            StyleSupport::Unsupported {
                blocking_if_required,
            } => {
                let sev = if blocking_if_required && required_flag {
                    sink.blocking_codes.push(WarningCode::UnsupportedStyle);
                    Severity::Blocking
                } else {
                    Severity::Warn
                };
                sink.warnings.push(Warning {
                    code: WarningCode::UnsupportedStyle,
                    severity: sev,
                    operation_id: draft.operation_id.clone(),
                    method: Some(draft.method.clone()),
                    path: Some(draft.path.clone()),
                    message: format!("unsupported style for parameter {name}"),
                    pointer: None,
                });
                continue;
            }
        };

        let schema = param
            .get("schema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "string"}));
        let bundled = bundle_schema(doc, &schema, retriever);
        let schema = match apply_bundle(bundled, sink.defs, sink.had_cycle) {
            Ok((s, leftover)) => {
                note_schema(&s, leftover, draft, sink.warnings);
                s
            }
            Err(_) => {
                sink.blocking_codes.push(WarningCode::UnresolvedRef);
                sink.warnings.push(super::warn(
                    WarningCode::UnresolvedRef,
                    draft.operation_id.clone(),
                    Some(&draft.method),
                    Some(&draft.path),
                    format!("unresolved $ref in parameter {name}"),
                ));
                continue;
            }
        };

        out.ir_params.push(Parameter {
            name: name.clone(),
            location: loc,
            required: required_flag,
            schema: schema.clone(),
            style,
            explode: explode_flag,
            description: str_field(param, "description"),
        });
        let binding = ArgBinding {
            arg_name: name.clone(),
            wire_name: name.clone(),
            style,
            explode: explode_flag,
            required: required_flag,
        };
        match loc {
            ParamLocation::Path => out.path_bindings.push(binding),
            ParamLocation::Query => out.query_bindings.push(binding),
            ParamLocation::Header => out.header_bindings.push(binding),
            ParamLocation::Cookie => out.cookie_bindings.push(binding),
        }
        sink.properties.insert(name.clone(), schema);
        if required_flag {
            sink.required.push(name);
        }
    }
    out
}

fn merge_parameters(
    doc: &Value,
    path_item: &Value,
    op: &Value,
    retriever: &Retriever,
    warnings: &mut Vec<Warning>,
    draft: &Draft,
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut push_resolved = |raw: &Value| match resolve_openapi_object(doc, raw, retriever) {
        Ok(v) => out.push(v),
        Err(_) => {
            warnings.push(super::warn(
                WarningCode::UnresolvedRef,
                draft.operation_id.clone(),
                Some(&draft.method),
                Some(&draft.path),
                "unresolved parameter $ref".into(),
            ));
            out.push(raw.clone());
        }
    };
    if let Some(arr) = path_item.get("parameters").and_then(Value::as_array) {
        for p in arr {
            push_resolved(p);
        }
    }
    if let Some(arr) = op.get("parameters").and_then(Value::as_array) {
        for p in arr {
            let resolved = match resolve_openapi_object(doc, p, retriever) {
                Ok(v) => v,
                Err(_) => {
                    warnings.push(super::warn(
                        WarningCode::UnresolvedRef,
                        draft.operation_id.clone(),
                        Some(&draft.method),
                        Some(&draft.path),
                        "unresolved parameter $ref".into(),
                    ));
                    p.clone()
                }
            };
            let name = str_field(&resolved, "name");
            let loc = str_field(&resolved, "in");
            out.retain(|existing| {
                str_field(existing, "name") != name || str_field(existing, "in") != loc
            });
            out.push(resolved);
        }
    }
    out
}
