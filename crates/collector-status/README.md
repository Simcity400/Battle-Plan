# collector-status

Borrowed status objects, sequences, formatting adapters and checked streaming JSON serialization used by capture diagnostics. Construction allocates no heap storage. The DLL host owns whole-capture status composition and fault exercises.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-status` and `cargo clippy -p collector-status --all-targets -- -D warnings` for its synthetic checks.
