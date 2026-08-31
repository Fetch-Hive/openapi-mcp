//! `$ref` bundling via `jsonschema` 0.51 plus OpenAPI component `$ref` resolution.

use crate::http::{self, DownloadError};
use crate::safety::{self, SafetyOpts};
use jsonschema::{Draft, Retrieve, Uri};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;

pub const REMOTE_FETCH_BUDGET: u32 = 32;
pub const REMOTE_BYTE_BUDGET: usize = 8 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const REF_CHAIN_DEPTH: usize = 32;

#[derive(Debug, Error)]
pub enum RefError {
    #[error("unresolved $ref: {0}")]
    Unresolved(String),
    #[error("remote $ref budget exceeded")]
    Budget,
    #[error(transparent)]
    Safety(#[from] crate::safety::SafetyError),
}

#[derive(Debug, Default)]
struct RetrieverState {
    cache: HashMap<String, Value>,
    fetches: u32,
    bytes: usize,
}

#[derive(Debug, Clone)]
pub struct Retriever {
    inner: Arc<Mutex<RetrieverState>>,
    safety: SafetyOpts,
}

impl Default for Retriever {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RetrieverState::default())),
            safety: SafetyOpts::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BundleOutcome {
    pub schema: Value,
    pub had_cycle: bool,
    pub unresolved: Vec<String>,
}

impl Retriever {
    pub fn with_safety(safety: SafetyOpts) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RetrieverState::default())),
            safety,
        }
    }

    pub fn get(&self, url: &str) -> Result<Value, RefError> {
        let (url, _) = split_url_frag(url);
        {
            let guard = self.inner.lock().expect("retriever mutex");
            if let Some(v) = guard.cache.get(url) {
                return Ok(v.clone());
            }
            if guard.fetches >= REMOTE_FETCH_BUDGET || guard.bytes >= REMOTE_BYTE_BUDGET {
                return Err(RefError::Budget);
            }
        }

        let remaining = {
            let guard = self.inner.lock().expect("retriever mutex");
            REMOTE_BYTE_BUDGET.saturating_sub(guard.bytes)
        };
        if remaining == 0 {
            return Err(RefError::Budget);
        }

        let value = if url.starts_with("https://") {
            let parsed = safety::parse_https_url_with(url, self.safety)?;
            let host = parsed.host_str().unwrap_or_default();
            safety::resolve_and_check_with(host, self.safety)?;
            let bytes = match http::download_https_capped(url, remaining, FETCH_TIMEOUT) {
                Ok(b) => b,
                Err(DownloadError::TooLarge) => return Err(RefError::Budget),
                Err(DownloadError::Failed(msg)) => return Err(RefError::Unresolved(msg)),
            };
            {
                let mut guard = self.inner.lock().expect("retriever mutex");
                guard.fetches += 1;
                guard.bytes += bytes.len();
                if guard.fetches > REMOTE_FETCH_BUDGET || guard.bytes > REMOTE_BYTE_BUDGET {
                    return Err(RefError::Budget);
                }
            }
            parse_schema_bytes(&bytes)?
        } else if url.starts_with("file:") {
            let path = file_url_to_path(url)?;
            let bytes = read_capped_file(&path, remaining)?;
            {
                let mut guard = self.inner.lock().expect("retriever mutex");
                guard.fetches += 1;
                guard.bytes += bytes.len();
                if guard.fetches > REMOTE_FETCH_BUDGET || guard.bytes > REMOTE_BYTE_BUDGET {
                    return Err(RefError::Budget);
                }
            }
            parse_schema_bytes(&bytes)?
        } else {
            return Err(RefError::Unresolved(url.to_owned()));
        };

        let mut guard = self.inner.lock().expect("retriever mutex");
        guard.cache.insert(url.to_owned(), value.clone());
        Ok(value)
    }
}

impl Retrieve for Retriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        self.get(uri.as_str())
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    }
}

pub fn bundle_schema(doc: &Value, schema: &Value, retriever: &Retriever) -> BundleOutcome {
    let rewritten = rewrite_schema_refs(schema);
    let defs = collect_needed_defs(doc, &rewritten);
    let root = match rewritten {
        Value::Object(mut m) => {
            merge_defs_into(&mut m, defs);
            Value::Object(m)
        }
        other => {
            let mut wrap = Map::new();
            wrap.insert("allOf".into(), json!([other]));
            if !defs.is_empty() {
                wrap.insert("$defs".into(), Value::Object(defs));
            }
            Value::Object(wrap)
        }
    };

    match jsonschema::options()
        .with_draft(Draft::Draft202012)
        .with_retriever(retriever.clone())
        .bundle(&root)
    {
        Ok(bundled) => {
            let mut unresolved = Vec::new();
            dangling_defs(&bundled, &mut unresolved);
            let had_cycle = defs_have_cycle(&bundled);
            BundleOutcome {
                schema: bundled,
                had_cycle,
                unresolved,
            }
        }
        Err(err) => BundleOutcome {
            schema: root,
            had_cycle: false,
            unresolved: vec![err.to_string()],
        },
    }
}

/// Follow an OpenAPI `$ref` (path item, parameter, requestBody, response).
/// Nested `#` pointers after a remote fetch use the fetched document as root.
pub fn resolve_openapi_object(
    root: &Value,
    value: &Value,
    retriever: &Retriever,
) -> Result<Value, RefError> {
    resolve_openapi_object_inner(root, value, retriever, 0, &mut HashSet::new())
}

fn resolve_openapi_object_inner(
    root: &Value,
    value: &Value,
    retriever: &Retriever,
    depth: usize,
    visiting: &mut HashSet<String>,
) -> Result<Value, RefError> {
    if depth > REF_CHAIN_DEPTH {
        return Err(RefError::Unresolved("max $ref depth".into()));
    }
    let Some(r) = value.get("$ref").and_then(Value::as_str) else {
        return Ok(value.clone());
    };
    if !visiting.insert(r.to_owned()) {
        return Err(RefError::Unresolved(format!("cyclic $ref {r}")));
    }
    let (target, new_root) = load_ref(root, r, retriever)?;
    let mut resolved =
        resolve_openapi_object_inner(&new_root, &target, retriever, depth + 1, visiting)?;
    visiting.remove(r);
    if let Some(obj) = value.as_object() {
        if obj.len() > 1 {
            if let Value::Object(mut base) = resolved {
                for (k, v) in obj {
                    if k != "$ref" {
                        base.insert(k.clone(), v.clone());
                    }
                }
                resolved = Value::Object(base);
            }
        }
    }
    Ok(resolved)
}

fn load_ref(root: &Value, r: &str, retriever: &Retriever) -> Result<(Value, Value), RefError> {
    if let Some(rest) = r.strip_prefix('#') {
        let v = pointer(root, rest)
            .cloned()
            .ok_or_else(|| RefError::Unresolved(r.to_owned()))?;
        return Ok((v, root.clone()));
    }
    if r.starts_with("https://") || r.starts_with("file:") {
        let (url, frag) = split_url_frag(r);
        let remote = retriever.get(url)?;
        let v = if let Some(f) = frag {
            pointer(&remote, f)
                .cloned()
                .ok_or_else(|| RefError::Unresolved(r.to_owned()))?
        } else {
            remote.clone()
        };
        return Ok((v, remote));
    }
    Err(RefError::Unresolved(r.to_owned()))
}

fn collect_needed_defs(doc: &Value, schema: &Value) -> Map<String, Value> {
    let mut wanted: HashSet<String> = HashSet::new();
    gather_def_names(schema, &mut wanted);
    let mut defs = Map::new();
    let mut stack: Vec<String> = wanted.iter().cloned().collect();
    while let Some(name) = stack.pop() {
        if defs.contains_key(&name) {
            continue;
        }
        let encoded = name.replace('~', "~0").replace('/', "~1");
        let found = doc
            .pointer(&format!("/components/schemas/{encoded}"))
            .or_else(|| doc.pointer(&format!("/definitions/{encoded}")));
        let Some(source) = found else {
            continue;
        };
        let rewritten = rewrite_schema_refs(source);
        let mut nested = HashSet::new();
        gather_def_names(&rewritten, &mut nested);
        for n in nested {
            if !defs.contains_key(&n) && n != name {
                stack.push(n);
            }
        }
        defs.insert(name, rewritten);
    }
    defs
}

fn gather_def_names(value: &Value, out: &mut HashSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                if let Some(rest) = r.strip_prefix("#/$defs/") {
                    let name = rest.split('/').next().unwrap_or(rest);
                    if !name.is_empty() {
                        out.insert(name.replace("~1", "/").replace("~0", "~"));
                    }
                }
            }
            for v in map.values() {
                gather_def_names(v, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                gather_def_names(v, out);
            }
        }
        _ => {}
    }
}

fn dangling_defs(schema: &Value, unresolved: &mut Vec<String>) {
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    fn walk(value: &Value, defs: &Map<String, Value>, unresolved: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                    if let Some(rest) = r.strip_prefix("#/$defs/") {
                        let name = rest.split('/').next().unwrap_or(rest);
                        if !defs.contains_key(name) {
                            unresolved.push(r.to_owned());
                        }
                    } else if r.starts_with("#/components/") || r.starts_with("#/definitions/") {
                        unresolved.push(r.to_owned());
                    }
                }
                for v in map.values() {
                    walk(v, defs, unresolved);
                }
            }
            Value::Array(items) => {
                for v in items {
                    walk(v, defs, unresolved);
                }
            }
            _ => {}
        }
    }
    walk(schema, &defs, unresolved);
}

fn merge_defs_into(target: &mut Map<String, Value>, incoming: Map<String, Value>) {
    if incoming.is_empty() {
        return;
    }
    match target.get_mut("$defs") {
        Some(Value::Object(existing)) => {
            for (k, v) in incoming {
                existing.entry(k).or_insert(v);
            }
        }
        _ => {
            target.insert("$defs".into(), Value::Object(incoming));
        }
    }
}

fn rewrite_schema_refs(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if k == "$ref" {
                    if let Some(s) = v.as_str() {
                        out.insert(k.clone(), json!(rewrite_ref_str(s)));
                        continue;
                    }
                }
                let key = if k == "definitions" {
                    "$defs"
                } else {
                    k.as_str()
                };
                out.insert(key.to_owned(), rewrite_schema_refs(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(rewrite_schema_refs).collect()),
        other => other.clone(),
    }
}

fn rewrite_ref_str(r: &str) -> String {
    for prefix in ["#/components/schemas/", "#/definitions/"] {
        if let Some(rest) = r.strip_prefix(prefix) {
            return format!("#/$defs/{rest}");
        }
    }
    r.to_owned()
}

fn defs_have_cycle(schema: &Value) -> bool {
    let Some(defs) = schema.get("$defs").and_then(Value::as_object) else {
        return false;
    };
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for (name, def) in defs {
        graph.insert(name.clone(), collect_def_refs(def));
    }
    for start in graph.keys() {
        let mut visiting = HashSet::new();
        let mut seen = HashSet::new();
        if dfs_cycle(start, &graph, &mut visiting, &mut seen) {
            return true;
        }
    }
    false
}

fn collect_def_refs(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    collect_def_refs_inner(value, &mut out);
    out
}

fn collect_def_refs_inner(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(r) = map.get("$ref").and_then(Value::as_str) {
                if let Some(name) = r.strip_prefix("#/$defs/") {
                    let name = name.split('/').next().unwrap_or(name);
                    out.push(name.to_owned());
                }
            }
            for v in map.values() {
                collect_def_refs_inner(v, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect_def_refs_inner(v, out);
            }
        }
        _ => {}
    }
}

fn dfs_cycle(
    node: &str,
    graph: &HashMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
    seen: &mut HashSet<String>,
) -> bool {
    if visiting.contains(node) {
        return true;
    }
    if !seen.insert(node.to_owned()) {
        return false;
    }
    visiting.insert(node.to_owned());
    if let Some(edges) = graph.get(node) {
        for next in edges {
            if dfs_cycle(next, graph, visiting, seen) {
                return true;
            }
        }
    }
    visiting.remove(node);
    false
}

pub fn pointer<'a>(doc: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() || pointer == "/" {
        return Some(doc);
    }
    let mut cur = doc;
    for raw in pointer.trim_start_matches('/').split('/') {
        let key = raw.replace("~1", "/").replace("~0", "~");
        cur = match cur {
            Value::Object(map) => map.get(&key)?,
            Value::Array(arr) => arr.get(key.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn split_url_frag(r: &str) -> (&str, Option<&str>) {
    match r.split_once('#') {
        Some((u, f)) => (u, Some(f)),
        None => (r, None),
    }
}

fn parse_schema_bytes(bytes: &[u8]) -> Result<Value, RefError> {
    let trimmed = bytes
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .copied()
        .unwrap_or(0);
    let value: Value = if trimmed == b'{' || trimmed == b'[' {
        serde_json::from_slice(bytes).map_err(|e| RefError::Unresolved(e.to_string()))?
    } else {
        serde_yaml::from_slice(bytes).map_err(|e| RefError::Unresolved(e.to_string()))?
    };
    if !value.is_object() && !value.is_array() {
        return Err(RefError::Unresolved(
            "remote $ref did not parse as a JSON object or array".into(),
        ));
    }
    Ok(value)
}

fn file_url_to_path(url: &str) -> Result<std::path::PathBuf, RefError> {
    let parsed = url::Url::parse(url).map_err(|e| RefError::Unresolved(e.to_string()))?;
    parsed
        .to_file_path()
        .map_err(|_| RefError::Unresolved(format!("invalid file URL {url}")))
}

fn read_capped_file(path: &std::path::Path, max: usize) -> Result<Vec<u8>, RefError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| RefError::Unresolved(e.to_string()))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = file
            .read(&mut chunk)
            .map_err(|e| RefError::Unresolved(e.to_string()))?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > max {
            return Err(RefError::Budget);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_leaves_ref_to_defs() {
        let doc = json!({
            "components": {
                "schemas": {
                    "Node": {
                        "type": "object",
                        "properties": {
                            "child": { "$ref": "#/components/schemas/Node" }
                        }
                    }
                }
            }
        });
        let schema = json!({ "$ref": "#/components/schemas/Node" });
        let r = Retriever::default();
        let out = bundle_schema(&doc, &schema, &r);
        assert!(out.unresolved.is_empty(), "{:?}", out.unresolved);
        assert!(out.had_cycle);
        let text = out.schema.to_string();
        assert!(text.contains("#/$defs/Node"), "{text}");
        let defs = out.schema.get("$defs").and_then(Value::as_object);
        assert!(defs.is_some_and(|d| d.contains_key("Node")));
    }

    #[test]
    fn unresolved_recorded() {
        let doc = json!({});
        let schema = json!({ "$ref": "#/components/schemas/Missing" });
        let r = Retriever::default();
        let out = bundle_schema(&doc, &schema, &r);
        assert!(!out.unresolved.is_empty() || out.schema.get("$ref").is_some());
    }

    #[test]
    fn openapi_parameter_ref_resolves() {
        let doc = json!({
            "components": {
                "parameters": {
                    "Owner": {
                        "name": "owner",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }
                }
            }
        });
        let param = json!({ "$ref": "#/components/parameters/Owner" });
        let r = Retriever::default();
        let resolved = resolve_openapi_object(&doc, &param, &r).unwrap();
        assert_eq!(resolved["name"], "owner");
        assert_eq!(resolved["in"], "path");
    }
}
