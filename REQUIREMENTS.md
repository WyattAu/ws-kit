# Requirements — ws-kit

Each requirement is tagged in code (`REQ-WSKIT-*` doc-comment markers) and
mapped to verifying tests. Security requirements are closed only when the
cited test exists and passes in CI. (REQ-WSKIT-001 — bearer parse
char-boundary safety — is cited historically in THREAT-MODEL.md.)

## REQ-WSKIT-200 — Origin validation on the WebSocket upgrade (CSWSH)

When `WsConfig::allowed_origins` is **non-empty**, a WebSocket upgrade
request whose `Origin` header is **missing** or **not in the allow-list**
must be rejected with **403 Forbidden before `on_upgrade`** — the socket is
never upgraded.

- Check entry: `origin_allowed_in_parts(&Parts, &allowed_origins)`
  (axum feature), decision fn `origin_allowed(Option<&str>, &[String])`.
- Comparison: **exact** match on scheme + host + port after normalization
  (`normalize_origin`: lowercase scheme/host, default ports `http`/`ws` 80
  and `https`/`wss` 443 omitted — `https://example.com:443` ==
  `https://example.com`).
- **No wildcard, suffix, prefix, or subdomain matching in v1** —
  `https://example.com` does not accept `https://evil-example.com`,
  `https://example.com.evil.com`, or `https://api.example.com`.
- Non-default ports are significant: `https://example.com:8443` != `https://example.com`.
- Bracketed IPv6 hosts normalize (`https://[2001:DB8::1]:443` ==
  `https://[2001:db8::1]`).

Tagged at: `src/origin.rs::origin_allowed`, `src/origin.rs::origin_allowed_in_parts`.

### Verifying tests

| Case | Test |
|------|------|
| allowed origin → 101 upgrade | `tests/integration.rs::origin_upgrade::req_wskit_200_allowed_origin_passes_upgrade` |
| disallowed origin → 403 pre-upgrade | `tests/integration.rs::origin_upgrade::req_wskit_200_disallowed_origin_rejected_403_pre_upgrade` |
| missing Origin (non-empty list) → 403 | `tests/integration.rs::origin_upgrade::req_wskit_200_missing_origin_rejected_403` |
| default-port normalization matches (`:443`) | `tests/integration.rs::origin_upgrade::req_wskit_200_port_normalized_match_passes`; unit `src/origin.rs::normalize_origin_default_ports_omitted` |
| no wildcard/suffix/subdomain match | `tests/integration.rs::origin_upgrade::req_wskit_200_no_wildcard_suffix_match`; unit `src/origin.rs::origin_allowed_no_wildcard_or_suffix_matching` |
| scheme + host case-insensitive; entries normalized identically | unit `src/origin.rs::origin_allowed_match_and_normalization` |
| scheme is significant (`http` != `https`) | unit `src/origin.rs::origin_allowed_scheme_matters` |
| Host header never stands in for Origin | unit `src/origin.rs::origin_allowed_in_parts_host_header_ignored` |
| malformed origin (no scheme) fails closed | unit `src/origin.rs::normalize_origin_no_scheme_fail_closed` |
| builder surface sets/replaces list | unit `src/config.rs::req_wskit_200_builder_sets_allowed_origins` |

## REQ-WSKIT-201 — Default empty allow-list = allow all (documented residual)

`WsConfig::default()` has `allowed_origins: []`, which **allows every
origin** — including upgrades with no Origin header — preserving pre-0.3.0
behavior so existing users do not break. This is a **documented, accepted
residual risk** (CSWSH remains possible until the integrator sets the list;
see THREAT-MODEL.md OPEN→residual note). The `WsConfig::allowed_origins`
field and builder docs carry a loud warning.

Tagged at: `src/config.rs::WsConfig.allowed_origins`.

### Verifying tests

| Case | Test |
|------|------|
| default config allows any origin and missing Origin → 101 | `tests/integration.rs::origin_upgrade::req_wskit_201_default_config_allows_all_origins` |
| default list is empty | unit `src/config.rs::req_wskit_201_default_allowed_origins_empty_allows_all`; decision fn `src/origin.rs::origin_allowed_empty_list_allows_all` |

## REQ-WSKIT-300 — Binary frames are opaque (never parsed as text/JSON)

`Frame::Binary` payloads must be carried verbatim by routing (hub), room
broadcast, and the Redis adapter. No ws-kit code path may parse a binary
payload as text or JSON; converting binary to text requires an explicit
API call (`Frame::into_text`), which **errors** on `Frame::Binary` instead
of lossy-coercing. Text is guaranteed UTF-8 end to end (`Frame::Text` can
only be constructed from a `String`).

Tagged at: `src/codec.rs::Frame`, `src/codec.rs::Frame::into_text`.

### Verifying tests

| Case | Test |
|------|------|
| binary routing through the hub, byte-exact, both receivers | `tests/integration.rs::binary_frame_routes_through_hub` |
| binary routing through a room; text/binary interleave in order | `tests/integration.rs::binary_frame_routes_through_room` |
| `into_text` on binary errors (`InvalidMessage`), never coerces | unit `src/codec.rs::frame_into_bytes_and_text`; `tests/integration.rs::frame_into_text_rejects_binary` |
| Frame kind constructors/lengths | unit `src/codec.rs::frame_constructors_and_kinds`, `frame_len_and_bytes` |
| binary across the Redis hop byte-exact | live fixture `tests/redis_rooms.rs::live_binary_frame_crosses_nodes`; mocked `src/redis_rooms.rs::remote_binary_envelope_roundtrips` |

## REQ-WSKIT-400 — Compression is bounded and fails closed (0.4.0 `compression`)

The `compression` feature's `FrameCompressor` must:

1. **Cap decompression**: decompressed output may never exceed
   `CompressionConfig::max_size` (default 1 MiB), checked *during*
   inflation (`Read::take`) so bombs do not materialize in memory; identity
   payloads over the cap are equally rejected → `WsError::PayloadTooLarge`.
2. **Fail closed on envelopes**: unknown flag bits, corrupt deflate
   streams, empty envelopes, and a `Text` tag over non-UTF-8 bytes must all
   error (`WsError::Compression`) — never guess, never lossy-coerce.
3. **Never grow the wire**: payloads below `min_size` and incompressible
   payloads are stored as identity.

Tagged at: `src/compression.rs::FrameCompressor::decompress`,
`src/compression.rs::decompress_frame`.

### Verifying tests

| Case | Test |
|------|------|
| 1 MiB bomb against 1 KiB cap → `PayloadTooLarge`; oversized identity rejected | unit `src/compression.rs::bomb_guard_caps_decompression` |
| unknown flags / empty envelope fail closed | unit `src/compression.rs::unknown_flags_fail_closed` |
| corrupt deflate stream errors | unit `src/compression.rs::corrupt_deflate_stream_is_an_error` |
| fake `Text` tag with non-UTF-8 errors, no lossy coercion | unit `src/compression.rs::text_frame_with_invalid_utf8_flag_rejected` |
| incompressible input stays identity (never grows) | unit `src/compression.rs::incompressible_payload_falls_back_to_identity` |
| text/binary round-trip incl. multibyte UTF-8 | unit `src/compression.rs::roundtrip_text_and_binary`, `multibyte_utf8_text_survives_exactly` |

## REQ-WSKIT-500 — Redis fan-out semantics are explicit (0.4.0 `redis`)

`RedisRoomRegistry` (multi-node room fan-out) must:

1. Deliver a broadcast to **local sinks and PUBLISH once** to
   `prefix + room_id`.
2. Apply remote messages **only to locally hosted rooms** — remote traffic
   never materializes rooms.
3. **Suppress self-echoes** (per-registry random node id) so a publisher's
   local receivers see each message exactly once.
4. **Fail closed** on malformed envelopes (version byte, kind tag, UTF-8).
5. **Document delivery as at-most-once, best-effort** — no persistence, no
   replay, no at-least-once claim.

Tagged at: `src/redis_rooms.rs` (module docs), `apply_remote`, `broadcast`.

### Verifying tests

| Case | Test |
|------|------|
| local fan-out + exactly one PUBLISH carrying the exact envelope (mocked `ConnectionLike`) | unit `src/redis_rooms.rs::broadcast_fans_out_locally_and_publishes` |
| remote envelope reaches a hosted room | unit `src/redis_rooms.rs::remote_envelope_reaches_hosted_room` |
| self-echo dropped | unit `src/redis_rooms.rs::self_echo_is_dropped`; live `tests/redis_rooms.rs::live_broadcast_fans_out_locally_and_across` (no duplicate) |
| remote traffic never materializes rooms | unit `src/redis_rooms.rs::remote_traffic_never_materializes_rooms`; live `tests/redis_rooms.rs::live_remote_traffic_never_materializes_rooms` |
| malformed envelopes fail closed (truncated/version/kind/UTF-8) | unit `src/redis_rooms.rs::malformed_envelopes_fail_closed` |
| channel naming/prefix isolation | unit `src/redis_rooms.rs::channel_names_roundtrip`, `custom_prefix_changes_channels`; live `tests/redis_rooms.rs::live_custom_prefix_isolates_tenants` |
| two-node text/binary delivery | live fixtures (`#[ignore]`, docker Redis): `tests/redis_rooms.rs::live_text_frame_crosses_nodes`, `live_binary_frame_crosses_nodes` |
