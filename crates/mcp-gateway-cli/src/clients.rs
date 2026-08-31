//! Client paste snippets. Re-verify against live vendor docs before each release.
//!
//! Cursor: https://cursor.com/docs/mcp (use `"type": "http"` until streamable-http
//! is accepted by Cursor CLI).
//! Claude: https://code.claude.com/docs/en/mcp
//! VS Code: https://code.visualstudio.com/docs/copilot/customization/mcp-servers
//! ChatGPT: custom MCP connectors accept a URL + bearer header.

use crate::cli::ClientKind;
use serde_json::{json, Value};

pub fn snippet(kind: ClientKind, name: &str, bind: &str, path: &str) -> Value {
    let url = format!("http://{bind}{path}");
    match kind {
        ClientKind::Cursor => json!({
            "mcpServers": {
                name: {
                    "type": "http",
                    "url": url,
                    "headers": {
                        "Authorization": "Bearer ${env:MCP_GATEWAY_TOKEN}"
                    }
                }
            }
        }),
        ClientKind::Claude => json!({
            "mcpServers": {
                name: {
                    "command": "mcp-gateway",
                    "args": ["serve", name, "--stdio"]
                }
            }
        }),
        ClientKind::Vscode => json!({
            "servers": {
                name: {
                    "type": "http",
                    "url": url,
                    "headers": {
                        "Authorization": "Bearer ${env:MCP_GATEWAY_TOKEN}"
                    }
                }
            }
        }),
        ClientKind::Chatgpt => json!({
            "name": name,
            "url": url,
            "authorization": "Bearer $MCP_GATEWAY_TOKEN"
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_uses_http_type() {
        let v = snippet(ClientKind::Cursor, "petstore", "127.0.0.1:8787", "/mcp");
        assert_eq!(v["mcpServers"]["petstore"]["type"], "http");
        assert_eq!(
            v["mcpServers"]["petstore"]["url"],
            "http://127.0.0.1:8787/mcp"
        );
    }

    #[test]
    fn claude_uses_stdio_command() {
        let v = snippet(ClientKind::Claude, "petstore", "127.0.0.1:8787", "/mcp");
        assert_eq!(v["mcpServers"]["petstore"]["command"], "mcp-gateway");
    }
}
