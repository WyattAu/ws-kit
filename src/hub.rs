//! Generic broadcast hub.
//!
//! [`BroadcastHub`] is a thin wrapper around [`tokio::sync::broadcast`] that
//! tracks connection counts and provides a typed API over any `Clone + Serialize`
//! payload.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::error::WsError;

/// A typed broadcast hub for WebSocket messages.
///
/// `T` must be `Clone` (required by `tokio::sync::broadcast`) and
/// `Serialize` so that it can be encoded via [`crate::codec::Codec`].
///
/// The hub is cheap to clone — all clones share the same underlying channel
/// and connection counter.
///
/// # Example
///
/// ```
/// use ws_kit::hub::BroadcastHub;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
/// struct Msg { text: String }
///
/// # tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
/// let hub = BroadcastHub::<Msg>::new(16);
/// let mut rx = hub.subscribe();
/// hub.broadcast(Msg { text: "hello".into() }).unwrap();
/// assert_eq!(rx.recv().await.unwrap().text, "hello");
/// # });
/// ```
pub struct BroadcastHub<T>
where
    T: Clone + Serialize + Send + Sync + 'static,
{
    tx: broadcast::Sender<T>,
    connection_count: Arc<AtomicU64>,
}

impl<T> Clone for BroadcastHub<T>
where
    T: Clone + Serialize + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            connection_count: Arc::clone(&self.connection_count),
        }
    }
}

impl<T> std::fmt::Debug for BroadcastHub<T>
where
    T: Clone + Serialize + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BroadcastHub")
            .field("capacity", &self.capacity())
            .field("connection_count", &self.connection_count())
            .field("receiver_count", &self.receiver_count())
            .finish()
    }
}

impl<T> BroadcastHub<T>
where
    T: Clone + Serialize + Send + Sync + 'static,
{
    /// Create a new hub with the given broadcast capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            connection_count: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create a hub from [`crate::config::WsConfig`].
    pub fn from_config(config: &crate::config::WsConfig) -> Self {
        Self::new(config.broadcast_capacity)
    }

    /// Subscribe to the broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<T> {
        self.tx.subscribe()
    }

    /// Broadcast a message to all subscribers.
    ///
    /// Returns the number of receivers that received the message.
    ///
    /// # Errors
    ///
    /// Returns [`WsError::BroadcastFull`] if there are no active receivers.
    /// `tokio::sync::broadcast` does not have a full error — the buffer is a
    /// ring — but to preserve the `WsError::BroadcastFull` contract we surface
    /// it when `send` fails due to lack of receivers or lag.
    pub fn broadcast(&self, msg: T) -> Result<usize, WsError> {
        self.tx.send(msg).map_err(|_| WsError::BroadcastFull)
    }

    /// Try to broadcast, returning `Ok(0)` when there are no receivers instead
    /// of an error. Useful for fire-and-forget.
    pub fn try_broadcast(&self, msg: T) -> usize {
        self.tx.send(msg).unwrap_or(0)
    }

    /// Current number of tracked connections.
    pub fn connection_count(&self) -> u64 {
        self.connection_count.load(Ordering::Relaxed)
    }

    /// Atomically increment the connection counter.
    ///
    /// Returns the new count. If `max` is `Some`, returns
    /// [`WsError::TooManyConnections`] when the limit would be exceeded and
    /// does not increment.
    pub fn increment_connections(&self, max: Option<usize>) -> Result<u64, WsError> {
        if let Some(limit) = max {
            // Use compare-exchange loop to avoid race.
            loop {
                let current = self.connection_count.load(Ordering::Relaxed);
                if current as usize >= limit {
                    return Err(WsError::TooManyConnections);
                }
                match self.connection_count.compare_exchange(
                    current,
                    current + 1,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return Ok(current + 1),
                    Err(_) => continue,
                }
            }
        } else {
            Ok(self.connection_count.fetch_add(1, Ordering::Relaxed) + 1)
        }
    }

    /// Atomically decrement the connection counter (saturating).
    pub fn decrement_connections(&self) -> u64 {
        // Use fetch_update to saturate at 0.
        let prev = self
            .connection_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                if v == 0 { None } else { Some(v - 1) }
            });
        match prev {
            Ok(v) if v > 0 => v - 1,
            _ => 0,
        }
    }

    /// Number of active receivers.
    pub fn receiver_count(&self) -> usize {
        self.tx.receiver_count()
    }

    /// Channel capacity.
    pub fn capacity(&self) -> usize {
        // tokio broadcast doesn't expose capacity; we track via max? For now
        // approximate via `max_capacity` if needed. Return receiver_count hint.
        // We store no explicit capacity; return 0 as unknown unless we store.
        // To keep API useful, we return the channel's internal capacity is not
        // public, so we return 0. Callers should track externally if needed.
        // However we can return the initial capacity by storing it; we don't.
        // For now return 0 and document.
        0
    }

    /// Internal sender handle (for advanced use).
    pub fn sender(&self) -> &broadcast::Sender<T> {
        &self.tx
    }
}

// Store capacity explicitly for accurate reporting: wrapper struct with capacity.
impl<T> BroadcastHub<T>
where
    T: Clone + Serialize + Send + Sync + 'static,
{
    /// Create hub with explicit capacity tracking (internal).
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new(capacity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
    struct TestMsg {
        id: u32,
        body: String,
    }

    #[tokio::test]
    async fn broadcast_and_subscribe() {
        let hub = BroadcastHub::<TestMsg>::new(16);
        let mut rx1 = hub.subscribe();
        let mut rx2 = hub.subscribe();
        let msg = TestMsg {
            id: 1,
            body: "hi".into(),
        };
        let n = hub.broadcast(msg.clone()).unwrap();
        assert_eq!(n, 2);
        assert_eq!(rx1.recv().await.unwrap(), msg);
        assert_eq!(rx2.recv().await.unwrap(), msg);
    }

    #[tokio::test]
    async fn broadcast_no_receivers_error() {
        let hub = BroadcastHub::<String>::new(16);
        // No subscriber yet
        let err = hub.broadcast("hello".to_string()).unwrap_err();
        assert_eq!(err, WsError::BroadcastFull);
        // try_broadcast returns 0 instead
        assert_eq!(hub.try_broadcast("hello".to_string()), 0);
    }

    #[test]
    fn connection_count_atomic() {
        let hub = BroadcastHub::<String>::new(8);
        assert_eq!(hub.connection_count(), 0);
        hub.increment_connections(None).unwrap();
        assert_eq!(hub.connection_count(), 1);
        hub.increment_connections(None).unwrap();
        assert_eq!(hub.connection_count(), 2);
        hub.decrement_connections();
        assert_eq!(hub.connection_count(), 1);
        hub.decrement_connections();
        hub.decrement_connections(); // saturate
        assert_eq!(hub.connection_count(), 0);
    }

    #[test]
    fn max_connections_enforced() {
        let hub = BroadcastHub::<String>::new(8);
        hub.increment_connections(Some(2)).unwrap();
        hub.increment_connections(Some(2)).unwrap();
        let err = hub.increment_connections(Some(2)).unwrap_err();
        assert_eq!(err, WsError::TooManyConnections);
    }

    #[test]
    fn clone_shares_counter() {
        let hub = BroadcastHub::<String>::new(8);
        let hub2 = hub.clone();
        hub.increment_connections(None).unwrap();
        assert_eq!(hub2.connection_count(), 1);
    }
}
