# recorder-oracle-air

Regional air-combat records and air-force save comparisons. Owns [air.rs](src/air.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

RaidAirScores' weights, movement impact and regional aircraft spotting scale are omitted by the native save serializer. The oracle names that limitation; integration compares these unsaved operands against the matched native phase observation and the viewer's Air/Navy results.
