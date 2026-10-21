# recorder-oracle-combat-details

Retained combat detail rows, commander operands and nested value trees. Owns [combat_details.rs](src/combat_details.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Land-combat side country sets accumulate joining logical owners and survive ownership transfers, including expeditionary returns. The save serializes neither set, and its current division owners cannot certify set membership. `LandCombatSides` compares the combat census, rejects null or duplicate combat handles and resolves raw primary country references; the ordered lists, secondary sets, indicator and flags need a matched paused live-memory check. Current roster, logical and expeditionary owners still compare exactly in [recorder-oracle-military's `compare_divisions`](../recorder-oracle-military/src/military.rs).

Naval current commanders compare with the saved `last_leader` while nonnull. A null current selection preserves that retained leader at any hour, so it has no exact save counterpart and receives a named limitation. Other side operands and wrong nonnull commander identities still compare exactly.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.
