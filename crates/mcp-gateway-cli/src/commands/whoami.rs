use crate::exit::ExitCode;
use crate::fetchhive::credentials::{self, TokenSource};
use crate::fetchhive::me::{account_label, quota_text, WhoamiError};
use crate::fetchhive::{whoami, Client};
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::CliError;

pub fn run(
    paths: &PlatformPaths,
    out: &Output,
    clear: bool,
    cli_token: Option<String>,
    api_url: Option<String>,
) -> Result<ExitCode, CliError> {
    let stored = credentials::load(&paths.credentials_file)?;
    let resolved = credentials::resolve_token(cli_token.as_deref(), stored.as_ref());
    let Some(resolved) = resolved else {
        if clear {
            credentials::delete(&paths.credentials_file)?;
        }
        emit_logged_out(out);
        return Ok(ExitCode::Ok);
    };

    let api_url = credentials::api_url_for(
        api_url.as_deref(),
        stored.as_ref().map(|item| item.api_url.as_str()),
    );
    let client = Client::new(&api_url, Some(resolved.token))?;
    match whoami(&client) {
        Ok(me) => {
            emit_me(out, &me, resolved.source);
            Ok(ExitCode::Ok)
        }
        Err(WhoamiError::Revoked(err)) => {
            let removed = clear && resolved.source == TokenSource::File;
            if removed {
                credentials::delete(&paths.credentials_file)?;
            }
            let extra = if removed {
                "Removed credentials.toml."
            } else if clear {
                "credentials.toml was left in place because this token did not come from that file."
            } else {
                "Pass --clear to delete credentials.toml."
            };
            Err(CliError::usage(format!("{err}. {extra}")))
        }
        Err(WhoamiError::Other(err)) => Err(err),
    }
}

fn emit_logged_out(out: &Output) {
    if out.json {
        out.json_value(&serde_json::json!({ "logged_in": false }));
        return;
    }
    println!("Not logged in. Run `mcp-gateway login`.");
}

fn emit_me(out: &Output, me: &crate::fetchhive::Me, source: TokenSource) {
    if out.json {
        out.json_value(&serde_json::json!({
            "logged_in": true,
            "source": match source {
                TokenSource::Flag => "flag",
                TokenSource::Env => "env",
                TokenSource::File => "file",
            },
            "account": {
                "id": me.account.id,
                "name": me.account.name,
                "plan_type": me.account.plan_type,
            },
            "user": { "email": me.user_email },
            "token": {
                "name": me.token.name,
                "last_four": me.token.last_four,
                "created_at": me.token.created_at,
            },
            "limits": {
                "tunnel_endpoints": {
                    "used": me.limits.used,
                    "limit": me.limits.limit,
                }
            }
        }));
        return;
    }
    let account = account_label(me.account.name.as_deref(), me.account.plan_type.as_deref());
    println!("{}", me.user_email);
    println!("Account: {account}");
    println!("Token: {} ····{}", me.token.name, me.token.last_four);
    println!("Tunnel endpoints: {}", quota_text(&me.limits));
}
