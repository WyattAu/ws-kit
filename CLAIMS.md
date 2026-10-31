# Performance claims inventory: ws-kit
- **criterion**; wall-clock bench, noisy across machines/loads; the number
- **iai**; iai-callgrind instruction counts under valgrind; reproducible
- **test**; unit/integration test, including the counting-allocator gate
## Latency (criterion: measured-on records)
## Message-model cost equality (iai: instruction counts)
## Allocation profile (counting allocator: runs on every `cargo test`)
| 16 | `Frame::Binary` clone is a refcount bump; no allocation | same test (delta = 0 over 100 iters) | **proven** |
## Compression ratios (deterministic: re-verified 2026-09-12)
- **Deleted/reworded:** 1 (claim 19; the "not yet verified" disclaimer,