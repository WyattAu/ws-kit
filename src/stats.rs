//! Backpressure and traffic metrics.
//!
//! [`StatsRecorder`] is a cheap-to-clone handle over atomic counters —
//! global (all connections/rooms) and per-room. It is the single hook for
//! the numbers an operator needs under load:
//!
//! - **connections** — accepted sockets (integrator accept loop)
//! - **rooms** — live room count (wired into [`crate::room::RoomManager`])
//! - **messages/bytes in/out** — traffic volume; "in" is recorded by the
//!   integrator's read loop (ws-kit provides building blocks, not the read
//!   loop), "out" is recorded by room broadcasts and by hub callers
//! - **drops** — messages that hit a closed/empty broadcast channel
//!   (`WsError::BroadcastFull`), i.e. backpressure that lost data
//!
//! Read state via [`StatsRecorder::snapshot`] — an immutable [`Stats`]
//! struct for logging, health endpoints, or assertion in tests.
//!
//! With the optional `metrics` feature (off by default) the same counters
//! are additionally emitted through the [`metrics`] facade
//! (`ws_kit_*` counters/gauges), exportable to Prometheus/OTLP with any
//! exporter — the same pattern the `breaker` crate uses.
//!
//! # Example
//!
//! ```
//! use ws_kit::stats::StatsRecorder;
//!
//! let stats = StatsRecorder::new();
//! stats.inc_connections();
//! stats.record_message_in(128);
//! let snap = stats.snapshot();
//! assert_eq!(snap.connections, 1);
//! assert_eq!(snap.messages_in, 1);
//! assert_eq!(snap.bytes_in, 128);
//! ```

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;

/// Traffic counters for a single room.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomStats {
    /// Messages recorded as received in this room (integrator read loop).
    pub messages_in: u64,
    /// Messages fanned out to this room's receivers.
    pub messages_out: u64,
    /// Payload bytes received for this room.
    pub bytes_in: u64,
    /// Payload bytes written to this room's receivers.
    pub bytes_out: u64,
    /// Broadcast attempts that found no live receiver (data lost).
    pub drops: u64,
}

/// Point-in-time snapshot of global and per-room counters.
///
/// Produced by [`StatsRecorder::snapshot`]; deltas between two snapshots
/// give rates.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    /// Live connection count (accept minus close).
    pub connections: u64,
    /// Live room count.
    pub rooms: u64,
    /// Global messages recorded inbound (includes all room traffic).
    pub messages_in: u64,
    /// Global messages fanned outbound (includes all room traffic).
    pub messages_out: u64,
    /// Global inbound payload bytes.
    pub bytes_in: u64,
    /// Global outbound payload bytes.
    pub bytes_out: u64,
    /// Global drops: broadcasts with no live receiver (backpressure loss).
    pub drops: u64,
    /// Per-room counters, keyed by room id (sorted for stable output).
    pub per_room: BTreeMap<String, RoomStats>,
}

#[derive(Debug, Default)]
struct Globals {
    connections: AtomicU64,
    rooms: AtomicU64,
    messages_in: AtomicU64,
    messages_out: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    drops: AtomicU64,
}

#[derive(Debug, Default)]
struct RoomCounters {
    messages_in: AtomicU64,
    messages_out: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    drops: AtomicU64,
}

#[derive(Debug, Default)]
struct Inner {
    globals: Globals,
    rooms: DashMap<String, RoomCounters>,
}

/// Cheap-to-clone handle over global + per-room atomic counters.
///
/// All recording methods are lock-free (`AtomicU64` / sharded `DashMap`)
/// and safe to call from any thread. Clone handles share state.
#[derive(Clone, Debug, Default)]
pub struct StatsRecorder {
    inner: Arc<Inner>,
}

impl StatsRecorder {
    /// Create a recorder with all counters at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a connection accepted. Returns the new count.
    pub fn inc_connections(&self) -> u64 {
        let v = self
            .inner
            .globals
            .connections
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        #[cfg(feature = "metrics")]
        ::metrics::gauge!("ws_kit_connections").set(v as f64);
        v
    }

    /// Record a connection closed (saturating at zero). Returns the new count.
    pub fn dec_connections(&self) -> u64 {
        let v = saturating_dec(&self.inner.globals.connections);
        #[cfg(feature = "metrics")]
        ::metrics::gauge!("ws_kit_connections").set(v as f64);
        v
    }

    /// Record a room coming into existence (manager created it).
    pub fn room_opened(&self) {
        let v = self.inner.globals.rooms.fetch_add(1, Ordering::Relaxed) + 1;
        #[cfg(feature = "metrics")]
        ::metrics::gauge!("ws_kit_rooms").set(v as f64);
        #[cfg(not(feature = "metrics"))]
        let _ = v;
    }

    /// Record a room removed (saturating at zero).
    pub fn room_closed(&self) {
        let v = saturating_dec(&self.inner.globals.rooms);
        #[cfg(feature = "metrics")]
        ::metrics::gauge!("ws_kit_rooms").set(v as f64);
        #[cfg(not(feature = "metrics"))]
        let _ = v;
    }

    /// Record an inbound message of `bytes` payload bytes (global only —
    /// use [`StatsRecorder::record_room_message_in`] to also attribute it
    /// to a room).
    pub fn record_message_in(&self, bytes: u64) {
        self.inner
            .globals
            .messages_in
            .fetch_add(1, Ordering::Relaxed);
        self.inner
            .globals
            .bytes_in
            .fetch_add(bytes, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        {
            ::metrics::counter!("ws_kit_messages_in_total").increment(1);
            ::metrics::counter!("ws_kit_bytes_in_total").increment(bytes);
        }
    }

    /// Record `count` outbound messages totalling `bytes` payload bytes
    /// (global only).
    pub fn record_message_out(&self, count: u64, bytes: u64) {
        self.inner
            .globals
            .messages_out
            .fetch_add(count, Ordering::Relaxed);
        self.inner
            .globals
            .bytes_out
            .fetch_add(bytes, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        {
            ::metrics::counter!("ws_kit_messages_out_total").increment(count);
            ::metrics::counter!("ws_kit_bytes_out_total").increment(bytes);
        }
    }

    /// Record a drop: a broadcast that found no live receiver.
    pub fn record_drop(&self) {
        self.inner.globals.drops.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "metrics")]
        ::metrics::counter!("ws_kit_drops_total").increment(1);
    }

    /// Record an inbound message attributed to `room` (also bumps globals).
    pub fn record_room_message_in(&self, room: &str, bytes: u64) {
        let counters = self.room_counters(room);
        counters.messages_in.fetch_add(1, Ordering::Relaxed);
        counters.bytes_in.fetch_add(bytes, Ordering::Relaxed);
        drop(counters);
        self.record_message_in(bytes);
    }

    /// Record a room broadcast reaching `count` receivers with `bytes`
    /// total payload bytes (also bumps globals).
    pub fn record_room_message_out(&self, room: &str, count: u64, bytes: u64) {
        let counters = self.room_counters(room);
        counters.messages_out.fetch_add(count, Ordering::Relaxed);
        counters.bytes_out.fetch_add(bytes, Ordering::Relaxed);
        drop(counters);
        self.record_message_out(count, bytes);
    }

    /// Record a drop attributed to `room` (also bumps the global count).
    pub fn record_room_drop(&self, room: &str) {
        let counters = self.room_counters(room);
        counters.drops.fetch_add(1, Ordering::Relaxed);
        drop(counters);
        self.record_drop();
    }

    /// Snapshot all counters. `Stats::per_room` includes every room that
    /// ever recorded traffic (zeroed rooms remain listed).
    pub fn snapshot(&self) -> Stats {
        let g = &self.inner.globals;
        let per_room: BTreeMap<String, RoomStats> = self
            .inner
            .rooms
            .iter()
            .map(|e| {
                let c = e.value();
                (
                    e.key().clone(),
                    RoomStats {
                        messages_in: c.messages_in.load(Ordering::Relaxed),
                        messages_out: c.messages_out.load(Ordering::Relaxed),
                        bytes_in: c.bytes_in.load(Ordering::Relaxed),
                        bytes_out: c.bytes_out.load(Ordering::Relaxed),
                        drops: c.drops.load(Ordering::Relaxed),
                    },
                )
            })
            .collect();
        Stats {
            connections: g.connections.load(Ordering::Relaxed),
            rooms: g.rooms.load(Ordering::Relaxed),
            messages_in: g.messages_in.load(Ordering::Relaxed),
            messages_out: g.messages_out.load(Ordering::Relaxed),
            bytes_in: g.bytes_in.load(Ordering::Relaxed),
            bytes_out: g.bytes_out.load(Ordering::Relaxed),
            drops: g.drops.load(Ordering::Relaxed),
            per_room,
        }
    }

    fn room_counters(&self, room: &str) -> dashmap::mapref::one::Ref<'_, String, RoomCounters> {
        self.inner
            .rooms
            .entry(room.to_string())
            .or_default()
            .downgrade()
    }
}

fn saturating_dec(counter: &AtomicU64) -> u64 {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
        if v == 0 {
            None
        } else {
            Some(v - 1)
        }
    });
    counter.load(Ordering::Relaxed)
}

// Tests exercise counter invariants directly; unwrap/expect and panicking
// asserts are the test signal here.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_snapshot_zeroed() {
        let s = StatsRecorder::new().snapshot();
        assert_eq!(s, Stats::default());
    }

    #[test]
    fn connection_counter_saturates() {
        let stats = StatsRecorder::new();
        assert_eq!(stats.inc_connections(), 1);
        assert_eq!(stats.inc_connections(), 2);
        assert_eq!(stats.dec_connections(), 1);
        assert_eq!(stats.dec_connections(), 0);
        stats.dec_connections(); // saturate
        assert_eq!(stats.snapshot().connections, 0);
    }

    #[test]
    fn room_lifecycle_gauge() {
        let stats = StatsRecorder::new();
        stats.room_opened();
        stats.room_opened();
        assert_eq!(stats.snapshot().rooms, 2);
        stats.room_closed();
        stats.room_closed();
        stats.room_closed(); // saturate
        assert_eq!(stats.snapshot().rooms, 0);
    }

    #[test]
    fn global_traffic_counters() {
        let stats = StatsRecorder::new();
        stats.record_message_in(128);
        stats.record_message_out(3, 96);
        stats.record_drop();
        let s = stats.snapshot();
        assert_eq!(s.messages_in, 1);
        assert_eq!(s.bytes_in, 128);
        assert_eq!(s.messages_out, 3);
        assert_eq!(s.bytes_out, 96);
        assert_eq!(s.drops, 1);
    }

    #[test]
    fn room_counters_roll_up_to_globals() {
        let stats = StatsRecorder::new();
        stats.record_room_message_in("lobby", 10);
        stats.record_room_message_out("lobby", 2, 20);
        stats.record_room_drop("lobby");

        let s = stats.snapshot();
        let lobby = s.per_room.get("lobby").unwrap();
        assert_eq!(lobby.messages_in, 1);
        assert_eq!(lobby.bytes_in, 10);
        assert_eq!(lobby.messages_out, 2);
        assert_eq!(lobby.bytes_out, 20);
        assert_eq!(lobby.drops, 1);

        // Roll-up: globals include room traffic.
        assert_eq!(s.messages_in, 1);
        assert_eq!(s.messages_out, 2);
        assert_eq!(s.bytes_out, 20);
        assert_eq!(s.drops, 1);
    }

    #[test]
    fn rooms_are_isolated_in_snapshot() {
        let stats = StatsRecorder::new();
        stats.record_room_message_out("a", 1, 5);
        stats.record_room_message_out("b", 2, 10);
        let s = stats.snapshot();
        assert_eq!(s.per_room.len(), 2);
        assert_eq!(s.per_room.get("a").unwrap().messages_out, 1);
        assert_eq!(s.per_room.get("b").unwrap().messages_out, 2);
        assert_eq!(s.messages_out, 3);
    }

    #[test]
    fn clone_shares_counters() {
        let stats = StatsRecorder::new();
        let clone = stats.clone();
        stats.record_message_in(7);
        assert_eq!(clone.snapshot().messages_in, 1);
    }

    #[tokio::test]
    async fn concurrent_recording_no_lost_updates() {
        let stats = StatsRecorder::new();
        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = stats.clone();
            handles.push(tokio::spawn(async move {
                for _ in 0..1000 {
                    s.record_message_in(1);
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        assert_eq!(stats.snapshot().messages_in, 8000);
        assert_eq!(stats.snapshot().bytes_in, 8000);
    }
}
