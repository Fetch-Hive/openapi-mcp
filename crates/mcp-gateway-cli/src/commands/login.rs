use crate::cli::Globals;
use crate::exit::ExitCode;
use crate::fetchhive::credentials::{self, TokenSource};
use crate::fetchhive::device::{self, Poll};
use crate::fetchhive::me::{account_label, quota_text, WhoamiError};
use crate::fetchhive::{local_client_meta, Client};
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::CliError;
use std::io::{self, IsTerminal, Write};
use std::time::Instant;

pub fn run(
    paths: &PlatformPaths,
    globals: &Globals,
    out: &Output,
    no_browser: bool,
    api_url: Option<String>,
    force: bool,
) -> Result<ExitCode, CliError> {
    let stored = credentials::load(&paths.credentials_file)?;
    let existing = credentials::resolve_token(None, stored.as_ref());
    if !force {
        if let Some(existing) = existing {
            return already(out, paths, stored.as_ref(), existing.source);
        }
    }

    let api_url = credentials::api_url_for(
        api_url.as_deref(),
        stored.as_ref().map(|item| item.api_url.as_str()),
    );
    let client = Client::new(&api_url, None)?;
    let code = device::request_code(&client, &local_client_meta())?;
    emit_code(out, &code.user_code, &code.verification_uri_complete);
    if should_open_browser(globals, no_browser) {
        if openable(&code.verification_uri_complete) {
            match open::that(&code.verification_uri_complete) {
                Ok(()) => show(out, "Opening your browser…"),
                Err(_) => show(
                    out,
                    "Could not open a browser. Open the URL above yourself.",
                ),
            }
        } else {
            show(
                out,
                "Did not open a browser. The verification URL is not http or https.",
            );
        }
    }
    show(
        out,
        &format!(
            "Waiting for approval… (expires in {})",
            clock(code.expires_in)
        ),
    );

    let deadline = Instant::now() + std::time::Duration::from_secs(code.expires_in.max(1));
    let grant = loop {
        if Instant::now() >= deadline {
            return Err(CliError::usage("code expired; run login again"));
        }
        match device::poll_token(&client, &code.device_code)? {
            Poll::Ready(grant) => break grant,
            Poll::Denied => {
                return Err(CliError::usage("login denied in browser"));
            }
            Poll::Expired => {
                return Err(CliError::usage("code expired; run login again"));
            }
            Poll::Pending => thread_sleep(device::poll_wait(code.interval, false)),
            Poll::SlowDown => thread_sleep(device::poll_wait(code.interval, true)),
        }
    };

    credentials::save(&paths.credentials_file, client.api_url(), &grant)?;
    let me = Client::new(client.api_url(), Some(clone_secret(&grant.access_token)))
        .and_then(|authed| crate::fetchhive::whoami(&authed).map_err(WhoamiError::into_cli));
    let (account, endpoints) = match me {
        Ok(me) => (
            account_label(me.account.name.as_deref(), me.account.plan_type.as_deref()),
            quota_text(&me.limits),
        ),
        Err(_) => (
            account_label(grant.account_name.as_deref(), grant.plan_type.as_deref()),
            "unavailable".to_string(),
        ),
    };
    let credentials = paths.credentials_file.display().to_string();
    emit_logged_in(
        out,
        &LoggedIn {
            email: &grant.user_email,
            account_id: &grant.account_id,
            account_name: grant.account_name.as_deref(),
            plan_type: grant.plan_type.as_deref(),
            account: &account,
            endpoints: &endpoints,
            credentials: &credentials,
        },
    );
    if std::env::var("MCP_GATEWAY_CLI_TOKEN")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
    {
        show(
            out,
            "MCP_GATEWAY_CLI_TOKEN is set and overrides credentials.toml for whoami and logout.",
        );
    }
    Ok(ExitCode::Ok)
}

fn already(
    out: &Output,
    paths: &PlatformPaths,
    stored: Option<&credentials::StoredCredentials>,
    source: TokenSource,
) -> Result<ExitCode, CliError> {
    if out.json {
        let email = stored.map(|item| item.user_email.as_str());
        out.json_value(&serde_json::json!({
            "event": "already_logged_in",
            "source": source_name(source),
            "user": { "email": email },
            "account": {
                "id": stored.map(|item| item.account_id.as_str()),
                "name": stored.and_then(|item| item.account_name.clone()),
                "plan_type": stored.and_then(|item| item.plan_type.clone()),
            },
            "logged_in_at": stored.map(|item| item.logged_in_at.clone()),
        }));
        return Ok(ExitCode::Ok);
    }
    match source {
        TokenSource::Env => show(
            out,
            "Already logged in via MCP_GATEWAY_CLI_TOKEN. Run `mcp-gateway login --force` to write credentials.toml as well.",
        ),
        TokenSource::Flag | TokenSource::File => {
            let email = stored
                .map(|item| item.user_email.as_str())
                .unwrap_or("the current account");
            let account = account_label(
                stored.and_then(|item| item.account_name.as_deref()),
                stored.and_then(|item| item.plan_type.as_deref()),
            );
            show(
                out,
                &format!("Already logged in as {email} ({account})."),
            );
            if let Some(stored) = stored {
                show(out, &format!("Signed in at {}.", stored.logged_in_at));
            }
            show(
                out,
                &format!("Credentials: {}", paths.credentials_file.display()),
            );
            show(out, "Run `mcp-gateway login --force` to sign in again.");
        }
    }
    Ok(ExitCode::Ok)
}

fn emit_code(out: &Output, user_code: &str, url: &str) {
    if out.json {
        out.json_value(&serde_json::json!({
            "event": "device_code",
            "user_code": user_code,
            "verification_uri_complete": url,
        }));
        return;
    }
    show(out, "To sign in, open:");
    show(out, "");
    show(out, &format!("    {url}"));
    show(out, "");
    show(out, &format!("and enter the code:  {user_code}"));
    show(out, "");
}

struct LoggedIn<'a> {
    email: &'a str,
    account_id: &'a str,
    account_name: Option<&'a str>,
    plan_type: Option<&'a str>,
    account: &'a str,
    endpoints: &'a str,
    credentials: &'a str,
}

fn emit_logged_in(out: &Output, logged_in: &LoggedIn<'_>) {
    if out.json {
        out.json_value(&serde_json::json!({
            "event": "logged_in",
            "account": {
                "id": logged_in.account_id,
                "name": logged_in.account_name,
                "plan_type": logged_in.plan_type,
            },
            "user": { "email": logged_in.email },
            "tunnel_endpoints": logged_in.endpoints,
            "credentials_file": logged_in.credentials,
        }));
        return;
    }
    let _ = writeln!(
        io::stdout(),
        "✓ Logged in to Fetch Hive as {email}\n  Account: {account}  ·  tunnel endpoints: {endpoints}\n  Credentials: {credentials}\n\nNext: `mcp-gateway serve NAME --tunnel --name SLUG` keeps that hostname. Anonymous tunnels still need no account.",
        email = logged_in.email,
        account = logged_in.account,
        endpoints = logged_in.endpoints,
        credentials = logged_in.credentials,
    );
}

fn should_open_browser(globals: &Globals, no_browser: bool) -> bool {
    !no_browser && !globals.json && io::stdout().is_terminal()
}

fn openable(url: &str) -> bool {
    url::Url::parse(url)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https"))
}

fn show(out: &Output, msg: &str) {
    if out.json {
        return;
    }
    println!("{msg}");
}

fn clock(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn source_name(source: TokenSource) -> &'static str {
    match source {
        TokenSource::Flag => "flag",
        TokenSource::Env => "env",
        TokenSource::File => "file",
    }
}

fn thread_sleep(wait: std::time::Duration) {
    if !wait.is_zero() {
        std::thread::sleep(wait);
    }
}

fn clone_secret(secret: &secrecy::SecretString) -> secrecy::SecretString {
    use secrecy::ExposeSecret;
    secrecy::SecretString::from(secret.expose_secret().to_string())
}
