# installed-content-wargoals

Authored war-goal definitions and naming fields. The implementation and its tests live in [`wargoals`](src/wargoals.rs); the same public paths are exported through `installed_content::wargoals`. Shared parsing, identity and authored foundations come from [installed-content-core](../installed-content-core/README.md).

Resolve every authored input through the recorded `ResolvedLayers` content stack, including folders, archives, DLC, mods and replace_path. Preserve native order, lookup identity, duplicate handling, recovery and named/counted diagnostics. Keep evaluation and rendering in consumers.

`Database::terms` projects annexation, ordered allowed/available action conditions and peace-term conditions, limits, cost and threat factors, generation costs and default expiry. Repeated trigger blocks append in authored order. Repeated scalar assignments follow native conversion and factor arithmetic. Same-token definitions with differing terms refuse the lookup; no definition pointer is inferred from a recorded token. Term conditions remain separate from action eligibility.

## Tests

Synthetic tests live beside the reader. Integration tests of independent readers live in `installed-content`; layer fixtures come from the core through dev-dependencies. `test-support` forwards the existing core test constructors and retains reader test gating. Runtime metadata tests initialize `BATTLE_PLAN_GAME_DIR`; ignored installation tests remain read-only. See the [installed-content contracts](../installed-content/README.md) and [project rules](../../AGENTS.md).
