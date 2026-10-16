# collector-memory

Terminal witness storage, scoped allocation context, checked raw-reserve diagnostics, per-table native read accounting and optional fault injection. The collector host installs the native allocation callbacks and cancels its frame wait after the first failure is published. Witness bytes and committed-watermark order are owned here.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-memory` and `cargo clippy -p collector-memory --all-targets -- -D warnings` for its synthetic checks.
