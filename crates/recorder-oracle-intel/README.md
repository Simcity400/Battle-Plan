# recorder-oracle-intel

Intelligence agencies, operatives and operation save comparisons. Owns [intel.rs](src/intel.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

The active operative roster, including idle and training leaders without a mission, compares each leader's nonempty portrait sprite against saved `gfx`; an empty portrait requires the key to be omitted.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

`AgencyOperationBuffers` compares resolved target states for every buffered reference that names a saved operation, using its saved province with verified content membership or its target country's saved capital and the saved state inventory. Unknown province membership makes the comparison incomplete. Membership, order and freshness remain a runtime-only census: reset clears the shared UI lists and load/open/daily refresh rebuilds them; the save has no membership, order or freshness counterpart. Serialized operation identities and target operands also compare under `Operations`; neither check infers UI membership from the saved roster.
