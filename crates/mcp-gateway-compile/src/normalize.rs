use serde_json::{json, Value};

/// Rewrite a 3.0 document tree into 3.1-shaped JSON before IR lowering.
pub fn normalize_3_0(value: &mut Value) {
    walk(value);
}

fn walk(value: &mut Value) {
    match value {
        Value::Object(map) => {
            rewrite_schema_object(map);
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if let Some(child) = map.get_mut(&key) {
                    walk(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item);
            }
        }
        _ => {}
    }
}

fn rewrite_schema_object(map: &mut serde_json::Map<String, Value>) {
    if let Some(Value::Bool(true)) = map.get("nullable") {
        if map.contains_key("$ref") {
            let refer = map.clone();
            map.clear();
            map.insert(
                "anyOf".into(),
                json!([refer_without_nullable(&refer), { "type": "null" }]),
            );
            return;
        }
        match map.get("type").cloned() {
            Some(Value::String(t)) => {
                map.insert("type".into(), json!([t, "null"]));
            }
            Some(Value::Array(mut arr)) => {
                if !arr.iter().any(|v| v.as_str() == Some("null")) {
                    arr.push(json!("null"));
                }
                map.insert("type".into(), Value::Array(arr));
            }
            _ => {
                let mut inner = map.clone();
                inner.remove("nullable");
                map.clear();
                map.insert(
                    "anyOf".into(),
                    json!([Value::Object(inner), { "type": "null" }]),
                );
                return;
            }
        }
    }
    map.remove("nullable");

    rewrite_exclusive(map, "exclusiveMinimum", "minimum");
    rewrite_exclusive(map, "exclusiveMaximum", "maximum");

    if let Some(example) = map.remove("example") {
        if !map.contains_key("examples") {
            map.insert("examples".into(), json!([example]));
        }
    }
}

fn refer_without_nullable(map: &serde_json::Map<String, Value>) -> Value {
    let mut out = map.clone();
    out.remove("nullable");
    Value::Object(out)
}

fn rewrite_exclusive(
    map: &mut serde_json::Map<String, Value>,
    exclusive_key: &str,
    bound_key: &str,
) {
    let Some(ex) = map.get(exclusive_key).cloned() else {
        return;
    };
    if let Value::Bool(true) = ex {
        if let Some(bound) = map.remove(bound_key) {
            map.insert(exclusive_key.to_owned(), bound);
        } else {
            map.remove(exclusive_key);
        }
    } else if let Value::Bool(false) = ex {
        map.remove(exclusive_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nullable_type_becomes_type_array() {
        let mut v = json!({"type": "string", "nullable": true});
        normalize_3_0(&mut v);
        assert_eq!(v["type"], json!(["string", "null"]));
        assert!(v.get("nullable").is_none());
    }

    #[test]
    fn nullable_ref_wraps_any_of() {
        let mut v = json!({"$ref": "#/components/schemas/Foo", "nullable": true});
        normalize_3_0(&mut v);
        assert_eq!(
            v,
            json!({
                "anyOf": [
                    {"$ref": "#/components/schemas/Foo"},
                    {"type": "null"}
                ]
            })
        );
    }

    #[test]
    fn exclusive_minimum_true() {
        let mut v = json!({"minimum": 1, "exclusiveMinimum": true});
        normalize_3_0(&mut v);
        assert_eq!(v["exclusiveMinimum"], json!(1));
        assert!(v.get("minimum").is_none());
    }

    #[test]
    fn exclusive_minimum_false() {
        let mut v = json!({"minimum": 1, "exclusiveMinimum": false});
        normalize_3_0(&mut v);
        assert_eq!(v["minimum"], json!(1));
        assert!(v.get("exclusiveMinimum").is_none());
    }

    #[test]
    fn example_becomes_examples() {
        let mut v = json!({"type": "string", "example": "hi"});
        normalize_3_0(&mut v);
        assert_eq!(v["examples"], json!(["hi"]));
        assert!(v.get("example").is_none());
    }

    #[test]
    fn existing_examples_kept() {
        let mut v = json!({"type": "string", "example": "a", "examples": ["b"]});
        normalize_3_0(&mut v);
        assert_eq!(v["examples"], json!(["b"]));
        assert!(v.get("example").is_none());
    }

    #[test]
    fn nullable_allof_without_type_is_wrapped() {
        let mut v = json!({
            "allOf": [{ "type": "object" }],
            "nullable": true
        });
        normalize_3_0(&mut v);
        assert_eq!(v["anyOf"][1], json!({"type": "null"}));
        assert!(v.get("nullable").is_none());
    }
}
