# collector-output

Output totals capture: the production line output and construction IC observers (`line_output`, `construction_ic`) with their shared hook sites (`output_hooks`), their hourly table packs (`line_tables` and `construction_tables`, the host's `extended::line_output` and `extended::construction_ic`), rocket output and the output continuity both packs carry. The host installs the hooks, drains at the hourly boundary and assembles the tables.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-output` and `cargo clippy -p collector-output --all-targets -- -D warnings` for its synthetic checks.
