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

    /// Connection has been closed.
    #[error("connection closed")]
    Closed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages() {
        assert_eq!(WsError::AuthMissing.to_string(), "authentication token missing");
        assert_eq!(WsError::AuthInvalid.to_string(), "authentication token invalid");
        assert_eq!(WsError::TooManyConnections.to_string(), "too many connections");
        assert_eq!(WsError::BroadcastFull.to_string(), "broadcast channel full");
        assert_eq!(WsError::InvalidMessage.to_string(), "invalid message");
        assert_eq!(WsError::Closed.to_string(), "connection closed");
    }

    #[test]
    fn clone_eq() {
        let e = WsError::AuthMissing;
        assert_eq!(e.clone(), WsError::AuthMissing);
        assert_ne!(WsError::AuthMissing, WsError::Closed);
    }
}
