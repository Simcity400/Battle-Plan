# engine-land-inputs

Native land inputs readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-air-history](../engine-air-history/README.md), [engine-deployment](../engine-deployment/README.md), [engine-executable](../engine-executable/README.md), [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.
