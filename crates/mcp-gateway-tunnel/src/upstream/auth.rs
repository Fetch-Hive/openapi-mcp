//! Bearer gate in front of a tunnel upstream.
//!
//! Token mode compares `Authorization: Bearer` in constant time when the
//! lengths match. A missing or non-`Bearer ` value is `401` with
//! `WWW-Authenticate: Bearer` and `missing authorization`. A value of the
//! same length that does not match is `invalid authorization`. The header is
//! removed before the inner service runs, so the upstream never sees the
//! gateway token.
//!
//! Passthrough does not check and does not remove the header. Public does
//! not check and does remove it. A mismatch never calls the inner service.

use std::future::Future;

use axum::body::Body;
use base64::Engine;
use rand::RngCore;
use subtle::ConstantTimeEq;

/// 32 raw bytes, base64url, no padding. That encoding is 43 characters.
pub fn generate_bearer_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

use super::rpc_response;
use crate::LocalService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyAuth {
    Token,
    Passthrough,
    Public,
}

#[derive(Clone)]
pub struct AuthGate<S> {
    inner: S,
    mode: ProxyAuth,
    token: Option<String>,
}

impl<S> AuthGate<S> {
    pub fn new(mode: ProxyAuth, token: Option<String>, inner: S) -> Self {
        Self { inner, mode, token }
    }
}

impl<S: LocalService> LocalService for AuthGate<S> {
    fn call(
        &self,
        request: ::http::Request<Body>,
    ) -> impl Future<Output = ::http::Response<Body>> + Send {
        let inner = self.inner.clone();
        let mode = self.mode;
        let token = self.token.clone();
        async move {
            match mode {
                ProxyAuth::Passthrough => inner.call(request).await,
                ProxyAuth::Public => inner.call(strip_authorization(request)).await,
                ProxyAuth::Token => match bearer_status(request.headers(), token.as_deref()) {
                    BearerStatus::Match => inner.call(strip_authorization(request)).await,
                    BearerStatus::Missing => rpc_response(
                        401,
                        -32000,
                        "missing authorization",
                        serde_json::Value::Null,
                    ),
                    BearerStatus::Invalid => rpc_response(
                        401,
                        -32000,
                        "invalid authorization",
                        serde_json::Value::Null,
                    ),
                },
            }
        }
    }
}

enum BearerStatus {
    Match,
    Missing,
    Invalid,
}

fn bearer_status(headers: &::http::HeaderMap, expected: Option<&str>) -> BearerStatus {
    let Some(expected) = expected.filter(|value| !value.is_empty()) else {
        return BearerStatus::Missing;
    };
    let Some(got) = headers
        .get(::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return BearerStatus::Missing;
    };
    if got.len() == expected.len() && bool::from(got.as_bytes().ct_eq(expected.as_bytes())) {
        BearerStatus::Match
    } else {
        BearerStatus::Invalid
    }
}

fn strip_authorization(mut request: ::http::Request<Body>) -> ::http::Request<Body> {
    request.headers_mut().remove(::http::header::AUTHORIZATION);
    request
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn generated_token_is_32_bytes_of_base64url() {
        let token = generate_bearer_token();
        assert_eq!(token.len(), 43);
        assert!(token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'));
    }

    #[derive(Clone)]
    struct Record {
        hits: Arc<AtomicUsize>,
        auth: Arc<Mutex<Option<String>>>,
    }

    impl LocalService for Record {
        fn call(
            &self,
            request: ::http::Request<Body>,
        ) -> impl Future<Output = ::http::Response<Body>> + Send {
            let hits = self.hits.clone();
            let auth = self.auth.clone();
            async move {
                hits.fetch_add(1, Ordering::SeqCst);
                let value = request
                    .headers()
                    .get(::http::header::AUTHORIZATION)
                    .and_then(|header| header.to_str().ok())
                    .map(str::to_owned);
                *auth.lock().expect("lock") = value;
                ::http::Response::new(Body::from("ok"))
            }
        }
    }

    fn record() -> (
        AuthGate<Record>,
        Arc<AtomicUsize>,
        Arc<Mutex<Option<String>>>,
    ) {
        let hits = Arc::new(AtomicUsize::new(0));
        let auth = Arc::new(Mutex::new(None));
        let gate = AuthGate::new(
            ProxyAuth::Token,
            Some("secret".into()),
            Record {
                hits: hits.clone(),
                auth: auth.clone(),
            },
        );
        (gate, hits, auth)
    }

    fn request(bearer: Option<&str>) -> ::http::Request<Body> {
        let mut builder = ::http::Request::builder().method("POST").uri("/mcp");
        if let Some(bearer) = bearer {
            builder = builder.header(::http::header::AUTHORIZATION, format!("Bearer {bearer}"));
        }
        builder.body(Body::empty()).unwrap()
    }

    async fn body_text(response: ::http::Response<Body>) -> String {
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn missing_and_wrong_bearer_never_reach_the_upstream() {
        let (gate, hits, auth) = record();
        let missing = gate.call(request(None)).await;
        assert_eq!(missing.status(), 401);
        assert_eq!(
            missing
                .headers()
                .get(::http::header::WWW_AUTHENTICATE)
                .unwrap(),
            "Bearer"
        );
        assert!(body_text(missing).await.contains("missing authorization"));
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        let wrong = gate.call(request(Some("nope"))).await;
        assert_eq!(wrong.status(), 401);
        assert!(body_text(wrong).await.contains("invalid authorization"));
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert!(auth.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn matching_bearer_is_stripped_before_forwarding() {
        let (gate, hits, auth) = record();
        let response = gate.call(request(Some("secret"))).await;
        assert_eq!(response.status(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(auth.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn passthrough_keeps_the_header_and_public_removes_it() {
        let hits = Arc::new(AtomicUsize::new(0));
        let auth = Arc::new(Mutex::new(None));
        let inner = Record {
            hits: hits.clone(),
            auth: auth.clone(),
        };
        let pass = AuthGate::new(ProxyAuth::Passthrough, None, inner.clone());
        pass.call(request(Some("upstream-token"))).await;
        assert_eq!(
            auth.lock().unwrap().as_deref(),
            Some("Bearer upstream-token")
        );

        let public = AuthGate::new(ProxyAuth::Public, None, inner);
        public.call(request(Some("upstream-token"))).await;
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        assert!(auth.lock().unwrap().is_none());
    }
}
