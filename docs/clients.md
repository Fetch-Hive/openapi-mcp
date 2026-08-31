# Client snippets

`mcp-gateway inspect NAME --client cursor|claude|vscode|chatgpt` prints a
paste-ready fragment.

**Re-verify against live vendor docs before each release.** Cursor currently
needs `"type": "http"` (not `streamable-http`) for CLI. Claude Desktop uses
stdio `command`/`args`. VS Code Copilot MCP uses `servers`. ChatGPT custom
connectors take a URL plus a bearer header. This phase does not implement
OAuth.
