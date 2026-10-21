# collector-naval-counters

Naval counter capture: the task force counter inputs and their supply operands, carried under the naval tree observer's certificates (collector-observers). The host owns the ordinary task force part's dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-naval-counters` and `cargo clippy -p collector-naval-counters --all-targets -- -D warnings` for its synthetic checks.

The existing asynchronous fleet census forwards its already-observed mission-region header. Its payload is acquired only on first observation or a changed fleet-region writer certificate and stored as a shared sorted multiset. No-op native mutation calls and pure permutations carry the payload. Disabled observation withholds this field by name rather than adding periodic reads; quiet wire rows clone the shared owner without per-fleet payload allocation.

The writer token starts before the original region-header read and is checked again before using either the held payload or a changed payload. A writer completing during that header copy cannot certify its old count under the newer token; the affected field is withheld and the next stable census retries. Header/payload races invalidate only this observation. The original header is reused without another native read, and quiet certificates read no vector payload.

The recorder hands refusal diagnostic scans and retired-fleet region-map pruning
to FIFO decode on this frame's owned census. That tail accepts no Memory and
finishes before the frame can be reused. Native-demand selection, receiver
registration, held-census certification/comparison and its watch-poison verdict
remain parked; direct captures run the same owned tail synchronously.
Reset settles completed diagnostics before a sibling failure clears their rows.
