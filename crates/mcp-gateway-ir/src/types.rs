use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompileBundle {
    pub api: NormalizedApi,
    pub report: crate::AnalysisReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NormalizedApi {
    pub ir_version: String,
    pub gateway: GatewayMeta,
    pub servers: Vec<Server>,
    pub security_schemes: BTreeMap<String, SecurityScheme>,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GatewayMeta {
    pub title: String,
    pub description: Option<String>,
    pub spec_version: String,
    pub source: SourceMeta,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceMeta {
    pub kind: SourceKind,
    pub locator: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    File,
    Url,
    Stdin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub url_template: String,
    pub variables: BTreeMap<String, ServerVariable>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServerVariable {
    pub default: Option<String>,
    pub enum_values: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecurityScheme {
    ApiKey {
        name: String,
        location: ApiKeyLocation,
    },
    Http {
        scheme: String,
        bearer_format: Option<String>,
    },
    OAuth2 {
        flows: Value,
    },
    OpenIdConnect {
        open_id_connect_url: String,
    },
    MutualTls,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyLocation {
    Header,
    Query,
    Cookie,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub id: String,
    pub tool: McpTool,
    pub source: OperationSource,
    pub http: HttpBinding,
    pub security: Vec<SecurityRequirement>,
    pub tags: Vec<String>,
    pub category: String,
    pub deprecated: bool,
    pub destructive: bool,
    pub enabled_by_default: bool,
    pub execution_plan: ExecutionPlan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OperationSource {
    pub operation_id: Option<String>,
    pub method: String,
    pub path_template: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpTool {
    pub name: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    pub annotations: ToolAnnotations,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolAnnotations {
    pub read_only_hint: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive_hint: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent_hint: Option<bool>,
    pub open_world_hint: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpBinding {
    pub method: String,
    pub path_template: String,
    pub effective_servers: Vec<Server>,
    pub parameters: Vec<Parameter>,
    pub request_body: Option<RequestBody>,
    pub responses: BTreeMap<String, ResponseBody>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub location: ParamLocation,
    pub required: bool,
    pub schema: Value,
    pub style: Style,
    pub explode: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParamLocation {
    Path,
    Query,
    Header,
    Cookie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    Simple,
    Form,
    Label,
    Matrix,
    SpaceDelimited,
    PipeDelimited,
    DeepObject,
}

impl Style {
    /// Serialise a JSON value onto the wire for this style. Pure; no I/O.
    pub fn serialise(self, explode: bool, name: &str, value: &Value) -> Vec<(String, String)> {
        match self {
            Self::Simple => vec![(name.to_owned(), simple(value))],
            Self::Form => form(name, value, explode),
            Self::SpaceDelimited => vec![(name.to_owned(), join(value, " "))],
            Self::PipeDelimited => vec![(name.to_owned(), join(value, "|"))],
            Self::Label => vec![(name.to_owned(), format!(".{}", simple(value)))],
            Self::Matrix => vec![(name.to_owned(), format!(";{name}={}", simple(value)))],
            Self::DeepObject => vec![],
        }
    }
}

fn simple(value: &Value) -> String {
    match value {
        Value::Array(items) => items.iter().map(scalar).collect::<Vec<_>>().join(","),
        Value::Object(map) => map
            .iter()
            .flat_map(|(k, v)| [k.clone(), scalar(v)])
            .collect::<Vec<_>>()
            .join(","),
        other => scalar(other),
    }
}

fn form(name: &str, value: &Value, explode: bool) -> Vec<(String, String)> {
    match value {
        Value::Array(items) if explode => {
            items.iter().map(|v| (name.to_owned(), scalar(v))).collect()
        }
        Value::Array(_) => vec![(name.to_owned(), join(value, ","))],
        Value::Object(map) if explode => map.iter().map(|(k, v)| (k.clone(), scalar(v))).collect(),
        Value::Object(map) => vec![(
            name.to_owned(),
            map.iter()
                .flat_map(|(k, v)| [k.clone(), scalar(v)])
                .collect::<Vec<_>>()
                .join(","),
        )],
        other => vec![(name.to_owned(), scalar(other))],
    }
}

fn join(value: &Value, sep: &str) -> String {
    match value {
        Value::Array(items) => items.iter().map(scalar).collect::<Vec<_>>().join(sep),
        other => scalar(other),
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestBody {
    pub required: bool,
    pub content_type: String,
    pub schema: Value,
    pub folded_into_input: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResponseBody {
    pub content_type: Option<String>,
    pub schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecurityRequirement {
    pub schemes: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPlan {
    pub method: String,
    #[serde(alias = "path")]
    pub path_template: String,
    pub path_params: Vec<ArgBinding>,
    pub query_params: Vec<ArgBinding>,
    pub header_params: Vec<ArgBinding>,
    pub cookie_params: Vec<ArgBinding>,
    pub body: Option<BodyBinding>,
    pub accept: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArgBinding {
    pub arg_name: String,
    pub wire_name: String,
    pub style: Style,
    pub explode: bool,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BodyBinding {
    pub mode: BodyMode,
    pub content_type: String,
    pub arg_name: Option<String>,
    /// MCP argument name → JSON object key used on the wire.
    pub folded_keys: Vec<FoldedKey>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FoldedKey {
    pub arg_name: String,
    pub wire_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BodyMode {
    Fold,
    Whole,
    FormUrlencoded,
}
