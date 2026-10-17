# engine-resources

Native resources readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

Resource origins and resource-right entries retain their native list positions. Convoy clients retain independent endpoint list positions for supply transfers, outgoing/received lend-lease and sold/bought market contracts. Directional joins authenticate client identity and the country market-manager backlink; positions are never inferred from handles or global contract ownership order.

Market-referenced routes acquire native state endpoints from the authenticated route header independently of naval port provinces, checking both state vtables and state-roster backlinks.
