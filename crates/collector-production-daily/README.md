# collector-production-daily

Daily production slot capture: IndustrialOrganisations, CountryProductionModifiers and ProductionLineModifiers, the latter read in up to four consecutive census slot ranges. The host owns dispatch, the range split, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-production-daily` and `cargo clippy -p collector-production-daily --all-targets -- -D warnings` for its synthetic checks.
