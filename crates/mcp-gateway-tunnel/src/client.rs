use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use http::header::AUTHORIZATION;
use mcp_gateway_tunnel_proto::{
    Hello, Reclaim, RejectCode, RelayHandshake, TunnelMode, Welcome, HELLO_TIMEOUT_SECS,
    MAX_WS_MESSAGE_BYTES,
};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{protocol::WebSocketConfig, Message},
    MaybeTlsStream, WebSocketStream,
};

use crate::TunnelConfig;

/// The relay closes the hello window at [`HELLO_TIMEOUT_SECS`]. Wait longer so a
/// slow welcome is still a rejection rather than a local timeout racing the relay.
const WELCOME_WAIT: Duration = Duration::from_secs((HELLO_TIMEOUT_SECS as u64) + 5);

pub(crate) struct Opened {
    pub socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pub welcome: Welcome,
}

pub(crate) enum Handshake {
    Open(Box<Opened>),
    /// Drop any stored reclaim and open a fresh anonymous session.
    ReclaimAgain,
    Wait(Duration),
    Terminal {
        code: RejectCode,
        message: String,
    },
    Failed,
}

pub(crate) async fn connect(cfg: &TunnelConfig, reclaim: Option<&Reclaim>) -> Handshake {
    let mut ws = WebSocketConfig::default();
    ws.max_message_size = Some(MAX_WS_MESSAGE_BYTES);
    ws.max_frame_size = Some(MAX_WS_MESSAGE_BYTES);
    let Ok(mut request) = cfg.relay_url.clone().into_client_request() else {
        return Handshake::Failed;
    };
    if let Some(token) = cfg
        .bearer
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let Ok(value) = format!("Bearer {token}").parse() else {
            return Handshake::Failed;
        };
        request.headers_mut().insert(AUTHORIZATION, value);
    }
    let connected = connect_async_with_config(request, Some(ws), true).await;
    let Ok((mut socket, _)) = connected else {
        return Handshake::Failed;
    };
    let mode = match &cfg.name {
        Some(name) => TunnelMode::Named { name: name.clone() },
        None => TunnelMode::Anonymous,
    };
    let hello = Hello::new(
        cfg.client.clone(),
        mode,
        cfg.auth_mode,
        reclaim.cloned(),
        cfg.mcp_path.clone(),
    );
    let Ok(text) = serde_json::to_string(&hello) else {
        return Handshake::Failed;
    };
    if socket.send(Message::Text(text.into())).await.is_err() {
        return Handshake::Failed;
    }
    let frame = tokio::time::timeout(WELCOME_WAIT, next_text(&mut socket)).await;
    let Ok(Ok(Some(text))) = frame else {
        return Handshake::Failed;
    };
    match RelayHandshake::from_json_str(&text) {
        Ok(RelayHandshake::Welcome(welcome)) => {
            Handshake::Open(Box::new(Opened { socket, welcome }))
        }
        Ok(RelayHandshake::Rejected(rejected)) => match rejected.code {
            RejectCode::ReclaimExpired | RejectCode::ReclaimInvalid => Handshake::ReclaimAgain,
            RejectCode::RateLimited => Handshake::Wait(Duration::from_secs(u64::from(
                rejected.retry_after_secs.unwrap_or(1),
            ))),
            RejectCode::Maintenance => Handshake::Wait(Duration::from_secs(1)),
            other => Handshake::Terminal {
                code: other,
                message: rejected.message,
            },
        },
        Err(_) => Handshake::Failed,
    }
}

async fn next_text(
    socket: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
) -> Result<Option<String>, ()> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => return Ok(Some(text.to_string())),
            Some(Ok(Message::Ping(payload))) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    return Err(());
                }
            }
            Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(None),
            Some(Ok(_)) => {}
        }
    }
}
