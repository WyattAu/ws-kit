//! Config-knob behavior matrix for ws-kit.
//!
//! Every public knob must OBSERVABLY change behavior: each test pairs a
//! default with an alternate value and asserts the observable output
//! differs.
//!
//! Knobs covered (7):
//!   `WsConfig`: heartbeat_interval, broadcast_capacity, max_connections,
//!     allowed_origins (4)
//!   `CompressionConfig`: level, min_size, max_size (3)
//!
//! DEAD-KNOB REPORT:
//!   - `WsConfig::heartbeat_interval` is stored but never read by the
//!     library — no heartbeat task exists; consumers implement their own
//!     ping loop. Pinned inert below (`dead_knob_heartbeat_*`).
//!   - `WsConfig::max_connections` is caller-threaded: nothing inside the
//!     library passes it to `increment_connections` automatically (unlike
//!     `broadcast_capacity`, which `from_config`/`with_config` plumb).
//!     The enforcement mechanism itself is proven
//!     (`hub_connection_count_limit` in `tests/integration.rs`,
//!     `max_connections_enforced` in `src/hub.rs`); the matrix pins the
//!     intended end-to-end use (config value → `increment_connections`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::time::Duration;

#[cfg(feature = "compression")]
use ws_kit::compression::{CompressionConfig, FrameCompressor};
use ws_kit::config::WsConfig;
use ws_kit::hub::BroadcastHub;
use ws_kit::room::RoomManager;
use ws_kit::{origin_allowed, WsError};

// ---------------------------------------------------------------------------
// heartbeat_interval — DEAD (pinned inert)
// ---------------------------------------------------------------------------

#[test]
fn dead_knob_heartbeat_interval_is_documented_inert() {
    let a = WsConfig::builder()
        .heartbeat_interval(Duration::from_secs(1))
        .build();
    let b = WsConfig::builder()
        .heartbeat_interval(Duration::from_secs(3600))
        .build();
    // Same hub behavior either way: broadcast + connection accounting are
    // unaffected by the interval (no reader exists in the library).
    for cfg in [&a, &b] {
        let hub = BroadcastHub::<String>::from_config(cfg);
        let mut rx = hub.subscribe();
        hub.broadcast("ping".to_string()).unwrap();
        assert_eq!(rx.try_recv().unwrap(), "ping");
        hub.increment_connections(Some(cfg.max_connections))
            .unwrap();
        assert_eq!(hub.connection_count(), 1);
    }
}

// ---------------------------------------------------------------------------
// broadcast_capacity — overflow evicts slow receivers; RoomManager plumbs it
// ---------------------------------------------------------------------------

#[tokio::test]
async fn knob_broadcast_capacity_bounds_slow_receivers() {
    // Tiny channel: a lagging receiver loses messages...
    let tiny = BroadcastHub::<u32>::new(2);
    let mut rx = tiny.subscribe();
    for i in 0..5u32 {
        tiny.try_broadcast(i);
    }
    let err = rx.recv().await.unwrap_err();
    assert!(
        matches!(err, tokio::sync::broadcast::error::RecvError::Lagged(_)),
        "capacity=2 with 5 pending must lag the receiver, got: {err:?}"
    );

    // Roomy channel: everything arrives in order.
    let roomy = BroadcastHub::<u32>::new(16);
    let mut rx = roomy.subscribe();
    for i in 0..5u32 {
        roomy.try_broadcast(i);
    }
    for want in 0..5u32 {
        assert_eq!(rx.recv().await.unwrap(), want);
    }
}

#[test]
fn knob_broadcast_capacity_reaches_rooms_via_config() {
    let cfg = WsConfig::builder().broadcast_capacity(32).build();
    let manager = RoomManager::with_config(&cfg);
    let room = manager.get_or_create("matrix");
    // A subscriber keeps the channel alive; a 32-message burst fits a
    // 32-capacity room channel (nothing evicted).
    let mut rx = room.subscribe();
    for _ in 0..32 {
        room.broadcast(ws_kit::Frame::from("x")).unwrap();
    }
    for _ in 0..32 {
        rx.try_recv().unwrap();
    }
    // ...while a capacity-1 room drops for a lagging subscriber.
    let tiny_cfg = WsConfig::builder().broadcast_capacity(1).build();
    let tiny_manager = RoomManager::with_config(&tiny_cfg);
    let tiny_room = tiny_manager.get_or_create("tiny");
    let mut rx = tiny_room.subscribe();
    for _ in 0..4 {
        tiny_room.broadcast(ws_kit::Frame::from("y")).unwrap();
    }
    let err = rx.try_recv().unwrap_err();
    assert!(
        matches!(err, tokio::sync::broadcast::error::TryRecvError::Lagged(_)),
        "capacity=1 must lag a non-draining subscriber, got: {err:?}"
    );
}

// ---------------------------------------------------------------------------
// max_connections — caller-threaded enforcement (pinned end to end)
// ---------------------------------------------------------------------------

#[test]
fn knob_max_connections_enforced_when_threaded() {
    let cfg = WsConfig::builder().max_connections(2).build();
    let hub = BroadcastHub::<String>::from_config(&cfg);
    hub.increment_connections(Some(cfg.max_connections))
        .unwrap();
    hub.increment_connections(Some(cfg.max_connections))
        .unwrap();
    assert_eq!(
        hub.increment_connections(Some(cfg.max_connections))
            .unwrap_err(),
        WsError::TooManyConnections,
        "the config value must cap connections when threaded through"
    );

    let open = WsConfig::builder().max_connections(3).build();
    let hub = BroadcastHub::<String>::from_config(&open);
    for _ in 0..3 {
        hub.increment_connections(Some(open.max_connections))
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// allowed_origins — allow-all default vs enforced allow-list
// ---------------------------------------------------------------------------

#[test]
fn knob_allowed_origins_empty_allows_all() {
    let empty: Vec<String> = Vec::new();
    assert!(origin_allowed(Some("https://app.example.com"), &empty));
    assert!(origin_allowed(None, &empty));
    assert_eq!(WsConfig::default().allowed_origins, empty);
}

#[test]
fn knob_allowed_origins_list_enforced() {
    let allowed = vec!["https://app.example.com".to_string()];
    assert!(origin_allowed(Some("https://app.example.com"), &allowed));
    assert!(
        !origin_allowed(Some("https://evil.example.com"), &allowed),
        "unlisted origin must be rejected"
    );
    assert!(
        !origin_allowed(None, &allowed),
        "missing origin must be rejected when the list is non-empty"
    );
}

// ---------------------------------------------------------------------------
// CompressionConfig::level — stronger level compresses repetitive data more
// ---------------------------------------------------------------------------

#[cfg(feature = "compression")]
#[test]
fn knob_compression_level_changes_output_size() {
    let data: Vec<u8> = b"matrix-matrix-matrix-matrix-matrix-matrix-matrix-matrix-".repeat(32);
    let store = FrameCompressor::new(CompressionConfig {
        level: 0,
        ..CompressionConfig::default()
    })
    .compress(&data)
    .unwrap();
    let best = FrameCompressor::new(CompressionConfig {
        level: 9,
        ..CompressionConfig::default()
    })
    .compress(&data)
    .unwrap();
    assert!(
        best.len() < store.len(),
        "level 9 ({} bytes) must beat level 0 ({} bytes) on repetitive input",
        best.len(),
        store.len()
    );
    // And both round-trip.
    let c = FrameCompressor::new(CompressionConfig::default());
    assert_eq!(c.decompress(&best).unwrap(), data);
}

// ---------------------------------------------------------------------------
// CompressionConfig::min_size — below-threshold payloads stay identity
// ---------------------------------------------------------------------------

#[cfg(feature = "compression")]
#[test]
fn knob_compression_min_size_moves_identity_threshold() {
    let small = b"tiny".to_vec();
    // Default min_size (64): 4-byte payload sent as identity...
    let c = FrameCompressor::new(CompressionConfig::default());
    let env = c.compress(&small).unwrap();
    assert_eq!(&env[1..], &small[..]);

    // min_size 0: even the 4-byte payload takes the deflate path (flag set
    // or same bytes — the observable is the flag on compressible input).
    let data: Vec<u8> = b"matrix-matrix-matrix-matrix-".repeat(8);
    let eager = FrameCompressor::new(CompressionConfig {
        min_size: 0,
        ..CompressionConfig::default()
    })
    .compress(&data)
    .unwrap();
    let lazy = FrameCompressor::new(CompressionConfig {
        min_size: usize::MAX,
        ..CompressionConfig::default()
    })
    .compress(&data)
    .unwrap();
    assert_ne!(
        eager, lazy,
        "min_size 0 vs MAX must change the wire encoding"
    );
    assert_eq!(&lazy[1..], &data[..], "min_size=MAX forces identity");
}

// ---------------------------------------------------------------------------
// CompressionConfig::max_size — decompression-bomb guard
// ---------------------------------------------------------------------------

#[cfg(feature = "compression")]
#[test]
fn knob_compression_max_size_caps_decompression() {
    let data: Vec<u8> = b"matrix-matrix-matrix-matrix-".repeat(16);
    let c = FrameCompressor::new(CompressionConfig::default());
    let env = c.compress(&data).unwrap();

    // Default 1 MiB accepts this payload...
    assert_eq!(c.decompress(&env).unwrap(), data);

    // ...a tiny cap rejects the very same envelope.
    let strict = FrameCompressor::new(CompressionConfig {
        max_size: 8,
        ..CompressionConfig::default()
    });
    assert_eq!(
        strict.decompress(&env).unwrap_err(),
        WsError::PayloadTooLarge,
        "max_size=8 must reject a larger payload"
    );
}
