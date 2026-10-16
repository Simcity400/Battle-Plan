# collector-logistics

Concurrent observation of native logistics balance writes and their available-stock operands. Writers retain independent cell refusals, prepared storage, first observations and lifetime checks. Hooks are ordinary Rust library code linked into the existing collector DLL.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-logistics` and `cargo clippy -p collector-logistics --all-targets -- -D warnings` for its synthetic checks.
