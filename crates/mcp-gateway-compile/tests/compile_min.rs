use mcp_gateway_compile::{compile, SpecSource};
use mcp_gateway_ir::SourceKind;
use std::io::Write;

#[test]
fn compiles_minimal_github_like_spec() {
    let yaml = r#"
openapi: 3.0.3
info:
  title: GitHub REST API
  version: "1.1.4"
servers:
  - url: https://api.github.com
paths:
  /repos/{owner}/{repo}:
    get:
      tags: [repos]
      summary: Get a repository
      description: The parent and source objects are present when the repository is a fork.
      operationId: repos/get
      parameters:
        - name: owner
          in: path
          required: true
          schema: { type: string }
        - name: repo
          in: path
          required: true
          schema: { type: string }
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema:
                type: object
      security:
        - bearerAuth: []
components:
  securitySchemes:
    bearerAuth:
      type: http
      scheme: bearer
"#;
    let mut path = std::env::temp_dir();
    path.push("mcp-gateway-min.yaml");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(yaml.as_bytes()).unwrap();
    drop(f);

    let bundle = compile(SpecSource::File(path.clone())).unwrap();
    assert_eq!(bundle.api.ir_version, "1.0");
    assert_eq!(bundle.api.operations.len(), 1);
    let op = &bundle.api.operations[0];
    assert_eq!(op.tool.name, "repos_get");
    assert_eq!(op.category, "repos");
    assert!(op.enabled_by_default);
    assert!(op.id.starts_with("op_"));
    assert_eq!(op.execution_plan.path_params.len(), 2);
    let _ = SourceKind::File;
}

#[test]
fn compiles_component_parameter_refs() {
    let yaml = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
servers:
  - url: https://api.example.com/{region}
    variables:
      region:
        default: us
paths:
  /repos/{owner}:
    get:
      operationId: getRepo
      parameters:
        - $ref: "#/components/parameters/Owner"
      responses:
        "200":
          description: OK
          content:
            application/json; charset=utf-8:
              schema: { type: object }
components:
  parameters:
    Owner:
      name: owner
      in: path
      required: true
      schema: { type: string }
"##;
    let mut path = std::env::temp_dir();
    path.push("mcp-gateway-param-ref.yaml");
    std::fs::write(&path, yaml).unwrap();
    let bundle = compile(SpecSource::File(path)).unwrap();
    assert_eq!(bundle.api.operations.len(), 1);
    let op = &bundle.api.operations[0];
    assert_eq!(op.execution_plan.path_params.len(), 1);
    assert_eq!(op.execution_plan.path_params[0].wire_name, "owner");
    assert_eq!(
        bundle.api.servers[0].variables["region"].default.as_deref(),
        Some("us")
    );
}

#[test]
fn body_fold_collision_keeps_wire_name() {
    let yaml = r#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /item/{id}:
    post:
      operationId: updateItem
      parameters:
        - name: id
          in: path
          required: true
          schema: { type: string }
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              required: [id]
              properties:
                id: { type: string }
                name: { type: string }
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema: { type: object }
"#;
    let mut path = std::env::temp_dir();
    path.push("mcp-gateway-fold.yaml");
    std::fs::write(&path, yaml).unwrap();
    let bundle = compile(SpecSource::File(path)).unwrap();
    let op = &bundle.api.operations[0];
    let body = op.execution_plan.body.as_ref().unwrap();
    let id = body
        .folded_keys
        .iter()
        .find(|k| k.wire_name == "id")
        .expect("id wire key");
    assert_eq!(id.arg_name, "body_id");
    assert!(op.tool.input_schema["properties"].get("body_id").is_some());
    assert!(op.tool.input_schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str() == Some("body_id")));
    assert!(bundle
        .report
        .warnings
        .iter()
        .any(|w| w.code == mcp_gateway_ir::WarningCode::NameCollision));
}

#[test]
fn description_mentioning_sse_is_not_skipped() {
    let yaml = r#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /plain:
    get:
      operationId: getPlain
      description: Mentions text/event-stream in prose but returns JSON.
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema: { type: object }
"#;
    let mut path = std::env::temp_dir();
    path.push("mcp-gateway-sse-prose.yaml");
    std::fs::write(&path, yaml).unwrap();
    let bundle = compile(SpecSource::File(path)).unwrap();
    assert_eq!(bundle.api.operations.len(), 1);
}

#[test]
fn unknown_in_is_a_parse_error() {
    let yaml = r#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /x:
    get:
      operationId: getX
      parameters:
        - name: q
          in: mystery
          schema: { type: string }
      responses:
        "200":
          description: OK
          content:
            application/json:
              schema: { type: object }
"#;
    let mut path = std::env::temp_dir();
    path.push("mcp-gateway-bad-in.yaml");
    std::fs::write(&path, yaml).unwrap();
    let err = compile(SpecSource::File(path)).unwrap_err();
    assert!(
        err.to_string().contains("parse") || err.to_string().contains("oas3"),
        "{err}"
    );
}

#[test]
fn broken_spec_is_parse_error() {
    let yaml = "not: openapi\n";
    let mut path = std::env::temp_dir();
    path.push("mcp-gateway-broken.yaml");
    std::fs::write(&path, yaml).unwrap();
    let err = compile(SpecSource::File(path)).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("parse")
            || msg.contains("openapi")
            || msg.contains("oas3")
            || msg.contains("openapiv3"),
        "{msg}"
    );
}

#[test]
fn delete_disabled_by_default() {
    let yaml = r#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /item/{id}:
    delete:
      operationId: deleteItem
      parameters:
        - name: id
          in: path
          required: true
          schema: { type: string }
      responses:
        "204": { description: gone }
"#;
    let mut path = std::env::temp_dir();
    path.push("mcp-gateway-del.yaml");
    std::fs::write(&path, yaml).unwrap();
    let bundle = compile(SpecSource::File(path)).unwrap();
    assert_eq!(bundle.api.operations.len(), 1);
    assert!(!bundle.api.operations[0].enabled_by_default);
    assert!(bundle.api.operations[0].destructive);
}
