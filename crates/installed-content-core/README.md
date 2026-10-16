# installed-content-core

Verified content stacks, shared text readers, script dispatch, authored catalogues and map/asset foundations for Battle Plan. The [installed-content guide](../installed-content/README.md) describes content identity, native reader and refusal contracts. Public reader paths are also exported through `installed-content`. This crate reads installations and synthetic fixtures; it does not read live memory or alter installed content.

It builds on [engine-executable](../engine-executable/README.md) (the executable tables) and [replay-identity](../replay-identity/README.md) (the recorded content identity) only, so it compiles alongside the native readers and the replay chain rather than after them, and edits there do not rebuild it.

## Layout

- `content`: recorded identities, hashes, ordered folder/archive mounts, lookup and shared derived caches.
- `parser`, `crt`, `fixed5`, `fields`: byte-preserving trees, native script consumption, number conversion and named scalar refusals.
- `key_names`, `combat_tactics`, `trigger_keys`, `effect_keys`, `modifiers`: load-point identity and shared dispatch.
- Authored catalogues retain their database-specific order, duplicate identity and coverage refusals.
- `gui`, `pdxshader`, `mesh`, `particles`: native GUI, shader declarations and authored model/particle data. Bitmap decoding is exported from [installed-content-image](../installed-content-image/README.md). The registered descriptor roster (`mesh::Roster`, memoized as `ResolvedLayers::descriptor_roster`) keeps each registered clone's parent (`clones`) beside the flattened copy, and `Roster::refused_declaration` names the first declaration an entity uses that cannot be played as registered: one a descriptor file that does not parse may hold, an attachment cycle, or a non-positive `animation_speed`.
- `map`, `movement`, `province_geometry`, `province_graph`: definition rows, rasters, native geometry and adjacency foundations.

Resolve every authored fact through `ResolvedLayers`, preserving vanilla/DLC/mod layer order, replace_path, filename bytes, recorded coverage and verified reads. Keep the same stack-scoped cache ownership, native recovery, diagnostics and missing-input refusals. Renderer and recorded-world evaluation remain consumers.

## Tests

Synthetic tests live beside their readers. `layer-fixtures` exposes the existing synthetic `Stack` and `ResolvedLayers::from_folders` constructors for dev-dependencies. `test-support` also enables the existing catalogue/image fixtures. The two features keep default test gating separate from cross-package layer construction. Runtime metadata tests initialize the supported installation through `BATTLE_PLAN_GAME_DIR`; real-installation tests remain ignored and read-only. Follow [AGENTS.md](../../AGENTS.md) for formatting, clippy and tests.
