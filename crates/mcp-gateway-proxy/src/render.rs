//! IR execution plan → HTTP request. Host comes only from the published base URL.

use crate::error::ProxyError;
use crate::headers::{is_reserved_request_header, strip_hop_by_hop};
use http::{header::HeaderName, HeaderMap, HeaderValue, Method};
use mcp_gateway_ir::{ArgBinding, BodyBinding, BodyMode, ExecutionPlan, Style};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::Value;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone)]
pub struct RenderedRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

pub fn render(
    base_url: &str,
    plan: &ExecutionPlan,
    arguments: &Value,
) -> Result<RenderedRequest, ProxyError> {
    let method = Method::from_bytes(plan.method.as_bytes())
        .map_err(|_| ProxyError::Schema("method".into()))?;
    let args = arguments
        .as_object()
        .ok_or_else(|| ProxyError::Schema("$".into()))?;

    let mut path = plan.path_template.clone();
    for binding in &plan.path_params {
        let value = arg_value(args, binding)?;
        let encoded = encode_path_segment(&value);
        reject_open_redirect(&encoded)?;
        let needle = format!("{{{}}}", binding.wire_name);
        if !path.contains(&needle) {
            let alt = format!("{{{}}}", binding.arg_name);
            path = path.replace(&alt, &encoded);
        } else {
            path = path.replace(&needle, &encoded);
        }
    }
    if path.contains('{') {
        return Err(ProxyError::Schema("path".into()));
    }
    if path.contains("://") || path.contains("..") {
        return Err(ProxyError::OpenRedirect);
    }

    let origin = Url::parse(base_url).map_err(|e| ProxyError::Schema(e.to_string()))?;
    if origin.host_str().is_none() {
        return Err(ProxyError::OpenRedirect);
    }
    let joined = join_openapi_server(&origin, &path)?;
    if joined.host_str() != origin.host_str() || joined.scheme() != origin.scheme() {
        return Err(ProxyError::OpenRedirect);
    }
    if !joined.username().is_empty() || joined.password().is_some() {
        return Err(ProxyError::OpenRedirect);
    }

    let mut url = joined;
    if !plan.query_params.is_empty() {
        {
            let mut pairs = url.query_pairs_mut();
            pairs.clear();
            for binding in &plan.query_params {
                if let Some(value) = optional_arg(args, binding) {
                    for (k, v) in
                        binding
                            .style
                            .serialise(binding.explode, &binding.wire_name, &value)
                    {
                        pairs.append_pair(&k, &v);
                    }
                } else if binding.required {
                    return Err(ProxyError::Schema(binding.arg_name.clone()));
                }
            }
        }
        if url.query().map(|q| q.is_empty()).unwrap_or(true) {
            url.set_query(None);
        }
    } else {
        url.set_query(None);
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::ACCEPT,
        HeaderValue::from_str(&plan.accept).unwrap_or(HeaderValue::from_static("application/json")),
    );
    headers.insert(
        http::header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    for binding in &plan.header_params {
        if is_reserved_request_header(&binding.wire_name) {
            return Err(ProxyError::ReservedHeader);
        }
        if let Some(value) = optional_arg(args, binding) {
            let pairs = binding
                .style
                .serialise(binding.explode, &binding.wire_name, &value);
            if let Some((_, v)) = pairs.first() {
                let name = HeaderName::from_bytes(binding.wire_name.as_bytes())
                    .map_err(|_| ProxyError::ReservedHeader)?;
                headers.insert(
                    name,
                    HeaderValue::from_str(v)
                        .map_err(|_| ProxyError::Schema(binding.arg_name.clone()))?,
                );
            }
        } else if binding.required {
            return Err(ProxyError::Schema(binding.arg_name.clone()));
        }
    }
    strip_hop_by_hop(&mut headers);

    let body = match &plan.body {
        None => Vec::new(),
        Some(binding) => encode_body(binding, arguments)?,
    };
    if body.len() > 1024 * 1024 {
        return Err(ProxyError::TooLarge);
    }
    if let Some(binding) = &plan.body {
        if !body.is_empty() {
            headers.insert(
                http::header::CONTENT_TYPE,
                HeaderValue::from_str(&binding.content_type)
                    .unwrap_or(HeaderValue::from_static("application/json")),
            );
        }
    }

    Ok(RenderedRequest {
        method,
        url,
        headers,
        body,
        timeout: Duration::from_millis(plan.timeout_ms.max(1)).min(Duration::from_secs(30)),
    })
}

fn arg_value(
    args: &serde_json::Map<String, Value>,
    binding: &ArgBinding,
) -> Result<Value, ProxyError> {
    args.get(&binding.arg_name)
        .cloned()
        .ok_or_else(|| ProxyError::Schema(binding.arg_name.clone()))
}

fn optional_arg(args: &serde_json::Map<String, Value>, binding: &ArgBinding) -> Option<Value> {
    args.get(&binding.arg_name).cloned()
}

fn encode_path_segment(value: &Value) -> String {
    let raw = match value {
        Value::String(s) => s.clone(),
        other => other.to_string().trim_matches('"').to_owned(),
    };
    utf8_percent_encode(&raw, NON_ALPHANUMERIC).to_string()
}

fn reject_open_redirect(encoded: &str) -> Result<(), ProxyError> {
    let decoded = percent_encoding::percent_decode_str(encoded)
        .decode_utf8_lossy()
        .to_ascii_lowercase();
    if decoded.contains("://") || decoded.contains("..") || decoded.contains('/') {
        return Err(ProxyError::OpenRedirect);
    }
    Ok(())
}

fn encode_body(binding: &BodyBinding, arguments: &Value) -> Result<Vec<u8>, ProxyError> {
    let args = arguments
        .as_object()
        .ok_or_else(|| ProxyError::Schema("$".into()))?;
    match binding.mode {
        BodyMode::Whole => {
            let name = binding
                .arg_name
                .as_deref()
                .ok_or_else(|| ProxyError::Schema("body".into()))?;
            let value = args
                .get(name)
                .ok_or_else(|| ProxyError::Schema(name.into()))?;
            serde_json::to_vec(value).map_err(|_| ProxyError::Schema(name.into()))
        }
        BodyMode::Fold => {
            let mut obj = serde_json::Map::new();
            for key in &binding.folded_keys {
                if let Some(v) = args.get(&key.arg_name) {
                    obj.insert(key.wire_name.clone(), v.clone());
                }
            }
            serde_json::to_vec(&Value::Object(obj)).map_err(|_| ProxyError::Schema("body".into()))
        }
        BodyMode::FormUrlencoded => {
            let mut encoded = String::new();
            for key in &binding.folded_keys {
                if let Some(v) = args.get(&key.arg_name) {
                    if !encoded.is_empty() {
                        encoded.push('&');
                    }
                    let pairs = Style::Form.serialise(true, &key.wire_name, v);
                    encoded.push_str(
                        &pairs
                            .iter()
                            .map(|(k, val)| {
                                format!(
                                    "{}={}",
                                    utf8_percent_encode(k, NON_ALPHANUMERIC),
                                    utf8_percent_encode(val, NON_ALPHANUMERIC)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("&"),
                    );
                }
            }
            Ok(encoded.into_bytes())
        }
    }
}

/// OpenAPI server URL + operation path: append, do not RFC 3986-replace.
/// `https://host/api/v3` + `/store/inventory` → `https://host/api/v3/store/inventory`.
fn join_openapi_server(origin: &Url, op_path: &str) -> Result<Url, ProxyError> {
    let mut joined = origin.clone();
    let base_path = origin.path().trim_end_matches('/');
    let op = if op_path.starts_with('/') {
        op_path
    } else {
        return Err(ProxyError::Schema("path".into()));
    };
    let new_path = if op == "/" {
        if base_path.is_empty() {
            "/".to_owned()
        } else {
            base_path.to_owned()
        }
    } else {
        format!("{base_path}{op}")
    };
    joined.set_path(&new_path);
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_gateway_ir::ArgBinding;
    use serde_json::json;

    fn plan_get() -> ExecutionPlan {
        ExecutionPlan {
            method: "GET".into(),
            path_template: "/v1/widgets/{id}".into(),
            path_params: vec![ArgBinding {
                arg_name: "id".into(),
                wire_name: "id".into(),
                style: Style::Simple,
                explode: false,
                required: true,
            }],
            query_params: vec![],
            header_params: vec![],
            cookie_params: vec![],
            body: None,
            accept: "application/json".into(),
            timeout_ms: 15_000,
        }
    }

    #[test]
    fn substitutes_path_params() {
        let req = render(
            "https://api.example.com",
            &plan_get(),
            &json!({"id": "abc"}),
        )
        .unwrap();
        assert_eq!(req.url.as_str(), "https://api.example.com/v1/widgets/abc");
        assert_eq!(req.method, Method::GET);
    }

    #[test]
    fn rejects_absolute_url_in_path_param() {
        let err = render(
            "https://api.example.com",
            &plan_get(),
            &json!({"id": "https://evil.example/"}),
        )
        .unwrap_err();
        assert!(matches!(err, ProxyError::OpenRedirect));
    }

    #[test]
    fn rejects_dotdot_in_path_param() {
        let err = render("https://api.example.com", &plan_get(), &json!({"id": ".."})).unwrap_err();
        assert!(matches!(err, ProxyError::OpenRedirect));
    }

    #[test]
    fn leftover_path_placeholder_fails_closed() {
        let mut plan = plan_get();
        plan.path_params.clear();
        let err = render("https://api.example.com", &plan, &json!({"id": "abc"})).unwrap_err();
        assert!(matches!(err, ProxyError::Schema(_)));
    }

    #[test]
    fn appends_path_to_server_prefix() {
        let req = render(
            "https://petstore3.swagger.io/api/v3",
            &plan_get(),
            &json!({"id": "1"}),
        )
        .unwrap();
        assert_eq!(
            req.url.as_str(),
            "https://petstore3.swagger.io/api/v3/v1/widgets/1"
        );
    }
}
