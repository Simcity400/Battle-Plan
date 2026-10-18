# engine-governance-tests

Synthetic country worlds, native-address selections and guarded-call traces shared by the research and focus reader tests. It owns the common fixture once and retains the tests' existing 5,000-country cases and comparison semantics. The fixture performs no live capture. Development callers enable `test-support`; the default library has no fixture dependencies.

[engine-research](../engine-research/README.md) and [engine-focus](../engine-focus/README.md) use it as a development dependency. Shared rules live in [AGENTS.md](../../AGENTS.md).

The single fixture source is [kit.rs](src/kit.rs). Research, focus and collector
governance tests use this package with `test-support`; no consumer compiles a
source file from another crate. The default library has no fixture dependencies.
