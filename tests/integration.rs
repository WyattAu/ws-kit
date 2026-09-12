// Tests exercise hostile and failure paths directly; unwrap/expect, slicing,
// and panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use std::time::Duration;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use ws_kit::{
    codec::{Codec, Frame, JsonCodec},
    config::WsConfig,
    error::WsError,
    extractor::{TokenExtractor, TokenSourceKind},
    hub::BroadcastHub,
    room::RoomManager,
    stats::StatsRecorder,
};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
struct TestMsg {
    id: u32,
    text: String,
}

// --- Config ---

#[test]
fn config_defaults() {
    let c = WsConfig::default();
    assert_eq!(c.heartbeat_interval, Duration::from_secs(30));
    assert_eq!(c.broadcast_capacity, 1024);
    assert_eq!(c.max_connections, 1000);
}

#[test]
fn config_builder() {
    let c = WsConfig::builder()
        .heartbeat_interval(Duration::from_secs(5))
        .broadcast_capacity(2048)
        .max_connections(200)
        .build();
    assert_eq!(c.heartbeat_interval, Duration::from_secs(5));
    assert_eq!(c.broadcast_capacity, 2048);
    assert_eq!(c.max_connections, 200);
}

// --- Error ---

#[test]
fn ws_error_display() {
    assert_eq!(
        WsError::AuthMissing.to_string(),
        "authentication token missing"
    );
    assert_eq!(WsError::Closed.to_string(), "connection closed");
}

// --- Hub ---

#[tokio::test]
async fn hub_broadcast_subscribe() {
    let hub = BroadcastHub::<TestMsg>::new(16);
    let mut rx = hub.subscribe();
    let msg = TestMsg {
        id: 1,
        text: "hello".into(),
    };
    hub.broadcast(msg.clone()).unwrap();
    let got = rx.recv().await.unwrap();
    assert_eq!(got, msg);
}

#[tokio::test]
async fn hub_multiple_subscribers() {
    let hub = BroadcastHub::<String>::new(16);
    let mut rx1 = hub.subscribe();
    let mut rx2 = hub.subscribe();
    hub.broadcast("hello".to_string()).unwrap();
    assert_eq!(rx1.recv().await.unwrap(), "hello");
    assert_eq!(rx2.recv().await.unwrap(), "hello");
}

#[test]
fn hub_connection_count_limit() {
    let hub = BroadcastHub::<String>::new(8);
    hub.increment_connections(Some(2)).unwrap();
    hub.increment_connections(Some(2)).unwrap();
    assert_eq!(
        hub.increment_connections(Some(2)).unwrap_err(),
        WsError::TooManyConnections
    );
}

#[tokio::test]
async fn hub_integration_with_config() {
    let cfg = WsConfig::builder().broadcast_capacity(32).build();
    let hub = BroadcastHub::<TestMsg>::from_config(&cfg);
    let mut rx = hub.subscribe();
    hub.broadcast(TestMsg {
        id: 99,
        text: "from_config".into(),
    })
    .unwrap();
    assert_eq!(rx.recv().await.unwrap().id, 99);
}

// --- Room ---

#[test]
fn room_manager_basic() {
    let m = RoomManager::new();
    let r = m.get_or_create("general");
    r.join(1, "Alice".to_string());
    r.join(2, "Bob".to_string());
    assert!(r.contains(1));
    let mut names = r.participant_names();
    names.sort();
    assert_eq!(names, vec!["Alice".to_string(), "Bob".to_string()]);
    assert_eq!(r.participant_count(), 2);
    assert_eq!(m.total_participants(), 2);
    assert!(r.leave(2));
    assert_eq!(r.participant_count(), 1);
    r.leave(1);
    assert_eq!(m.cleanup_empty(), 1);
    assert_eq!(m.room_count(), 0);
}

#[tokio::test]
async fn room_isolation() {
    let m = RoomManager::new();
    let r1 = m.get_or_create("r1");
    let r2 = m.get_or_create("r2");
    let mut rx1 = r1.subscribe();
    let mut rx2 = r2.subscribe();
    r1.broadcast("msg1".to_string()).unwrap();
    assert_eq!(rx1.recv().await.unwrap(), Frame::from("msg1"));
    // r2 must not receive
    let res = tokio::time::timeout(Duration::from_millis(20), rx2.recv()).await;
    assert!(res.is_err());
}

#[test]
fn room_cleanup() {
    let m = RoomManager::new();
    let _r = m.get_or_create("temp");
    // No participants/receivers => empty => cleanup removes
    assert_eq!(m.cleanup(), 1);
    assert_eq!(m.room_count(), 0);
}

// --- Binary frames (0.4.0) ---

#[tokio::test]
async fn binary_frame_routes_through_hub() {
    let hub = BroadcastHub::<Frame>::new(16);
    let mut rx1 = hub.subscribe();
    let mut rx2 = hub.subscribe();
    let payload = Bytes::from_static(&[0x00, 0xff, 0x7f, 0x80]);
    let n = hub.broadcast(Frame::Binary(payload.clone())).unwrap();
    assert_eq!(n, 2);
    for rx in [&mut rx1, &mut rx2] {
        let got = rx.recv().await.unwrap();
        assert!(got.is_binary(), "hub must not re-interpret binary as text");
        assert_eq!(got.as_bytes(), &payload[..]);
    }
}

#[tokio::test]
async fn binary_frame_routes_through_room() {
    let m = RoomManager::new();
    let room = m.get_or_create("bin");
    let mut rx = room.subscribe();
    room.join(1, "alice".to_string());
    room.broadcast(vec![9u8; 300]).unwrap();
    let got = rx.recv().await.unwrap();
    assert!(got.is_binary());
    assert_eq!(got.len(), 300);
    // Text and binary interleave in arrival order on one channel.
    room.broadcast("after".to_string()).unwrap();
    assert_eq!(rx.recv().await.unwrap(), Frame::from("after"));
}

#[tokio::test]
async fn frame_into_text_rejects_binary() {
    let frame = Frame::binary(Bytes::from_static(&[0xff, 0xfe]));
    assert_eq!(frame.into_text().unwrap_err(), WsError::InvalidMessage);
}

// --- Stats (0.4.0) ---

#[tokio::test]
async fn stats_recorder_counts_connections_rooms_and_traffic() {
    let stats = StatsRecorder::new();
    stats.inc_connections();
    stats.inc_connections();
    stats.dec_connections();
    stats.record_message_in(42);

    let cfg = WsConfig::builder().broadcast_capacity(8).build();
    let manager = ws_kit::room::RoomManager::with_stats(&cfg, stats.clone());
    {
        let room = manager.get_or_create("lobby");
        let mut rx = room.subscribe();
        room.broadcast("12345".to_string()).unwrap();
        assert_eq!(rx.recv().await.unwrap(), Frame::from("12345"));
    }
    manager.remove("lobby");

    let snap = stats.snapshot();
    assert_eq!(snap.connections, 1);
    assert_eq!(snap.rooms, 0, "create/remove balanced");
    assert_eq!(snap.messages_out, 1);
    assert_eq!(snap.bytes_out, 5);
    assert_eq!(snap.messages_in, 1, "room roll-up + manual");
    assert_eq!(snap.bytes_in, 42);
    assert_eq!(snap.per_room.get("lobby").unwrap().messages_out, 1);
}

#[tokio::test]
async fn stats_record_broadcast_drops() {
    let stats = StatsRecorder::new();
    let cfg = WsConfig::default();
    let manager = ws_kit::room::RoomManager::with_stats(&cfg, stats.clone());
    let room = manager.get_or_create("dead");
    // No receivers: broadcast fails and counts as a drop.
    room.broadcast("lost".to_string()).unwrap_err();
    let snap = stats.snapshot();
    assert_eq!(snap.drops, 1);
    assert_eq!(snap.per_room.get("dead").unwrap().drops, 1);
}

// --- Extractor ---

#[test]
fn extractor_bearer_and_query() {
    let ex = TokenExtractor::default();
    // header wins
    let tok = ex.extract_from_parts(Some("Bearer hdr"), None, "token=qry");
    assert_eq!(tok, Some("hdr".to_string()));
    // fallback to query
    let tok2 = ex.extract_from_parts(None, None, "token=qry");
    assert_eq!(tok2, Some("qry".to_string()));
    // access_token
    let tok3 = ex.extract_from_parts(None, None, "access_token=secret");
    assert_eq!(tok3, Some("secret".to_string()));
}

#[test]
fn extractor_cookie() {
    let ex = TokenExtractor::new(vec![TokenSourceKind::Cookie("sess".to_string())]);
    let tok = ex.extract_from_parts(None, Some("sess=abc123; other=xyz"), "");
    assert_eq!(tok, Some("abc123".to_string()));
}

// --- Codec ---

#[test]
fn codec_json_roundtrip() {
    let msg = TestMsg {
        id: 7,
        text: "codec".into(),
    };
    let s = msg.encode();
    let decoded = TestMsg::decode(&s).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn json_codec_helper() {
    let msg = TestMsg {
        id: 1,
        text: "helper".into(),
    };
    let s = JsonCodec::encode(&msg).unwrap();
    let d: TestMsg = JsonCodec::decode(&s).unwrap();
    assert_eq!(msg, d);
}

#[test]
fn codec_invalid() {
    let err = TestMsg::decode("%%%").unwrap_err();
    assert_eq!(err, WsError::InvalidMessage);
}

#[cfg(feature = "axum")]
#[test]
fn extractor_axum_parts() {
    use http::{HeaderMap, Request};
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        "Bearer axum_token".parse().unwrap(),
    );
    let req = Request::builder()
        .uri("/ws?token=ignored")
        .body(())
        .unwrap();
    let (mut parts, _) = req.into_parts();
    parts.headers = headers;
    let ex = TokenExtractor::default();
    assert_eq!(
        ex.extract_token(&parts, "token=ignored"),
        Some("axum_token".to_string())
    );
}

// --- Origin validation (REQ-WSKIT-200/201) — axum test server ---

#[cfg(feature = "axum")]
mod origin_upgrade {
    use axum::extract::ws::WebSocketUpgrade;
    use axum::extract::State;
    use axum::http::request::Parts;
    use axum::http::StatusCode;
    use axum::response::{IntoResponse, Response};
    use axum::routing::get;
    use axum::Router;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Upgrade handler: validates Origin (REQ-WSKIT-200) BEFORE on_upgrade;
    /// a failed check short-circuits with 403 — the socket is never upgraded.
    async fn ws_handler(
        State(allowed): State<Vec<String>>,
        parts: Parts,
        ws: WebSocketUpgrade,
    ) -> Response {
        if !ws_kit::origin_allowed_in_parts(&parts, &allowed) {
            return StatusCode::FORBIDDEN.into_response();
        }
        ws.on_upgrade(|_socket| async {})
    }

    async fn spawn_ws_app(allowed_origins: Vec<String>) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .with_state(allowed_origins);
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        port
    }

    /// Raw HTTP upgrade request; returns the response status code.
    async fn handshake_status(port: u16, origin: Option<&str>) -> u16 {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let origin_line = origin
            .map(|o| format!("Origin: {o}\r\n"))
            .unwrap_or_default();
        let req = format!(
            "GET /ws HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n{origin_line}\r\n"
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        loop {
            let n = stream.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8_lossy(&buf)
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn req_wskit_200_allowed_origin_passes_upgrade() {
        let port = spawn_ws_app(vec!["https://example.com".to_string()]).await;
        assert_eq!(
            handshake_status(port, Some("https://example.com")).await,
            101
        );
    }

    #[tokio::test]
    async fn req_wskit_200_disallowed_origin_rejected_403_pre_upgrade() {
        let port = spawn_ws_app(vec!["https://example.com".to_string()]).await;
        assert_eq!(handshake_status(port, Some("https://evil.com")).await, 403);
    }

    #[tokio::test]
    async fn req_wskit_200_missing_origin_rejected_403() {
        let port = spawn_ws_app(vec!["https://example.com".to_string()]).await;
        assert_eq!(handshake_status(port, None).await, 403);
    }

    #[tokio::test]
    async fn req_wskit_200_port_normalized_match_passes() {
        let port = spawn_ws_app(vec!["https://example.com".to_string()]).await;
        // https default port 443 is omitted during normalization
        assert_eq!(
            handshake_status(port, Some("https://example.com:443")).await,
            101
        );
        // non-default ports do NOT collapse to the bare entry
        assert_eq!(
            handshake_status(port, Some("https://example.com:8443")).await,
            403
        );
    }

    #[tokio::test]
    async fn req_wskit_200_no_wildcard_suffix_match() {
        let port = spawn_ws_app(vec!["https://example.com".to_string()]).await;
        assert_eq!(
            handshake_status(port, Some("https://evil-example.com")).await,
            403
        );
        assert_eq!(
            handshake_status(port, Some("https://example.com.evil.com")).await,
            403
        );
        assert_eq!(
            handshake_status(port, Some("https://api.example.com")).await,
            403
        );
    }

    #[tokio::test]
    async fn req_wskit_201_default_config_allows_all_origins() {
        // Empty allow-list (default) = allow all, even a missing Origin —
        // documented residual risk, preserved 0.2.x behavior.
        let port = spawn_ws_app(Vec::new()).await;
        assert_eq!(
            handshake_status(port, Some("https://any-site.dev")).await,
            101
        );
        assert_eq!(handshake_status(port, None).await, 101);
    }
}
