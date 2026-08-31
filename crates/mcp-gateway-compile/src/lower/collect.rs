use super::{skip, str_field, string_array, Draft, Lowering};
use crate::destructive;
use crate::names::{self, NameKind, NameSource};
use crate::refs::{resolve_openapi_object, Retriever};
use mcp_gateway_ir::*;
use serde_json::Value;
use std::collections::HashSet;

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

pub fn collect(doc: &Value, retriever: &Retriever) -> Lowering {
    let mut warnings = Vec::new();
    let mut skipped = Vec::new();
    let mut drafts = Vec::new();
    let mut operations_total = 0u32;
    let mut seen_ids: HashSet<String> = HashSet::new();

    skip_webhooks(doc, &mut operations_total, &mut skipped, &mut warnings);

    let Some(paths) = doc.get("paths").and_then(Value::as_object) else {
        return Lowering {
            drafts,
            warnings,
            skipped,
            operations_total,
        };
    };

    for (path, path_item) in paths {
        let path_item = match resolve_openapi_object(doc, path_item, retriever) {
            Ok(v) => v,
            Err(_) => {
                skip(
                    &mut skipped,
                    &mut warnings,
                    "GET",
                    path,
                    None,
                    WarningCode::UnresolvedRef,
                    "unresolved path item $ref",
                );
                operations_total += 1;
                continue;
            }
        };
        for method in METHODS {
            let Some(op) = path_item.get(method) else {
                continue;
            };
            if !op.is_object() {
                continue;
            }
            operations_total += 1;
            let method_u = method.to_ascii_uppercase();
            let mut op = op.clone();
            if let Some(body) = op.get("requestBody").cloned() {
                match resolve_openapi_object(doc, &body, retriever) {
                    Ok(resolved) => {
                        op.as_object_mut()
                            .expect("operation object")
                            .insert("requestBody".into(), resolved);
                    }
                    Err(_) => {
                        skip(
                            &mut skipped,
                            &mut warnings,
                            &method_u,
                            path,
                            str_field(&op, "operationId"),
                            WarningCode::UnresolvedRef,
                            "unresolved requestBody $ref",
                        );
                        continue;
                    }
                }
            }
            if let Some(Value::Object(responses)) = op.get("responses").cloned() {
                let mut resolved_map = serde_json::Map::new();
                let mut failed = false;
                for (status, resp) in responses {
                    match resolve_openapi_object(doc, &resp, retriever) {
                        Ok(v) => {
                            resolved_map.insert(status, v);
                        }
                        Err(_) => {
                            failed = true;
                            skip(
                                &mut skipped,
                                &mut warnings,
                                &method_u,
                                path,
                                str_field(&op, "operationId"),
                                WarningCode::UnresolvedRef,
                                "unresolved response $ref",
                            );
                            break;
                        }
                    }
                }
                if failed {
                    continue;
                }
                op.as_object_mut()
                    .expect("operation object")
                    .insert("responses".into(), Value::Object(resolved_map));
            }

            let operation_id = str_field(&op, "operationId");
            let summary = str_field(&op, "summary");
            let description = str_field(&op, "description");
            let tags = string_array(op.get("tags"));
            let deprecated = op
                .get("deprecated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let class = destructive::classify(&method_u, path, operation_id.as_deref());

            if class.skip {
                skip(
                    &mut skipped,
                    &mut warnings,
                    &method_u,
                    path,
                    operation_id.clone(),
                    WarningCode::UnsupportedMethod,
                    "method is not compiled",
                );
                continue;
            }
            if super::media::is_streaming(&op) {
                skip(
                    &mut skipped,
                    &mut warnings,
                    &method_u,
                    path,
                    operation_id.clone(),
                    WarningCode::Streaming,
                    "streaming / websocket / event-stream is not compiled",
                );
                continue;
            }
            if super::media::is_binary_body(&op) {
                skip(
                    &mut skipped,
                    &mut warnings,
                    &method_u,
                    path,
                    operation_id.clone(),
                    WarningCode::BinaryBody,
                    "binary or multipart request body is not compiled",
                );
                continue;
            }
            if let Some(id) = &operation_id {
                if !seen_ids.insert(id.clone()) {
                    warnings.push(super::warn(
                        WarningCode::DuplicateOperationId,
                        operation_id.clone(),
                        Some(&method_u),
                        Some(path),
                        format!("duplicate operationId {id}"),
                    ));
                }
            }
            let named = names::candidate_name(&NameSource {
                operation_id: operation_id.as_deref(),
                summary: summary.as_deref(),
                description: description.as_deref(),
                method: &method_u,
                path_template: path,
            });
            if named.source_kind != NameKind::OperationId {
                warnings.push(super::warn(
                    WarningCode::MissingOperationId,
                    operation_id.clone(),
                    Some(&method_u),
                    Some(path),
                    "no operationId; name synthesised".into(),
                ));
            }
            drafts.push(Draft {
                method: method_u,
                path: path.clone(),
                operation_id,
                summary,
                description,
                tags,
                deprecated,
                op,
                path_item: path_item.clone(),
                class,
                candidate_name: named.name,
            });
        }
    }

    Lowering {
        drafts,
        warnings,
        skipped,
        operations_total,
    }
}

fn skip_webhooks(
    doc: &Value,
    operations_total: &mut u32,
    skipped: &mut Vec<SkippedOperation>,
    warnings: &mut Vec<Warning>,
) {
    let Some(hooks) = doc.get("webhooks").and_then(Value::as_object) else {
        return;
    };
    for (name, item) in hooks {
        for method in METHODS {
            if item.get(method).is_some_and(Value::is_object) {
                *operations_total += 1;
                skip(
                    skipped,
                    warnings,
                    &method.to_ascii_uppercase(),
                    &format!("webhooks/{name}"),
                    None,
                    WarningCode::Streaming,
                    "webhook is not compiled",
                );
            }
        }
    }
}
