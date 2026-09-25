use crate::exit::ExitCode;
use crate::fetchhive::credentials::{self, TokenSource};
use crate::fetchhive::{revoke, Client, Revoke};
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::CliError;

pub fn run(
    paths: &PlatformPaths,
    out: &Output,
    keep_remote: bool,
    cli_token: Option<String>,
    api_url: Option<String>,
) -> Result<ExitCode, CliError> {
    let stored = credentials::load(&paths.credentials_file)?;
    let resolved = credentials::resolve_token(cli_token.as_deref(), stored.as_ref());
    let Some(resolved) = resolved else {
        credentials::delete(&paths.credentials_file)?;
        emit(out, "not_logged_in", false);
        return Ok(ExitCode::Ok);
    };

    if resolved.source != TokenSource::File {
        emit_kept(out, resolved.source);
        return Ok(ExitCode::Ok);
    }

    let mut remote = false;
    if !keep_remote {
        let api_url = credentials::api_url_for(
            api_url.as_deref(),
            stored.as_ref().map(|item| item.api_url.as_str()),
        );
        let client = Client::new(&api_url, Some(resolved.token))?;
        remote = match revoke(&client)? {
            Revoke::Revoked | Revoke::AlreadyGone => true,
        };
    }
    credentials::delete(&paths.credentials_file)?;
    emit(out, "logged_out", remote);
    Ok(ExitCode::Ok)
}

fn emit(out: &Output, event: &str, remote: bool) {
    if out.json {
        out.json_value(&serde_json::json!({
            "event": event,
            "remote": remote,
        }));
        return;
    }
    if event == "not_logged_in" {
        println!("Not logged in.");
        return;
    }
    println!("Logged out.");
}

fn emit_kept(out: &Output, source: TokenSource) {
    let name = match source {
        TokenSource::Env => "MCP_GATEWAY_CLI_TOKEN",
        TokenSource::Flag => "--cli-token",
        TokenSource::File => "credentials.toml",
    };
    if out.json {
        out.json_value(&serde_json::json!({
            "event": "kept",
            "source": name,
        }));
        return;
    }
    println!("{name} is set. Unset it to sign out. credentials.toml was left in place.");
}
