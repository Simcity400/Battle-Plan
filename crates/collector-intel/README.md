# collector-intel

Intel capture: country intel, agencies, operations and operatives, script flags and script variables, the shared daily CountryIntel buffer, and the flag and variable writer observers that let quiet scopes carry their rows. The host owns dispatch, loans, the spool conversion caches it lends and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-intel` and `cargo clippy -p collector-intel --all-targets -- -D warnings` for its synthetic checks.

`AgencyOperationBuffers` reads the four shared agency UI lists every hour, including stale and empty snapshots. Header-sized worker storage grows through the guarded allocation path; unchanged snapshots use the ordinary row delta carry. This client cache is not a reconstructed per-country saved roster.
