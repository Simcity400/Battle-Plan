# recorder-oracle-projects

Special-project and raid identities, equipment, state and path comparisons. Owns [projects.rs](src/projects.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Project comparison includes every saved primary/supporting scientist assignment and delayed timer, pending reward identity and project variable values/seeds. Scientist comparison uses the injury timer's positive-only serializer projection without changing its signed recorded value.
