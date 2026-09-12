//! Error types for `ws-kit`.
//!
//! All WebSocket failures are represented by [`WsError`]. For token-extraction
//! specific errors see [`crate::extractor::WsAuthError`].

use thiserror::Error;

/// WebSocket error variants.
///
/// Each variant maps to a failure mode encountered during authentication,
/// connection management, or message handling.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WsError {
    /// No authentication token was provided.
    #[error("authentication token missing")]
    AuthMissing,

    /// Provided authentication token is invalid or expired.
    #[error("authentication token invalid")]
    AuthInvalid,

    /// Connection limit reached.
    #[error("too many connections")]
    TooManyConnections,

    /// Broadcast channel is full or lagged.
    #[error("broadcast channel full")]
    BroadcastFull,

    /// Incoming message could not be parsed or is semantically invalid.
    #[error("invalid message")]
    InvalidMessage,

    /// A compressed payload was malformed: bad envelope flags, corrupt
    /// deflate stream, or invalid UTF-8 for a text frame.
    #[error("malformed compressed payload")]
    Compression,

    /// A payload exceeded the configured size limit — for decompression this
    /// is the decompression-bomb guard (`CompressionConfig::max_size`).
    #[error("payload exceeds size limit")]
    PayloadTooLarge,

    /// Redis (multi-node room fan-out) transport failure. The underlying
    /// `redis::RedisError` is flattened to its display form to keep
    /// `WsError` `Clone + Eq` (0.4.0 `redis` feature).
    #[error("redis error: {0}")]
    Redis(String),

    /// Connection has been closed.
    #[error("connection closed")]
    Closed,
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

    #[test]
    fn display_messages() {
        assert_eq!(
            WsError::AuthMissing.to_string(),
            "authentication token missing"
        );
        assert_eq!(
            WsError::AuthInvalid.to_string(),
            "authentication token invalid"
        );
        assert_eq!(
            WsError::TooManyConnections.to_string(),
            "too many connections"
        );
        assert_eq!(WsError::BroadcastFull.to_string(), "broadcast channel full");
        assert_eq!(WsError::InvalidMessage.to_string(), "invalid message");
        assert_eq!(
            WsError::Compression.to_string(),
            "malformed compressed payload"
        );
        assert_eq!(
            WsError::PayloadTooLarge.to_string(),
            "payload exceeds size limit"
        );
        assert_eq!(
            WsError::Redis("down".to_string()).to_string(),
            "redis error: down"
        );
        assert_eq!(WsError::Closed.to_string(), "connection closed");
    }

    #[test]
    fn clone_eq() {
        let e = WsError::AuthMissing;
        assert_eq!(e.clone(), WsError::AuthMissing);
        assert_ne!(WsError::AuthMissing, WsError::Closed);
    }
}
