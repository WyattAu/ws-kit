//! Configuration for WebSocket infrastructure.
//!
//! [`WsConfig`] controls heartbeat cadence, broadcast buffering, global
//! connection limits, and the Origin allow-list for the upgrade handshake
//! ([`WsConfig::allowed_origins`]). Use the builder for ergonomic
//! construction.

use std::time::Duration;

/// Configuration for WebSocket connections and broadcast channels.
///
/// # Example
///
/// ```
/// use ws_kit::config::WsConfig;
/// use std::time::Duration;
///
/// let cfg = WsConfig::builder()
///     .heartbeat_interval(Duration::from_secs(15))
///     .broadcast_capacity(2048)
///     .max_connections(5000)
///     .allow_origin("https://app.example.com")
///     .build();
/// assert_eq!(cfg.heartbeat_interval, Duration::from_secs(15));
/// assert_eq!(cfg.allowed_origins, vec!["https://app.example.com".to_string()]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsConfig {
    /// Interval between heartbeat pings.
    pub heartbeat_interval: Duration,
    /// Capacity of each broadcast channel.
    pub broadcast_capacity: usize,
    /// Maximum concurrent connections allowed (0 = unlimited).
    pub max_connections: usize,
    /// Origin allow-list for the WebSocket upgrade handshake (REQ-WSKIT-200).
    ///
    /// **WARNING — DEFAULT IS ALLOW-ALL**: an empty list (the default)
    /// accepts upgrades from ANY origin — cross-site WebSocket hijacking
    /// (CSWSH) is possible, especially when authentication rides cookies.
    /// This matches pre-0.3.0 behavior so existing users don't break; it is
    /// a documented residual risk (see THREAT-MODEL.md). Set at least the
    /// origins your own frontend uses, e.g. `https://app.example.com`.
    ///
    /// When non-empty, an upgrade whose `Origin` header is missing or not in
    /// this list must be rejected with `403` before `on_upgrade` — use
    /// `ws_kit::origin_allowed_in_parts`. Comparison is exact after
    /// normalization (lowercase scheme/host, default ports omitted); no
    /// wildcard or suffix matching in v1.
    pub allowed_origins: Vec<String>,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(30),
            broadcast_capacity: 1024,
            max_connections: 1000,
            allowed_origins: Vec::new(),
        }
    }
}

impl WsConfig {
    /// Create a new config with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a builder for [`WsConfig`].
    pub fn builder() -> WsConfigBuilder {
        WsConfigBuilder::default()
    }
}

/// Builder for [`WsConfig`].
#[derive(Debug, Clone)]
pub struct WsConfigBuilder {
    heartbeat_interval: Duration,
    broadcast_capacity: usize,
    max_connections: usize,
    allowed_origins: Vec<String>,
}

impl Default for WsConfigBuilder {
    fn default() -> Self {
        let d = WsConfig::default();
        Self {
            heartbeat_interval: d.heartbeat_interval,
            broadcast_capacity: d.broadcast_capacity,
            max_connections: d.max_connections,
            allowed_origins: d.allowed_origins,
        }
    }
}

impl WsConfigBuilder {
    /// Set heartbeat interval.
    pub fn heartbeat_interval(mut self, v: Duration) -> Self {
        self.heartbeat_interval = v;
        self
    }

    /// Set broadcast channel capacity.
    pub fn broadcast_capacity(mut self, v: usize) -> Self {
        self.broadcast_capacity = v;
        self
    }

    /// Set maximum connection limit.
    pub fn max_connections(mut self, v: usize) -> Self {
        self.max_connections = v;
        self
    }

    /// Replace the Origin allow-list (REQ-WSKIT-200). Empty (default) =
    /// allow ALL origins — see [`WsConfig::allowed_origins`] for why you
    /// almost certainly want to set this.
    pub fn allowed_origins(mut self, origins: Vec<String>) -> Self {
        self.allowed_origins = origins;
        self
    }

    /// Add one origin to the allow-list (REQ-WSKIT-200), e.g.
    /// `.allow_origin("https://app.example.com")`. Exact match after
    /// normalization; no wildcards in v1.
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_origins.push(origin.into());
        self
    }

    /// Build the [`WsConfig`].
    pub fn build(self) -> WsConfig {
        WsConfig {
            heartbeat_interval: self.heartbeat_interval,
            broadcast_capacity: self.broadcast_capacity,
            max_connections: self.max_connections,
            allowed_origins: self.allowed_origins,
        }
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

    #[test]
    fn defaults() {
        let c = WsConfig::default();
        assert_eq!(c.heartbeat_interval, Duration::from_secs(30));
        assert_eq!(c.broadcast_capacity, 1024);
        assert_eq!(c.max_connections, 1000);
    }

    #[test]
    fn builder_overrides() {
        let c = WsConfig::builder()
            .heartbeat_interval(Duration::from_secs(10))
            .broadcast_capacity(512)
            .max_connections(10)
            .build();
        assert_eq!(c.heartbeat_interval, Duration::from_secs(10));
        assert_eq!(c.broadcast_capacity, 512);
        assert_eq!(c.max_connections, 10);
    }

    #[test]
    fn builder_default_equals_config_default() {
        let via_builder = WsConfig::builder().build();
        assert_eq!(via_builder, WsConfig::default());
    }

    #[test]
    fn req_wskit_201_default_allowed_origins_empty_allows_all() {
        // Default is allow-all (documented residual): empty list.
        assert!(WsConfig::default().allowed_origins.is_empty());
        assert!(WsConfig::builder().build().allowed_origins.is_empty());
    }

    #[test]
    fn req_wskit_200_builder_sets_allowed_origins() {
        let c = WsConfig::builder()
            .allow_origin("https://app.example.com")
            .allow_origin("https://staging.example.com")
            .build();
        assert_eq!(
            c.allowed_origins,
            vec![
                "https://app.example.com".to_string(),
                "https://staging.example.com".to_string()
            ]
        );
        // replace semantics
        let c2 = WsConfig::builder()
            .allow_origin("https://a.dev")
            .allowed_origins(vec!["https://b.dev".to_string()])
            .build();
        assert_eq!(c2.allowed_origins, vec!["https://b.dev".to_string()]);
    }
}
