//! Multi-node room fan-out over Redis pub/sub (`redis` feature, off by
//! default).
//!
//! ## Topology
//!
//! Each application node keeps its local rooms in an ordinary
//! [`RoomManager`]. A [`RedisRoomRegistry`] wraps that local manager plus a
//! multiplexed Redis connection:
//!
//! ```text
//!        node A                                node B
//!  ┌─────────────────┐                  ┌─────────────────┐
//!  │ RoomManager (A) │                  │ RoomManager (B) │
//!  │   room "lobby"  │                  │   room "lobby"  │
//!  └───┬────────┬────┘                  └───┬────────┬────┘
//!      │        │                           │        │
//!   local    PUBLISH ws-kit:room:lobby   local    PUBLISH
//!   sinks          │                     sinks        │
//!      │        ┌───▼───────────────────────┼────────┘ │
//!      │        │            Redis          │          │
//!      │        └───┬───────────────────────▼──────────┘
//!      │            │        (pub/sub, no persistence)
//!   every node PSUBSCRIBEs ws-kit:room:* → delivers each message
//!   to the LOCAL room with that id (if the node hosts one)
//! ```
//!
//! - **Broadcast** ([`RedisRoomRegistry::broadcast`]): delivers to the
//!   local room's sinks **and** `PUBLISH`es once to the room's channel.
//! - **Subscribe**: each node runs a subscriber loop
//!   ([`RedisRoomRegistry::run_subscriber`], spawn it once at startup)
//!   `PSUBSCRIBE ws-kit:room:*`. Every received message is applied to the
//!   local room of that id — *if* the node hosts it. Remote messages never
//!   materialize new rooms.
//! - **Self-echo suppression**: each registry carries a random 16-byte node
//!   id; the envelope is stamped with it and a node ignores its own
//!   publishes (otherwise a node's local receivers would get every message
//!   twice: once directly, once via the Redis echo).
//!
//! ## Delivery semantics — read this before trusting it
//!
//! Redis pub/sub is **at-most-once, best-effort**: no persistence, no
//! replay, no acknowledgement. If a node is disconnected, restarting, or a
//! subscriber briefly lags, messages published in that window are **gone**
//! — this crate does *not* provide at-least-once delivery, and makes no
//! persistence claim. Use it for ephemeral fan-out (presence, chat, live
//! notifications) where losing the odd message on failover beats carrying
//! infrastructure; for at-least-once, switch the transport to Redis Streams
//! or a log — the [`RoomRegistry`] trait keeps that swap local to this
//! module.
//!
//! ## Security
//!
//! The Redis channel is a **trusted bus**: any process that can PUBLISH to
//! `ws-kit:room:*` can inject messages into any room on any node, and any
//! process that can SUBSCRIBE can read them. Restrict access (Redis ACLs,
//! network isolation), use `rediss://` (TLS) for untrusted links, and treat
//! channel payloads as you would any network input — this module validates
//! the envelope (`version` byte, kind tag, UTF-8 for text) and fails closed
//! on anything unexpected.
//!
//! # Example
//!
//! ```no_run
//! # async fn demo() -> Result<(), ws_kit::error::WsError> {
//! use std::sync::Arc;
//! use ws_kit::codec::Frame;
//! use ws_kit::room::{RoomRegistry, RoomManager};
//! use ws_kit::redis_rooms::RedisRoomRegistry;
//!
//! let registry = Arc::new(RedisRoomRegistry::connect("redis://127.0.0.1:6379").await?);
//!
//! // One subscriber task per node, at startup:
//! let sub = Arc::clone(&registry);
//! tokio::spawn(async move {
//!     let _ = sub.run_subscriber().await; // ends only on redis failure
//! });
//!
//! // Local room + fan-out to every node hosting "lobby":
//! registry.get_or_create("lobby").join(1, "alice".into());
//! registry.broadcast("lobby", Frame::from("hello all nodes")).await?;
//!
//! // The plain RoomManager API still works for local-only concerns:
//! let local: &RoomManager = registry.local();
//! assert!(local.get("lobby").is_some());
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use redis::aio::{ConnectionLike, MultiplexedConnection};
use redis::{Client, Cmd};

use crate::codec::Frame;
use crate::error::WsError;
use crate::room::{RoomManager, RoomRegistry};

/// Default Redis channel prefix: `ws-kit:room:{room_id}`.
pub const DEFAULT_CHANNEL_PREFIX: &str = "ws-kit:room:";

/// Envelope protocol version byte (fail-closed on mismatch).
const ENVELOPE_VERSION: u8 = 1;

/// Payload kind tags.
const KIND_TEXT: u8 = 0;
const KIND_BINARY: u8 = 1;

/// Total envelope header length: version (1) + node id (16) + kind (1).
const HEADER_LEN: usize = 18;

/// Best-effort unique node id: two independently-seeded `RandomState`
/// hashers give 128 random-ish bits. Not cryptographic — collisions are
/// merely improbable and would only re-enable self-echo duplication for a
/// specific message, never cross-node corruption (the id is opaque to
/// receivers).
fn generate_node_id() -> [u8; 16] {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut a = RandomState::new().build_hasher();
    let mut b = RandomState::new().build_hasher();
    a.write_u64(0x77_73_2d_6b_69_74_01);
    b.write_u64(std::process::id() as u64);
    let hi = a.finish();
    let lo = b.finish();
    let mut id = [0u8; 16];
    let (left, right) = id.split_at_mut(8);
    left.copy_from_slice(&hi.to_le_bytes());
    right.copy_from_slice(&lo.to_le_bytes());
    id
}

/// Encode a frame into the pub/sub envelope:
/// `version(1) | node_id(16) | kind(1) | payload`.
fn encode_envelope(node_id: &[u8; 16], frame: &Frame) -> Vec<u8> {
    let payload = frame.as_bytes();
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.push(ENVELOPE_VERSION);
    out.extend_from_slice(node_id);
    out.push(match frame {
        Frame::Text(_) => KIND_TEXT,
        Frame::Binary(_) => KIND_BINARY,
    });
    out.extend_from_slice(payload);
    out
}

/// Decode an envelope, keeping the publisher's node id so receivers can
/// drop self-echoes. Fails closed on wrong version, unknown kind, or
/// truncated header.
fn decode_envelope(bytes: &[u8]) -> Result<([u8; 16], Frame), WsError> {
    if bytes.len() < HEADER_LEN || bytes.first() != Some(&ENVELOPE_VERSION) {
        return Err(WsError::InvalidMessage);
    }
    let mut node_id = [0u8; 16];
    let id_bytes = bytes.get(1..17).ok_or(WsError::InvalidMessage)?;
    node_id.copy_from_slice(id_bytes);
    let kind = bytes.get(17).copied().ok_or(WsError::InvalidMessage)?;
    let payload = bytes.get(HEADER_LEN..).ok_or(WsError::InvalidMessage)?;
    let frame = match kind {
        KIND_TEXT => {
            let s = String::from_utf8(payload.to_vec()).map_err(|_| WsError::InvalidMessage)?;
            Frame::Text(s)
        }
        KIND_BINARY => Frame::Binary(payload.to_vec().into()),
        _ => return Err(WsError::InvalidMessage),
    };
    Ok((node_id, frame))
}

/// Multi-node [`crate::room::RoomRegistry`]: local [`RoomManager`] + Redis
/// pub/sub fan-out. See the [module docs](self) for topology and delivery
/// semantics.
///
/// The generic parameter is the publish connection (redis'
/// `ConnectionLike`); it defaults to the production
/// `MultiplexedConnection` and exists so tests can substitute a mock —
/// in application code always write the default `RedisRoomRegistry`.
pub struct RedisRoomRegistry<C = MultiplexedConnection> {
    local: RoomManager,
    client: Client,
    publish: Arc<tokio::sync::Mutex<C>>,
    prefix: String,
    node_id: [u8; 16],
}

impl RedisRoomRegistry<MultiplexedConnection> {
    /// Connect with the default channel prefix
    /// ([`DEFAULT_CHANNEL_PREFIX`]).
    ///
    /// # Errors
    ///
    /// [`WsError::Redis`] when the URL is invalid or the server is
    /// unreachable.
    pub async fn connect(url: &str) -> Result<Self, WsError> {
        Self::connect_with_prefix(url, DEFAULT_CHANNEL_PREFIX).await
    }

    /// Connect with a custom channel prefix (rooms of unrelated
    /// deployments can share one Redis by differing prefixes).
    pub async fn connect_with_prefix(url: &str, prefix: &str) -> Result<Self, WsError> {
        let client = Client::open(url.to_string()).map_err(redis_err)?;
        let publish = client
            .get_multiplexed_async_connection()
            .await
            .map_err(redis_err)?;
        Ok(Self::from_parts(
            RoomManager::new(),
            client,
            publish,
            prefix,
        ))
    }
}

impl<C> RedisRoomRegistry<C>
where
    C: ConnectionLike + Clone + Send + 'static,
{
    /// Assemble a registry from a local manager, a Redis client (used by
    /// [`Self::run_subscriber`]), and an existing publish connection.
    ///
    /// Public to let integrators and tests supply custom
    /// `ConnectionLike` implementations (the trait-mocked publish path);
    /// prefer [`Self::connect`] in application code.
    pub fn from_parts(
        local: RoomManager,
        client: Client,
        publish: C,
        prefix: impl Into<String>,
    ) -> Self {
        Self {
            local,
            client,
            publish: Arc::new(tokio::sync::Mutex::new(publish)),
            prefix: prefix.into(),
            node_id: generate_node_id(),
        }
    }
}

impl<C> RedisRoomRegistry<C> {
    /// The local half of the registry — the ordinary [`RoomManager`] API
    /// for joins, leaves, local subscription, and cleanup.
    pub fn local(&self) -> &RoomManager {
        &self.local
    }

    /// Channel prefix in use.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Channel name for a room id.
    pub fn channel_name(&self, room_id: &str) -> String {
        format!("{}{}", self.prefix, room_id)
    }

    /// Room id carried by a channel name, if it matches the prefix.
    pub fn room_id_from_channel<'a>(&self, channel: &'a str) -> Option<&'a str> {
        channel.strip_prefix(self.prefix.as_str())
    }

    /// This registry's random publisher id (self-echo suppression).
    pub fn node_id(&self) -> [u8; 16] {
        self.node_id
    }
}

impl<C> RedisRoomRegistry<C>
where
    C: ConnectionLike + Clone + Send + 'static,
{
    /// Broadcast a frame to a room across the fleet.
    ///
    /// 1. Fans out to the **local** room's receivers (creating the room if
    ///    this node does not host it yet — matching
    ///    [`RoomRegistry::get_or_create`] semantics).
    /// 2. `PUBLISH`es the envelope to `prefix + room_id` for every other
    ///    node.
    ///
    /// Returns the number of **local** receivers reached. An empty local
    /// room is *not* an error here (unlike [`crate::room::Room::broadcast`]):
    /// other nodes may still hold receivers, so `Ok(0)` just means "no local
    /// subscriber". A Redis failure surfaces as [`WsError::Redis`].
    ///
    /// **At-most-once**: see the module docs — delivery on other nodes is
    /// best-effort.
    pub async fn broadcast(
        &self,
        room_id: &str,
        frame: impl Into<Frame>,
    ) -> Result<usize, WsError> {
        let frame = frame.into();
        // Local first: room-local receivers must not wait on the network.
        let local_n = match self.local.get_or_create(room_id).broadcast(frame.clone()) {
            Ok(n) => n,
            // No local receivers is fine — the PUBLISH still fans out.
            Err(WsError::BroadcastFull) => 0,
            Err(e) => return Err(e),
        };
        self.publish(&frame, room_id).await?;
        Ok(local_n)
    }

    /// Publish one frame to `room_id`'s channel (no local delivery).
    pub async fn publish(&self, frame: &Frame, room_id: &str) -> Result<(), WsError> {
        let envelope = encode_envelope(&self.node_id, frame);
        let mut conn = self.publish.lock().await;
        let mut cmd = Cmd::new();
        cmd.arg("PUBLISH")
            .arg(self.channel_name(room_id))
            .arg(envelope);
        conn.req_packed_command(&cmd).await.map_err(redis_err)?;
        Ok(())
    }

    /// Apply a raw pub/sub envelope to the local registry: decode it, drop
    /// self-echoes, and deliver to the local room **only if this node
    /// hosts it** (remote traffic never materializes rooms).
    ///
    /// Returns `Ok(Some(n))` with the local receiver count when delivered,
    /// `Ok(None)` when the message was a self-echo or for an unhosted room,
    /// and the decode error for malformed envelopes.
    pub fn apply_remote(&self, room_id: &str, envelope: &[u8]) -> Result<Option<usize>, WsError> {
        let (publisher, frame) = decode_envelope(envelope)?;
        if publisher == self.node_id {
            return Ok(None);
        }
        match self.local.get(room_id) {
            Some(room) => match room.broadcast(frame) {
                Ok(n) => Ok(Some(n)),
                Err(WsError::BroadcastFull) => Ok(None),
                Err(e) => Err(e),
            },
            None => Ok(None),
        }
    }
}

impl<C> RoomRegistry for RedisRoomRegistry<C>
where
    C: ConnectionLike + Clone + Send + 'static,
{
    // The local half implements the room-collection trait: a
    // RedisRoomRegistry can drop in anywhere a RoomManager did, while the
    // cross-node fan-out is the inherent `broadcast`/`run_subscriber` API.
    fn get_or_create(&self, room_id: &str) -> std::sync::Arc<crate::room::Room> {
        self.local.get_or_create(room_id)
    }

    fn get(&self, room_id: &str) -> Option<std::sync::Arc<crate::room::Room>> {
        self.local.get(room_id)
    }

    fn remove(&self, room_id: &str) -> Option<std::sync::Arc<crate::room::Room>> {
        self.local.remove(room_id)
    }

    fn room_count(&self) -> usize {
        self.local.room_count()
    }

    fn room_ids(&self) -> Vec<String> {
        self.local.room_ids()
    }

    fn total_participants(&self) -> usize {
        self.local.total_participants()
    }

    fn cleanup(&self) -> usize {
        self.local.cleanup()
    }

    fn cleanup_empty(&self) -> usize {
        self.local.cleanup_empty()
    }
}

impl RedisRoomRegistry<MultiplexedConnection> {
    /// Run the subscriber loop **on the current task**: `PSUBSCRIBE
    /// prefix*` and apply every message to the local registry until the
    /// Redis connection dies.
    ///
    /// Spawn it once per node at startup (see the module example); to
    /// survive Redis restarts wrap it in your own retry/backoff — the loop
    /// intentionally returns instead of hiding reconnect policy.
    ///
    /// # Errors
    ///
    /// [`WsError::Redis`] on subscription/transport failure. Malformed
    /// envelopes from the channel are dropped (fail-closed), not fatal.
    pub async fn run_subscriber(&self) -> Result<(), WsError> {
        let mut pubsub = self.client.get_async_pubsub().await.map_err(redis_err)?;
        let pattern = format!("{}*", self.prefix);
        pubsub.psubscribe(pattern).await.map_err(redis_err)?;
        let mut stream = pubsub.on_message();
        while let Some(msg) = futures_util::StreamExt::next(&mut stream).await {
            let channel = msg.get_channel_name().to_string();
            let payload = msg.get_payload_bytes().to_vec();
            if let Some(room_id) = self.room_id_from_channel(&channel) {
                // Fail-closed on malformed envelopes: count as nothing,
                // keep the loop alive.
                let _ = self.apply_remote(room_id, &payload);
            }
        }
        Err(WsError::Closed)
    }
}

fn redis_err(e: redis::RedisError) -> WsError {
    WsError::Redis(e.to_string())
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
    use std::sync::Mutex;

    /// In-memory publish connection: records PUBLISH payloads so the
    /// fan-out path is testable without Redis (the trait-mocked pubsub
    /// strategy; full loop is covered by the #[ignore] live fixture).
    #[derive(Clone, Default)]
    struct MockPublishConn {
        published: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl ConnectionLike for MockPublishConn {
        fn req_packed_command<'a>(
            &'a mut self,
            cmd: &'a Cmd,
        ) -> redis::RedisFuture<'a, redis::Value> {
            let packed = cmd.get_packed_command();
            self.published.lock().unwrap().push(packed);
            Box::pin(async { Ok(redis::Value::Nil) })
        }

        fn req_packed_commands<'a>(
            &'a mut self,
            _cmd: &'a redis::Pipeline,
            _offset: usize,
            _count: usize,
        ) -> redis::RedisFuture<'a, Vec<redis::Value>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn get_db(&self) -> i64 {
            0
        }
    }

    type PublishedLog = Arc<Mutex<Vec<Vec<u8>>>>;

    fn test_registry() -> (RedisRoomRegistry<MockPublishConn>, PublishedLog) {
        let conn = MockPublishConn::default();
        let published = Arc::clone(&conn.published);
        // Client::open only parses the URL — no I/O, safe for mocks.
        let client = Client::open("redis://127.0.0.1:6379").unwrap();
        let reg =
            RedisRoomRegistry::from_parts(RoomManager::new(), client, conn, DEFAULT_CHANNEL_PREFIX);
        (reg, published)
    }

    #[tokio::test]
    async fn broadcast_fans_out_locally_and_publishes() {
        let (reg, published) = test_registry();
        let mut rx = reg.local().get_or_create("lobby").subscribe();
        let n = reg
            .broadcast("lobby", Frame::from("hi fleet"))
            .await
            .unwrap();
        assert_eq!(n, 1, "one local receiver");
        assert_eq!(
            rx.recv().await.unwrap(),
            Frame::from("hi fleet"),
            "local sink got the frame directly"
        );
        let sent = published.lock().unwrap();
        assert_eq!(sent.len(), 1, "exactly one PUBLISH");
        let packed = &sent[0];
        let packed_str = String::from_utf8_lossy(packed).to_string();
        assert!(
            packed_str.contains("ws-kit:room:lobby"),
            "PUBLISH targets the room channel: {packed_str:?}"
        );
        // The full envelope (version | node id | kind | payload) rides the
        // same command.
        let expected = encode_envelope(&reg.node_id(), &Frame::from("hi fleet"));
        assert!(
            packed
                .windows(expected.len())
                .any(|w| w == expected.as_slice()),
            "PUBLISH carries the exact envelope"
        );
    }

    #[tokio::test]
    async fn broadcast_creates_local_room_on_demand() {
        let (reg, _published) = test_registry();
        assert!(reg.local().get("fresh").is_none());
        // No local receivers: still Ok, still publishes.
        let n = reg.broadcast("fresh", Frame::from("x")).await.unwrap();
        assert_eq!(n, 0);
        assert!(reg.local().get("fresh").is_some(), "room materialized");
    }

    #[tokio::test]
    async fn remote_envelope_reaches_hosted_room() {
        let (reg, _published) = test_registry();
        let mut rx = reg.local().get_or_create("lobby").subscribe();

        // A peer publishes; craft the envelope as their node would.
        let peer_id = [7u8; 16];
        let frame = Frame::from("from peer");
        let envelope = encode_envelope(&peer_id, &frame);
        let delivered = reg.apply_remote("lobby", &envelope).unwrap();
        assert_eq!(delivered, Some(1));
        assert_eq!(rx.recv().await.unwrap(), frame);
    }

    #[tokio::test]
    async fn remote_binary_envelope_roundtrips() {
        let (reg, _published) = test_registry();
        let mut rx = reg.local().get_or_create("bin").subscribe();
        let frame = Frame::Binary(Bytes::from_static(b"\x00\xff\x10"));
        let envelope = encode_envelope(&[9u8; 16], &frame);
        reg.apply_remote("bin", &envelope).unwrap();
        let got = rx.recv().await.unwrap();
        assert!(got.is_binary());
        assert_eq!(got.as_bytes(), b"\x00\xff\x10");
    }

    #[tokio::test]
    async fn self_echo_is_dropped() {
        let (reg, _published) = test_registry();
        let mut rx = reg.local().get_or_create("lobby").subscribe();
        let envelope = encode_envelope(&reg.node_id(), &Frame::from("my own publish"));
        assert_eq!(reg.apply_remote("lobby", &envelope).unwrap(), None);
        use tokio::time::{timeout, Duration};
        assert!(timeout(Duration::from_millis(20), rx.recv()).await.is_err());
    }

    #[tokio::test]
    async fn remote_traffic_never_materializes_rooms() {
        let (reg, _published) = test_registry();
        let envelope = encode_envelope(&[3u8; 16], &Frame::from("anyone there?"));
        assert_eq!(reg.apply_remote("ghost", &envelope).unwrap(), None);
        assert!(reg.local().get("ghost").is_none());
    }

    #[test]
    fn malformed_envelopes_fail_closed() {
        let (reg, _published) = test_registry();
        // Truncated header
        assert_eq!(
            reg.apply_remote("r", &[0u8; 10]).unwrap_err(),
            WsError::InvalidMessage
        );
        // Empty
        assert_eq!(
            reg.apply_remote("r", &[]).unwrap_err(),
            WsError::InvalidMessage
        );
        // Wrong version byte
        let mut evil = encode_envelope(&[0u8; 16], &Frame::from("v0"));
        evil[0] = 99;
        assert_eq!(
            reg.apply_remote("r", &evil).unwrap_err(),
            WsError::InvalidMessage
        );
        // Unknown kind tag
        let mut evil = encode_envelope(&[0u8; 16], &Frame::from("bad kind"));
        evil[17] = 42;
        assert_eq!(
            reg.apply_remote("r", &evil).unwrap_err(),
            WsError::InvalidMessage
        );
        // Text frame with non-UTF-8 payload
        let mut evil = encode_envelope(&[0u8; 16], &Frame::from("ok"));
        evil[17] = KIND_TEXT;
        evil[18..].copy_from_slice(&[0xFF, 0xFE]);
        assert_eq!(
            reg.apply_remote("r", &evil).unwrap_err(),
            WsError::InvalidMessage
        );
    }

    #[test]
    fn channel_names_roundtrip() {
        let (reg, _published) = test_registry();
        assert_eq!(reg.channel_name("lobby"), "ws-kit:room:lobby");
        assert_eq!(reg.room_id_from_channel("ws-kit:room:lobby"), Some("lobby"));
        // Room ids may themselves contain ':' — prefix split is exact.
        assert_eq!(reg.room_id_from_channel("ws-kit:room:a:b:c"), Some("a:b:c"));
        assert_eq!(reg.room_id_from_channel("other:room:x"), None);
        assert_eq!(reg.room_id_from_channel("ws-kit:room:"), Some(""));
    }

    #[test]
    fn custom_prefix_changes_channels() {
        let conn = MockPublishConn::default();
        let client = Client::open("redis://127.0.0.1:6379").unwrap();
        let reg: RedisRoomRegistry<MockPublishConn> =
            RedisRoomRegistry::from_parts(RoomManager::new(), client, conn, "tenant-a/rooms:");
        assert_eq!(reg.channel_name("lobby"), "tenant-a/rooms:lobby");
        assert_eq!(
            reg.room_id_from_channel("tenant-a/rooms:lobby"),
            Some("lobby")
        );
        assert_eq!(reg.prefix(), "tenant-a/rooms:");
    }

    #[test]
    fn envelope_decode_roundtrip_preserves_frame_kind() {
        let node_id = [1u8; 16];
        for frame in [
            Frame::from("hello".to_string()),
            Frame::Binary(Bytes::from_static(&[0, 1, 2, 255])),
        ] {
            let env = encode_envelope(&node_id, &frame);
            let (got_id, got_frame) = decode_envelope(&env).unwrap();
            assert_eq!(got_id, node_id);
            assert_eq!(got_frame, frame);
        }
    }

    #[test]
    fn node_ids_are_distinct_per_registry() {
        let (a, _) = test_registry();
        let (b, _) = test_registry();
        assert_ne!(a.node_id(), b.node_id());
    }

    /// Live fixture — requires a Redis server (docker: `docker run -p
    /// 6379:6379 redis:7`). Run: `cargo test --features redis --test
    /// redis_rooms -- --ignored`. The full two-node pub/sub loop cannot be
    /// exercised by the in-memory publish mock; these cover it.
    #[tokio::test]
    #[ignore = "requires a live Redis server (docker run -p 6379:6379 redis:7)"]
    async fn live_two_nodes_fan_out_over_pubsub() {
        let node_a = Arc::new(
            RedisRoomRegistry::connect("redis://127.0.0.1:6379")
                .await
                .unwrap(),
        );
        let node_b = Arc::new(
            RedisRoomRegistry::connect("redis://127.0.0.1:6379")
                .await
                .unwrap(),
        );
        assert_ne!(node_a.node_id(), node_b.node_id());

        // Subscriber loops on both nodes.
        let sub_a = Arc::clone(&node_a);
        let sub_b = Arc::clone(&node_b);
        tokio::spawn(async move { sub_a.run_subscriber().await });
        tokio::spawn(async move { sub_b.run_subscriber().await });

        // Both nodes host the room; A subscribes locally.
        let mut rx_a = node_a.local().get_or_create("live").subscribe();
        let _rx_b = node_b.local().get_or_create("live").subscribe();

        // Wait for PSUBSCRIBE to be established on both nodes.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        // B broadcasts: A must receive it via pub/sub.
        node_b
            .broadcast("live", Frame::from("cross-node hello"))
            .await
            .unwrap();
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), rx_a.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, Frame::from("cross-node hello"));

        // Self-echo suppression: A broadcasts to a room it hosts; its own
        // subscriber must NOT double-deliver.
        node_a
            .broadcast("live", Frame::from("from a itself"))
            .await
            .unwrap();
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), rx_a.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got, Frame::from("from a itself"));
        // No duplicate within a grace window:
        let dup = tokio::time::timeout(std::time::Duration::from_millis(300), rx_a.recv()).await;
        assert!(dup.is_err(), "self-echo must be suppressed");
    }

    /// Live fixture: binary frames survive the pub/sub hop.
    #[tokio::test]
    #[ignore = "requires a live Redis server (docker run -p 6379:6379 redis:7)"]
    async fn live_binary_frame_crosses_nodes() {
        let node_a = Arc::new(
            RedisRoomRegistry::connect("redis://127.0.0.1:6379")
                .await
                .unwrap(),
        );
        let node_b = Arc::new(
            RedisRoomRegistry::connect("redis://127.0.0.1:6379")
                .await
                .unwrap(),
        );
        let sub_a = Arc::clone(&node_a);
        tokio::spawn(async move { sub_a.run_subscriber().await });
        let mut rx_a = node_a.local().get_or_create("bin-live").subscribe();
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        node_b
            .broadcast(
                "bin-live",
                Frame::Binary(Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef])),
            )
            .await
            .unwrap();
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), rx_a.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(got.is_binary());
        assert_eq!(got.as_bytes(), &[0xde, 0xad, 0xbe, 0xef]);
    }
}
