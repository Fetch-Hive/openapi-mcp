use crate::fetchhive::credentials::{api_url_for, load, resolve_token};
use crate::fetchhive::me::{account_label, quota_text, whoami, WhoamiError};
use crate::fetchhive::tunnels;
use crate::fetchhive::Client;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::CliError;
use mcp_gateway_tunnel_proto::{validate_named_slug, SlugError};
use secrecy::ExposeSecret;
use std::path::Path;

pub const LOGIN_REQUIRED: &str = "`--name` needs a Fetch Hive login. Run `mcp-gateway login` (free: 1 persistent endpoint), or drop `--name` for an anonymous URL.";

pub struct NamedSession {
    pub slug: String,
    pub token: String,
    pub label: String,
    pub quota: String,
    pub created: bool,
}

pub struct AccountClient {
    pub client: Client,
    pub label_fallback: String,
}

pub async fn prepare(paths: &PlatformPaths, slug: &str) -> Result<NamedSession, CliError> {
    check_slug(slug)?;
    let credentials = paths.credentials_file.clone();
    let slug = slug.to_owned();
    tokio::task::spawn_blocking(move || prepare_blocking(&credentials, &slug))
        .await
        .map_err(|err| CliError::io(format!("login check failed: {err}")))?
}

pub fn open_account(credentials: &Path) -> Result<AccountClient, CliError> {
    Ok(load_account(credentials)?.0)
}

fn load_account(credentials: &Path) -> Result<(AccountClient, String), CliError> {
    let stored = load(credentials)?;
    let resolved =
        resolve_token(None, stored.as_ref()).ok_or_else(|| CliError::usage(LOGIN_REQUIRED))?;
    let token = resolved.token.expose_secret().to_owned();
    let api_url = api_url_for(None, stored.as_ref().map(|item| item.api_url.as_str()));
    let client = Client::new(&api_url, Some(resolved.token))?;
    let label_fallback = stored
        .as_ref()
        .map(|item| account_label(item.account_name.as_deref(), item.plan_type.as_deref()))
        .unwrap_or_else(|| "account".to_owned());
    Ok((
        AccountClient {
            client,
            label_fallback,
        },
        token,
    ))
}

pub fn check_slug(slug: &str) -> Result<(), CliError> {
    validate_named_slug(slug).map_err(|err| CliError::usage(slug_message(slug, err)))
}

pub fn announce_reserved(out: &Output, session: &NamedSession) {
    if out.json {
        super::serve::print_event(&serde_json::json!({
            "event": "reserved",
            "slug": session.slug,
            "account": session.label,
            "tunnel_endpoints": session.quota,
        }));
        return;
    }
    eprintln!(
        "✓ reserved {} for account {} ({} endpoints used)",
        session.slug, session.label, session.quota
    );
}

pub fn quota_line(limit: &crate::fetchhive::me::EndpointLimit) -> String {
    match limit.limit {
        Some(cap) if limit.used >= cap => {
            format!(
                "{} endpoints used · delete one or upgrade",
                quota_text(limit)
            )
        }
        _ => format!("{} endpoints used", quota_text(limit)),
    }
}

fn prepare_blocking(credentials: &Path, slug: &str) -> Result<NamedSession, CliError> {
    let (account, token) = load_account(credentials)?;
    let existing = tunnels::list(&account.client)?;
    let created = if existing.iter().any(|row| row.slug == slug) {
        false
    } else {
        tunnels::create(&account.client, slug)?;
        true
    };
    let (label, quota) = identity(&account)?;
    Ok(NamedSession {
        slug: slug.to_owned(),
        token,
        label,
        quota,
        created,
    })
}

pub(crate) fn identity(account: &AccountClient) -> Result<(String, String), CliError> {
    match whoami(&account.client) {
        Ok(me) => Ok((
            account_label(me.account.name.as_deref(), me.account.plan_type.as_deref()),
            quota_text(&me.limits),
        )),
        Err(WhoamiError::Revoked(err)) => Err(err),
        Err(WhoamiError::Other(_)) => {
            Ok((account.label_fallback.clone(), "unavailable".to_owned()))
        }
    }
}

fn slug_message(slug: &str, err: SlugError) -> String {
    match err {
        SlugError::Reserved => format!("{slug} is reserved"),
        SlugError::AnonymousShaped => format!("{slug} looks like an anonymous tunnel id"),
        other => format!("{slug} is not a valid tunnel name ({other})"),
    }
}
