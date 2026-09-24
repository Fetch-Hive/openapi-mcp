//! Client for the Fetch Hive MCP tunnel.
//!
//! `run` dials the relay, answers proxied `/mcp` calls with `service`, and
//! reconnects until shutdown or a terminal rejection. The reclaim credential
//! stays in memory for this process.

mod client;
mod reconnect;
mod session;
mod stats;

use std::future::Future;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

pub use mcp_gateway_tunnel_proto::{EndpointAuthMode, DEFAULT_RELAY_URL, RELAY_URL_ENV};
pub use stats::Stats;

/// Env, then config, then [`DEFAULT_RELAY_URL`].
#[must_use]
pub fn resolve_relay_url(config_value: &str) -> String {
    if let Ok(value) = std::env::var(RELAY_URL_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    let trimmed = config_value.trim();
    if trimmed.is_empty() {
        DEFAULT_RELAY_URL.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// `mcp-gateway/<version> (<os>-<arch>)`.
#[must_use]
pub fn client_identity() -> String {
    format!(
        "mcp-gateway/{} ({}-{})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

#[derive(Debug, Clone)]
pub struct TunnelConfig {
    pub relay_url: String,
    /// `Host` sent to the local server, including the port (`127.0.0.1:8787`).
    pub local_authority: String,
    /// Path the local server mounted. Public traffic always arrives at `/mcp`.
    pub mcp_path: String,
    pub auth_mode: EndpointAuthMode,
    pub client: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TunnelState {
    Connecting,
    Connected { slug: String, url: String },
    Reconnecting,
    Rejected { code: String, message: String },
    Stopped,
}

pub struct TunnelHandle {
    pub public_url: watch::Receiver<Option<String>>,
    pub state: watch::Receiver<TunnelState>,
    pub stats: std::sync::Arc<Stats>,
    pub shutdown: CancellationToken,
    pub done: watch::Receiver<bool>,
}

impl TunnelHandle {
    /// Resolves when the supervisor has exited.
    pub async fn join(&mut self) {
        if *self.done.borrow() {
            return;
        }
        let _ = self.done.changed().await;
    }
}

/// Local HTTP server the tunnel calls. [`axum::Router`] implements this.
pub trait LocalService: Clone + Send + Sync + 'static {
    fn call(
        &self,
        request: http::Request<axum::body::Body>,
    ) -> impl Future<Output = http::Response<axum::body::Body>> + Send;
}

impl LocalService for axum::Router {
    fn call(
        &self,
        request: http::Request<axum::body::Body>,
    ) -> impl Future<Output = http::Response<axum::body::Body>> + Send {
        let app = self.clone();
        async move {
            match app.oneshot(request).await {
                Ok(response) => response,
                Err(err) => match err {},
            }
        }
    }
}

/// Spawn the reconnect loop. Returns immediately.
pub fn run<S: LocalService>(config: TunnelConfig, service: S) -> TunnelHandle {
    let shutdown = CancellationToken::new();
    let (state_tx, state_rx) = watch::channel(TunnelState::Connecting);
    let (url_tx, url_rx) = watch::channel(None);
    let (done_tx, done_rx) = watch::channel(false);
    let stats = std::sync::Arc::new(Stats::default());
    let task_shutdown = shutdown.clone();
    let task_stats = stats.clone();
    tokio::spawn(async move {
        reconnect::supervise(config, service, task_shutdown, state_tx, url_tx, task_stats).await;
        let _ = done_tx.send(true);
    });
    TunnelHandle {
        public_url: url_rx,
        state: state_rx,
        stats,
        shutdown,
        done: done_rx,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TunnelError {
    #[error("tunnel rejected ({code}): {message}")]
    Rejected { code: String, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_the_production_relay() {
        if std::env::var(RELAY_URL_ENV)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return;
        }
        assert_eq!(resolve_relay_url(""), DEFAULT_RELAY_URL);
        assert_eq!(
            resolve_relay_url("wss://example.test/v1/tunnel"),
            "wss://example.test/v1/tunnel"
        );
    }
}
