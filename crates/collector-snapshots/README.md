# collector-snapshots

Owned dependency traces and prepared read plans that certify unchanged capture results. Changed controls and failed prepared reads resume the ordinary reader from the observed prefix, preserving order and avoiding retries. Canonical-range checking is shared with raw-copy capture.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-snapshots` and `cargo clippy -p collector-snapshots --all-targets -- -D warnings` for its synthetic checks.
