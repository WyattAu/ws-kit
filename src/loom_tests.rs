//! Loom model-checking tests for [`crate::counter::ConnectionCounter`].
//!
//! These verify the counter's invariants under *all* bounded interleavings:
//! 1. Balanced increment/decrement pairs across threads always return the
//!    count to zero (no lost updates) and the count never goes negative.
//! 2. `increment(Some(max))` never lets more than `max` concurrent
//!    increments succeed, even when threads race inside the CAS loop.
//! 3. A saturating decrement racing a capped increment never overshoots
//!    the limit nor underflows, and the pair lands back at zero.
//! 4. A three-way CAS race at a limit of 2 produces exactly two winners;
//!    teardown saturates at zero.
//!
//! The tokio broadcast channel and DashMap participants (hub/room
//! subscription tables, broadcast rings) are NOT model-checked — loom only
//! models `loom::sync` primitives, and those are trusted via tokio's own
//! concurrency testing (standards §3). The counter is the crate's own
//! atomic discipline, and the "subscribe during broadcast" delivery
//! guarantee lives entirely inside tokio's broadcast channel, not in
//! ws-kit code.
//!
//! Run with:
//! ```text
//! RUSTFLAGS="--cfg loom" cargo test --release --features loom loom -- --test-threads=1
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

/// Model 3: a saturating decrement races a capped increment.
///
/// Thread A runs `increment(Some(1))` (a CAS loop); thread B runs
/// `decrement()` (a saturating `fetch_update` loop) concurrently. Invariants
/// under every interleaving:
/// * the count never exceeds 1 (the increment's CAS cannot win past the
///   limit, even when B's decrement invalidates its reads mid-loop);
/// * the count never goes negative (B's saturating update cannot apply a
///   value below 0, including when B's read races A's successful CAS);
/// * the final count is 0 or 1: a decrement that ran before the increment
///   saturates at 0 and the increment lands on 1; a decrement that ran
///   after an accepted increment undoes it to 0; a rejected increment
///   leaves the saturating decrement at 0.
#[test]
fn loom_decrement_races_capped_increment_never_under_or_overshoots() {
    loom::model(|| {
        let counter = ConnectionCounter::new();

        let a = {
            let counter = counter.clone();
            loom::thread::spawn(move || counter.increment(Some(1)))
        };
        let b = {
            let counter = counter.clone();
            loom::thread::spawn(move || counter.decrement())
        };

        let incremented = a.join().unwrap();
        b.join().unwrap();

        match incremented {
            Ok(_) | Err(WsError::TooManyConnections) => {}
            Err(_) => panic!("unexpected error"),
        }
        let final_count = counter.get();
        assert!(
            final_count <= 1,
            "capped increment raced past the limit: {final_count}"
        );
        // Either order is legal; nothing else is.
        assert!(
            final_count == 0 || final_count == 1,
            "inc/dec race must land on 0 (dec after inc) or 1 (dec before inc)"
        );
    });
}

/// Model 4: three threads race a limit of 2 — exactly two winners, then
/// full saturation on teardown.
///
/// The CAS retry loop is exercised at its contended boundary: with three
/// racers and a limit of 2, a loser's failed compare_exchange must re-read
/// a value that is already at the limit and bail out with
/// `TooManyConnections`, never overshoot. Teardown: three decrements (one
/// more than the count) must saturate at exactly 0.
#[test]
fn loom_three_way_cas_race_yields_exactly_limit_winners() {
    loom::model(|| {
        let counter = ConnectionCounter::new();
        let winners = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..3 {
            let counter = counter.clone();
            let winners = Arc::clone(&winners);
            handles.push(loom::thread::spawn(move || {
                match counter.increment(Some(2)) {
                    Ok(_) => {
                        winners.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(WsError::TooManyConnections) => {}
                    Err(_) => panic!("unexpected error"),
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            winners.load(Ordering::Relaxed),
            2,
            "exactly two CAS winners"
        );
        assert_eq!(counter.get(), 2);

        for _ in 0..3 {
            counter.decrement(); // third is a no-op: saturating
        }
        assert_eq!(counter.get(), 0, "teardown must saturate at zero");
    });
}
