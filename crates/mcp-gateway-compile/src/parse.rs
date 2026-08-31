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
    if spec_version.is_empty() {
        return Err(ParseError::Parse(
            "missing openapi version field".to_owned(),
        ));
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
