//! Destructive detection and MCP annotation mapping.

use mcp_gateway_ir::ToolAnnotations;

pub const DESTRUCTIVE_KEYWORDS: &[&str] = &[
    "delete",
    "cancel",
    "remove",
    "revoke",
    "terminate",
    "destroy",
    "refund",
    "transfer",
    "purge",
    "reset",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classification {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub skip: bool,
}

pub fn classify(method: &str, path_template: &str, operation_id: Option<&str>) -> Classification {
    let method = method.to_ascii_uppercase();
    let keyword = keyword_hit(path_template, operation_id);
    match method.as_str() {
        "GET" | "HEAD" => Classification {
            read_only: !keyword,
            destructive: keyword,
            idempotent: true,
            skip: false,
        },
        "PUT" | "PATCH" => Classification {
            read_only: false,
            destructive: keyword,
            idempotent: true,
            skip: false,
        },
        "POST" => Classification {
            read_only: false,
            destructive: keyword,
            idempotent: false,
            skip: false,
        },
        "DELETE" => Classification {
            read_only: false,
            destructive: true,
            idempotent: true,
            skip: false,
        },
        "OPTIONS" | "TRACE" => Classification {
            read_only: false,
            destructive: false,
            idempotent: false,
            skip: true,
        },
        _ => Classification {
            read_only: false,
            destructive: false,
            idempotent: false,
            skip: true,
        },
    }
}

pub fn keyword_hit(path_template: &str, operation_id: Option<&str>) -> bool {
    let mut blob = path_template.to_ascii_lowercase();
    if let Some(id) = operation_id {
        blob.push(' ');
        blob.push_str(&id.to_ascii_lowercase());
    }
    let mut tokens = blob.split(|c: char| !c.is_ascii_alphanumeric());
    tokens.any(|tok| DESTRUCTIVE_KEYWORDS.contains(&tok))
}

pub fn annotations(class: Classification) -> ToolAnnotations {
    if class.read_only {
        ToolAnnotations {
            read_only_hint: true,
            destructive_hint: None,
            idempotent_hint: None,
            open_world_hint: true,
        }
    } else {
        ToolAnnotations {
            read_only_hint: false,
            destructive_hint: Some(class.destructive),
            idempotent_hint: Some(class.idempotent),
            open_world_hint: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_refund_is_destructive() {
        let c = classify("GET", "/v1/charges/{id}/refund", Some("refundCharge"));
        assert!(c.destructive);
        assert!(!c.read_only);
    }

    #[test]
    fn delete_without_keyword_is_destructive() {
        let c = classify("DELETE", "/repos/{owner}/{repo}", Some("repos/delete"));
        assert!(c.destructive);
        assert!(c.idempotent);
        let a = annotations(c);
        assert!(!a.read_only_hint);
        assert_eq!(a.destructive_hint, Some(true));
        assert_eq!(a.idempotent_hint, Some(true));
    }

    #[test]
    fn get_is_read_only() {
        let c = classify("GET", "/repos/{owner}/{repo}", Some("repos/get"));
        assert!(c.read_only);
        assert!(!c.destructive);
    }

    #[test]
    fn options_skipped() {
        assert!(classify("OPTIONS", "/", None).skip);
        assert!(classify("TRACE", "/", None).skip);
    }
}
