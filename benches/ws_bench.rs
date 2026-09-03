//! Benchmarks for ws-kit — BroadcastHub, RoomManager, and codec.
//!
//! Run with `cargo bench` (requires `criterion` feature if you want criterion
//! harness). This file is `harness = false` and provides a fallback manual
//! benchmark so `cargo check` and `cargo bench` work without extra deps.

#![allow(missing_docs)]

use std::hint::black_box;
use std::time::Instant;

use ws_kit::{codec::JsonCodec, hub::BroadcastHub, room::RoomManager};

#[cfg(feature = "criterion")]
use criterion::{Criterion, criterion_group, criterion_main};

#[cfg(feature = "criterion")]
fn bench_hub_broadcast(c: &mut Criterion) {
    let hub = BroadcastHub::<String>::new(1024);
    let _rx = hub.subscribe();
    c.bench_function("hub_broadcast", |b| {
        b.iter(|| {
            let msg = "hello world".to_string();
            black_box(hub.try_broadcast(msg));
        })
    });
}

#[cfg(feature = "criterion")]
fn bench_room_get_or_create(c: &mut Criterion) {
    let m = RoomManager::new();
    c.bench_function("room_get_or_create", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let id = format!("room_{}", i % 100);
            black_box(m.get_or_create(&id));
            i += 1;
        })
    });
}

#[cfg(feature = "criterion")]
fn bench_codec(c: &mut Criterion) {
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct Payload {
        id: u64,
        name: String,
        data: Vec<u8>,
    }

    let p = Payload {
        id: 42,
        name: "benchmark".into(),
        data: vec![1u8; 256],
    };

    c.bench_function("codec_encode", |b| {
        b.iter(|| black_box(JsonCodec::encode(&p).unwrap()))
    });

    let s = JsonCodec::encode(&p).unwrap();
    c.bench_function("codec_decode", |b| {
        b.iter(|| {
            let v: Payload = JsonCodec::decode(&s).unwrap();
            black_box(v)
        })
    });
}

#[cfg(feature = "criterion")]
criterion_group!(benches, bench_hub_broadcast, bench_room_get_or_create, bench_codec);
#[cfg(feature = "criterion")]
criterion_main!(benches);

#[cfg(not(feature = "criterion"))]
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
