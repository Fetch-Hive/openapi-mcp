use crate::loader::{OpenApiFamily, SpecFormat};
use crate::normalize::normalize_3_0;
use openapiv3::OpenAPI;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("failed to parse OpenAPI document: {0}")]
    Parse(String),
}

impl ParseError {
    pub fn exit_code(&self) -> i32 {
        1
    }
}

/// Parse bytes with the version-appropriate crate, then return a 3.1-shaped JSON document.
pub fn parse_to_value(
    bytes: &[u8],
    format: SpecFormat,
    family: OpenApiFamily,
) -> Result<(Value, String), ParseError> {
    let mut value = crate::loader::parse_value(bytes, format).map_err(ParseError::Parse)?;
    let spec_version = value
        .get("openapi")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    // Versions below 3.0 (including Swagger 2.0) must return an error. The
    // parser crates are not a safe fallback for those documents.
    if let Err(message) = require_openapi_3(&value, &spec_version) {
        return Err(ParseError::Parse(message));
    }

    match family {
        OpenApiFamily::V3_0 => {
            let text = match format {
                SpecFormat::Json => String::from_utf8_lossy(bytes).into_owned(),
                SpecFormat::Yaml => {
                    serde_json::to_string(&value).map_err(|e| ParseError::Parse(e.to_string()))?
                }
            };
            let _spec: OpenAPI = serde_json::from_str(&text)
                .or_else(|_| serde_yaml::from_slice(bytes))
                .map_err(|e| ParseError::Parse(format!("openapiv3: {e}")))?;
            normalize_3_0(&mut value);
        }
        OpenApiFamily::V3_1 => {
            let json =
                serde_json::to_string(&value).map_err(|e| ParseError::Parse(e.to_string()))?;
            oas3::from_json(&json).map_err(|e| ParseError::Parse(format!("oas3: {e}")))?;
        }
    }

    Ok((value, spec_version))
}

fn require_openapi_3(value: &Value, spec_version: &str) -> Result<(), String> {
    if spec_version.starts_with("3.0") || spec_version.starts_with("3.1") {
        return Ok(());
    }
    if let Some(swagger) = value.get("swagger").and_then(|v| v.as_str()) {
        return Err(format!(
            "Swagger {swagger} is not supported; use an OpenAPI 3.0 or 3.1 document"
        ));
    }
    if spec_version.is_empty() {
        return Err("missing openapi version field; use an OpenAPI 3.0 or 3.1 document".to_owned());
    }
    Err(format!(
        "OpenAPI {spec_version} is not supported; use an OpenAPI 3.0 or 3.1 document"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{OpenApiFamily, SpecFormat};

    #[test]
    fn rejects_swagger_2_with_a_version_message() {
        let err = parse_to_value(
            br#"{"swagger":"2.0","info":{"title":"x","version":"1"},"paths":{}}"#,
            SpecFormat::Json,
            OpenApiFamily::V3_1,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Swagger 2.0"), "{msg}");
        assert!(msg.contains("not supported"), "{msg}");
    }

    #[test]
    fn rejects_openapi_2_without_panicking() {
        let err = parse_to_value(
            br#"{"openapi":"2.0","info":{"title":"x","version":"1"},"paths":{}}"#,
            SpecFormat::Json,
            OpenApiFamily::V3_1,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("OpenAPI 2.0 is not supported"), "{msg}");
    }
}
