# engine-intelligence

Native intelligence readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

[Agency operation buffers](src/agency_operation_buffers.rs) passively capture the shared client UI buckets in native visit order, retaining stale/empty membership and unresolved references. Their resolved target states are authenticated separately from saved operation-roster admission.
