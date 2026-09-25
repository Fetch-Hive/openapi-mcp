use super::{map_status, retry_after, Client};
use crate::CliError;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Me {
    pub account: Account,
    pub user_email: String,
    pub token: TokenInfo,
    pub limits: EndpointLimit,
}

#[derive(Debug, Clone)]
pub struct Account {
    pub id: String,
    pub name: Option<String>,
    pub plan_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub name: String,
    pub last_four: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct EndpointLimit {
    pub used: u64,
    pub limit: Option<u64>,
}

#[derive(Deserialize)]
struct MeBody {
    account: AccountBody,
    user: UserBody,
    token: TokenBody,
    limits: LimitsBody,
}

#[derive(Deserialize)]
struct AccountBody {
    id: String,
    name: Option<String>,
    plan_type: Option<String>,
}

#[derive(Deserialize)]
struct UserBody {
    email: String,
}

#[derive(Deserialize)]
struct TokenBody {
    name: String,
    last_four: String,
    created_at: String,
}

#[derive(Deserialize)]
struct LimitsBody {
    tunnel_endpoints: EndpointBody,
}

#[derive(Deserialize)]
struct EndpointBody {
    used: u64,
    limit: Option<u64>,
}

pub enum Revoke {
    Revoked,
    AlreadyGone,
}

pub enum WhoamiError {
    Revoked(CliError),
    Other(CliError),
}

impl WhoamiError {
    pub fn into_cli(self) -> CliError {
        match self {
            Self::Revoked(err) | Self::Other(err) => err,
        }
    }
}

pub fn whoami(client: &Client) -> Result<Me, WhoamiError> {
    let response = client
        .send(client.http.get(client.url("/v1/cli/me")))
        .map_err(WhoamiError::Other)?;
    let status = response.status();
    let retry = retry_after(&response);
    let body = response.text().map_err(|err| {
        WhoamiError::Other(CliError::io(format!(
            "could not read Fetch Hive response: {err}"
        )))
    })?;
    if !status.is_success() {
        let err = map_status(status, &body, retry);
        return Err(if revoked_response(status, &body) {
            WhoamiError::Revoked(err)
        } else {
            WhoamiError::Other(err)
        });
    }
    let parsed: MeBody = serde_json::from_str(&body).map_err(|_| {
        WhoamiError::Other(CliError::upstream(
            "Fetch Hive whoami response was missing account or user",
        ))
    })?;
    Ok(Me {
        account: Account {
            id: parsed.account.id,
            name: blank_to_none(parsed.account.name),
            plan_type: blank_to_none(parsed.account.plan_type),
        },
        user_email: parsed.user.email,
        token: TokenInfo {
            name: parsed.token.name,
            last_four: parsed.token.last_four,
            created_at: parsed.token.created_at,
        },
        limits: EndpointLimit {
            used: parsed.limits.tunnel_endpoints.used,
            limit: parsed.limits.tunnel_endpoints.limit,
        },
    })
}

pub fn revoke(client: &Client) -> Result<Revoke, CliError> {
    let response = client.send(client.http.delete(client.url("/v1/cli/token")))?;
    let status = response.status();
    let retry = retry_after(&response);
    let body = response
        .text()
        .map_err(|err| CliError::io(format!("could not read Fetch Hive response: {err}")))?;
    if status.as_u16() == 401 || status.as_u16() == 404 || status.is_success() {
        return Ok(if status.is_success() {
            Revoke::Revoked
        } else {
            Revoke::AlreadyGone
        });
    }
    Err(map_status(status, &body, retry))
}

fn revoked_response(status: reqwest::StatusCode, body: &str) -> bool {
    if status.as_u16() == 401 {
        return true;
    }
    let parsed = serde_json::from_str::<serde_json::Value>(body).unwrap_or(serde_json::Value::Null);
    matches!(
        parsed.get("error_code").and_then(serde_json::Value::as_str),
        Some("cli_token_invalid" | "unauthorized")
    )
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

pub fn quota_text(limit: &EndpointLimit) -> String {
    match limit.limit {
        Some(cap) => format!("{}/{}", limit.used, cap),
        None => format!("{} (no cap)", limit.used),
    }
}

pub fn plan_label(plan: Option<&str>) -> String {
    let Some(plan) = plan.map(str::trim).filter(|plan| !plan.is_empty()) else {
        return "Unknown".into();
    };
    let mut chars = plan.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => "Unknown".into(),
    }
}

pub fn account_label(name: Option<&str>, plan: Option<&str>) -> String {
    let plan = plan_label(plan);
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("{name} ({plan})"),
        None => plan,
    }
}
