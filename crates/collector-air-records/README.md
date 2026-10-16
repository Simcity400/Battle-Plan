# collector-air-records

Regional air combat capture: AirRegionCombat, reading a holder only where the air combat record updater wrote it and carrying the others, with that updater's observer. The host owns dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-air-records` and `cargo clippy -p collector-air-records --all-targets -- -D warnings` for its synthetic checks.
