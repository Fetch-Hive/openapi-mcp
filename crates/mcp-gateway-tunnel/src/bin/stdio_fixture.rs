//! Test double for the stdio bridge. Not an MCP server.
//!
//! One JSON object per line on stdin. `initialize` counts how many times it
//! was seen and returns that count in `result.inits`. `tools/list` returns
//! the same count plus `lines` (how many method-bearing lines have been read)
//! and `child_id` (the id on this request). `exit` terminates with status 1
//! and no response.
//!
//! `--request` writes one `sampling/createMessage` after the first initialize
//! response and records whether stdin later carries JSON-RPC `-32601` for id
//! `child-1`. A `tools/list` that arrives before that error is answered after
//! the error, so the flag is stable.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn main() {
    let want_request = std::env::args().any(|arg| arg == "--request");
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut inits = 0u64;
    let mut seen = 0u64;
    let mut rejected = false;
    let mut request_sent = false;
    let mut deferred: Option<Value> = None;

    while let Some(Ok(line)) = lines.next() {
        if line.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if value.get("method").and_then(Value::as_str) == Some("notifications/initialized") {
            seen += 1;
            continue;
        }
        if value.get("error").is_some()
            && value.get("id").and_then(Value::as_str) == Some("child-1")
        {
            rejected = value["error"]["code"] == -32601;
            if let Some(message) = deferred.take() {
                answer_tools(&message, inits, seen, rejected);
            }
            continue;
        }
        let Some(method) = value.get("method").and_then(Value::as_str) else {
            continue;
        };
        seen += 1;
        if method == "exit" {
            std::process::exit(1);
        }
        if method == "initialize" {
            inits += 1;
            write_message(&json!({
                "jsonrpc": "2.0",
                "id": value.get("id").cloned().unwrap_or(Value::Null),
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": { "name": "fixture", "version": "0" },
                    "inits": inits,
                    "lines": seen
                }
            }));
            if want_request && !request_sent {
                request_sent = true;
                write_message(&json!({
                    "jsonrpc": "2.0",
                    "id": "child-1",
                    "method": "sampling/createMessage",
                    "params": {}
                }));
            }
            continue;
        }
        if method == "tools/list" && want_request && !rejected {
            deferred = Some(value);
            continue;
        }
        if method == "tools/list" {
            answer_tools(&value, inits, seen, rejected);
            continue;
        }
        if method == "hang" {
            continue;
        }
        write_message(&json!({
            "jsonrpc": "2.0",
            "id": value.get("id").cloned().unwrap_or(Value::Null),
            "result": {
                "ok": true,
                "inits": inits,
                "lines": seen,
                "child_id": value.get("id").cloned().unwrap_or(Value::Null)
            }
        }));
    }
}

fn answer_tools(value: &Value, inits: u64, seen: u64, rejected: bool) {
    write_message(&json!({
        "jsonrpc": "2.0",
        "id": value.get("id").cloned().unwrap_or(Value::Null),
        "result": {
            "tools": [{ "name": "echo" }],
            "inits": inits,
            "lines": seen,
            "child_id": value.get("id").cloned().unwrap_or(Value::Null),
            "child_rejected": rejected,
            "gateway_token_present": std::env::var_os("MCP_GATEWAY_TOKEN").is_some()
        }
    }));
}

fn write_message(value: &Value) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}
