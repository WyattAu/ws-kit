//! Configuration for WebSocket infrastructure.
//!
//! [`WsConfig`] controls heartbeat cadence, broadcast buffering, and global
//! connection limits. Use the builder for ergonomic construction.

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
///     .build();
/// assert_eq!(cfg.heartbeat_interval, Duration::from_secs(15));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsConfig {
    /// Interval between heartbeat pings.
    pub heartbeat_interval: Duration,
    /// Capacity of each broadcast channel.
    pub broadcast_capacity: usize,
    /// Maximum concurrent connections allowed (0 = unlimited).
    pub max_connections: usize,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(30),
            broadcast_capacity: 1024,
            max_connections: 1000,
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
}

impl Default for WsConfigBuilder {
    fn default() -> Self {
        let d = WsConfig::default();
        Self {
            heartbeat_interval: d.heartbeat_interval,
            broadcast_capacity: d.broadcast_capacity,
            max_connections: d.max_connections,
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

    /// Build the [`WsConfig`].
    pub fn build(self) -> WsConfig {
        WsConfig {
            heartbeat_interval: self.heartbeat_interval,
            broadcast_capacity: self.broadcast_capacity,
            max_connections: self.max_connections,
        }
    }
}

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
}
