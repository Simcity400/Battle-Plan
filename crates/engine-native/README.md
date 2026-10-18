# engine-native

Native layouts, guarded memory operations, shared containers, authentication and coupled native readers. The existing Memory capabilities, native observation order and refusal semantics belong to this crate. The SEH copy helper is compiled and linked here once.

Consumers depend on this library and the native domain libraries they use.
[engine](../engine/README.md) remains a public module index and integration-test
host. Date layout, saved allocation counters and modifier breakdown readers
also live here; none requires the complete engine facade. Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-executable](../engine-executable/README.md) and [engine-allocation](../engine-allocation/README.md), re-exported here as `allocation`. Tests keep their synthetic fixtures and existing ignored scale gates.
