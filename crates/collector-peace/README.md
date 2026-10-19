# collector-peace

Peace capture: the peace conference observer (`peace_settlements`), its hourly table pack (`tables`, the host's `extended::peace_settlements`), the peace bid pack and the synthetic peace world fixture their tests share. The host installs the hooks, drains at the hourly boundary and assembles the tables.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-peace` and `cargo clippy -p collector-peace --all-targets -- -D warnings` for its synthetic checks.

Finalizer hook failure affects PeaceSettlements. PeaceBids polls resident conference history independently and retains its own successful samples while the finalizer family is refused.
