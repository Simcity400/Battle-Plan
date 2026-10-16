# engine-raid-inputs

Native raid inputs readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-executable](../engine-executable/README.md), [engine-intelligence](../engine-intelligence/README.md), [engine-native](../engine-native/README.md), [engine-projects](../engine-projects/README.md), [engine-resources](../engine-resources/README.md), [engine-stability](../engine-stability/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.
