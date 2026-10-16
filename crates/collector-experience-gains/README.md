# collector-experience-gains

Experience gain capture: the order-independent observer that sums concurrent native experience writers into live-sized lock-free tables, with growth offered off-thread and adopted at the drain (`experience_gains`), and its hourly table pack (`tables`, the host's `extended::experience_gains`). The host installs the hooks, drains at the hourly boundary and assembles the tables.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-experience-gains` and `cargo clippy -p collector-experience-gains --all-targets -- -D warnings` for its synthetic checks.
