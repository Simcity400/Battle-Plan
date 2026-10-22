# collector-raw

Ordered owned raw-copy spans, typed decoding over those bytes, occurrence-aware lookup and prepared copy plans. Captured reads preserve exact addresses, order, duplicates, faults and table ownership. No deferred reader follows live pointers.

Certified carried segments append their relocated spans as one exact-length
extension after the original whole-segment capacity check. This avoids a
capacity test per span without changing ordering, aliases, masks, bulk lookup,
overflow or native-read accounting.

Independent replay scopes may retain separate `RecordedCache` indexes. Each
reader still validates its current span shape and owns independent occurrence
cursors; payload offsets and bytes always come from the current log. A cache
owns only charged index allocations, never raw payloads or native capabilities.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-raw` and `cargo clippy -p collector-raw --all-targets -- -D warnings` for its synthetic checks.
