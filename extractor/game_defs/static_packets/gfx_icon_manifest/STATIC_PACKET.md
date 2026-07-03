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

## Layer Model

This packet uses the real the game asset layer model, not the definition-parser
layer model. Ordered asset layers are:

1. Vanilla install root.
2. `integrated_dlc/*` subdirectories, always active, sorted deterministically.
3. Installed `dlc/*` subdirectories that are not listed in the user-data
   `dlc_load.json` `disabled_dlcs`, sorted deterministically.
4. Active mods, preserving the existing mod-over-lower-layer behavior.

Each asset layer contributes both direct `interface/*.gfx` sprite sources and
`gfx/**` texture/flag assets. File-level override is by case-folded relative
path; later layers win. Definition-side static parsers do not use this layer
extension because the DLC asset directories do not contribute `common/` or
localisation content.

If `dlc_load.json` is absent under the selected user-data directory, all
installed loose DLC asset directories are treated as enabled and the catalog
records that fallback in `meta.asset_layer_model`.

## Texture Resolution

Sprite `texturefile` refs are normalized for slash style and doubled separators.
The gate first resolves the exact effective asset path. If that path is absent,
it tries sibling `.dds` and `.tga` variants in the same layer order. Extension
fallback hits store the actual resolved path in `resolved_texturefile`, the real
bytes/SHA-256, and `resolved_via = "extension_fallback"`.

The committed missing-texture allowlist enumerates every remaining absent
texture ref explicitly. Each entry records the sprite, texture path, source
layer/file, and the adjudication reason:
`absent in vanilla root, integrated_dlc/*, enabled dlc/*, and as extension variant; engine renders placeholder at runtime`.

## Join Keys

- Sprite key: `sprites.<GFX_name>`.
- Texture path: `sprites.<GFX_name>.texturefile`.
- Flag file path: `flags[*].relpath`.

## Gate Status

The strict gate passes against the real asset layer model. Allowed missing
textures must exactly match the explicit allowlist; stale allowlist entries or
new unresolved texture refs fail the gate.

## Packing

Gate PASSES; pack the payload as replay `role="static_artifact"` named
`gfx_icon_asset_manifest_static`.
