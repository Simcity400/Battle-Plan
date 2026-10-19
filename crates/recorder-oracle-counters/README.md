# recorder-oracle-counters

Unit and headquarters counter operands and leader-picture inputs. Owns [counters.rs](src/counters.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

CountryCounterInputs sent-expeditionary references compare with the complete saved ordered list, preserving nulls, duplicates and stale references.
