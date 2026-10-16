# collector-slow-tables

The Slow job's single-file country tables: country factories, country tactic unlocks, faction theatres, logistics availability (from the collector-logistics balance observer) and modifier definitions. The host owns the Slow job's dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-slow-tables` and `cargo clippy -p collector-slow-tables --all-targets -- -D warnings` for its synthetic checks.
