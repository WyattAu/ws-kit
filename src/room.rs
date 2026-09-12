//! Room-based group messaging.
//!
//! [`RoomManager`] owns a [`DashMap`] of [`Room`]s. Each [`Room`] is an
//! isolated broadcast domain with a participant roster keyed by connection
//! id (`u32`) mapping to a display name (`String`).
//!
//! Since 0.4.0 rooms carry [`Frame`] payloads — both text and binary —
//! instead of bare `String`s. Existing `room.broadcast(text_string)` call
//! sites keep compiling (`String: Into<Frame>`, zero-copy move); binary
//! payloads broadcast via `room.broadcast(bytes)`. Subscribers now receive
//! [`Frame`]s: match on the variant, or use [`Frame::into_text`] to restore
//! the old behavior for text-only consumers.
//!
//! ## The [`RoomRegistry`] trait
//!
//! [`RoomManager`] implements [`RoomRegistry`], the in-memory single-node
//! registry. Multi-node deployments swap in
//! [`crate::redis_rooms::RedisRoomRegistry`] (feature `redis`), which
//! implements the same trait for the local half while fanning broadcasts
//! out through Redis pub/sub.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::broadcast;

use crate::codec::Frame;
use crate::config::WsConfig;
use crate::error::WsError;
use crate::stats::StatsRecorder;

/// A single broadcast room with a participant roster.
pub struct Room {
    /// Room identifier.
    pub id: String,
    tx: broadcast::Sender<Frame>,
    /// Participant roster: connection id -> display name.
    participants: DashMap<u32, String>,
    stats: Option<StatsRecorder>,
}

impl std::fmt::Debug for Room {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Room")
            .field("id", &self.id)
            .field("participants", &self.participant_count())
            .field("receiver_count", &self.receiver_count())
            .finish()
    }
}

impl Room {
    /// Create a new room with the given id and broadcast capacity.
    pub fn new(id: impl Into<String>, capacity: usize) -> Self {
        Self::with_stats(id, capacity, None)
    }

    /// Create a new room that records outbound traffic and drops into
    /// `stats` (0.4.0 backpressure hook — see [`crate::stats`]).
    pub fn with_stats(
        id: impl Into<String>,
        capacity: usize,
        stats: Option<StatsRecorder>,
    ) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            id: id.into(),
            tx,
            participants: DashMap::new(),
            stats,
        }
    }

    /// Subscribe to this room's messages (text **and** binary).
    pub fn subscribe(&self) -> broadcast::Receiver<Frame> {
        self.tx.subscribe()
    }

    /// Broadcast a frame (text or binary) to all participants.
    ///
    /// Accepts anything [`Frame`] converts from — `String`, `&str`,
    /// `bytes::Bytes`, `Vec<u8>` — with no re-encoding for owned payloads.
    ///
    /// # Errors
    ///
    /// Returns [`WsError::BroadcastFull`] when the room has no active
    /// receivers; with a [`StatsRecorder`] attached this also counts as a
    /// drop (backpressure loss).
    pub fn broadcast(&self, msg: impl Into<Frame>) -> Result<usize, WsError> {
        let frame = msg.into();
        let n = self
            .tx
            .send(frame.clone())
            .map_err(|_| self.record_drop())?;
        if let Some(stats) = &self.stats {
            stats.record_room_message_out(&self.id, n as u64, frame.len() as u64 * n as u64);
        }
        Ok(n)
    }

    /// Add a participant (or refresh its display name on re-join).
    pub fn join(&self, participant_id: u32, name: String) {
        self.participants.insert(participant_id, name);
    }

    /// Remove a participant. Returns true if it was present.
    pub fn leave(&self, participant_id: u32) -> bool {
        self.participants.remove(&participant_id).is_some()
    }

    /// Check if participant is in room.
    pub fn contains(&self, participant_id: u32) -> bool {
        self.participants.contains_key(&participant_id)
    }

    /// Snapshot of `(participant_id, name)` pairs.
    pub fn participants(&self) -> Vec<(u32, String)> {
        self.participants
            .iter()
            .map(|r| (*r.key(), r.value().clone()))
            .collect()
    }

    /// Snapshot of participant display names.
    pub fn participant_names(&self) -> Vec<String> {
        self.participants
            .iter()
            .map(|r| r.value().clone())
            .collect()
    }

    /// Number of tracked participants.
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    /// Number of active broadcast receivers.
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Returns true if no participants and no receivers.
    pub fn is_empty(&self) -> bool {
        self.participants.is_empty() && self.tx.receiver_count() == 0
    }

    /// Internal sender.
    pub fn sender(&self) -> &broadcast::Sender<Frame> {
        &self.tx
    }

    fn record_drop(&self) -> WsError {
        if let Some(stats) = &self.stats {
            stats.record_room_drop(&self.id);
        }
        WsError::BroadcastFull
    }
}

/// Storage for rooms, abstracted so a room *collection* can be swapped
/// without changing room/messaging code (0.4.0).
///
/// The canonical single-node implementation is [`RoomManager`] (in-memory
/// [`DashMap`]). The multi-node implementation is
/// [`crate::redis_rooms::RedisRoomRegistry`] (feature `redis`), which
/// delegates to an inner `RoomManager` for the local node and fans
/// broadcasts out through Redis pub/sub.
pub trait RoomRegistry: Send + Sync {
    /// Get an existing room or create it.
    fn get_or_create(&self, room_id: &str) -> Arc<Room>;

    /// Get a room if it exists.
    fn get(&self, room_id: &str) -> Option<Arc<Room>>;

    /// Remove a room by id, returning it if present.
    fn remove(&self, room_id: &str) -> Option<Arc<Room>>;

    /// Number of rooms.
    fn room_count(&self) -> usize;

    /// List room ids.
    fn room_ids(&self) -> Vec<String>;

    /// Total participants across all rooms.
    fn total_participants(&self) -> usize;

    /// Remove all empty rooms (no participants, no receivers). Returns
    /// number removed.
    fn cleanup(&self) -> usize;

    /// Remove all rooms with zero participants (regardless of receivers).
    /// Returns number removed.
    fn cleanup_empty(&self) -> usize;
}

/// Manages a collection of [`Room`]s.
#[derive(Debug)]
pub struct RoomManager {
    rooms: DashMap<String, Arc<Room>>,
    default_capacity: usize,
    stats: Option<StatsRecorder>,
}

impl Default for RoomManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RoomManager {
    /// Create an empty manager with default capacity (1024).
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
            default_capacity: 1024,
            stats: None,
        }
    }

    /// Create an empty manager whose default capacity comes from [`WsConfig`].
    pub fn with_config(config: &WsConfig) -> Self {
        Self {
            rooms: DashMap::new(),
            default_capacity: config.broadcast_capacity,
            stats: None,
        }
    }

    /// Create an empty manager wired to a [`StatsRecorder`]: room
    /// create/remove updates the room gauge, room broadcasts record
    /// per-room + global outbound traffic and drops (0.4.0).
    pub fn with_stats(config: &WsConfig, stats: StatsRecorder) -> Self {
        Self {
            rooms: DashMap::new(),
            default_capacity: config.broadcast_capacity,
            stats: Some(stats),
        }
    }

    /// The recorder this manager reports to, if any.
    pub fn stats(&self) -> Option<&StatsRecorder> {
        self.stats.as_ref()
    }
}

impl RoomRegistry for RoomManager {
    // Pure delegation: the stats hooks live in the inherent methods below
    // (method resolution prefers those, so every call path is counted).
    fn get_or_create(&self, room_id: &str) -> Arc<Room> {
        RoomManager::get_or_create(self, room_id)
    }

    fn get(&self, room_id: &str) -> Option<Arc<Room>> {
        RoomManager::get(self, room_id)
    }

    fn remove(&self, room_id: &str) -> Option<Arc<Room>> {
        RoomManager::remove(self, room_id)
    }

    fn room_count(&self) -> usize {
        RoomManager::room_count(self)
    }

    fn room_ids(&self) -> Vec<String> {
        RoomManager::room_ids(self)
    }

    fn total_participants(&self) -> usize {
        RoomManager::total_participants(self)
    }

    fn cleanup(&self) -> usize {
        RoomManager::cleanup(self)
    }

    fn cleanup_empty(&self) -> usize {
        RoomManager::cleanup_empty(self)
    }
}

impl RoomManager {
    /// Get an existing room or create it with the manager's default capacity.
    pub fn get_or_create(&self, room_id: &str) -> Arc<Room> {
        self.get_or_create_with_capacity(room_id, self.default_capacity)
    }

    /// Get a room if it exists.
    pub fn get(&self, room_id: &str) -> Option<Arc<Room>> {
        self.rooms.get(room_id).map(|r| Arc::clone(&r))
    }

    /// Remove a room by id, returning it if present.
    pub fn remove(&self, room_id: &str) -> Option<Arc<Room>> {
        let removed = self.rooms.remove(room_id).map(|(_, v)| v);
        if removed.is_some() {
            if let Some(stats) = &self.stats {
                stats.room_closed();
            }
        }
        removed
    }

    /// Number of rooms.
    pub fn room_count(&self) -> usize {
        self.rooms.len()
    }

    /// Total participants across all rooms.
    pub fn total_participants(&self) -> usize {
        self.rooms
            .iter()
            .map(|e| e.value().participant_count())
            .sum()
    }

    /// Remove all empty rooms. Returns number removed.
    pub fn cleanup(&self) -> usize {
        let mut to_remove = Vec::new();
        for entry in self.rooms.iter() {
            if entry.value().is_empty() {
                to_remove.push(entry.key().clone());
            }
        }
        let n = to_remove.len();
        for k in to_remove {
            self.rooms.remove(&k);
        }
        if n > 0 {
            if let Some(stats) = &self.stats {
                for _ in 0..n {
                    stats.room_closed();
                }
            }
        }
        n
    }

    /// Remove all rooms with zero participants (regardless of receivers).
    /// Returns number removed.
    pub fn cleanup_empty(&self) -> usize {
        let to_remove: Vec<String> = self
            .rooms
            .iter()
            .filter(|e| e.value().participant_count() == 0)
            .map(|e| e.key().clone())
            .collect();
        let n = to_remove.len();
        for k in to_remove {
            self.rooms.remove(&k);
        }
        if n > 0 {
            if let Some(stats) = &self.stats {
                for _ in 0..n {
                    stats.room_closed();
                }
            }
        }
        n
    }

    /// List room ids.
    pub fn room_ids(&self) -> Vec<String> {
        self.rooms.iter().map(|e| e.key().clone()).collect()
    }

    /// Get an existing room or create with custom capacity.
    pub fn get_or_create_with_capacity(&self, room_id: &str, capacity: usize) -> Arc<Room> {
        if let Some(r) = self.rooms.get(room_id) {
            return Arc::clone(&r);
        }
        let room = Arc::new(Room::with_stats(room_id, capacity, self.stats.clone()));
        if let Some(stats) = &self.stats {
            stats.room_opened();
        }
        // Insert-or-get to handle race. room_opened fires before the
        // insert-or-get, so a lost race (another thread inserted first)
        // can transiently overcount; room_closed on removal keeps the
        // gauge convergent.
        let entry = self
            .rooms
            .entry(room_id.to_string())
            .or_insert_with(|| Arc::clone(&room));
        Arc::clone(entry.value())
    }
}

// Tests exercise failure paths and invariants directly; unwrap/expect,
// slicing, and panicking asserts are acceptable here — violations
// surface as test failures, not production panics.
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn name(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn room_join_leave() {
        let room = Room::new("lobby", 16);
        assert_eq!(room.participant_count(), 0);
        room.join(1, name("alice"));
        room.join(2, name("bob"));
        assert_eq!(room.participant_count(), 2);
        assert!(room.contains(1));
        assert!(room.leave(1));
        assert!(!room.contains(1));
        assert_eq!(room.participant_count(), 1);
    }

    #[test]
    fn room_rejoin_updates_name() {
        let room = Room::new("lobby", 16);
        room.join(1, name("alice"));
        room.join(1, name("alice2"));
        assert_eq!(room.participant_count(), 1);
        assert_eq!(room.participant_names(), vec!["alice2".to_string()]);
    }

    #[test]
    fn room_participant_snapshots() {
        let room = Room::new("lobby", 16);
        room.join(7, name("alice"));
        room.join(3, name("bob"));
        let mut pairs = room.participants();
        pairs.sort();
        assert_eq!(
            pairs,
            vec![(3, "bob".to_string()), (7, "alice".to_string())]
        );
        let mut names = room.participant_names();
        names.sort();
        assert_eq!(names, vec!["alice".to_string(), "bob".to_string()]);
        assert_eq!(room.participant_count(), 2);
    }

    #[tokio::test]
    async fn room_broadcast() {
        let room = Room::new("chat", 16);
        let mut rx = room.subscribe();
        let n = room.broadcast("hello".to_string()).unwrap();
        assert_eq!(n, 1);
        assert_eq!(rx.recv().await.unwrap(), Frame::from("hello"));
    }

    #[tokio::test]
    async fn room_broadcast_binary() {
        let room = Room::new("binroom", 16);
        let mut rx = room.subscribe();
        let payload = vec![0xde_u8, 0xad, 0xbe, 0xef];
        let n = room.broadcast(Frame::from(payload.clone())).unwrap();
        assert_eq!(n, 1);
        let got = rx.recv().await.unwrap();
        assert!(got.is_binary());
        assert_eq!(got.as_bytes(), &payload[..]);
    }

    #[tokio::test]
    async fn room_broadcast_bytes_vec_convenience() {
        // Vec<u8> converts into a Binary frame directly.
        let room = Room::new("binroom", 16);
        let mut rx = room.subscribe();
        room.broadcast(vec![1u8, 2, 3]).unwrap();
        let got = rx.recv().await.unwrap();
        assert!(got.is_binary());
        assert_eq!(got.as_bytes(), &[1, 2, 3]);
    }

    #[tokio::test]
    async fn room_text_and_binary_share_one_channel_without_cross_talk() {
        let room = Room::new("mixed", 16);
        let mut rx = room.subscribe();
        room.broadcast("text".to_string()).unwrap();
        room.broadcast(Bytes::from_static(b"\x00\x01")).unwrap();
        assert!(rx.recv().await.unwrap().is_text());
        assert!(rx.recv().await.unwrap().is_binary());
    }

    #[tokio::test]
    async fn room_broadcast_no_receiver() {
        let room = Room::new("empty", 16);
        let err = room.broadcast("hi".to_string()).unwrap_err();
        assert_eq!(err, WsError::BroadcastFull);
    }

    #[tokio::test]
    async fn room_stats_records_outbound_and_drops() {
        let stats = StatsRecorder::new();
        let room = Room::with_stats("metrics", 16, Some(stats.clone()));
        let mut rx = room.subscribe();
        room.broadcast("12345".to_string()).unwrap(); // 5 bytes * 1 rx
        rx.recv().await.unwrap();

        let s = stats.snapshot();
        let room_stats = s.per_room.get("metrics").unwrap();
        assert_eq!(room_stats.messages_out, 1);
        assert_eq!(room_stats.bytes_out, 5);
        assert_eq!(s.messages_out, 1);

        drop(rx);
        room.broadcast("nobody".to_string()).unwrap_err();
        assert_eq!(stats.snapshot().per_room.get("metrics").unwrap().drops, 1);
        assert_eq!(stats.snapshot().drops, 1);
    }

    #[test]
    fn manager_get_or_create_idempotent() {
        let m = RoomManager::new();
        let r1 = m.get_or_create("room1");
        let r2 = m.get_or_create("room1");
        assert!(Arc::ptr_eq(&r1, &r2));
        assert_eq!(m.room_count(), 1);
    }

    #[test]
    fn manager_with_config_capacity() {
        let cfg = WsConfig::builder().broadcast_capacity(2048).build();
        let m = RoomManager::with_config(&cfg);
        let r = m.get_or_create("room1");
        // Capacity is not directly observable; verify the room works and is shared.
        let r2 = m.get_or_create("room1");
        assert!(Arc::ptr_eq(&r, &r2));
        assert_eq!(m.room_count(), 1);
    }

    #[test]
    fn manager_cleanup_removes_empty() {
        let m = RoomManager::new();
        let _r = m.get_or_create("a");
        // No participants and no receivers -> empty, should be cleaned.
        assert_eq!(m.cleanup(), 1);
        assert_eq!(m.room_count(), 0);

        let r = m.get_or_create("b");
        r.join(1, name("user1"));
        assert_eq!(m.cleanup(), 0);
        assert_eq!(m.room_count(), 1);
    }

    #[test]
    fn manager_cleanup_empty_removes_zero_participant_rooms() {
        let m = RoomManager::new();
        let _a = m.get_or_create("a");
        // No participants -> removed regardless of receivers.
        assert_eq!(m.cleanup_empty(), 1);
        assert_eq!(m.room_count(), 0);

        // Participant present -> kept.
        let a = m.get_or_create("a");
        a.join(1, name("alice"));
        assert_eq!(m.cleanup_empty(), 0);
        assert_eq!(m.room_count(), 1);
        let kept = m.get_or_create("a");
        assert!(Arc::ptr_eq(&a, &kept));

        a.leave(1);
        assert_eq!(m.cleanup_empty(), 1);
        assert_eq!(m.room_count(), 0);
    }

    #[test]
    fn manager_total_participants() {
        let m = RoomManager::new();
        let r1 = m.get_or_create("r1");
        let r2 = m.get_or_create("r2");
        assert_eq!(m.total_participants(), 0);
        r1.join(1, name("alice"));
        r1.join(2, name("bob"));
        r2.join(3, name("carol"));
        assert_eq!(m.total_participants(), 3);
        r2.leave(3);
        assert_eq!(m.total_participants(), 2);
    }

    #[test]
    fn manager_get_and_remove() {
        let m = RoomManager::new();
        assert!(m.get("nonexistent").is_none());
        let r = m.get_or_create("x");
        assert!(m.get("x").is_some());
        let removed = m.remove("x").unwrap();
        assert!(Arc::ptr_eq(&r, &removed));
        assert!(m.get("x").is_none());
    }

    #[test]
    fn manager_stats_room_lifecycle() {
        let stats = StatsRecorder::new();
        let cfg = WsConfig::default();
        let m = RoomManager::with_stats(&cfg, stats.clone());
        let _r = m.get_or_create("a");
        assert_eq!(stats.snapshot().rooms, 1);
        m.get_or_create("a"); // idempotent: no double count
        assert_eq!(stats.snapshot().rooms, 1);
        m.remove("a");
        assert_eq!(stats.snapshot().rooms, 0);
        m.get_or_create("b");
        m.get_or_create("c");
        m.cleanup();
        assert_eq!(stats.snapshot().rooms, 0);
    }

    #[test]
    fn trait_object_room_registry() {
        // The trait exists so room *collections* are swappable; prove it is
        // object-safe and delegates.
        let manager = RoomManager::new();
        let registry: &dyn RoomRegistry = &manager;
        let r = registry.get_or_create("dyn_room");
        r.join(1, name("alice"));
        assert_eq!(registry.room_count(), 1);
        assert_eq!(registry.total_participants(), 1);
        assert_eq!(registry.room_ids(), vec!["dyn_room".to_string()]);
        assert!(registry.get("dyn_room").is_some());
        assert!(registry.remove("dyn_room").is_some());
        assert_eq!(registry.cleanup(), 0);
        assert_eq!(registry.cleanup_empty(), 0);
    }

    #[tokio::test]
    async fn manager_broadcast_isolation() {
        let m = RoomManager::new();
        let r1 = m.get_or_create("r1");
        let r2 = m.get_or_create("r2");
        let mut rx1 = r1.subscribe();
        let mut rx2 = r2.subscribe();
        r1.broadcast("for r1".to_string()).unwrap();
        assert_eq!(rx1.recv().await.unwrap(), Frame::from("for r1"));
        // r2 should not receive r1's message
        use tokio::time::{timeout, Duration};
        let res = timeout(Duration::from_millis(10), rx2.recv()).await;
        assert!(res.is_err());
    }
}
