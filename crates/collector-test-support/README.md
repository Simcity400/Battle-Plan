# collector-test-support

The shared synthetic allocation-denial allocator, pre-main test-stack reservation and per-call benchmark helper for collector libraries. This package is a development dependency; each test executable has one allocator and one set of controls.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-test-support` and `cargo clippy -p collector-test-support --all-targets -- -D warnings` for its synthetic checks.
