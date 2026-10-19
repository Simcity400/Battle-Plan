# recorder-oracle-production

Production lines, variants, industrial organisations and output accounting. Owns [production.rs](src/production.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Saved design authors compare their raw handles under the independently current
DesignOrganisationLiveness witness. Frozen attribution and applied bonuses are
compared separately; disappearance never erases the raw author.
