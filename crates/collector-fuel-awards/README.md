# collector-fuel-awards

Fuel award capture: the observer of applied country fuel updates (`fuel_awards`) and the pack that sums each country's last 24 applied updates into CountryFuel's daily suffix (`tables`, the host's `extended::fuel_awards`). The host installs the hooks, drains at the hourly boundary and assembles the tables.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-fuel-awards` and `cargo clippy -p collector-fuel-awards --all-targets -- -D warnings` for its synthetic checks.

Air receipt taps retain fuel awards' own exact observation without forwarding container stores to a separate fuel certificate. Detach stops receipt admission before restoring taps and waits for admitted callbacks to complete.

Held-read diagnostics use the timer's owned thread counters, avoiding shared
atomic increments. Writer epochs, boundary stamps and receipt publication are
independent and keep their existing fences.
