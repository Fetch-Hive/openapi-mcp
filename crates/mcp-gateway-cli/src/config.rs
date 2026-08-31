use crate::secrets::{looks_like_secret_literal, SecretRef};
use crate::CliError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    pub schema_version: u32,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub ssrf: SsrfConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub specs: Vec<SpecEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_path")]
    pub path: String,
    #[serde(default)]
    pub expose: bool,
    #[serde(default)]
    pub allow_anonymous: bool,
    #[serde(default)]
    pub token: Option<SecretRef>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            path: default_path(),
            expose: false,
            allow_anonymous: false,
            token: Some(SecretRef::env("MCP_GATEWAY_TOKEN")),
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1:8787".into()
}
fn default_path() -> String {
    "/mcp".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SsrfConfig {
    #[serde(default)]
    pub allow_private_networks: bool,
    #[serde(default)]
    pub allow_insecure_http: bool,
    #[serde(default)]
    pub allow_metadata: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    #[serde(default)]
    pub dir: String,
    #[serde(default = "default_ttl")]
    pub ttl: String,
}

fn default_ttl() -> String {
    "168h".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LogConfig {
    #[serde(default)]
    pub file: String,
    #[serde(default = "default_level")]
    pub level: String,
}

fn default_level() -> String {
    "info".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecEntry {
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub ir_pin: Option<String>,
    #[serde(default)]
    pub enabled_tools: Vec<String>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    #[serde(default)]
    pub upstream: Option<UpstreamAuth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpstreamAuth {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub token: Option<SecretRef>,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub username: Option<SecretRef>,
    #[serde(default)]
    pub password: Option<SecretRef>,
    #[serde(default)]
    pub headers: Vec<CustomHeaderRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomHeaderRef {
    pub name: String,
    pub value: SecretRef,
}

impl GatewayConfig {
    pub fn load(path: &Path) -> Result<Self, CliError> {
        let raw = fs::read_to_string(path)
            .map_err(|e| CliError::usage(format!("cannot read {}: {e}", path.display())))?;
        Self::parse(&raw)
    }

    pub fn parse(raw: &str) -> Result<Self, CliError> {
        let cfg: Self =
            toml::from_str(raw).map_err(|e| CliError::usage(format!("config parse: {e}")))?;
        if cfg.schema_version > SCHEMA_VERSION {
            return Err(CliError::usage(format!(
                "config schema_version {} is newer than this CLI; run mcp-gateway upgrade",
                cfg.schema_version
            )));
        }
        if cfg.schema_version == 0 {
            return Err(CliError::usage("config schema_version is required"));
        }
        cfg.scan_secret_literals()?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<(), CliError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| CliError::io(format!("create config dir: {e}")))?;
        }
        let body = format!(
            "{}\n{}",
            SECRET_WARNING,
            toml::to_string_pretty(self)
                .map_err(|e| CliError::io(format!("serialize config: {e}")))?
        );
        fs::write(path, body).map_err(|e| CliError::io(format!("write config: {e}")))
    }

    pub fn spec(&self, name: &str) -> Result<&SpecEntry, CliError> {
        self.specs
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| CliError::usage(format!("unknown spec '{name}'")))
    }

    pub fn spec_mut(&mut self, name: &str) -> Result<&mut SpecEntry, CliError> {
        self.specs
            .iter_mut()
            .find(|s| s.name == name)
            .ok_or_else(|| CliError::usage(format!("unknown spec '{name}'")))
    }

    fn scan_secret_literals(&self) -> Result<(), CliError> {
        let encoded = toml::to_string(self).unwrap_or_default();
        if looks_like_secret_literal(&encoded) {
            return Err(CliError::usage(
                "config looks like it contains a live token (fh_mcp_, sk_live, or Bearer ); use env or file references",
            ));
        }
        Ok(())
    }
}

pub const SECRET_WARNING: &str = "\
# WARNING: never commit tokens, passwords, or private keys to this file.
# Use env references or file references. `mcp-gateway doctor` fails if it
# finds a value that looks like a live token (fh_mcp_, sk_live, Bearer ).
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_keys() {
        let err = GatewayConfig::parse("schema_version = 1\nnope = true\n").unwrap_err();
        assert!(err.to_string().contains("config parse"));
    }

    #[test]
    fn parses_example_shape() {
        let raw = r#"
schema_version = 1
[server]
bind = "127.0.0.1:8787"
token = { env = "MCP_GATEWAY_TOKEN" }
[[specs]]
name = "petstore"
url = "https://petstore3.swagger.io/api/v3/openapi.json"
"#;
        let cfg = GatewayConfig::parse(raw).unwrap();
        assert_eq!(cfg.specs[0].name, "petstore");
    }
}
