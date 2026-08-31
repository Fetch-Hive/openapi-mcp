use mcp_gateway_ir::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn effective_security(doc: &Value, op: &Value) -> Vec<SecurityRequirement> {
    let list = op
        .get("security")
        .or_else(|| doc.get("security"))
        .and_then(Value::as_array);
    let Some(list) = list else {
        return vec![];
    };
    list.iter()
        .filter_map(|req| {
            let obj = req.as_object()?;
            let mut schemes = BTreeMap::new();
            for (k, v) in obj {
                let scopes = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                schemes.insert(k.clone(), scopes);
            }
            Some(SecurityRequirement { schemes })
        })
        .collect()
}

pub fn security_supported(doc: &Value, reqs: &[SecurityRequirement]) -> bool {
    if reqs.is_empty() {
        return true;
    }
    let schemes = doc
        .pointer("/components/securitySchemes")
        .and_then(Value::as_object);
    reqs.iter().any(|req| {
        req.schemes.keys().all(|name| {
            let Some(scheme) = schemes.and_then(|m| m.get(name)) else {
                return false;
            };
            match scheme.get("type").and_then(Value::as_str) {
                Some("http") => scheme
                    .get("scheme")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.eq_ignore_ascii_case("bearer")),
                Some("apiKey") => scheme.get("in").and_then(Value::as_str) == Some("header"),
                _ => false,
            }
        })
    })
}

pub fn parse_security_schemes(doc: &Value) -> BTreeMap<String, SecurityScheme> {
    let mut out = BTreeMap::new();
    let Some(map) = doc
        .pointer("/components/securitySchemes")
        .and_then(Value::as_object)
    else {
        return out;
    };
    for (name, scheme) in map {
        let parsed = match scheme.get("type").and_then(Value::as_str) {
            Some("apiKey") => Some(SecurityScheme::ApiKey {
                name: scheme
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(name)
                    .to_owned(),
                location: match scheme.get("in").and_then(Value::as_str) {
                    Some("query") => ApiKeyLocation::Query,
                    Some("cookie") => ApiKeyLocation::Cookie,
                    _ => ApiKeyLocation::Header,
                },
            }),
            Some("http") => Some(SecurityScheme::Http {
                scheme: scheme
                    .get("scheme")
                    .and_then(Value::as_str)
                    .unwrap_or("bearer")
                    .to_owned(),
                bearer_format: scheme
                    .get("bearerFormat")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }),
            Some("oauth2") => Some(SecurityScheme::OAuth2 {
                flows: scheme.get("flows").cloned().unwrap_or(json!({})),
            }),
            Some("openIdConnect") => Some(SecurityScheme::OpenIdConnect {
                open_id_connect_url: scheme
                    .get("openIdConnectUrl")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }),
            Some("mutualTLS") => Some(SecurityScheme::MutualTls),
            _ => None,
        };
        if let Some(p) = parsed {
            out.insert(name.clone(), p);
        }
    }
    out
}
