//! Tool name generation. Pure, locale-independent.

const RESERVED: &[&str] = &[
    "initialize",
    "ping",
    "shutdown",
    "tools_list",
    "tools_call",
    "resources_list",
    "prompts_list",
    "rpc",
    "rpc.internal",
];

#[derive(Debug, Clone)]
pub struct NameSource<'a> {
    pub operation_id: Option<&'a str>,
    pub summary: Option<&'a str>,
    pub description: Option<&'a str>,
    pub method: &'a str,
    pub path_template: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedTool {
    pub name: String,
    pub source_kind: NameKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    OperationId,
    Summary,
    Description,
    Fallback,
}

pub fn candidate_name(src: &NameSource<'_>) -> NamedTool {
    if let Some(id) = src.operation_id.map(str::trim).filter(|s| !s.is_empty()) {
        let name = normalize_raw(id, true);
        if !name.is_empty() {
            return NamedTool {
                name: finish_reserved(name),
                source_kind: NameKind::OperationId,
            };
        }
    }
    if let Some(summary) = src.summary.map(str::trim).filter(|s| !s.is_empty()) {
        let name = normalize_raw(summary, false);
        if !name.is_empty() {
            return NamedTool {
                name: finish_reserved(name),
                source_kind: NameKind::Summary,
            };
        }
    }
    if let Some(desc) = src.description.map(str::trim).filter(|s| !s.is_empty()) {
        let first = desc.split(['.', '\n']).next().unwrap_or(desc);
        let clipped: String = first.chars().take(80).collect();
        let name = normalize_raw(&clipped, false);
        if !name.is_empty() {
            return NamedTool {
                name: finish_reserved(name),
                source_kind: NameKind::Description,
            };
        }
    }
    NamedTool {
        name: normalize(&method_path_fallback(src.method, src.path_template), false),
        source_kind: NameKind::Fallback,
    }
}

pub fn method_path_fallback(method: &str, path: &str) -> String {
    let mut s = method.to_ascii_lowercase();
    s.push('_');
    let stripped = path.trim_start_matches('/');
    for ch in stripped.chars() {
        match ch {
            '{' | '}' | '/' => s.push('_'),
            other => s.push(other),
        }
    }
    s
}

pub fn normalize(source: &str, is_operation_id: bool) -> String {
    finish_reserved(normalize_raw(source, is_operation_id))
}

fn normalize_raw(source: &str, is_operation_id: bool) -> String {
    let mut s = source.to_owned();
    if is_operation_id {
        s = s.replace(['/', '.'], "_");
    }
    s = camel_to_snake(&s);
    let mut out = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }
    collapse_separators(&mut out);
    trim_separators(&mut out);
    if out.len() > 64 {
        out = truncate_64(&out);
    }
    out
}

fn finish_reserved(mut out: String) -> String {
    if out.is_empty()
        || RESERVED.contains(&out.as_str())
        || out.starts_with("mcp.")
        || out.starts_with("mcp_")
    {
        out = if out.is_empty() {
            "api_op".into()
        } else {
            format!("api_{out}")
        };
        if out.len() > 128 {
            out.truncate(128);
            trim_separators(&mut out);
        }
    }
    out
}

fn camel_to_snake(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                let prev = chars[i - 1];
                if prev.is_ascii_lowercase() || prev.is_ascii_digit() {
                    out.push('_');
                }
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii() {
            out.push(ch);
        }
    }
    out
}

fn collapse_separators(s: &mut String) {
    let mut out = String::new();
    let mut last_sep = false;
    for ch in s.chars() {
        let sep = ch == '_' || ch == '-' || ch == '.';
        if sep {
            if !last_sep {
                out.push('_');
            }
            last_sep = true;
        } else {
            last_sep = false;
            out.push(ch);
        }
    }
    *s = out;
}

fn trim_separators(s: &mut String) {
    let trimmed = s.trim_matches(|c: char| c == '_' || c == '-' || c == '.');
    *s = trimmed.to_owned();
}

fn truncate_64(s: &str) -> String {
    let mut out: String = s.chars().take(64).collect();
    trim_separators(&mut out);
    out
}

/// Assign unique names. Returns names in the same order as `candidates`.
pub fn uniquify(candidates: &[String], keys: &[(String, String, String)]) -> Vec<String> {
    assert_eq!(candidates.len(), keys.len());
    let mut groups: std::collections::BTreeMap<String, Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, name) in candidates.iter().enumerate() {
        groups.entry(name.clone()).or_default().push(i);
    }
    let mut result = candidates.to_vec();
    let mut taken: std::collections::BTreeSet<String> = candidates.iter().cloned().collect();
    for (name, idxs) in groups {
        if idxs.len() == 1 {
            continue;
        }
        let mut ordered = idxs;
        ordered.sort_by(|&a, &b| {
            (&keys[a].0, &keys[a].1, &keys[a].2).cmp(&(&keys[b].0, &keys[b].1, &keys[b].2))
        });
        for (n, &idx) in ordered.iter().enumerate().skip(1) {
            let mut suffix = n as u32 + 1;
            loop {
                let mut candidate = format!("{name}_{suffix}");
                if candidate.len() > 128 {
                    candidate.truncate(128);
                    trim_separators(&mut candidate);
                }
                if !taken.contains(&candidate) {
                    taken.insert(candidate.clone());
                    result[idx] = candidate;
                    break;
                }
                suffix += 1;
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_repos_get() {
        let n = candidate_name(&NameSource {
            operation_id: Some("repos/get"),
            summary: Some("Get a repository"),
            description: None,
            method: "GET",
            path_template: "/repos/{owner}/{repo}",
        });
        assert_eq!(n.name, "repos_get");
        assert_eq!(n.source_kind, NameKind::OperationId);
    }

    #[test]
    fn stripe_camel_case() {
        let n = normalize("getCharge", true);
        assert_eq!(n, "get_charge");
    }

    #[test]
    fn reserved_prefixed() {
        assert_eq!(normalize("ping", true), "api_ping");
        assert_eq!(normalize("mcp.foo", false), "api_mcp_foo");
    }

    #[test]
    fn truncate_64() {
        let long = "a".repeat(80);
        let n = normalize(&long, false);
        assert!(n.len() <= 64);
    }

    #[test]
    fn fallback_path() {
        let n = candidate_name(&NameSource {
            operation_id: None,
            summary: None,
            description: None,
            method: "GET",
            path_template: "/repos/{owner}/{repo}",
        });
        assert_eq!(n.name, "get_repos_owner_repo");
        assert_eq!(n.source_kind, NameKind::Fallback);
    }

    #[test]
    fn collision_suffix() {
        let names = vec!["foo".into(), "foo".into(), "bar".into()];
        let keys = vec![
            ("POST".into(), "/a".into(), "z".into()),
            ("GET".into(), "/a".into(), "a".into()),
            ("GET".into(), "/b".into(), "".into()),
        ];
        let out = uniquify(&names, &keys);
        assert_eq!(out[1], "foo");
        assert_eq!(out[0], "foo_2");
        assert_eq!(out[2], "bar");
    }

    #[test]
    fn empty_operation_id_uses_method_path_fallback() {
        let n = candidate_name(&NameSource {
            operation_id: Some("!!!"),
            summary: None,
            description: None,
            method: "GET",
            path_template: "/repos/{owner}/{repo}",
        });
        assert_eq!(n.name, "get_repos_owner_repo");
        assert_eq!(n.source_kind, NameKind::Fallback);
    }
}
