# recorder-oracle-intel

Intelligence agencies, operatives and operation save comparisons. Owns [intel.rs](src/intel.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

`PropagandaEfforts` compares every saved strategic-operative slot, including reserved and unused-country slots, its logical effort order and both raw Q15 channels. Weekly drift is compared independently, using the loaded cache until a recorded daily writer certifies the sum of retained efforts. Missing/extra membership and a missing header fail comparison.

The active operative roster, including idle and training leaders without a mission, compares each leader's nonempty portrait sprite against saved `gfx`; an empty portrait requires the key to be omitted.

`IntelNetworkSummaries` compares each independent saved coverage count, controlled/core strength sum and maximum state strength by owner slot and target tag. It compares membership even for networks without states, rejects unknown/duplicate recorded pairs, and treats omitted saved cache operands as incomplete rather than zero.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

`AgencyOperationBuffers` compares resolved target states for every buffered reference that names a saved operation, using its saved province with verified content membership or its target country's saved capital and the saved state inventory. Unknown province membership makes the comparison incomplete. Membership, order and freshness remain a runtime-only census: reset clears the shared UI lists and load/open/daily refresh rebuilds them; the save has no membership, order or freshness counterpart. Serialized operation identities and target operands also compare under `Operations`; neither check infers UI membership from the saved roster.


`IntelContributions` and `IntelSources` compare ordered native extents, sparse dense values, keyed occurrences, all four signed lanes, static IDs/removals, dynamic pending/accumulator values, prior-day contributions, discriminants and held source references. Reserved ownership joins only through authenticated physical save-emission metadata. Native raw country-reference aliases serialize as tags; a reserved root with no positive physical emission has no saved counterpart.

`IntelNetworkAuxiliaries` compares every saved ordered auxiliary occurrence by owner/target, kind, subnetwork and ordinal. It joins raw country aliases through authenticated country identities and record token spellings through recorded lexical tokens. Independent subnetwork words compare directly; only previously recorded per-state and selected-subnetwork operands are joined rather than recaptured. Missing/extra/duplicate occurrence membership is a failure.
