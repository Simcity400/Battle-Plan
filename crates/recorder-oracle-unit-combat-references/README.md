# recorder-oracle-unit-combat-references

Unit-to-combat identity and membership comparisons. Owns [unit_combat_references.rs](src/unit_combat_references.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

A present diagnostic movement-speed refusal fails. A native speed has no saved
counterpart; its exact same-boundary comparison belongs to recorder-derived.
