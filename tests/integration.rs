use std::time::Duration;

use serde::{Deserialize, Serialize};
use ws_kit::{
    codec::{Codec, JsonCodec},
    config::WsConfig,
    error::WsError,
    extractor::{TokenExtractor, TokenSourceKind},
    hub::BroadcastHub,
    room::RoomManager,
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
    assert_eq!(WsError::AuthMissing.to_string(), "authentication token missing");
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
    r.join("alice");
    assert!(r.contains("alice"));
    assert_eq!(r.participant_count(), 1);
    r.leave("alice");
    assert_eq!(r.participant_count(), 0);
}

#[tokio::test]
async fn room_isolation() {
    let m = RoomManager::new();
    let r1 = m.get_or_create("r1");
    let r2 = m.get_or_create("r2");
    let mut rx1 = r1.subscribe();
    let mut rx2 = r2.subscribe();
    r1.broadcast("msg1".to_string()).unwrap();
    assert_eq!(rx1.recv().await.unwrap(), "msg1");
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
    let req = Request::builder().uri("/ws?token=ignored").body(()).unwrap();
    let (mut parts, _) = req.into_parts();
    parts.headers = headers;
    let ex = TokenExtractor::default();
    assert_eq!(
        ex.extract_token(&parts, "token=ignored"),
        Some("axum_token".to_string())
    );
}
