Generic authenticated typed WebSocket toolkit for Rust; `BroadcastHub`, `RoomManager`, heartbeat, and token extraction for Axum.
- `axum` (default); Axum `ws` + `tower`, `TokenExtractor::extract_token(&Parts, _)`
- `compression`; application-layer DEFLATE for message payloads (see [Compression](#compression))
- `redis`; multi-node room fan-out over Redis pub/sub (see [Multi-node rooms](#multi-node-rooms-redis))
- `metrics`; emit the [`StatsRecorder`](#stats--backpressure) counters through the `metrics` facade
- `tracing`; deprecated no-op (kept for 0.3.x compatibility)
room.broadcast(vec![0xde, 0xad, 0xbe, 0xef]).unwrap(); // binary; zero re-encode
// Default (empty list) ALLOWS ALL origins; set your real origins:
- `Frame::Text(String)`; UTF-8 payloads; `Codec`/`JsonCodec` operate here.
- `Frame::Binary(Bytes)`; opaque bytes, carried verbatim.
Routing and room broadcast accept both (`impl Into<Frame>`; `String`,
or JSON by any ws-kit code path; the JSON decoders take `&str`, so the
`miniz_oxide` backend; the crate stays `#![forbid(unsafe_code)]`).
**Which layer; honest assessment:** the *right* layer for WebSocket
The stack ws-kit builds on; `tokio-tungstenite`/`tungstenite` 0.29 (what
axum 0.8 pulls); does **not implement it** and exposes no extension hook
  in; **browsers cannot use this** (they only speak `permessage-deflate`).
  *during* inflation (`CompressionConfig::max_size`, default 1 MiB); a
  64 B) and incompressible payloads are sent as identity; compression
  of that id; only if the node hosts it (remote traffic never
- Channel prefix is configurable (`connect_with_prefix`); unrelated
**Delivery semantics; read before trusting:** Redis pub/sub is
`StatsRecorder`; cheap-to-clone, lock-free (atomics + sharded map):
create/remove/cleanup), **messages/bytes in** (`record_message_in`; from
**drops** (broadcasts with no live receiver; backpressure that lost
export; same pattern as the `breaker` crate.
`--cfg loom`; see `src/loom_tests.rs`:
(`--no-default-features` drops the `axum` stack; `tokio-tungstenite` does
internals (not loom-compatible); those are trusted via tokio's and
The broadcast ring is pre-allocated at channel construction; steady-state `broadcast()` performs no per-message allocation beyond the caller's message value.