# recorder-oracle-characters

Character rosters, leader roles, stored names and portrait/codename inputs. Owns [characters.rs](src/characters.rs) and its child tests. The [oracle command](../recorder-oracle/README.md) composes comparisons; this crate calls its specific dependent comparisons directly.

Table completeness, exact joins, differences and named limitations use [shared oracle support](../recorder-oracle-support/README.md). Content-dependent facts resolve through the recorded content stack. Synthetic fixtures are dev-only through the support crate's `test-support` feature.

Shared rules live in [AGENTS.md](../../AGENTS.md); the [recorder guide](../recorder/README.md) defines fresh-save/load and matched-native-observation checks.

Commander comparisons include residual experience and complete trait-training membership, order, repeated definitions and progress, at the save writer's precision.

The commander training comparison also checks every ordered runtime XP-factor override, including repetitions and signed fixed5 values.
