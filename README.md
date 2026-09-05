# ws-kit

Generic authenticated typed WebSocket toolkit for Rust — `BroadcastHub`, `RoomManager`, heartbeat, and token extraction for Axum.

## Features

- `axum` (default) — Axum `ws` + `tower`, `TokenExtractor::extract_token(&Parts, _)`
- `tracing` — tracing instrumentation

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

Rooms:

```rust
use ws_kit::room::RoomManager;
let m = RoomManager::new();
let room = m.get_or_create("lobby");
room.join("alice");
room.broadcast("hello".to_string()).unwrap();
```

Token extraction:

```rust
use ws_kit::extractor::{TokenExtractor, TokenSourceKind};
let ex = TokenExtractor::default(); // Bearer, ?token=, ?access_token=
let tok = ex.extract_from_parts(Some("Bearer xyz"), None, "token=qry");
```

Codec:

```rust
use ws_kit::codec::Codec;
use serde::{Serialize, Deserialize};
#[derive(Serialize, Deserialize, PartialEq, Debug)] struct Msg { t: String }
let m = Msg { t: "hi".into() };
let s = m.encode();
let d = Msg::decode(&s).unwrap();
```

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
