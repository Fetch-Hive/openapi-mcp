//! Capped blocking HTTPS GET used by the spec loader and `$ref` retriever.

use std::io::{self, Write};
use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("download failed: {0}")]
    Failed(String),
    #[error("response exceeds size cap")]
    TooLarge,
}

pub fn download_https_capped(
    url: &str,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, DownloadError> {
    // `reqwest::blocking` builds a private Tokio runtime. Dropping that runtime
    // panics when this function is called from inside another runtime (the CLI
    // runs every command under `block_on`). A fresh thread has no entered runtime.
    let url = url.to_owned();
    let handle = std::thread::Builder::new()
        .name("mcp-gateway-spec-download".into())
        .spawn(move || download_https_capped_blocking(&url, max_bytes, timeout))
        .map_err(|e| DownloadError::Failed(e.to_string()))?;
    match handle.join() {
        Ok(result) => result,
        Err(_) => Err(DownloadError::Failed("download thread panicked".to_owned())),
    }
}

fn download_https_capped_blocking(
    url: &str,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, DownloadError> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .https_only(true)
        .build()
        .map_err(|e| DownloadError::Failed(e.to_string()))?;
    let mut response = client
        .get(url)
        .send()
        .map_err(|e| DownloadError::Failed(e.to_string()))?;
    if !response.status().is_success() {
        return Err(DownloadError::Failed(format!("HTTP {}", response.status())));
    }
    if let Some(len) = response.content_length() {
        if len as usize > max_bytes {
            return Err(DownloadError::TooLarge);
        }
    }
    let mut writer = CappedWriter {
        buf: Vec::new(),
        max: max_bytes,
    };
    response
        .copy_to(&mut writer)
        .map_err(|e| DownloadError::Failed(e.to_string()))?;
    Ok(writer.buf)
}

struct CappedWriter {
    buf: Vec<u8>,
    max: usize,
}

impl Write for CappedWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.buf.len().saturating_add(data.len()) > self.max {
            return Err(io::Error::other("response exceeds size cap"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_inside_a_tokio_runtime_returns_an_error() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let err = rt.block_on(async {
            download_https_capped("https://127.0.0.1:1/", 64, Duration::from_millis(500))
        });
        assert!(err.is_err(), "expected a connection error, not a panic");
    }
}
