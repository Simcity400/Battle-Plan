# the game Map Replica Plan

This is the canonical map-parity plan. The map app and the map-generation
checkpoint are Rust-owned.

Sequencing note (2026-07-02): the old rule that no UI or dynamic replay work
may start until the map is 1:1 is retired. `REPLAY_ARCHITECTURE.md` now owns
sequencing: the replay spine and the data-driven map surface (`replay_app`)
proceed in parallel with the terrain-parity program in this document. The
grounding rules, stop points, and per-layer evidence standards below remain
binding for every visual parity claim. Mock UI, browser debug viewers, and
invented map styling remain forbidden.

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

The former Rust `map_visual.rgbx` source-file-backed scaffold has been removed
from fresh bundle output and from `replay_app` loading/rendering. Product map
rendering requires a validated the game D3D11 capture or a fully reproduced engine
terrain pipeline. These are the known grounding boundaries.

Capture evidence 2026-07-02 (session at
`map_viewer/out/captures/session_20260702`, gitignored): an early-injection
run with `multi_sampling=0` captured a full 21-draw terrain pass with
single-sample render/depth targets (the old MSAA blocker is gone), all 16 PS
SRVs, VS/PS constant buffers, IA buffers, and shader bytecode/reflection.
`TerrainIDMap` re-captured hash-identical to 2026-06-30
(`f493368c56224ab1`) — the engine-packed map is stable per save/build.
Measured: `GradientBorderChannel1/2` are 2816x2050 `B8G8R8A8_UNORM_SRGB`
(half map resolution, two stacked ~1024-row halves) but were ALL-ZERO
placeholders bound to both slots — the engine generates them only after map
load completes, as is `ProvinceSecondaryColorMap`; `SnowMudData` was already
real at load. The bundle is formally blocked (pass never closed) because the
capture hook's ~3.7 GB of synchronous per-draw readbacks froze the game
during load; the DLL is being reworked (payload dedupe, finalize-time
snapshotting of generated textures, off-thread writes) before the next
capture window.

| Area | the game source evidence | Current Rust status | Next grounded action |
| --- | --- | --- | --- |
| Province identity | `map/default.map` names `definition.csv` and `provinces.bmp`; `map/definition.csv` maps RGB to province IDs. | Confirmed. Rust decodes the full province-id buffer from these files. | Keep as the hit-test source. |
| States and 1936 owner | `history/states/*.txt` state blocks define province membership and owner. | Confirmed for static 1936 ownership. | Later replace owner/controller from replay snapshots. |
| Terrain index to texture ID | `map/default.map` names `terrain.bmp`; `common/terrain/00_terrain.txt` says `color = index in bitmap color table (see terrain.bmp)` and maps each color index to `texture = N`. | Confirmed mapping with explicit gaps. Rust reads the terrain index and texture ID, writes unmapped indices as `255`, and records unmapped index counts in `map_object_meta.json` instead of defaulting them to a fabricated terrain. The native replay path now feeds this resolver-derived data into the documented `TerrainIDMap` synthesizer when `--game` / `BATTLE_PLAN_VANILLA` evidence is available. | Keep the mapping and its capture byte gate; remaining terrain work is runtime lighting/FOW/shadow/snow-mud state, not terrain material sampling or guessed IDs. |
| Texture sampler filtering | `pdxmap.shader` declares `TerrainDiffuse`, `HeightNormal`, `TerrainColorTint`, `TerrainNormal`, and other color samplers with linear filtering; `TerrainIDMap` is point-sampled. | Confirmed for current static inputs. Rust now linearly samples color DDS inputs used by the CPU scaffold and leaves ID/index data point-sampled. | Add sampler-address modes per shader as each remaining layer is ported. |
| Terrain ID blending | `pdxmap.shader` calls `calculate_map_tex_index(...)`, samples four terrain IDs, and performs the nested `vFrac` lerps; `standardfuncsgfx.fxh` defines the all-same `+100` sentinel and 4x4 atlas decode. | **Closed for the terrain material path.** Resolver-backed `TerrainIDMap` synthesis validates against captured RGBA bytes. The replay point/clamp samples the half-texel-shifted ID map, decodes the four channels and sentinel, samples diffuse and normal atlases at the selected LOD, and reproduces the exact RU/LU/RD/base nested lerps. HLSL `frac = x-floor(x)` is used for wrapped negative map copies instead of Rust's sign-preserving `fract`. | Broaden byte-oracle coverage to additional modded maps; remaining blockers are runtime lighting/FOW/shadow/snow-mud inputs, not terrain-ID material blending. |
| Colormap overlay | `pdxmap.shader` samples `TerrainColorTint`; `standardfuncsgfx.fxh` defines `GetOverlay(...)`; `constants.fxh` sets `COLORMAP_OVERLAY_STRENGTH = 0.75f`. | Confirmed for the static CPU scaffold. Rust now ports `GetOverlay(...)` and the `0.75f` constant. | Keep this path unless later GPU shader execution replaces the CPU scaffold. |
| Terrain normals and lighting | `pdxmap.shader` samples `HeightNormal` and `TerrainNormal`, rotates normals, applies sun/shadow/point light/FOW/day-night/fog. `map/default.map` names `heightmap.bmp`; vanilla also ships `map/world_normal.bmp` and `map/terrain/atlas_normal*.dds`. `pdxmap.shader` uses `TerrainDiffuse.a` for glossiness and `TerrainNormal.a` for specular strength. | Partly grounded. Rust now preserves `map/world_normal.bmp`, RGBX terrain atlases for the current viewer, and alpha-preserving RGBA copies of the diffuse/normal atlas sources. The old visible CPU height-gradient shade has been removed because its constants were not shader-grounded; shader lighting is still not claimed. | Port `HeightNormal`/`TerrainNormal` sampling and the selected lighting subset before changing terrain lighting. |
| Water and underwater | `pdxmap.shader` has `PixelShaderUnderwater`; `pdxwater.shader` uses water color, reflection, LEAN textures, specular alpha, ice, gradient borders, lighting, and constants such as `WATER_HEIGHT` and `WATER_COLOR_LIGHTNESS`. `standardfuncsgfx.fxh` uses `lean2.zw` and `lean1.w` in `SampleLEAN(...)`; `pdxwater.shader` samples `SpecularMap.a`, `IceDiffuse.a`, `WaterRefraction`, gradient-border channels, light maps, and `ShadowMap`. | Production path is captured native D3D11 replay, not CPU water-shader porting. Rust validates explicit `water_pass` map-frame bundles, requires `pdxwater.shader` source evidence, requires captured IA topology/draw state/output-merger evidence, matches reflected PS SRVs, reflected constant buffers, and sampler slots, requires camera-updateable `TerrainSceneCB0`, and only promotes water in the app-owned terrain-first pass graph when every water draw targets descriptors matching the terrain color/depth graph. Runtime-generated inputs such as refraction, light maps, shadow maps, gradient borders, snow/mud, and secondary color remain blockers until captured from HOI4. | Capture a coherent real the game water bundle with all reflected `pdxwater` SRVs, constant buffers, samplers, draw state, and output-state identity, then replay it after terrain against the app-owned terrain color/depth targets. |
| Rivers | `map/default.map` names `rivers.bmp`; `river.shader` uses `WaterColor`, `Diffuse`, `NormalMap`, `RiverData`, LEAN textures, gradient borders, snow/mud, lighting, reflection, and alpha blending. `river.shader` receives generated vertex data (`vPosition`, `vUV_Tangent`) and samples `RiverData`; localisation exposes a `LOAD_GFX_PDXMAP_RIVERS` phase for creating rivers. | Blocked shader/geometry gap with preserved inputs and a material contract. Rust preserves `map/rivers.bmp`, all `RiverSurface_diffuse_*`, all `RiverSurface_normal_*`, and `RiverSurface_masks.dds`, but it still does not draw a direct `rivers.bmp` overlay because the mesh/data generation path is engine-side and not grounded in shipped text files. `RiverData` is explicitly marked generated/runtime-blocked. The capture DLL can request `rivers_pass` only when the active pixel shader declares the `river.shader` resource family; map-viewer now validates that bundle and can promote it in the production graph after water. | Capture a real `rivers_pass` bundle from the game, validate its generated mesh/data and resources, then implement D3D11 replay of the pass after water. |
| Borders | `pdxmap.shader`, `pdxwater.shader`, and `river.shader` call `gradient_border_apply(...)`; `standardfuncsgfx.fxh` defines gradient-border alpha and camera behavior; `border.shader` samples `BorderDiffuse` with `BORDER_TILE = 0.4f`; `common/defines/00_graphics.lua` defines border width, gradient-border thicknesses, outline cutoffs, camera overrides, optimization range, and refresh frequency. | Blocked generated-buffer gap with preserved material inputs and a material contract. Rust preserves all primary country/state/sea/province/impassable/sea-region border DDS variants as alpha-capable RGBA sources, but it does not raster-stamp province contacts. Gradient border channels are explicitly marked generated/runtime-blocked. The capture DLL can request `borders_pass` only when the active pixel shader declares the `border.shader` resource family; map-viewer now validates that bundle and can promote it in the production graph after rivers. | Capture a real `borders_pass` bundle from the game and validate whether it carries standalone border mesh data, generated gradient channels, or both before implementing pass replay. |
| Trees, cities, seasons | `map/default.map` names `trees.bmp` and `seasons.txt`; `map/cities.bmp`, `map/cities.txt`, `map/terrain/Tree_*.bmp`, `tree.shader`, city-light textures, snow/mud/ice textures are present. `map/cities.txt` points to `map/cities.bmp` and maps `color_index` groups to entity meshes. | Source-cataloged, not rendered. Rust now copies the exact city/tree/season source files, decodes `cities.r8`, `trees.r8`, `tree_tint.rgbx`, and `tree_season.rgbx`, catalogs city groups, default-map tree indices, season dates, entity/shader source files, and common building rules in `map_object_meta.json`. It does not scatter trees/cities or apply seasons because the engine generation path is not yet reproduced. The capture DLL can request `trees_pass` only when the active pixel shader declares the `tree.shader` resource family; map-viewer now validates that bundle and can promote it in the production graph after borders. `cities_pass` remains rejected until an equivalent grounded shader/resource signature exists. | Capture a real `trees_pass` bundle from the game, implement D3D11 replay of that pass after borders, then identify a grounded city draw signature before accepting `cities_pass`. |
| Static objects and map positions | Vanilla ships `map/buildings.txt`, `map/railways.txt`, `map/supply_nodes.txt`, `map/unitstacks.txt`, `map/ambient_object.txt`, `map/weatherpositions.txt`, and `map/positions.txt`. Entity/rule sources include `gfx/entities/*.gfx`, `gfx/entities/*.asset`, `common/buildings/*.txt`, supply/rail model GFX, and related shaders. | Source-cataloged, not rendered. Rust now copies these exact files into the generated bundle, hashes them in the manifest, and emits row counts, category counts, ambient object counts, building-rule metadata, and text source catalogs in `map_object_meta.json`. | Parse validated placement/topology/entity references into renderer-ready data before drawing buildings, railways, supply nodes, stack positions, ambient objects, or weather markers. |

## Map Completion Checklist (to 100%, before UI work)

Definition of done: every row below is either (a) rendered through the
captured-pass replay or an exact shader-source port and signed off by the
visual QA harness, or (b) explicitly badged with a decision recorded here.
Statuses: DONE / IN FLIGHT / READY (unblocked, not started) / CAPTURE-GATED
(needs the next clean capture) / RESEARCH (generation semantics unknown) /
DECISION (user scoping call).

Track 1 — political look (no game session needed):
- [BLOCKED 2026-07-08] Primary political fill: no primary fill SRV/render-target
  or engine generation path has been identified. The replay app must not invent
  a solid country-color layer from owner/controller data until this is captured
  or reverse-engineered from `hoi4.exe`.
- [RESEARCH 2026-07-08] Gradient borders: `gradient_border_apply` shader
  application is ported, and `hoi4.exe` contains the right trail
  (`CGradientBorderManager`, `CGradientBorder::CFillBufferThreaded`,
  `CClearBufferThreaded`), but the exact runtime channel-generation law is not
  ported. The Rust CPU generator contains captured-fit distance pieces and is
  research-only; product rendering keeps generated replay GB channels inert and
  does not substitute them into captured the game passes.
- [BLOCKED 2026-07-08] Occupation striping: `secondary_color_mask` shader
  semantics are understood and captured `ProvinceSecondaryColorMap` is
  all-transparent in observed default political-map captures. The replay app
  keeps the texture empty unless a real mode that populates it is captured and
  its CPU write law is identified.

Track 2 — terrain look:
- [REMOVED 2026-07-08] CPU `map_visual.rgbx` underlay: removed from bundle
  generation and from `replay_app` loading/rendering. It mixed source-file
  decoding with unreproduced engine behavior (`TerrainIDMap` packing/blend,
  mip/derivative behavior, runtime buffers, lighting, and post-processing), and
  produced the checkerboard/noisy terrain symptom. Do not restore it except as a
  clearly separate research artifact.
- [READY: re-dispatch] CPU lighting subset (HeightNormal/TerrainNormal N.L sun +
  day ambient; verified port was destroyed by a concurrent job revert before
  commit — re-run from its report).
- [NEW, READY] Post-processing tone map: the game composes the map into an fp16
  HDR target (~0.23 land luminance measured in the 2026-07-02 capture) and
  brightens via its postfx shader; port that term or all brightness parity
  claims are off. Source: the postfx shader family + captured HDR values.
- [CAPTURE-GATED] Captured-pass terrain replay as the app's base layer
  (supersedes the CPU underlay wherever a valid bundle exists).

Track 3 — capture-gated layers:
- [DONE 2026-07-02] Capture DLL rework + correctness fixes (dedupe,
  settle/dwell-end snapshots, CB naming contract + repair tool, variant-aware
  pass close, boundary-time references). Sessions under
  `map_viewer/out/captures/`.
- [DONE 2026-07-02] Terrain bundle VALIDATED: session_20260702d replays the
  captured pass BYTE-IDENTICAL to the game's own render target (7 draws,
  camera-updatable TerrainSceneCB0). Water bundle VALIDATED same session.
- [DONE 2026-07-02] Live GradientBorderChannel1/2 ground truth captured
  (dwell-end snapshots); ProvinceSecondaryColorMap confirmed all-zero on a
  live occupied political map (user-reported no-stripes rule proven).
- [CAPTURE-GATED] rivers_pass, borders_pass, trees_pass: need a session with
  deep-zoom camera work; the 2026-07-02 session crashed ~3 min in before
  rivers drew (crash cause undiagnosed — investigate DLL involvement before
  the next window).
- [RESEARCH] cities_pass: no grounded shader/resource signature yet.
- [PARTIAL 2026-07-08] Convergence integration: replay_app renders the validated
  captured terrain+water passes as its map base (grounded camera bridge from
  decomposed captured VP + defines; water via engine-equivalent fp16->BGRA8
  refraction copy). Replay-driven political SRV substitution is disabled until
  the primary political fill and exact `CGradientBorderManager` channel
  generation are ported; captured passes keep the HOI4-captured political SRVs.
- [CAPTURE-GATED, named 2026-07-02] Terrain CHUNK LIBRARY: the game
  frustum-culls terrain chunks CPU-side, so a captured session only contains
  the chunks visible to the game's camera at capture time (whole-map fit
  renders 0% black; deep Europe zoom 98.5% black). Free-camera coverage
  requires accumulating unique chunk draws across a camera SWEEP during
  capture, then culling/drawing from that library per app frame. This is the
  main remaining captured-terrain work item.

Track 4 — time-varying visuals from replay data:
- [READY] Day/night cycle: `DayNightWithBlend` + sun/moon selection ported,
  driven by the replay clock; ship as a toggle like the game's.
- [RESEARCH] Weather visuals: SnowMudData generation from recorded
  `weather_live` drivers + seasons; interim = captured static state, badged.
- [READY after day/night] City lights at night (tail term + light maps).
- [DECISION: recorded] FOW intentionally omitted — the replay is all-seeing.
  This is an intentional read-only divergence, not a gap.

Track 5 — map furniture:
- [READY] Map labels: curved country names, state names, VP/city labels with
  zoom LOD (fonts done; curved-layout algorithm is engine-side — approximation
  must be badged until compared side-by-side).
- [DECISION] 3D map objects (fort/naval-base/rail/supply models, unit-stack
  meshes, ambient objects): a full entity-mesh render system. Visible only at
  close zoom. Proposed: defer past UI phase; placement data is already
  bundled, nothing is lost by deferring.

Track 6 — identity and sign-off:
- [READY] Mod-over-vanilla map bundle: generate the bundle from the recording's
  mod-set (the group plays a mod that edits states/colors); currently
  vanilla-only.
- [READY] Visual QA harness (Phase 6): fixed-camera side-by-side scenes vs real
  the game, per-layer diff ledger. The 100% claim is made HERE, not per-lane.

## Stop Points Before More Visual Work

- Terrain: do not tune atlas output by eye. `MAP_POW2_X/Y`, mip LOD, atlas
  addressing, and four-ID blending are engine/shader derived and closed for
  DX11. Keep captured shader-cache constants authoritative per modset, keep
  unmapped `terrain.bmp` indices explicit, and focus remaining work on runtime
  lighting/FOW/shadow/snow-mud inputs.
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
- Runtime buffers: `TerrainIDMap` may be supplied only by the documented
  resolver-backed synthesis law, with captured-byte validation when available.
  Do not fabricate `RiverData`, `GradientBorderChannel1/2/3`, `SnowMudData`,
  `WaterRefraction`, light maps, shadow maps, or `ProvinceSecondaryColorMap`;
  keep them as blocked until their generation paths are found in engine/runtime
  data.

## Rust Ownership

The native map work lives in `replay_interface/native/map_viewer`.

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
the tracked source. The remaining `replay_interface/map_pipeline/assets/*.json` files are legacy
compatibility data for other recorder/research modules until those callers are
migrated.

## Phase 0: Rust Map Bundle

Generate a gitignored bundle from the selected the game install.

Minimum bundle contents:

- `manifest.json`
- `province_ids.r32`: full-resolution little-endian `u32` province IDs
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

Runtime extraction is owned by `replay_interface/native/terrain_capture`. It hooks the
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
