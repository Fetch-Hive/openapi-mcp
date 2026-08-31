# SSRF policy

Default outbound policy (same as Fetch Hive Cloud):

- HTTPS only (HTTP requires `--allow-insecure-http`)
- Ports 80, 443, 8443
- Public resolvers (1.1.1.1 and 8.8.8.8), not `/etc/resolv.conf`
- Deny RFC1918, loopback, ULA, link-local, cloud metadata, NAT64 unwrap
- Hostname denylist includes `localhost`, `.internal`, `metadata.google.internal`

`mcp-gateway doctor` runs a self-test matrix of `https://` loopback, RFC1918,
and metadata literals. A pin that should have been denied exits `2`. With
`--allow-private-networks`, RFC1918 literals are expected to pin; metadata
stays denied.
