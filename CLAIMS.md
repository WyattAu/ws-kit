# Performance claims inventory — ws-kit

Every numeric performance claim in [README.md](README.md) and
[PERF-SLO.md](PERF-SLO.md), mapped to its proof artifact. Created 2026-09-12
(ws-kit 0.4.1).

Proof artifact kinds:

- **criterion** — wall-clock bench, noisy across machines/loads; the number
  is a measurement record ("measured on"), not a reproducible constant.
  CI re-runs these with regression visibility (non-gating).
- **iai** — iai-callgrind instruction counts under valgrind; reproducible
  for a given binary, fit for a CI pass/fail gate
  (`cargo bench --bench iai_hot_path`).
- **test** — unit/integration test, including the counting-allocator gate
  (`tests/zero_alloc_frame_rx.rs`), which runs on every `cargo test`.

## Latency (criterion — measured-on records)

| # | Claim | Artifact | Status |
|---|---|---|---|
| 1 | broadcast→recv round-trip 1 rx, `String`: 99.1 ns | `benches/broadcast_roundtrip.rs::broadcast_recv_1rx` (0.3.x clean run, 2026-09; i5-9400F) | backed (measured on) |
| 2 | fan-out 41.6 ns/receiver @ 100 rx | same bench, `broadcast_recv_100rx` | backed (measured on) |
| 3 | fan-out 43.6 ns/receiver @ 1000 rx | same bench, `broadcast_recv_1000rx` | backed (measured on) |
| 4 | sustained 82.2 ns/msg, 1 rx | `sustained_broadcast::broadcast_1msgs_1rx` | backed (measured on) |
| 5 | sustained 80.2 ns/msg ×100 | same bench, `broadcast_100msgs_1rx` | backed (measured on) |
| 6 | sustained 82.5 ns/msg ×1000 | same bench, `broadcast_1000msgs_1rx` | backed (measured on) |
| 7 | `Frame::Text` round-trip 1 rx: 89.7 ns | `frame_roundtrip::frame_text_1rx` (0.4.0 re-run; loaded-machine caveat in PERF-SLO.md) | backed (measured on) |
| 8 | `Frame::Binary` round-trip 1 rx: 80.7 ns | `frame_roundtrip::frame_binary_1rx` | backed (measured on) |
| 9 | `Frame::Text` fan-out ~40 ns/rx @ 1000 | `frame_roundtrip::frame_text_1000rx` | backed (measured on) |
| 10 | `Frame::Binary` fan-out ~31 ns/rx @ 1000 | `frame_roundtrip::frame_binary_1000rx` | backed (measured on) |
| 11 | SLO: round-trip < 150 ns P50, 1 rx | policy; evidence = claims 1+7 | backed (policy) |
| 12 | fan-out sub-linear, 40–44 ns/receiver @ 100–1000 | evidence = claims 2+3 | backed (policy) |

## Message-model cost equality (iai — instruction counts)

| # | Claim | Artifact | Status |
|---|---|---|---|
| 13 | binary frame path is not slower than text | `benches/iai_hot_path.rs`: `frame_binary_roundtrip` = 21199 instr vs `frame_text_roundtrip` = 21399 instr (2026-09-12, valgrind 3.25.1) | **proven** |

## Allocation profile (counting allocator — runs on every `cargo test`)

| # | Claim | Artifact | Status |
|---|---|---|---|
| 14 | ring pre-allocated; steady-state `broadcast()` allocates nothing beyond the caller's message value | `tests/zero_alloc_frame_rx.rs` (delta = exactly 1 alloc/iter = the caller's `String`) | **proven** |
| 15 | each delivered receiver's `try_recv` clones the message: `String` = 1 heap alloc per receiver | same test (delta = exactly 1) | **proven** |
| 16 | `Frame::Binary` clone is a refcount bump — no allocation | same test (delta = 0 over 100 iters) | **proven** |
| 17 | owned-payload → `Frame` conversions are moves | same test (delta = 0) | **proven** |
| 18 | compression allocates one output `Vec` per call, by design | code reading (`src/compression.rs`); off the hot path unless the feature is enabled | backed (code) |
| 19 | ~~"Not yet verified with a counting allocator"~~ | superseded by claims 14–17 | **reworded** (now verified) |

## Compression ratios (deterministic — re-verified 2026-09-12)

| # | Claim | Artifact | Status |
|---|---|---|---|
| 20 | 2 KiB synthetic chat JSON: 2070 B → 174 B, **11.9×** | `benches/compression_roundtrip.rs` run output (`json_2k: raw=2070 compressed=174 ratio=11.9x`) | **proven** (re-measured) |
| 21 | repetitive 16 KiB binary: **~126×** | same bench (`binrep_16k: raw=16448 compressed=131 ratio=125.6x`); PERF-SLO table raw/compressed corrected to 16448/131 | **proven** (re-measured) |
| 22 | incompressible 4 KiB pseudo-random → identity, wire size never grows | unit test `incompressible_payload_falls_back_to_identity` + `small_payloads_stay_identity` (`src/compression.rs`) | **proven** (test) |
| 23 | bomb guard: decompressed output capped *during* inflation | unit tests in `src/compression.rs` (bomb cap, oversized identity refused) | **proven** (test) |
| 24 | `min_size` default 64 B | code + `small_payloads_stay_identity` | backed |
| 25 | `max_size` default 1 MiB | code + bomb-cap tests | backed |

## Other

| # | Claim | Artifact | Status |
|---|---|---|---|
| 26 | `StatsRecorder` is lock-free (atomics + sharded map) | code reading (`src/stats.rs`: atomics + `DashMap`) | backed (code) |
| 27 | README perf table (99.1 ns / 42–44 ns / ~90 ns) | = claims 1, 12, 7 | backed |

## Totals

- **Proven by hard artifact (iai/test/re-measured):** 12 (claims 13–17, 20–23)
- **Backed (measured-on criterion records, policy, or code reading):** 14
- **Deleted/reworded:** 1 (claim 19 — the "not yet verified" disclaimer,
  which the new counting-allocator test retires)

## Reproducing

```sh
cargo bench --bench broadcast_roundtrip          # wall-clock trend
cargo bench --bench iai_hot_path                 # instruction gate (needs valgrind)
cargo test  --test zero_alloc_frame_rx           # allocation gate
cargo test  --features compression               # compression identity/bomb tests
cargo bench --features compression --bench compression_roundtrip -- --quick
```
