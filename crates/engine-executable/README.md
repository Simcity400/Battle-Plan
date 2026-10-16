# engine-executable

Verified executable identity, file-backed image reads and immutable decoded executable metadata. It reads the selected installation and verifies existing size, hash and semantic digests before publishing the process-wide tables.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are the executable decoder and hashing libraries. Tests keep their synthetic fixtures and existing ignored scale gates.
