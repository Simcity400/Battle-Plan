# viewer-counter-products

Unit, task-force and province hover text, fresh counter route and arrival queries, selected movement routes and battle-plan products. The host supplies a fresh immutable content identity containing concrete recorded-stack facts and composes the products with counter grids, raids and world submission.

Shared drawn-control observations, authored GUI fallback, stack anchors, counter input/action/status folds, movement reads and timing come from viewer-foundation. This crate owns hover caches and query demand gates; counter grid lifecycle and mutable selection belong to viewer-counters. It imports neither the host nor another viewer feature. Content facts resolve through installed-content; queries retain occurrence order and counted refusals.

The province hover follows the map mode ([mode_lines.rs](src/map/province_tooltips/mode_lines.rs)): the location block, units and selection only in the default and states modes (states adds the state's live category, States `category`); manpower, infrastructure (`MAPMODE_TOOLTIP_INFRASTRUCTURE` with the state's level, zero for a province outside every state), diplomacy, ideology, resistance and compliance, and scripted modes show their own lines alone; factions, players, air, navy and raids show nothing of their own. The terrain mode's hover is viewer-combat's ([terrain_mode.rs](../viewer-combat/src/terrain_mode.rs)), which the app asks instead. The resources and operatives modes' lines are not built and are refused by name; a scripted mode's `<mode>_tooltip` and `_tooltip_delayed` expand through the shared scripted text in the game's scopes (ROOT, THIS and PREV the viewing country, FROM the hovered province's state, whose FROM is the viewing country), and a scope function it does not support is refused by name; the state resistance breakdown is refused.

Hover caches retain content and frame identities. Reached route queries build their whole-frame indexes lazily within each hover or diagnostics batch.

Tests live with their modules. `slow-tests` preserves existing gates; `test-support` enables shared synthetic foundation inputs for consumers. Follow the workspace [working rules](../../AGENTS.md).
