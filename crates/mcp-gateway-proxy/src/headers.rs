//! Hop-by-hop and always-stripped headers (Phase 2 spec §4.8.7).

use http::{header::HeaderName, HeaderMap};

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

const ALWAYS_STRIP: &[&str] = &[
    "host",
    "cookie",
    "cookie2",
    "set-cookie",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "x-forwarded-server",
    "x-real-ip",
    "forwarded",
    "via",
];

pub fn strip_hop_by_hop(headers: &mut HeaderMap) {
    for name in HOP_BY_HOP.iter().chain(ALWAYS_STRIP.iter()) {
        if let Ok(n) = HeaderName::from_bytes(name.as_bytes()) {
            headers.remove(n);
        }
    }
    let extra: Vec<HeaderName> = headers
        .get_all(http::header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(','))
        .filter_map(|tok| HeaderName::from_bytes(tok.trim().as_bytes()).ok())
        .collect();
    for name in extra {
        headers.remove(name);
    }
}

pub fn is_reserved_request_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "host" || lower == "authorization" || ALWAYS_STRIP.contains(&lower.as_str())
}
