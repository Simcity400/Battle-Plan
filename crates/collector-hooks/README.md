# collector-hooks

Windows x64 patch publication, thread suspension and synchronization, Tap bridges and publication measurements. These ordinary Rust library modules are linked into the collector DLL. They preserve native registers, flags, unwind registration and continuation, and retain executable storage whenever restoration or quiescence cannot be proved.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-hooks` and `cargo clippy -p collector-hooks --all-targets -- -D warnings` for its synthetic checks.
