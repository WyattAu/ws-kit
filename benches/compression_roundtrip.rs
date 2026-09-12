// Benchmarks run on fixed, known-good inputs; unwrap failures abort the
// bench run visibly, which is the desired behavior here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Compression cost vs raw payload handling: `FrameCompressor`
//! compress→decompress round-trip against a no-compression baseline
//! (`std::mem::replace` of the payload — what an identity envelope costs).
//!
//! Payloads:
//! - `json_*`: synthetic chat JSON, a realistic text workload
//! - `binrep_*`: a repetitive 64-byte binary ramp (worst case for the
//!   compressor's favorability — near-zero entropy, crushes to almost
//!   nothing)
//!
//! Compile/run: `cargo bench --features compression --bench compression_roundtrip`

use std::hint::black_box;
use std::mem;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use ws_kit::compression::{CompressionConfig, FrameCompressor};

fn synthetic_json(size: usize) -> Vec<u8> {
    // Chat-message-ish JSON expanded to roughly `size` bytes by repeating
    // message entries in an array — realistic text redundancy profile.
    let entry = serde_json::json!({
        "type": "chat.message",
        "room": "lobby-42",
        "user": "alice",
        "seq": 172_000,
        "body": "hello world, this is a plausible chat line with words",
        "reactions": ["👍", "🔥"],
        "mentions": ["bob", "carol"],
    });
    let one = serde_json::to_vec(&entry).expect("json serializes");
    let base = one.len();
    let mut per_entry = Vec::with_capacity(one.len() + 1);
    per_entry.extend_from_slice(&one);
    per_entry.push(b',');
    let count = size / base + 1;
    let mut out = Vec::with_capacity(count * per_entry.len() + 2);
    out.push(b'[');
    for _ in 0..count {
        out.extend_from_slice(&per_entry);
    }
    out.push(b']');
    out
}

fn repetitive_binary(size: usize) -> Vec<u8> {
    let pattern: Vec<u8> = (0..=63u8).collect();
    pattern.repeat(size / 64 + 1)
}

fn bench_payload(c: &mut Criterion, label: &str, payload: &[u8]) {
    let compressor = FrameCompressor::new(CompressionConfig::default());
    let compressed = compressor.compress(payload).unwrap();
    let ratio = payload.len() as f64 / compressed.len().max(1) as f64;
    eprintln!(
        "{label}: raw={} compressed={} ratio={ratio:.1}x",
        payload.len(),
        compressed.len()
    );

    let mut group = c.benchmark_group(format!("compression/{label}"));
    group.throughput(Throughput::Bytes(payload.len() as u64));

    // Baseline: what "raw" handling costs per message (identity envelope
    // ≈ one memcpy out of a ring slot).
    group.bench_function("raw_identity", |b| {
        let mut slot = Vec::new();
        b.iter(|| {
            let _ = mem::replace(&mut slot, black_box(payload.to_vec()));
        });
    });

    group.bench_function("compress_decompress", |b| {
        b.iter(|| {
            let env = compressor.compress(black_box(payload)).unwrap();
            black_box(compressor.decompress(&env).unwrap());
        });
    });

    // Compress-only: the sender-side hot path.
    group.bench_function("compress_only", |b| {
        b.iter(|| black_box(compressor.compress(black_box(payload)).unwrap()));
    });

    // Decompress-only: the fan-out side (measured on the pre-compressed
    // envelope — receiver cost).
    group.bench_function("decompress_only", |b| {
        let env = compressor.compress(payload).unwrap();
        b.iter(|| black_box(compressor.decompress(&env).unwrap()));
    });

    group.finish();
}

fn bench_compression(c: &mut Criterion) {
    bench_payload(c, "json_2k", &synthetic_json(2048));
    bench_payload(c, "json_16k", &synthetic_json(16 * 1024));
    bench_payload(c, "binrep_2k", &repetitive_binary(2048));
    bench_payload(c, "binrep_16k", &repetitive_binary(16 * 1024));
}

criterion_group!(benches, bench_compression);
criterion_main!(benches);
