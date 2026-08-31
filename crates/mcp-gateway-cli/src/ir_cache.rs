use crate::CliError;
use mcp_gateway_ir::CompileBundle;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub fn cache_file(cache_dir: &Path, spec_name: &str, sha256: &str) -> PathBuf {
    let pin = if sha256.len() >= 12 {
        &sha256[..12]
    } else {
        sha256
    };
    cache_dir.join(format!("{spec_name}-{pin}.ir.json"))
}

pub fn write_bundle(
    cache_dir: &Path,
    spec_name: &str,
    bundle: &CompileBundle,
) -> Result<(PathBuf, String), CliError> {
    fs::create_dir_all(cache_dir).map_err(|e| CliError::io(format!("create IR cache: {e}")))?;
    let json = serde_json::to_vec_pretty(bundle)
        .map_err(|e| CliError::io(format!("serialize IR: {e}")))?;
    let sha = hex::encode(Sha256::digest(&json));
    let path = cache_file(cache_dir, spec_name, &sha);
    fs::write(&path, json).map_err(|e| CliError::io(format!("write IR cache: {e}")))?;
    Ok((path, sha))
}

pub fn load_bundle(path: &Path) -> Result<CompileBundle, CliError> {
    let raw = fs::read_to_string(path)
        .map_err(|e| CliError::usage(format!("cannot read IR {}: {e}", path.display())))?;
    serde_json::from_str(&raw).map_err(|e| CliError::usage(format!("invalid IR document: {e}")))
}

pub fn find_cached(cache_dir: &Path, spec_name: &str) -> Option<PathBuf> {
    let prefix = format!("{spec_name}-");
    let entries = fs::read_dir(cache_dir).ok()?;
    let mut matches: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".ir.json"))
        })
        .collect();
    matches.sort();
    matches.pop()
}
