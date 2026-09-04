//! Shared connection counter.
//!
//! Extracted from [`crate::hub::BroadcastHub`] so the atomic logic can be
//! model-checked with loom: under `cfg(loom)` the atomic and `Arc` types are
//! swapped for loom's equivalents (see `loom_tests`).

#[cfg(loom)]
use loom::sync::atomic::{AtomicU64, Ordering};
#[cfg(loom)]
use loom::sync::Arc;
#[cfg(not(loom))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(not(loom))]
use std::sync::Arc;

use crate::error::WsError;

/// Connection counter shared between all clones of a hub.
///
/// Invariants (model-checked under `cfg(loom)`):
/// - `increment(Some(max))` never lets the count exceed `max`, even when
///   multiple threads race.
/// - `decrement` saturates at zero — the count never goes negative.
/// - Balanced increment/decrement pairs always return the count to zero,
///   regardless of interleaving (no lost updates).
#[derive(Clone)]
pub(crate) struct ConnectionCounter {
    inner: Arc<AtomicU64>,
}

impl ConnectionCounter {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Current count.
    pub(crate) fn get(&self) -> u64 {
        self.inner.load(Ordering::Relaxed)
    }

    /// Atomically increment the counter.
    ///
    /// Returns the new count. If `max` is `Some`, the count is never allowed
    /// to exceed the limit: racing threads use a compare-exchange loop so
    /// exactly the first `max` concurrent increments succeed and the rest
    /// get [`WsError::TooManyConnections`].
    pub(crate) fn increment(&self, max: Option<usize>) -> Result<u64, WsError> {
        match max {
            None => Ok(self.inner.fetch_add(1, Ordering::Relaxed) + 1),
            Some(limit) => loop {
                let current = self.inner.load(Ordering::Relaxed);
                if current as usize >= limit {
                    return Err(WsError::TooManyConnections);
                }
                match self.inner.compare_exchange(
                    current,
                    current + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return Ok(current + 1),
                    Err(_) => continue,
                }
            },
        }
    }

    /// Atomically decrement the counter, saturating at zero.
    ///
    /// Returns the count after the decrement.
    pub(crate) fn decrement(&self) -> u64 {
        let _ = self
            .inner
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                if v == 0 {
                    None
                } else {
                    Some(v - 1)
                }
            });
        self.inner.load(Ordering::Relaxed)
    }
}
