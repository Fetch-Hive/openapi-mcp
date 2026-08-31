//! Paste-ready MCP client snippets.
//!
//! Cursor: https://cursor.com/docs/mcp (`"type": "http"`).
//! Claude Code: https://code.claude.com/docs/en/mcp
//! Claude Desktop: stdio `command` / `args`.
//! Codex: https://developers.openai.com/codex/mcp (`url` + `bearer_token_env_var`).
//! VS Code: https://code.visualstudio.com/docs/copilot/customization/mcp-servers
//! ChatGPT: custom MCP connectors (HTTPS URL + bearer).

use crate::cli::ClientKind;
use serde_json::{json, Value};

pub struct ClientSnippet {
    pub client: &'static str,
    pub paste_into: &'static str,
    pub format: &'static str,
    pub body: String,
}

pub fn render(kind: ClientKind, name: &str, bind: &str, path: &str) -> ClientSnippet {
    let url = format!("http://{bind}{path}");
    match kind {
        ClientKind::Cursor => ClientSnippet {
            client: "cursor",
            paste_into: ".cursor/mcp.json (project) or Cursor Settings → MCP",
            format: "json",
            body: pretty(cursor_json(name, &url)),
        },
        ClientKind::ClaudeCode => ClientSnippet {
            client: "claude-code",
            paste_into: ".mcp.json (project) or `claude mcp add --transport http`",
            format: "json",
            body: pretty(claude_code_json(name, &url)),
        },
        ClientKind::Claude => ClientSnippet {
            client: "claude",
            paste_into: "Claude Desktop mcpServers (stdio)",
            format: "json",
            body: pretty(json!({
                "mcpServers": {
                    name: {
                        "command": "mcp-gateway",
                        "args": ["serve", name, "--stdio"]
                    }
                }
            })),
        },
        ClientKind::Codex => ClientSnippet {
            client: "codex",
            paste_into: "~/.codex/config.toml or project .codex/config.toml",
            format: "toml",
            body: format!(
                "[mcp_servers.{name}]\nurl = \"{url}\"\nbearer_token_env_var = \"MCP_GATEWAY_TOKEN\"\n"
            ),
        },
        ClientKind::Vscode => ClientSnippet {
            client: "vscode",
            paste_into: ".vscode/mcp.json",
            format: "json",
            body: pretty(json!({
                "servers": {
                    name: {
                        "type": "http",
                        "url": url,
                        "headers": {
                            "Authorization": "Bearer ${env:MCP_GATEWAY_TOKEN}"
                        }
                    }
                }
            })),
        },
        ClientKind::Chatgpt => ClientSnippet {
            client: "chatgpt",
            paste_into: "ChatGPT custom connector (HTTPS URL in production)",
            format: "json",
            body: pretty(json!({
                "name": name,
                "url": url,
                "authorization": "Bearer $MCP_GATEWAY_TOKEN"
            })),
        },
    }
}

pub fn json_value(snippet: &ClientSnippet) -> Value {
    let parsed = if snippet.format == "json" {
        serde_json::from_str(&snippet.body).unwrap_or(Value::String(snippet.body.clone()))
    } else {
        Value::String(snippet.body.clone())
    };
    json!({
        "client": snippet.client,
        "paste_into": snippet.paste_into,
        "format": snippet.format,
        "snippet": parsed,
    })
}

fn cursor_json(name: &str, url: &str) -> Value {
    json!({
        "mcpServers": {
            name: {
                "type": "http",
                "url": url,
                "headers": {
                    "Authorization": "Bearer ${env:MCP_GATEWAY_TOKEN}"
                }
            }
        }
    })
}

fn claude_code_json(name: &str, url: &str) -> Value {
    json!({
        "mcpServers": {
            name: {
                "type": "http",
                "url": url,
                "headers": {
                    "Authorization": "Bearer ${MCP_GATEWAY_TOKEN}"
                }
            }
        }
    })
}

fn pretty(v: Value) -> String {
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_uses_http_type() {
        let s = render(ClientKind::Cursor, "petstore", "127.0.0.1:8787", "/mcp");
        let v: Value = serde_json::from_str(&s.body).unwrap();
        assert_eq!(v["mcpServers"]["petstore"]["type"], "http");
        assert_eq!(
            v["mcpServers"]["petstore"]["url"],
            "http://127.0.0.1:8787/mcp"
        );
        assert!(s.paste_into.contains(".cursor/mcp.json"));
    }

    #[test]
    fn claude_uses_stdio_command() {
        let s = render(ClientKind::Claude, "petstore", "127.0.0.1:8787", "/mcp");
        let v: Value = serde_json::from_str(&s.body).unwrap();
        assert_eq!(v["mcpServers"]["petstore"]["command"], "mcp-gateway");
    }

    #[test]
    fn claude_code_uses_http() {
        let s = render(ClientKind::ClaudeCode, "petstore", "127.0.0.1:8787", "/mcp");
        let v: Value = serde_json::from_str(&s.body).unwrap();
        assert_eq!(v["mcpServers"]["petstore"]["type"], "http");
    }

    #[test]
    fn codex_is_toml_with_bearer_env() {
        let s = render(ClientKind::Codex, "petstore", "127.0.0.1:8787", "/mcp");
        assert_eq!(s.format, "toml");
        assert!(s.body.contains("[mcp_servers.petstore]"));
        assert!(s.body.contains("http://127.0.0.1:8787/mcp"));
        assert!(s.body.contains("bearer_token_env_var"));
    }
}
