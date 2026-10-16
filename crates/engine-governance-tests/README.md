# engine-governance-tests

Synthetic country worlds, native-address selections and guarded-call traces shared by the research and focus reader tests. It owns the common fixture once and retains the tests' existing 5,000-country cases and comparison semantics. The fixture performs no live capture. Development callers enable `test-support`; the default library has no fixture dependencies.

[engine-research](../engine-research/README.md) and [engine-focus](../engine-focus/README.md) use it as a development dependency. Shared rules live in [AGENTS.md](../../AGENTS.md).

The single fixture source is [test_kit.rs](../engine/src/governance_batch/test_kit.rs). The package includes it through its module path; collector tests include that same path through engine. Research and focus use this package with `test-support`. Each test binary retains its original fixture and counter identity.
