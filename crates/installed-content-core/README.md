# installed-content-core

The equipment database retains ordered module and upgrade stat groups, including
explicit zero average terms, and module conversion/XP operands. Stat arithmetic
keeps source order, fixed-point wrapping, average presence and integer-stat
rounding. Module XP defaults come from the resolved defines; explicit zero costs
remain distinct from absent overrides. Generic upgrade costs retain each
definition's currency and normal/linear mode, separate licensed linear costs,
signed dword step wrapping and the native order of final factor arithmetic.
Module mission-specific terms retain their authored mission names and source
groups separately from the saved mission-result caches. Equipment mission names
resolve through the executable equipment reader's own bit domain.
Faction goal content retains native-loader-reached conditions, flags, effect bodies, ordered ratio operands and progress sections. Repeated scalar operands replace their earlier declaration; section occurrences append in content-stack order. Inline identity ambiguity and unreadable content remain named refusals.

Verified content stacks, shared text readers, script dispatch, authored catalogues and map/asset foundations for Battle Plan. The [installed-content guide](../installed-content/README.md) describes content identity, native reader and refusal contracts. Public reader paths are also exported through `installed-content`. This crate reads installations and synthetic fixtures; it does not read live memory or alter installed content.

`faction_goals` shares the native goal field reader with faction templates, retaining bound names/descriptions and authored categories. Standalone declarations merge by their declaration identity; an `id` field does not rename a goal. Inline declarations are independent objects, and token identities naming distinct inline definitions are refused and counted. `cached` binds the catalogue and failures to the resolved layer roster.

It builds on [engine-executable](../engine-executable/README.md) (the executable tables) and [replay-identity](../replay-identity/README.md) (the recorded content identity) only, so it compiles alongside the native readers and the replay chain rather than after them, and edits there do not rebuild it.

## Layout

- `content`: recorded identities, hashes, ordered folder/archive mounts, lookup and shared derived caches.
- `parallel`: independent work on the machine's threads, results in order. `ResolvedLayers::read_paths` reads many files at once, each exactly as `read_path` reads it alone: a stacked lookup probes every layer above the one that answers, and those probes dominate a bulk read. The `.gui` and `.gfx` registries read and parse their files this way; GUI files that generate element names are re-read from their place in the shared counter, so names match a read in turn.
- `parser`, `crt`, `fixed5`, `fields`: byte-preserving trees, native script consumption, number conversion and named scalar refusals.
- `key_names`, `combat_tactics`, `trigger_keys`, `effect_keys`, `modifiers`: load-point identity and shared dispatch.
- Authored catalogues retain their database-specific order, duplicate identity and coverage refusals.
- `gui`, `pdxshader`, `mesh`, `particles`: native GUI, shader declarations and authored model/particle data. Bitmap decoding is exported from [installed-content-image](../installed-content-image/README.md). The registered descriptor roster (`mesh::Roster`, memoized as `ResolvedLayers::descriptor_roster`) keeps each registered clone's parent (`clones`) beside the flattened copy, and `Roster::refused_declaration` names the first declaration an entity uses that cannot be played as registered: one a descriptor file that does not parse may hold, an attachment cycle, or a non-positive `animation_speed`.
- `map`, `movement`, `province_geometry`, `province_graph`: definition rows, rasters, native geometry and adjacency foundations.

Resolve every authored fact through `ResolvedLayers`, preserving vanilla/DLC/mod layer order, replace_path, filename bytes, recorded coverage and verified reads. Keep the same stack-scoped cache ownership, native recovery, diagnostics and missing-input refusals. Renderer and recorded-world evaluation remain consumers.

## Tests

Synthetic tests live beside their readers. `layer-fixtures` exposes the existing synthetic `Stack` and `ResolvedLayers::from_folders` constructors for dev-dependencies. `test-support` also enables the existing catalogue/image fixtures. The two features keep default test gating separate from cross-package layer construction. Runtime metadata tests initialize the supported installation through `BATTLE_PLAN_GAME_DIR`; real-installation tests remain ignored and read-only. Follow [AGENTS.md](../../AGENTS.md) for formatting, clippy and tests.

The MIO trait catalogue keeps the initial trait separate from ordinary traits and retains local coordinates, relative anchors, parent groups, exclusions and equipment filters. Position resolution uses the native first-match anchor lookup and wrapping coordinates; unresolved cycles are named refusals.

Equipment slot required flags and category declarations preserve slot order, aliases, inherited slots, duplicate skipping and generated slot removal. Designer-family predicates traverse archetype chains using each type's own categories and default modules. Organisation cost gates retain the organisation-level available/visible bodies separately from initial-trait modifiers; inherited gates select the parent local body. Organisation modifier inheritance resolves effective emptiness through recorded modifier eligibility, preserving tooltip copy order at each include boundary.

Module build, dismantle and source-specific conversion resource pools replace
repeated resource tokens in their original position, including zero amounts.
Upgrade resource thresholds reset on each block, sort by level and retain the
last whole pool for duplicate levels. Industrial consumers resolve these names
through recorded resource definitions and calculate the selected pair only.

Equipment validity joins explicit assignments with inherited required/default slots, category dependencies, present-key module counts and local count overrides. Role queries retain type overrides and exact-role bans separately from category overlap. Rejected module tokens resolve to the native empty definition.

The unit-leader catalogue resolves trait training costs through the recorded stack, retaining native integer conversion and the constructor default when a cost is absent.

The unit-medal reader preserves native definition order and duplicate identities, resolves variables through the recorded stack and reads the unit, leader and ship modifier domains and on-grant effects.
The doctrine catalogue retains authored base costs, bound descriptions, reward
requirements and milestone/reward benefits. Doctrine bonuses share the
technology reader's registered unit/category stat forms and the common modifier
reader, preserving mod rosters, repeated definitions and native order.
The special-project catalogue resolves generic and inline prototype rewards through one ordered occurrence catalogue and the global reward lookup, without a duplicate inline projection. It retains narrative keys, ordered options (including repeated tokens) and every authored iteration-output effect family, reusing the native content readers.

Scientist visibility resolves the retained role conjunction by its exact source file and visible-block line, including replacement roles nested in effects and repeated visible blocks. Prototype rewards retain their ordered inline/reference occurrences across both reward blocks and repeated project definitions; pending native occurrence indices select exact duplicate inline objects.

Faction member upgrades resolve standalone and inline definitions from the recorded stack's member-upgrade and member-group directories. The reader retains numeric bonuses, costs and group types, distinguishes inline identity collisions, and keeps AI weight syntax separate from inline upgrade declarations. Faction goal catalogues expose their definitions for candidate-specific command checks with the same coverage and ambiguity rules as direct lookup.
The special-project catalogue resolves generic and inline prototype rewards through one ordered occurrence catalogue and the global reward lookup, without a duplicate inline projection. It retains narrative keys, ordered options (including repeated tokens) and every authored iteration-output effect family, reusing the native content readers.

Member-upgrade and member-group readers retain a leading UTF-8 byte-order mark in the first identity, matching both native database loaders.
Runtime trigger dispatch can carry a caller's typed source error through
`trigger_keys::dispatch_with_error`; the content reader's `dispatch` keeps its
String diagnostics. Dispatch adds no error identity and does not format a runtime
failure. Synthetic evaluator tests exercise both interfaces.
Decision content retains complete, remove, cancel and timeout effect bodies separately for scoped descriptions.

Decision catalogues retain category and decision allowed/visible/available conditions, custom cost/activation/remove/cancel triggers and scoped cost operands for viewer roster evaluation. Repeated trigger bodies append in load order; mods resolve through the recorded stack.

Aircraft content retains each equipment type's signed air_map_icon_frame and carrier_capable fields, including their reset and clone rules. Sub-unit content resolves the four Air wing sizes against its first equipment requirement, applying the native land/carrier zero defaults and invalid non-carrier correction. These supply Air wing icon frames and capacity bars from the recorded stack.

Sub-unit carrier admission resolves the first equipment requirement's carrier_capable value separately from the authored wing capacity.

Province geometry exports its native fixed5 square-root helper for simulation consumers. Small/large scaling, Newton tolerance and signed wrapping remain shared, so Navy Air disruption and geometry keep the same integer rounding.

Occupation modifier definitions read the recorded stack's direct resistance/compliance modifier files with exact name identity and whole-object duplicate replacement. The catalogue retains definition kind, authored large sprite/frame, threshold and margin, tooltip key, modifier fields and the separate visible, enabled, enable-effect and disable-effect bodies. Invalid types remain unregistered; unread suffixes produce named, counted refusals until a later definition establishes the requested name. Active membership comes from recorded retained identities.

Mesh animation frames can stream joint records into retained sampling storage.
`mesh::PoseBinding` retains a mesh/animation name join for repeated sampled poses,
including last supplied duplicate animation names, first skeleton-index matches,
partial animations, bind fallbacks and the existing bounded parent walk. It changes
neither sample arithmetic nor skeleton order.
