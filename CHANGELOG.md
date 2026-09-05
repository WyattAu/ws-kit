# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

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
