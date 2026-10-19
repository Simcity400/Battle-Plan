# recorder-oracle-wars

War participants and directed relation comparisons. Owns [wars.rs](src/wars.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

The participation comparison joins both saved directional score blocks independently of integer casualties and compares complete captured-province membership. Delivery-history comparison includes every nonzero native history of countries named by recorded war rosters, including retained/asymmetric endpoints and targets outside the displayed side. It joins actual recorded/saved histories without enumerating candidate pairs; playback owns the same-side gate. Missing rows in the complete sparse history table certify exact zero; an omitted save block is compared by its native binary-zero gate. Runtime unknown-casualty score and computed total/share remain explicit unsaved limitations.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

The war-goal inventory comparison checks every country, including empty inventories: full handles and roster order, unused references, actor/recipient/type, all target occurrences, expiry and every serialized justification field. Native scalar/date omission gates are compared explicitly.
