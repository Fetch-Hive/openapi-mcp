use mcp_gateway_ir::{SourceKind, SourceMeta};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

use crate::safety::{self, SafetyError, SafetyOpts};

pub const SPEC_MAX_BYTES: usize = 10 * 1024 * 1024;
pub const SPEC_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("spec exceeds 10 MiB cap")]
    TooLarge,
    #[error(transparent)]
    Safety(#[from] SafetyError),
    #[error("download failed: {0}")]
    Download(String),
}

impl LoadError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Safety(_) => 3,
            Self::Io(_) | Self::Download(_) | Self::TooLarge => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SpecSource {
    File(PathBuf),
    Stdin,
    Url(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecFormat {
    Json,
    Yaml,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenApiFamily {
    V3_0,
    V3_1,
}

#[derive(Debug, Clone)]
pub struct LoadedSpec {
    pub bytes: Vec<u8>,
    pub source: SourceMeta,
    pub format: SpecFormat,
    pub family: OpenApiFamily,
    pub spec_version: String,
}

pub fn load(source: SpecSource) -> Result<LoadedSpec, LoadError> {
    load_with(source, SafetyOpts::default())
}

pub fn load_with(source: SpecSource, safety_opts: SafetyOpts) -> Result<LoadedSpec, LoadError> {
    let (bytes, kind, locator) = match &source {
        SpecSource::File(path) => {
            let bytes = read_capped(std::fs::File::open(path)?)?;
            (bytes, SourceKind::File, path.display().to_string())
        }
        SpecSource::Stdin => {
            let bytes = read_capped(std::io::stdin())?;
            (bytes, SourceKind::Stdin, "-".to_owned())
        }
        SpecSource::Url(raw) => {
            let url = safety::parse_https_url_with(raw, safety_opts)?;
            let host = url.host_str().unwrap_or_default();
            safety::resolve_and_check_with(host, safety_opts)?;
            let allow_http = url.scheme() == "http";
            let bytes = download_spec(url.as_str(), allow_http)?;
            (bytes, SourceKind::Url, strip_credentials(raw))
        }
    };
    finish(bytes, kind, locator)
}

pub fn load_bytes(
    bytes: Vec<u8>,
    kind: SourceKind,
    locator: impl Into<String>,
) -> Result<LoadedSpec, LoadError> {
    if bytes.len() > SPEC_MAX_BYTES {
        return Err(LoadError::TooLarge);
    }
    finish(bytes, kind, locator.into())
}

pub fn load_file(path: impl AsRef<Path>) -> Result<LoadedSpec, LoadError> {
    load(SpecSource::File(path.as_ref().to_path_buf()))
}

fn finish(bytes: Vec<u8>, kind: SourceKind, locator: String) -> Result<LoadedSpec, LoadError> {
    if bytes.len() > SPEC_MAX_BYTES {
        return Err(LoadError::TooLarge);
    }
    let format = sniff_format(&bytes);
    let (family, spec_version) = detect_version(&bytes, format);
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Ok(LoadedSpec {
        bytes,
        source: SourceMeta {
            kind,
            locator,
            sha256,
        },
        format,
        family,
        spec_version,
    })
}

pub fn sniff_format(bytes: &[u8]) -> SpecFormat {
    let trimmed = bytes
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .copied()
        .unwrap_or(0);
    if trimmed == b'{' || trimmed == b'[' {
        SpecFormat::Json
    } else {
        SpecFormat::Yaml
    }
}

pub fn detect_version(bytes: &[u8], format: SpecFormat) -> (OpenApiFamily, String) {
    let value = parse_value(bytes, format).unwrap_or(serde_json::Value::Null);
    let ver = value
        .get("openapi")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let family = if ver.starts_with("3.0") {
        OpenApiFamily::V3_0
    } else {
        OpenApiFamily::V3_1
    };
    (family, ver)
}

pub fn parse_value(bytes: &[u8], format: SpecFormat) -> Result<serde_json::Value, String> {
    match format {
        SpecFormat::Json => serde_json::from_slice(bytes).map_err(|e| e.to_string()),
        SpecFormat::Yaml => serde_yaml::from_slice(bytes).map_err(|e| e.to_string()),
    }
}

fn read_capped(mut reader: impl Read) -> Result<Vec<u8>, LoadError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > SPEC_MAX_BYTES {
            return Err(LoadError::TooLarge);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

fn download_spec(url: &str, allow_http: bool) -> Result<Vec<u8>, LoadError> {
    match crate::http::download_capped(url, SPEC_MAX_BYTES, SPEC_TIMEOUT, allow_http) {
        Ok(bytes) => Ok(bytes),
        Err(crate::http::DownloadError::TooLarge) => Err(LoadError::TooLarge),
        Err(crate::http::DownloadError::Failed(msg)) => Err(LoadError::Download(msg)),
    }
}

fn strip_credentials(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.to_string()
    } else {
        raw.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_json() {
        assert_eq!(sniff_format(b"  {\"openapi\":\"3.1.0\"}"), SpecFormat::Json);
    }

    #[test]
    fn sniffs_yaml() {
        assert_eq!(sniff_format(b"openapi: 3.0.3\n"), SpecFormat::Yaml);
    }

    #[test]
    fn detects_3_0_and_3_1() {
        let (f, v) = detect_version(br#"{"openapi":"3.0.3"}"#, SpecFormat::Json);
        assert_eq!(f, OpenApiFamily::V3_0);
        assert_eq!(v, "3.0.3");
        let (f, v) = detect_version(b"openapi: \"3.1.0\"\n", SpecFormat::Yaml);
        assert_eq!(f, OpenApiFamily::V3_1);
        assert_eq!(v, "3.1.0");
    }

    #[test]
    fn load_bytes_sets_sha256() {
        let spec = load_bytes(
            b"openapi: 3.1.0\ninfo:\n  title: t\n  version: '1'\npaths: {}\n".to_vec(),
            SourceKind::File,
            "mem.yaml",
        )
        .unwrap();
        assert_eq!(spec.source.sha256.len(), 64);
        assert_eq!(spec.family, OpenApiFamily::V3_1);
        assert_eq!(spec.format, SpecFormat::Yaml);
    }

    #[test]
    fn rejects_oversize() {
        let err = load_bytes(vec![0; SPEC_MAX_BYTES + 1], SourceKind::Stdin, "-").unwrap_err();
        assert!(matches!(err, LoadError::TooLarge));
    }

    const LOCAL_SPEC: &[u8] = br#"{"openapi":"3.0.3","info":{"title":"Local","version":"1"},"paths":{"/ping":{"get":{"operationId":"ping","responses":{"200":{"description":"ok"}}}}}}"#;

    fn both_flags() -> crate::safety::SafetyOpts {
        crate::safety::SafetyOpts {
            allow_private: true,
            allow_insecure_http: true,
        }
    }

    /// One HTTP response, then the thread exits. Redirects are not followed by
    /// the client; this helper only serves the status line it is given.
    fn serve_once(status_and_headers: &'static str, body: &'static [u8]) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_secs(2)));
            let mut buf = Vec::new();
            let mut tmp = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
                match std::io::Read::read(&mut sock, &mut tmp) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                }
            }
            let head = format!(
                "{status_and_headers}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut sock, head.as_bytes());
            let _ = std::io::Write::write_all(&mut sock, body);
        });
        port
    }

    #[test]
    fn loads_loopback_http_only_with_both_flags() {
        let port = serve_once("HTTP/1.1 200 OK", LOCAL_SPEC);
        let url = format!("http://127.0.0.1:{port}/openapi.json");
        let missing = load_with(
            SpecSource::Url(url.clone()),
            crate::safety::SafetyOpts::default(),
        );
        assert!(missing.is_err(), "http without flags must be refused");
        let private_only = load_with(
            SpecSource::Url(url.clone()),
            crate::safety::SafetyOpts {
                allow_private: true,
                allow_insecure_http: false,
            },
        );
        assert!(private_only.is_err());
        let http_only = load_with(
            SpecSource::Url(url.clone()),
            crate::safety::SafetyOpts {
                allow_private: false,
                allow_insecure_http: true,
            },
        );
        assert!(http_only.is_err());
        let loaded = load_with(SpecSource::Url(url), both_flags()).unwrap();
        assert_eq!(loaded.family, OpenApiFamily::V3_0);
        assert!(loaded.bytes.windows(5).any(|w| w == b"Local"));
    }

    #[test]
    fn http_redirect_is_not_followed() {
        let port = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: http://example.com/stolen",
            b"",
        );
        let url = format!("http://127.0.0.1:{port}/openapi.json");
        let err = load_with(SpecSource::Url(url), both_flags()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("302"),
            "redirect must fail the download, got {msg}"
        );
    }
}
