# dashboard-text

The dashboard's text interpreter and the content facts it reads. It evaluates recorded triggers and scopes, renders scripted localisation and native effect descriptions, formats modifiers, and applies the game's name rules. It also holds the content catalogue and the recording metadata helpers that products and map tooltips share.

Nothing here names a product or a source. The [dashboard](../dashboard/README.md) loads one catalogue per source and folders, and re-exports every module at its old `dashboard::` path. The map's tooltip crates (viewer-foundation, viewer-world, viewer-combat, viewer-counters, viewer-counter-products and viewer-raids) depend on this crate and [dashboard-core](../dashboard-core/README.md) instead of the whole dashboard, so editing a product doesn't rebuild them.

| Module | What it does |
|---|---|
| [triggers.rs](src/triggers.rs), [scope.rs](src/scope.rs), [character_scope.rs](src/character_scope.rs) | Recorded-hour trigger evaluation, scope links, aliases and character operands. |
| [scripted_text.rs](src/scripted_text.rs), [effect_description.rs](src/effect_description.rs) | Scoped localisation and native effect descriptions, sharing the trigger/variable evaluator. |
| [modifier_text.rs](src/modifier_text.rs), [faction_names.rs](src/faction_names.rs), [unit_names.rs](src/unit_names.rs) | Shared modifier formatting and native stored/generated names. |
| [party_names.rs](src/party_names.rs) | The ruling party naming ladder shared by party histories and scripted text. |
| [idea_names.rs](src/idea_names.rs) | Country-specific idea names: an idea's selected localisation key or its generated advisor name. |
| [identity.rs](src/identity.rs) | Character and country identity operands names are chosen by: script tokens, templates, tags, looks and civil-war state. |
| [variant_names.rs](src/variant_names.rs) | Equipment variant names as the game writes them, and the operands that order variants. |
| [air_wing_names.rs](src/air_wing_names.rs) | Retained air wing names and the authenticated active-raid projection older prefixes use. |
| [flag_choices.rs](src/flag_choices.rs) | Flag asset candidates per recorded country at one hour, shared by the flag product and map tooltips. |
| [cryptology.rs](src/cryptology.rs), [naval_battle_name.rs](src/naval_battle_name.rs), [state_resources.rs](src/state_resources.rs) | Native helpers shared by products and viewer tooltips. |
| [catalogue.rs](src/catalogue.rs) | Shared lazy localisation, content databases, native GUI/sprite lookup, texture decoding and PNG replies; the per-source loader is in the dashboard. |
| [metadata.rs](src/metadata.rs) | Branch bounds, gaps, initial country catalogue, frame checks, country-link resolution and supplying-read detection. |

The `test-support` feature compiles the catalogue and scope fixtures (`catalogue::tests`, `catalogue::texture_tests`, `scope::tests`) and the test-only instrumentation, such as read counters and synthetic scripted environments, for the tests of crates above this one. Only dev-dependencies enable it.
