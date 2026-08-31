//! Pinned TCP+TLS dialer. Never re-resolves the hostname after classification.
//!
//! Customer URLs never go through `reqwest`. Control-plane HTTP is a separate
//! client in `mcp-gateway-server`.

use crate::credentials::{inject, InjectedCredential};
use crate::error::ProxyError;
use crate::headers::strip_hop_by_hop;
use crate::map::UpstreamResponse;
use crate::render::RenderedRequest;
use crate::ssrf::{pin_url, Pinned, Resolver, SsrfError, SsrfPolicy};
use bytes::Bytes;
use http::{header, HeaderMap, Method, Request, Uri};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use rustls::ClientConfig;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::timeout;
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TLS_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const IDLE_READ_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_CAP: usize = 2 * 1024 * 1024;
const DECOMPRESS_CAP: usize = 2 * 1024 * 1024;
const MAX_RATIO: usize = 10;
const HEADER_MAP_CAP: usize = 16 * 1024;

pub async fn validate_and_dial(
    req: RenderedRequest,
    policy: &SsrfPolicy,
    resolver: &dyn Resolver,
    cred: Option<&InjectedCredential>,
) -> Result<UpstreamResponse, ProxyError> {
    execute_hops(req, 0, policy, resolver, cred, None).await
}

async fn execute_hops(
    mut req: RenderedRequest,
    hop: u8,
    policy: &SsrfPolicy,
    resolver: &dyn Resolver,
    cred: Option<&InjectedCredential>,
    previous_origin: Option<(String, u16)>,
) -> Result<UpstreamResponse, ProxyError> {
    let pinned = pin_url(&req.url, hop, policy, resolver).await?;
    strip_hop_by_hop(&mut req.headers);
    if let Some(cred) = cred {
        let same_origin = previous_origin
            .as_ref()
            .map(|(h, p)| h == &pinned.hostname && *p == pinned.port)
            .unwrap_or(true);
        if same_origin {
            inject(&mut req.headers, &mut req.url, cred)?;
        }
    }
    let budget = req.timeout.min(REQUEST_TIMEOUT);
    let response = timeout(budget, dial_once(&pinned, &req))
        .await
        .map_err(|_| ProxyError::Timeout)??;

    if is_redirect(response.status) {
        if matches!(req.method, Method::POST | Method::PUT | Method::PATCH) {
            return Err(ProxyError::RedirectOnPost);
        }
        let location = response
            .headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ProxyError::Upstream("redirect without Location".into()))?;
        let next = req
            .url
            .join(location)
            .map_err(|_| ProxyError::Upstream("invalid Location".into()))?;
        req.url = next;
        req.method = Method::GET;
        req.body.clear();
        req.headers.remove(header::AUTHORIZATION);
        req.headers.remove(header::COOKIE);
        req.headers.remove(header::CONTENT_LENGTH);
        req.headers.remove(header::CONTENT_TYPE);
        return Box::pin(execute_hops(
            req,
            hop + 1,
            policy,
            resolver,
            cred,
            Some((pinned.hostname, pinned.port)),
        ))
        .await;
    }

    decode_body(response)
}

struct RawResponse {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

async fn dial_once(pinned: &Pinned, req: &RenderedRequest) -> Result<RawResponse, ProxyError> {
    install_crypto();
    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect(pinned.addr))
        .await
        .map_err(|_| ProxyError::Timeout)?
        .map_err(|e| ProxyError::Upstream(e.to_string()))?;
    let peer: SocketAddr = tcp
        .peer_addr()
        .map_err(|e| ProxyError::Upstream(e.to_string()))?;
    if peer.ip() != pinned.addr.ip() {
        return Err(ProxyError::PeerMismatch);
    }
    tcp.set_nodelay(true).ok();

    if pinned.scheme == "https" {
        let tls = handshake_tls(tcp, &pinned.hostname).await?;
        http1_exchange(tls, pinned, req).await
    } else {
        http1_exchange(tcp, pinned, req).await
    }
}

async fn handshake_tls(
    tcp: TcpStream,
    hostname: &str,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, ProxyError> {
    let config = tls_config();
    let connector = tokio_rustls::TlsConnector::from(config);
    let server_name = ServerName::try_from(hostname.to_owned())
        .map_err(|_| ProxyError::Ssrf(SsrfError::HostnameDenied(hostname.into())))?;
    timeout(TLS_TIMEOUT, connector.connect(server_name, tcp))
        .await
        .map_err(|_| ProxyError::Timeout)?
        .map_err(|e| ProxyError::Upstream(e.to_string()))
}

async fn http1_exchange<S>(
    stream: S,
    pinned: &Pinned,
    req: &RenderedRequest,
) -> Result<RawResponse, ProxyError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| ProxyError::Upstream(e.to_string()))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let uri = build_origin_form(&req.url)?;
    let mut builder = Request::builder().method(req.method.clone()).uri(uri);
    let host_value = if (pinned.scheme == "https" && pinned.port == 443)
        || (pinned.scheme == "http" && pinned.port == 80)
    {
        pinned.hostname.clone()
    } else {
        format!("{}:{}", pinned.hostname, pinned.port)
    };
    builder = builder.header(header::HOST, host_value);
    for (name, value) in &req.headers {
        if name == header::HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    if !req.body.is_empty() {
        builder = builder.header(header::CONTENT_LENGTH, req.body.len() as u64);
    }
    let request = builder
        .body(Full::new(Bytes::from(req.body.clone())))
        .map_err(|e| ProxyError::Upstream(e.to_string()))?;

    let response: http::Response<Incoming> =
        timeout(IDLE_READ_TIMEOUT, sender.send_request(request))
            .await
            .map_err(|_| ProxyError::Timeout)?
            .map_err(|e| ProxyError::Upstream(e.to_string()))?;

    let status = response.status().as_u16();
    let headers = response.headers().clone();
    enforce_header_budget(&headers)?;
    let mut body = Vec::new();
    let mut stream = response.into_body();
    loop {
        match timeout(IDLE_READ_TIMEOUT, stream.frame()).await {
            Err(_) => return Err(ProxyError::Timeout),
            Ok(None) => break,
            Ok(Some(Err(e))) => return Err(ProxyError::Upstream(e.to_string())),
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    if body.len() + data.len() > RESPONSE_CAP {
                        return Err(ProxyError::TooLarge);
                    }
                    body.extend_from_slice(data);
                }
            }
        }
    }
    Ok(RawResponse {
        status,
        headers,
        body,
    })
}

fn build_origin_form(url: &Url) -> Result<Uri, ProxyError> {
    let mut path = url.path().to_owned();
    if path.is_empty() {
        path = "/".into();
    }
    if let Some(q) = url.query() {
        path.push('?');
        path.push_str(q);
    }
    path.parse::<Uri>()
        .map_err(|e| ProxyError::Upstream(e.to_string()))
}

fn enforce_header_budget(headers: &HeaderMap) -> Result<(), ProxyError> {
    if headers.len() > 64 {
        return Err(ProxyError::TooLarge);
    }
    let mut total = 0usize;
    for (k, v) in headers {
        total += k.as_str().len() + v.as_bytes().len();
        if v.len() > 4 * 1024 || total > HEADER_MAP_CAP {
            return Err(ProxyError::TooLarge);
        }
    }
    Ok(())
}

fn decode_body(raw: RawResponse) -> Result<UpstreamResponse, ProxyError> {
    let encoding = raw
        .headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("identity")
        .to_ascii_lowercase();
    let body = match encoding.as_str() {
        "identity" | "" => raw.body,
        "gzip" => gunzip(&raw.body)?,
        "deflate" => inflate(&raw.body)?,
        "br" => brotli_decode(&raw.body)?,
        other => {
            return Err(ProxyError::Upstream(format!(
                "unsupported encoding {other}"
            )))
        }
    };
    if body.len() > DECOMPRESS_CAP {
        return Err(ProxyError::TooLarge);
    }
    let content_type = raw
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());
    Ok(UpstreamResponse {
        status: raw.status,
        content_type,
        body,
    })
}

fn gunzip(input: &[u8]) -> Result<Vec<u8>, ProxyError> {
    use flate2::read::GzDecoder;
    decode_with(input, |r, w| {
        let mut d = GzDecoder::new(r);
        std::io::copy(&mut d, w).map(|_| ())
    })
}

fn inflate(input: &[u8]) -> Result<Vec<u8>, ProxyError> {
    use flate2::read::DeflateDecoder;
    decode_with(input, |r, w| {
        let mut d = DeflateDecoder::new(r);
        std::io::copy(&mut d, w).map(|_| ())
    })
}

fn brotli_decode(input: &[u8]) -> Result<Vec<u8>, ProxyError> {
    decode_with(input, |r, w| {
        let mut d = brotli::Decompressor::new(r, 4096);
        std::io::copy(&mut d, w).map(|_| ())
    })
}

fn decode_with<F>(input: &[u8], f: F) -> Result<Vec<u8>, ProxyError>
where
    F: FnOnce(&[u8], &mut CapWriter) -> io::Result<()>,
{
    let mut out = CapWriter {
        buf: Vec::new(),
        compressed: input.len(),
    };
    f(input, &mut out).map_err(|_| ProxyError::TooLarge)?;
    Ok(out.buf)
}

struct CapWriter {
    buf: Vec<u8>,
    compressed: usize,
}

impl Write for CapWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.buf.len() + data.len() > DECOMPRESS_CAP {
            return Err(io::Error::other("decompressed response too large"));
        }
        if self.compressed > 0
            && self.buf.len() + data.len() > self.compressed.saturating_mul(MAX_RATIO)
        {
            return Err(io::Error::other("compression ratio exceeded"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn install_crypto() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn tls_config() -> Arc<ClientConfig> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            install_crypto();
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let mut config = ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Arc::new(config)
        })
        .clone()
}
