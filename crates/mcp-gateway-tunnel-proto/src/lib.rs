//! Wire types for the MCP Gateway tunnel.
//!
//! No I/O and no async runtime. The CLI and a relay (hosted or self-hosted)
//! compile against this crate so a JSON text frame written by one deserialises
//! as the same value in the other. The normative description is
//! `docs/tunnel-protocol.md`.

#![forbid(unsafe_code)]

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u16 = 1;

/// Hosted relay ingress. Override with [`RELAY_URL_ENV`] for staging or a self-hosted relay.
pub const DEFAULT_RELAY_URL: &str = "wss://connect.mcp.fetchhive.com/v1/tunnel";

/// Production public suffix. `Welcome.public_url` is absolute; do not rebuild it from this
/// constant when the relay is staging or self-hosted.
pub const PUBLIC_SUFFIX: &str = "mcp.fetchhive.com";

pub const CONNECT_HOST: &str = "connect.mcp.fetchhive.com";

/// CLI env var that replaces [`DEFAULT_RELAY_URL`].
pub const RELAY_URL_ENV: &str = "MCP_GATEWAY_RELAY_URL";

/// Relay env var that replaces [`PUBLIC_SUFFIX`] when minting `Welcome.public_url`.
pub const PUBLIC_SUFFIX_ENV: &str = "MCP_TUNNEL_PUBLIC_SUFFIX";

pub const MCP_PATH: &str = "/mcp";
pub const HEALTH_PATH: &str = "/health";

/// Anonymous tunnel ids are this long and drawn from [`ANONYMOUS_ALPHABET`].
pub const ANONYMOUS_SLUG_LEN: usize = 8;

/// Crockford-like alphabet: no `0`, `o`, `1`, `l`, or `i`.
pub const ANONYMOUS_ALPHABET: &str = "abcdefghjkmnpqrstuvwxyz23456789";

pub const NAMED_SLUG_MIN_LEN: usize = 3;
pub const NAMED_SLUG_MAX_LEN: usize = 48;

/// Exact match, lowercase. Sorted so the spec line and binary search stay stable.
pub const RESERVED_SLUGS: &[&str] = &[
    "admin",
    "api",
    "app",
    "auth",
    "connect",
    "health",
    "internal",
    "login",
    "mcp",
    "null",
    "relay",
    "signup",
    "staging",
    "status",
    "undefined",
    "www",
];

/// Plaintext reclaim secret size. The relay stores SHA-256 only.
pub const RECLAIM_CREDENTIAL_BYTES: usize = 32;

/// base64url (no padding) length of [`RECLAIM_CREDENTIAL_BYTES`] random bytes.
pub const RECLAIM_CREDENTIAL_LEN: usize = 43;

pub const DEFAULT_MAX_BODY_BYTES: u64 = 1_048_576;
pub const DEFAULT_MAX_INFLIGHT: u32 = 16;
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u32 = 60;
pub const DEFAULT_RPM: u32 = 60;
pub const DEFAULT_ANON_LEASE_GRACE_SECS: u32 = 1800;
pub const DEFAULT_ANON_MAX_SESSION_SECS: u32 = 8 * 60 * 60;

/// Max size of one base64 `body` or `chunk` field. Larger HTTP bodies are split.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

/// Max size of one WebSocket text message, including JSON around the body field.
pub const MAX_WS_MESSAGE_BYTES: usize = 512 * 1024;

pub const HEARTBEAT_INTERVAL_SECS: u32 = 20;
pub const HEARTBEAT_TIMEOUT_SECS: u32 = 10;
pub const HELLO_TIMEOUT_SECS: u32 = 5;

pub const OFFLINE_RETRY_AFTER_SECS: u32 = 10;
pub const JSONRPC_ENDPOINT_OFFLINE_CODE: i32 = -32_001;

/// Body of the public `503` when the slug's lease exists and the CLI is disconnected.
pub const OFFLINE_JSONRPC_BODY: &str =
    r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"MCP endpoint offline"},"id":null}"#;

pub const HEADER_FORWARDED_FOR: &str = "X-Forwarded-For";
pub const HEADER_FORWARDED_PROTO: &str = "X-Forwarded-Proto";
pub const HEADER_FORWARDED_HOST: &str = "X-Forwarded-Host";
pub const HEADER_TUNNEL_REQUEST_ID: &str = "X-Tunnel-Request-Id";
pub const FORWARDED_PROTO_HTTPS: &str = "https";

pub const CONTENT_TYPE_JSON: &str = "application/json";
pub const CONTENT_TYPE_EVENT_STREAM: &str = "text/event-stream";

pub const CLOSE_NORMAL: u16 = 1000;
pub const CLOSE_POLICY: u16 = 1008;
pub const CLOSE_SERVICE_RESTART: u16 = 1012;
pub const CLOSE_TRY_AGAIN_LATER: u16 = 1013;
pub const CLOSE_AUTH: u16 = 4001;
pub const CLOSE_RATE_LIMITED: u16 = 4029;

/// Public HTTP status codes the relay uses on `https://<slug>.<suffix>/mcp`.
pub mod status {
    pub const NOT_JSONRPC: u16 = 400;
    pub const MISSING_AUTHORIZATION: u16 = 401;
    pub const UNKNOWN_SLUG: u16 = 404;
    pub const METHOD_NOT_ALLOWED: u16 = 405;
    pub const BODY_TOO_LARGE: u16 = 413;
    pub const UNSUPPORTED_MEDIA_TYPE: u16 = 415;
    pub const TOO_MANY_INFLIGHT: u16 = 429;
    /// In-flight request whose tunnel socket closed before `response_end`.
    pub const TUNNEL_CLOSED: u16 = 502;
    pub const OFFLINE: u16 = 503;
    pub const TIMEOUT: u16 = 504;
}

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// `https://{slug}.{PUBLIC_SUFFIX}/mcp`. Staging and self-hosted relays put the real URL in
/// [`Welcome::public_url`] instead of calling this.
#[must_use]
pub fn public_mcp_url(slug: &str) -> String {
    format!("https://{slug}.{PUBLIC_SUFFIX}/mcp")
}

#[must_use]
pub fn is_anonymous_shaped(slug: &str) -> bool {
    slug.len() == ANONYMOUS_SLUG_LEN
        && slug
            .bytes()
            .all(|byte| ANONYMOUS_ALPHABET.as_bytes().contains(&byte))
}

#[must_use]
pub fn is_reserved_slug(slug: &str) -> bool {
    RESERVED_SLUGS.binary_search(&slug).is_ok()
}

/// Named-slug check shared by the CLI and the relay.
///
/// Anonymous ids are allocated by the relay; this function rejects anything that
/// could be confused with one, plus the reserved words and the hostname grammar.
pub fn validate_named_slug(slug: &str) -> Result<(), SlugError> {
    if slug.is_empty() {
        return Err(SlugError::Empty);
    }
    if slug.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(SlugError::Uppercase);
    }
    if slug.len() < NAMED_SLUG_MIN_LEN {
        return Err(SlugError::TooShort);
    }
    if slug.len() > NAMED_SLUG_MAX_LEN {
        return Err(SlugError::TooLong);
    }
    if slug.starts_with('-') {
        return Err(SlugError::LeadingDash);
    }
    if slug.ends_with('-') {
        return Err(SlugError::TrailingDash);
    }
    if !slug
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(SlugError::InvalidCharset);
    }
    if is_reserved_slug(slug) {
        return Err(SlugError::Reserved);
    }
    if is_anonymous_shaped(slug) {
        return Err(SlugError::AnonymousShaped);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum SlugError {
    #[error("slug is empty")]
    Empty,
    #[error("slug is shorter than {NAMED_SLUG_MIN_LEN} characters")]
    TooShort,
    #[error("slug is longer than {NAMED_SLUG_MAX_LEN} characters")]
    TooLong,
    #[error("slug contains uppercase letters")]
    Uppercase,
    #[error("slug starts with a hyphen")]
    LeadingDash,
    #[error("slug ends with a hyphen")]
    TrailingDash,
    #[error("slug contains a character outside [a-z0-9-]")]
    InvalidCharset,
    #[error("slug is reserved")]
    Reserved,
    #[error("slug is shaped like an anonymous tunnel id")]
    AnonymousShaped,
}

/// RFC 7230 hop-by-hop names plus any `Proxy-*` header. ASCII case-insensitive.
///
/// `Cookie` is not hop-by-hop. Use [`is_stripped_request_header`] for the set the relay drops.
#[must_use]
pub fn is_hop_by_hop_header(name: &str) -> bool {
    if name
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("proxy-"))
    {
        return true;
    }
    HOP_BY_HOP.iter().any(|hop| name.eq_ignore_ascii_case(hop))
}

/// Headers the relay removes before forwarding. Everything else, including `Authorization`
/// and `Host`, is copied unchanged.
#[must_use]
pub fn is_stripped_request_header(name: &str) -> bool {
    is_hop_by_hop_header(name) || name.eq_ignore_ascii_case("cookie")
}

#[must_use]
pub fn is_mcp_method(method: &str) -> bool {
    matches!(method, "POST" | "GET" | "DELETE")
}

/// `true` when `value` is 32 bytes encoded as base64url without padding.
#[must_use]
pub fn is_reclaim_credential(value: &str) -> bool {
    value.len() == RECLAIM_CREDENTIAL_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
enum HandshakeTag {
    Hello,
    Welcome,
    Rejected,
}

fn deserialize_tag<'de, D>(
    deserializer: D,
    expected: &'static str,
    value: HandshakeTag,
) -> Result<HandshakeTag, D::Error>
where
    D: Deserializer<'de>,
{
    let tag = String::deserialize(deserializer)?;
    if tag == expected {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "expected t={expected}, got {tag}"
        )))
    }
}

fn deserialize_hello_tag<'de, D>(deserializer: D) -> Result<HandshakeTag, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_tag(deserializer, "hello", HandshakeTag::Hello)
}

fn deserialize_welcome_tag<'de, D>(deserializer: D) -> Result<HandshakeTag, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_tag(deserializer, "welcome", HandshakeTag::Welcome)
}

fn deserialize_rejected_tag<'de, D>(deserializer: D) -> Result<HandshakeTag, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_tag(deserializer, "rejected", HandshakeTag::Rejected)
}

/// First client frame after the WebSocket upgrade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Hello {
    #[serde(rename = "t", deserialize_with = "deserialize_hello_tag")]
    tag: HandshakeTag,
    pub version: u16,
    /// `mcp-gateway/<semver> (<target>)`, for example `mcp-gateway/0.7.0 (darwin-arm64)`.
    pub client: String,
    pub mode: TunnelMode,
    pub auth_mode: EndpointAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reclaim: Option<Reclaim>,
    /// Path the local server serves. The relay's public path is always [`MCP_PATH`].
    pub mcp_path: String,
}

impl Hello {
    #[must_use]
    pub fn new(
        client: impl Into<String>,
        mode: TunnelMode,
        auth_mode: EndpointAuthMode,
        reclaim: Option<Reclaim>,
        mcp_path: impl Into<String>,
    ) -> Self {
        Self {
            tag: HandshakeTag::Hello,
            version: PROTOCOL_VERSION,
            client: client.into(),
            mode,
            auth_mode,
            reclaim,
            mcp_path: mcp_path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TunnelMode {
    Anonymous,
    Named { name: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum EndpointAuthMode {
    /// Remote clients must send `Authorization`. The relay does not read the token.
    #[default]
    Token,
    /// Remote clients may omit `Authorization`. The local server must allow anonymous MCP.
    Public,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Reclaim {
    pub slug: String,
    /// Plaintext from the earlier [`Welcome::credential`].
    pub credential: String,
}

/// Relay → client. The slug is live and `credential` will not be sent again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Welcome {
    #[serde(rename = "t", deserialize_with = "deserialize_welcome_tag")]
    tag: HandshakeTag,
    pub slug: String,
    pub public_url: String,
    pub credential: String,
    pub lease_grace_secs: u32,
    /// Present for anonymous sessions. Omitted for named endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_session_secs: Option<u32>,
    pub limits: Limits,
    pub kind: EndpointKind,
}

impl Welcome {
    #[must_use]
    pub fn new(
        slug: impl Into<String>,
        public_url: impl Into<String>,
        credential: impl Into<String>,
        lease_grace_secs: u32,
        max_session_secs: Option<u32>,
        limits: Limits,
        kind: EndpointKind,
    ) -> Self {
        Self {
            tag: HandshakeTag::Welcome,
            slug: slug.into(),
            public_url: public_url.into(),
            credential: credential.into(),
            lease_grace_secs,
            max_session_secs,
            limits,
            kind,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub max_body_bytes: u64,
    pub max_inflight: u32,
    pub request_timeout_secs: u32,
    pub rpm: u32,
}

impl Limits {
    #[must_use]
    pub const fn anonymous_defaults() -> Self {
        Self {
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            max_inflight: DEFAULT_MAX_INFLIGHT,
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
            rpm: DEFAULT_RPM,
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::anonymous_defaults()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum EndpointKind {
    Anonymous,
    Named { endpoint_id: String },
}

/// Relay → client. The socket then closes with [`RejectCode::close_code`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct Rejected {
    #[serde(rename = "t", deserialize_with = "deserialize_rejected_tag")]
    tag: HandshakeTag,
    pub code: RejectCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u32>,
}

impl Rejected {
    #[must_use]
    pub fn new(
        code: RejectCode,
        message: impl Into<String>,
        retry_after_secs: Option<u32>,
    ) -> Self {
        Self {
            tag: HandshakeTag::Rejected,
            code,
            message: message.into(),
            retry_after_secs,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    VersionUnsupported,
    Unauthorized,
    NameTaken,
    NameInvalid,
    NameReserved,
    PlanLimit,
    RateLimited,
    ReclaimExpired,
    ReclaimInvalid,
    Maintenance,
}

impl RejectCode {
    /// Close code sent after this rejection. `GoAway` uses [`CLOSE_TRY_AGAIN_LATER`] on its own.
    #[must_use]
    pub const fn close_code(self) -> u16 {
        match self {
            Self::Unauthorized | Self::ReclaimInvalid => CLOSE_AUTH,
            Self::RateLimited => CLOSE_RATE_LIMITED,
            Self::Maintenance => CLOSE_TRY_AGAIN_LATER,
            Self::VersionUnsupported
            | Self::NameTaken
            | Self::NameInvalid
            | Self::NameReserved
            | Self::PlanLimit
            | Self::ReclaimExpired => CLOSE_POLICY,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayHandshake {
    Welcome(Welcome),
    Rejected(Rejected),
}

impl RelayHandshake {
    pub fn from_json_str(text: &str) -> Result<Self, ProtocolError> {
        Self::from_json_slice(text.as_bytes())
    }

    pub fn from_json_slice(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let value: Value = serde_json::from_slice(bytes)?;
        match value.get("t").and_then(Value::as_str) {
            Some("welcome") => Ok(Self::Welcome(serde_json::from_value(value)?)),
            Some("rejected") => Ok(Self::Rejected(serde_json::from_value(value)?)),
            other => Err(ProtocolError::UnknownHandshake {
                got: other.map(str::to_owned),
            }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("unknown relay handshake type {got:?}")]
    UnknownHandshake { got: Option<String> },
}

/// One WebSocket text frame after the handshake. `t` is the snake_case variant name.
///
/// Struct variants stay inline so the JSON stays flat. That makes the largest variants
/// much bigger than `Ping`; boxing them would change the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum Frame {
    RequestStart {
        id: u64,
        method: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        query: Option<String>,
        headers: Vec<(String, String)>,
        body_complete: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        client_ip: String,
        request_id: String,
    },
    RequestBody {
        id: u64,
        chunk: String,
        last: bool,
    },
    ResponseStart {
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        body_complete: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
    },
    ResponseBody {
        id: u64,
        chunk: String,
        last: bool,
    },
    Cancel {
        id: u64,
        reason: CancelReason,
    },
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    Stats {
        inflight: u32,
        total: u64,
    },
    GoAway {
        reason: String,
        reconnect_after_secs: u32,
    },
}

impl Frame {
    #[must_use]
    pub const fn direction(&self) -> FrameDirection {
        match self {
            Self::RequestStart { .. } | Self::RequestBody { .. } | Self::GoAway { .. } => {
                FrameDirection::RelayToClient
            }
            Self::ResponseStart { .. } | Self::ResponseBody { .. } | Self::Stats { .. } => {
                FrameDirection::ClientToRelay
            }
            Self::Cancel { .. } | Self::Ping { .. } | Self::Pong { .. } => FrameDirection::Either,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDirection {
    RelayToClient,
    ClientToRelay,
    Either,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    /// Relay waited [`Limits::request_timeout_secs`] and gave up.
    Timeout,
    /// Public HTTP client disconnected.
    ClientGone,
    /// CLI aborted the in-flight call.
    Client,
    /// Relay dropped the call (overload or local policy).
    Relay,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    fn assert_roundtrip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &back, "{json}");
    }

    fn hello_anonymous() -> Hello {
        Hello::new(
            "mcp-gateway/0.7.0 (darwin-arm64)",
            TunnelMode::Anonymous,
            EndpointAuthMode::Token,
            None,
            MCP_PATH,
        )
    }

    fn welcome_anonymous() -> Welcome {
        let slug = "x8kj32ab";
        Welcome::new(
            slug,
            public_mcp_url(slug),
            "A".repeat(RECLAIM_CREDENTIAL_LEN),
            DEFAULT_ANON_LEASE_GRACE_SECS,
            Some(DEFAULT_ANON_MAX_SESSION_SECS),
            Limits::anonymous_defaults(),
            EndpointKind::Anonymous,
        )
    }

    fn sample_frames() -> Vec<Frame> {
        vec![
            Frame::RequestStart {
                id: 1,
                method: "POST".into(),
                path: MCP_PATH.into(),
                query: None,
                headers: vec![
                    ("authorization".into(), "Bearer local-token".into()),
                    ("content-type".into(), CONTENT_TYPE_JSON.into()),
                    (HEADER_FORWARDED_FOR.into(), "203.0.113.4".into()),
                    (HEADER_FORWARDED_PROTO.into(), FORWARDED_PROTO_HTTPS.into()),
                    (
                        HEADER_FORWARDED_HOST.into(),
                        "x8kj32ab.mcp.fetchhive.com".into(),
                    ),
                    (
                        HEADER_TUNNEL_REQUEST_ID.into(),
                        "6b9f1c2e-4d3a-4f1b-9c8e-0a1b2c3d4e5f".into(),
                    ),
                ],
                body_complete: true,
                body: Some(
                    "eyJqc29ucnBjIjoiMi4wIiwiaWQiOjEsIm1ldGhvZCI6InRvb2xzL2xpc3QifQ==".into(),
                ),
                client_ip: "203.0.113.4".into(),
                request_id: "6b9f1c2e-4d3a-4f1b-9c8e-0a1b2c3d4e5f".into(),
            },
            Frame::RequestBody {
                id: 1,
                chunk: "eyJqc29ucnBjIjoiMi4wIn0=".into(),
                last: true,
            },
            Frame::ResponseStart {
                id: 1,
                status: 200,
                headers: vec![("content-type".into(), CONTENT_TYPE_EVENT_STREAM.into())],
                body_complete: false,
                body: None,
            },
            Frame::ResponseBody {
                id: 1,
                chunk: "ZGF0YTogb2sKCg==".into(),
                last: true,
            },
            Frame::Cancel {
                id: 1,
                reason: CancelReason::Timeout,
            },
            Frame::Ping { nonce: 7 },
            Frame::Pong { nonce: 7 },
            Frame::Stats {
                inflight: 1,
                total: 4,
            },
            Frame::GoAway {
                reason: "deploy".into(),
                reconnect_after_secs: 0,
            },
        ]
    }

    #[test]
    fn handshake_and_frames_roundtrip() {
        assert_roundtrip(&hello_anonymous());
        assert_roundtrip(&Hello::new(
            "mcp-gateway/0.7.0 (darwin-arm64)",
            TunnelMode::Named {
                name: "stripe-dev".into(),
            },
            EndpointAuthMode::Public,
            Some(Reclaim {
                slug: "stripe-dev".into(),
                credential: "A".repeat(RECLAIM_CREDENTIAL_LEN),
            }),
            MCP_PATH,
        ));
        assert_roundtrip(&welcome_anonymous());
        assert_roundtrip(&Welcome::new(
            "stripe-dev",
            public_mcp_url("stripe-dev"),
            "A".repeat(RECLAIM_CREDENTIAL_LEN),
            DEFAULT_ANON_LEASE_GRACE_SECS,
            None,
            Limits::anonymous_defaults(),
            EndpointKind::Named {
                endpoint_id: "01JXYZ".into(),
            },
        ));
        assert_roundtrip(&Rejected::new(
            RejectCode::VersionUnsupported,
            "protocol version 99 is not supported",
            Some(30),
        ));
        assert_roundtrip(&Rejected::new(
            RejectCode::Maintenance,
            "tunnel ingress is disabled",
            None,
        ));
        for frame in sample_frames() {
            assert_roundtrip(&frame);
        }
    }

    #[test]
    fn omitted_optional_fields_deserialize_as_none() {
        let welcome = Welcome::new(
            "stripe-dev",
            public_mcp_url("stripe-dev"),
            "A".repeat(RECLAIM_CREDENTIAL_LEN),
            1800,
            None,
            Limits::default(),
            EndpointKind::Named {
                endpoint_id: "ep_1".into(),
            },
        );
        let json = serde_json::to_value(&welcome).unwrap();
        assert!(json.get("max_session_secs").is_none());
        let hello = serde_json::to_value(hello_anonymous()).unwrap();
        assert!(hello.get("reclaim").is_none());
        assert_eq!(hello["t"], "hello");
        assert_eq!(hello["mode"]["type"], "anonymous");
        assert_eq!(hello["auth_mode"], "token");
    }

    #[test]
    fn relay_handshake_branches_on_t() {
        let welcome = serde_json::to_string(&welcome_anonymous()).unwrap();
        match RelayHandshake::from_json_str(&welcome).unwrap() {
            RelayHandshake::Welcome(parsed) => assert_eq!(parsed, welcome_anonymous()),
            RelayHandshake::Rejected(_) => panic!("welcome parsed as rejected"),
        }
        let rejected = serde_json::to_string(&Rejected::new(
            RejectCode::RateLimited,
            "too many tunnels",
            Some(60),
        ))
        .unwrap();
        match RelayHandshake::from_json_str(&rejected).unwrap() {
            RelayHandshake::Rejected(parsed) => {
                assert_eq!(parsed.code, RejectCode::RateLimited);
                assert_eq!(parsed.code.close_code(), CLOSE_RATE_LIMITED);
            }
            RelayHandshake::Welcome(_) => panic!("rejected parsed as welcome"),
        }
        let err = RelayHandshake::from_json_str(r#"{"t":"hello"}"#).unwrap_err();
        assert!(matches!(err, ProtocolError::UnknownHandshake { .. }));
    }

    #[test]
    fn unknown_fields_and_wrong_tag_are_rejected() {
        let mut hello = serde_json::to_value(hello_anonymous()).unwrap();
        hello["extra"] = Value::Bool(true);
        assert!(serde_json::from_value::<Hello>(hello).is_err());

        let mut as_welcome = serde_json::to_value(hello_anonymous()).unwrap();
        as_welcome["t"] = Value::String("welcome".into());
        assert!(serde_json::from_value::<Hello>(as_welcome).is_err());

        let err = serde_json::from_str::<Frame>(r#"{"t":"nope"}"#).unwrap_err();
        assert!(err.to_string().contains("nope") || err.to_string().contains("unknown"));
    }

    #[test]
    fn frame_tags_and_directions() {
        let cases = [
            (
                sample_frames().remove(0),
                "request_start",
                FrameDirection::RelayToClient,
            ),
            (
                Frame::RequestBody {
                    id: 2,
                    chunk: "YQ==".into(),
                    last: false,
                },
                "request_body",
                FrameDirection::RelayToClient,
            ),
            (
                Frame::ResponseStart {
                    id: 2,
                    status: 200,
                    headers: vec![],
                    body_complete: true,
                    body: None,
                },
                "response_start",
                FrameDirection::ClientToRelay,
            ),
            (
                Frame::ResponseBody {
                    id: 2,
                    chunk: "YQ==".into(),
                    last: true,
                },
                "response_body",
                FrameDirection::ClientToRelay,
            ),
            (
                Frame::Cancel {
                    id: 2,
                    reason: CancelReason::ClientGone,
                },
                "cancel",
                FrameDirection::Either,
            ),
            (Frame::Ping { nonce: 1 }, "ping", FrameDirection::Either),
            (Frame::Pong { nonce: 1 }, "pong", FrameDirection::Either),
            (
                Frame::Stats {
                    inflight: 0,
                    total: 0,
                },
                "stats",
                FrameDirection::ClientToRelay,
            ),
            (
                Frame::GoAway {
                    reason: "drain".into(),
                    reconnect_after_secs: 1,
                },
                "go_away",
                FrameDirection::RelayToClient,
            ),
        ];
        for (frame, tag, direction) in cases {
            let json = serde_json::to_value(&frame).unwrap();
            assert_eq!(json["t"], tag);
            assert_eq!(frame.direction(), direction);
        }
        assert_eq!(
            serde_json::to_value(CancelReason::ClientGone).unwrap(),
            "client_gone"
        );
    }

    #[test]
    fn named_slug_table() {
        let cases = [
            ("stripe-dev", Ok(())),
            ("a-b", Ok(())),
            ("a--b", Ok(())),
            ("9stripe", Ok(())),
            ("ab", Err(SlugError::TooShort)),
            ("Stripe", Err(SlugError::Uppercase)),
            ("-stripe", Err(SlugError::LeadingDash)),
            ("stripe-", Err(SlugError::TrailingDash)),
            ("stripe_dev", Err(SlugError::InvalidCharset)),
            ("connect", Err(SlugError::Reserved)),
            ("www", Err(SlugError::Reserved)),
            ("staging", Err(SlugError::Reserved)),
            ("internal", Err(SlugError::Reserved)),
            ("abcdefgh", Err(SlugError::AnonymousShaped)),
            ("", Err(SlugError::Empty)),
        ];
        for (slug, expected) in cases {
            assert_eq!(validate_named_slug(slug), expected, "{slug}");
        }
        let too_long = "a".repeat(NAMED_SLUG_MAX_LEN + 1);
        assert_eq!(validate_named_slug(&too_long), Err(SlugError::TooLong));
        assert!(validate_named_slug(&"a".repeat(NAMED_SLUG_MAX_LEN)).is_ok());
        assert!(is_anonymous_shaped("x8kj32ab"));
        assert!(!is_anonymous_shaped("x8kj32"));
        assert!(!is_anonymous_shaped("abcdefg1"));
        assert!(!is_anonymous_shaped("ABCDEFGH"));
        for slug in RESERVED_SLUGS {
            assert_eq!(validate_named_slug(slug), Err(SlugError::Reserved));
        }
    }

    #[test]
    fn header_classification() {
        for name in [
            "Connection",
            "keep-alive",
            "Transfer-Encoding",
            "Upgrade",
            "TE",
            "Trailer",
            "Proxy-Connection",
            "proxy-authorization",
        ] {
            assert!(is_hop_by_hop_header(name), "{name}");
            assert!(is_stripped_request_header(name), "{name}");
        }
        assert!(!is_hop_by_hop_header("cookie"));
        assert!(is_stripped_request_header("Cookie"));
        assert!(!is_hop_by_hop_header("12345é"));
        for kept in [
            "Host",
            "Authorization",
            "Content-Type",
            HEADER_FORWARDED_FOR,
        ] {
            assert!(!is_hop_by_hop_header(kept), "{kept}");
            assert!(!is_stripped_request_header(kept), "{kept}");
        }
        assert!(is_mcp_method("POST") && is_mcp_method("GET") && is_mcp_method("DELETE"));
        assert!(!is_mcp_method("PUT") && !is_mcp_method("post"));
    }

    #[test]
    fn reclaim_credential_shape() {
        assert!(is_reclaim_credential(&"A".repeat(RECLAIM_CREDENTIAL_LEN)));
        assert!(is_reclaim_credential(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQ"
        ));
        assert!(!is_reclaim_credential(
            &"A".repeat(RECLAIM_CREDENTIAL_LEN - 1)
        ));
        assert!(!is_reclaim_credential(&format!(
            "{}=",
            "A".repeat(RECLAIM_CREDENTIAL_LEN - 1)
        )));
    }

    #[test]
    fn reject_close_codes() {
        assert_eq!(RejectCode::Unauthorized.close_code(), CLOSE_AUTH);
        assert_eq!(RejectCode::ReclaimInvalid.close_code(), CLOSE_AUTH);
        assert_eq!(RejectCode::RateLimited.close_code(), CLOSE_RATE_LIMITED);
        assert_eq!(RejectCode::Maintenance.close_code(), CLOSE_TRY_AGAIN_LATER);
        assert_eq!(RejectCode::VersionUnsupported.close_code(), CLOSE_POLICY);
        assert_eq!(RejectCode::ReclaimExpired.close_code(), CLOSE_POLICY);
        assert_eq!(RejectCode::NameTaken.close_code(), CLOSE_POLICY);
        assert_eq!(status::OFFLINE, 503);
        assert_eq!(JSONRPC_ENDPOINT_OFFLINE_CODE, -32001);
        assert!(OFFLINE_JSONRPC_BODY.contains("-32001"));
    }

    #[test]
    fn spec_quotes_constants_and_examples() {
        let doc = include_str!("../../../docs/tunnel-protocol.md");
        let lines = [
            format!("PROTOCOL_VERSION = {PROTOCOL_VERSION}"),
            format!("DEFAULT_RELAY_URL = {DEFAULT_RELAY_URL}"),
            format!("PUBLIC_SUFFIX = {PUBLIC_SUFFIX}"),
            format!("CONNECT_HOST = {CONNECT_HOST}"),
            format!("RELAY_URL_ENV = {RELAY_URL_ENV}"),
            format!("PUBLIC_SUFFIX_ENV = {PUBLIC_SUFFIX_ENV}"),
            format!("MCP_PATH = {MCP_PATH}"),
            format!("HEALTH_PATH = {HEALTH_PATH}"),
            format!("ANONYMOUS_SLUG_LEN = {ANONYMOUS_SLUG_LEN}"),
            format!("ANONYMOUS_ALPHABET = {ANONYMOUS_ALPHABET}"),
            format!("RESERVED_SLUGS = {}", RESERVED_SLUGS.join(" ")),
            format!("NAMED_SLUG_MIN_LEN = {NAMED_SLUG_MIN_LEN}"),
            format!("NAMED_SLUG_MAX_LEN = {NAMED_SLUG_MAX_LEN}"),
            format!("RECLAIM_CREDENTIAL_BYTES = {RECLAIM_CREDENTIAL_BYTES}"),
            format!("RECLAIM_CREDENTIAL_LEN = {RECLAIM_CREDENTIAL_LEN}"),
            format!("DEFAULT_MAX_BODY_BYTES = {DEFAULT_MAX_BODY_BYTES}"),
            format!("DEFAULT_MAX_INFLIGHT = {DEFAULT_MAX_INFLIGHT}"),
            format!("DEFAULT_REQUEST_TIMEOUT_SECS = {DEFAULT_REQUEST_TIMEOUT_SECS}"),
            format!("DEFAULT_RPM = {DEFAULT_RPM}"),
            format!("DEFAULT_ANON_LEASE_GRACE_SECS = {DEFAULT_ANON_LEASE_GRACE_SECS}"),
            format!("DEFAULT_ANON_MAX_SESSION_SECS = {DEFAULT_ANON_MAX_SESSION_SECS}"),
            format!("MAX_FRAME_BYTES = {MAX_FRAME_BYTES}"),
            format!("MAX_WS_MESSAGE_BYTES = {MAX_WS_MESSAGE_BYTES}"),
            format!("HEARTBEAT_INTERVAL_SECS = {HEARTBEAT_INTERVAL_SECS}"),
            format!("HEARTBEAT_TIMEOUT_SECS = {HEARTBEAT_TIMEOUT_SECS}"),
            format!("HELLO_TIMEOUT_SECS = {HELLO_TIMEOUT_SECS}"),
            format!("OFFLINE_RETRY_AFTER_SECS = {OFFLINE_RETRY_AFTER_SECS}"),
            format!("JSONRPC_ENDPOINT_OFFLINE_CODE = {JSONRPC_ENDPOINT_OFFLINE_CODE}"),
            format!("OFFLINE_JSONRPC_BODY = {OFFLINE_JSONRPC_BODY}"),
            format!("HEADER_FORWARDED_FOR = {HEADER_FORWARDED_FOR}"),
            format!("HEADER_FORWARDED_PROTO = {HEADER_FORWARDED_PROTO}"),
            format!("HEADER_FORWARDED_HOST = {HEADER_FORWARDED_HOST}"),
            format!("HEADER_TUNNEL_REQUEST_ID = {HEADER_TUNNEL_REQUEST_ID}"),
            format!("FORWARDED_PROTO_HTTPS = {FORWARDED_PROTO_HTTPS}"),
            format!("CONTENT_TYPE_JSON = {CONTENT_TYPE_JSON}"),
            format!("CONTENT_TYPE_EVENT_STREAM = {CONTENT_TYPE_EVENT_STREAM}"),
            format!("CLOSE_NORMAL = {CLOSE_NORMAL}"),
            format!("CLOSE_POLICY = {CLOSE_POLICY}"),
            format!("CLOSE_SERVICE_RESTART = {CLOSE_SERVICE_RESTART}"),
            format!("CLOSE_TRY_AGAIN_LATER = {CLOSE_TRY_AGAIN_LATER}"),
            format!("CLOSE_AUTH = {CLOSE_AUTH}"),
            format!("CLOSE_RATE_LIMITED = {CLOSE_RATE_LIMITED}"),
            format!("STATUS_NOT_JSONRPC = {}", status::NOT_JSONRPC),
            format!(
                "STATUS_MISSING_AUTHORIZATION = {}",
                status::MISSING_AUTHORIZATION
            ),
            format!("STATUS_UNKNOWN_SLUG = {}", status::UNKNOWN_SLUG),
            format!("STATUS_METHOD_NOT_ALLOWED = {}", status::METHOD_NOT_ALLOWED),
            format!("STATUS_BODY_TOO_LARGE = {}", status::BODY_TOO_LARGE),
            format!(
                "STATUS_UNSUPPORTED_MEDIA_TYPE = {}",
                status::UNSUPPORTED_MEDIA_TYPE
            ),
            format!("STATUS_TOO_MANY_INFLIGHT = {}", status::TOO_MANY_INFLIGHT),
            format!("STATUS_TUNNEL_CLOSED = {}", status::TUNNEL_CLOSED),
            format!("STATUS_OFFLINE = {}", status::OFFLINE),
            format!("STATUS_TIMEOUT = {}", status::TIMEOUT),
        ];
        let mut missing = Vec::new();
        for line in &lines {
            if !doc.contains(line) {
                missing.push(line.clone());
            }
        }
        let mut examples = vec![
            serde_json::to_string_pretty(&hello_anonymous()).unwrap(),
            serde_json::to_string_pretty(&welcome_anonymous()).unwrap(),
            serde_json::to_string_pretty(&Rejected::new(
                RejectCode::VersionUnsupported,
                "protocol version 99 is not supported",
                Some(30),
            ))
            .unwrap(),
        ];
        for frame in sample_frames() {
            examples.push(serde_json::to_string_pretty(&frame).unwrap());
        }
        for example in &examples {
            if !doc.contains(example) {
                missing.push(example.clone());
            }
        }
        assert!(
            missing.is_empty(),
            "spec missing:\n{}",
            missing.join("\n---\n")
        );
    }

    #[test]
    fn proto_manifest_has_no_io_stack() {
        let manifest = include_str!("../Cargo.toml");
        for banned in ["tokio", "axum", "reqwest", "tungstenite", "hyper"] {
            assert!(
                !manifest.contains(banned),
                "{banned} must not be a proto dependency"
            );
        }
    }
}
