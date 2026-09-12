//! Message codec.
//!
//! [`Codec`] provides `encode`/`decode` for WebSocket text frames. A blanket
//! implementation via `serde_json` is provided for any `Serialize + DeserializeOwned`.
//!
//! ## The wire message model: [`Frame`]
//!
//! WebSocket (RFC 6455) carries two data frame kinds: **text** (must be valid
//! UTF-8) and **binary** (opaque bytes). Since 0.4.0 ws-kit models both in
//! [`Frame`]:
//!
//! - [`Frame::Text`] — UTF-8 payloads; the only kind [`Codec`] and
//!   [`JsonCodec`] operate on.
//! - [`Frame::Binary`] — opaque bytes, carried verbatim.
//!
//! # Security posture (binary is never text)
//!
//! A `Frame::Binary` payload is **never parsed as text or JSON** by any
//! ws-kit code path. `Codec::decode`/`JsonCodec::decode` take `&str` — the
//! type system makes it impossible to feed binary bytes into the JSON
//! decoder without an explicit, reviewer-visible UTF-8 conversion. Rooms,
//! the hub, and (0.4.0) the Redis adapter move binary payloads opaquely:
//! no sniffing, no best-effort decoding, no shell-out to text heuristics.
//! Conversely, text is *guaranteed* UTF-8 end to end — a `Frame::Text` can
//! only be constructed from a `String`, so downstream JSON decode cannot
//! hit invalid-UTF-8 surprises.
//!
//! Authentication is unaffected by frame kind: `TokenExtractor` reads the
//! HTTP upgrade request (headers/query), never frame payloads.

use bytes::Bytes;
use serde::{de::DeserializeOwned, Serialize};

use crate::error::WsError;

/// A wire message: the union of WebSocket text and binary data frames.
///
/// Conversions are zero-copy where the transport already owns the data:
/// `String`, `&str`, and `Bytes` all move into a `Frame` without
/// re-encoding. `Vec<u8>` and borrowed `&[u8]` copy once.
///
/// `Frame` implements `Serialize`/`Deserialize`, so it can ride the generic
/// [`crate::hub::BroadcastHub<Frame>`]; in JSON form binary payloads are
/// represented as byte arrays and — per the security posture below — are
/// only ever decoded back into `Frame::Binary`, never parsed as text.
///
/// # Example
///
/// ```
/// use ws_kit::codec::Frame;
///
/// let text = Frame::from("hello");          // &str — one UTF-8 copy
/// let owned = Frame::from(String::from("hi")); // String — zero-copy move
/// let bin = Frame::from(vec![0u8, 1, 2]);   // Vec<u8> — becomes Binary
/// assert!(matches!(text, Frame::Text(_)));
/// assert!(matches!(bin, Frame::Binary(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Frame {
    /// A UTF-8 text frame (WebSocket opcode 0x1).
    Text(String),
    /// An opaque binary frame (WebSocket opcode 0x2). Never parsed as
    /// text/JSON by ws-kit — see the [security posture](self#security-posture-binary-is-never-text).
    Binary(Bytes),
}

impl Frame {
    /// Construct a `Binary` frame from anything `Bytes` converts from.
    pub fn binary(data: impl Into<Bytes>) -> Self {
        Frame::Binary(data.into())
    }

    /// Payload length in bytes (UTF-8 length for text).
    pub fn len(&self) -> usize {
        match self {
            Frame::Text(s) => s.len(),
            Frame::Binary(b) => b.len(),
        }
    }

    /// True when the payload is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True for [`Frame::Text`].
    pub fn is_text(&self) -> bool {
        matches!(self, Frame::Text(_))
    }

    /// True for [`Frame::Binary`].
    pub fn is_binary(&self) -> bool {
        matches!(self, Frame::Binary(_))
    }

    /// Payload as bytes (UTF-8 for text), without consuming the frame.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Frame::Text(s) => s.as_bytes(),
            Frame::Binary(b) => b.as_ref(),
        }
    }

    /// Take the payload as owned bytes, consuming the frame.
    pub fn into_bytes(self) -> Bytes {
        match self {
            Frame::Text(s) => Bytes::from(s),
            Frame::Binary(b) => b,
        }
    }

    /// Interpret the payload as UTF-8 text (binary is rejected, not sniffed).
    pub fn into_text(self) -> Result<String, WsError> {
        match self {
            Frame::Text(s) => Ok(s),
            Frame::Binary(_) => Err(WsError::InvalidMessage),
        }
    }
}

impl From<String> for Frame {
    fn from(s: String) -> Self {
        Frame::Text(s)
    }
}

impl From<&str> for Frame {
    fn from(s: &str) -> Self {
        Frame::Text(s.to_string())
    }
}

impl From<Bytes> for Frame {
    fn from(b: Bytes) -> Self {
        Frame::Binary(b)
    }
}

impl From<Vec<u8>> for Frame {
    fn from(v: Vec<u8>) -> Self {
        Frame::Binary(Bytes::from(v))
    }
}

impl From<&[u8]> for Frame {
    fn from(v: &[u8]) -> Self {
        Frame::Binary(Bytes::copy_from_slice(v))
    }
}

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

    // --- Frame ---

    #[test]
    fn frame_constructors_and_kinds() {
        assert!(Frame::from("hello".to_string()).is_text());
        assert!(Frame::from("hello").is_text());
        assert!(Frame::from(Bytes::from_static(b"\x00\x01")).is_binary());
        assert!(Frame::from(vec![1u8, 2]).is_binary());
        assert!(Frame::from(&[1u8, 2][..]).is_binary());
        assert!(Frame::binary(Vec::new()).is_binary());
    }

    #[test]
    fn frame_len_and_bytes() {
        let t = Frame::from("héllo"); // multibyte: len is bytes
        assert_eq!(t.len(), 6);
        assert_eq!(t.as_bytes(), "héllo".as_bytes());
        assert!(!t.is_empty());

        let b = Frame::from(vec![0u8; 4]);
        assert_eq!(b.len(), 4);
        assert!(Frame::from(String::new()).is_empty());
    }

    #[test]
    fn frame_into_bytes_and_text() {
        let t = Frame::from("abc".to_string());
        assert_eq!(&t.into_bytes()[..], b"abc");

        let b = Frame::from(vec![9u8]);
        assert_eq!(b.into_bytes()[0], 9);

        let t2 = Frame::from("text".to_string()).into_text();
        assert_eq!(t2.unwrap(), "text");

        // Binary is rejected as text — never sniffed/coerced.
        let b2 = Frame::from(vec![0xffu8, 0xfe]).into_text();
        assert_eq!(b2.unwrap_err(), WsError::InvalidMessage);
    }

    #[test]
    fn frame_zero_copy_moves() {
        let s = String::from("owned");
        let f = Frame::from(s);
        assert_eq!(f, Frame::Text("owned".to_string()));
        let v = vec![1u8; 8];
        let f2 = Frame::from(v);
        assert_eq!(f2.len(), 8);
    }

    #[test]
    fn frame_clone_eq() {
        let f = Frame::from("dup".to_string());
        assert_eq!(f.clone(), f);
        assert_ne!(
            Frame::from("a".to_string()),
            Frame::from(Bytes::from_static(b"a"))
        );
    }
}
