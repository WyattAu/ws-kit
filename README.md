# ws-kit

Generic authenticated typed WebSocket toolkit for Rust — `BroadcastHub`, `RoomManager`, heartbeat, and token extraction for Axum.

## Features

- `std` (default) — `std` support for `thiserror`
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
