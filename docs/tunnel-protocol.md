# MCP Gateway tunnel protocol

Version 1 of the wire protocol between `mcp-gateway` and a tunnel relay.
The types live in the public crate `mcp-gateway-tunnel-proto`. A Fetch Hive
relay hosts the production endpoints below. The same crate is enough to run
a self-hosted relay; this repository does not ship one.

The open-source CLI speaks this protocol. It does not send telemetry.
Anonymous tunnels need no Fetch Hive account. Login, named hostnames, and
the account database are hosted features and are not required to implement
version 1.

## Overview and threat model

A developer runs an MCP server on loopback. The CLI opens one outbound
WebSocket to the relay. Remote MCP clients call
`https://<slug>.mcp.fetchhive.com/mcp`. The relay multiplexes those HTTP
requests over the WebSocket. The developer's machine accepts no inbound
connections.

The relay forwards `Authorization` and does not store it. The local server
checks `MCP_GATEWAY_TOKEN`. The relay can answer `401` when the endpoint is
in `token` mode and the request has no `Authorization` header, without
reading the credential.

The reclaim secret is 32 random bytes, returned once in `Welcome` as
base64url without padding. The relay stores the hex SHA-256 of those 32
bytes (it decodes the base64url form first) and compares it in constant
time. The plaintext is not written to disk or Redis.

The relay is MCP-only. It forwards `POST`, `GET`, and `DELETE` on `/mcp`.
It answers `/health` itself. It does not open TCP, SSH, or arbitrary HTTP
paths.

What is open source: this document, the frame types, slug rules, and the
CLI that dials the relay. What is Fetch Hive hosted: the production relay
at `connect.mcp.fetchhive.com`, the `*.mcp.fetchhive.com` certificates, and
(later) named-endpoint ownership. Point the CLI at another relay with
`MCP_GATEWAY_RELAY_URL`.

## Endpoints

| Role | URL |
| --- | --- |
| CLI → relay | `wss://connect.mcp.fetchhive.com/v1/tunnel` |
| Remote MCP client → relay | `https://<slug>.mcp.fetchhive.com/mcp` |
| Liveness, answered by the relay | `https://<slug>.mcp.fetchhive.com/health` |

The WebSocket is text frames only in version 1. Each message is one JSON
object. Version 1 denies unknown fields. A new field requires
`PROTOCOL_VERSION` to increase. A future version may switch to binary
frames; peers that send any other version receive `Rejected` with
`version_unsupported`.

## Handshake

The first client message is `Hello`. The relay answers `Welcome` or
`Rejected`, then closes on rejection. Later messages are [`Frame`](#frames)
values. `t` discriminates the handshake the same way it discriminates frames.

`Hello.version` must be `1`. `Hello.client` is
`mcp-gateway/<semver> (<target>)`. `Hello.mcp_path` is the path the local
server serves (usually `/mcp`). The public path is always `/mcp`; the CLI
maps between them. `Hello.mode` is `anonymous` or `named`. `Hello.auth_mode`
is `token` (default) or `public`. `Hello.reclaim` resumes a slug after a
disconnect.

Anonymous `Hello` with no reclaim:

```json
{
  "t": "hello",
  "version": 1,
  "client": "mcp-gateway/0.7.0 (darwin-arm64)",
  "mode": {
    "type": "anonymous"
  },
  "auth_mode": "token",
  "mcp_path": "/mcp"
}
```

`Welcome.credential` is base64url without padding (43 characters).
`Welcome.public_url` is absolute. Display that URL. Do not rebuild it from
`PUBLIC_SUFFIX`, so staging and self-hosted relays keep their own hostnames.
`max_session_secs` is present for anonymous sessions and omitted for named
ones. `lease_grace_secs` is how long the slug survives a disconnect.

```json
{
  "t": "welcome",
  "slug": "x8kj32ab",
  "public_url": "https://x8kj32ab.mcp.fetchhive.com/mcp",
  "credential": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
  "lease_grace_secs": 1800,
  "max_session_secs": 28800,
  "limits": {
    "max_body_bytes": 1048576,
    "max_inflight": 16,
    "request_timeout_secs": 60,
    "rpm": 60
  },
  "kind": {
    "type": "anonymous"
  }
}
```

`Rejected` is the last message. The relay then closes with
`RejectCode.close_code`.

```json
{
  "t": "rejected",
  "code": "version_unsupported",
  "message": "protocol version 99 is not supported",
  "retry_after_secs": 30
}
```

The relay must see `Hello` within `HELLO_TIMEOUT_SECS`. `mode: named` on a
relay that has no account authorizer returns `unauthorized`.

## Frames

After the handshake, `t` is the snake_case variant name. `id` is a `u64`
the relay allocates per connection, starting at 1. Ordering is guaranteed
only within one `id`. Many ids are in flight at once, up to `max_inflight`.

| `t` | Direction | Meaning |
| --- | --- | --- |
| `request_start` | relay → CLI | HTTP request on the public `/mcp` |
| `request_body` | relay → CLI | Next base64 chunk of that request |
| `response_start` | CLI → relay | HTTP status and headers |
| `response_body` | CLI → relay | Next base64 chunk of that response |
| `cancel` | either | Stop the in-flight id |
| `ping` | either | Heartbeat nonce |
| `pong` | either | Echo of that nonce |
| `stats` | CLI → relay | Optional inflight / total counters |
| `go_away` | relay → CLI | Drain. Reconnect after `reconnect_after_secs` |

Header values are JSON arrays of `[name, value]` pairs, original casing
kept, duplicates allowed. Bodies are standard base64 with padding
(RFC 4648 §4). The relay never logs `Authorization`, `Cookie`, or bodies.

`request_start` for a small JSON-RPC call. The inline `body` is the whole
entity because `body_complete` is true.

```json
{
  "t": "request_start",
  "id": 1,
  "method": "POST",
  "path": "/mcp",
  "headers": [
    [
      "authorization",
      "Bearer local-token"
    ],
    [
      "content-type",
      "application/json"
    ],
    [
      "X-Forwarded-For",
      "203.0.113.4"
    ],
    [
      "X-Forwarded-Proto",
      "https"
    ],
    [
      "X-Forwarded-Host",
      "x8kj32ab.mcp.fetchhive.com"
    ],
    [
      "X-Tunnel-Request-Id",
      "6b9f1c2e-4d3a-4f1b-9c8e-0a1b2c3d4e5f"
    ]
  ],
  "body_complete": true,
  "body": "eyJqc29ucnBjIjoiMi4wIiwiaWQiOjEsIm1ldGhvZCI6InRvb2xzL2xpc3QifQ==",
  "client_ip": "203.0.113.4",
  "request_id": "6b9f1c2e-4d3a-4f1b-9c8e-0a1b2c3d4e5f"
}
```

A further request chunk. `query` is omitted when the public URL has no
query string. It is the raw query without `?`.

```json
{
  "t": "request_body",
  "id": 1,
  "chunk": "eyJqc29ucnBjIjoiMi4wIn0=",
  "last": true
}
```

SSE and other streamed responses set `body_complete` false and then send
`response_body` until `last` is true. `Content-Type: text/event-stream` is
forwarded as the bytes arrive.

```json
{
  "t": "response_start",
  "id": 1,
  "status": 200,
  "headers": [
    [
      "content-type",
      "text/event-stream"
    ]
  ],
  "body_complete": false
}
```

```json
{
  "t": "response_body",
  "id": 1,
  "chunk": "ZGF0YTogb2sKCg==",
  "last": true
}
```

```json
{
  "t": "cancel",
  "id": 1,
  "reason": "timeout"
}
```

```json
{
  "t": "ping",
  "nonce": 7
}
```

```json
{
  "t": "pong",
  "nonce": 7
}
```

```json
{
  "t": "stats",
  "inflight": 1,
  "total": 4
}
```

```json
{
  "t": "go_away",
  "reason": "deploy",
  "reconnect_after_secs": 0
}
```

`cancel.reason` is `timeout`, `client_gone`, `client`, or `relay`.
`timeout` means the relay hit `request_timeout_secs`; the CLI drops the
in-flight entry. `client_gone` means the public HTTP client disconnected.
`reconnect_after_secs` of `0` means reconnect immediately and reclaim.

## Streaming

`body_complete: true` plus an inline `body` is the fast path for a body
that fits in one frame. Omit `body` when the entity is empty. Otherwise
send `request_body` or `response_body` chunks with `last: true` on the
final chunk. The first chunk may also be inlined on `*_start` with
`body_complete: false`; following chunks are `*_body` frames.

A `GET` stays open until `response_body` with `last: true` or until either
side sends `cancel`. One base64 `body` or `chunk` is at most
`MAX_FRAME_BYTES` (256 KiB). One WebSocket text message is at most
`MAX_WS_MESSAGE_BYTES` (512 KiB).

## Headers

The relay copies request headers except hop-by-hop names (`Connection`,
`Keep-Alive`, `Transfer-Encoding`, `Upgrade`, `TE`, `Trailer`, and
`Proxy-*`) and `Cookie`. `is_hop_by_hop_header` matches the hop-by-hop
set. `is_stripped_request_header` is that set plus `Cookie`. Comparison
is ASCII case-insensitive.

It adds:

- `X-Forwarded-For`
- `X-Forwarded-Proto` with value `https`
- `X-Forwarded-Host` set to `<slug>.mcp.fetchhive.com`
- `X-Tunnel-Request-Id` set to the same id as `request_id`

It does not rewrite `Host`. The CLI rewrites `Host` to its bind authority
before the local router sees the request, and records the original public
host from `X-Forwarded-Host`.

## Limits

Defaults the hosted relay advertises in `Welcome.limits`. A self-hosted
relay may send different numbers. The CLI honours the `Welcome` values for
that connection.

Anonymous leases survive a disconnect for `DEFAULT_ANON_LEASE_GRACE_SECS`
(30 minutes) and then expire. Named leases are held until released; while
the socket is down the public URL returns `503`. An anonymous socket is
recycled after `DEFAULT_ANON_MAX_SESSION_SECS` (8 hours) with `go_away`
and `reconnect_after_secs: 0`. The client reclaims, so the hostname stays.

The relay pings every `HEARTBEAT_INTERVAL_SECS`. The peer answers with
`pong` within `HEARTBEAT_TIMEOUT_SECS` or the socket is dropped. The CLI
may ping as well.

## Reconnect and reclaim

On disconnect the CLI reconnects and sends `Hello` with
`reclaim: { slug, credential }` using the credential from `Welcome`. The
relay hashes the credential, compares it in constant time to the stored
digest, and resumes the slug when the lease is still held. A live socket
for that slug is closed with `1012` (`CLOSE_SERVICE_RESTART`) and replaced.
`reclaim_expired` means the grace period elapsed. `reclaim_invalid` means
the digest did not match.

The open-source client (`mcp-gateway-tunnel`) waits
`HELLO_TIMEOUT_SECS + 5` (10 seconds) for `Welcome` or `Rejected`. The
relay's own deadline for seeing `Hello` is `HELLO_TIMEOUT_SECS` (5 seconds).
A connect failure, a timeout, or a message that is not `Welcome` or
`Rejected` is treated as a failed dial.

Retry delay when the session did not name one:

| Attempt after the failure | Base delay |
| --- | --- |
| 0 | 1s |
| 1 | 2s |
| 2 | 4s |
| 3 | 8s |
| 4 | 16s |
| 5 | 32s |
| 6 and later | 60s |

The base is multiplied by a uniform random factor in `[0.8, 1.2)`, with a
floor of 50ms. `go_away` is different: the CLI finishes in-flight calls,
then waits exactly `reconnect_after_secs` with no jitter, and reclaims.
The 8 hour anonymous recycle sends `reconnect_after_secs: 0`.

| `Rejected.code` | CLI |
| --- | --- |
| `reclaim_expired`, `reclaim_invalid` | Drop the secret. Next `Hello` is a fresh anonymous tunnel, so the slug changes. Then the backoff above. |
| `rate_limited` | Wait `retry_after_secs`, or 1 second when the field is absent. The backoff counter resets to 0. |
| `maintenance` | Wait 1 second and dial again. `retry_after_secs` is not used for this code. |
| `unauthorized`, `plan_limit`, `version_unsupported`, `name_taken`, `name_invalid`, `name_reserved`, and any other code | Stop. The process exits. |

`mcp-gateway` maps those terminal codes to exit status: `2` for
`unauthorized` and `plan_limit`, `1` for `version_unsupported` and the
`name_*` codes, `4` for every other terminal code. Ctrl-C closes the
WebSocket with `1000` and the process exits `130`.

When `Welcome.limits.max_inflight` calls are already running, the CLI
answers that request itself with HTTP `429`, `Retry-After: 1`, and

```json
{"jsonrpc":"2.0","error":{"code":-32000,"message":"too many in-flight requests"},"id":null}
```

The local MCP server is not called.

While the lease exists and no socket is attached, public requests receive
`503` with `Retry-After: 10` and this body:

```json
{"jsonrpc":"2.0","error":{"code":-32001,"message":"MCP endpoint offline"},"id":null}
```

## Slug rules

Anonymous slugs are 8 characters from `abcdefghjkmnpqrstuvwxyz23456789`
(no `0`, `o`, `1`, `l`, `i`). Each character is chosen independently, so a
slug can be all letters. They are never permanently reserved, and anonymous
allocation does not consult `RESERVED_SLUGS`. `internal` is 8 letters in
that alphabet, so it can be issued. `connect` is 7 characters, so it cannot.

A relay must not give one live slug to two sessions. The production relay
writes `mcp_tunnel:lease:<slug>` with Redis `SET key NX EX`. The value is
the lease record, and the reclaim field in it is the hex SHA-256 of the
32 raw bytes. If the
key already exists, it draws again, up to 8 times, then sends `Rejected`
with `code: maintenance`, `message: could not allocate a tunnel name`, and
`retry_after_secs: 5`. The CLI still waits 1 second for `maintenance`, as
in the table above.

The public hostname is one label, lowercased, in front of the public
suffix. `connect.<suffix>` is the WebSocket host. The apex
(`mcp.fetchhive.com` on the hosted suffix) is not a tenant. A name with an
extra dot (`a.b.mcp.fetchhive.com`) is not a tenant. `Host: AbCdEfGh.…` and
`Host: abcdefgh.…` are the same lease.

The production relay also caps new anonymous tunnels at 10 per hour per
client IP (`MCP_TUNNEL_ANON_CREATE_PER_HOUR`). The counter is
`mcp_tunnel:create:<sha256(ip)>:<unix_hour>`. Over the cap the relay sends
`rate_limited` with `message: too many anonymous tunnels from this network`
and `retry_after_secs: 3600`. A draw that fails after the counter
increments returns that slot.

Named slugs match `^[a-z0-9]([a-z0-9-]{1,46}[a-z0-9])$` (3 to 48
characters), are not in the reserved list, and are not anonymous-shaped
(exactly 8 characters, all in the anonymous alphabet). `validate_named_slug`
and `is_anonymous_shaped` implement this.

Reserved, exact and lowercase: `admin`, `api`, `app`, `auth`, `connect`,
`health`, `internal`, `login`, `mcp`, `null`, `relay`, `signup`, `staging`,
`status`, `undefined`, `www`.

## Status and close codes

Public HTTP mapping on the tenant host:

| Condition | Status |
| --- | --- |
| Body is not a JSON-RPC 2.0 object or array | `400` |
| `auth_mode` is `token` and `Authorization` is missing | `401`, `WWW-Authenticate: Bearer`, body `{"jsonrpc":"2.0","error":{"code":-32000,"message":"missing authorization"},"id":null}`. No OAuth discovery URL. |
| No lease for the slug | `404` |
| Method is not `POST`, `GET`, or `DELETE` | `405` |
| Body larger than `max_body_bytes` | `413` |
| `POST` `Content-Type` is not `application/json` | `415` |
| In-flight cap or per-endpoint rate limit | `429` |
| Socket closed while the request was in flight | `502` |
| Lease exists, CLI disconnected | `503` |
| `request_timeout_secs` elapsed | `504` |

`/health` on a tenant host is `200` from the relay and is not forwarded.
The body is `{"slug":"<slug>","online":true}` while a socket is attached
and `{"slug":"<slug>","online":false}` while the lease exists and the CLI
is gone. Any other path is `404`.

WebSocket close codes:

| Code | When |
| --- | --- |
| `1000` | Normal close |
| `1008` | Policy rejection (`version_unsupported`, `name_taken`, `name_invalid`, `name_reserved`, `plan_limit`, `reclaim_expired`) |
| `1012` | This socket was replaced by a reclaim |
| `1013` | `go_away`, or `Rejected` with `maintenance` |
| `4001` | `unauthorized` or `reclaim_invalid` |
| `4029` | `rate_limited` |

## Versioning policy

`PROTOCOL_VERSION` is `1`. Receivers reject every other version with
`version_unsupported` and close `1008`. Within a version the JSON schema
is closed: unknown fields and unknown `t` values fail deserialization.
Adding a field or a frame is a version bump. Optional omissions that are
already specified (`reclaim`, `query`, `body`, `max_session_secs`,
`retry_after_secs`) stay valid.

## Self-hosting a relay

Set `MCP_GATEWAY_RELAY_URL` on the CLI to the relay's WebSocket URL. The
hosted default is `wss://connect.mcp.fetchhive.com/v1/tunnel`. The relay
that mints public URLs reads `MCP_TUNNEL_PUBLIC_SUFFIX` (hosted value
`mcp.fetchhive.com`) and returns the absolute URL in `Welcome`. Fetch Hive
does not publish a relay binary in this repository. Implement one against
`mcp-gateway-tunnel-proto` and this document.

## Constants

These lines are the crate's public constants. Tests fail when this list
drifts from the code.

```text
PROTOCOL_VERSION = 1
DEFAULT_RELAY_URL = wss://connect.mcp.fetchhive.com/v1/tunnel
PUBLIC_SUFFIX = mcp.fetchhive.com
CONNECT_HOST = connect.mcp.fetchhive.com
RELAY_URL_ENV = MCP_GATEWAY_RELAY_URL
PUBLIC_SUFFIX_ENV = MCP_TUNNEL_PUBLIC_SUFFIX
MCP_PATH = /mcp
HEALTH_PATH = /health
ANONYMOUS_SLUG_LEN = 8
ANONYMOUS_ALPHABET = abcdefghjkmnpqrstuvwxyz23456789
RESERVED_SLUGS = admin api app auth connect health internal login mcp null relay signup staging status undefined www
NAMED_SLUG_MIN_LEN = 3
NAMED_SLUG_MAX_LEN = 48
RECLAIM_CREDENTIAL_BYTES = 32
RECLAIM_CREDENTIAL_LEN = 43
DEFAULT_MAX_BODY_BYTES = 1048576
DEFAULT_MAX_INFLIGHT = 16
DEFAULT_REQUEST_TIMEOUT_SECS = 60
DEFAULT_RPM = 60
DEFAULT_ANON_LEASE_GRACE_SECS = 1800
DEFAULT_ANON_MAX_SESSION_SECS = 28800
MAX_FRAME_BYTES = 262144
MAX_WS_MESSAGE_BYTES = 524288
HEARTBEAT_INTERVAL_SECS = 20
HEARTBEAT_TIMEOUT_SECS = 10
HELLO_TIMEOUT_SECS = 5
OFFLINE_RETRY_AFTER_SECS = 10
JSONRPC_ENDPOINT_OFFLINE_CODE = -32001
OFFLINE_JSONRPC_BODY = {"jsonrpc":"2.0","error":{"code":-32001,"message":"MCP endpoint offline"},"id":null}
HEADER_FORWARDED_FOR = X-Forwarded-For
HEADER_FORWARDED_PROTO = X-Forwarded-Proto
HEADER_FORWARDED_HOST = X-Forwarded-Host
HEADER_TUNNEL_REQUEST_ID = X-Tunnel-Request-Id
FORWARDED_PROTO_HTTPS = https
CONTENT_TYPE_JSON = application/json
CONTENT_TYPE_EVENT_STREAM = text/event-stream
CLOSE_NORMAL = 1000
CLOSE_POLICY = 1008
CLOSE_SERVICE_RESTART = 1012
CLOSE_TRY_AGAIN_LATER = 1013
CLOSE_AUTH = 4001
CLOSE_RATE_LIMITED = 4029
STATUS_NOT_JSONRPC = 400
STATUS_MISSING_AUTHORIZATION = 401
STATUS_UNKNOWN_SLUG = 404
STATUS_METHOD_NOT_ALLOWED = 405
STATUS_BODY_TOO_LARGE = 413
STATUS_UNSUPPORTED_MEDIA_TYPE = 415
STATUS_TOO_MANY_INFLIGHT = 429
STATUS_TUNNEL_CLOSED = 502
STATUS_OFFLINE = 503
STATUS_TIMEOUT = 504
```
