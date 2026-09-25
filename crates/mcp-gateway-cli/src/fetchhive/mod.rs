pub(crate) mod credentials;
pub(crate) mod device;
pub(crate) mod me;

pub use credentials::{doctor_note, DoctorNote};
pub use me::{revoke, whoami, Me, Revoke};

use crate::CliError;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use std::time::Duration;
use url::Url;

pub const DEFAULT_API_URL: &str = "https://api.fetchhive.com";

pub struct Client {
    http: reqwest::blocking::Client,
    api_url: String,
    token: Option<SecretString>,
}

pub struct ClientMeta {
    pub name: String,
    pub version: String,
    pub os: String,
    pub hostname: String,
}

impl Client {
    pub fn new(api_url: &str, token: Option<SecretString>) -> Result<Self, CliError> {
        let api_url = normalize_api_url(api_url)?;
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(user_agent())
            .tls_built_in_native_certs(true)
            .build()
            .map_err(|err| CliError::io(format!("http client: {err}")))?;
        Ok(Self {
            http,
            api_url,
            token,
        })
    }

    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.api_url)
    }

    fn send(
        &self,
        request: reqwest::blocking::RequestBuilder,
    ) -> Result<reqwest::blocking::Response, CliError> {
        let request = if let Some(token) = &self.token {
            request.bearer_auth(token.expose_secret())
        } else {
            request
        };
        request.send().map_err(transport_error)
    }
}

pub fn normalize_api_url(raw: &str) -> Result<String, CliError> {
    let url = Url::parse(raw.trim()).map_err(|_| {
        CliError::usage(format!(
            "MCP_GATEWAY_API_URL is not a URL ({raw}). Use https://api.fetchhive.com"
        ))
    })?;
    let host = url.host_str().unwrap_or("");
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => {
            return Err(CliError::usage(
                "MCP_GATEWAY_API_URL must be https. http is only accepted for localhost.",
            ));
        }
        _ => {
            return Err(CliError::usage(
                "MCP_GATEWAY_API_URL must be https. http is only accepted for localhost.",
            ));
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CliError::usage(
            "MCP_GATEWAY_API_URL must not include a username or password",
        ));
    }
    let mut cleaned = url.clone();
    cleaned.set_path("");
    cleaned.set_query(None);
    cleaned.set_fragment(None);
    Ok(cleaned.as_str().trim_end_matches('/').to_string())
}

pub fn user_agent() -> String {
    format!(
        "mcp-gateway/{} ({}-{})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

pub fn local_client_meta() -> ClientMeta {
    ClientMeta {
        name: "mcp-gateway".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        os: std::env::consts::OS.into(),
        hostname: hostname(),
    }
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .unwrap_or_default()
}

fn transport_error(err: reqwest::Error) -> CliError {
    let mut detail = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(inner) = source {
        let next = inner.to_string();
        if !detail.contains(&next) {
            detail.push_str(": ");
            detail.push_str(&next);
        }
        source = std::error::Error::source(inner);
    }
    CliError::io(format!(
        "could not reach Fetch Hive ({detail}). Check MCP_GATEWAY_API_URL and your network."
    ))
}

pub(crate) fn map_status(
    status: reqwest::StatusCode,
    body: &str,
    retry_after: Option<u64>,
) -> CliError {
    if status.as_u16() == 429 {
        let wait = retry_after
            .map(|secs| format!(" Retry after {secs} seconds."))
            .unwrap_or_default();
        return CliError::upstream(format!("Fetch Hive login is rate limited.{wait}"));
    }
    let parsed = serde_json::from_str::<Value>(body).unwrap_or(Value::Null);
    if parsed.get("access_token").is_some() || body.contains("fh_cli_") {
        return CliError::upstream("Fetch Hive returned a token on an error response");
    }
    let code = parsed
        .get("error_code")
        .and_then(Value::as_str)
        .unwrap_or("");
    if code == "cli_device_unavailable" || status.as_u16() == 503 {
        return CliError::upstream(
            "Fetch Hive login is temporarily unavailable; local commands and anonymous tunnels still work.",
        );
    }
    if status.as_u16() == 401 || code == "cli_token_invalid" || code == "unauthorized" {
        return CliError::usage("Token revoked or expired; run `mcp-gateway login`");
    }
    let message = parsed
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("message").and_then(Value::as_str))
        .unwrap_or("request failed");
    if code.is_empty() {
        CliError::upstream(format!(
            "Fetch Hive returned HTTP {}: {message}",
            status.as_u16()
        ))
    } else {
        CliError::upstream(format!("{code}: {message}"))
    }
}

pub(crate) fn retry_after(response: &reqwest::blocking::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_api_url_is_trimmed() {
        let url = normalize_api_url("https://api.fetchhive.com/v1/").unwrap();
        assert_eq!(url, "https://api.fetchhive.com");
    }

    #[test]
    fn loopback_http_is_allowed() {
        let url = normalize_api_url("http://127.0.0.1:3456").unwrap();
        assert_eq!(url, "http://127.0.0.1:3456");
    }

    #[test]
    fn public_http_is_refused() {
        let err = normalize_api_url("http://api.fetchhive.com").unwrap_err();
        assert!(err.to_string().contains("https"));
    }

    #[test]
    fn error_body_does_not_echo_a_token() {
        let err = map_status(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"access_token":"fh_cli_secret","error":"nope"}"#,
            None,
        );
        assert!(!err.to_string().contains("fh_cli_"));
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn error_message_does_not_echo_a_token() {
        let err = map_status(
            reqwest::StatusCode::BAD_REQUEST,
            r#"{"error":"bad fh_cli_secret"}"#,
            None,
        );
        assert!(!err.to_string().contains("fh_cli_"));
        assert!(!err.to_string().contains("secret"));
    }
}
