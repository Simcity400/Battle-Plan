# recorder-oracle-combat-log

Combat-log records and their saved source joins. Owns [combat_log.rs](src/combat_log.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Retained comparisons check group and collection order, empty months, enemy equipment, all manpower counters, template statistics, repeated battle references, global record counts and both battle sides and arrays. Active observer comparisons include activity groups and their templates and damage, leader hours, damage attribution, progress, counters and serialized side pools; aggregate loss quantities reuse the matching LandCombatProgress acquisition.
