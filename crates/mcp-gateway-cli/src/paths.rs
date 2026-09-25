use directories::ProjectDirs;
use std::path::PathBuf;

pub struct PlatformPaths {
    pub config_file: PathBuf,
    pub credentials_file: PathBuf,
    pub cache_dir: PathBuf,
    pub log_file: PathBuf,
}

impl PlatformPaths {
    pub fn resolve(config_override: Option<&std::path::Path>) -> Self {
        if let Some(path) = config_override {
            let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
            let config_file = path.to_path_buf();
            return Self {
                credentials_file: credentials_beside(&config_file),
                config_file,
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
                credentials_file: credentials_beside(&path),
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
            credentials_file: credentials_beside(&config_file),
            config_file,
            cache_dir: cache_dir(),
            log_file,
        }
    }
}

fn credentials_beside(config: &std::path::Path) -> PathBuf {
    match config.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join("credentials.toml"),
        _ => PathBuf::from("credentials.toml"),
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
        assert_eq!(
            paths.credentials_file,
            PathBuf::from("/tmp/mg/credentials.toml")
        );
        assert!(paths.cache_dir.ends_with("ir"));
    }

    #[test]
    fn credentials_follow_config_parent() {
        let paths = PlatformPaths::resolve(Some(std::path::Path::new("config.toml")));
        assert_eq!(paths.credentials_file, PathBuf::from("credentials.toml"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn default_credentials_use_the_platform_config_dir() {
        if std::env::var_os("MCP_GATEWAY_CONFIG").is_some() {
            return;
        }
        let paths = PlatformPaths::resolve(None);
        let rendered = paths.credentials_file.to_string_lossy();
        assert!(rendered.contains("com.fetchhive.mcp-gateway"), "{rendered}");
        assert!(rendered.ends_with("credentials.toml"), "{rendered}");
        assert_eq!(paths.credentials_file.parent(), paths.config_file.parent());
    }
}
