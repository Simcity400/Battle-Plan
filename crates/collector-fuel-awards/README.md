# collector-fuel-awards

Fuel award capture: the observer of applied country fuel updates (`fuel_awards`) and the pack that sums each country's last 24 applied updates into CountryFuel's daily suffix (`tables`, the host's `extended::fuel_awards`). The host installs the hooks, drains at the hourly boundary and assembles the tables.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-fuel-awards` and `cargo clippy -p collector-fuel-awards --all-targets -- -D warnings` for its synthetic checks.

The existing Air receipt tap forwards a container notification to the Air-base fuel certificate. Its guard stays live until the next receipt or terminal update callback, after the original store, so another capture cannot certify the pre-store interval. This adds no patch, native read or callback allocation.

Detach stops receipt admission before restoring taps. Admitted callbacks and forwarded Air-store intervals retain the terminal tap until cleanup completes, including callbacks after admission stops. The callback guard precedes its admission check, so an admitted receipt cannot cross the drain check unnoticed. No extra patch or native read is added.
