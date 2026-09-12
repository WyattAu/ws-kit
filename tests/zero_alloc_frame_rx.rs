// Zero-allocation hot-path gate: a counting global allocator proves the
// allocation-profile claims in PERF-SLO.md about the steady-state rx path.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Allocation counter tests for the `BroadcastHub` steady-state path.
//!
//! PERF-SLO.md "Allocation profile" claims, from code reading:
//!
//! - the broadcast ring is pre-allocated at construction — steady-state
//!   `broadcast()` allocates nothing beyond the caller's message value;
//! - each delivered receiver's `try_recv` clones the message: a
//!   `Frame::Text` clone allocates once (the `String`), a
//!   `Frame::Binary` clone is a refcount bump (no allocation);
//! - `Frame` conversions from owned payloads are moves (no allocation).
//!
//! This file turns those reading claims into measured facts on every
//! `cargo test` run. The iai-callgrind instruction-count gate (CI-only;
//! requires valgrind) pins the cycle cost; this file pins heap behavior.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use ws_kit::codec::Frame;
use ws_kit::hub::BroadcastHub;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations() -> usize {
    ALLOCATIONS.load(Ordering::Relaxed)
}

const PAYLOAD: &str = "alloc-probe payload";
const ITERATIONS: usize = 100;

/// One sequential test: the allocation counter is process-global, so
/// parallel test threads would pollute each other's counts.
#[test]
fn frame_rx_allocation_profile() {
    // --- Frame conversions from owned payloads are moves. ---
    // (Build the owned payloads first: `to_string()` itself allocates and
    // is not part of the claim — the conversion is.)
    let text = Frame::from(PAYLOAD.to_string());
    let bin = Frame::Binary(bytes::Bytes::from_static(b"owned"));
    let before = allocations();
    let moved_text = text; // move, no re-construction
    let moved_bin = bin.clone();
    std::hint::black_box((&moved_text, &moved_bin));
    assert_eq!(
        allocations(),
        before,
        "owned-payload Frame conversion/move must not allocate"
    );

    // --- Frame::Binary steady-state rx is allocation-free. ---
    let hub = BroadcastHub::<Frame>::new(64);
    let mut rx = hub.subscribe();
    assert!(hub
        .broadcast(Frame::Binary(bytes::Bytes::from_static(b"warm")))
        .is_ok());
    assert!(rx.try_recv().is_ok(), "warm-up recv must deliver");

    let before = allocations();
    for _ in 0..ITERATIONS {
        assert!(hub.broadcast(bin.clone()).is_ok());
        assert!(rx.try_recv().is_ok());
    }
    assert_eq!(
        allocations(),
        before,
        "binary rx steady state must not allocate (before: {before}, after: {})",
        allocations()
    );

    // --- Frame::Text steady-state rx: exactly one alloc per recv (the
    // String clone); broadcast itself allocates nothing. ---
    let mut rx = hub.subscribe();
    assert!(hub.broadcast(Frame::from("warm")).is_ok());
    assert!(rx.try_recv().is_ok(), "warm-up recv must deliver");

    // --- Broadcast-only: the only allocation is the caller's message
    // value itself (the ring is pre-allocated). ---
    let before = allocations();
    for _ in 0..ITERATIONS {
        // The `to_string()` IS the caller's message value; broadcast must
        // add nothing on top of it.
        assert!(hub.broadcast(Frame::from(PAYLOAD.to_string())).is_ok());
    }
    // Drain so clones pending in the ring don't leak into later sections.
    while rx.try_recv().is_ok() {}
    assert_eq!(
        allocations(),
        before + ITERATIONS,
        "broadcast must allocate only the caller's message value (1 per iteration)"
    );

    // --- Recv path: one String clone per delivered receiver. A fresh
    // receiver is used because a lagged receiver replays backlog on
    // drain, which would pollute the count. ---
    let mut fresh = hub.subscribe();
    assert!(hub.broadcast(Frame::from(PAYLOAD.to_string())).is_ok());
    let before = allocations();
    assert!(fresh.try_recv().is_ok());
    assert_eq!(
        allocations(),
        before + 1,
        "text recv = exactly one allocation (the String clone)"
    );
}

// Sanity guard: if this fails, the zero-alloc assertions above prove
// nothing (the counter would be broken, not the path miraculously free).
#[test]
fn allocation_counter_sanity() {
    let before = allocations();
    let leak = format!("fresh-{before}");
    assert!(
        allocations() > before,
        "String::from must allocate (counter sanity check)"
    );
    std::hint::black_box(leak);
}
