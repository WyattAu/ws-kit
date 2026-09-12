#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, allow(unused_attributes))]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! # ws-kit
//!
//! Generic authenticated typed WebSocket toolkit.
//!
//! Provides:
//! - [`config::WsConfig`] — heartbeat, broadcast capacity, connection limits, Origin allow-list with builder.
//! - [`hub::BroadcastHub`] — typed `broadcast::Sender<T>` wrapper with connection counting, generic over `T: Clone + Serialize`.
//! - [`room::RoomManager`] / [`room::Room`] — DashMap-backed room isolation with participant tracking, over the [`codec::Frame`] message model (text **and** binary).
//! - [`room::RoomRegistry`] — the room-collection trait; swap the in-memory manager for a multi-node backend.
//! - [`extractor::TokenExtractor`] — configurable token extraction (`Authorization: Bearer`, `?token=`, `?access_token=`, `Cookie`).
//! - [`origin_allowed_in_parts`] — Origin validation for the upgrade handshake (CSWSH defense, REQ-WSKIT-200).
//! - [`codec::Codec`] / [`codec::JsonCodec`] — JSON encode/decode via serde.
//! - [`stats::StatsRecorder`] — backpressure/traffic counters (connections, rooms, bytes/messages in/out, drops), global + per-room, optional `metrics`-facade emission.
//! - With `compression`: [`compression::FrameCompressor`] — application-layer DEFLATE for `Frame`s (not `permessage-deflate`; the module docs explain why).
//! - With `redis`: [`redis_rooms::RedisRoomRegistry`] — multi-node room fan-out over Redis pub/sub (at-most-once; the module docs explain the topology).
//!
//! ## Quick start
//!
//! ```rust
//! use ws_kit::{config::WsConfig, hub::BroadcastHub, room::RoomManager, codec::Codec};
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
//! struct ChatMsg { user: String, body: String }
//!
//! # tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
//! let hub = BroadcastHub::<ChatMsg>::new(1024);
//! let mut rx = hub.subscribe();
//! hub.broadcast(ChatMsg { user: "alice".into(), body: "hi".into() }).unwrap();
//! let msg = rx.recv().await.unwrap();
//! assert_eq!(msg.user, "alice");
//! # });
//! ```
//!
//! ## Binary messages (0.4.0)
//!
//! ```rust
//! use ws_kit::codec::Frame;
//! use ws_kit::room::RoomManager;
//!
//! # tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
//! let m = RoomManager::new();
//! let room = m.get_or_create("media");
//! let mut rx = room.subscribe();
//! room.broadcast(Frame::binary(vec![0xde_u8, 0xad, 0xbe, 0xef])).unwrap();
//! assert!(rx.recv().await.unwrap().is_binary());
//! # });
//! ```
//!
//! Binary payloads are carried opaquely — ws-kit never parses them as
//! text/JSON; [`codec::Frame::Text`] and [`codec::Frame::Binary`] are kept
//! distinct end to end. Authentication (`TokenExtractor`) operates on the
//! HTTP upgrade request and is frame-kind-agnostic.
//!
//! ## Features
//!
//! - `axum` (default): enables `axum` WebSocket and `tower` deps, and `TokenExtractor::extract_token(&Parts, _)`
//! - `compression`: application-layer DEFLATE for `Frame` payloads (see [`compression`] for the layer assessment)
//! - `redis`: multi-node room fan-out via [`redis_rooms::RedisRoomRegistry`] (at-most-once pub/sub)
//! - `metrics`: emit [`stats`] counters through the `metrics` facade
//! - `tracing`: deprecated no-op kept for 0.3.x compatibility

pub mod codec;
#[cfg(feature = "compression")]
#[cfg_attr(docsrs, doc(cfg(feature = "compression")))]
pub mod compression;
pub mod config;
mod counter;
pub mod error;
pub mod extractor;
pub mod hub;
pub mod origin;
#[cfg(feature = "redis")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis")))]
pub mod redis_rooms;
pub mod room;
pub mod stats;

// Model-checking tests for `counter` — compiled only under `--cfg loom`.
#[cfg(loom)]
mod loom_tests;

// Re-exports for ergonomic use
pub use codec::{Codec, Frame, JsonCodec};
pub use config::{WsConfig, WsConfigBuilder};
pub use error::WsError;
pub use extractor::{TokenExtractor, TokenSourceKind, WsAuthError};
pub use hub::BroadcastHub;
pub use origin::{normalize_origin, origin_allowed};
pub use room::{Room, RoomManager, RoomRegistry};
pub use stats::{RoomStats, Stats, StatsRecorder};

#[cfg(feature = "axum")]
#[cfg_attr(docsrs, doc(cfg(feature = "axum")))]
pub use origin::origin_allowed_in_parts;

#[cfg(feature = "compression")]
#[cfg_attr(docsrs, doc(cfg(feature = "compression")))]
pub use compression::{extension_accepted, CompressionConfig, FrameCompressor};

#[cfg(feature = "redis")]
#[cfg_attr(docsrs, doc(cfg(feature = "redis")))]
pub use redis_rooms::RedisRoomRegistry;
