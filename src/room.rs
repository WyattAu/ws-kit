//! Room-based group messaging.
//!
//! [`RoomManager`] owns a [`DashMap`] of [`Room`]s. Each [`Room`] is an
//! isolated broadcast domain with participant tracking.

use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::broadcast;

use crate::error::WsError;

/// A single broadcast room.
pub struct Room {
    /// Room identifier.
    pub id: String,
    tx: broadcast::Sender<String>,
    participants: DashMap<String, ()>,
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

    /// Add a participant.
    pub fn join(&self, participant_id: impl Into<String>) {
        self.participants.insert(participant_id.into(), ());
    }

    /// Remove a participant.
    pub fn leave(&self, participant_id: &str) -> bool {
        self.participants.remove(participant_id).is_some()
    }

    /// Check if participant is in room.
    pub fn contains(&self, participant_id: &str) -> bool {
        self.participants.contains_key(participant_id)
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
#[derive(Debug, Default)]
pub struct RoomManager {
    rooms: DashMap<String, Arc<Room>>,
}

impl RoomManager {
    /// Create an empty manager.
    pub fn new() -> Self {
        Self {
            rooms: DashMap::new(),
        }
    }

    /// Get an existing room or create a new one with default capacity (1024).
    pub fn get_or_create(&self, room_id: &str) -> Arc<Room> {
        if let Some(r) = self.rooms.get(room_id) {
            return Arc::clone(&r);
        }
        let room = Arc::new(Room::new(room_id, 1024));
        // Insert-or-get to handle race.
        let entry = self.rooms.entry(room_id.to_string()).or_insert_with(|| Arc::clone(&room));
        Arc::clone(entry.value())
    }

    /// Get an existing room or create with custom capacity.
    pub fn get_or_create_with_capacity(&self, room_id: &str, capacity: usize) -> Arc<Room> {
        if let Some(r) = self.rooms.get(room_id) {
            return Arc::clone(&r);
        }
        let room = Arc::new(Room::new(room_id, capacity));
        let entry = self.rooms.entry(room_id.to_string()).or_insert_with(|| Arc::clone(&room));
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

    /// List room ids.
    pub fn room_ids(&self) -> Vec<String> {
        self.rooms.iter().map(|e| e.key().clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_join_leave() {
        let room = Room::new("lobby", 16);
        assert_eq!(room.participant_count(), 0);
        room.join("alice");
        room.join("bob");
        assert_eq!(room.participant_count(), 2);
        assert!(room.contains("alice"));
        assert!(room.leave("alice"));
        assert!(!room.contains("alice"));
        assert_eq!(room.participant_count(), 1);
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
    fn manager_cleanup_removes_empty() {
        let m = RoomManager::new();
        let _r = m.get_or_create("a");
        // Need a receiver or participant to keep alive? is_empty checks both.
        // No participants and no receivers -> empty, should be cleaned.
        // But the test creates a room with no receivers, so cleanup should remove.
        // However `get_or_create` doesn't create a receiver.
        assert_eq!(m.cleanup(), 1);
        assert_eq!(m.room_count(), 0);

        let r = m.get_or_create("b");
        r.join("user1");
        assert_eq!(m.cleanup(), 0);
        assert_eq!(m.room_count(), 1);
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
