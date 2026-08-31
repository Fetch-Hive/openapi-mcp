use super::{apply_bundle, note_schema, Draft};
use crate::refs::{bundle_schema, Retriever};
use mcp_gateway_ir::*;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub struct ResponsesOut {
    pub responses: BTreeMap<String, ResponseBody>,
    pub output_schema: Option<Value>,
    pub json_response: bool,
    pub blocking: Option<Vec<WarningCode>>,
}

pub fn lower_responses(
    doc: &Value,
    draft: &Draft,
    retriever: &Retriever,
    warnings: &mut Vec<Warning>,
) -> ResponsesOut {
    let mut out = BTreeMap::new();
    let mut json_ok = false;
    let mut blocking = Vec::new();
    let mut defs = Map::new();
    let mut had_cycle = false;
    let Some(responses) = draft.op.get("responses").and_then(Value::as_object) else {
        return ResponsesOut {
            responses: out,
            output_schema: None,
            json_response: false,
            blocking: None,
        };
    };
    for (status, resp) in responses {
        let content = resp.get("content").and_then(Value::as_object);
        let selected = content.and_then(super::media::select_json_content);
        if selected.is_some() {
            json_ok = json_ok || status.starts_with('2') || status == "default";
        }
        let (ct, schema) = if let Some((ct, media)) = selected {
            let schema = media.get("schema").cloned();
            if let Some(s) = schema {
                let bundled = bundle_schema(doc, &s, retriever);
                match apply_bundle(bundled, &mut defs, &mut had_cycle) {
                    Ok((schema, leftover)) => {
                        note_schema(&schema, leftover, draft, warnings);
                        (Some(ct), Some(schema))
                    }
                    Err(_) => {
                        blocking.push(WarningCode::UnresolvedRef);
                        warnings.push(super::warn(
                            WarningCode::UnresolvedRef,
                            draft.operation_id.clone(),
                            Some(&draft.method),
                            Some(&draft.path),
                            format!("unresolved $ref in response {status}"),
                        ));
                        (Some(ct), None)
                    }
                }
            } else {
                (Some(ct), None)
            }
        } else {
            (None, None)
        };
        out.insert(
            status.clone(),
            ResponseBody {
                content_type: ct,
                schema,
            },
        );
    }
    let output = ["200", "201"]
        .into_iter()
        .chain(
            out.keys()
                .filter(|k| k.starts_with('2'))
                .map(String::as_str),
        )
        .find_map(|k| out.get(k).and_then(|r| r.schema.clone()));
    ResponsesOut {
        responses: out,
        output_schema: output,
        json_response: json_ok,
        blocking: if blocking.is_empty() {
            None
        } else {
            Some(blocking)
        },
    }
}
