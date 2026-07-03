# GFX/Icon Asset Manifest Packet

Artifact: `gfx_icon_asset_manifest_static`

Generated payload: `extractor/game_defs/out/gfx_icon_manifest/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json`

Gate report: `extractor/game_defs/out/gfx_icon_manifest/gfx_icon_manifest_gate_report.json`

## Schema

`gfx_icon_asset_manifest_static.static_artifact.v1`

- `sprites`: keyed by `SpriteType.name` from effective `interface/*.gfx`; includes category, texture path, texture layer, texture bytes/SHA-256 when resolved, source metadata, and raw sprite block data.
- `flags`: path/hash inventory for effective `gfx/flags` image assets.
- `source_files`: interface `.gfx` and flag file inventories with bytes and SHA-256.
- `meta.syntax_allowlist_hits`: documented base-game syntax quirks accepted by the strict precheck. Current known entry is vanilla `interface/powerbalanceview.gfx`, whose installed file is missing the final top-level `spriteTypes` brace while the tolerant parser still recovers sprite blocks.

## Join Keys

- Sprite key: `sprites.<GFX_name>`.
- Texture path: `sprites.<GFX_name>.texturefile`.
- Flag file path: `flags[*].relpath`.

## Gate Status

The current strict gate builds and hash-verifies the manifest store, but does not pass because 534 unique sprite texture references do not resolve to files in the effective vanilla+mod layers: 472 vanilla-layer references and 62 `AMSB - DEV` references.

## Packing

Do not pack this artifact as complete until missing textures are fixed or explicitly allowlisted. Once closed, pack the payload as replay `role="static_artifact"` named `gfx_icon_asset_manifest_static`.
