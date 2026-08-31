# SSRF policy

Default outbound policy:

- HTTPS only (HTTP requires `--allow-insecure-http`)
- Ports 80, 443, 8443
- Public resolvers (1.1.1.1 and 8.8.8.8), not `/etc/resolv.conf`
- Deny RFC1918, loopback, ULA, link-local, cloud metadata, NAT64 unwrap
- Hostname denylist includes `localhost`, `.internal`, `metadata.google.internal`

`mcp-gateway doctor` runs a self-test matrix of `https://` loopback, RFC1918,
and metadata literals. A pin that should have been denied exits `2`. With
`--allow-private-networks`, loopback and RFC1918 are expected to pin (so a
local API on `127.0.0.1:3000` can be the upstream); metadata stays denied.
