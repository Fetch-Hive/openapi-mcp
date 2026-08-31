use super::{apply_bundle, note_schema, Draft, SchemaSink};
use crate::refs::{bundle_schema, Retriever};
use mcp_gateway_ir::*;
use serde_json::{json, Value};

pub fn lower_body(
    doc: &Value,
    draft: &Draft,
    retriever: &Retriever,
    sink: &mut SchemaSink<'_>,
) -> (Option<RequestBody>, Option<BodyBinding>) {
    let Some(body) = draft.op.get("requestBody") else {
        return (None, None);
    };
    let content = body.get("content").and_then(Value::as_object);
    let Some(content) = content else {
        sink.warnings.push(super::warn(
            WarningCode::MissingRequestSchema,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "request body has no content".into(),
        ));
        return (None, None);
    };
    let selected = super::media::select_json_content(content);
    let Some((content_type, media)) = selected else {
        return (None, None);
    };
    let required_flag = body
        .get("required")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let schema = media.get("schema").cloned();
    if schema.is_none() && required_flag {
        sink.warnings.push(super::warn(
            WarningCode::MissingRequestSchema,
            draft.operation_id.clone(),
            Some(&draft.method),
            Some(&draft.path),
            "body required but schema absent".into(),
        ));
    }
    let schema = schema.unwrap_or_else(|| json!({"type": "object"}));
    let bundled = bundle_schema(doc, &schema, retriever);
    let bundled_schema = match apply_bundle(bundled, sink.defs, sink.had_cycle) {
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
                "unresolved $ref in request body".into(),
            ));
            return (None, None);
        }
    };

    let form = super::media::media_type_base(&content_type) == "application/x-www-form-urlencoded";
    let is_object = bundled_schema.get("type").and_then(Value::as_str) == Some("object")
        || bundled_schema.get("properties").is_some();

    if is_object && !form {
        let mut folded_keys = Vec::new();
        if let Some(props) = bundled_schema.get("properties").and_then(Value::as_object) {
            for (key, val) in props {
                let mut arg = key.clone();
                if sink.properties.contains_key(&arg) {
                    arg = format!("body_{key}");
                    if sink.properties.contains_key(&arg) {
                        let mut n = 2u32;
                        loop {
                            let candidate = format!("body_{key}_{n}");
                            if !sink.properties.contains_key(&candidate) {
                                arg = candidate;
                                break;
                            }
                            n += 1;
                        }
                    }
                    sink.warnings.push(super::warn(
                        WarningCode::NameCollision,
                        draft.operation_id.clone(),
                        Some(&draft.method),
                        Some(&draft.path),
                        format!("body property {key} collided; renamed {arg}"),
                    ));
                }
                sink.properties.insert(arg.clone(), val.clone());
                folded_keys.push(FoldedKey {
                    arg_name: arg,
                    wire_name: key.clone(),
                });
            }
        }
        if let Some(req) = bundled_schema.get("required").and_then(Value::as_array) {
            for r in req {
                if let Some(wire) = r.as_str() {
                    let name = folded_keys
                        .iter()
                        .find(|k| k.wire_name == wire)
                        .map(|k| k.arg_name.clone())
                        .unwrap_or_else(|| wire.to_owned());
                    if !sink.required.contains(&name) {
                        sink.required.push(name);
                    }
                }
            }
        }
        (
            Some(RequestBody {
                required: required_flag,
                content_type: content_type.clone(),
                schema: bundled_schema,
                folded_into_input: true,
            }),
            Some(BodyBinding {
                mode: BodyMode::Fold,
                content_type,
                arg_name: None,
                folded_keys,
            }),
        )
    } else {
        sink.properties
            .insert("body".into(), bundled_schema.clone());
        if required_flag {
            sink.required.push("body".into());
        }
        let mode = if form {
            BodyMode::FormUrlencoded
        } else {
            BodyMode::Whole
        };
        (
            Some(RequestBody {
                required: required_flag,
                content_type: content_type.clone(),
                schema: bundled_schema,
                folded_into_input: false,
            }),
            Some(BodyBinding {
                mode,
                content_type,
                arg_name: Some("body".into()),
                folded_keys: vec![],
            }),
        )
    }
}
