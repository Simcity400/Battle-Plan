# collector-air-records

Regional air combat capture: AirRegionCombat, reading a holder only where the air combat record updater wrote it and carrying the others, with that updater's observer. The host owns dispatch, loans and table assembly.

Touched tails update the held vectors in place, retaining equal damage buffers and comparing only the replaced suffix. Prune or tail changes still rebuild the shared publication; a refused update discards the incomplete baseline.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-air-records` and `cargo clippy -p collector-air-records --all-targets -- -D warnings` for its synthetic checks.
