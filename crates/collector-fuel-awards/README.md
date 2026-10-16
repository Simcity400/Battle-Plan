# collector-fuel-awards

Fuel award capture: the observer of applied country fuel updates (`fuel_awards`) and the pack that sums each country's last 24 applied updates into CountryFuel's daily suffix (`tables`, the host's `extended::fuel_awards`). The host installs the hooks, drains at the hourly boundary and assembles the tables.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-fuel-awards` and `cargo clippy -p collector-fuel-awards --all-targets -- -D warnings` for its synthetic checks.
