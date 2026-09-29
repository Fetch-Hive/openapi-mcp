# SSRF policy

Default outbound policy for the runtime proxy and for spec fetches
(`add-spec --url`, `serve --url`):

- HTTPS only, except the HTTP case in the table below
- Ports 80, 443, and 8443. `--allow-private-networks` turns that allow-list
  off (`allow_loopback` is set with the same flag), so a local spec on port
  8000 can be fetched
- Public resolvers (1.1.1.1 and 8.8.8.8), not `/etc/resolv.conf`, unless
  `--allow-private-networks` is set, in which case the system resolver is used
- Deny RFC1918, loopback, ULA, link-local, cloud metadata, NAT64 unwrap
- Hostname denylist includes `localhost`, `.internal`, `metadata.google.internal`

## Spec document URL

| Request | Result |
|---|---|
| `https://` public host | allowed |
| `https://` loopback, RFC1918, or ULA without `--allow-private-networks` | refused, exit 2 |
| `http://` public host, with or without flags | refused, exit 2 |
| `http://` loopback, RFC1918, or ULA, missing either flag | refused, exit 2. The error names the flag that is missing and does not repeat a flag that is already set |
| `http://` loopback, RFC1918, or ULA, both flags set | allowed. The GET uses HTTP, does not follow redirects, ignores `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY`, caps the body at 10 MiB, and times out after 15 seconds |
| Cloud metadata (`169.254.169.254`, `fd00:ec2::254`, link-local, `metadata.google.internal`) | refused, exit 2, with or without the flags |

`add-spec` spells the HTTP opt-in `--insecure-http`. `serve` spells it
`--allow-insecure-http`. Each flag is OR'd with `ssrf.allow_insecure_http`
in config. `--allow-private-networks` is OR'd with
`ssrf.allow_private_networks`. A name that is not a literal is resolved, and
every address must be loopback, RFC1918, or ULA. One public address refuses
the URL. A failed lookup refuses it.

An absolute `http://` `$ref` inside the document stays unresolved. Relative
and in-document `$ref`s are bundled. A remote `$ref` must be `https://` or
`file:`.

`mcp-gateway doctor` runs a self-test matrix of `https://` loopback, RFC1918,
and metadata literals. A pin that should have been denied exits `2`. With
`--allow-private-networks`, loopback and RFC1918 are expected to pin (so a
local API on `127.0.0.1:3000` can be the upstream); metadata stays denied.
