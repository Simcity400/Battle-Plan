# collector-timing

Sampled per-thread hook timing, TSC calibration, original-call accounting and borrowed status output. Timer reads and sampling policy belong here; hook installation and capture-family behavior belong to their callers.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-timing` and `cargo clippy -p collector-timing --all-targets -- -D warnings` for its synthetic checks.
