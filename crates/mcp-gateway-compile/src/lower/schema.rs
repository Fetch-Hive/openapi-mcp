use serde_json::{json, Map, Value};

/// Flatten `allOf` of object schemas. Returns true if an `allOf` remains.
pub fn flatten_allof(schema: &mut Value) -> bool {
    let Value::Object(map) = schema else {
        return false;
    };
    let Some(Value::Array(branches)) = map.get("allOf").cloned() else {
        return false;
    };
    let mut props = Map::new();
    let mut required: Vec<String> = Vec::new();
    for branch in &branches {
        let Some(obj) = branch.as_object() else {
            return true;
        };
        if obj.contains_key("$ref") {
            return true;
        }
        if let Some(ty) = obj.get("type").and_then(Value::as_str) {
            if ty != "object" {
                return true;
            }
        }
        if let Some(p) = obj.get("properties").and_then(Value::as_object) {
            for (k, v) in p {
                if let Some(existing) = props.get(k) {
                    if existing != v {
                        return true;
                    }
                } else {
                    props.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(req) = obj.get("required").and_then(Value::as_array) {
            for r in req {
                if let Some(s) = r.as_str() {
                    if !required.iter().any(|x| x == s) {
                        required.push(s.to_owned());
                    }
                }
            }
        }
    }
    map.remove("allOf");
    if !props.is_empty() {
        match map.get_mut("properties") {
            Some(Value::Object(existing)) => {
                for (k, v) in props {
                    existing.entry(k).or_insert(v);
                }
            }
            _ => {
                map.insert("properties".into(), Value::Object(props));
            }
        }
    }
    if !required.is_empty() {
        match map.get_mut("required") {
            Some(Value::Array(existing)) => {
                for r in required {
                    if !existing.iter().any(|v| v.as_str() == Some(r.as_str())) {
                        existing.push(json!(r));
                    }
                }
            }
            _ => {
                map.insert("required".into(), json!(required));
            }
        }
    }
    if !map.contains_key("type") && map.contains_key("properties") {
        map.insert("type".into(), json!("object"));
    }
    false
}

/// Put hoisted component schemas back on a schema document.
///
/// `hoist_defs` lifts `$defs` into a side map so input schemas can share
/// them. A response schema that still says `#/$defs/Pet` has to carry those
/// definitions itself, or output validation cannot resolve the ref.
pub fn graft_defs(schema: &mut Value, defs: &Map<String, Value>) {
    if defs.is_empty() {
        return;
    }
    let Some(map) = schema.as_object_mut() else {
        return;
    };
    match map.get_mut("$defs") {
        Some(Value::Object(existing)) => {
            for (key, value) in defs {
                existing.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
        _ => {
            map.insert("$defs".into(), Value::Object(defs.clone()));
        }
    }
}

pub fn hoist_defs(schema: &mut Value, sink: &mut Map<String, Value>) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::Object(defs)) = map.remove("$defs") {
                for (k, mut v) in defs {
                    hoist_defs(&mut v, sink);
                    sink.entry(k).or_insert(v);
                }
            }
            if let Some(Value::Object(defs)) = map.remove("definitions") {
                for (k, mut v) in defs {
                    hoist_defs(&mut v, sink);
                    sink.entry(k).or_insert(v);
                }
            }
            for v in map.values_mut() {
                hoist_defs(v, sink);
            }
        }
        Value::Array(items) => {
            for v in items {
                hoist_defs(v, sink);
            }
        }
        _ => {}
    }
}
