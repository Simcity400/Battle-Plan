# installed-content-settings

Read-only local game settings, display preferences and texture decoding selection. The implementation and its tests live in [`settings`](src/settings.rs); the same public paths are exported through `installed_content::settings`. Shared parsing, identity and authored foundations come from [installed-content-core](../installed-content-core/README.md).

Read local settings without writing them. Preserve native defaults, stored values, display-index rules and the selected texture decode flags.

## Tests

Synthetic tests live beside the reader. Integration tests of independent readers live in `installed-content`; layer fixtures come from the core through dev-dependencies. `test-support` forwards the existing core test constructors and retains reader test gating. Runtime metadata tests initialize `BATTLE_PLAN_GAME_DIR`; ignored installation tests remain read-only. See the [installed-content contracts](../installed-content/README.md) and [project rules](../../AGENTS.md).
