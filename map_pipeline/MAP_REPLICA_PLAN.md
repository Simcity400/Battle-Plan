# the game Map Replica Plan

This is the canonical map-first plan for rebuilding the replay surface after the
old UI/map reset. The map app and the map-generation checkpoint are Rust-owned.

The first visible replay surface must be the map itself. No decorative panels,
mock UI, browser debug viewers, or inherited renderer assumptions should return
until the native Rust map is inspectable and being compared against the real
game.

## Source Of Truth

The vanilla install is supplied at runtime by CLI argument or environment. Do not
commit machine-specific install paths.

The the game map is a layered Clausewitz render system, not a single picture:

| Layer | Source files |
| --- | --- |
| Province identity | `map/default.map`, `map/provinces.bmp`, `map/definition.csv` |
| States | `history/states/*.txt` |
| Strategic/naval/air/weather regions | `map/strategicregions/*.txt`, `map/weatherpositions.txt` |
| Movement and special crossings | `map/adjacencies.csv`, `map/adjacency_rules.txt` |
| Terrain | `map/terrain.bmp`, `common/terrain/00_terrain.txt`, `map/heightmap.bmp`, `map/world_normal.bmp` |
| Water/rivers/coasts | `map/rivers.bmp`, `map/terrain/*.dds` |
| Trees/cities/seasons | `map/trees.bmp`, `map/cities.bmp`, `map/cities.txt`, `map/seasons.txt` |
| Static objects | `map/buildings.txt`, `map/railways.txt`, `map/supply_nodes.txt`, `map/unitstacks.txt` |
| Political colors and labels | `common/country_tags/*.txt`, `common/countries/*.txt`, localisation files |
| Map UI art | `interface/mapicons.*`, `interface/mapmodes_interface.*`, `interface/minimap.gui`, referenced `gfx/interface`, `gfx/mapitems`, `gfx/minimap`, `gfx/maparrows` assets |
| Shader reference | `gfx/FX/pdxmap.shader`, `pdxmap.fxh`, `pdxwater.shader`, `river.shader`, `border.shader`, `tree.shader`, `maparrow.shader` |

## Grounding Rules

Map rendering work must be tied to a named the game source file. The live viewer is
only an output surface; it is not evidence that a color, opacity, scale, blend,
or layer order is correct.

Every renderer step should fall into one of these buckets:

- Confirmed: implemented directly from the game data files or shader code.
- Mechanical decode: implemented from a documented file format needed to read a
  confirmed source asset, such as BMP or DDS decoding.
- Blocked gap: the source asset is known, but the engine generation step or
  shader behavior has not been reproduced yet.

Do not add or tune visual constants unless they are traced to the game files. When a
gap depends on engine-generated textures, mesh generation, camera state, or
runtime buffers that are not present as plain files, stop and document the gap
before adding an approximation.

## Current Grounding Audit

The current Rust map visual is useful as a source-file-backed scaffold, but it is
not yet a 1:1 the game render. These are the known grounding boundaries.

| Area | the game source evidence | Current Rust status | Next grounded action |
| --- | --- | --- | --- |
| Province identity | `map/default.map` names `definition.csv` and `provinces.bmp`; `map/definition.csv` maps RGB to province IDs. | Confirmed. Rust decodes the full province-id buffer from these files. | Keep as the hit-test source. |
| States and 1936 owner | `history/states/*.txt` state blocks define province membership and owner. | Confirmed for static 1936 ownership. | Later replace owner/controller from replay snapshots. |
| Terrain index to texture ID | `map/default.map` names `terrain.bmp`; `common/terrain/00_terrain.txt` says `color = index in bitmap color table (see terrain.bmp)` and maps each color index to `texture = N`. | Confirmed mapping with explicit gaps. Rust reads the terrain index and texture ID, writes unmapped indices as `255`, and records unmapped index counts in `map_object_meta.json` instead of defaulting them to a fabricated terrain. | Keep the mapping, but replace the current texture sampling with shader-equivalent sampling once `TerrainIDMap` generation is understood. |
| Texture sampler filtering | `pdxmap.shader` declares `TerrainDiffuse`, `HeightNormal`, `TerrainColorTint`, `TerrainNormal`, and other color samplers with linear filtering; `TerrainIDMap` is point-sampled. | Confirmed for current static inputs. Rust now linearly samples color DDS inputs used by the CPU scaffold and leaves ID/index data point-sampled. | Add sampler-address modes per shader as each remaining layer is ported. |
| Terrain atlas sampling | `gfx/FX/constants.fxh` sets `TERRAIN_TILE_FREQ = 128.0f`, `MAP_NUM_TILES = 4.0f`, `TEXELS_PER_TILE = 512.0f`, and `ATLAS_TEXEL_POW2_EXPONENT = 11.0f`; `gfx/FX/pdxmap.shader` multiplies `uv2` by `MAP_POW2_X/Y` before `sample_terrain(...)`. | Partly grounded. Rust now preserves the relevant shader source files, parses the current scaffold constants from `gfx/FX/constants.fxh` into `map_material_contract.json`, uses the shader tile constants, linear sampler behavior, and `sample_terrain(...)` atlas coordinate formula at base LOD. `MAP_POW2_X/Y` use is known from shaders, but the vanilla files inspected so far do not define the values or derivation, so Rust no longer applies an inferred ratio. Mip selection also remains unresolved. | Find engine/runtime evidence for `MAP_POW2_X/Y`, then port `mipmapLevel(...)` with real screen derivatives before claiming shader-equivalent terrain sampling. |
| Terrain ID blending | `pdxmap.shader` calls `calculate_map_tex_index(...)`, samples multiple terrain IDs, and blends `diffuse`, `ColorRD`, `ColorLU`, and `ColorRU` using `vFrac`. | Blocked gap. Rust now bundles `terrain_indices.r8` from `terrain.bmp` and `terrain_texture_ids.r8` from `00_terrain.txt`, but it still does not claim the engine's packed RGBA `TerrainIDMap` generation. | Determine and document how the game builds packed `TerrainIDMap`; then implement the exact four-ID blend. |
| Colormap overlay | `pdxmap.shader` samples `TerrainColorTint`; `standardfuncsgfx.fxh` defines `GetOverlay(...)`; `constants.fxh` sets `COLORMAP_OVERLAY_STRENGTH = 0.75f`. | Confirmed for the static CPU scaffold. Rust now ports `GetOverlay(...)` and the `0.75f` constant. | Keep this path unless later GPU shader execution replaces the CPU scaffold. |
| Terrain normals and lighting | `pdxmap.shader` samples `HeightNormal` and `TerrainNormal`, rotates normals, applies sun/shadow/point light/FOW/day-night/fog. `map/default.map` names `heightmap.bmp`; vanilla also ships `map/world_normal.bmp` and `map/terrain/atlas_normal*.dds`. `pdxmap.shader` uses `TerrainDiffuse.a` for glossiness and `TerrainNormal.a` for specular strength. | Partly grounded. Rust now preserves `map/world_normal.bmp`, RGBX terrain atlases for the current viewer, and alpha-preserving RGBA copies of the diffuse/normal atlas sources. The old visible CPU height-gradient shade has been removed because its constants were not shader-grounded; shader lighting is still not claimed. | Port `HeightNormal`/`TerrainNormal` sampling and the selected lighting subset before changing terrain lighting. |
| Water and underwater | `pdxmap.shader` has `PixelShaderUnderwater`; `pdxwater.shader` uses water color, reflection, LEAN textures, specular alpha, ice, gradient borders, lighting, and constants such as `WATER_HEIGHT` and `WATER_COLOR_LIGHTNESS`. `standardfuncsgfx.fxh` uses `lean2.zw` and `lean1.w` in `SampleLEAN(...)`; `pdxwater.shader` samples `SpecularMap.a`, `IceDiffuse.a`, `WaterRefraction`, gradient-border channels, light maps, and `ShadowMap`. | Production path is captured native D3D11 replay, not CPU water-shader porting. Rust validates explicit `water_pass` map-frame bundles, requires `pdxwater.shader` source evidence, requires captured IA topology/draw state/output-merger evidence, matches reflected PS SRVs, reflected constant buffers, and sampler slots, requires camera-updateable `TerrainSceneCB0`, and only promotes water in the app-owned terrain-first pass graph when every water draw targets descriptors matching the terrain color/depth graph. Runtime-generated inputs such as refraction, light maps, shadow maps, gradient borders, snow/mud, and secondary color remain blockers until captured from HOI4. | Capture a coherent real the game water bundle with all reflected `pdxwater` SRVs, constant buffers, samplers, draw state, and output-state identity, then replay it after terrain against the app-owned terrain color/depth targets. |
| Rivers | `map/default.map` names `rivers.bmp`; `river.shader` uses `WaterColor`, `Diffuse`, `NormalMap`, `RiverData`, LEAN textures, gradient borders, snow/mud, lighting, reflection, and alpha blending. `river.shader` receives generated vertex data (`vPosition`, `vUV_Tangent`) and samples `RiverData`; localisation exposes a `LOAD_GFX_PDXMAP_RIVERS` phase for creating rivers. | Blocked shader/geometry gap with preserved inputs and a material contract. Rust preserves `map/rivers.bmp`, all `RiverSurface_diffuse_*`, all `RiverSurface_normal_*`, and `RiverSurface_masks.dds`, but it still does not draw a direct `rivers.bmp` overlay because the mesh/data generation path is engine-side and not grounded in shipped text files. `RiverData` is explicitly marked generated/runtime-blocked. The capture DLL can request `rivers_pass` only when the active pixel shader declares the `river.shader` resource family; map-viewer now validates that bundle and can promote it in the production graph after water. | Capture a real `rivers_pass` bundle from the game, validate its generated mesh/data and resources, then implement D3D11 replay of the pass after water. |
| Borders | `pdxmap.shader`, `pdxwater.shader`, and `river.shader` call `gradient_border_apply(...)`; `standardfuncsgfx.fxh` defines gradient-border alpha and camera behavior; `border.shader` samples `BorderDiffuse` with `BORDER_TILE = 0.4f`; `common/defines/00_graphics.lua` defines border width, gradient-border thicknesses, outline cutoffs, camera overrides, optimization range, and refresh frequency. | Blocked generated-buffer gap with preserved material inputs and a material contract. Rust preserves all primary country/state/sea/province/impassable/sea-region border DDS variants as alpha-capable RGBA sources, but it does not raster-stamp province contacts. Gradient border channels are explicitly marked generated/runtime-blocked. The capture DLL can request `borders_pass` only when the active pixel shader declares the `border.shader` resource family; map-viewer now validates that bundle and can promote it in the production graph after rivers. | Capture a real `borders_pass` bundle from the game and validate whether it carries standalone border mesh data, generated gradient channels, or both before implementing pass replay. |
| Trees, cities, seasons | `map/default.map` names `trees.bmp` and `seasons.txt`; `map/cities.bmp`, `map/cities.txt`, `map/terrain/Tree_*.bmp`, `tree.shader`, city-light textures, snow/mud/ice textures are present. `map/cities.txt` points to `map/cities.bmp` and maps `color_index` groups to entity meshes. | Source-cataloged, not rendered. Rust now copies the exact city/tree/season source files, decodes `cities.r8`, `trees.r8`, `tree_tint.rgbx`, and `tree_season.rgbx`, catalogs city groups, default-map tree indices, season dates, entity/shader source files, and common building rules in `map_object_meta.json`. It does not scatter trees/cities or apply seasons because the engine generation path is not yet reproduced. The capture DLL can request `trees_pass` only when the active pixel shader declares the `tree.shader` resource family; map-viewer now validates that bundle and can promote it in the production graph after borders. `cities_pass` remains rejected until an equivalent grounded shader/resource signature exists. | Capture a real `trees_pass` bundle from the game, implement D3D11 replay of that pass after borders, then identify a grounded city draw signature before accepting `cities_pass`. |
| Static objects and map positions | Vanilla ships `map/buildings.txt`, `map/railways.txt`, `map/supply_nodes.txt`, `map/unitstacks.txt`, `map/ambient_object.txt`, `map/weatherpositions.txt`, and `map/positions.txt`. Entity/rule sources include `gfx/entities/*.gfx`, `gfx/entities/*.asset`, `common/buildings/*.txt`, supply/rail model GFX, and related shaders. | Source-cataloged, not rendered. Rust now copies these exact files into the generated bundle, hashes them in the manifest, and emits row counts, category counts, ambient object counts, building-rule metadata, and text source catalogs in `map_object_meta.json`. | Parse validated placement/topology/entity references into renderer-ready data before drawing buildings, railways, supply nodes, stack positions, ambient objects, or weather markers. |

## Stop Points Before More Visual Work

- Terrain: do not tune current atlas output by eye; `GetOverlay(...)` is ported,
  but `MAP_POW2_X/Y`, mip LOD, and terrain-ID blending are still unresolved.
  Unmapped `terrain.bmp` indices must remain explicit gaps, not silent terrain
  defaults.
- Water: do not adjust water by eye; first capture and bind every
  shader-reflected `pdxwater` sampler/resource, reflected constant buffer,
  camera constant, draw state, topology, and output target/depth descriptor from
  the active the game water pass.
- Rivers: do not treat `rivers.bmp` as a final screen overlay until the river
  mesh/data generation path is understood.
- Borders: do not tune country/state/coast opacity by eye; first reproduce
  gradient-border channels or border mesh rendering.
- Lighting: do not keep the current heightmap gradient as final; first identify
  `HeightNormal` and the relevant shader lighting subset.
- Runtime buffers: do not fabricate `TerrainIDMap`, `RiverData`,
  `GradientBorderChannel1/2/3`, `SnowMudData`, `WaterRefraction`, light maps,
  shadow maps, or `ProvinceSecondaryColorMap`; keep them as blocked until their
  generation paths are found in engine/runtime data.

## Rust Ownership

The native map work lives in `replay_native/map_viewer`.

Current responsibilities:

- Build and inspect generated map bundles from the selected the game install when
  needed for source-file inventory and validation.
- Own the production terrain renderer backend API.
- Validate coherent `terrain_active_pass` map-frame bundles captured from
  the game's D3D11 runtime.
- Replay captured the game terrain VS/PS bytecode and D3D11 resources/state through
  the native D3D11 terrain pass.
- Own the terrain-first production pass graph. Current sequence is
  `terrain_active_pass` -> `water_pass` -> `rivers_pass` -> `borders_pass` ->
  `trees_pass` -> `cities_pass` -> `overlays_pass`; every pass after terrain is
  blocked until a named the game runtime bundle or proven generated data exists.
- Keep the older CPU terrain renderer as a validation/reference path only; it is
  not the target visible renderer.

The old Python map pipeline scripts and browser debug viewer are removed from
the tracked source. The remaining `map_pipeline/assets/*.json` files are legacy
compatibility data for other recorder/research modules until those callers are
migrated.

## Phase 0: Rust Map Bundle

Generate a gitignored bundle from the selected the game install.

Minimum bundle contents:

- `manifest.json`
- `province_ids.r32`: full-resolution little-endian `u32` province IDs
- `map_visual.rgbx`: full-resolution `0x00RRGGBB` terrain/water base layer
- `terrain_indices.r8`: raw `map/terrain.bmp` index per map pixel
- `terrain_texture_ids.r8`: resolved per-pixel terrain atlas texture ID from
  `common/terrain/00_terrain.txt`; unmapped terrain indices are encoded as
  `255` and recorded in `map_object_meta.json`
- `heightmap.r8`: raw `map/heightmap.bmp`
- `rivers.r8`: raw `map/rivers.bmp`
- `height_normal.rgbx`: `map/world_normal.bmp` converted to `0x00RRGGBB` for
  the shader normal path
- `cities.r8`, `trees.r8`: raw indexed map pixels from `map/cities.bmp` and
  `map/trees.bmp` for future city/tree generation
- `tree_tint.rgbx`, `tree_season.rgbx`: decoded
  `map/terrain/Tree_tint.bmp` and `map/terrain/Tree_season.bmp`
- `terrain_diffuse_atlas0.rgbx`, `terrain_normal_atlas0.rgbx`: the primary
  `TerrainDiffuse` and `TerrainNormal` atlas inputs used by `pdxmap.shader`
- `files.shader_sources` in `manifest.json`: vanilla shader material inputs
  converted to raw RGBA byte order where 2D decoding is exact, plus the raw
  `reflection.dds` cubemap source. This includes terrain atlas alpha channels,
  water LEAN/spec/ice sources, all shipped water/underwater variants, river
  surface variants, city/snow/mud sources, and all primary border material
  variants.
- `map_material_contract.json`: source-derived material contract listing shipped
  DDS source roles, output files, dimensions, DDS format, mip count, known shader
  sampler names, parsed constants from `gfx/FX/constants.fxh`, and
  generated/runtime blockers such as `RiverData`, `GradientBorderChannel1/2/3`,
  `WaterRefraction`, `ProvinceSecondaryColorMap`, `SnowMudData`,
  `LightIndexMap`, `LightDataMap`, `ShadowMap`, tree masks, and FOW/intel
  textures.
- `files.map_sources` in `manifest.json`: exact vanilla source-file copies for
  `map/default.map`, city/tree/season files, static placement files, railways,
  supply nodes, unit stacks, ambient objects, weather positions, and map
  positions, building/entity definitions, supply/rail model GFX, tree/mesh/rail
  shaders, core map/water/river/border shader sources and includes, and graphics
  defines. These copies are evidence and future inputs, not rendering claims.
- `map_object_meta.json`: compact source-derived catalogs for default-map file
  references, terrain mapping gaps, city groups, indexed map histograms, season
  dates, static placement row/category counts, ambient object counts, common
  building rules, and render-source text catalogs.
- `province_to_state.json`
- `state_meta.json`
- `state_owner_1936.json`
- `country_colors.json`
- `province_definitions.json`
- `province_to_strategic_region.json`
- `strategic_region_to_provinces.json`
- `strategic_region_meta.json`

Acceptance gate:

- Bundle builds from Rust CLI args only.
- No local absolute paths are committed.
- Dimensions are `5632x2048`.
- Province-id decoding agrees with source `provinces.bmp` and `definition.csv`.

## Phase 1: Rust Live Map

Open a native Rust map window from the generated bundle.

Required interactions:

- Pan.
- Zoom.
- Reset camera.
- Exact hover/click province picking from the Rust-generated province-id buffer.
- Probe title/log output with province ID, state ID, owner tag, terrain,
  province type, and strategic-region ID.

Acceptance gate:

- The visible inspection surface is Rust, not HTML.
- Picking uses the raw Rust bundle, not rendered colors.
- The visible map is built from the game visual sources, not flat owner colors.

## Phase 2: Native Renderer Skeleton

Replace the current CPU visual composition with the real native renderer path.

The production terrain renderer entry point is `map_viewer render-terrain`. It
does not consume `map_visual.rgbx` and it must fail closed until the real packed
`TerrainIDMap` is present as `terrain_id_map.rgba` or an equivalent runtime
extraction is implemented. Do not replace this with `terrain.bmp` expansion or a
single-ID approximation.

Runtime terrain intake is `map_viewer import-terrain-runtime`. It accepts only a
runtime-extracted raw RGBA TerrainIDMap plus explicit width, height, provenance,
and either explicit `MAP_POW2_X`/`MAP_POW2_Y` values or a generated shader cache
directory that contains a `TerrainIDMap_Texture register(tN)` pdxmap cache
signature. It records the declared slot in provenance instead of assuming `t5`.
It then writes `terrain_runtime.json` with byte length, FNV hash,
shader-grounded channel order, MAP_POW2 provenance, and the point/no-mip clamp
sampler contract. `render-terrain` validates the raw texture and metadata
together, then renders `terrain_render.rgbx` from the packed TerrainIDMap
channels, `sample_terrain` atlas coordinates, the shader's four-corner blend
order, and the colormap overlay. Lighting, snow/mud, water, borders, rivers,
and object layers remain blocked until their shader/runtime inputs are
available.

Runtime extraction is owned by `replay_native/terrain_capture`. It hooks the
D3D11 terrain shader/resource binding path, validates the bound RGBA8
`Texture2D`, and writes only real GPU TerrainIDMap bytes. Late attach after the
terrain shaders were already created is a blocker unless a separate shader
identity path is proven; guessed slots and `terrain.bmp` expansion are not valid
fallbacks.

Required architecture:

- Camera coordinates live in map pixels.
- GPU shaders own recoloring and map-mode drawing.
- CPU mirror of `province_ids.r32` owns hit testing.
- Replay snapshots update compact buffers such as state owner/controller,
  province controller, regions, units, combats, and fronts.

Acceptance gate:

- Snapshot scrubbing updates political/control colors without CPU rerasterizing
  the whole map.
- Map picking stays bit-identical with Phase 1.

## Phase 3: Terrain Replica

Recreate the Clausewitz terrain look from real installed content.

Required layers:

- Terrain indices.
- Heightmap.
- World normal map.
- Water material.
- River mask/material.
- Tree/city scatter.
- Seasonal snow/mud/ice.
- Terrain DDS atlases and color-correction references.

Acceptance gate:

- Fixed-camera screenshots are compared against real HOI4.
- Differences are tracked by named layer.

## Phase 4: Borders, Map Modes, Labels

Add borders and game map modes.

Required layers:

- Province, state, country, sea-region, strategic-region, coast, impassable,
  strait, and canal borders.
- Political, control, state, terrain, supply, resources, infrastructure,
  strategic region, air/naval, weather, resistance/compliance, combat/front
  modes.
- Country/state/VP/city/region labels with zoom LOD and collision control.

Acceptance gate:

- Missing replay-backed data is visibly badged, never fabricated.
- Borders honor `adjacencies.csv` and `adjacency_rules.txt`, not only raster
  contact.

## Phase 5: Units, Fronts, Combat, Orders

Add the dynamic gameplay overlays.

Required layers:

- HOI4-style unit counters and stacks.
- Movement arrows.
- Battle plans/front lines.
- Combat bubbles.
- Air/naval mission and weather overlays.
- Selection/hover/click priority: unit, combat, label, province.

Acceptance gate:

- Click-through data agrees with replay snapshots.
- Unit and front rendering can be checked against a captured reference moment.

## Phase 6: Visual QA Harness

Create screenshot comparison against real HOI4.

Reference scenes:

- Whole world.
- Europe.
- UK and North Sea.
- Mediterranean.
- China/Japan.
- Pacific island chains.
- Americas.
- Province zoom with borders and labels.
- Terrain mode.
- Supply mode.
- Strategic air/naval modes.

Acceptance gate:

- Every layer has a real-game screenshot comparison.
- Manual review happens before expanding scope.
