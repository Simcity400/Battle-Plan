# installed-content-country-names

Recorded country naming witnesses, rule selection and name templates. The implementation and its tests live in [`country_names`](src/country_names.rs); the same public paths are exported through `installed_content::country_names`. Shared parsing, identity and authored foundations come from [installed-content-core](../installed-content-core/README.md).

This reader also consumes [installed-content-localisation](../installed-content-localisation/README.md).

Resolve every authored input through the recorded `ResolvedLayers` content stack, including folders, archives, DLC, mods and replace_path. Preserve native order, lookup identity, duplicate handling, recovery and named/counted diagnostics. Keep evaluation and rendering in consumers.

## Tests

Synthetic tests live beside the reader. Integration tests of independent readers live in `installed-content`; layer fixtures come from the core through dev-dependencies. `test-support` forwards the existing core test constructors and retains reader test gating. Runtime metadata tests initialize `BATTLE_PLAN_GAME_DIR`; ignored installation tests remain read-only. See the [installed-content contracts](../installed-content/README.md) and [project rules](../../AGENTS.md).
