# Threat Model: ws-kit
query string, Cookie header; all client-controlled), (2) messages flowing
multi-node fan-out (0.4.0; any process that can PUBLISH to the channel
| T10 | Binary frame confused with text (parse confusion / mojibake / JSON injection into decoders) | Tampering | `Frame` (0.4.0) | `Frame::Binary` is carried opaquely; never parsed as text/JSON; JSON decoders take `&str`, so binary cannot reach them without an explicit UTF-8 conversion; `Frame::into_text` on binary errors instead of coercing | `src/codec.rs::frame_into_bytes_and_text`, `tests/integration.rs::frame_into_text_rejects_binary`, `binary_frame_routes_through_{hub,room}` |
| T13 | Malicious/corrupt Redis publisher injects or corrupts room messages | Tampering / Spoofing | `redis_rooms` (0.4.0) | Envelope validated fail-closed (version byte, kind tag, UTF-8 for text) → `WsError::InvalidMessage`; self-echo suppression via per-registry node id; residual: the Redis channel is a **trusted bus**; ACLs/network isolation are deployment requirements | `src/redis_rooms.rs::malformed_envelopes_fail_closed`, `self_echo_is_dropped`; live fixtures `tests/redis_rooms.rs` |
| T14 | Node outage/maintenance silently loses pub/sub messages | Availability / Repudiation | `redis_rooms` (0.4.0) | **Documented, not mitigated**: Redis pub/sub is at-most-once, best-effort; no persistence/replay/acks. README + module docs state semantics loudly; use Redis Streams (integrator swap behind `RoomRegistry`) where at-least-once is required | documented in `src/redis_rooms.rs` module docs + README (no test can assert absence of persistence) |
## CLOSED RISKS (mitigated: cited by tests)
- **CLOSED-1 (was OPEN-3); no origin check on the upgrade request** -
  empty allow-list = allow all**, preserving 0.2.x behavior; CSWSH stays
## OPEN RISKS (missing mitigations: not fabricated)
- **OPEN-1; tokens in query strings are a default source.**
- **OPEN-2; URL decoding is byte-to-`char` naive.** `url_decode` maps each
  non-ASCII token) decodes to mojibake and is then rejected downstream -
- **OPEN-4; `max_connections = 0` means unlimited.** The config documents
- **OPEN-5; `parse_cookie` does not handle escaped separators** inside
- **OPEN-6 (0.4.0); Redis pub/sub channels are an unauthenticated trusted
- **OPEN-7 (0.4.0); compression side channels are out of scope.**
- Token *validation* (signature/expiry); extraction only; verification is
  stale query token uses the header first; predictable, but callers should