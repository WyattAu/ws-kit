//! Message codec.
//!
//! [`Codec`] provides `encode`/`decode` for WebSocket text frames. A blanket
//! implementation via `serde_json` is provided for any `Serialize + DeserializeOwned`.

use serde::{de::DeserializeOwned, Serialize};

use crate::error::WsError;

/// Trait for encoding/decoding WebSocket messages.
///
/// The default blanket implementation uses JSON via `serde_json`.
///
/// # Example
///
/// ```
/// use ws_kit::codec::Codec;
/// use serde::{Serialize, Deserialize};
///
/// #[derive(Serialize, Deserialize, PartialEq, Debug)]
/// struct Msg { text: String }
///
/// let msg = Msg { text: "hello".into() };
/// let s = msg.encode();
/// let decoded = Msg::decode(&s).unwrap();
/// assert_eq!(msg, decoded);
/// ```
pub trait Codec: Sized {
    /// Encode `self` into a JSON string.
    ///
    /// This implementation serializes via `serde_json::to_string`.
    /// Panics are avoided — if serialization fails the error is surfaced
    /// through `WsError::InvalidMessage` in the `Result` variant. The
    /// infallible `encode` variant below unwraps for convenience; use
    /// [`Codec::encode_result`] when you need error handling.
    fn encode(&self) -> String
    where
        Self: Serialize,
    {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Encode with error handling.
    fn encode_result(&self) -> Result<String, WsError>
    where
        Self: Serialize,
    {
        serde_json::to_string(self).map_err(|_| WsError::InvalidMessage)
    }

    /// Decode `Self` from a JSON string.
    fn decode(s: &str) -> Result<Self, WsError>
    where
        Self: DeserializeOwned,
    {
        serde_json::from_str(s).map_err(|_| WsError::InvalidMessage)
    }
}

/// Blanket implementation for all serde types.
impl<T> Codec for T where T: Serialize + DeserializeOwned {}

/// JSON codec helper for types that don't want to implement [`Codec`] directly
/// or where you need associated functions without trait dispatch.
pub struct JsonCodec;

impl JsonCodec {
    /// Encode a value to JSON string.
    pub fn encode<T: Serialize>(value: &T) -> Result<String, WsError> {
        serde_json::to_string(value).map_err(|_| WsError::InvalidMessage)
    }

    /// Decode a value from JSON string.
    pub fn decode<T: DeserializeOwned>(s: &str) -> Result<T, WsError> {
        serde_json::from_str(s).map_err(|_| WsError::InvalidMessage)
    }

    /// Encode pretty-printed JSON (for debugging).
    pub fn encode_pretty<T: Serialize>(value: &T) -> Result<String, WsError> {
        serde_json::to_string_pretty(value).map_err(|_| WsError::InvalidMessage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    struct Payload {
        id: u32,
        msg: String,
    }

    #[test]
    fn codec_roundtrip_trait() {
        let p = Payload {
            id: 42,
            msg: "hello".into(),
        };
        let s = p.encode();
        let d = Payload::decode(&s).unwrap();
        assert_eq!(p, d);
    }

    #[test]
    fn codec_roundtrip_result() {
        let p = Payload {
            id: 1,
            msg: "world".into(),
        };
        let s = p.encode_result().unwrap();
        let d = Payload::decode(&s).unwrap();
        assert_eq!(p, d);
    }

    #[test]
    fn codec_invalid_decode() {
        let err = Payload::decode("not json").unwrap_err();
        assert_eq!(err, WsError::InvalidMessage);
    }

    #[test]
    fn json_codec_helpers() {
        let p = Payload {
            id: 7,
            msg: "test".into(),
        };
        let s = JsonCodec::encode(&p).unwrap();
        let d: Payload = JsonCodec::decode(&s).unwrap();
        assert_eq!(p, d);

        let pretty = JsonCodec::encode_pretty(&p).unwrap();
        assert!(pretty.contains('\n'));
    }

    #[test]
    fn encode_vec() {
        let v = vec![1, 2, 3];
        let s = v.encode();
        let d: Vec<i32> = Vec::decode(&s).unwrap();
        assert_eq!(v, d);
    }
}
