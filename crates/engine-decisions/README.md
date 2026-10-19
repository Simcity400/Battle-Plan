# engine-decisions

Native decisions readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

Event identities retain both native reference words and their content token. Targeted decision selector views remain separate from definition and instance pointers; the selected instance lifecycle distinguishes execution from mission completion. Shared event-feed entry descriptors keep installation and patch-collision validation aligned.

Authored cancel/remove effect entry descriptors identify execution, independently of later native status transitions.

`decision_runtime` authenticates only the selected retained record and its definition gate. Demanded owner baselines use the same container layouts; target/ignore and timer-pass operands are purpose-named constants.
