# engine-resources

The faction resource-pool reader retains every native slot, including slot zero, its token and exact signed amount. Zero amounts stay in replay rows; the save comparison alone applies the serializer's nonzero omission rule. Storage grows from the live pool header and is reused.

Native resources readers and their operands, layouts and synthetic tests.

The public capture API is available through [engine](../engine/README.md). Shared rules live in [AGENTS.md](../../AGENTS.md); field contracts, arithmetic, native gates and failure order stay beside each reader.

The module list is in [lib.rs](src/lib.rs). Production dependencies are [engine-native](../engine-native/README.md). Tests keep their synthetic fixtures and existing ignored scale gates.

Resource origins and resource-right entries retain their native list positions. Convoy clients retain independent endpoint list positions for supply transfers, outgoing/received lend-lease and sold/bought market contracts. Directional joins authenticate client identity and the country market-manager backlink; positions are never inferred from handles or global contract ownership order.

Resource-origin efficiency/route factors and demanded convoy-client efficiency/convoy words use ordered scattered copies of only their existing spans, after the same authentication gates. Failed prefixes retain the serial refusal and demand no later span; standalone bad-route checks still read no convoy counts.

Market-referenced routes acquire native state endpoints from the authenticated route header independently of naval port provinces, checking both state vtables and state-roster backlinks.

Demanded strategic-resource pools can copy their stored amounts into reusable
worker storage. The reader retains installed resource tokens and native order,
omits the null-resource slot as the serializer does, and performs no owner census.

`clend_lease_exchange` also reads selected agreement terms into reusable storage and identifies per-model delivery/loss stores without walking the country or exchange census. Monthly amounts, production shares, one-time amounts, active dates and the two fuel scales remain independent. Delivery/loss observations authenticate the reached exchange, giver resources and variant; unrelated pool-store callers acquire no lend-lease entity.

Rare agreement amendments stream all three ordered schedules into caller-prepared storage, including signed and zero entries. Header validation precedes publication; callbacks never allocate.
