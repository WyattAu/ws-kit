// iai-callgrind benchmarks run once under Valgrind on fixed inputs; the
// harness measures instruction counts, so there is no "expected failure"
// recovery path — a panic aborts the run visibly, which is what we want.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Deterministic regression gate for the hot paths behind PERF-SLO.md:
//!
//! - `frame_text_roundtrip` / `frame_binary_roundtrip` — the 0.4.0
//!   `Frame` rx path (PERF-SLO: text ~90 ns, binary not slower).
//! - `broadcast_fanout_1000rx` — fan-out per receiver (~40 ns/receiver
//!   at 1000 receivers).
//!
//! Criterion (`benches/broadcast_roundtrip.rs`) stays the source of the
//! wall-clock trend; this file is the pass/fail gate (instruction counts
//! are reproducible for a given binary — wall-clock is not).
//!
//! Workflow:
//!
//! - main: `cargo bench --bench iai_hot_path -- --save-baseline=main`
//!   (done by the `perf-gate` CI job; baselines update intentionally on
//!   every main push).
//! - PRs: `cargo bench --bench iai_hot_path -- --baseline=main --fail-fast`
//!   — any instruction-count regression fails the job.
//! - Locally this needs `valgrind` installed; without it, compile-check
//!   only: `cargo bench --no-run --bench iai_hot_path`.

use std::hint::black_box;

use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use ws_kit::codec::Frame;
use ws_kit::hub::BroadcastHub;

const PAYLOAD: &str = "bench payload";

type TextHub = BroadcastHub<ws_kit::codec::Frame>;
type BinHub = BroadcastHub<ws_kit::codec::Frame>;

fn setup_text_hub() -> (TextHub, tokio::sync::broadcast::Receiver<Frame>) {
    let hub = BroadcastHub::<Frame>::new(4096);
    let mut rx = hub.subscribe();
    // Warm the path: first broadcast/recv touches lazy ring state; the
    // measured call must be the steady-state path real traffic hits.
    black_box(hub.broadcast(Frame::from(PAYLOAD))).unwrap();
    black_box(rx.try_recv()).ok();
    (hub, rx)
}

fn setup_binary_hub() -> (BinHub, tokio::sync::broadcast::Receiver<Frame>) {
    let hub = BroadcastHub::<Frame>::new(4096);
    let mut rx = hub.subscribe();
    black_box(hub.broadcast(Frame::Binary(bytes::Bytes::from_static(b"bench payload")))).unwrap();
    black_box(rx.try_recv()).ok();
    (hub, rx)
}

fn setup_fanout() -> (TextHub, Vec<tokio::sync::broadcast::Receiver<Frame>>) {
    const RECEIVERS: usize = 1000;
    let hub = BroadcastHub::<Frame>::new(4096.max(RECEIVERS));
    let mut rxs: Vec<_> = (0..RECEIVERS).map(|_| hub.subscribe()).collect();
    black_box(hub.broadcast(Frame::from(PAYLOAD))).unwrap();
    for rx in &mut rxs {
        black_box(rx.try_recv()).ok();
    }
    (hub, rxs)
}

// Frame::Text steady-state round-trip: publish → consume, 1 receiver.
// PERF-SLO.md: ~90 ns mean (criterion); this gate pins the instructions.
#[library_benchmark]
#[bench::steady_state(setup = setup_text_hub)]
fn frame_text_roundtrip(env: (TextHub, tokio::sync::broadcast::Receiver<Frame>)) -> Option<Frame> {
    let (hub, mut rx) = env;
    black_box(hub.broadcast(Frame::from(PAYLOAD))).unwrap();
    rx.try_recv().ok()
}

// Frame::Binary steady-state round-trip: `Bytes` clones are refcount
// bumps — PERF-SLO claims binary rides the ring at text-level cost.
#[library_benchmark]
#[bench::steady_state(setup = setup_binary_hub)]
fn frame_binary_roundtrip(env: (BinHub, tokio::sync::broadcast::Receiver<Frame>)) -> Option<Frame> {
    let (hub, mut rx) = env;
    let payload = bytes::Bytes::from_static(b"bench payload");
    black_box(hub.broadcast(Frame::Binary(payload))).unwrap();
    rx.try_recv().ok()
}

// Fan-out at 1000 receivers: one broadcast + one try_recv each. PERF-SLO
// claims ~40 ns/receiver (sub-linear scaling).
#[library_benchmark]
#[bench::fanout_1000(setup = setup_fanout)]
fn broadcast_fanout_1000rx(env: (TextHub, Vec<tokio::sync::broadcast::Receiver<Frame>>)) -> usize {
    let (hub, mut rxs) = env;
    black_box(hub.broadcast(Frame::from(PAYLOAD))).unwrap();
    let mut delivered = 0;
    for rx in &mut rxs {
        if rx.try_recv().is_ok() {
            delivered += 1;
        }
    }
    delivered
}

library_benchmark_group!(
    name = iai_hot_path;
    benchmarks =
        frame_text_roundtrip,
        frame_binary_roundtrip,
        broadcast_fanout_1000rx
);

main!(library_benchmark_groups = iai_hot_path);
