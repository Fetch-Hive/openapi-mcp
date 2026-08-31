//! Upstream credential injection. Secrets live in `SecretString` and never
//! appear in `Debug` / logs.

use crate::error::ProxyError;
use crate::headers::strip_hop_by_hop;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use http::{header::AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialSpec {
    None,
    ApiKeyHeader {
        ciphertext: String,
        #[serde(default = "default_api_key_header")]
        header_name: String,
    },
    ApiKeyQuery {
        ciphertext: String,
        param_name: String,
    },
    Bearer {
        ciphertext: String,
    },
    Basic {
        ciphertext: String,
    },
    CustomHeaders {
        ciphertext: String,
    },
}

fn default_api_key_header() -> String {
    "X-API-Key".into()
}

#[derive(Clone)]
pub struct InjectedCredential {
    pub kind: InjectedKind,
    pub secret: SecretString,
    pub header_name: Option<String>,
    pub param_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectedKind {
    None,
    ApiKeyHeader,
    ApiKeyQuery,
    Bearer,
    Basic,
    CustomHeaders,
}

impl std::fmt::Debug for InjectedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InjectedCredential")
            .field("kind", &self.kind)
            .field("secret", &"<redacted>")
            .field("header_name", &self.header_name)
            .field("param_name", &self.param_name)
            .finish()
    }
}

impl InjectedCredential {
    pub fn none() -> Self {
        Self {
            kind: InjectedKind::None,
            secret: SecretString::from(""),
            header_name: None,
            param_name: None,
        }
    }

    pub fn bearer(secret: SecretString) -> Self {
        Self {
            kind: InjectedKind::Bearer,
            secret,
            header_name: None,
            param_name: None,
        }
    }

    pub fn basic(secret: SecretString) -> Self {
        Self {
            kind: InjectedKind::Basic,
            secret,
            header_name: None,
            param_name: None,
        }
    }

    pub fn api_key_header(secret: SecretString, header_name: impl Into<String>) -> Self {
        Self {
            kind: InjectedKind::ApiKeyHeader,
            secret,
            header_name: Some(header_name.into()),
            param_name: None,
        }
    }

    pub fn api_key_query(secret: SecretString, param_name: impl Into<String>) -> Self {
        Self {
            kind: InjectedKind::ApiKeyQuery,
            secret,
            header_name: None,
            param_name: Some(param_name.into()),
        }
    }

    pub fn custom_headers(secret: SecretString) -> Self {
        Self {
            kind: InjectedKind::CustomHeaders,
            secret,
            header_name: None,
            param_name: None,
        }
    }

    pub fn unwrap(spec: &CredentialSpec, key: &SecretString) -> Result<Self, ProxyError> {
        match spec {
            CredentialSpec::None => Ok(Self {
                kind: InjectedKind::None,
                secret: SecretString::from(""),
                header_name: None,
                param_name: None,
            }),
            CredentialSpec::ApiKeyHeader {
                ciphertext,
                header_name,
            } => Ok(Self {
                kind: InjectedKind::ApiKeyHeader,
                secret: decrypt(ciphertext, key)?,
                header_name: Some(header_name.clone()),
                param_name: None,
            }),
            CredentialSpec::ApiKeyQuery {
                ciphertext,
                param_name,
            } => Ok(Self {
                kind: InjectedKind::ApiKeyQuery,
                secret: decrypt(ciphertext, key)?,
                header_name: None,
                param_name: Some(param_name.clone()),
            }),
            CredentialSpec::Bearer { ciphertext } => Ok(Self {
                kind: InjectedKind::Bearer,
                secret: decrypt(ciphertext, key)?,
                header_name: None,
                param_name: None,
            }),
            CredentialSpec::Basic { ciphertext } => Ok(Self {
                kind: InjectedKind::Basic,
                secret: decrypt(ciphertext, key)?,
                header_name: None,
                param_name: None,
            }),
            CredentialSpec::CustomHeaders { ciphertext } => Ok(Self {
                kind: InjectedKind::CustomHeaders,
                secret: decrypt(ciphertext, key)?,
                header_name: None,
                param_name: None,
            }),
        }
    }
}

pub fn encrypt(plaintext: &str, key: &SecretString) -> Result<String, ProxyError> {
    let cipher = cipher_from_key(key)?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut pt = plaintext.as_bytes().to_vec();
    let ct = cipher
        .encrypt(nonce, pt.as_ref())
        .map_err(|_| ProxyError::Credential)?;
    pt.zeroize();
    let mut packed = nonce_bytes.to_vec();
    packed.extend_from_slice(&ct);
    Ok(format!(
        "v1:{}",
        base64::engine::general_purpose::STANDARD.encode(packed)
    ))
}

pub fn decrypt(ciphertext: &str, key: &SecretString) -> Result<SecretString, ProxyError> {
    // Fixture-only escape hatch. Release binaries reject it so a published
    // config cannot smuggle plaintext through the hosted unwrap path.
    #[cfg(any(test, debug_assertions))]
    if let Some(rest) = ciphertext.strip_prefix("plain:") {
        return Ok(SecretString::from(rest));
    }
    let packed = ciphertext
        .strip_prefix("v1:")
        .ok_or(ProxyError::Credential)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(packed)
        .map_err(|_| ProxyError::Credential)?;
    if bytes.len() < 13 {
        return Err(ProxyError::Credential);
    }
    let cipher = cipher_from_key(key)?;
    let nonce = Nonce::from_slice(&bytes[..12]);
    let pt = cipher
        .decrypt(nonce, &bytes[12..])
        .map_err(|_| ProxyError::Credential)?;
    let s = String::from_utf8(pt).map_err(|_| ProxyError::Credential)?;
    Ok(SecretString::from(s))
}

fn cipher_from_key(key: &SecretString) -> Result<Aes256Gcm, ProxyError> {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(key.expose_secret().as_bytes());
    Aes256Gcm::new_from_slice(&hash).map_err(|_| ProxyError::Credential)
}

/// Inject after hop-by-hop stripping. An `Authorization` mapping from the plan
/// collides with bearer/basic and fails closed.
pub fn inject(
    headers: &mut HeaderMap,
    url: &mut url::Url,
    cred: &InjectedCredential,
) -> Result<(), ProxyError> {
    strip_hop_by_hop(headers);
    match cred.kind {
        InjectedKind::None => Ok(()),
        InjectedKind::ApiKeyHeader => {
            let name = cred.header_name.as_deref().unwrap_or("X-API-Key");
            set_header(headers, name, cred.secret.expose_secret())?;
            Ok(())
        }
        InjectedKind::ApiKeyQuery => {
            let param = cred.param_name.as_deref().unwrap_or("api_key");
            url.query_pairs_mut()
                .append_pair(param, cred.secret.expose_secret());
            Ok(())
        }
        InjectedKind::Bearer => {
            if headers.contains_key(AUTHORIZATION) {
                return Err(ProxyError::ReservedHeader);
            }
            set_header(
                headers,
                "authorization",
                &format!("Bearer {}", cred.secret.expose_secret()),
            )?;
            Ok(())
        }
        InjectedKind::Basic => {
            if headers.contains_key(AUTHORIZATION) {
                return Err(ProxyError::ReservedHeader);
            }
            let encoded = base64::engine::general_purpose::STANDARD
                .encode(cred.secret.expose_secret().as_bytes());
            set_header(headers, "authorization", &format!("Basic {encoded}"))?;
            Ok(())
        }
        InjectedKind::CustomHeaders => {
            let value: serde_json::Value = serde_json::from_str(cred.secret.expose_secret())
                .map_err(|_| ProxyError::Credential)?;
            if let Some(arr) = value.as_array() {
                for item in arr {
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .ok_or(ProxyError::Credential)?;
                    let val = item
                        .get("value")
                        .and_then(|v| v.as_str())
                        .ok_or(ProxyError::Credential)?;
                    set_header(headers, name, val)?;
                }
            }
            Ok(())
        }
    }
}

fn set_header(headers: &mut HeaderMap, name: &str, value: &str) -> Result<(), ProxyError> {
    let n = HeaderName::from_bytes(name.as_bytes()).map_err(|_| ProxyError::ReservedHeader)?;
    let v = HeaderValue::from_str(value).map_err(|_| ProxyError::Credential)?;
    headers.insert(n, v);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_does_not_contain_plaintext() {
        let cred = InjectedCredential {
            kind: InjectedKind::Bearer,
            secret: SecretString::from("supersecret-token-value"),
            header_name: None,
            param_name: None,
        };
        let rendered = format!("{cred:?}");
        assert!(
            !rendered.contains("supersecret-token-value"),
            "Debug leaked secret: {rendered}"
        );
    }

    #[test]
    fn encrypt_decrypt_round_trip() {
        let key = SecretString::from("unit-test-key");
        let ct = encrypt("hello", &key).unwrap();
        assert!(ct.starts_with("v1:"));
        let pt = decrypt(&ct, &key).unwrap();
        assert_eq!(pt.expose_secret(), "hello");
    }

    #[test]
    fn second_bearer_inject_is_reserved() {
        let cred = InjectedCredential {
            kind: InjectedKind::Bearer,
            secret: SecretString::from("tok"),
            header_name: None,
            param_name: None,
        };
        let mut headers = HeaderMap::new();
        let mut url = url::Url::parse("https://api.example.com/v1").unwrap();
        inject(&mut headers, &mut url, &cred).unwrap();
        let err = inject(&mut headers, &mut url, &cred).unwrap_err();
        assert!(matches!(err, ProxyError::ReservedHeader));
    }

    #[test]
    fn plain_prefix_round_trip_in_tests() {
        let key = SecretString::from("unit-test-key");
        let pt = decrypt("plain:fixture-secret", &key).unwrap();
        assert_eq!(pt.expose_secret(), "fixture-secret");
    }

    #[test]
    fn bearer_constructor_injects_and_redacts() {
        let cred = InjectedCredential::bearer(SecretString::from("bearer-secret"));
        let rendered = format!("{cred:?}");
        assert!(!rendered.contains("bearer-secret"));
        let mut headers = HeaderMap::new();
        let mut url = url::Url::parse("https://api.example.com/v1").unwrap();
        inject(&mut headers, &mut url, &cred).unwrap();
        assert_eq!(
            headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()),
            Some("Bearer bearer-secret")
        );
    }

    #[test]
    fn basic_constructor_injects_and_redacts() {
        let cred = InjectedCredential::basic(SecretString::from("user:pass"));
        assert!(!format!("{cred:?}").contains("user:pass"));
        let mut headers = HeaderMap::new();
        let mut url = url::Url::parse("https://api.example.com/v1").unwrap();
        inject(&mut headers, &mut url, &cred).unwrap();
        let got = headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        assert!(got.starts_with("Basic "));
        assert!(!got.contains("user:pass"));
    }

    #[test]
    fn api_key_header_constructor_injects_and_redacts() {
        let cred =
            InjectedCredential::api_key_header(SecretString::from("header-secret"), "X-API-Key");
        assert!(!format!("{cred:?}").contains("header-secret"));
        let mut headers = HeaderMap::new();
        let mut url = url::Url::parse("https://api.example.com/v1").unwrap();
        inject(&mut headers, &mut url, &cred).unwrap();
        assert_eq!(
            headers.get("x-api-key").and_then(|v| v.to_str().ok()),
            Some("header-secret")
        );
    }

    #[test]
    fn api_key_query_constructor_injects_and_redacts() {
        let cred = InjectedCredential::api_key_query(SecretString::from("query-secret"), "api_key");
        assert!(!format!("{cred:?}").contains("query-secret"));
        let mut headers = HeaderMap::new();
        let mut url = url::Url::parse("https://api.example.com/v1").unwrap();
        inject(&mut headers, &mut url, &cred).unwrap();
        assert!(url.as_str().contains("api_key=query-secret"));
    }
}
