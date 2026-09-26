use super::{retry_after, Client};
use crate::CliError;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct TunnelEndpoint {
    pub slug: String,
    pub public_url: String,
    pub online: bool,
    pub last_connected_at: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Deserialize)]
struct ListBody {
    mcp_tunnel_endpoints: Vec<EndpointBody>,
}

#[derive(Deserialize)]
struct CreateBody {
    mcp_tunnel_endpoint: EndpointBody,
}

#[derive(Deserialize)]
struct EndpointBody {
    slug: String,
    public_url: String,
    online: bool,
    #[serde(default)]
    last_connected_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

impl From<EndpointBody> for TunnelEndpoint {
    fn from(body: EndpointBody) -> Self {
        Self {
            slug: body.slug,
            public_url: body.public_url,
            online: body.online,
            last_connected_at: body.last_connected_at,
            created_at: body.created_at,
        }
    }
}

pub fn list(client: &Client) -> Result<Vec<TunnelEndpoint>, CliError> {
    let response = client.send(client.http.get(client.url("/v1/cli/tunnel_endpoints")))?;
    let (status, body) = read(response)?;
    if !status.is_success() {
        return Err(map_endpoint_error("", status, &body));
    }
    let parsed: ListBody = serde_json::from_str(&body).map_err(|_| {
        CliError::upstream("Fetch Hive tunnel list was missing mcp_tunnel_endpoints")
    })?;
    Ok(parsed
        .mcp_tunnel_endpoints
        .into_iter()
        .map(TunnelEndpoint::from)
        .collect())
}

pub fn create(client: &Client, slug: &str) -> Result<TunnelEndpoint, CliError> {
    let response = client.send(
        client
            .http
            .post(client.url("/v1/cli/tunnel_endpoints"))
            .json(&serde_json::json!({ "slug": slug })),
    )?;
    let (status, body) = read(response)?;
    if !status.is_success() {
        return Err(map_endpoint_error(slug, status, &body));
    }
    let parsed: CreateBody = serde_json::from_str(&body).map_err(|_| {
        CliError::upstream("Fetch Hive tunnel create was missing mcp_tunnel_endpoint")
    })?;
    Ok(parsed.mcp_tunnel_endpoint.into())
}

pub fn delete(client: &Client, slug: &str) -> Result<(), CliError> {
    let response = client.send(
        client
            .http
            .delete(client.url(&format!("/v1/cli/tunnel_endpoints/{slug}"))),
    )?;
    let (status, body) = read(response)?;
    if status.is_success() {
        return Ok(());
    }
    Err(map_endpoint_error(slug, status, &body))
}

fn read(response: reqwest::blocking::Response) -> Result<(reqwest::StatusCode, String), CliError> {
    let status = response.status();
    let _retry = retry_after(&response);
    let body = response
        .text()
        .map_err(|err| CliError::io(format!("could not read Fetch Hive response: {err}")))?;
    Ok((status, body))
}

pub fn map_endpoint_error(slug: &str, status: reqwest::StatusCode, body: &str) -> CliError {
    if body.contains("fh_cli_") {
        return CliError::upstream("Fetch Hive returned a token on an error response");
    }
    let parsed: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = parsed
        .get("error_code")
        .and_then(Value::as_str)
        .unwrap_or("");
    if status.as_u16() == 401 || code == "cli_token_invalid" || code == "unauthorized" {
        return CliError::usage("login expired or revoked; run `mcp-gateway login`");
    }
    if status.as_u16() == 402 || code == "mcp_tunnel_endpoint_limit" {
        return CliError::policy(plan_limit_message(&parsed));
    }
    if status.as_u16() == 403 || code == "mcp_tunnel_not_owner" {
        return CliError::usage(format!(
            "{slug} belongs to another account; pick another name"
        ));
    }
    if code == "mcp_tunnel_slug_reserved" {
        return CliError::usage(format!("{slug} is reserved"));
    }
    if code == "mcp_tunnel_slug_invalid" {
        return CliError::usage(format!("{slug} is not a valid tunnel name"));
    }
    if code == "mcp_tunnel_slug_taken" {
        return CliError::usage(format!("{slug} is already reserved"));
    }
    if status.as_u16() == 404 {
        return CliError::usage(format!("no tunnel named {slug}"));
    }
    let message = parsed
        .get("error")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("message").and_then(Value::as_str))
        .unwrap_or("request failed");
    CliError::upstream(format!(
        "Fetch Hive returned HTTP {}: {message}",
        status.as_u16()
    ))
}

fn plan_limit_message(body: &Value) -> String {
    let limit = body.get("limit").and_then(Value::as_u64);
    let current = body.get("current").and_then(Value::as_u64);
    let url = body.get("upgrade_url").and_then(Value::as_str);
    match (limit, current, url) {
        (Some(limit), Some(current), Some(url)) => format!(
            "this plan allows {limit} persistent endpoints ({current} in use). upgrade: {url}"
        ),
        _ => "plan limit reached".to_owned(),
    }
}
