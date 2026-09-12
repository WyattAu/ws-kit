// Benchmarks run on fixed, known-good inputs; unwrap failures abort the
// bench run visibly, which is the desired behavior here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Hot-path latency: `BroadcastHub::broadcast` publish→consume round-trip,
//! measured with criterion. One iteration = one broadcast + one `try_recv`
//! per subscribed receiver (full fan-out round trip).

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use ws_kit::codec::Frame;
use ws_kit::hub::BroadcastHub;

fn bench_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("broadcast_roundtrip");
    for n in [1usize, 100, 1000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(format!("broadcast_recv_{n}rx"), |b| {
            let hub = BroadcastHub::<String>::new(4096.max(n));
            let mut rxs = Vec::with_capacity(n);
            for _ in 0..n {
                rxs.push(hub.subscribe());
            }
            b.iter(|| {
                black_box(hub.broadcast("bench payload".to_string())).unwrap();
                for rx in &mut rxs {
                    black_box(rx.try_recv().ok());
                }
            });
        });
    }
    group.finish();
}

/// 0.4.0 re-measurement: the same rx path over the `Frame` message model —
/// text must stay at String-level cost, binary must not slow it.
fn bench_frame_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_roundtrip");
    for n in [1usize, 1000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(format!("frame_text_{n}rx"), |b| {
            let hub = BroadcastHub::<Frame>::new(4096.max(n));
            let mut rxs: Vec<_> = (0..n).map(|_| hub.subscribe()).collect();
            b.iter(|| {
                black_box(hub.broadcast(Frame::from("bench payload".to_string()))).unwrap();
                for rx in &mut rxs {
                    black_box(rx.try_recv().ok());
                }
            });
        });
        group.bench_function(format!("frame_binary_{n}rx"), |b| {
            let hub = BroadcastHub::<Frame>::new(4096.max(n));
            let payload = bytes::Bytes::from_static(b"bench payload");
            let mut rxs: Vec<_> = (0..n).map(|_| hub.subscribe()).collect();
            b.iter(|| {
                black_box(hub.broadcast(Frame::Binary(payload.clone()))).unwrap();
                for rx in &mut rxs {
                    black_box(rx.try_recv().ok());
                }
            });
        });
    }
    group.finish();
}

fn bench_sustained_broadcast(c: &mut Criterion) {
    let mut group = c.benchmark_group("sustained_broadcast");
    for n in [1usize, 100, 1000] {
        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(format!("broadcast_{n}msgs_1rx"), |b| {
            let hub = BroadcastHub::<String>::new(4096);
            let mut rx = hub.subscribe();
            b.iter_custom(|iters| {
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    for _ in 0..n {
                        black_box(hub.try_broadcast("bench payload".to_string()));
                        black_box(rx.try_recv().ok());
                    }
                }
                start.elapsed()
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_roundtrip,
    bench_frame_roundtrip,
    bench_sustained_broadcast
);
criterion_main!(benches);
