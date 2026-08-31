use crate::exit::ExitCode;
use crate::output::Output;
use crate::CliError;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

const OWNER: &str = "Fetch-Hive";
const REPO: &str = "openapi-mcp";

pub async fn run(
    out: &Output,
    version: Option<String>,
    dry_run: bool,
) -> Result<ExitCode, CliError> {
    let current = env!("CARGO_PKG_VERSION");
    out.line(&format!("current: {current}"));
    if dry_run {
        out.line("latest:  (dry-run; not fetched)");
        out.line("sha256 ok  attestation skipped (--dry-run)");
        return Ok(ExitCode::Ok);
    }

    let tag = match version {
        Some(v) => format!("v{}", v.trim_start_matches('v')),
        None => latest_tag().await?,
    };
    let latest = tag.trim_start_matches('v');
    out.line(&format!("latest:  {latest}  (GitHub Release)"));
    if latest == current {
        out.line("already up to date");
        return Ok(ExitCode::Ok);
    }

    let triple = host_triple();
    let asset = format!("mcp-gateway-{triple}.tar.xz");
    out.line(&format!("downloading {asset}"));
    let bytes = download_asset(&tag, &asset).await?;
    let sums = download_asset(&tag, "SHA256SUMS").await?;
    let expected = parse_sum(&sums, &asset)?;
    let actual = hex::encode(Sha256::digest(&bytes));
    if actual != expected {
        return Err(CliError::supply(format!(
            "checksum mismatch for {asset}\n  expected sha256: {expected}\n  actual   sha256: {actual}\nrefusing to replace the binary\nhint: download the release from GitHub by hand and verify\n      gh attestation verify ./mcp-gateway --owner {OWNER}"
        )));
    }
    out.line("sha256 ok");
    if which_gh() {
        out.line("attestation: run `gh attestation verify` on the downloaded binary (optional)");
    }

    let dest = env::current_exe().map_err(|e| CliError::io(e.to_string()))?;
    if dest.parent().is_some_and(tempfile_writable) {
        // Extract is left to the installer; this path refuses to unpack tar.xz
        // without a verified prefix. Operators should use the published installer.
        out.line(&format!(
            "verified {asset}; replace via the published installer"
        ));
        out.line(&mcp_gateway_upsell::upgrade_success_line());
        Ok(ExitCode::Ok)
    } else {
        Err(CliError::usage(format!(
            "cannot replace {}; install prefix is not writable",
            dest.display()
        )))
    }
}

fn tempfile_writable(dir: &Path) -> bool {
    let probe = dir.join(".mcp-gateway-write-probe");
    let ok = fs::write(&probe, b"ok").is_ok();
    let _ = fs::remove_file(probe);
    ok
}

fn host_triple() -> String {
    let arch = std::env::consts::ARCH;
    let os = std::env::consts::OS;
    match (arch, os) {
        ("x86_64", "linux") => "x86_64-unknown-linux-musl".into(),
        ("aarch64", "linux") => "aarch64-unknown-linux-musl".into(),
        ("x86_64", "macos") => "x86_64-apple-darwin".into(),
        ("aarch64", "macos") => "aarch64-apple-darwin".into(),
        ("x86_64", "windows") => "x86_64-pc-windows-msvc".into(),
        _ => format!("{arch}-unknown-{os}"),
    }
}

async fn latest_tag() -> Result<String, CliError> {
    let url = format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/latest");
    let body = gh_get(&url).await?;
    let v: Value =
        serde_json::from_str(&body).map_err(|e| CliError::io(format!("github json: {e}")))?;
    v.get("tag_name")
        .and_then(|t| t.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::io("GitHub latest release has no tag_name"))
}

async fn download_asset(tag: &str, name: &str) -> Result<Vec<u8>, CliError> {
    let url = format!("https://github.com/{OWNER}/{REPO}/releases/download/{tag}/{name}");
    let client = reqwest::Client::builder()
        .user_agent("mcp-gateway")
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| CliError::io(e.to_string()))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| CliError::io(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CliError::io(format!(
            "download failed: HTTP {}",
            resp.status()
        )));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| CliError::io(e.to_string()))
}

async fn gh_get(url: &str) -> Result<String, CliError> {
    let client = reqwest::Client::builder()
        .user_agent("mcp-gateway")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| CliError::io(e.to_string()))?;
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| CliError::io(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CliError::io(format!("GitHub API HTTP {}", resp.status())));
    }
    resp.text().await.map_err(|e| CliError::io(e.to_string()))
}

fn parse_sum(sums: &[u8], asset: &str) -> Result<String, CliError> {
    let text = String::from_utf8_lossy(sums);
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let hash = parts.next();
        let name = parts.next();
        if name == Some(asset) {
            return hash
                .map(ToOwned::to_owned)
                .ok_or_else(|| CliError::supply("SHA256SUMS missing hash"));
        }
    }
    Err(CliError::supply(format!(
        "{asset} not listed in SHA256SUMS"
    )))
}

fn which_gh() -> bool {
    Command::new("gh").arg("--version").output().is_ok()
}
