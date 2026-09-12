# Performance SLOs — ws-kit

Every numeric claim in this file and the README is inventoried against its
proof artifact in [CLAIMS.md](CLAIMS.md).

Measured with criterion (`cargo bench --bench broadcast_roundtrip`),
2026-09 (0.2.x–0.3.x runs) and re-measured 2026-09-12 for 0.4.0.
Hardware: Intel(R) Core(TM) i5-9400F CPU @ 2.90GHz, 6 cores, Linux x86_64.
Criterion reports mean/median/stddev, not percentiles; **P50 column = criterion
mean** (P99 is not directly measured; the CI bench job compares means against
the saved `ci` baseline).

**0.4.0 measurement caveat:** the 2026-09-12 re-run executed on a machine
with sustained background load (~30–40 on 6 cores from unrelated projects).
Absolute means in that state are inflated up to ~3×; the numbers below are
from the cleanest window (best of repeated pinned runs), and the
String-vs-Frame comparisons are same-run, so the *relative* claim (frame
does not slow the text path) is load-independent.

## Measured (mean per operation)

| Benchmark | P50 (mean) | Notes |
|---|---|---|
| `BroadcastHub::broadcast` + `try_recv` round-trip, 1 receiver (String) | **99.1 ns** | 0.3.x clean run, publish→consume |
| Round-trip, 100 receivers | 41.6 ns/receiver | fan-out amortized |
| Round-trip, 1000 receivers | 43.6 ns/receiver | fan-out amortized |
| Sustained broadcast+consume, single receiver | 82.2 ns/msg | steady-state |
| Sustained, ×100 msgs | 80.2 ns/msg | |
| Sustained, ×1000 msgs | 82.5 ns/msg | |

### 0.4.0 re-measurement — `Frame` message model (`frame_roundtrip` bench)

The rx path was re-measured after the message-model change (room/hub
payloads now `Frame = Text(String) | Binary(Bytes)`):

| Benchmark | P50 (mean) | vs 0.3.x |
|---|---|---|
| `Frame::Text` round-trip, 1 receiver | **89.7 ns** | at/below the 99.1 ns clean baseline — no regression |
| `Frame::Binary` round-trip, 1 receiver | 80.7 ns | **binary is not slower than text** |
| `Frame::Text` round-trip, 1000 receivers | ~40 ns/receiver | 42–44 ns/receiver claim holds |
| `Frame::Binary` round-trip, 1000 receivers | ~31 ns/receiver | — |

Verification statement: **the `Frame` message model does not slow the text
hot path, and the binary variant rides the same broadcast ring at the same
cost.** `Frame` is a same-size enum wrapper (discriminant + inline
`String`/`Bytes`); conversion from owned payloads is a move, and the
`BroadcastHub<T>` SLO path (`BroadcastHub<String>`) is unchanged code.

## SLO statements

- A `BroadcastHub` broadcast→receive round-trip completes in **< 150 ns P50
  for a single receiver** (measured 99 ns clean, 2026-09; 0.4.0 `Frame`
  variant 90 ns, 2026-09-12, 6-core x86_64).
- Fan-out scales **sub-linearly**: ~40–44 ns per receiver at 100–1000
  receivers (tokio broadcast ring; the send itself stays O(1), receivers
  drain independently).
- The binary frame path must not cost more than the text path (verified:
  binary ≤ text in the same run).

## Allocation profile (proven, not just read)

- The broadcast ring's slots are pre-allocated at channel construction —
  steady-state `broadcast()` performs **no per-message allocation beyond
  the caller's message value**. *(Verified: counting allocator,
  `tests/zero_alloc_frame_rx.rs` — delta = exactly the caller's `String`.)*
- Each receiver's `try_recv` clones the message value (`T: Clone`) — for a
  `String` payload that is 1 heap allocation per delivered receiver;
  `Frame::Binary` clones are refcount bumps (no allocation). *(Verified by
  the same test: text delta = 1, binary delta = 0.)*
- Owned-payload → `Frame` conversions are moves (no allocation). *(Verified
  by the same test.)*
- Compression (`compression` feature) allocates one output `Vec` per
  compress/decompress call by design; it is off the hot path unless opted
  in.
- Instruction-count gate: `cargo bench --bench iai_hot_path` (iai-callgrind;
  CI-only) pins the text/binary round-trip and 1000-receiver fan-out. In the
  2026-09-12 run binary (21 199 instructions) ≤ text (21 399) — the "binary
  is not slower" claim holds at the instruction level, not just wall-clock.

## Compression round-trip (`compression_roundtrip` bench, 0.4.0)

Ratios are deterministic; absolute times vary with machine load. Level 6,
defaults. Raw baseline = identity envelope (one memcpy).

| Payload | Raw | Compressed | Ratio |
|---|---|---|---|
| Synthetic 2 KiB chat JSON | 2070 B | 174 B | **11.9×** |
| Repetitive 16 KiB binary ramp | 16448 B | 131 B | **~126×** (125.6×) |
| Incompressible 4 KiB pseudo-random | 4096 B | 4097 B | 1× — sent as identity (envelope only, never grows) |

Ratios re-verified 2026-09-12 with `cargo bench --features compression
--bench compression_roundtrip -- --quick` (see CLAIMS.md). The identity and
bomb-guard behaviors are pinned by unit tests in `src/compression.rs`.

## Regression policy

- Baselines are saved on main in CI by the shared bench job
  ([rust-kit.yml](https://github.com/WyattAu/engineering-standards/blob/main/.github/workflows/rust-kit.yml),
  `cargo bench -- --save-baseline ci`), non-gating (regression visibility).
  The iai-callgrind gate (`iai_hot_path`: text/binary round-trip + 1000-rx
  fan-out) is the deterministic pass/fail signal; criterion is the trend.
- Local: `cargo bench --bench broadcast_roundtrip -- --save-baseline main`,
  compare with `-- --baseline main`.
- Alert threshold: >2× mean regression on
  `broadcast_roundtrip/broadcast_recv_1rx` **or** on
  `frame_roundtrip/frame_text_1rx` (0.4.0+).
- Do not compare against baselines recorded on a loaded machine; re-run
  when load < nproc.
