# Pattern adoption: ws-kit

Authenticated typed WebSocket toolkit. Declared per
`pattern-standards/references/adoption-format.md`.

| Pattern | Status | Evidence |
|---|---|---|
| T13 | adopted | src/counter.rs:1 (connection counter extracted as a pure atomic core so it can be model-checked; src/hub.rs:1 is the shell) |
| T16 | adopted | src/config.rs:1 (broadcast capacity and connection limits with TooManyConnections) and src/stats.rs:1 (drop and backpressure counters, global and per-room) |
| T36 | partial | src/loom_tests.rs:1 (loom model-checks all bounded interleavings of the counter; full-system simulation is not attempted) |
| T20 | n/a | src/hub.rs:1 uses tokio broadcast channels; no single-writer ring buffer in this design |
