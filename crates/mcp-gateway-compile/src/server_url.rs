//! OpenAPI `servers[].url` resolution against the document URL (OAS 3).

use url::Url;

/// True when `s` is an http(s) URL with a host after substituting `{variables}`.
pub fn is_absolute_http_url(s: &str) -> bool {
    let dummy = substitute_template_vars(s);
    matches!(
        Url::parse(&dummy),
        Ok(u) if u.has_host() && matches!(u.scheme(), "http" | "https")
    )
}

/// Resolve a server URL against the OpenAPI document URL.
///
/// Absolute http(s) templates are returned unchanged. Relative URLs (Petstore's
/// `/api/v3`) are joined with `document_url` per RFC 3986, which is what OAS 3
/// specifies for relative server URLs.
pub fn resolve_server_url(document_url: Option<&str>, server_url: &str) -> String {
    if is_absolute_http_url(server_url) {
        return server_url.to_owned();
    }
    let Some(doc) = document_url else {
        return server_url.to_owned();
    };
    match Url::parse(doc).and_then(|base| base.join(server_url)) {
        Ok(joined) if joined.has_host() => joined.to_string(),
        _ => server_url.to_owned(),
    }
}

fn substitute_template_vars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '{' {
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
            }
            out.push('x');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn petstore_relative_server_against_document() {
        let resolved = resolve_server_url(
            Some("https://petstore3.swagger.io/api/v3/openapi.json"),
            "/api/v3",
        );
        assert_eq!(resolved, "https://petstore3.swagger.io/api/v3");
    }

    #[test]
    fn leaves_absolute_github() {
        assert_eq!(
            resolve_server_url(
                Some("https://example.com/openapi.json"),
                "https://api.github.com"
            ),
            "https://api.github.com"
        );
    }

    #[test]
    fn keeps_relative_without_document() {
        assert_eq!(resolve_server_url(None, "/api/v3"), "/api/v3");
    }

    #[test]
    fn templates_with_variables_count_as_absolute() {
        assert!(is_absolute_http_url("https://api.example.com/{region}"));
        assert!(!is_absolute_http_url("/api/v3"));
        assert!(!is_absolute_http_url("api/v3"));
    }
}
