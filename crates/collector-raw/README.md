# collector-raw

Ordered owned raw-copy spans, typed decoding over those bytes, occurrence-aware lookup and prepared copy plans. Captured reads preserve exact addresses, order, duplicates, faults and table ownership. No deferred reader follows live pointers.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-raw` and `cargo clippy -p collector-raw --all-targets -- -D warnings` for its synthetic checks.
