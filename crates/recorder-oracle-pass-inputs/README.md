# recorder-oracle-pass-inputs

Naval, country and orders-group counter operands and leader-picture inputs. Owns [pass_inputs.rs](src/pass_inputs.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Orders-group headquarters context compares the saved nearest-front province, including nonnegative zero and the writer's omission of negative values. Missing acquisition fails the group comparison.

Fleet mission-region comparison sorts the saved identities and compares their complete multiset, including repeats, with the certified FleetCounterInputs payload; omission projects an empty vector and an unavailable captured field remains a difference.
