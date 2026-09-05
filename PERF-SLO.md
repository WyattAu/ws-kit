# Performance SLOs — ws-kit

Measured with criterion (`cargo bench --bench broadcast_roundtrip`), 2026-09.
Hardware: Intel(R) Core(TM) i5-9400F CPU @ 2.90GHz, 6 cores, Linux x86_64.
Criterion reports mean/median/stddev, not percentiles; **P50 column = criterion
mean** (P99 is not directly measured; the CI bench job compares means against
the saved `ci` baseline).

## Measured (mean per operation)

| Benchmark | P50 (mean) | Notes |
|---|---|---|
| `BroadcastHub::broadcast` + `try_recv` round-trip, 1 receiver | **99.1 ns** | publish→consume |
| Round-trip, 100 receivers | 41.6 ns/receiver | fan-out amortized |
| Round-trip, 1000 receivers | 43.6 ns/receiver | fan-out amortized |
| Sustained broadcast+consume, single receiver | 82.2 ns/msg | steady-state |
| Sustained, ×100 msgs | 80.2 ns/msg | |
| Sustained, ×1000 msgs | 82.5 ns/msg | |

## SLO statements

- A `BroadcastHub` broadcast→receive round-trip completes in **< 150 ns P50
  for a single receiver** (measured 99 ns, 2026-09, 6-core x86_64).
- Fan-out scales **sub-linearly**: ~42–44 ns per receiver at 100–1000
  receivers (tokio broadcast ring; the send itself stays O(1), receivers
  drain independently).

## Allocation profile (from code reading)

- The broadcast ring's slots are pre-allocated at channel construction —
  steady-state `broadcast()`/`try_broadcast()` performs **no per-message
  allocation beyond the caller's message value**.
- Each receiver's `try_recv` clones the message value (`T: Clone`) — for the
  bench's `String` payload that is 1 heap allocation per delivered receiver.
- Not yet verified with a counting allocator; the breaker probe (see its
  PERF-SLO.md) demonstrates the method.

## Regression policy

- Baselines are saved on main in CI by the shared bench job
  ([rust-kit.yml](https://github.com/WyattAu/engineering-standards/blob/main/.github/workflows/rust-kit.yml),
  `cargo bench -- --save-baseline ci`), non-gating (regression visibility).
- Local: `cargo bench --bench broadcast_roundtrip -- --save-baseline main`,
  compare with `-- --baseline main`.
- Alert threshold: >2× mean regression on
  `broadcast_roundtrip/broadcast_recv_1rx`.
