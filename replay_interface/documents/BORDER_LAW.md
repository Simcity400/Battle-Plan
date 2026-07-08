# the game Thin Border Geometry Law

Status: **CAPTURED-ENGINE-DERIVED, DEFAULT ON** (BORD5, 2026-07-07).

## BORD5 — the geometry law from the captured engine pool

`session_BORDERZOOM_3` recorded a genuine `border.shader` draw (state border,
Brittany 9495 | Loire 527, both FRA; `BorderDiffuse` SRV byte-identical to
shipped `map/terrain/border_state_0.dds`, all 7 mips, `BC3_UNORM_SRGB`) plus
the engine's ENTIRE resident border vertex pool: 31,824,096 bytes, stride 16
(`float3` position + `R16G16_SINT` uv), decomposing into **40,959 ribbon
strips covering 100.0% of the pool**. The construction below reproduces
31,528 strips EXACTLY (max err 3.3e-4), including 27,014/27,048 land/land and
3,901 land/water chains — the authoritative geometry law, implemented in
`border_world_synthesis.rs` and gate-tested by
`gate_vanilla_captured_border_strip_evidence` (all 32 captured vertices,
heights included):

1. Chain = maximal run of province-pair boundary lattice edges, split at any
   corner whose pair-boundary degree != 2.
2. Stations at Manhattan arc `{0, 0.5, 1.5, …, M−0.5, M}` (rows = M + 2).
3. One box-3 smoothing pass, endpoints fixed.
4. Extrusion: per-row central-difference direction, verts = P ± 0.75·perp,
   `uv = (side 0|1, row k)` integers; heights = bilinear heightmap drape
   (grid vertex `(i,j)` = texel `(i,(H−1)−j)`, `raw*0.1`).
5. Coordinates: pixel `(x,y)` center = world `(x, (H−1)−y)`; corner `(i,j)` =
   world `(i+0.5, (H−1)−j−0.5)`.

Known non-covered cases (pinpointed, never guessed around): sea|sea
(`sea_region`) strips follow a DIFFERENT, underived construction and are
skipped by the render pass; closed loops (~0.13% of land chains) use an
unverified anchor rule; ~3.4% of pool strips (dominated by water classes)
remain undiagnosed in the harness. See FIDELITY_LEDGER 2.5.

Everything below this line is the pre-BORD5 file-first derivation record;
where it conflicts with the captured law above, the captured law wins (the
diag23b "Open Unknowns" are RESOLVED by session_BORDERZOOM_3).

## Shipped Inputs

- `map/default.map` binds `definitions = "definition.csv"`,
  `provinces = "provinces.bmp"`, and `heightmap = "heightmap.bmp"`; border
  synthesis therefore treats those three map files as the required static
  geometry inputs. Citations: `map/default.map:1`, `map/default.map:2`,
  `map/default.map:6`.
- `definition.csv` is the province color/type table. The first row shows the
  id/R/G/B/type/coastal/terrain/continent shape, and shipped rows include
  `land`, `sea`, and `lake` province types. Citations:
  `map/definition.csv:1`, `map/definition.csv:5`,
  `map/definition.csv:2`.
- `definition.csv` type census (vanilla 1.16): exactly **10155 `land`**,
  **3133 `sea`**, and **126 `lake`** rows, and no other province `type` value is
  shipped. `province_is_water` therefore treats `sea`/`lake` as water, `land` as
  land, and fails closed on any other type. Any unknown type at
  `border_world_synthesis.rs` `province_is_water` blocks synthesis rather than
  guessing. Citation for the type column shape: `map/definition.csv:1`.
- `map/adjacencies.csv` is a border-classification input. Header (line 1):
  `From;To;Type;Through;start_x;start_y;stop_x;stop_y;adjacency_rule_name;Comment`.
  The `Type` column (field 3) is consumed by border synthesis; vanilla ships
  **93 `impassable` rows** that name specific province pairs as impassable
  (e.g. the Himalaya wastelands), **151 `sea` rows**, and 10 blank-`Type` rows
  (7 named strait/adjacency-rule crossings such as `GIBRALTAR_STRAIT`, the
  `-1;-1;;...` terminator sentinel, and empty trailing lines). Example rows:
  `map/adjacencies.csv:1` (header), `map/adjacencies.csv:2`
  (`12251;10321;sea;8313;...` — a sea crossing), `map/adjacencies.csv:146`
  (`10825;1991;impassable;-1;-1;-1;-1;-1;;Himalayas`),
  `map/adjacencies.csv:147` (`10825;8029;impassable;...`).
- 1936 state membership and owner come from `history/states/*.txt`; the
  example state file has `id=1`, `owner = FRA`, and a `provinces={...}` block.
  Citations: `history/states/1-France.txt:3`,
  `history/states/1-France.txt:10`,
  `history/states/1-France.txt:24`-`history/states/1-France.txt:26`.
- The shader-side vertex input is exactly `float3 position : POSITION` plus
  `int2 uv : TEXCOORD0`; the vertex output carries clip position, world
  position, uv, and screen coordinates. Citations:
  `gfx/FX/border.shader:57`-`gfx/FX/border.shader:60`,
  `gfx/FX/border.shader:63`-`gfx/FX/border.shader:69`.
- The border shader declares `BorderDiffuse` at sampler index 0,
  `SnowMudData` at 1, `IntelMap` at 2, and `ShadowMap` at 3. Citations:
  `gfx/FX/border.shader:13`-`gfx/FX/border.shader:19`,
  `gfx/FX/border.shader:25`-`gfx/FX/border.shader:31`,
  `gfx/FX/border.shader:34`-`gfx/FX/border.shader:40`,
  `gfx/FX/border.shader:43`-`gfx/FX/border.shader:49`.
- The pixel shader samples the border material with
  `float2( Input.uv.y * BORDER_TILE, Input.uv.x )`; `BORDER_TILE` is `0.4f`.
  Citations: `gfx/FX/border.shader:127`,
  `gfx/FX/constants.fxh:103`.
- The only shipped scalar found for thin border geometry width is
  `NGraphics.BORDER_WIDTH = 1.5`; `constants.fxh` explicitly points border
  width back to defines.lua. Citations: `common/defines/00_graphics.lua:773`,
  `gfx/FX/constants.fxh:104`.
- The shipped border fade defines are camera-height **visibility windows**, not
  geometry widths. In `common/defines/00_graphics.lua` the `NGraphics` block
  declares `BORDER_WIDTH = 1.5`, `PROVINCE_BORDER_FADE_NEAR = 200`,
  `PROVINCE_BORDER_FADE_FAR = 300`, `STATE_BORDER_FADE_NEAR = 400`, and
  `STATE_BORDER_FADE_FAR = 500`. These `*_FADE_NEAR/FAR` values are camera-height
  thresholds controlling **alpha** (a border class fades out as the camera pulls
  back past its window), and they do not change the expanded-quad width used by
  the synthesized geometry. The engine wiring is corroborated by
  `gfx/FX/border.shader:146`, whose pixel shader multiplies the output alpha by
  `vTime_Transparency.y`
  (`return float4( vColor.rgb, max( vColor.a, vPulseFactor - 0.2f ) * vTime_Transparency.y )`);
  `vTime_Transparency.y` is the per-draw transparency the engine drives from the
  fade window. The exact fade curve (how a camera height maps through
  `*_FADE_NEAR/FAR` to that alpha) is computed engine-side and is not present in
  any shipped text file, so it remains an open unknown. Citations:
  `common/defines/00_graphics.lua:773`-`common/defines/00_graphics.lua:777`,
  `gfx/FX/border.shader:146`, `gfx/FX/border.shader:75`.
- The shipped border material family is `map/terrain/border_*.dds` alongside
  `provinces.bmp`, `definition.csv`, and `gfx/FX/border.shader`. Citation:
  `replay_interface/documents/GAME_FILES_ATLAS.md:499`.

## Derived Geometry Law

- Candidate border segments are the 4-neighbor grid edges in `provinces.bmp`
  whose decoded province ids differ. This is a derived rule from the
  `provinces.bmp` binding and definition color table, not a captured engine
  trace. Citations: `map/default.map:2`, `map/definition.csv:1`.
- Horizontal map wrapping is included for the left/right map seam because the
  map is a world map and the shader uses map-space coordinates; this remains a
  capture-check item because no `borders_pass` geometry has been observed.
  Citation for shader map-space position input:
  `gfx/FX/border.shader:59`.
- Water classification uses `definition.csv` type: `sea` and `lake` are water,
  `land` is land. This matches the existing map visual water mask in
  `cpu_compose.rs`, where water is `province_type == "sea" ||
  province_type == "lake"`. Citations: `map/definition.csv:5`,
  `map/definition.csv:2`,
  `replay_interface/native/map_viewer/src/cpu_compose.rs:33`-`replay_interface/native/map_viewer/src/cpu_compose.rs:35`.
- Land/land boundaries are ranked by 1936 state/country mapping: same state is
  `province`, different states with the same owner is `state`, and different
  owners is `country`. This is the file-derived ranking from state
  `provinces` and `owner` fields. Citations:
  `history/states/1-France.txt:10`,
  `history/states/1-France.txt:24`-`history/states/1-France.txt:26`.
- An explicit `impassable` adjacency row in `map/adjacencies.csv` overrides the
  province/state/country rank for that exact land/land province pair, tagging
  the raster edge as `impassable` material. The override is applied inside the
  land/land branch only (before the province/state/country rank decision), so a
  coastline (land/water) or water/water edge is never turned impassable by an
  adjacency row. This is disjoint from the state-level `impassable = yes`
  wasteland heuristic (`history/states` `impassable` flag): both can raise the
  `impassable` material, but the adjacency override keys on the exact province
  pair while the state heuristic keys on either endpoint's state. On vanilla
  1.16 the 93 `impassable` adjacency pairs are all land/land (0 non-land rows,
  0 missing-state provinces) and reclassify 1942 raster segments. The two
  mechanisms are measured disjoint: none of the 93 pairs has an endpoint in an
  `impassable = yes` state (15 pairs are same-state — which the rank heuristic
  would otherwise call `province` — and 78 are passable different-state pairs
  the heuristic would call `state`/`country`), so all 1942 segments are newly
  reclassified rather than double-counted with the wasteland heuristic.
  Citations: `map/adjacencies.csv:1`, `map/adjacencies.csv:146`,
  `map/adjacencies.csv:147`.
- Coastline consistency is a real cross-check, not a tautology. The synthesized
  coastline count comes from the border classification path (segments tagged
  `land_water`). Independently, a second pass rebuilds a per-pixel water mask
  through the exact renderer rule (`province_type == "sea" || "lake"`, missing
  id treated as water via `unwrap_or(true)`) from `cpu_compose.rs`, then walks
  the same 4-neighbor raster-edge domain (including the wrapped left/right seam)
  and counts an edge as a coastline edge when exactly one pixel is water.
  `coastline_disagreement_edges` is the number of edges where the two passes
  disagree; on vanilla 1.16 it is 0 (synthesized 124945 = water-mask 124945).
  The number is honestly computed and reported; a nonzero value would indicate a
  regression in edge enumeration, wrap handling, or water classification.
  Citations:
  `replay_interface/native/map_viewer/src/cpu_compose.rs:33`-`replay_interface/native/map_viewer/src/cpu_compose.rs:35`.
- Land/water boundaries are tagged as coastline material (`border_sea`) with
  province rank; water/water boundaries are tagged as sea-region material
  (`border_sea_region`) with province rank. The existence of these shipped
  materials is in the border material family. Citation:
  `replay_interface/documents/GAME_FILES_ATLAS.md:499`.
- Geometry is emitted as expanded quads around each two-point grid-edge
  polyline. The logical vertex layout is 20 bytes: three `f32` position values
  followed by two `i32` uv values, matching the HLSL input fields. Citations:
  `gfx/FX/border.shader:57`-`gfx/FX/border.shader:60`.
- Vertex height samples `heightmap.bmp` at the generated map position and uses
  the same `0.1 * height` scale used by the river world precedent. This is a
  synthesis rule, not proven by a captured border VB. Static heightmap input is
  cited at `map/default.map:6`.
- Width is currently `1.5` for every material class, sourced from
  `NGraphics.BORDER_WIDTH`. Per-material texture dimensions affect material
  appearance, but no shipped text file declares per-class geometry widths.
  Citations: `common/defines/00_graphics.lua:773`,
  `gfx/FX/constants.fxh:104`.

## Fail-Closed Rules

- Missing `default.map`, `definition.csv`, `provinces.bmp`, `heightmap.bmp`,
  `00_graphics.lua`, or `history/states` data blocks synthesis. Citations:
  `map/default.map:1`, `map/default.map:2`, `map/default.map:6`,
  `common/defines/00_graphics.lua:773`,
  `history/states/1-France.txt:24`-`history/states/1-France.txt:26`.
- Any `provinces.bmp` pixel color not present in `definition.csv` blocks
  synthesis instead of becoming province id 0. Citation for the color table
  shape: `map/definition.csv:1`.
- Any land/land boundary whose province lacks a state, whose state is missing,
  or whose passable state lacks a 1936 owner blocks synthesis. Citations:
  `history/states/1-France.txt:3`,
  `history/states/1-France.txt:10`,
  `history/states/1-France.txt:24`-`history/states/1-France.txt:26`.
- Any `province_type` in `definition.csv` other than `land`/`sea`/`lake` blocks
  synthesis (fail-closed at `border_world_synthesis.rs` `province_is_water`)
  rather than defaulting the province to land or water. The vanilla census is
  10155 `land` / 3133 `sea` / 126 `lake`, so no other type is expected.
  Citation: `map/definition.csv:1`.
- Any `Type` value in `map/adjacencies.csv` other than `impassable`, `sea`, or
  blank blocks synthesis with an error naming the offending type
  (`parse_impassable_adjacencies`). Blank `Type` (strait/adjacency-rule
  crossings), the `#` legend line, the `-1;-1;;...` terminator sentinel, and
  empty lines are skipped; a non-numeric `From` field never reaches `u32`
  parsing. Citation: `map/adjacencies.csv:1`.

## Render Law (BORD2, `--borders-from-files`, default off)

The BORD2 pass renders the synthesized mesh above with the shipped border shader,
files-first through the mod-aware resolver. It is default-off and byte-identical
when off (no shader compile, no allocation). Implemented in
`replay_interface/native/map_viewer/src/border_law.rs` (the law) and
`border_pass.rs` (the GPU pass).

- **Shader.** `gfx/FX/border.shader` `Effect border` (VS + PS) is compiled
  byte-exact through the OFF-3 machinery (`build_stage_hlsl` + `d3d_compile_stage`
  at default flags). Both stages compile against the shipped files. Citations:
  `gfx/FX/border.shader:180`-`gfx/FX/border.shader:184` (`Effect border`).
- **Vertex layout.** The synthesized mesh is one interleaved buffer (stride 20:
  `float3 position` + `int2 uv`); the input layout declares POSITION
  (`R32G32B32_FLOAT`, offset 0) and TEXCOORD (`R32G32_SINT`, offset 12), both from
  input slot 0. This is functionally equivalent to the shader's `VS_INPUT_BORDER`.
  The engine's own border draw split the two fields across two vertex streams (see
  Open Unknowns), which does not change the values the VS reads. Citations:
  `gfx/FX/border.shader:57`-`gfx/FX/border.shader:60`.
- **BorderDiffuse (per class).** Each class binds its own shipped
  `map/terrain/border_<class>_0.dds` (`border_law::border_diffuse_source`). The
  one captured `borders_pass` (`diag23b`) recorded a single 32×32
  `R8G8B8A8_UNORM` RUNTIME `BorderDiffuse` for an all-classes draw, NOT these
  files, so per-class DDS is a files-first APPROXIMATION of the runtime texture
  (tracked below). Citation: `replay_interface/documents/GAME_FILES_ATLAS.md:499`.
- **Other PS resources.** The shader declares four samplers (`BorderDiffuse`
  0, `SnowMudData` 1, `IntelMap` 2, `ShadowMap` 3), but the COMPILED PS blob
  keeps exactly two in its DXBC reflection: `BorderDiffuse` (t0/s0) and
  `ShadowMap` (t3/s3) — verified empirically by parsing the reflection of the
  blob the pass compiles (FXC strips the commented-out `SnowMudData` and the
  unreferenced `IntelMap`; the diag23b captured PS, a different 1064-byte
  engine blob, listed all four). The pass binds `BorderDiffuse` per class and
  one NEUTRAL `(1, 1, 0, 1)` texel for the other three slots (1/2 are
  defensive no-ops for this blob; 3 is load-bearing). The shipped DX11 header maps `sampler2DShadow` to a plain
  texture+sampler and `tex2Dproj` to `SampleLevel(uv.xy/uv.w, 0)`
  (`gfx/FX/defines_hlsl_dx11.fxh:47`, `:54`), so `ShadowMap` is sampled as a
  regular texture whose `.r` is the shadow term source and whose `.g/.b` are
  the FOW factors (`shadow.fxh:44`-`shadow.fxh:53`, `fow.fxh:85`-`fow.fxh:89`);
  the neutral texel therefore yields shadow term 1.0 and FOW alpha 0 — borders
  render unshadowed/unfogged, a documented approximation (the captured
  screen-space shadow buffer belongs to the captured camera and would misalign
  under replay cameras; only a live `borders_pass` capture settles the real
  runtime binding). `SnowMudData` is commented out of the PS body and
  `IntelMap` is declared-but-unreferenced, so the neutral texel is inert for
  both. Citations: `gfx/FX/border.shader:13`-`gfx/FX/border.shader:49`,
  `gfx/FX/border.shader:127`-`gfx/FX/border.shader:146`.
- **Blend.** From the shader `BlendState` block, NOT the capture (which recorded
  `default_state`): `BlendEnable = yes`, `SourceBlend = src_alpha`,
  `DestBlend = inv_src_alpha`, `WriteMask = RED|GREEN|BLUE` (no alpha write).
  Borders are an overlay: depth test on, depth write off. Citations:
  `gfx/FX/border.shader:160`-`gfx/FX/border.shader:167`.
- **Camera-height fade (the ONE ramp function).** The PS scales output alpha by
  `vTime_Transparency.y` (`border.shader:146`); the engine sets that per draw from
  the camera height and the fade windows. `border_law::border_transparency` is
  the single function that reconstructs it: `1.0` at/below the class `NEAR`, `0.0`
  at/above `FAR`, **linear** between. The windows are files-exact
  (`PROVINCE 200/300`, `STATE 400/500`); the LINEAR curve is a documented
  APPROXIMATION — only the endpoints are certain, and only a live `borders_pass`
  capture that records `vTime_Transparency.y` at a known camera height can settle
  the true easing. Per-frame the mesh + index buffers are uploaded once and each
  class draws its own contiguous index range (`class_index_partition`) with its
  own `BorderDiffuse` and its own transparency in CB2, so the fade re-scales each
  class every frame without re-uploading geometry. Citations:
  `common/defines/00_graphics.lua:774`-`common/defines/00_graphics.lua:777`,
  `gfx/FX/border.shader:146`.
- **Class → window mapping.** `province → province window`;
  `state → state window`; `impassable → state window` (the synthesis ranks
  impassable as `State`; the missing `IMPASSABLE_BORDER_FADE_*` define means only
  that impassable has no SEPARATE window, not that it is opaque — a tracked
  approximation, the alternative being permanently opaque); `country`,
  `sea_coast`, `sea_region → no window` (fully opaque, no shipped fade define).
- **Draw order.** The capture layer's pass-boundary sequence declares
  `terrain → water → rivers → borders → trees → cities → overlays`
  (every `map_capture_session.json`, including `diag23b`, which is the one
  session where a `borders_pass` bundle actually FIRED — `finalized_by:
  pass_close` with real buffers — inside that slot), corroborated by
  `replay_interface/documents/MAP_REPLICA_PLAN.md`. The synthesized border pass
  therefore draws AFTER water and rivers and BEFORE trees, over the terrain
  color/depth targets. Caveat: the manifest order is the capture tool's
  declared moment sequence (derived from observed engine pass boundaries), not
  a per-draw trace; a clean live `borders_pass` capture would settle it
  per-draw.

## BORD3 — the decoded texture-appearance law (evidence, not taste)

Every border-appearance parameter traces to a shipped byte. BORD3 decoded the
shipped `map/terrain/border_<class>_0.dds` cross-sections and the one captured
`borders_pass`, and encoded both as regression-protected laws
(`map_viewer/src/border_texture_law.rs`,
`map_viewer/src/border_capture_evidence.rs`) so no future reader can slip in an
invented value.

### The across-path UV mapping (where the soft edge lives)

`gfx/FX/border.shader:127` samples `tex2D( BorderDiffuse, float2( Input.uv.y *
BORDER_TILE, Input.uv.x ) )`. So texture **U** (width) = `uv.y * BORDER_TILE`
(along-path station, `BORDER_TILE = 0.4f` `constants.fxh:103`, `AddressU = Wrap`
`border.shader:22`), and texture **V** (height) = `uv.x` (**across-path**,
`AddressV = Clamp` `border.shader:23`). The synthesized quad emits `uv.x = side
∈ {0,1}` across its `BORDER_WIDTH`-wide extent, so the GPU interpolates `V = 0 →
1` across the quad and samples the texture's full across-path cross-section —
the authored alpha ramp IS the soft edge, and the ink centred at `V = 0.5`
(corroborated by the pulse's `abs(Input.uv.x - 0.5f)`, `border.shader:133`).

### Decoded per-class cross-sections (vanilla 1.16, top mip, middle column)

Measured by `border_texture_law::decode_border_texture_profile` and asserted in
its tests (a mod/patch re-authoring a texture changes these and fails loudly):

| class      | dims    | fmt   | peak α | ink rows (of h)      | ink RGB          |
|------------|---------|-------|--------|----------------------|------------------|
| province   | 64×32   | DXT5  | 179    | 14–17  (≈12% centre) | black `(0,0,0)`  |
| state      | 128×64  | DXT5  | 114    | 21–42  (≈34% centre) | black `(0,0,0)`  |
| country    | 256×128 | RGBA8 | 207    | 32–95  (≈47% centre) | navy `(24,26,43)`|
| sea_coast  | 64×32   | RGBA8 | 114    | 10–21  (soft bell)   | black `(0,0,0)`  |
| sea_region | 64×32   | DXT5  | 219    | 4–27   (wide bell)   | black `(0,0,0)`  |
| impassable | 128×64  | RGBA8 | 255    | 0–63   (full, soft)  | brown `(16,8,8)` |

Two consequences settle two user-reported defects, both files-first:

1. **No border texture is red.** province/state/sea are pure-black RGB, country
   is dark navy, impassable is brown. The GPU pass binds these DDS directly, so
   its country border draws as a thin dark navy line — a rendered ON-vs-OFF diff
   of the pass shows ~22.9k darkened pixels and **essentially zero red pixels**.
   Any "hot red band" seen at close zoom is therefore NOT this thin-border pass:
   it is the separate gradient-border/political-fill layer (`cpu_terrain.rs`
   gradient borders + state political colour), which is out of this pass's scope.
   The old CPU debug oracle (`bin/dump_border_render.rs`) DID paint classes with
   invented colours (`country = [255,83,73]` red, etc.); BORD3 replaced those
   with the decoded `peak_rgb`/`peak_alpha` so the oracle can no longer imply a
   colour vanilla never draws.
2. **Per-class thickness is authored in the textures, not in geometry.** All
   classes share the single `BORDER_WIDTH = 1.5` quad; the visible line thickness
   is the class's ink-band fraction of that quad — province ≈12% (hair-thin), sea
   ≈31–69% (fat, soft), impassable 100%. This reproduces vanilla's "thin subtle
   province lines, thick soft sea lines" WITHOUT a per-class width constant (none
   is shipped — see Open Unknowns). The residual hard/aliased look at diagonals
   is geometric (independent axis-aligned per-raster-edge quads, no MSAA), and
   the correct corner treatment is UNKNOWN because the one capture is
   contaminated (below) — inventing a mitering/width scheme would violate the
   files-first mandate, so it is left as a tracked capture item.

### PROVEN: the `diag23b` `borders_pass` capture is contaminated (not borders)

BORD1/BORD2 flagged this capture as an *unresolved* unknown; BORD3 decoded it and
proved it carries **zero usable border evidence**
(`border_capture_evidence::classify_captured_border_geometry`):

- Its IA POSITION stream is `R16G16B16A16_SINT` stride 8 (4×i16), decoding to a
  perfect **65×65 integer grid** (`vertex i = (i % 65, i / 65, …)`, values
  `0..64`), and its index buffer (`R16_UINT`, 24576 indices) is a **64×64-cell
  grid tessellation** (`0,1,66 / 0,66,65`, row stride 65). That is a terrain
  tessellation patch, NOT border polylines (which are 4-vert/6-index independent
  quads with float world positions). The classifier returns
  `TerrainGridPatch { grid_side_cells: 64 }`.
- Its recorded `BorderDiffuse` (and `IntelMap`/`ShadowMap`) SRV payloads are
  **all-zero RGBA**, and the captured pre-draw vs post-pass render targets are
  **byte-identical (0 diffs)** — the "border draw" changed nothing on screen.
- Its blend/depth/rasterizer were `default_state` with null descriptors, and its
  CB2 was truncated (16 of the declared 48 bytes, nonsense values).

So `diag23b` does NOT validate the border geometry/width/UV/tint, and it is NOT a
usable appearance reference. All BORD3 appearance law comes from the shipped
files. `DIAG23B_BORDERS_PASS_LAYOUT` and the classifier lock this verdict so a
future re-decode disagreeing with it fails a test.

## Open Unknowns

- PROVEN CONTAMINATED (was "unresolved"): the one captured `borders_pass`
  (`diag23b`) is a mis-recorded 65×65 terrain grid patch with an all-zero
  `BorderDiffuse` and byte-identical pre/post render targets — see the BORD3
  section above. It provides no border geometry/width/UV/tint. A CLEAN live
  `borders_pass` capture is still needed to settle the items below.
- The runtime `BorderDiffuse` a genuine capture would record: the `diag23b`
  32×32 `R8G8B8A8_UNORM` slot was all-zero, so how the engine composites the
  per-class `border_*.dds` family into a runtime texture (one atlas vs per-class
  draw) is still unknown. The pass binds the per-class DDS files as a files-first
  stand-in (their decoded profiles above are the appearance ground truth).
- Corner/junction treatment (mitered vs rounded vs overlapped quads) and engine
  polyline run merging are UNKNOWN — the only capture is the contaminated grid,
  so there is no evidence for the corner geometry. The synthesized pass emits
  independent axis-aligned per-raster-edge quads (which staircase at diagonals);
  a mitered/rounded scheme is deliberately NOT invented. **Capture plan:** a
  clean `borders_pass` VB (float POSITION + int UV, 6-index quads, classified as
  `BorderPolylines`) at a known camera height would reveal whether consecutive
  edges are merged into strips and how normals are handled at bends.
- The engine's real border depth/stencil, rasterizer (cull winding), and
  sampler states are uncaptured (`default_state`, null descriptors in
  `diag23b`); the BORD2 pass's LESS_EQUAL/write-off depth, CULL_NONE raster,
  and Samplers-block-derived samplers are implementation law pending capture.
- The exact per-class `vTime_Transparency.y` values the engine uploads (the
  fade curve between the NEAR/FAR endpoints, and impassable's real window) need
  a live capture of the border CB2 at known camera heights.
- Exact D3D input-layout packing still requires capture. The emitted logical
  layout follows the shader input fields, but the engine could pack those
  fields with a smaller DXGI format.
- Per-class geometry width beyond `NGraphics.BORDER_WIDTH = 1.5` is not declared
  in shipped text, so the pass uses the single `BORDER_WIDTH` for every class and
  lets each texture's ink-band fraction (BORD3 table above) set the visible
  thickness. This already reproduces vanilla's thin-province / fat-sea contrast
  without inventing a width per class. Whether the engine ALSO scales the quad
  width per class is UNKNOWN. **Capture plan:** a clean border VB would show the
  world-space quad half-width directly (compare consecutive cross-edge vertex
  pairs, as the rivers precedent measured `river_world_width_for_class`).
  Citations: `common/defines/00_graphics.lua:773`,
  `replay_interface/documents/GAME_FILES_ATLAS.md:499`.
- Shader runtime inputs `SnowMudData`, `IntelMap`, and `ShadowMap` are declared
  by the border shader but are runtime resources, not static geometry sources.
  Citations: `gfx/FX/border.shader:25`-`gfx/FX/border.shader:31`,
  `gfx/FX/border.shader:34`-`gfx/FX/border.shader:40`,
  `gfx/FX/border.shader:43`-`gfx/FX/border.shader:49`.
- The exact border fade curve is engine-side. The shipped
  `PROVINCE_BORDER_FADE_NEAR/FAR` (200/300) and `STATE_BORDER_FADE_NEAR/FAR`
  (400/500) name the camera-height windows, and `border.shader:146` shows the
  output alpha is scaled by `vTime_Transparency.y`, but the function mapping
  camera height through those thresholds to `vTime_Transparency.y` is not in any
  shipped text file and requires a live capture to pin down. Citations:
  `common/defines/00_graphics.lua:774`-`common/defines/00_graphics.lua:777`,
  `gfx/FX/border.shader:146`.

## BORD4 — the GRADIENT-BORDER system (the borders you actually see in vanilla)

The thin-border pass above (`border.shader`) is a *separate* mapmode/context pass.
A decisive at-launch capture session (`session_CAPFIX2_GAPS_4`, hooks installed
before any shader creation) recorded **zero** `border.shader` pixel/vertex shader
creations across a full close→far→close zoom sweep, while trees/underwater/mapname/
pdxmap all were created. The borders visible in normal vanilla play are therefore
NOT `border.shader` draws — they are the **GRADIENT-BORDER system** composited
inside the pdxmap terrain shader, consistent with BORD3 (the "hot red band" is the
gradient-border/political layer, and `border_*.dds` carry no red ink).

### The compositing math (engine truth, ported)

`gfx/FX/standardfuncsgfx.fxh` is byte-exact-recompilable (OFF-3), so its source IS
the engine math. `pdxmap.shader:325` calls
`gradient_border_apply( diffuse.rgb, normal, Input.uv2, GradientBorderChannel1,
GradientBorderChannel2, ... )` over the map-space UV. The function
(`standardfuncsgfx.fxh:886`-`928`) splits the ONE `GradientBorderChannel1` texture
vertically: **layer1 = top half** (`vUV.y *= 0.5 - HalfPix`, `:904`-`905`),
**layer2 = bottom half** (`+0.5`, `:906`), processing layer1 first (under) then
layer2 (over). `gradient_border_process_channel` (`:840`-`884`) reads channel1
`.a` as a distance/height field and `.rgb` as the border colour, converts the
height field to a camera-distance-dependent fill via
`gradient_border_distance_to_alpha` (`:818`-`823`), and gates each layer by
`GradientBorderChannel2.g` (`TranspA`/`TranspB`, `:913`,`:918`). Constants
(`constants.fxh:195`-`209`): `GB_CAM_MIN 100`, `GB_CAM_MAX 350`,
`GB_CAM_MAX_FILLING_CLAMP 0.8`, `GB_THRESHOLD 0.05`, `GB_THRESHOLD2 0.25`,
`GB_OPACITY_NEAR 1.0`, `GB_OPACITY_FAR 0.85`, `GB_FIRST_LAYER_PRIORITY 0.4`. This
math is ported faithfully (line-cited) in both `map_viewer/src/cpu_terrain.rs`
(`apply_terrain_gradient_borders` + helpers) and the GPU HLSL in
`replay_app/src/render.rs` (`gradient_border_apply`).

### The channel textures (measured ground truth, synthesized from files)

The two channel textures are `2816×2050` `B8G8R8A8` = two stacked `2816×1024`
half-res layers of the `5632×2048` province map (+ a zero separator row + a pad
row), an exact 2:1 zero-offset downsample (measured owned-land-mask agreement
99.48%, row-profile correlation 0.9999 at dy=0). Structure, measured across
capture sessions `session_20260702c_fixed`/`d` (Channel1 byte-identical across
both → deterministic, camera-independent) and cross-validated by two independent
forensic passes + Codex:

| region | layer | contents | class table |
|--------|-------|----------|-------------|
| **Channel1 TOP** | strategic-region / sea-zone | constant tint (`51,63,89` in RGB memory), distance field over sea | INERT in the pdxmap pass — Channel2 top `.g = 0` gates it off (default political mapmode) |
| **Channel1 BOTTOM** | **country layer** (the visible borders) | per-owner political RGB; alpha = distance ramp | **country** edge: wide monotone ramp, peak ≈253 (`0.99·255`) at the edge, slope ≈ −18.9/texel, 0 by ≈13.6 texels. **province-internal** edge: shallow dip ≈52→0 by 2 texels. **state-only** edge: ≈blanked. |
| **Channel2 BOTTOM** | land mask | `G=255` on owned land, `0` on sea (binary) | turns the country layer ON over land via `TranspB` |
| **Channel2 TOP** | naval supremacy | zeroed in the replay (no supremacy data) | disables the sea-zone layer |

The channels are SYNTHESIZED from files (mod-compatible) in
`replay_app/src/gb_field.rs` from `provinces.bmp` + `history/states` owners +
`country_colors`, not from captures. The country-layer alpha law is
`alpha = clip(255·(1 − (D − RAMP_D0)/T))` where `D` is a chamfer (5-7-11) distance
to the nearest different-country half-res texel (x-wrapped), `RAMP_D0 = 0.875`
(measured fit to the vanilla center-thickness plateau), and `T` is the per-country
thickness interpolated by pixel-area. This matches the measured country ramp
(peak-at-edge, ≈0 at ≈13.6 texels for a large country → `T ≈ 12.5`).

### BORD4 mod-compatibility fix (files-first)

`gb_field.rs` previously HARDCODED the four country-field/thickness defines. They
are now resolver-read from the mod-resolved `common/defines/00_graphics.lua`
(`CountryFieldDefines::from_resolver`), so a mod that re-tunes its country borders
is honoured. Citations:
`common/defines/00_graphics.lua:865` (`GRADIENT_BORDERS_FIELD_COUNTRY_LOW = 300`),
`:866` (`FIELD_COUNTRY_HIGH = 9000`), `:867` (`THICKNESS_COUNTRY_LOW = 5`),
`:869` (`THICKNESS_COUNTRY_HIGH = 25`), `:868`
(`GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS = 2`). The shipped-vanilla values are
the fail-open fallback when no resolver is present (headless), so the vanilla path
is byte-identical to the previous hardcoded field (env-gated regression test
`shipped_graphics_defines_match_vanilla_country_field`).

### BORD4 Open Unknowns (UNTRACED)

- `RAMP_D0 = 0.875` and the chamfer 5-7-11 metric are *measured engine
  texture-generation constants*, not shipped-file parameters — they are not
  resolver-readable. Because `RAMP_D0` was fit against the vanilla
  `GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS = 2.0`, a mod that changes that define
  would invalidate the fit; the synthesis therefore fails closed on a non-vanilla
  center thickness rather than render a wrong ramp. Deriving `RAMP_D0` from a
  modded center is UNTRACED.
- `GRADIENT_BORDERS_FIELD_COUNTRY_REFRESH = 10` (`00_graphics.lua:864`) is the
  engine's rebuild cadence; the replay rebuilds the country layer on every
  ownership-signature change instead — a timing/perf difference, not an appearance
  one.
- Channel2 `.b` (the `FX_Alpha.r` pulse/width selector) exact spatial law is
  UNTRACED (its distribution did not correlate with edge class in the captures;
  needs a capture with the selection pulse active).
- The purpose of the (inert) Channel1 TOP sea-zone band beyond `gradient_border_apply`
  is UNTRACED — it may feed the `dominance_fx_apply` / second GB path
  (`standardfuncsgfx.fxh:989`-`1060`) or be reserved.
- The per-channel runtime `vOutlineCutoff` (`vGBCamDistOverride_GBOutlineCutoff`)
  is a scene CB0 value decoded from the capture, not a channel input; the shipped
  `GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY = 0.973` (`00_graphics.lua:880`) vs the
  captured `0.9999` are a documented CB-vs-define discrepancy outside channel
  synthesis scope.
- Sub-texel appearance after the shader's 9-tap box multisample
  (`gradient_border_multisample_alpha`, `standardfuncsgfx.fxh:791`) is measured on
  the stored texel field only, not the post-filter values.
