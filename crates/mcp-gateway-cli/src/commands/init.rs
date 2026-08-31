use crate::config::{GatewayConfig, ServerConfig, SCHEMA_VERSION};
use crate::exit::ExitCode;
use crate::output::Output;
use crate::paths::PlatformPaths;
use crate::secrets::SecretRef;
use crate::CliError;
use rand::RngCore;

pub fn run(
    paths: &PlatformPaths,
    out: &Output,
    force: bool,
    bind: Option<String>,
    cloud: bool,
    allow_private: bool,
) -> Result<ExitCode, CliError> {
    if cloud {
        out.line(mcp_gateway_upsell::CLOUD_URL);
        return Ok(ExitCode::Ok);
    }
    if paths.config_file.exists() && !force {
        return Err(CliError::usage(format!(
            "config already exists at {}\nhint: mcp-gateway init --force to overwrite, or mcp-gateway inspect",
            paths.config_file.display()
        )));
    }
    std::fs::create_dir_all(&paths.cache_dir)
        .map_err(|e| CliError::io(format!("create IR cache: {e}")))?;
    let bind = bind.unwrap_or_else(|| "127.0.0.1:8787".into());
    let token = generate_token();
    let mut cfg = GatewayConfig {
        schema_version: SCHEMA_VERSION,
        server: ServerConfig {
            bind: bind.clone(),
            token: Some(SecretRef::env("MCP_GATEWAY_TOKEN")),
            ..ServerConfig::default()
        },
        ..GatewayConfig::default()
    };
    if allow_private {
        cfg.ssrf.allow_private_networks = true;
    }
    cfg.save(&paths.config_file)?;
    write_gitignore(paths);

    if out.json {
        out.json_value(&serde_json::json!({
            "config": paths.config_file,
            "bind": bind,
            "ir_cache": paths.cache_dir,
            "token_env": "MCP_GATEWAY_TOKEN",
            "token": token,
        }));
        return Ok(ExitCode::Ok);
    }

    out.line(&format!("mcp-gateway {}", env!("CARGO_PKG_VERSION")));
    out.line(&format!("Creating {}", paths.config_file.display()));
    out.line(&format!(
        "Wrote bind = \"{bind}\" (localhost only; pass --bind 0.0.0.0:8787 to expose)"
    ));
    out.line("Generated MCP bearer token and stored a reference as env MCP_GATEWAY_TOKEN");
    out.line(&format!(
        "  export MCP_GATEWAY_TOKEN={token}   (printed once; not written to the TOML)"
    ));
    out.line(&format!("IR cache: {}", paths.cache_dir.display()));
    if allow_private {
        out.err_line("warning: --allow-private-networks is on; this process can reach RFC1918/ULA targets. Cloud never enables this.");
    }
    out.line("Next: mcp-gateway add-spec --name petstore --url https://petstore3.swagger.io/api/v3/openapi.json");
    Ok(ExitCode::Ok)
}

fn generate_token() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("fh_mcp_live_{}", hex::encode(bytes))
}

fn write_gitignore(paths: &PlatformPaths) {
    if let Some(dir) = paths.config_file.parent() {
        let gi = dir.join(".gitignore");
        if !gi.exists() {
            let _ = std::fs::write(gi, "config.toml\n*.jsonl\n");
        }
    }
}
