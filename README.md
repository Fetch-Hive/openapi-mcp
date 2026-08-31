# mcp-gateway

Turn an OpenAPI document into a local MCP server.

`mcp-gateway` compiles OpenAPI 3.0/3.1 to a versioned IR, serves tools over
Streamable HTTP or stdio, and injects upstream credentials from env or file
references. Outbound HTTP uses an SSRF-hardened dialer.

Prefer a hosted MCP Gateway with tokens, quotas, and a dashboard?
https://fetchhive.com/mcp

## Install

Order: Homebrew → Docker → npx → curl|sh → cargo.

```bash
# Homebrew (after Fetch-Hive/homebrew-tap exists)
brew install Fetch-Hive/tap/mcp-gateway

# Docker
docker run --rm -p 127.0.0.1:8787:8787 ghcr.io/fetch-hive/mcp-gateway:0.1.0 version

# npx (optionalDependencies, no postinstall)
npx --yes @fetch-hive/mcp-gateway version

# curl | sh (after the first GitHub Release)
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/Fetch-Hive/openapi-mcp/releases/latest/download/mcp-gateway-installer.sh | sh

# From source
cargo install --path crates/mcp-gateway-cli
```

## Five-minute quickstart

```bash
mcp-gateway init
mcp-gateway add-spec --name demo --file ./openapi.yaml
# or: mcp-gateway add-spec --name demo --url https://petstore3.swagger.io/api/v3/openapi.json
export MCP_GATEWAY_TOKEN=…   # printed once by init
mcp-gateway serve demo
mcp-gateway doctor
```

Point Cursor at `http://127.0.0.1:8787/mcp` with the bearer token, or:

```bash
mcp-gateway inspect demo --client cursor
mcp-gateway serve demo --stdio
```

## Security

Default bind is loopback. Binding `0.0.0.0` requires `--expose`. Upstream
private networks require `--allow-private-networks` (loud; never enabled in
Fetch Hive Cloud). Report vulnerabilities to security@fetchhive.com — see
`SECURITY.md`.

## Docs

- [CLI reference](docs/cli.md)
- [Config schema](docs/config.md)
- [SSRF policy](docs/ssrf.md)
- [Private-network flag](docs/private-networks.md)
- [Client snippets](docs/clients.md)

## Licence

Apache License 2.0. See `LICENSE` and `NOTICE`.
