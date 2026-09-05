//! Room-based group messaging.
//!
//! [`RoomManager`] owns a [`DashMap`] of [`Room`]s. Each [`Room`] is an
//! isolated broadcast domain with a participant roster keyed by connection
//! id (`u32`) mapping to a display name (`String`).

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::broadcast;

use crate::config::WsConfig;
use crate::error::WsError;

/// A single broadcast room with a participant roster.
pub struct Room {
    /// Room identifier.
    pub id: String,
    tx: broadcast::Sender<String>,
    /// Participant roster: connection id -> display name.
    participants: DashMap<u32, String>,
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
        let (tx, _) = broadcast::channel(capacity);
        Self {
            id: id.into(),
            tx,
            participants: DashMap::new(),
        }
    }

    /// Subscribe to this room's messages.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// Broadcast a text message to all participants.
    pub fn broadcast(&self, msg: String) -> Result<usize, WsError> {
        self.tx.send(msg).map_err(|_| WsError::BroadcastFull)
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
    pub fn sender(&self) -> &broadcast::Sender<String> {
        &self.tx
    }
}

/// Manages a collection of [`Room`]s.
#[derive(Debug)]
pub struct RoomManager {
    rooms: DashMap<String, Arc<Room>>,
    default_capacity: usize,
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
        }
    }

    /// Create an empty manager whose default capacity comes from [`WsConfig`].
    pub fn with_config(config: &WsConfig) -> Self {
        Self {
            rooms: DashMap::new(),
            default_capacity: config.broadcast_capacity,
        }
    }

    /// Get an existing room or create a new one with the manager's default capacity.
    pub fn get_or_create(&self, room_id: &str) -> Arc<Room> {
        self.get_or_create_with_capacity(room_id, self.default_capacity)
    }

    /// Get an existing room or create with custom capacity.
    pub fn get_or_create_with_capacity(&self, room_id: &str, capacity: usize) -> Arc<Room> {
        if let Some(r) = self.rooms.get(room_id) {
            return Arc::clone(&r);
        }
        let room = Arc::new(Room::new(room_id, capacity));
        // Insert-or-get to handle race.
        let entry = self
            .rooms
            .entry(room_id.to_string())
            .or_insert_with(|| Arc::clone(&room));
        Arc::clone(entry.value())
    }

    /// Get a room if it exists.
    pub fn get(&self, room_id: &str) -> Option<Arc<Room>> {
        self.rooms.get(room_id).map(|r| Arc::clone(&r))
    }

    /// Remove a room by id, returning it if present.
    pub fn remove(&self, room_id: &str) -> Option<Arc<Room>> {
        self.rooms.remove(room_id).map(|(_, v)| v)
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
        n
    }

    /// List room ids.
    pub fn room_ids(&self) -> Vec<String> {
        self.rooms.iter().map(|e| e.key().clone()).collect()
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
        assert_eq!(rx.recv().await.unwrap(), "hello");
    }

    #[tokio::test]
    async fn room_broadcast_no_receiver() {
        let room = Room::new("empty", 16);
        let err = room.broadcast("hi".to_string()).unwrap_err();
        assert_eq!(err, WsError::BroadcastFull);
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

    #[tokio::test]
    async fn manager_broadcast_isolation() {
        let m = RoomManager::new();
        let r1 = m.get_or_create("r1");
        let r2 = m.get_or_create("r2");
        let mut rx1 = r1.subscribe();
        let mut rx2 = r2.subscribe();
        r1.broadcast("for r1".to_string()).unwrap();
        assert_eq!(rx1.recv().await.unwrap(), "for r1");
        // r2 should not receive r1's message
        use tokio::time::{timeout, Duration};
        let res = timeout(Duration::from_millis(10), rx2.recv()).await;
        assert!(res.is_err());
    }
}
