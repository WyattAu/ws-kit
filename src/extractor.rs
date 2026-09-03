//! Token extraction for WebSocket authentication.
//!
//! Extracts bearer tokens from HTTP upgrade requests using a configurable set
//! of [`TokenSourceKind`] strategies. Supports `Authorization: Bearer <token>`,
//! query parameters (`?token=` / `?access_token=`), and `Cookie` headers.

use thiserror::Error;

#[cfg(feature = "axum")]
use http::request::Parts;

/// Where to look for a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSourceKind {
    /// `Authorization: Bearer <token>` header.
    AuthorizationHeader,
    /// Query parameter with the given name (e.g. `token`, `access_token`).
    QueryParam(String),
    /// Cookie with the given name.
    Cookie(String),
}

/// Error during authentication extraction/validation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WsAuthError {
    /// No token found in any configured source.
    #[error("authentication token missing")]
    Missing,

    /// Token was found but is invalid.
    #[error("authentication token invalid")]
    Invalid,

    /// Token is expired (if expiry is checked downstream).
    #[error("authentication token expired")]
    Expired,
}

/// Extracts tokens from HTTP request parts according to configured sources.
#[derive(Debug, Clone)]
pub struct TokenExtractor {
    sources: Vec<TokenSourceKind>,
}

impl Default for TokenExtractor {
    fn default() -> Self {
        Self {
            sources: vec![
                TokenSourceKind::AuthorizationHeader,
                TokenSourceKind::QueryParam("token".to_string()),
                TokenSourceKind::QueryParam("access_token".to_string()),
            ],
        }
    }
}

impl TokenExtractor {
    /// Create a new extractor with the given ordered sources.
    pub fn new(sources: Vec<TokenSourceKind>) -> Self {
        Self { sources }
    }

    /// Bearer-only extractor.
    pub fn bearer_only() -> Self {
        Self {
            sources: vec![TokenSourceKind::AuthorizationHeader],
        }
    }

    /// Add a source.
    pub fn with_source(mut self, src: TokenSourceKind) -> Self {
        self.sources.push(src);
        self
    }

    /// Ordered sources.
    pub fn sources(&self) -> &[TokenSourceKind] {
        &self.sources
    }

    /// Extract the first token found according to source order.
    ///
    /// When the `axum` feature is enabled the first argument is `&http::request::Parts`
    /// (from `axum`'s request). The second argument is the raw query string
    /// (e.g. `"token=abc&foo=bar"` without leading `?`).
    ///
    /// Returns `None` if no token was found in any source.
    #[cfg(feature = "axum")]
    pub fn extract_token(&self, parts: &Parts, query: &str) -> Option<String> {
        for src in &self.sources {
            match src {
                TokenSourceKind::AuthorizationHeader => {
                    if let Some(v) = Self::extract_bearer(parts) {
                        return Some(v);
                    }
                }
                TokenSourceKind::QueryParam(name) => {
                    if let Some(v) = Self::extract_query_param(query, name) {
                        return Some(v);
                    }
                }
                TokenSourceKind::Cookie(name) => {
                    if let Some(v) = Self::extract_cookie(parts, name) {
                        return Some(v);
                    }
                }
            }
        }
        None
    }

    /// Extract token without `Parts` (query and cookie header strings only).
    ///
    /// Useful when `axum` is not enabled or for tests.
    pub fn extract_from_parts(
        &self,
        auth_header: Option<&str>,
        cookie_header: Option<&str>,
        query: &str,
    ) -> Option<String> {
        for src in &self.sources {
            match src {
                TokenSourceKind::AuthorizationHeader => {
                    if let Some(v) = auth_header.and_then(Self::parse_bearer) {
                        if !v.is_empty() {
                            return Some(v);
                        }
                    }
                }
                TokenSourceKind::QueryParam(name) => {
                    if let Some(v) = Self::extract_query_param(query, name) {
                        return Some(v);
                    }
                }
                TokenSourceKind::Cookie(name) => {
                    if let Some(v) = cookie_header.and_then(|c| Self::parse_cookie(c, name)) {
                        return Some(v);
                    }
                }
            }
        }
        None
    }

    /// Validate presence of token, returning a typed error.
    #[cfg(feature = "axum")]
    pub fn require_token(&self, parts: &Parts, query: &str) -> Result<String, WsAuthError> {
        self.extract_token(parts, query).ok_or(WsAuthError::Missing)
    }

    #[cfg(feature = "axum")]
    fn extract_bearer(parts: &Parts) -> Option<String> {
        let val = parts.headers.get(http::header::AUTHORIZATION)?;
        let s = val.to_str().ok()?;
        Self::parse_bearer(s)
    }

    fn parse_bearer(s: &str) -> Option<String> {
        // Case-insensitive "Bearer " prefix
        if s.len() < 7 {
            return None;
        }
        let prefix = &s[..7];
        if !prefix.eq_ignore_ascii_case("bearer ") {
            return None;
        }
        let token = s[7..].trim();
        if token.is_empty() {
            None
        } else {
            Some(token.to_string())
        }
    }

    fn extract_query_param(query: &str, name: &str) -> Option<String> {
        let query = query.trim_start_matches('?');
        if query.is_empty() {
            return None;
        }
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let mut split = pair.splitn(2, '=');
            let k = split.next().unwrap_or("");
            let v = split.next().unwrap_or("");
            // URL-decode k and v minimally: '+' -> ' ', %XX
            let k_decoded = url_decode(k);
            if k_decoded == name {
                let v_decoded = url_decode(v);
                if !v_decoded.is_empty() {
                    return Some(v_decoded);
                }
            }
        }
        None
    }

    #[cfg(feature = "axum")]
    fn extract_cookie(parts: &Parts, name: &str) -> Option<String> {
        let val = parts.headers.get(http::header::COOKIE)?;
        let s = val.to_str().ok()?;
        Self::parse_cookie(s, name)
    }

    fn parse_cookie(header: &str, name: &str) -> Option<String> {
        for part in header.split(';') {
            let part = part.trim();
            let mut split = part.splitn(2, '=');
            let k = split.next().unwrap_or("").trim();
            let v = split.next().unwrap_or("").trim();
            // Trim optional quotes
            let v = v.trim_matches('"');
            if k == name && !v.is_empty() {
                return Some(url_decode(v));
            }
        }
        None
    }
}

fn url_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '+' => out.push(' '),
            '%' => {
                let hi = chars.next();
                let lo = chars.next();
                if let (Some(h), Some(l)) = (hi, lo) {
                    if let (Some(hv), Some(lv)) = (hex_val(h), hex_val(l)) {
                        out.push((hv * 16 + lv) as char);
                    } else {
                        out.push('%');
                        out.push(h);
                        out.push(l);
                    }
                } else {
                    out.push('%');
                    if let Some(h) = hi {
                        out.push(h);
                    }
                    if let Some(l) = lo {
                        out.push(l);
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}

fn hex_val(c: char) -> Option<u8> {
    match c {
        '0'..='9' => Some(c as u8 - b'0'),
        'a'..='f' => Some(c as u8 - b'a' + 10),
        'A'..='F' => Some(c as u8 - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bearer_ok() {
        assert_eq!(
            TokenExtractor::parse_bearer("Bearer abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            TokenExtractor::parse_bearer("bearer abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            TokenExtractor::parse_bearer("BEARER abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(
            TokenExtractor::parse_bearer("Bearer   spaced  "),
            Some("spaced".to_string())
        );
    }

    #[test]
    fn parse_bearer_fail() {
        assert_eq!(TokenExtractor::parse_bearer("Basic abc"), None);
        assert_eq!(TokenExtractor::parse_bearer("Bearer "), None);
        assert_eq!(TokenExtractor::parse_bearer("Bearer"), None);
        assert_eq!(TokenExtractor::parse_bearer(""), None);
    }

    #[test]
    fn query_param_extraction() {
        assert_eq!(
            TokenExtractor::extract_query_param("token=abc123&foo=bar", "token"),
            Some("abc123".to_string())
        );
        assert_eq!(
            TokenExtractor::extract_query_param("?access_token=xyz", "access_token"),
            Some("xyz".to_string())
        );
        assert_eq!(
            TokenExtractor::extract_query_param("foo=1&token=second", "token"),
            Some("second".to_string())
        );
        assert_eq!(TokenExtractor::extract_query_param("", "token"), None);
        assert_eq!(
            TokenExtractor::extract_query_param("token=&foo=bar", "token"),
            None
        );
    }

    #[test]
    fn query_url_decode() {
        assert_eq!(
            TokenExtractor::extract_query_param("token=hello%20world", "token"),
            Some("hello world".to_string())
        );
        assert_eq!(
            TokenExtractor::extract_query_param("token=a%2Bb", "token"),
            Some("a+b".to_string())
        );
    }

    #[test]
    fn cookie_extraction() {
        assert_eq!(
            TokenExtractor::parse_cookie("session=abc; token=xyz; foo=bar", "token"),
            Some("xyz".to_string())
        );
        assert_eq!(
            TokenExtractor::parse_cookie("token=hello%20world", "token"),
            Some("hello world".to_string())
        );
        assert_eq!(TokenExtractor::parse_cookie("a=b; c=d", "token"), None);
    }

    #[test]
    fn extractor_order_header_first() {
        let ex = TokenExtractor::default();
        let token = ex.extract_from_parts(Some("Bearer header_tok"), None, "token=query_tok");
        assert_eq!(token, Some("header_tok".to_string()));
    }

    #[test]
    fn extractor_fallback_to_query() {
        let ex = TokenExtractor::default();
        let token = ex.extract_from_parts(None, None, "token=query_tok");
        assert_eq!(token, Some("query_tok".to_string()));
    }

    #[test]
    fn extractor_fallback_access_token() {
        let ex = TokenExtractor::default();
        let token = ex.extract_from_parts(None, None, "access_token=at");
        assert_eq!(token, Some("at".to_string()));
    }

    #[test]
    fn extractor_cookie_source() {
        let ex = TokenExtractor::new(vec![TokenSourceKind::Cookie("session".to_string())]);
        let token = ex.extract_from_parts(None, Some("session=mycookie"), "");
        assert_eq!(token, Some("mycookie".to_string()));
    }

    #[test]
    fn extractor_none_when_missing() {
        let ex = TokenExtractor::default();
        assert_eq!(ex.extract_from_parts(None, None, ""), None);
    }

    #[cfg(feature = "axum")]
    mod axum_tests {
        use super::*;
        use http::{HeaderMap, Request};

        fn parts_with(headers: HeaderMap, uri: &str) -> http::request::Parts {
            let req = Request::builder()
                .uri(uri)
                .body(())
                .unwrap();
            let (mut parts, _) = req.into_parts();
            parts.headers = headers;
            parts
        }

        #[test]
        fn axum_bearer_extraction() {
            let mut headers = HeaderMap::new();
            headers.insert(
                http::header::AUTHORIZATION,
                "Bearer axum_tok".parse().unwrap(),
            );
            let parts = parts_with(headers, "/ws?token=ignored");
            let ex = TokenExtractor::default();
            assert_eq!(
                ex.extract_token(&parts, "token=ignored"),
                Some("axum_tok".to_string())
            );
        }

        #[test]
        fn axum_query_fallback() {
            let parts = parts_with(HeaderMap::new(), "/ws?token=qtoken");
            let ex = TokenExtractor::default();
            assert_eq!(
                ex.extract_token(&parts, "token=qtoken"),
                Some("qtoken".to_string())
            );
        }

        #[test]
        fn axum_cookie() {
            let mut headers = HeaderMap::new();
            headers.insert(http::header::COOKIE, "tok=cookie_val".parse().unwrap());
            let parts = parts_with(headers, "/ws");
            let ex = TokenExtractor::new(vec![TokenSourceKind::Cookie("tok".to_string())]);
            assert_eq!(ex.extract_token(&parts, ""), Some("cookie_val".to_string()));
        }
    }
}
