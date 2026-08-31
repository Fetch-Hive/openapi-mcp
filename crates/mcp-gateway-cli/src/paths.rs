use directories::ProjectDirs;
use std::path::PathBuf;

pub struct PlatformPaths {
    pub config_file: PathBuf,
    pub cache_dir: PathBuf,
    pub log_file: PathBuf,
}

impl PlatformPaths {
    pub fn resolve(config_override: Option<&std::path::Path>) -> Self {
        if let Some(path) = config_override {
            let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            return Self {
                config_file: path.to_path_buf(),
                cache_dir: parent.join("cache").join("ir"),
                log_file: parent.join("mcp-gateway.jsonl"),
            };
        }
        if let Ok(env) = std::env::var("MCP_GATEWAY_CONFIG") {
            let path = PathBuf::from(env);
            let parent = path
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            return Self {
                config_file: path,
                cache_dir: cache_dir(),
                log_file: parent.join("mcp-gateway.jsonl"),
            };
        }
        let dirs = ProjectDirs::from("com", "fetchhive", "mcp-gateway");
        let config_file = dirs
            .as_ref()
            .map(|d| d.config_dir().join("config.toml"))
            .unwrap_or_else(|| PathBuf::from("config.toml"));
        let log_file = dirs
            .as_ref()
            .map(|d| d.data_local_dir().join("mcp-gateway.jsonl"))
            .unwrap_or_else(|| PathBuf::from("mcp-gateway.jsonl"));
        Self {
            config_file,
            cache_dir: cache_dir(),
            log_file,
        }
    }
}

fn cache_dir() -> PathBuf {
    if let Some(dirs) = ProjectDirs::from("com", "fetchhive", "mcp-gateway") {
        return dirs.cache_dir().join("ir");
    }
    PathBuf::from(".cache/mcp-gateway/ir")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_wins() {
        let paths = PlatformPaths::resolve(Some(std::path::Path::new("/tmp/mg/config.toml")));
        assert_eq!(paths.config_file, PathBuf::from("/tmp/mg/config.toml"));
        assert!(paths.cache_dir.ends_with("ir"));
    }
}
