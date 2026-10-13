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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::load_bytes;
    use serde_json::json;

    #[test]
    fn response_ref_keeps_component_defs_on_the_output_schema() {
        let spec = r##"
openapi: 3.0.3
info:
  title: Pets
  version: "1.0.0"
servers:
  - url: https://example.com
paths:
  /pets:
    get:
      operationId: findPets
      responses:
        "200":
          description: A list of pets.
          content:
            application/json:
              schema:
                type: array
                items:
                  $ref: "#/components/schemas/Pet"
  /pets/{petId}:
    get:
      operationId: getPet
      parameters:
        - name: petId
          in: path
          required: true
          schema:
            type: integer
      responses:
        "200":
          description: One pet by id.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/Pet"
components:
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        id:
          type: integer
        name:
          type: string
        category:
          $ref: "#/components/schemas/Category"
    Category:
      type: object
      properties:
        name:
          type: string
"##;
        let loaded =
            load_bytes(spec.as_bytes().to_vec(), SourceKind::Stdin, "pets.yaml").expect("load");
        let bundle = compile_loaded(&loaded, CompileOptions::default()).expect("compile");
        let find = operation(&bundle, "/pets");
        let get = operation(&bundle, "/pets/{petId}");
        for op in [find, get] {
            let schema = op.tool.output_schema.as_ref().expect("output schema");
            let defs = schema
                .get("$defs")
                .and_then(serde_json::Value::as_object)
                .expect("output schema keeps $defs");
            assert!(defs.contains_key("Pet"), "{schema}");
            assert!(defs.contains_key("Category"), "{schema}");
            jsonschema::options()
                .build(schema)
                .expect("output schema builds");
        }

        let pet = get.tool.output_schema.as_ref().unwrap();
        assert!(valid(
            pet,
            &json!({"id": 1, "name": "doggie", "category": {"name": "Dogs"}})
        ));
        assert!(!valid(pet, &json!({"id": 1})));

        let list = find.tool.output_schema.as_ref().unwrap();
        assert!(valid(list, &json!([{"name": "doggie"}])));
        assert!(!valid(list, &json!([{"id": 1}])));
    }

    fn operation<'a>(bundle: &'a CompileBundle, path: &str) -> &'a Operation {
        bundle
            .api
            .operations
            .iter()
            .find(|op| op.source.path_template == path)
            .unwrap_or_else(|| panic!("missing {path}"))
    }

    fn valid(schema: &serde_json::Value, instance: &serde_json::Value) -> bool {
        jsonschema::options()
            .build(schema)
            .expect("schema builds")
            .validate(instance)
            .is_ok()
    }
}
