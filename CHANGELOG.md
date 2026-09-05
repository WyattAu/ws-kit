# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

## [0.3.0] - 2026-09-05

### Added

- Opt-in Origin validation for the WebSocket upgrade handshake (CSWSH
  defense, REQ-WSKIT-200/201):
  - `WsConfig::allowed_origins` field + `WsConfigBuilder::allow_origin` /
    `allowed_origins`.
  - `origin_allowed_in_parts(&Parts, &allowed)` — call in the upgrade
    handler **before** `on_upgrade`; missing or mismatched `Origin` should
    map to `403 Forbidden`.
  - `normalize_origin` — lowercase scheme/host, default ports (`http`/`ws`
    80, `https`/`wss` 443) omitted; exact match only, **no wildcard or
    suffix matching in v1**.
  - **Default (empty list) remains allow-all** — documented residual risk
    (REQ-WSKIT-201); existing users are not broken. Set the list to defend
    against cross-site WebSocket hijacking when auth rides cookies.

### Fixed

- Pre-existing `clippy::indexing_slicing` violation in
  `TokenExtractor::parse_bearer` (`bytes.get(..7)` instead of unchecked
  slicing) — the CI `-D warnings` gate now passes.

## [0.2.1] - 2026-09-04

### Fixed

- `--no-default-features` build.

### Testing

- Loom model-checking of `ConnectionCounter` invariants
  (`RUSTFLAGS="--cfg loom" cargo test --release --lib -- loom`): balanced
  increment/decrement pairs never lose updates or go negative; bounded
  increments hold under all interleavings.

## [0.2.0] - 2026-09-03

### Added

- Participants roster and manager counts on `RoomManager`.

## [0.1.0] - 2026-09-03

### Added

- Generic authenticated typed WebSocket kit: `BroadcastHub`,
  `RoomManager`, `TokenExtractor`, `Codec`, `WsConfig`.
- Feature-gated `axum` integration (default) and `tracing`
  instrumentation.
