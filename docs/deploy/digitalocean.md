# DigitalOcean App Platform

A Deploy to DigitalOcean button is supported. App Platform **does not attach
volumes**; local disk is ephemeral (recompile from `MCP_GATEWAY_SPEC_URL` on
each deploy). Streamable HTTP / SSE may hit edge request timeouts (~100s);
DigitalOcean documents WebSockets as the realtime path. If your MCP client
holds a long stream, prefer [Render](render.md) or [Hetzner](hetzner.md).

## One-click

[![Deploy to DigitalOcean](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/apps/new?repo=https://github.com/Fetch-Hive/openapi-mcp/tree/main)

Uses [`.do/deploy.template.yaml`](../../.do/deploy.template.yaml) (`spec:`
wrapper). Public GitHub/GitLab only. The image is **amd64**.

Set `MCP_GATEWAY_TOKEN` and `MCP_GATEWAY_SPEC_URL` in the App Platform env UI.
`http_port` is 8080; App Platform sets `PORT` to match. Start command is
`serve demo` (no shell interpolation).

Bind loopback (`127.0.0.1`) will fail health checks — `PORT` handling in
`mcp-gateway serve` binds `0.0.0.0`.

## After deploy

Point Cursor, Codex, or Claude Code at `https://<app>.ondigitalocean.app/mcp`
with the bearer token ([clients](../clients.md)).
