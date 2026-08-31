use crate::cli::{AuthCmd, AuthType};
use crate::commands::load_cfg;
use crate::config::{CustomHeaderRef, UpstreamAuth};
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::secrets::SecretRef;
use crate::CliError;

pub fn run(paths: &PlatformPaths, out: &Output, cmd: AuthCmd) -> Result<ExitCode, CliError> {
    match cmd {
        AuthCmd::Add {
            name,
            r#type,
            header,
            query,
            from_env,
            from_file,
            from_env_headers,
        } => add(
            paths,
            out,
            name,
            r#type,
            header,
            query,
            from_env,
            from_file,
            from_env_headers,
        ),
        AuthCmd::List { name } => list(paths, out, name),
        AuthCmd::Remove { name } => remove(paths, out, name),
    }
}

#[allow(clippy::too_many_arguments)]
fn add(
    paths: &PlatformPaths,
    out: &Output,
    name: String,
    kind: AuthType,
    header: Option<String>,
    query: Option<String>,
    from_env: Option<String>,
    from_file: Option<std::path::PathBuf>,
    from_env_headers: Vec<String>,
) -> Result<ExitCode, CliError> {
    let mut cfg = load_cfg(paths)?;
    let spec = cfg.spec_mut(&name)?;
    let token = match (from_env, from_file) {
        (Some(env), None) => Some(SecretRef::env(env)),
        (None, Some(path)) => Some(SecretRef::file(path.display().to_string())),
        (None, None) if matches!(kind, AuthType::None | AuthType::CustomHeaders) => None,
        _ => {
            return Err(CliError::usage(
                "exactly one of --from-env or --from-file is required",
            ))
        }
    };
    if let Some(ref t) = token {
        t.resolve().map_err(|e| {
            if let Some(env) = match t {
                SecretRef::Env { env } => Some(env.clone()),
                _ => None,
            } {
                CliError::usage(format!(
                    "{env} is not set in the environment\nhint: export {env}=…   or use --from-file /run/secrets/{name}"
                ))
            } else {
                e
            }
        })?;
    }
    let mut headers = Vec::new();
    for item in from_env_headers {
        let (h, var) = item
            .split_once('=')
            .ok_or_else(|| CliError::usage("--from-env-header expects HEADER=VAR"))?;
        let r = SecretRef::env(var);
        r.resolve()?;
        headers.push(CustomHeaderRef {
            name: h.to_owned(),
            value: r,
        });
    }
    spec.upstream = Some(UpstreamAuth {
        kind: type_name(kind).into(),
        token,
        header,
        query,
        username: None,
        password: None,
        headers,
    });
    let display = spec
        .upstream
        .as_ref()
        .and_then(|u| u.token.as_ref().map(|t| t.display()))
        .unwrap_or_else(|| "none".into());
    let env_checked = spec.upstream.as_ref().and_then(|u| match &u.token {
        Some(SecretRef::Env { env }) => Some(env.clone()),
        _ => None,
    });
    cfg.save(&paths.config_file)?;
    if out.json {
        out.json_value(&serde_json::json!({
            "name": name,
            "type": type_name(kind),
            "source": display,
        }));
        return Ok(ExitCode::Ok);
    }
    out.line(&format!(
        "credential [{name}] type={} source={display}",
        type_name(kind)
    ));
    if let Some(env) = env_checked {
        out.line(&format!("checked: {env} is set (value not printed)"));
    }
    out.line("warning: do not put secrets in config.toml; this file is reference-only");
    Ok(ExitCode::Ok)
}

fn list(paths: &PlatformPaths, out: &Output, name: Option<String>) -> Result<ExitCode, CliError> {
    let cfg = load_cfg(paths)?;
    let rows: Vec<serde_json::Value> = cfg
        .specs
        .iter()
        .filter(|s| name.as_ref().is_none_or(|n| &s.name == n))
        .map(|s| {
            let (kind, source, present) = match &s.upstream {
                Some(u) => (
                    u.kind.clone(),
                    u.token
                        .as_ref()
                        .map(|t| t.display())
                        .unwrap_or_else(|| "none".into()),
                    u.token.as_ref().map(|t| t.present()).unwrap_or(true),
                ),
                None => ("none".into(), "none".into(), true),
            };
            serde_json::json!({
                "name": s.name,
                "type": kind,
                "source": source,
                "present": present,
            })
        })
        .collect();
    if out.json {
        out.json_value(&rows);
        return Ok(ExitCode::Ok);
    }
    for row in rows {
        out.line(&format!(
            "{}  type={}  source={}  present={}",
            row["name"].as_str().unwrap_or("-"),
            row["type"].as_str().unwrap_or("-"),
            row["source"].as_str().unwrap_or("-"),
            row["present"]
        ));
    }
    Ok(ExitCode::Ok)
}

fn remove(paths: &PlatformPaths, out: &Output, name: String) -> Result<ExitCode, CliError> {
    let mut cfg = load_cfg(paths)?;
    let spec = cfg.spec_mut(&name)?;
    spec.upstream = None;
    cfg.save(&paths.config_file)?;
    out.line(&format!("removed credential [{name}]"));
    Ok(ExitCode::Ok)
}

fn type_name(kind: AuthType) -> &'static str {
    match kind {
        AuthType::None => "none",
        AuthType::Bearer => "bearer",
        AuthType::Basic => "basic",
        AuthType::ApiKeyHeader => "api_key_header",
        AuthType::ApiKeyQuery => "api_key_query",
        AuthType::CustomHeaders => "custom_headers",
    }
}
