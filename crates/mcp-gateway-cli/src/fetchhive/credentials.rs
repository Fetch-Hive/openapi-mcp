use super::device::Grant;
use crate::CliError;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenSource {
    Flag,
    Env,
    File,
}

pub struct StoredCredentials {
    pub api_url: String,
    pub token: SecretString,
    pub account_id: String,
    pub account_name: Option<String>,
    pub user_email: String,
    pub plan_type: Option<String>,
    pub logged_in_at: String,
}

pub struct ResolvedToken {
    pub token: SecretString,
    pub source: TokenSource,
}

#[derive(Serialize, Deserialize)]
struct CredentialsFile {
    fetchhive: CredentialsSection,
}

#[derive(Serialize, Deserialize)]
struct CredentialsSection {
    api_url: String,
    token: String,
    account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account_name: Option<String>,
    user_email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan_type: Option<String>,
    logged_in_at: String,
}

#[derive(Debug)]
pub enum DoctorNote {
    Missing,
    LoggedIn { email: String },
    Loose { email: String },
    Unreadable(String),
}

pub fn load(path: &Path) -> Result<Option<StoredCredentials>, CliError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)
        .map_err(|err| CliError::io(format!("could not read {}: {err}", path.display())))?;
    let parsed: CredentialsFile = toml::from_str(&raw)
        .map_err(|err| CliError::usage(format!("could not parse {}: {err}", path.display())))?;
    let section = parsed.fetchhive;
    if section.token.is_empty() {
        return Err(CliError::usage(format!(
            "{} has an empty token. Run `mcp-gateway login --force`.",
            path.display()
        )));
    }
    Ok(Some(StoredCredentials {
        api_url: section.api_url,
        token: SecretString::from(section.token),
        account_id: section.account_id,
        account_name: blank(section.account_name),
        user_email: section.user_email,
        plan_type: blank(section.plan_type),
        logged_in_at: section.logged_in_at,
    }))
}

pub fn save(path: &Path, api_url: &str, grant: &Grant) -> Result<(), CliError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                CliError::io(format!("could not create {}: {err}", parent.display()))
            })?;
        }
    }
    let file = CredentialsFile {
        fetchhive: CredentialsSection {
            api_url: api_url.to_string(),
            token: grant.access_token.expose_secret().to_string(),
            account_id: grant.account_id.clone(),
            account_name: grant.account_name.clone(),
            user_email: grant.user_email.clone(),
            plan_type: grant.plan_type.clone(),
            logged_in_at: timestamp(),
        },
    };
    let rendered = toml::to_string_pretty(&file)
        .map_err(|err| CliError::io(format!("could not encode credentials: {err}")))?;
    write_private(path, rendered.as_bytes())?;
    Ok(())
}

pub fn delete(path: &Path) -> Result<(), CliError> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|err| CliError::io(format!("could not remove {}: {err}", path.display())))?;
    }
    Ok(())
}

pub fn resolve_token(
    flag: Option<&str>,
    stored: Option<&StoredCredentials>,
) -> Option<ResolvedToken> {
    if let Some(flag) = flag.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(ResolvedToken {
            token: SecretString::from(flag.to_string()),
            source: TokenSource::Flag,
        });
    }
    if let Some(env) = std::env::var("MCP_GATEWAY_CLI_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Some(ResolvedToken {
            token: SecretString::from(env),
            source: TokenSource::Env,
        });
    }
    stored.map(|stored| ResolvedToken {
        token: SecretString::from(stored.token.expose_secret().to_string()),
        source: TokenSource::File,
    })
}

pub fn doctor_note(path: &Path) -> DoctorNote {
    if !path.exists() {
        return DoctorNote::Missing;
    }
    let email = match load(path) {
        Ok(Some(stored)) => stored.user_email,
        Ok(None) => return DoctorNote::Missing,
        Err(err) => return DoctorNote::Unreadable(err.to_string()),
    };
    match file_mode(path) {
        Ok(Some(mode)) if mode & 0o077 != 0 => DoctorNote::Loose { email },
        Ok(_) => DoctorNote::LoggedIn { email },
        Err(err) => DoctorNote::Unreadable(err),
    }
}

pub fn api_url_for(flag: Option<&str>, stored: Option<&str>) -> String {
    if let Some(flag) = flag.map(str::trim).filter(|value| !value.is_empty()) {
        return flag.to_string();
    }
    if let Some(env) = std::env::var("MCP_GATEWAY_API_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return env;
    }
    if let Some(stored) = stored.map(str::trim).filter(|value| !value.is_empty()) {
        return stored.to_string();
    }
    super::DEFAULT_API_URL.to_string()
}

fn blank(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into())
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|err| CliError::io(format!("could not write {}: {err}", path.display())))?;
    file.write_all(bytes)
        .map_err(|err| CliError::io(format!("could not write {}: {err}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|err| {
            CliError::io(format!(
                "could not set permissions on {}: {err}",
                path.display()
            ))
        })?;
    }
    Ok(())
}

fn file_mode(path: &Path) -> Result<Option<u32>, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = fs::metadata(path).map_err(|err| err.to_string())?;
        Ok(Some(meta.permissions().mode() & 0o777))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(None)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::fetchhive::device::Grant;
    use secrecy::ExposeSecret;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn save_is_owner_read_write_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        let grant = Grant {
            access_token: SecretString::from("fh_cli_testsecret".to_string()),
            account_id: "acct".into(),
            account_name: None,
            plan_type: Some("developer".into()),
            user_email: "tom@example.com".into(),
        };
        save(&path, "https://api.fetchhive.com", &grant).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.token.expose_secret(), "fh_cli_testsecret");
        assert_eq!(loaded.user_email, "tom@example.com");
        let rendered = fs::read_to_string(&path).unwrap();
        assert!(rendered.contains("fh_cli_testsecret"));
    }

    #[test]
    fn flag_beats_the_file() {
        let stored = StoredCredentials {
            api_url: "https://api.fetchhive.com".into(),
            token: SecretString::from("fh_cli_file".to_string()),
            account_id: "acct".into(),
            account_name: None,
            user_email: "file@example.com".into(),
            plan_type: None,
            logged_in_at: "now".into(),
        };
        let resolved = resolve_token(Some("fh_cli_flag"), Some(&stored)).unwrap();
        assert_eq!(resolved.source, TokenSource::Flag);
        assert_eq!(resolved.token.expose_secret(), "fh_cli_flag");
    }

    #[test]
    fn corrupt_credentials_are_not_logged_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.toml");
        fs::write(&path, "this is not toml").unwrap();
        match doctor_note(&path) {
            DoctorNote::Unreadable(err) => assert!(err.contains("could not parse"), "{err}"),
            other => panic!("expected unreadable, got {other:?}"),
        }
    }
}
