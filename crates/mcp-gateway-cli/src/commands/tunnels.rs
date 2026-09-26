use super::named::{self, quota_line};
use crate::cli::TunnelsCmd;
use crate::exit::ExitCode;
use crate::fetchhive::me::whoami;
use crate::fetchhive::tunnels::{self, TunnelEndpoint};
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::CliError;
use std::io::{self, IsTerminal, Write};

pub fn run(paths: &PlatformPaths, out: &Output, cmd: TunnelsCmd) -> Result<ExitCode, CliError> {
    match cmd {
        TunnelsCmd::List => list(paths, out),
        TunnelsCmd::Create { name } => create(paths, out, &name),
        TunnelsCmd::Delete { name, yes } => delete(paths, out, &name, yes),
    }
}

fn list(paths: &PlatformPaths, out: &Output) -> Result<ExitCode, CliError> {
    let account = named::open_account(&paths.credentials_file)?;
    let rows = tunnels::list(&account.client)?;
    if out.json {
        let body = rows.iter().map(json_row).collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string(&body).unwrap_or_else(|_| "[]".into())
        );
        return Ok(ExitCode::Ok);
    }
    if rows.is_empty() {
        println!("No persistent tunnel names. Reserve one with `mcp-gateway tunnels create NAME`.");
    } else {
        println!(
            "{:<16} {:<48} {:<8} {:<25} CREATED",
            "NAME", "URL", "STATUS", "LAST CONNECTED"
        );
        for row in &rows {
            println!(
                "{:<16} {:<48} {:<8} {:<25} {}",
                row.slug,
                row.public_url,
                if row.online { "online" } else { "offline" },
                row.last_connected_at.as_deref().unwrap_or("—"),
                row.created_at.as_deref().unwrap_or("—")
            );
        }
    }
    if let Ok(me) = whoami(&account.client) {
        println!("{}", quota_line(&me.limits));
    }
    Ok(ExitCode::Ok)
}

fn create(paths: &PlatformPaths, out: &Output, name: &str) -> Result<ExitCode, CliError> {
    named::check_slug(name)?;
    let account = named::open_account(&paths.credentials_file)?;
    let endpoint = tunnels::create(&account.client, name)?;
    let (label, quota) = named::identity(&account)?;
    if out.json {
        println!(
            "{}",
            serde_json::to_string(&json_row(&endpoint)).unwrap_or_else(|_| "{}".into())
        );
    } else {
        println!("✓ reserved {name} for account {label} ({quota} endpoints used)");
        println!("{}", endpoint.public_url);
    }
    Ok(ExitCode::Ok)
}

fn delete(
    paths: &PlatformPaths,
    out: &Output,
    name: &str,
    yes: bool,
) -> Result<ExitCode, CliError> {
    named::check_slug(name)?;
    if !yes {
        if !io::stdin().is_terminal() {
            return Err(CliError::usage(
                "pass --yes to release a tunnel name without a prompt",
            ));
        }
        print!("Release {name}? A running tunnel using it will disconnect. [y/N] ");
        io::stdout().flush().ok();
        let mut line = String::new();
        io::stdin()
            .read_line(&mut line)
            .map_err(|err| CliError::io(err.to_string()))?;
        if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            return Ok(ExitCode::Ok);
        }
    }
    let account = named::open_account(&paths.credentials_file)?;
    tunnels::delete(&account.client, name)?;
    let message = format!("released {name}; a running tunnel using it will disconnect.");
    if out.json {
        println!(
            "{}",
            serde_json::json!({"event": "released", "slug": name, "message": message})
        );
    } else {
        println!("{message}");
    }
    Ok(ExitCode::Ok)
}

fn json_row(row: &TunnelEndpoint) -> serde_json::Value {
    serde_json::json!({
        "slug": row.slug,
        "public_url": row.public_url,
        "online": row.online,
        "last_connected_at": row.last_connected_at,
        "created_at": row.created_at,
    })
}
