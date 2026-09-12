//! Application-layer payload compression.
//!
//! ## Honest layer assessment: why this is NOT `permessage-deflate`
//!
//! The right layer for WebSocket compression is the **handshake extension**
//! (`permessage-deflate`, RFC 7692): it compresses at the frame boundary,
//! shares context across frames, and is negotiated by the server echoing
//! `Sec-WebSocket-Extensions` during the upgrade.
//!
//! **The underlying stack cannot do that today.** As of the versions this
//! crate builds against — `tokio-tungstenite` 0.29 (what `axum` 0.8 pulls)
//! and `tungstenite` 0.29 — `permessage-deflate` is *not implemented*;
//! tungstenite merely *parses* the `Sec-WebSocket-Extensions` header for
//! handshake validation and offers no extension hook a library could wire
//! a negotiation into. Shipping a hand-rolled RFC 7692 extension on top of
//! a transport that owns the framing would mean re-implementing the
//! context-takeover state machine against frame internals we do not
//! control — the wrong layer and a correctness hazard.
//!
//! So the `compression` feature provides an **application-layer** codec
//! instead, and says so plainly:
//!
//! - Payloads are DEFLATE-compressed (RFC 1951, raw) via `flate2` with the
//!   pure-Rust `miniz_oxide` backend — the crate's `#![forbid(unsafe_code)]`
//!   posture is preserved end to end.
//! - Compressed payloads travel inside ordinary **binary frames** with a
//!   1-byte envelope (see [`FrameCompressor`]). Both endpoints must run
//!   ws-kit and opt in — **browsers cannot use this**: they only speak
//!   `permessage-deflate`. This is for Rust-to-Rust links (ws-kit server ↔
//!   ws-kit client/proxy/worker).
//! - "Negotiation" is out-of-band configuration (both sides build with the
//!   feature and construct a compressor). For integrators who signal the
//!   capability in a custom handshake header, [`extension_accepted`]
//!   matches the [`EXTENSION_NAME`] token case-insensitively in a
//!   comma-separated header value. When a tungstenite/axum handshake-level
//!   implementation lands upstream, [`CompressionConfig`] is what gets
//!   wired through — the config struct is deliberately transport-shaped.
//!
//! ## Security posture
//!
//! - **Decompression-bomb guard**: [`FrameCompressor::decompress`] refuses
//!   output larger than [`CompressionConfig::max_size`] (checked *during*
//!   inflation via `Read::take`, so a 10 GiB bomb never materializes in
//!   memory) and refuses identity payloads above the same cap.
//! - **Fail-closed envelopes**: unknown flag bits are rejected, corrupt
//!   deflate streams error ([`WsError::Compression`]), and a text frame
//!   restored from non-UTF-8 bytes errors rather than lossy-coercing.
//! - Compressed payloads ride `Frame::Binary` — the crate-wide rule that
//!   binary is never parsed as text/JSON applies to them too.
//!
//! # Example
//!
//! ```
//! use ws_kit::codec::Frame;
//! use ws_kit::compression::{CompressionConfig, FrameCompressor};
//!
//! let c = FrameCompressor::new(CompressionConfig::default());
//! let frame = Frame::from("a very repetitive payload ".repeat(50));
//! let wire = c.compress_frame(frame.clone()).unwrap();
//! // Wire is always a Binary frame with the envelope prefix:
//! assert!(wire.is_binary());
//! let restored = c.decompress_frame(wire).unwrap();
//! assert_eq!(restored, frame);
//! ```

use std::io::{Read, Write};

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;

use crate::codec::Frame;
use crate::error::WsError;

/// Capability token for out-of-band negotiation in a custom handshake
/// header. **Not** `permessage-deflate` — see the module docs for why this
/// is an application-layer codec.
pub const EXTENSION_NAME: &str = "x-ws-kit-deflate";

/// Envelope flag: payload is raw-DEFLATE (RFC 1951); unset = identity.
const FLAG_COMPRESSED: u8 = 0x01;
/// Envelope flag: original frame was [`Frame::Text`] (restores the kind).
const FLAG_TEXT: u8 = 0x02;
/// All defined flag bits; anything else fails closed.
const FLAG_KNOWN: u8 = FLAG_COMPRESSED | FLAG_TEXT;

/// Compression settings.
///
/// Transport-shaped so it can be wired through to a future handshake-level
/// implementation; today it configures [`FrameCompressor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressionConfig {
    /// DEFLATE level, 0 (store) – 9 (best). Default 6.
    pub level: u8,
    /// Payloads shorter than this are sent as identity inside the envelope
    /// (deflate overhead beats the savings on tiny payloads). Default 64.
    pub min_size: usize,
    /// Decompression-bomb guard: decompressed (or identity) payloads larger
    /// than this are rejected with [`WsError::PayloadTooLarge`]. Default
    /// 1 MiB.
    pub max_size: usize,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            level: 6,
            min_size: 64,
            max_size: 1024 * 1024,
        }
    }
}

impl CompressionConfig {
    /// Default settings.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Encodes/decodes [`Frame`]s into compressed wire form.
///
/// Wire format (inside a `Frame::Binary`, big-endian irrelevant — single
/// byte header):
///
/// ```text
/// +-------+---------------------------------+
/// | flags | payload                         |
/// +-------+---------------------------------+
///   bit 0: payload is raw-DEFLATE compressed (else identity)
///   bit 1: original frame was Text (else Binary)
/// ```
///
/// Cloning is cheap; share one compressor across tasks (`&self` API).
#[derive(Debug, Clone)]
pub struct FrameCompressor {
    config: CompressionConfig,
}

impl FrameCompressor {
    /// A compressor with the given settings.
    pub fn new(config: CompressionConfig) -> Self {
        Self { config }
    }

    /// A compressor with [`CompressionConfig::default`] settings.
    pub fn with_defaults() -> Self {
        Self::new(CompressionConfig::default())
    }

    /// Active settings.
    pub fn config(&self) -> &CompressionConfig {
        &self.config
    }

    /// Compress a payload into the envelope form.
    ///
    /// Payloads below [`CompressionConfig::min_size`] — or payloads deflate
    /// cannot shrink (already-compressed data) — are stored as identity;
    /// the envelope still marks them so the peer decodes either way.
    pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>, WsError> {
        let mut flags = 0u8;
        let body: Vec<u8> = if data.len() >= self.config.min_size {
            let mut encoder = DeflateEncoder::new(
                Vec::with_capacity(data.len() / 2),
                Compression::new(u32::from(self.config.level)),
            );
            encoder.write_all(data).map_err(|_| WsError::Compression)?;
            let compressed = encoder.finish().map_err(|_| WsError::Compression)?;
            // Only "compress" when it actually helps; incompressible input
            // would otherwise grow on the wire.
            if compressed.len() < data.len() {
                flags |= FLAG_COMPRESSED;
                compressed
            } else {
                data.to_vec()
            }
        } else {
            data.to_vec()
        };

        let mut out = Vec::with_capacity(body.len() + 1);
        out.push(flags);
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Decode an envelope produced by [`FrameCompressor::compress`],
    /// returning the original payload bytes.
    ///
    /// Enforces the decompression-bomb guard during inflation.
    pub fn decompress(&self, enveloped: &[u8]) -> Result<Vec<u8>, WsError> {
        let (&flags, body) = enveloped.split_first().ok_or(WsError::Compression)?;
        if flags & !FLAG_KNOWN != 0 {
            // Unknown bits = newer/speaking-another-protocol peer: fail
            // closed instead of misinterpreting bytes.
            return Err(WsError::Compression);
        }
        if flags & FLAG_COMPRESSED == 0 {
            if body.len() > self.config.max_size {
                return Err(WsError::PayloadTooLarge);
            }
            return Ok(body.to_vec());
        }
        let mut decoder = DeflateDecoder::new(body).take(self.config.max_size as u64 + 1);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|_| WsError::Compression)?;
        if out.len() > self.config.max_size {
            return Err(WsError::PayloadTooLarge);
        }
        Ok(out)
    }

    /// Compress a [`Frame`] into a wire-ready [`Frame::Binary`] envelope.
    ///
    /// `Frame::Binary` payloads pass through as-is bytes; `Frame::Text`
    /// payloads are compressed from and restored to their exact UTF-8
    /// bytes — never re-validated through lossy conversion.
    pub fn compress_frame(&self, frame: Frame) -> Result<Frame, WsError> {
        let mut flags = 0u8;
        if frame.is_text() {
            flags |= FLAG_TEXT;
        }
        let payload = self.compress(frame.as_bytes())?;
        let mut out = Vec::with_capacity(payload.len() + 1);
        out.push(flags);
        out.extend_from_slice(&payload);
        Ok(Frame::Binary(out.into()))
    }

    /// Restore a [`Frame`] from a [`FrameCompressor::compress_frame`]
    /// envelope. Text frames come back as [`Frame::Text`] (invalid UTF-8 is
    /// a hard error — no lossy coercion), binary as [`Frame::Binary`].
    pub fn decompress_frame(&self, frame: Frame) -> Result<Frame, WsError> {
        let bytes = frame.into_bytes();
        let (&flags, body) = bytes.split_first().ok_or(WsError::Compression)?;
        if flags & !FLAG_KNOWN != 0 {
            return Err(WsError::Compression);
        }
        let payload = self.decompress(body)?;
        if flags & FLAG_TEXT != 0 {
            let s = String::from_utf8(payload).map_err(|_| WsError::Compression)?;
            Ok(Frame::Text(s))
        } else {
            Ok(Frame::Binary(payload.into()))
        }
    }
}

/// True when a handshake header value (e.g. the integrator's custom
/// capability header) offers [`EXTENSION_NAME`].
///
/// Matching is case-insensitive, token-exact, comma-separated — the same
/// shape as `Sec-WebSocket-Extensions`. Absent header → `false`.
///
/// ```
/// use ws_kit::compression::{extension_accepted, EXTENSION_NAME};
///
/// assert!(extension_accepted(Some("x-ws-kit-deflate")));
/// assert!(extension_accepted(Some("permessage-deflate, X-WS-KIT-DEFLATE")));
/// assert!(!extension_accepted(Some("permessage-deflate")));
/// assert!(!extension_accepted(None));
/// ```
pub fn extension_accepted(header: Option<&str>) -> bool {
    header.is_some_and(|value| {
        value
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case(EXTENSION_NAME))
    })
}

/// The value a client/server puts in its capability header to offer
/// compression. Alias of [`EXTENSION_NAME`] for call-site readability.
pub fn offer_extension() -> &'static str {
    EXTENSION_NAME
}

// Tests exercise failure paths and invariants directly; unwrap/expect,
// slicing, and panicking asserts are acceptable here — violations
// surface as test failures, not production panics.
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::panic
)]
#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn repetitive(n: usize) -> Vec<u8> {
        // Highly compressible: 64-byte ramp repeated.
        let pattern: Vec<u8> = (0..=63u8).collect();
        pattern.repeat(n / 64 + 1).into_iter().take(n).collect()
    }

    #[test]
    fn roundtrip_text_and_binary() {
        let c = FrameCompressor::with_defaults();

        let text = Frame::from("chat line ".repeat(40));
        let wire = c.compress_frame(text.clone()).unwrap();
        assert!(wire.is_binary());
        let restored = c.decompress_frame(wire).unwrap();
        assert_eq!(restored, text);
        assert!(restored.is_text());

        let bin = Frame::from(repetitive(4096));
        let wire = c.compress_frame(bin.clone()).unwrap();
        let restored = c.decompress_frame(wire).unwrap();
        assert_eq!(restored, bin);
        assert!(restored.is_binary());
    }

    #[test]
    fn small_payloads_stay_identity() {
        let c = FrameCompressor::with_defaults(); // min_size 64
        let small = b"tiny".to_vec();
        let env = c.compress(&small).unwrap();
        assert_eq!(env[0] & FLAG_COMPRESSED, 0, "below min_size: identity");
        assert_eq!(&env[1..], &small[..]);
        assert_eq!(c.decompress(&env).unwrap(), small);
    }

    #[test]
    fn repetitive_payload_compresses_smaller() {
        let c = FrameCompressor::with_defaults();
        let data = repetitive(4096);
        let env = c.compress(&data).unwrap();
        assert_eq!(env[0] & FLAG_COMPRESSED, FLAG_COMPRESSED);
        assert!(
            env.len() < data.len() / 4,
            "repetitive data should crush: env={} raw={}",
            env.len(),
            data.len()
        );
        assert_eq!(c.decompress(&env).unwrap(), data);
    }

    #[test]
    fn incompressible_payload_falls_back_to_identity() {
        let c = FrameCompressor::with_defaults();
        // 4 KiB of well-shuffled pseudo-random bytes: deflate cannot shrink.
        let mut data = vec![0u8; 4096];
        let mut x: u64 = 0x9E3779B97F4A7C15;
        for slot in data.iter_mut() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *slot = (x >> 32) as u8;
        }
        let env = c.compress(&data).unwrap();
        assert_eq!(env[0] & FLAG_COMPRESSED, 0, "must not grow on the wire");
        assert_eq!(c.decompress(&env).unwrap(), data);
    }

    #[test]
    fn empty_payload_roundtrips() {
        let c = FrameCompressor::with_defaults();
        let env = c.compress(&[]).unwrap();
        assert!(c.decompress(&env).unwrap().is_empty());
        let wire = c.compress_frame(Frame::from("")).unwrap();
        assert_eq!(c.decompress_frame(wire).unwrap(), Frame::from(""));
    }

    #[test]
    fn bomb_guard_caps_decompression() {
        let small = FrameCompressor::new(CompressionConfig {
            level: 6,
            min_size: 64,
            max_size: 1024,
        });
        let bomb_input = vec![0u8; 1 << 20]; // 1 MiB of zeros
        let env = small.compress(&bomb_input).unwrap();
        // Compresses fine; decompressing against the 1 KiB cap is refused
        // *after* taking at most cap+1 bytes from the stream.
        assert_eq!(
            small.decompress(&env).unwrap_err(),
            WsError::PayloadTooLarge
        );

        // Identity payloads over the cap are refused too.
        let big_identity = small.compress(&vec![7u8; 2048]).unwrap();
        assert_eq!(
            small.decompress(&big_identity).unwrap_err(),
            WsError::PayloadTooLarge
        );
    }

    #[test]
    fn unknown_flags_fail_closed() {
        let c = FrameCompressor::with_defaults();
        let err = c.decompress(&[0b1000_0000, 1, 2, 3]).unwrap_err();
        assert_eq!(err, WsError::Compression);
        let err = c
            .decompress_frame(Frame::from(vec![0b0000_0100u8, 1]))
            .unwrap_err();
        assert_eq!(err, WsError::Compression);
        // Empty envelope: no flags byte at all.
        assert_eq!(c.decompress(&[]).unwrap_err(), WsError::Compression);
        assert_eq!(
            c.decompress_frame(Frame::from(Bytes::new())).unwrap_err(),
            WsError::Compression
        );
    }

    #[test]
    fn corrupt_deflate_stream_is_an_error() {
        let c = FrameCompressor::with_defaults();
        let garbage_env = [FLAG_COMPRESSED, 0xDE, 0xAD, 0xBE, 0xEF];
        assert_eq!(
            c.decompress(&garbage_env).unwrap_err(),
            WsError::Compression
        );
    }

    #[test]
    fn text_frame_with_invalid_utf8_flag_rejected() {
        let c = FrameCompressor::with_defaults();
        // Hand-crafted envelope claiming Text with non-UTF-8 identity bytes.
        let evil = [FLAG_TEXT, 0xFF, 0xFE, 0xFF];
        assert_eq!(
            c.decompress_frame(Frame::from(evil.to_vec())).unwrap_err(),
            WsError::Compression
        );
    }

    #[test]
    fn multibyte_utf8_text_survives_exactly() {
        let c = FrameCompressor::with_defaults();
        let text = Frame::from("héllo wörld — ünïcode 🌐 ".repeat(20));
        let restored = c
            .decompress_frame(c.compress_frame(text.clone()).unwrap())
            .unwrap();
        assert_eq!(restored, text);
    }

    #[test]
    fn negotiation_tokens() {
        assert!(extension_accepted(Some(EXTENSION_NAME)));
        assert!(extension_accepted(Some("X-WS-KIT-DEFLATE")));
        assert!(extension_accepted(Some(
            " permessage-deflate, x-ws-kit-deflate "
        )));
        assert!(!extension_accepted(Some("permessage-deflate")));
        assert!(!extension_accepted(Some("x-ws-kit-deflate2")));
        assert!(!extension_accepted(Some("")));
        assert!(!extension_accepted(None));
        assert_eq!(offer_extension(), "x-ws-kit-deflate");
    }

    #[test]
    fn config_level_bounds_and_accessors() {
        let cfg = CompressionConfig {
            level: 9,
            ..CompressionConfig::default()
        };
        let c = FrameCompressor::new(cfg.clone());
        assert_eq!(c.config(), &cfg);
        assert_eq!(CompressionConfig::new(), CompressionConfig::default());
    }
}
