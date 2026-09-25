use super::{map_status, retry_after, Client, ClientMeta};
use crate::CliError;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug)]
pub struct DeviceCode {
    pub device_code: SecretString,
    pub user_code: String,
    #[allow(dead_code)]
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

pub enum Poll {
    Pending,
    SlowDown,
    Denied,
    Expired,
    Ready(Grant),
}

pub struct Grant {
    pub access_token: SecretString,
    pub account_id: String,
    pub account_name: Option<String>,
    pub plan_type: Option<String>,
    pub user_email: String,
}

#[derive(Deserialize)]
struct CodeBody {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct GrantBody {
    access_token: String,
    account: AccountBody,
    user: UserBody,
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
struct RfcBody {
    error: String,
}

pub fn poll_wait(interval_secs: u64, slow_down: bool) -> Duration {
    let extra = if slow_down { 5 } else { 0 };
    Duration::from_secs(interval_secs.saturating_add(extra))
}

pub fn request_code(client: &Client, meta: &ClientMeta) -> Result<DeviceCode, CliError> {
    let response = client.send(
        client
            .http
            .post(client.url("/v1/anonymous/cli_device/code"))
            .json(&serde_json::json!({
                "client": {
                    "name": meta.name,
                    "version": meta.version,
                    "os": meta.os,
                    "hostname": meta.hostname,
                }
            })),
    )?;
    let status = response.status();
    let retry = retry_after(&response);
    let body = response
        .text()
        .map_err(|err| CliError::io(format!("could not read Fetch Hive response: {err}")))?;
    if !status.is_success() {
        return Err(map_status(status, &body, retry));
    }
    let parsed: CodeBody = serde_json::from_str(&body).map_err(|_| {
        CliError::upstream("Fetch Hive device response was missing device_code or user_code")
    })?;
    if parsed.device_code.is_empty() || parsed.user_code.is_empty() {
        return Err(CliError::upstream(
            "Fetch Hive device response was missing device_code or user_code",
        ));
    }
    Ok(DeviceCode {
        device_code: SecretString::from(parsed.device_code),
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        verification_uri_complete: parsed.verification_uri_complete,
        expires_in: parsed.expires_in,
        interval: parsed.interval,
    })
}

pub fn poll_token(client: &Client, device_code: &SecretString) -> Result<Poll, CliError> {
    let response = client.send(
        client
            .http
            .post(client.url("/v1/anonymous/cli_device/token"))
            .json(&serde_json::json!({
                "device_code": device_code.expose_secret(),
            })),
    )?;
    let status = response.status();
    let retry = retry_after(&response);
    let body = response
        .text()
        .map_err(|err| CliError::io(format!("could not read Fetch Hive response: {err}")))?;
    if status.as_u16() == 400 {
        let parsed: RfcBody = serde_json::from_str(&body).map_err(|_| {
            CliError::upstream("Fetch Hive poll response was not an RFC 8628 error")
        })?;
        return Ok(match parsed.error.as_str() {
            "authorization_pending" => Poll::Pending,
            "slow_down" => Poll::SlowDown,
            "access_denied" => Poll::Denied,
            "expired_token" => Poll::Expired,
            _ => {
                return Err(CliError::upstream(
                    "Fetch Hive poll returned an unknown error code",
                ));
            }
        });
    }
    if !status.is_success() {
        return Err(map_status(status, &body, retry));
    }
    let parsed: GrantBody = serde_json::from_str(&body)
        .map_err(|_| CliError::upstream("Fetch Hive approval response was missing access_token"))?;
    if !parsed.access_token.starts_with("fh_cli_") {
        return Err(CliError::upstream(
            "Fetch Hive approval response was missing an fh_cli_ token",
        ));
    }
    Ok(Poll::Ready(Grant {
        access_token: SecretString::from(parsed.access_token),
        account_id: parsed.account.id,
        account_name: parsed
            .account
            .name
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty()),
        plan_type: parsed.account.plan_type.filter(|plan| !plan.is_empty()),
        user_email: parsed.user.email,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_down_adds_five_seconds() {
        assert_eq!(poll_wait(5, false), Duration::from_secs(5));
        assert_eq!(poll_wait(5, true), Duration::from_secs(10));
        assert_eq!(poll_wait(0, true), Duration::from_secs(5));
    }
}
