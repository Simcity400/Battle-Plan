# recorder-oracle-visibility

Country access, visibility and presentation operand comparisons. Owns [visibility.rs](src/visibility.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

The Air volunteer stockpile comparison joins every saved stockpile variant to its recorded archetype, groups raw quantities before whole-plane conversion and compares the uncapped signed sender total. Classification resolves through the verified recorded equipment database.
