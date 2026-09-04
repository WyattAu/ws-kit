//! Loom model-checking tests for [`crate::counter::ConnectionCounter`].
//!
//! These verify the counter's invariants under *all* bounded interleavings:
//! 1. Balanced increment/decrement pairs across threads always return the
//!    count to zero (no lost updates) and the count never goes negative.
//! 2. `increment(Some(max))` never lets more than `max` concurrent
//!    increments succeed, even when threads race inside the CAS loop.
//!
//! The tokio broadcast channel and DashMap participants are NOT model-checked
//! (loom only models `loom::sync` primitives) — those are trusted via tokio's
//! own concurrency testing.
//!
//! Run with:
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test --release --lib -- loom
//! ```

use crate::counter::ConnectionCounter;
use crate::error::WsError;
use loom::sync::atomic::{AtomicUsize, Ordering};
use loom::sync::Arc;

/// Model 1: two threads each perform an increment/decrement pair.
///
/// Invariants: final count is exactly 0 for every interleaving; the count
/// observed mid-flight never underflows.
#[test]
fn loom_inc_dec_pairs_balance_to_zero() {
    loom::model(|| {
        let counter = ConnectionCounter::new();

        let c2 = counter.clone();
        let handle = loom::thread::spawn(move || {
            c2.increment(None).unwrap();
            c2.decrement();
        });

        counter.increment(None).unwrap();
        counter.decrement();

        handle.join().unwrap();
        assert_eq!(counter.get(), 0);
    });
}

/// Model 2: two threads race to increment past a limit of 1.
///
/// Invariants: exactly one thread wins the CAS race; the loser observes the
/// limit is reached and gets `TooManyConnections`; the final count is exactly
/// 1; saturating decrements never push the count below 0.
#[test]
fn loom_increment_respects_limit_under_race() {
    loom::model(|| {
        let counter = ConnectionCounter::new();
        let ok_count = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let counter = counter.clone();
            let ok_count = Arc::clone(&ok_count);
            handles.push(loom::thread::spawn(move || {
                match counter.increment(Some(1)) {
                    Ok(_) => {
                        ok_count.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(WsError::TooManyConnections) => {}
                    Err(_) => panic!("unexpected error"),
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(ok_count.load(Ordering::Relaxed), 1);
        assert_eq!(counter.get(), 1);

        counter.decrement();
        counter.decrement(); // saturating: no underflow
        assert_eq!(counter.get(), 0);
    });
}
