//! Origin validation for WebSocket upgrades (Cross-Site WebSocket Hijacking
//! defense).
//!
//! A browser always attaches an `Origin` header to a WebSocket handshake. A
//! page on any site can open a WebSocket to your endpoint and the browser
//! will ride the victim's cookies/ambient credentials — unless the server
//! checks that Origin. [`origin_allowed_in_parts`] validates the `Origin`
//! header of the upgrade request against a configured allow-list
//! (`WsConfig::allowed_origins`) **before** `on_upgrade` is called.
//!
//! Comparison semantics: scheme + host + port **exact** match after
//! normalization ([`normalize_origin`]) — lowercase scheme and host, default
//! ports (`http`/`ws` 80, `https`/`wss` 443) omitted. **No suffix, prefix,
//! or wildcard matching in v1** — an allow-list entry `https://example.com`
//! does not accept `https://evil-example.com`, `https://example.com.evil.com`,
//! or subdomains.

/// Normalizes an Origin header value for comparison.
///
/// - scheme and host lowercased (both are case-insensitive)
/// - default port for the scheme omitted (`https://example.com:443` ==
///   `https://example.com`)
/// - bracketed IPv6 hosts supported (`https://[2001:DB8::1]:443` ==
///   `https://[2001:db8::1]`)
/// - values without a `scheme://` prefix are only case-normalized — they
///   will not match a scheme-normalized allow-list entry (fail-closed)
pub fn normalize_origin(origin: &str) -> String {
    let origin = origin.trim();
    let Some((scheme, authority)) = origin.split_once("://") else {
        return origin.to_ascii_lowercase();
    };
    let scheme = scheme.to_ascii_lowercase();
    let (host, port) = split_authority(authority);
    let mut out = format!("{scheme}://{}", host.to_ascii_lowercase());
    if let Some(port) = port {
        if !port.is_empty() && !is_default_port(&scheme, port) {
            out.push(':');
            out.push_str(port);
        }
    }
    out
}

/// Splits `host[:port]`, handling bracketed IPv6 hosts (`[::1]:8080`).
fn split_authority(authority: &str) -> (&str, Option<&str>) {
    if authority.starts_with('[') {
        if let Some(close) = authority.find(']') {
            match authority[close + 1..].strip_prefix(':') {
                Some(port) => return (&authority[..close + 1], Some(port)),
                None => return (authority, None),
            }
        }
        return (authority, None);
    }
    // Non-bracketed hosts cannot contain ':' before the port separator.
    match authority.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    }
}

/// True when `port` is the scheme's default (omitted during normalization).
fn is_default_port(scheme: &str, port: &str) -> bool {
    port.parse::<u16>().is_ok_and(|p| {
        matches!(
            (scheme, p),
            ("http", 80) | ("https", 443) | ("ws", 80) | ("wss", 443)
        )
    })
}

/// REQ-WSKIT-200/REQ-WSKIT-201: decides whether a WebSocket upgrade request
/// may proceed, based on its `Origin`.
///
/// - **Empty allow-list (default)**: everything is allowed — including a
///   missing Origin header (REQ-WSKIT-201, documented residual risk, matches
///   pre-0.3.0 behavior so existing users don't break).
/// - **Non-empty allow-list**: the request's Origin must be present and
///   normalize-match at least one entry exactly — a missing Origin header is
///   **rejected**; the check must happen before `on_upgrade`.
///
/// No wildcard/suffix matching: entries match after normalization only.
pub fn origin_allowed(origin: Option<&str>, allowed_origins: &[String]) -> bool {
    if allowed_origins.is_empty() {
        return true;
    }
    let Some(origin) = origin else {
        return false;
    };
    let normalized = normalize_origin(origin);
    allowed_origins
        .iter()
        .any(|entry| normalize_origin(entry) == normalized)
}

/// REQ-WSKIT-200: [`origin_allowed`] over the `Origin` header of an upgrade
/// request. Call this in the upgrade handler **before** `WebSocketUpgrade::
/// on_upgrade`; a `false` result should map to `403 Forbidden`.
#[cfg(feature = "axum")]
pub fn origin_allowed_in_parts(parts: &http::request::Parts, allowed_origins: &[String]) -> bool {
    let origin = parts
        .headers
        .get(http::header::ORIGIN)
        .and_then(|value| value.to_str().ok());
    origin_allowed(origin, allowed_origins)
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
    fn normalize_origin_basic_and_case() {
        assert_eq!(
            normalize_origin("https://example.com"),
            "https://example.com"
        );
        assert_eq!(
            normalize_origin("HTTPS://EXAMPLE.COM"),
            "https://example.com"
        );
        assert_eq!(
            normalize_origin("https://EXAMPLE.com"),
            "https://example.com"
        );
        assert_eq!(
            normalize_origin("  https://example.com  "),
            "https://example.com"
        );
    }

    #[test]
    fn normalize_origin_default_ports_omitted() {
        assert_eq!(
            normalize_origin("https://example.com:443"),
            "https://example.com"
        );
        assert_eq!(
            normalize_origin("http://example.com:80"),
            "http://example.com"
        );
        assert_eq!(
            normalize_origin("wss://example.com:443"),
            "wss://example.com"
        );
        assert_eq!(normalize_origin("ws://example.com:80"), "ws://example.com");
    }

    #[test]
    fn normalize_origin_custom_ports_kept() {
        assert_eq!(
            normalize_origin("https://example.com:8443"),
            "https://example.com:8443"
        );
        // Port differences must NOT collapse: different origin.
        assert_ne!(
            normalize_origin("https://example.com:8443"),
            normalize_origin("https://example.com")
        );
        assert_ne!(
            normalize_origin("http://example.com:8080"),
            normalize_origin("http://example.com")
        );
    }

    #[test]
    fn normalize_origin_ipv6() {
        assert_eq!(
            normalize_origin("https://[2001:DB8::1]:443"),
            "https://[2001:db8::1]"
        );
        assert_eq!(normalize_origin("http://[::1]:8080"), "http://[::1]:8080");
        assert_eq!(normalize_origin("http://[::1]"), "http://[::1]");
    }

    #[test]
    fn normalize_origin_no_scheme_fail_closed() {
        // Malformed values do not gain a scheme — they cannot masquerade as
        // a scheme-normalized allow-list entry.
        assert_eq!(normalize_origin("example.com"), "example.com");
        assert_eq!(normalize_origin("localhost:3000"), "localhost:3000");
    }

    #[test]
    fn origin_allowed_empty_list_allows_all() {
        // REQ-WSKIT-201: default behavior — allow all, even missing Origin.
        assert!(origin_allowed(Some("https://anything.dev"), &[]));
        assert!(origin_allowed(None, &[]));
    }

    #[test]
    fn origin_allowed_match_and_normalization() {
        let allowed = vec!["https://example.com".to_string()];
        assert!(origin_allowed(Some("https://example.com"), &allowed));
        // default-port equivalence
        assert!(origin_allowed(Some("https://example.com:443"), &allowed));
        // case-insensitive host and scheme
        assert!(origin_allowed(Some("HTTPS://EXAMPLE.COM"), &allowed));
        // allow-list entries are normalized the same way
        let allowed443 = vec!["https://example.com:443".to_string()];
        assert!(origin_allowed(Some("https://example.com"), &allowed443));
    }

    #[test]
    fn origin_allowed_missing_origin_rejected_when_listed() {
        let allowed = vec!["https://example.com".to_string()];
        assert!(!origin_allowed(None, &allowed));
    }

    #[test]
    fn origin_allowed_no_wildcard_or_suffix_matching() {
        let allowed = vec!["https://example.com".to_string()];
        assert!(!origin_allowed(Some("https://evil-example.com"), &allowed));
        assert!(!origin_allowed(
            Some("https://example.com.evil.com"),
            &allowed
        ));
        assert!(!origin_allowed(Some("https://api.example.com"), &allowed));
        assert!(!origin_allowed(Some("https://example.com:8443"), &allowed));
    }

    #[test]
    fn origin_allowed_scheme_matters() {
        let allowed = vec!["https://example.com".to_string()];
        assert!(!origin_allowed(Some("http://example.com"), &allowed));
        assert!(!origin_allowed(Some("ws://example.com"), &allowed));
    }

    #[cfg(feature = "axum")]
    mod parts_tests {
        use super::*;
        use http::{HeaderMap, Request};

        fn parts_with_origin(origin: Option<&str>) -> http::request::Parts {
            let mut builder = Request::builder().uri("/ws");
            if let Some(o) = origin {
                builder = builder.header(http::header::ORIGIN, o);
            }
            let (parts, _) = builder.body(()).unwrap().into_parts();
            parts
        }

        #[test]
        fn origin_allowed_in_parts_reads_header() {
            let allowed = vec!["https://example.com".to_string()];
            assert!(origin_allowed_in_parts(
                &parts_with_origin(Some("https://example.com")),
                &allowed
            ));
            assert!(!origin_allowed_in_parts(
                &parts_with_origin(Some("https://evil.com")),
                &allowed
            ));
            assert!(!origin_allowed_in_parts(&parts_with_origin(None), &allowed));
            assert!(origin_allowed_in_parts(&parts_with_origin(None), &[]));
        }

        #[test]
        fn origin_allowed_in_parts_host_header_ignored() {
            // A Host header must not stand in for Origin.
            let allowed = vec!["https://example.com".to_string()];
            let mut headers = HeaderMap::new();
            headers.insert(http::header::HOST, "example.com".parse().unwrap());
            let req = Request::builder().uri("/ws").body(()).unwrap();
            let (mut parts, _) = req.into_parts();
            parts.headers = headers;
            assert!(!origin_allowed_in_parts(&parts, &allowed));
        }
    }
}
