# collector-timing

Sampled per-thread hook timing, TSC calibration, original-call accounting and borrowed status output. Timer reads and sampling policy belong here; hook installation and capture-family behavior belong to their callers.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-timing` and `cargo clippy -p collector-timing --all-targets -- -D warnings` for its synthetic checks.

`faction_resource_rebuild` measures the bounded extraction observer separately from scalar companion taps. `faction_refresh_phases` excludes native originals while measuring phase/lifetime observation around complete refreshes and their parallel children.

The faction extraction family is a detour: its timing excludes the native rebuild and includes the bounded final-pool observer after return.

The market family combines native call detours and delivered/lost fuel store
taps. Quiet zero fuel operands bypass timer sampling with all capture work.
