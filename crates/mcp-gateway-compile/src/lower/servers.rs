use mcp_gateway_ir::*;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn effective_servers(doc: &Value, path_item: &Value, op: &Value) -> Vec<Server> {
    let list = op
        .get("servers")
        .or_else(|| path_item.get("servers"))
        .or_else(|| doc.get("servers"))
        .and_then(Value::as_array);
    list.map(|arr| arr.iter().filter_map(parse_server).collect())
        .unwrap_or_default()
}

pub fn parse_server(v: &Value) -> Option<Server> {
    let url = v.get("url")?.as_str()?.to_owned();
    let mut variables = BTreeMap::new();
    if let Some(vars) = v.get("variables").and_then(Value::as_object) {
        for (k, val) in vars {
            variables.insert(
                k.clone(),
                ServerVariable {
                    default: val
                        .get("default")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    enum_values: val.get("enum").and_then(Value::as_array).map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    }),
                },
            );
        }
    }
    Some(Server {
        url_template: url,
        variables,
        description: v
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}
