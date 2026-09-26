# Security Policy

## Reporting a vulnerability

Email **tom@fetchhive.com**. Do not open a public GitHub issue for
security reports.

We aim to acknowledge reports within 5 business days and to ship a fix or
mitigation within 90 days of a confirmed vulnerability, following coordinated
disclosure.

## Scope

This repository is the open-source MCP Gateway CLI and libraries
(`mcp-gateway-ir`, `mcp-gateway-compile`, `mcp-gateway-proxy`,
`mcp-gateway-server`, `mcp-gateway-cli`, `mcp-gateway-tunnel`,
`mcp-gateway-tunnel-proto`).

Reports about the production tunnel relay (`*.mcp.fetchhive.com`,
`connect.mcp.fetchhive.com`) are in scope for the same address. The relay
is hosted; the client and the protocol crate are in this repository.

Please include:

- Affected version / commit
- Reproduction steps
- Impact (SSRF, credential leak, auth bypass, RCE)

SSRF bypasses against the default public-internet policy are in scope.
`--allow-private-networks` is an explicit self-host opt-in; reports that it
can reach RFC1918 when enabled are not vulnerabilities.
