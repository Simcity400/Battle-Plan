# recorder-oracle-economy

Country economy, fuel, stockpile, resource and trade comparisons. Owns [economy.rs](src/economy.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Market comparisons project native pool zero omission and normal-price omission explicitly, preserving order, full identities, subsidy alternatives, independent price/quantity pools, runtime/history scalars and contract variables. Current lend-lease schedules, dates, delivered/lost pools and fuel words compare independently. Native occurrence order remains checked even when the save no longer contains a completed agreement.

MarketEvents amount-refusal rows make the comparison incomplete even when all surviving state matches the save. Every refusal reports its occurrence, hour, country, full holder identity, amount source and named reason; refusals do not count as compared events. Independently complete occurrences remain checked, including their ordering.
