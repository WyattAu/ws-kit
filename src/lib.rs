#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! # ws-kit
//!
//! Generic authenticated typed WebSocket toolkit.
//!
//! Provides:
//! - [`config::WsConfig`] — heartbeat, broadcast capacity, connection limits with builder.
//! - [`hub::BroadcastHub`] — typed `broadcast::Sender<T>` wrapper with connection counting, generic over `T: Clone + Serialize`.
//! - [`room::RoomManager`] / [`room::Room`] — DashMap-backed room isolation with participant tracking.
//! - [`extractor::TokenExtractor`] — configurable token extraction (`Authorization: Bearer`, `?token=`, `?access_token=`, `Cookie`).
//! - [`codec::Codec`] / [`codec::JsonCodec`] — JSON encode/decode via serde.
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
//! ## Features
//!
//! - `std` (default): enables `std` support for `thiserror`.
//! - `axum` (default): enables `axum` WebSocket and `tower` deps, and `TokenExtractor::extract_token(&Parts, _)`
//! - `tracing`: enables tracing instrumentation.

pub mod codec;
pub mod config;
pub mod error;
pub mod extractor;
pub mod hub;
pub mod room;

// Re-exports for ergonomic use
pub use codec::{Codec, JsonCodec};
pub use config::{WsConfig, WsConfigBuilder};
pub use error::WsError;
pub use extractor::{TokenExtractor, TokenSourceKind, WsAuthError};
pub use hub::BroadcastHub;
pub use room::{Room, RoomManager};
