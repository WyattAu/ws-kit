# ws-kit

[![docs.rs](https://docs.rs/ws-kit/badge.svg)](https://docs.rs/ws-kit)
[![crates.io](https://img.shields.io/crates/v/ws-kit.svg)](https://crates.io/crates/ws-kit)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

Generic authenticated typed WebSocket toolkit for Rust — `BroadcastHub`, `RoomManager`, heartbeat, and token extraction for Axum.

## Features

- `axum` (default) — Axum `ws` + `tower`, `TokenExtractor::extract_token(&Parts, _)`
- `compression` — application-layer DEFLATE for message payloads (see [Compression](#compression))
- `redis` — multi-node room fan-out over Redis pub/sub (see [Multi-node rooms](#multi-node-rooms-redis))
- `metrics` — emit the [`StatsRecorder`](#stats--backpressure) counters through the `metrics` facade
- `tracing` — deprecated no-op (kept for 0.3.x compatibility)

## Quick start

```rust
use ws_kit::{hub::BroadcastHub, codec::Codec};
use serde::{Serialize, Deserialize};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
struct Chat { user: String, body: String }

#[tokio::main]
async fn main() {
    let hub = BroadcastHub::<Chat>::new(1024);
    let mut rx = hub.subscribe();
    hub.broadcast(Chat { user: "alice".into(), body: "hi".into() }).unwrap();
    let msg = rx.recv().await.unwrap();
    assert_eq!(msg.user, "alice");
}
```

Rooms (text **and** binary since 0.4.0):

```rust
use ws_kit::room::RoomManager;
use ws_kit::codec::Frame;
let m = RoomManager::new();
let room = m.get_or_create("lobby");
room.join(1, "alice".to_string());
room.broadcast("hello".to_string()).unwrap();          // text
room.broadcast(vec![0xde, 0xad, 0xbe, 0xef]).unwrap(); // binary — zero re-encode
```

Token extraction:

```rust
use ws_kit::extractor::{TokenExtractor, TokenSourceKind};
let ex = TokenExtractor::default(); // Bearer, ?token=, ?access_token=
let tok = ex.extract_from_parts(Some("Bearer xyz"), None, "token=qry");
```

Origin validation (CSWSH defense):

```rust
use ws_kit::config::WsConfig;
use ws_kit::origin_allowed_in_parts;
use axum::http::StatusCode;

// Default (empty list) ALLOWS ALL origins — set your real origins:
let cfg = WsConfig::builder().allow_origin("https://app.example.com").build();

// In the upgrade handler, BEFORE ws.on_upgrade:
if !origin_allowed_in_parts(&parts, &cfg.allowed_origins) {
    return StatusCode::FORBIDDEN.into_response();
}
```

Exact match after normalization (lowercase scheme/host, default ports
omitted); no wildcard or suffix matching. See [REQUIREMENTS.md](REQUIREMENTS.md)
(REQ-WSKIT-200/201) and [THREAT-MODEL.md](THREAT-MODEL.md).

Codec:

```rust
use ws_kit::codec::Codec;
use serde::{Serialize, Deserialize};
#[derive(Serialize, Deserialize, PartialEq, Debug)] struct Msg { t: String }
let m = Msg { t: "hi".into() };
let s = m.encode();
let d = Msg::decode(&s).unwrap();
```

## Binary messages (0.4.0)

The wire message model is [`Frame`](https://docs.rs/ws-kit/latest/ws_kit/codec/enum.Frame.html):

- `Frame::Text(String)` — UTF-8 payloads; `Codec`/`JsonCodec` operate here.
- `Frame::Binary(Bytes)` — opaque bytes, carried verbatim.

Routing and room broadcast accept both (`impl Into<Frame>` — `String`,
`&str`, `Bytes`, `Vec<u8>` convert with no re-encoding of owned payloads),
and `BroadcastHub<Frame>` works out of the box (`Frame` is `Serialize`).

**Security posture:** a `Frame::Binary` payload is *never* parsed as text
or JSON by any ws-kit code path — the JSON decoders take `&str`, so the
type system enforces the separation. Text is guaranteed UTF-8 end to end
(`Frame::Text` can only be built from a `String`). Authentication
(`TokenExtractor`) reads the HTTP upgrade request and is frame-kind-agnostic.

## Compression

Feature: `compression` (off by default; pulls `flate2` with the pure-Rust
`miniz_oxide` backend — the crate stays `#![forbid(unsafe_code)]`).

```rust
use ws_kit::codec::Frame;
use ws_kit::compression::{CompressionConfig, FrameCompressor};

let c = FrameCompressor::new(CompressionConfig::default());
let wire = c.compress_frame(Frame::from("repetitive payload ".repeat(100)))?;
let restored = c.decompress_frame(wire)?; // == original frame
```

**Which layer — honest assessment:** the *right* layer for WebSocket
compression is the `permessage-deflate` handshake extension (RFC 7692).
The stack ws-kit builds on — `tokio-tungstenite`/`tungstenite` 0.29 (what
axum 0.8 pulls) — does **not implement it** and exposes no extension hook
to negotiate one. Rather than hand-roll an RFC 7692 state machine against
frame internals ws-kit doesn't own, 0.4.0 ships an **application-layer**
DEFLATE codec and says so:

- Payloads ride ordinary **binary frames** with a 1-byte envelope
  (`flags | deflate-or-identity`). Both endpoints must run ws-kit and opt
  in — **browsers cannot use this** (they only speak `permessage-deflate`).
  It is for Rust-to-Rust links (server ↔ ws-kit client/proxy/worker).
- "Negotiation" is out-of-band configuration; for integrators signaling the
  capability in a custom handshake header, `extension_accepted(Some(header))`
  matches the `x-ws-kit-deflate` token case-insensitively.
- **Decompression-bomb guard**: decompressed output is capped
  *during* inflation (`CompressionConfig::max_size`, default 1 MiB) — a
  compressed bomb never materializes in memory. Unknown envelope flags and
  corrupt streams fail closed (`WsError::Compression`).
- Oversharing is avoided on purpose: payloads below `min_size` (default
  64 B) and incompressible payloads are sent as identity — compression
  never *grows* the wire size.

Measured on the built-in bench (`cargo bench --features compression --bench
compression_roundtrip`, ratios are deterministic; absolute times vary with
machine load): synthetic 2 KiB chat JSON → 174 B (**11.9×**); repetitive
16 KiB binary → 130 B (**126×**).

When a handshake-level `permessage-deflate` lands upstream in
tokio-tungstenite/axum, `CompressionConfig` is the struct that gets wired
through.

## Multi-node rooms (Redis)

Feature: `redis` (off by default). `RedisRoomRegistry` keeps rooms local
(ordinary `RoomManager` semantics) and fans broadcasts out through Redis
pub/sub, so clients connected to different nodes share rooms:

```text
       node A                                node B
 ┌─────────────────┐                  ┌─────────────────┐
 │ RoomManager (A) │                  │ RoomManager (B) │
 │   room "lobby"  │                  │   room "lobby"  │
 └───┬────────┬────┘                  └───┬────────┬────┘
     │        │                           │        │
  local    PUBLISH ws-kit:room:lobby   local    PUBLISH
  sinks          │                     sinks        │
     │        ┌───▼───────────────────────┼──────────┤
     │        │            Redis          │          │
     │        └───┬───────────────────────┴──────────┤
     │            │     (pub/sub: best-effort)       │
  every node PSUBSCRIBEs ws-kit:room:* → delivers each message
  to its LOCAL room with that id (if hosted)
```

```rust,ignore
let registry = std::sync::Arc::new(
    ws_kit::redis_rooms::RedisRoomRegistry::connect("redis://127.0.0.1:6379").await?,
);
// One subscriber task per node, at startup:
let sub = std::sync::Arc::clone(&registry);
tokio::spawn(async move { let _ = sub.run_subscriber().await; });

registry.broadcast("lobby", ws_kit::codec::Frame::from("hi fleet")).await?;
```

Topology details:

- `broadcast(room, frame)` delivers to **local sinks and PUBLISHes once**;
  each node's subscriber loop applies received messages to the local room
  of that id — only if the node hosts it (remote traffic never
  materializes rooms).
- Each registry carries a random node id; **self-echoes are dropped**, so
  a publisher's local receivers see each message exactly once.
- Channel prefix is configurable (`connect_with_prefix`) — unrelated
  deployments can share one Redis.

**Delivery semantics — read before trusting:** Redis pub/sub is
**at-most-once, best-effort**. No persistence, no replay, no acks; a node
that is disconnected or lagging misses messages, permanently. ws-kit does
*not* provide at-least-once delivery and makes no persistence claim. Use
it for ephemeral fan-out (chat, presence, live notifications). For
at-least-once, the `RoomRegistry` trait keeps the swap to Redis Streams
(or any log) local to one module.

**Trust boundary:** anyone who can PUBLISH to the channel prefix can
inject messages into any room on any node; restrict with Redis ACLs /
network isolation and use `rediss://` for untrusted links.

Testing: the publish path is covered by a trait-mocked `ConnectionLike`
(see `src/redis_rooms.rs`); the full two-node pub/sub loop runs as
fixture-gated tests:

```sh
docker run -d -p 6379:6379 redis:7
cargo test --features redis --test redis_rooms -- --ignored
```

## Stats & backpressure (0.4.0)

`StatsRecorder` — cheap-to-clone, lock-free (atomics + sharded map):

```rust
use ws_kit::{config::WsConfig, room::RoomManager, stats::StatsRecorder};
let stats = StatsRecorder::new();
let manager = RoomManager::with_stats(&WsConfig::default(), stats.clone());
// room broadcasts now record per-room + global messages_out/bytes_out/drops
let snap = stats.snapshot();
// snap.connections, snap.rooms, snap.messages_in/out, snap.bytes_in/out,
// snap.drops, snap.per_room["lobby"]
```

Counters: **connections** (your accept loop: `inc_connections()` /
`dec_connections()`), **rooms** (wired into `RoomManager`
create/remove/cleanup), **messages/bytes in** (`record_message_in` — from
your read loop), **messages/bytes out** (wired into room broadcasts),
**drops** (broadcasts with no live receiver — backpressure that lost
data). With the `metrics` feature the same numbers are emitted as
`ws_kit_*` counters/gauges on the `metrics` facade for Prometheus/OTLP
export — same pattern as the `breaker` crate.

## Concurrency testing

The connection counter's atomic logic (bounded increment CAS loop, saturating
decrement) is model-checked with [loom](https://crates.io/crates/loom) under
`--cfg loom` — see `src/loom_tests.rs`:

```sh
RUSTFLAGS="--cfg loom" cargo test --release --lib --no-default-features -- loom
```

(`--no-default-features` drops the `axum` stack — `tokio-tungstenite` does
not compile under `--cfg loom`; the models only need the counter.)

Loom exhaustively explores bounded interleavings and proves: balanced
increment/decrement pairs always return the count to zero, the count never
goes negative, and the connection limit is never exceeded under races.
What loom does *not* cover: `tokio::sync::broadcast` and `DashMap`
internals (not loom-compatible) — those are trusted via tokio's and
dashmap's own concurrency testing.

## Security

Threat model: [THREAT-MODEL.md](THREAT-MODEL.md).

## Performance

Measured hot-path SLOs and allocation profile: [PERF-SLO.md](PERF-SLO.md). Benchmarks run in CI (non-gating regression visibility against the saved `ci` baseline).

| Hot path (criterion mean, 6-core x86_64) | P50 | SLO |
|---|---|---|
| `BroadcastHub` broadcast→recv round-trip, 1 receiver | **99.1 ns** | < 150 ns |
| fan-out per receiver (100–1000 receivers) | 42–44 ns | sub-linear scaling |
| `Frame`-based round-trip, 1 receiver (0.4.0 re-measure) | ~90 ns | text path unchanged; binary not slower |

The broadcast ring is pre-allocated at channel construction — steady-state `broadcast()` performs no per-message allocation beyond the caller's message value.
