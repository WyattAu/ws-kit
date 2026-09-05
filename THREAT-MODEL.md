# Threat Model — ws-kit

Status: **v1.0** · Method: STRIDE over the public API surface
(`TokenExtractor`, `JsonCodec`, `BroadcastHub`, `Room`/`RoomManager`,
`WsConfig`).

Trust boundaries: (1) the WebSocket upgrade request (Authorization header,
query string, Cookie header — all client-controlled), (2) messages flowing
through the codec (client-authored JSON), (3) concurrent senders/receivers
sharing hub and room state.

## Assets

| ID | Asset | Example |
|----|-------|---------|
| A1 | Connection authentication | Unauthenticated WS upgrade accepted |
| A2 | Token confidentiality | `?token=` values captured from proxy/access logs |
| A3 | Hub/room availability | Message floods or unbounded subscriber growth exhaust memory |
| A4 | Room isolation | Cross-room message leakage |

## STRIDE Analysis

| # | Threat | Category | Surface | Mitigation | Verifying test |
|---|--------|----------|---------|------------|----------------|
| T1 | Token sniffing via multi-byte header slicing panic/read | Spoofing | `TokenExtractor::parse_bearer` | ASCII byte-compare of the 7-byte scheme prefix gated by `is_char_boundary(7)`; mid-code-point values rejected, not sliced | `src/extractor.rs::parse_bearer_multibyte_no_panic` (regression for REQ-WSKIT-001 fuzz finding), `parse_bearer_ok`, `parse_bearer_fail` |
| T2 | Missing token accepted | Spoofing | `require_token`, `extract_token` | Ordered source scan; absence → `WsAuthError::Missing` | `src/extractor.rs::extractor_none_when_missing`; `tests/integration.rs` hub/room suite |
| T3 | Query/cookie parse confusion (`+`, `%XX`, quotes) | Spoofing | `extract_query_param`, `parse_cookie` | Minimal URL decode (`+`→space, `%XX`), quoted cookie values trimmed, empty values skipped | `src/extractor.rs::query_param_extraction`, cookie tests (`session=mycookie`), axum-feature tests `axum_bearer_extraction`, `axum_query_fallback`, `axum_cookie` |
| T4 | Hostile JSON crashes decode | DoS | `JsonCodec::decode` | Errors are `Result<T, WsError>`; `#![forbid(unsafe_code)]` | `fuzz/fuzz_targets/fuzz_codec.rs`, `fuzz_extract.rs`; `tests/integration.rs::ws_error_display` |
| T5 | Broadcast/channel flooding | DoS | `BroadcastHub` | Bounded `broadcast` channels (`broadcast_capacity`, default 1024); lagging receivers get `RecvError::Lagged` instead of unbounded growth | `tests/integration.rs::hub_broadcast_subscribe`, `hub_multiple_subscribers` |
| T6 | Connection-count exhaustion | DoS | `BroadcastHub::increment_connections` | Atomic counter with optional max; refusal returns `WsError` | `tests/integration.rs::hub_connection_count_limit` (default cap 1000, `WsConfig`) |
| T7 | Cross-room message leakage | Info disclosure | `Room` / `RoomManager` | Per-room isolated `broadcast::Sender`; cleanup drops the sender | `tests/integration.rs::room_isolation`, `room_cleanup`, `room_manager_basic` |
| T8 | Concurrent subscribe/broadcast races | Tampering | hub/room internals | `tokio::sync::broadcast` semantics; loom-modelled | `src/loom_tests.rs` (loom feature); `tests/integration.rs::hub_integration_with_config` |
| T9 | Cross-site WebSocket hijacking (CSWSH): malicious page opens an upgrade riding the victim's cookies/ambient auth | Spoofing / Elevation | upgrade handshake (`Origin` header) | REQ-WSKIT-200: opt-in Origin allow-list (`WsConfig::allowed_origins`); non-empty list → missing/mismatched Origin rejected **403 before `on_upgrade`**; exact match after normalization (lowercase scheme/host, default ports omitted); no wildcard/suffix matching | `tests/integration.rs::origin_upgrade::req_wskit_200_*` (allowed→101, disallowed/missing→403, port-normalized, no-suffix); unit `src/origin.rs::origin_allowed_*`, `normalize_origin_*` |

## CLOSED RISKS (mitigated — cited by tests)

- **CLOSED-1 (was OPEN-3) — no origin check on the upgrade request** —
  closed in **0.3.0** (REQ-WSKIT-200). `WsConfig::allowed_origins` +
  `origin_allowed_in_parts` validate the browser-supplied `Origin` header
  at upgrade time; with a non-empty list, a missing or mismatched Origin is
  rejected 403 before `on_upgrade` (tests:
  `tests/integration.rs::origin_upgrade::req_wskit_200_*`, unit
  `src/origin.rs`). **Residual (accepted, REQ-WSKIT-201): the default is an
  empty allow-list = allow all**, preserving 0.2.x behavior — CSWSH stays
  reachable for integrators who never set the list, exactly as before. The
  check is also opt-in at the handler site: ws-kit provides the decision
  fn; wiring it into the upgrade handler (before `on_upgrade`) is shown in
  the integration tests and remains integrator-owned, like all middleware
  ordering in axum.

## OPEN RISKS (missing mitigations — not fabricated)

- **OPEN-1 — tokens in query strings are a default source.**
  `TokenExtractor::default()` includes `QueryParam("token")` and
  `QueryParam("access_token")`; URLs (and therefore tokens) land in access
  logs, proxy logs, and browser history. `bearer_only()` exists but is
  opt-in; no warning, no log-scrubbing, no test documenting the choice.
- **OPEN-2 — URL decoding is byte-to-`char` naive.** `url_decode` maps each
  `%XX` to a `char` directly, so percent-encoded multi-byte UTF-8 (or any
  non-ASCII token) decodes to mojibake and is then rejected downstream —
  fail-closed, but a correctness/interoperability gap between header and
  query sources.
- **OPEN-4 — `max_connections = 0` means unlimited.** The config documents
  the sentinel; nothing prevents a caller building `WsConfig` with 0 by
  accident, and no test pins the "0 = unlimited" semantics.
- **OPEN-5 — `parse_cookie` does not handle escaped separators** inside
  quoted values; a token containing `;` splits. Fail-closed (wrong value →
  auth failure), listed for completeness.

## Out of Scope

- Token *validation* (signature/expiry) — extraction only; verification is
  delegated to e.g. `tokenkit`.
- TLS termination and wss:// transport security.
- Message authorization after decode (per-room ACLs are integrator logic).

## Residual Risks

- Ordered source scanning means a request carrying both a valid header and a
  stale query token uses the header first — predictable, but callers should
  not assume all sources agree.
- Heartbeat interval (default 30 s) detects dead peers but does not bound
  inbound message rate; per-connection read limits are integrator-owned.
