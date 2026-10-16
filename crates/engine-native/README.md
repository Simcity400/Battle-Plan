# engine-native

Native layouts, guarded memory operations, shared containers, authentication and coupled native readers. The existing Memory capabilities, native observation order and refusal semantics belong to this crate. The SEH copy helper is compiled and linked here once.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-executable](../engine-executable/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.
