use crate::CliError;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum SecretRef {
    Env { env: String },
    File { file: String },
    Interpolated(String),
}

impl SecretRef {
    pub fn env(name: impl Into<String>) -> Self {
        Self::Env { env: name.into() }
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self::File { file: path.into() }
    }

    pub fn display(&self) -> String {
        match self {
            Self::Env { env } => format!("env:{env}"),
            Self::File { file } => format!("file:{file}"),
            Self::Interpolated(s) => match parse_interpolation(s) {
                Ok(var) => format!("env:{var}"),
                Err(_) => "invalid".into(),
            },
        }
    }

    pub fn present(&self) -> bool {
        match self.resolve() {
            Ok(secret) => !secret.expose_secret().is_empty(),
            Err(_) => false,
        }
    }

    pub fn resolve(&self) -> Result<SecretString, CliError> {
        match self {
            Self::Env { env } => std::env::var(env)
                .map(SecretString::from)
                .map_err(|_| CliError::usage(format!("{env} is not set in the environment"))),
            Self::File { file } => read_file_secret(Path::new(file)),
            Self::Interpolated(raw) => {
                if looks_like_secret_literal(raw) {
                    return Err(CliError::usage(
                        "inline secret literals are not allowed; use { env = \"VAR\" } or { file = \"PATH\" }",
                    ));
                }
                let var = parse_interpolation(raw)?;
                std::env::var(&var)
                    .map(SecretString::from)
                    .map_err(|_| CliError::usage(format!("{var} is not set in the environment")))
            }
        }
    }
}

fn read_file_secret(path: &Path) -> Result<SecretString, CliError> {
    let mut raw = fs::read_to_string(path)
        .map_err(|e| CliError::usage(format!("cannot read secret file {}: {e}", path.display())))?;
    if raw.ends_with('\n') {
        raw.pop();
        if raw.ends_with('\r') {
            raw.pop();
        }
    }
    Ok(SecretString::from(raw))
}

pub fn parse_interpolation(raw: &str) -> Result<String, CliError> {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed.strip_prefix("${").and_then(|s| s.strip_suffix('}')) {
        if inner.is_empty() || inner.contains(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            return Err(CliError::usage(format!("invalid interpolation: {raw}")));
        }
        return Ok(inner.to_owned());
    }
    Err(CliError::usage(format!(
        "secret references must be ${{VAR}}, {{ env = \"VAR\" }}, or {{ file = \"PATH\" }} (got {raw})"
    )))
}

pub fn looks_like_secret_literal(raw: &str) -> bool {
    raw.contains("fh_mcp_") || raw.contains("sk_live") || raw.contains("Bearer ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_round_trip() {
        assert_eq!(
            parse_interpolation("${PETSTORE_TOKEN}").unwrap(),
            "PETSTORE_TOKEN"
        );
        assert!(parse_interpolation("fh_mcp_live_abc").is_err());
    }

    #[test]
    fn env_present() {
        std::env::set_var("MCP_GW_TEST_SECRET", "x");
        let r = SecretRef::env("MCP_GW_TEST_SECRET");
        assert!(r.present());
        std::env::remove_var("MCP_GW_TEST_SECRET");
    }
}
