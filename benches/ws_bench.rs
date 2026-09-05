// Benchmarks run on fixed, known-good inputs; unwrap failures abort the
// bench run visibly, which is the desired behavior here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Benchmarks for ws-kit — BroadcastHub, RoomManager, and codec.
//!
//! This file is `harness = false` and provides a manual benchmark so
//! `cargo check` and `cargo bench` work without extra deps.

#![allow(missing_docs)]

use std::hint::black_box;
use std::time::Instant;

use ws_kit::{codec::JsonCodec, hub::BroadcastHub, room::RoomManager};

fn main() {
    // Manual fallback benchmark — runs with `cargo bench` or `cargo run --bench ws_bench`
    let hub = BroadcastHub::<String>::new(1024);
    let _rx = hub.subscribe();
    let start = Instant::now();
    for _ in 0..50_000 {
        black_box(hub.try_broadcast("bench payload".to_string()));
    }
    eprintln!("hub 50k broadcasts: {:?}", start.elapsed());

    let m = RoomManager::new();
    let start = Instant::now();
    for i in 0..10_000 {
        black_box(m.get_or_create(&format!("room_{}", i % 100)));
    }
    eprintln!("room get_or_create 10k: {:?}", start.elapsed());

    let payload = serde_json::json!({"id": 42, "msg": "hello"});
    let start = Instant::now();
    for _ in 0..10_000 {
        let s = JsonCodec::encode(&payload).unwrap();
        let _: serde_json::Value = JsonCodec::decode(&s).unwrap();
        black_box(s);
    }
    eprintln!("codec 10k roundtrips: {:?}", start.elapsed());

    println!("ws_bench manual run complete");
}
