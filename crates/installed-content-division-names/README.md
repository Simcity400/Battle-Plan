# installed-content-division-names

Authored division and raid naming groups, tokens and diagnostic identity. The implementation and its tests live in [`division_names`](src/division_names.rs); the same public paths are exported through `installed_content::division_names`. Shared parsing, identity and authored foundations come from [installed-content-core](../installed-content-core/README.md).

Resolve every authored input through the recorded `ResolvedLayers` content stack, including folders, archives, DLC, mods and replace_path. Preserve native order, lookup identity, duplicate handling, recovery and named/counted diagnostics. Keep evaluation and rendering in consumers.

## Tests

Synthetic tests live beside the reader. Integration tests of independent readers live in `installed-content`; layer fixtures come from the core through dev-dependencies. `test-support` forwards the existing core test constructors and retains reader test gating. Runtime metadata tests initialize `BATTLE_PLAN_GAME_DIR`; ignored installation tests remain read-only. See the [installed-content contracts](../installed-content/README.md) and [project rules](../../AGENTS.md).
