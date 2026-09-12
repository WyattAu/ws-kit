// Tests exercise pub/sub delivery directly; unwrap/expect, slicing, and
// panicking asserts are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

//! Live Redis room fan-out tests — the full pub/sub loop cannot run on the
//! in-memory `ConnectionLike` mock used by the unit tests in
//! `src/redis_rooms.rs`, so these are fixture-gated like the rest of the
//! monorepo:
//!
//! ```sh
//! docker run -d --name ws-kit-redis -p 6379:6379 redis:7
//! cargo test --features redis --test redis_rooms -- --ignored --nocapture
//! ```

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use ws_kit::codec::Frame;
use ws_kit::redis_rooms::RedisRoomRegistry;

const REDIS_URL: &str = "redis://127.0.0.1:6379";

async fn node() -> Arc<RedisRoomRegistry> {
    Arc::new(RedisRoomRegistry::connect(REDIS_URL).await.unwrap())
}

fn spawn_subscriber(node: &Arc<RedisRoomRegistry>) {
    let sub = Arc::clone(node);
    tokio::spawn(async move {
        let _ = sub.run_subscriber().await;
    });
}

async fn wait_subscribed() {
    // Give PSUBSCRIBE time to establish. (A readiness handshake would be
    // nicer; a grace window matches the monorepo's other pub/sub fixtures.)
    tokio::time::sleep(Duration::from_millis(250)).await;
}

#[tokio::test]
#[ignore = "requires a live Redis server (docker run -p 6379:6379 redis:7)"]
async fn live_text_frame_crosses_nodes() {
    let node_a = node().await;
    let node_b = node().await;
    assert_ne!(node_a.node_id(), node_b.node_id());
    spawn_subscriber(&node_a);
    spawn_subscriber(&node_b);

    let mut rx_a = node_a.local().get_or_create("live-text").subscribe();
    let _rx_b = node_b.local().get_or_create("live-text").subscribe();
    wait_subscribed().await;

    node_b
        .broadcast("live-text", Frame::from("hello from b"))
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), rx_a.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, Frame::from("hello from b"));
}

#[tokio::test]
#[ignore = "requires a live Redis server (docker run -p 6379:6379 redis:7)"]
async fn live_binary_frame_crosses_nodes() {
    let node_a = node().await;
    let node_b = node().await;
    spawn_subscriber(&node_a);
    let mut rx_a = node_a.local().get_or_create("live-bin").subscribe();
    wait_subscribed().await;

    let payload = Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef, 0x00]);
    node_b
        .broadcast("live-bin", Frame::Binary(payload.clone()))
        .await
        .unwrap();
    let got = tokio::time::timeout(Duration::from_secs(5), rx_a.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(got.is_binary(), "binary stays binary across the hop");
    assert_eq!(got.as_bytes(), &payload[..]);
}

#[tokio::test]
#[ignore = "requires a live Redis server (docker run -p 6379:6379 redis:7)"]
async fn live_broadcast_fans_out_locally_and_across() {
    // The publishing node's local receivers get the message exactly once
    // (direct delivery; self-echo suppressed), while node B also gets it.
    let node_a = node().await;
    let node_b = node().await;
    spawn_subscriber(&node_a);
    spawn_subscriber(&node_b);

    let mut rx_a = node_a.local().get_or_create("live-both").subscribe();
    let mut rx_b = node_b.local().get_or_create("live-both").subscribe();
    wait_subscribed().await;

    let n = node_a
        .broadcast("live-both", Frame::from("once only"))
        .await
        .unwrap();
    assert_eq!(n, 1, "one local receiver on the publishing node");

    let got_a = tokio::time::timeout(Duration::from_secs(5), rx_a.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_a, Frame::from("once only"));
    let got_b = tokio::time::timeout(Duration::from_secs(5), rx_b.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_b, Frame::from("once only"));

    // Self-echo suppression: no duplicate on the publisher.
    let dup = tokio::time::timeout(Duration::from_millis(300), rx_a.recv()).await;
    assert!(dup.is_err(), "publisher must not see its own echo");
}

#[tokio::test]
#[ignore = "requires a live Redis server (docker run -p 6379:6379 redis:7)"]
async fn live_remote_traffic_never_materializes_rooms() {
    let node_a = node().await;
    let node_b = node().await;
    spawn_subscriber(&node_a);
    spawn_subscriber(&node_b);
    wait_subscribed().await;

    // B broadcasts to a room only B knows about; A must not grow a room.
    node_b
        .broadcast("only-on-b", Frame::from("anyone?"))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(node_a.local().get("only-on-b").is_none());
    assert!(node_b.local().get("only-on-b").is_some());
}

#[tokio::test]
#[ignore = "requires a live Redis server (docker run -p 6379:6379 redis:7)"]
async fn live_custom_prefix_isolates_tenants() {
    let node_a = Arc::new(
        RedisRoomRegistry::connect_with_prefix(REDIS_URL, "ws-kit-test:tenant-x:")
            .await
            .unwrap(),
    );
    let node_b = Arc::new(
        RedisRoomRegistry::connect_with_prefix(REDIS_URL, "ws-kit-test:tenant-y:")
            .await
            .unwrap(),
    );
    spawn_subscriber(&node_a);
    spawn_subscriber(&node_b);
    let mut rx_a = node_a.local().get_or_create("shared-id").subscribe();
    let _rx_b = node_b.local().get_or_create("shared-id").subscribe();
    wait_subscribed().await;

    // B (tenant-y) publishes to "shared-id": A (tenant-x) listens on a
    // different channel and must receive nothing.
    node_b
        .broadcast("shared-id", Frame::from("for tenant y only"))
        .await
        .unwrap();
    let quiet = tokio::time::timeout(Duration::from_millis(500), rx_a.recv()).await;
    assert!(quiet.is_err(), "tenant prefixes must isolate fan-out");
}
