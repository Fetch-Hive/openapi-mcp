use serde_json::{Map, Value};

pub fn media_type_base(ct: &str) -> String {
    ct.split(';')
        .next()
        .unwrap_or(ct)
        .trim()
        .to_ascii_lowercase()
}

pub fn is_json_media_type(ct: &str) -> bool {
    let base = media_type_base(ct);
    base == "application/json" || base.ends_with("+json")
}

pub fn select_json_content(content: &Map<String, Value>) -> Option<(String, &Value)> {
    content
        .iter()
        .find(|(ct, _)| is_json_media_type(ct))
        .or_else(|| {
            content
                .iter()
                .find(|(ct, _)| media_type_base(ct) == "application/x-www-form-urlencoded")
        })
        .map(|(ct, v)| (ct.clone(), v))
}

pub fn is_streaming(op: &Value) -> bool {
    if object_has_streaming_content(op.get("requestBody")) {
        return true;
    }
    if let Some(responses) = op.get("responses").and_then(Value::as_object) {
        if responses
            .values()
            .any(|r| object_has_streaming_content(Some(r)))
        {
            return true;
        }
    }
    false
}

fn object_has_streaming_content(obj: Option<&Value>) -> bool {
    let Some(obj) = obj else {
        return false;
    };
    let Some(content) = obj.get("content").and_then(Value::as_object) else {
        return false;
    };
    content.keys().any(|ct| {
        let base = media_type_base(ct);
        base == "text/event-stream"
            || base == "application/json-seq"
            || base == "application/websocket"
    })
}

pub fn is_binary_body(op: &Value) -> bool {
    let Some(content) = op
        .get("requestBody")
        .and_then(|b| b.get("content"))
        .and_then(Value::as_object)
    else {
        return false;
    };
    content.keys().any(|ct| {
        let base = media_type_base(ct);
        base.starts_with("multipart/")
            || base == "application/octet-stream"
            || base.starts_with("image/")
    })
}
