# installed-content-equipment-graphics

Equipment graphics pools, contexts, lookup identity and map/state joins. The implementation and its tests live in [`equipment_graphics`](src/equipment_graphics.rs); the same public paths are exported through `installed_content::equipment_graphics`. Shared parsing, identity and authored foundations come from [installed-content-core](../installed-content-core/README.md).

This reader also consumes [installed-content-states](../installed-content-states/README.md).

Resolve every authored input through the recorded `ResolvedLayers` content stack, including folders, archives, DLC, mods and replace_path. Preserve native order, lookup identity, duplicate handling, recovery and named/counted diagnostics. Keep evaluation and rendering in consumers.

## Tests

Synthetic tests live beside the reader. Integration tests of independent readers live in `installed-content`; layer fixtures come from the core through dev-dependencies. `test-support` forwards the existing core test constructors and retains reader test gating. Runtime metadata tests initialize `BATTLE_PLAN_GAME_DIR`; ignored installation tests remain read-only. See the [installed-content contracts](../installed-content/README.md) and [project rules](../../AGENTS.md).
