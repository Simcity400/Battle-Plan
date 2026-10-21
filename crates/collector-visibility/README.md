# collector-visibility

Country visibility capture: CountryVisibility, carrying an unmarked country's raw accumulator entry under the country input family's marks, with the live definition and eligibility metadata applied every hour. The host owns dispatch, loans and table assembly.

Owner-approved graduation (2026-10-20) verifies carried raw entries on the 576-hour rotation. `visibility_verify` still compares every eligible held country; missed-write detection poisons the same visibility domain, with unchanged epoch, identity and read-refusal rules.

The existing state-visibility census also returns its acquired state-array extent,
including holes, for SimulationFeatures. It performs no second header read.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-visibility` and `cargo clippy -p collector-visibility --all-targets -- -D warnings` for its synthetic checks.
