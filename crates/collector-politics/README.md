# collector-politics

Political event capture: the observer of native political writers (elections, ruling parties, ideas, diplomatic dispatch and civil wars, forwarded to the country input, event feed and war provenance observers in collector-observers) and the pack that converts its drained events into the hour's tables. The host installs the hooks, drains at the hourly boundary and assembles the tables.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-politics` and `cargo clippy -p collector-politics --all-targets -- -D warnings` for its synthetic checks.
