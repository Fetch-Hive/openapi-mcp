//! Versioned Normalized API Definition (IR) for MCP Gateway.
//!
//! No I/O. A TypeScript compiler can emit JSON that these types deserialise.
//! Unknown fields are denied so a future compiler cannot silently drift.

mod report;
mod types;

pub use report::*;
pub use types::*;

pub const IR_VERSION: &str = "1.0";

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn sample_bundle() -> CompileBundle {
        CompileBundle {
            api: NormalizedApi {
                ir_version: IR_VERSION.to_owned(),
                gateway: GatewayMeta {
                    title: "GitHub REST API".into(),
                    description: Some("example".into()),
                    spec_version: "3.0.3".into(),
                    source: SourceMeta {
                        kind: SourceKind::File,
                        locator: "spec.yaml".into(),
                        sha256: "a".repeat(64),
                    },
                },
                servers: vec![Server {
                    url_template: "https://api.github.com".into(),
                    variables: BTreeMap::new(),
                    description: None,
                }],
                security_schemes: BTreeMap::from([(
                    "bearerAuth".into(),
                    SecurityScheme::Http {
                        scheme: "bearer".into(),
                        bearer_format: None,
                    },
                )]),
                operations: vec![Operation {
                    id: "op_a1b2c3d4e5f60718".into(),
                    tool: McpTool {
                        name: "repos_get".into(),
                        title: "Get a repository".into(),
                        description: "Get a repository.".into(),
                        input_schema: json!({
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["owner", "repo"],
                            "properties": {
                                "owner": { "type": "string" },
                                "repo": { "type": "string" }
                            }
                        }),
                        output_schema: None,
                        annotations: ToolAnnotations {
                            read_only_hint: true,
                            destructive_hint: None,
                            idempotent_hint: None,
                            open_world_hint: true,
                        },
                    },
                    source: OperationSource {
                        operation_id: Some("repos/get".into()),
                        method: "GET".into(),
                        path_template: "/repos/{owner}/{repo}".into(),
                    },
                    http: HttpBinding {
                        method: "GET".into(),
                        path_template: "/repos/{owner}/{repo}".into(),
                        effective_servers: vec![],
                        parameters: vec![],
                        request_body: None,
                        responses: BTreeMap::new(),
                    },
                    security: vec![SecurityRequirement {
                        schemes: BTreeMap::from([("bearerAuth".into(), vec![])]),
                    }],
                    tags: vec!["repos".into()],
                    category: "repos".into(),
                    deprecated: false,
                    destructive: false,
                    enabled_by_default: true,
                    execution_plan: ExecutionPlan {
                        method: "GET".into(),
                        path_template: "/repos/{owner}/{repo}".into(),
                        path_params: vec![
                            ArgBinding {
                                arg_name: "owner".into(),
                                wire_name: "owner".into(),
                                style: Style::Simple,
                                explode: false,
                                required: true,
                            },
                            ArgBinding {
                                arg_name: "repo".into(),
                                wire_name: "repo".into(),
                                style: Style::Simple,
                                explode: false,
                                required: true,
                            },
                        ],
                        query_params: vec![],
                        header_params: vec![],
                        cookie_params: vec![],
                        body: None,
                        accept: "application/json".into(),
                        timeout_ms: 15_000,
                    },
                }],
            },
            report: AnalysisReport {
                spec_title: "GitHub REST API".into(),
                spec_version: "3.0.3".into(),
                operations_total: 1,
                operations_compiled: 1,
                operations_skipped: 0,
                tools_enabled_by_default: 1,
                compile_ms: 42,
                warnings: vec![],
                skipped: vec![],
            },
        }
    }

    use std::collections::BTreeMap;

    #[test]
    fn ir_version_is_1_0() {
        assert_eq!(IR_VERSION, "1.0");
    }

    #[test]
    fn serde_round_trip() {
        let bundle = sample_bundle();
        let json = serde_json::to_string_pretty(&bundle).unwrap();
        let back: CompileBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(bundle, back);
        assert_eq!(back.api.ir_version, "1.0");
    }

    #[test]
    fn deny_unknown_fields() {
        let mut value = serde_json::to_value(sample_bundle()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(true));
        let err = serde_json::from_value::<CompileBundle>(value).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "expected unknown field error, got {err}"
        );
    }

    #[test]
    fn deny_unknown_fields_on_operation() {
        let bundle = sample_bundle();
        let mut value = serde_json::to_value(&bundle.api.operations[0]).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("sneaky".into(), json!(1));
        let err = serde_json::from_value::<Operation>(value).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn schema_file_is_valid_json() {
        let raw = include_str!("../schema/ir.v1.schema.json");
        let schema: Value = serde_json::from_str(raw).expect("schema must be JSON");
        assert_eq!(schema["title"], "CompileBundle");
        assert_eq!(
            schema["$id"],
            "https://fetchhive.internal/mcp-gateway/ir/1.0"
        );
        let report = &schema["$defs"]["report"];
        assert!(
            report["properties"].is_object(),
            "report must list properties"
        );
        assert!(report["properties"]["spec_title"].is_object());
    }

    #[test]
    fn style_form_explode_serialises_array() {
        let pairs = Style::Form.serialise(true, "id", &json!([1, 2]));
        assert_eq!(
            pairs,
            vec![("id".into(), "1".into()), ("id".into(), "2".into())]
        );
    }
}
