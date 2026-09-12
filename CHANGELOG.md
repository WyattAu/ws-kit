# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

## [0.4.2] - 2026-09-12

### Added

- `tests/config_matrix.rs` — per-knob behavior matrix for all 7 knobs
  (`WsConfig` 4 + `CompressionConfig` 3): broadcast-capacity lag
  eviction, room plumbing, max-connections threading, origin
  allow-list, compression level/size/threshold contrasts. Dead-knob
  report: `WsConfig::heartbeat_interval` is stored but never read (no
  heartbeat task exists in the library) — pinned inert.

## [0.4.1] - 2026-09-12

### Added

- **Claims proof-back** ([CLAIMS.md](CLAIMS.md)): every numeric performance
  claim in README/PERF-SLO mapped to its proof artifact.
- `benches/iai_hot_path.rs` — iai-callgrind instruction-count gate for the
  hot paths: `Frame` text/binary round-trip and 1000-receiver fan-out
  (CI-gated; needs valgrind to execute locally). 2026-09-12 run: binary
  (21199 instr) ≤ text (21399) — the "binary is not slower" claim holds at
  the instruction level.
- `tests/zero_alloc_frame_rx.rs` — counting-allocator proof of the
  allocation profile: steady-state binary rx is allocation-free, text rx
  allocates exactly once (the `String` clone), `broadcast()` allocates only
  the caller's message value, owned-payload → `Frame` conversions are moves.
- Compression ratios re-verified (11.9× JSON 2 KiB; 125.6× ≈ 126× binary
  16 KiB); PERF-SLO table raw/compressed sizes corrected to the bench
  output (16448 B / 131 B).

### Changed

- PERF-SLO.md allocation profile upgraded from "code reading" to "proven"
  with test references; retired the "not yet verified with a counting
  allocator" disclaimer.

## [0.4.0] - 2026-09-12

### Added

- **Binary message support** — the wire model is now `codec::Frame`:
  `Frame::Text(String)` | `Frame::Binary(Bytes)`.
  - Routing and room broadcast accept anything `Frame` converts from
    (`String`, `&str`, `Bytes`, `Vec<u8>`, `&[u8]`) with zero re-encoding
    of owned payloads; `BroadcastHub<Frame>` works out of the box.
  - **Security posture (documented in `codec`):** binary payloads are never
    parsed as text/JSON — the JSON decoders take `&str`, so the type system
    enforces the separation; text is guaranteed UTF-8 end to end. Auth
    extraction is frame-kind-agnostic (it reads the HTTP upgrade request).
- **Compression** (`compression` feature, off by default) — application-layer
  DEFLATE via `flate2` (pure-Rust backend; `forbid(unsafe_code)` preserved):
  - `CompressionConfig` (level / `min_size` / `max_size`) +
    `FrameCompressor::compress_frame` / `decompress_frame` over a 1-byte
    envelope inside `Frame::Binary`.
  - **This is not `permessage-deflate`:** tokio-tungstenite 0.29 / axum 0.8
    do not implement RFC 7692 and expose no extension hook — the module
    docs carry the honest layer assessment. Rust-to-Rust links only;
    negotiation helpers (`EXTENSION_NAME`, `extension_accepted`) for custom
    handshake headers.
  - **Decompression-bomb guard**: output capped during inflation
    (default 1 MiB); unknown flags / corrupt streams fail closed.
  - Bench: `compression_roundtrip` — 2 KiB chat JSON 11.9×, repetitive
    16 KiB binary 126× (incompressible input is sent as identity, never grown).
- **Redis room adapter** (`redis` feature, off by default) — multi-node
  room fan-out over Redis pub/sub:
  - `room::RoomRegistry` trait extracted; `RoomManager` implements it
    (single-node), `RedisRoomRegistry` implements it for the local half.
  - Topology: `broadcast()` → local sinks **and** one `PUBLISH` on
    `ws-kit:room:{id}`; every node `PSUBSCRIBE`s the prefix and applies
    received messages to locally-hosted rooms only (no remote
    materialization). Random per-registry node id drops self-echoes.
  - **Semantics documented honestly: at-most-once, best-effort** — Redis
    pub/sub has no persistence or replay; ws-kit makes no at-least-once
    claim (see README for the Streams-based escape hatch).
  - Publish path tested against a trait-mocked `ConnectionLike`; the full
    two-node pub/sub loop is fixture-gated
    (`tests/redis_rooms.rs`, `#[ignore]`, docker Redis).
- **Backpressure stats hook** — `stats::StatsRecorder` (cheap-to-clone,
  lock-free): connections, rooms, messages/bytes in/out, drops — global +
  per-room, exposed as an immutable `Stats` snapshot. Wired into
  `RoomManager::with_stats` / `Room::with_stats` (outbound traffic and
  drops); `record_message_in` is the integrator read-loop hook.
- **`metrics` facade feature** (off by default) — the same counters emitted
  as `ws_kit_*` counters/gauges through the `metrics` crate, same pattern
  as the `breaker` crate.

### Changed

- `Room` payloads moved from `String` to `Frame`: `room.broadcast("x".into())`
  call sites keep compiling (`impl Into<Frame>`); `Room::subscribe()` now
  returns `broadcast::Receiver<Frame>` — match on the variant or use
  `Frame::into_text()` to restore old text-only behavior. (0.x minor;
  one-line migration for text-only subscribers.)
- `WsError` gained `Compression`, `PayloadTooLarge`, and `Redis(String)`
  variants (all `Clone + Eq` preserved).

### Performance

- Re-measured the rx path after the message-model change
  (`frame_roundtrip` bench): `Frame::Text` 1-rx round-trip ~90 ns (clean
  window; SLO < 150 ns), binary ~81 ns — **binary does not slow the text
  hot path**; 1000-receiver fan-out ~40 ns/receiver (was 42–44).
  Details: PERF-SLO.md.

## [0.3.0] - 2026-09-05

### Added

- Opt-in Origin validation for the WebSocket upgrade handshake (CSWSH
  defense, REQ-WSKIT-200/201):
  - `WsConfig::allowed_origins` field + `WsConfigBuilder::allow_origin` /
    `allowed_origins`.
  - `origin_allowed_in_parts(&Parts, &allowed)` — call in the upgrade
    handler **before** `on_upgrade`; missing or mismatched `Origin` should
    map to `403 Forbidden`.
  - `normalize_origin` — lowercase scheme/host, default ports (`http`/`ws`
    80, `https`/`wss` 443) omitted; exact match only, **no wildcard or
    suffix matching in v1**.
  - **Default (empty list) remains allow-all** — documented residual risk
    (REQ-WSKIT-201); existing users are not broken. Set the list to defend
    against cross-site WebSocket hijacking when auth rides cookies.

### Fixed

- Pre-existing `clippy::indexing_slicing` violation in
  `TokenExtractor::parse_bearer` (`bytes.get(..7)` instead of unchecked
  slicing) — the CI `-D warnings` gate now passes.

## [0.2.1] - 2026-09-04

### Fixed

- `--no-default-features` build.

### Testing

- Loom model-checking of `ConnectionCounter` invariants
  (`RUSTFLAGS="--cfg loom" cargo test --release --lib -- loom`): balanced
  increment/decrement pairs never lose updates or go negative; bounded
  increments hold under all interleavings.

## [0.2.0] - 2026-09-03

### Added

- Participants roster and manager counts on `RoomManager`.

## [0.1.0] - 2026-09-03

### Added

- Generic authenticated typed WebSocket kit: `BroadcastHub`,
  `RoomManager`, `TokenExtractor`, `Codec`, `WsConfig`.
- Feature-gated `axum` integration (default) and `tracing`
  instrumentation.
