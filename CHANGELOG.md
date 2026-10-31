- `tests/config_matrix.rs`; per-knob behavior matrix for all 7 knobs
  heartbeat task exists in the library); pinned inert.
- `benches/iai_hot_path.rs`; iai-callgrind instruction-count gate for the
  (21199 instr) ≤ text (21399); the "binary is not slower" claim holds at
- `tests/zero_alloc_frame_rx.rs`; counting-allocator proof of the
- **Binary message support**; the wire model is now `codec::Frame`:
    parsed as text/JSON; the JSON decoders take `&str`, so the type system
- **Compression** (`compression` feature, off by default); application-layer
    do not implement RFC 7692 and expose no extension hook; the module
  - Bench: `compression_roundtrip`; 2 KiB chat JSON 11.9×, repetitive
- **Redis room adapter** (`redis` feature, off by default); multi-node
  - **Semantics documented honestly: at-most-once, best-effort**; Redis
- **Backpressure stats hook**; `stats::StatsRecorder` (cheap-to-clone,
  lock-free): connections, rooms, messages/bytes in/out, drops; global +
- **`metrics` facade feature** (off by default); the same counters emitted
  returns `broadcast::Receiver<Frame>`; match on the variant or use
  window; SLO < 150 ns), binary ~81 ns; **binary does not slow the text
  - `origin_allowed_in_parts(&Parts, &allowed)`; call in the upgrade
  - `normalize_origin`; lowercase scheme/host, default ports (`http`/`ws`
  - **Default (empty list) remains allow-all**; documented residual risk
  slicing); the CI `-D warnings` gate now passes.