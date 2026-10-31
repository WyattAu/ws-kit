# Performance SLOs: ws-kit
### 0.4.0 re-measurement: `Frame` message model (`frame_roundtrip` bench)
| `Frame::Text` round-trip, 1 receiver | **89.7 ns** | at/below the 99.1 ns clean baseline; no regression |
| `Frame::Binary` round-trip, 1000 receivers | ~31 ns/receiver |; |
- The broadcast ring's slots are pre-allocated at channel construction -
  `tests/zero_alloc_frame_rx.rs`; delta = exactly the caller's `String`.)*
- Each receiver's `try_recv` clones the message value (`T: Clone`); for a
  2026-09-12 run binary (21 199 instructions) ≤ text (21 399); the "binary
| Incompressible 4 KiB pseudo-random | 4096 B | 4097 B | 1×; sent as identity (envelope only, never grows) |