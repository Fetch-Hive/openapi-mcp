//! HTTP response → MCP tool result mapping (Phase 2 spec §4.5.2–§4.5.3).

use crate::error::ProxyError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const ERROR_BODY_CAP: usize = 2 * 1024;
const IMAGE_CAP: usize = 512 * 1024;
const TEXT_CAP: usize = 32 * 1024;

const REDACT_KEYS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-api_key",
    "access_token",
    "refresh_token",
    "password",
    "secret",
    "api_key",
    "apikey",
    "private_key",
    "client_secret",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseKind {
    #[default]
    Json,
    Text,
    Image,
}

#[derive(Debug, Clone)]
pub struct UpstreamResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub is_error: bool,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

pub fn map_success(
    upstream: &UpstreamResponse,
    kind: ResponseKind,
    output_schema: Option<&Value>,
) -> ToolResult {
    if !(200..300).contains(&upstream.status) {
        return map_upstream_error(upstream);
    }
    match kind {
        ResponseKind::Image
            if upstream
                .content_type
                .as_deref()
                .is_some_and(|ct| ct.starts_with("image/")) =>
        {
            if upstream.body.len() > IMAGE_CAP {
                return error_result("too_large", "Upstream response too large");
            }
            ToolResult {
                is_error: false,
                text: format!("image ({} bytes)", upstream.body.len()),
                structured: None,
                image_b64: Some(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &upstream.body,
                )),
                error_code: None,
            }
        }
        ResponseKind::Json | ResponseKind::Image | ResponseKind::Text => {
            let raw = truncate_text(&upstream.body, TEXT_CAP);
            if let Ok(value) = serde_json::from_slice::<Value>(&upstream.body) {
                if let Some(schema) = output_schema {
                    if let Err(path) = validate_schema(schema, &value) {
                        return error_result("schema_output", &format!("Invalid output: {path}"));
                    }
                }
                ToolResult {
                    is_error: false,
                    text: serde_json::to_string(&value).unwrap_or(raw),
                    structured: Some(value),
                    image_b64: None,
                    error_code: None,
                }
            } else if output_schema.is_some() {
                error_result("schema_output", "Invalid output: $")
            } else {
                ToolResult {
                    is_error: false,
                    text: raw,
                    structured: None,
                    image_b64: None,
                    error_code: None,
                }
            }
        }
    }
}

pub fn map_proxy_error(err: &ProxyError, extra_body: Option<&[u8]>) -> ToolResult {
    let mut text = err.customer_message();
    if let Some(body) = extra_body {
        let snippet = sanitize_error_body(body);
        if !snippet.is_empty() {
            text.push_str(": ");
            text.push_str(&snippet);
        }
    }
    ToolResult {
        is_error: true,
        text,
        structured: Some(json!({ "error_code": err.error_code() })),
        image_b64: None,
        error_code: Some(err.error_code().to_owned()),
    }
}

pub fn map_upstream_error(upstream: &UpstreamResponse) -> ToolResult {
    let code = if upstream.status >= 500 {
        "upstream_5xx"
    } else {
        "upstream_4xx"
    };
    let mut text = format!("Upstream returned HTTP {}", upstream.status);
    let snippet = sanitize_error_body(&upstream.body);
    if !snippet.is_empty() {
        text.push_str(": ");
        text.push_str(&snippet);
    }
    ToolResult {
        is_error: true,
        text,
        structured: Some(json!({ "error_code": code })),
        image_b64: None,
        error_code: Some(code.into()),
    }
}

pub fn error_result(code: &str, text: &str) -> ToolResult {
    ToolResult {
        is_error: true,
        text: text.to_owned(),
        structured: Some(json!({ "error_code": code })),
        image_b64: None,
        error_code: Some(code.to_owned()),
    }
}

pub fn validate_schema(schema: &Value, instance: &Value) -> Result<(), String> {
    jsonschema::options()
        .with_retriever(DenyRemoteRefs)
        .build(schema)
        .map_err(|e| e.to_string())?
        .validate(instance)
        .map_err(|e| e.to_string())
}

/// Runtime schemas must already be bundled. External `$ref` would otherwise
/// fetch via jsonschema's default HTTP/file retriever and skip SSRF pinning.
struct DenyRemoteRefs;

impl jsonschema::Retrieve for DenyRemoteRefs {
    fn retrieve(
        &self,
        uri: &jsonschema::Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        Err(format!("external $ref is not allowed: {uri}").into())
    }
}

fn truncate_text(body: &[u8], cap: usize) -> String {
    let slice = if body.len() > cap { &body[..cap] } else { body };
    String::from_utf8_lossy(slice).into_owned()
}

pub fn sanitize_error_body(body: &[u8]) -> String {
    let raw = truncate_text(body, ERROR_BODY_CAP);
    let stripped: String = raw
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    redact(&stripped)
}

pub fn redact(input: &str) -> String {
    if let Ok(mut value) = serde_json::from_str::<Value>(input) {
        redact_value(&mut value);
        return serde_json::to_string(&value).unwrap_or_else(|_| redact_text(input));
    }
    redact_text(input)
}

fn is_redact_key(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase().replace('_', "-");
    REDACT_KEYS.iter().any(|key| {
        let key = key.replace('_', "-");
        normalized == key || normalized.ends_with(&format!("-{key}"))
    })
}

fn redact_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if is_redact_key(key) {
                    *child = Value::String("[REDACTED]".into());
                } else {
                    redact_value(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item);
            }
        }
        Value::String(s) => *s = redact_text(s),
        _ => {}
    }
}

fn redact_text(input: &str) -> String {
    let mut out = input.to_owned();
    for key in REDACT_KEYS {
        for variant in [*key, &key.replace('-', "_"), &key.replace('_', "-")] {
            redact_all(&mut out, &format!("{variant}="));
            redact_all(&mut out, &format!("{variant}:"));
        }
    }
    out
}

fn redact_all(out: &mut String, needle: &str) {
    let mut from = 0;
    loop {
        let lower = out.to_ascii_lowercase();
        let Some(rel) = lower[from..].find(&needle.to_ascii_lowercase()) else {
            break;
        };
        let idx = from + rel;
        let mut start = idx + needle.len();
        while start < out.len() && out.as_bytes()[start].is_ascii_whitespace() {
            start += 1;
        }
        let end = out[start..]
            .find(['&', '"', '\'', ',', '\n', '\r', '}'])
            .map(|i| start + i)
            .unwrap_or(out.len());
        out.replace_range(start..end, "[REDACTED]");
        from = start + "[REDACTED]".len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_authorization_in_error_body() {
        let s = sanitize_error_body(b"Authorization=Bearer supersecret");
        assert!(!s.contains("supersecret"), "{s}");
        assert!(s.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_json_authorization_and_header_line() {
        let json = sanitize_error_body(br#"{"authorization":"Bearer supersecret"}"#);
        assert!(!json.contains("supersecret"), "{json}");
        let header = sanitize_error_body(b"Authorization: Bearer supersecret");
        assert!(!header.contains("supersecret"), "{header}");
        assert!(header.contains("[REDACTED]"));
    }

    #[test]
    fn schema_failure_on_output() {
        let schema =
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}});
        let upstream = UpstreamResponse {
            status: 200,
            content_type: Some("application/json".into()),
            body: br#"{"nope":1}"#.to_vec(),
        };
        let result = map_success(&upstream, ResponseKind::Json, Some(&schema));
        assert!(result.is_error);
        assert_eq!(result.error_code.as_deref(), Some("schema_output"));
    }

    #[test]
    fn remote_schema_ref_is_rejected() {
        let schema = json!({"$ref": "http://169.254.169.254/latest/meta-data/"});
        let err = validate_schema(&schema, &json!({})).unwrap_err();
        assert!(
            err.contains("not allowed") || err.to_ascii_lowercase().contains("ref"),
            "{err}"
        );
    }
}
