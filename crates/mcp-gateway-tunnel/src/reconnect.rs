use std::sync::Arc;
use std::time::Duration;

use mcp_gateway_tunnel_proto::Reclaim;
use rand::Rng;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::client::{self, Handshake};
use crate::session;
use crate::stats::Stats;
use crate::{FinishedRequest, LocalService, TunnelConfig, TunnelState};

pub(crate) async fn supervise<S: LocalService>(
    cfg: TunnelConfig,
    service: S,
    shutdown: CancellationToken,
    state: watch::Sender<TunnelState>,
    public_url: watch::Sender<Option<String>>,
    stats: Arc<Stats>,
    requests: mpsc::UnboundedSender<FinishedRequest>,
) {
    let mut reclaim: Option<Reclaim> = None;
    let mut attempt: u32 = 0;
    loop {
        if shutdown.is_cancelled() {
            let _ = state.send(TunnelState::Stopped);
            return;
        }
        if attempt == 0 && reclaim.is_none() && !matches!(*state.borrow(), TunnelState::Connecting)
        {
            let _ = state.send(TunnelState::Connecting);
        }
        match client::connect(&cfg, reclaim.as_ref()).await {
            Handshake::Open(opened) => {
                attempt = 0;
                let slug = opened.welcome.slug.clone();
                let url = opened.welcome.public_url.clone();
                let _ = public_url.send(Some(url.clone()));
                let _ = state.send(TunnelState::Connected { slug, url });
                let stop = session::run(
                    opened.socket,
                    opened.welcome,
                    cfg.clone(),
                    service.clone(),
                    shutdown.clone(),
                    stats.clone(),
                    requests.clone(),
                )
                .await;
                if stop.shutdown {
                    let _ = state.send(TunnelState::Stopped);
                    return;
                }
                if let Some((code, message)) = stop.terminal {
                    let _ = state.send(TunnelState::Rejected { code, message });
                    return;
                }
                if stop.fresh {
                    reclaim = None;
                } else {
                    reclaim = stop.reclaim;
                }
                let delay = stop.after.unwrap_or_else(|| backoff_delay(attempt));
                if !wait_retry(&shutdown, &state, &stats, attempt, delay, true).await {
                    return;
                }
                attempt = attempt.saturating_add(1);
            }
            Handshake::ReclaimAgain => {
                reclaim = None;
                let delay = backoff_delay(attempt);
                if !wait_retry(&shutdown, &state, &stats, attempt, delay, true).await {
                    return;
                }
                attempt = attempt.saturating_add(1);
            }
            Handshake::Wait(delay) => {
                attempt = 0;
                if !wait_retry(&shutdown, &state, &stats, attempt, delay, false).await {
                    return;
                }
            }
            Handshake::Terminal { code, message } => {
                let _ = state.send(TunnelState::Rejected {
                    code: reject_name(code),
                    message,
                });
                return;
            }
            Handshake::Failed => {
                let delay = backoff_delay(attempt);
                if !wait_retry(&shutdown, &state, &stats, attempt, delay, true).await {
                    return;
                }
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

/// Sleep `delay`, publishing `Reconnecting` first. Counts one reconnect when
/// `count` is set. Returns false when shutdown wins the sleep.
async fn wait_retry(
    shutdown: &CancellationToken,
    state: &watch::Sender<TunnelState>,
    stats: &Stats,
    attempt: u32,
    delay: Duration,
    count: bool,
) -> bool {
    if count {
        stats
            .reconnects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    let _ = state.send(TunnelState::Reconnecting { attempt, delay });
    if sleep_or_shutdown(shutdown, delay).await {
        true
    } else {
        let _ = state.send(TunnelState::Stopped);
        false
    }
}

fn reject_name(code: mcp_gateway_tunnel_proto::RejectCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "rejected".into())
}

/// 1s, 2s, 4s, … capped at 60s. `attempt` 0 is the first retry.
#[must_use]
pub fn backoff_secs(attempt: u32) -> u64 {
    let shift = attempt.min(6);
    (1u64 << shift).min(60)
}

#[must_use]
pub fn backoff_delay(attempt: u32) -> Duration {
    let base = backoff_secs(attempt) as f64;
    let jitter: f64 = rand::thread_rng().gen_range(0.8..1.2);
    Duration::from_secs_f64((base * jitter).max(0.05))
}

async fn sleep_or_shutdown(shutdown: &CancellationToken, delay: Duration) -> bool {
    tokio::select! {
        _ = shutdown.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::backoff_secs;

    #[test]
    fn backoff_doubles_until_sixty_seconds() {
        assert_eq!(backoff_secs(0), 1);
        assert_eq!(backoff_secs(1), 2);
        assert_eq!(backoff_secs(2), 4);
        assert_eq!(backoff_secs(5), 32);
        assert_eq!(backoff_secs(6), 60);
        assert_eq!(backoff_secs(20), 60);
    }
}
