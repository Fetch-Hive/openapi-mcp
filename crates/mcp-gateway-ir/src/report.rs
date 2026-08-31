use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisReport {
    pub spec_title: String,
    pub spec_version: String,
    pub operations_total: u32,
    pub operations_compiled: u32,
    pub operations_skipped: u32,
    pub tools_enabled_by_default: u32,
    pub compile_ms: u64,
    pub warnings: Vec<Warning>,
    pub skipped: Vec<SkippedOperation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Warning {
    pub code: WarningCode,
    pub severity: Severity,
    pub operation_id: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub message: String,
    pub pointer: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warn,
    Blocking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum WarningCode {
    #[serde(rename = "W_MISSING_OPERATION_ID")]
    MissingOperationId,
    #[serde(rename = "W_DUPLICATE_OPERATION_ID")]
    DuplicateOperationId,
    #[serde(rename = "W_WEAK_DESCRIPTION")]
    WeakDescription,
    #[serde(rename = "W_MISSING_REQUEST_SCHEMA")]
    MissingRequestSchema,
    #[serde(rename = "W_MISSING_RESPONSE_SCHEMA")]
    MissingResponseSchema,
    #[serde(rename = "W_UNSUPPORTED_STYLE")]
    UnsupportedStyle,
    #[serde(rename = "W_DEPRECATED")]
    Deprecated,
    #[serde(rename = "W_DESTRUCTIVE")]
    Destructive,
    #[serde(rename = "W_AUTH_UNSUPPORTED")]
    AuthUnsupported,
    #[serde(rename = "W_BINARY_BODY")]
    BinaryBody,
    #[serde(rename = "W_STREAMING")]
    Streaming,
    #[serde(rename = "W_LARGE_SCHEMA")]
    LargeSchema,
    #[serde(rename = "W_RECURSIVE_SCHEMA")]
    RecursiveSchema,
    #[serde(rename = "W_NO_SERVERS")]
    NoServers,
    #[serde(rename = "W_UNRESOLVED_REF")]
    UnresolvedRef,
    #[serde(rename = "W_UNSUPPORTED_METHOD")]
    UnsupportedMethod,
    #[serde(rename = "W_NAME_COLLISION")]
    NameCollision,
    #[serde(rename = "W_TOOL_EXPLOSION")]
    ToolExplosion,
    #[serde(rename = "W_ALLOF_UNFLATTENED")]
    AllOfUnflattened,
    #[serde(rename = "W_DISCRIMINATOR")]
    Discriminator,
    #[serde(rename = "W_NO_JSON_RESPONSE")]
    NoJsonResponse,
}

impl WarningCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingOperationId => "W_MISSING_OPERATION_ID",
            Self::DuplicateOperationId => "W_DUPLICATE_OPERATION_ID",
            Self::WeakDescription => "W_WEAK_DESCRIPTION",
            Self::MissingRequestSchema => "W_MISSING_REQUEST_SCHEMA",
            Self::MissingResponseSchema => "W_MISSING_RESPONSE_SCHEMA",
            Self::UnsupportedStyle => "W_UNSUPPORTED_STYLE",
            Self::Deprecated => "W_DEPRECATED",
            Self::Destructive => "W_DESTRUCTIVE",
            Self::AuthUnsupported => "W_AUTH_UNSUPPORTED",
            Self::BinaryBody => "W_BINARY_BODY",
            Self::Streaming => "W_STREAMING",
            Self::LargeSchema => "W_LARGE_SCHEMA",
            Self::RecursiveSchema => "W_RECURSIVE_SCHEMA",
            Self::NoServers => "W_NO_SERVERS",
            Self::UnresolvedRef => "W_UNRESOLVED_REF",
            Self::UnsupportedMethod => "W_UNSUPPORTED_METHOD",
            Self::NameCollision => "W_NAME_COLLISION",
            Self::ToolExplosion => "W_TOOL_EXPLOSION",
            Self::AllOfUnflattened => "W_ALLOF_UNFLATTENED",
            Self::Discriminator => "W_DISCRIMINATOR",
            Self::NoJsonResponse => "W_NO_JSON_RESPONSE",
        }
    }

    pub fn default_severity(self) -> Severity {
        match self {
            Self::MissingResponseSchema
            | Self::Deprecated
            | Self::RecursiveSchema
            | Self::AllOfUnflattened
            | Self::Discriminator
            | Self::NoJsonResponse => Severity::Info,
            Self::BinaryBody | Self::Streaming | Self::UnresolvedRef | Self::UnsupportedMethod => {
                Severity::Blocking
            }
            _ => Severity::Warn,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkippedOperation {
    pub method: String,
    pub path: String,
    pub operation_id: Option<String>,
    pub codes: Vec<WarningCode>,
}
