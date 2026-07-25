# WEATHER_LAW — weather overlay rendering law (WEATHER1)

Mechanism-correct **render** law for the game airborne weather overlay (clouds,
rain, snowfall, blizzard, lightning storm, sandstorm), derived **files-first**
from the shipped installed content via the mod-aware `GameFileResolver`. This is the
RENDERING law: given a weather *type* per strategic region, how the engine
*draws* the overlay. The per-region *current type* is runtime state supplied by
the extractor feed (`REPLAY_DATA_REQUIREMENTS` item 6), NOT invented here.

Analysis + synthesis only — **no `replay_app` render wiring**. Threading the
overlay into the live pass is a later job.

Code: `replay_interface/native/map_viewer/src/weather_law.rs`, exported by
`replay_interface/native/map_viewer/src/lib.rs`.

Shipped sources of truth (game root
`<local-path> of Iron IV\`):
`common/weather.txt`, `gfx/entities/weather_entities.asset`,
`gfx/entities/particles.gfx`, `gfx/particles/environment/*_clouds.asset`,
`gfx/particles/*.dds`, `gfx/FX/particle.shader`, `gfx/FX/standardfuncsgfx.fxh`,
`map/weatherpositions.txt`, `map/strategicregions/*.txt`,
`common/defines/00_graphics.lua`.

---

## TL;DR

- The **airborne** weather overlay is a **particle system**, not a
  map-composite shader pass. There is **no** `cloud`/`weather`/`storm` string
  anywhere in `gfx/FX/*.shader` / `*.fxh`. The overlay the user sees from map
  altitude (clouds over the Alps) is the **`fading_entity`** (cloud) half of each
  weather effect, drawn with `gfx/FX/particle.shader`. (Ground **snow/mud** IS a
  shader effect — `pdxmap.shader:319-321` `GetMudSnowColor`/`ApplySnow` — but
  that is a separate terrain post-effect owned by SNOW1's `snow_law`, not this
  airborne-particle law.)
- Binding chain, fully traced from files:
  `map/strategicregions/*.txt` weather *type* → `common/weather.txt`
  `weather_effects` → `<name>_clouds_entity` in
  `gfx/entities/weather_entities.asset` → `<name>_clouds_particle` in
  `gfx/entities/particles.gfx` → `<name>_clouds_file` particle block in
  `gfx/particles/environment/<name>_clouds.asset` → sprite DDS + color + blend.
- The pixel-shader color law is exact: `ToLinear(color/255) · tex2D(DiffuseMap)`
  with one of three cited blend states (`ParticleAlphaBlend` /
  `ParticleAdditive` / `ParticlePreAlphaBlend`).
- Emitter world positions come from `map/weatherpositions.txt`
  (`region_id;x;y;z;{big|small}`).
- **Deterministic from files + a supplied type:** sprite texture, linear vertex
  color, blend mode, billboard size, spawn positions, and the `WEATHER_*` camera
  fade constants.
- **NOT derivable from files (extractor / capture state):** the per-region
  current weather type & intensity, the live per-particle ages (hence exact
  `smoke_fade` alpha), the camera height (hence the fade factor), the animated
  turbulence offsets, and the exact billboard perspective (runtime
  ViewProjection CB). See the UNTRACED list.

---

## 1. Weather is a particle system (no weather shader)

A whole-tree scan of `gfx/FX/*.shader` and `*.fxh` for `cloud|weather|storm`
returns **nothing**. The only weather-render assets are:

- `gfx/particles/environment/*_clouds.asset` + `*.asset` (rain/snow/sandstorm),
- `gfx/particles/*.dds` cloud sprites (`cloud_6.dds`, `cloud_7.dds`,
  `snow_cloud.dds`, …),
- `map/weatherpositions.txt` (emitter world positions),
- `gfx/FX/particle.shader` (`ParticleAlphaBlend` / `ParticleAdditive` effects).

So the render law is the **particle** law, parameterised per weather type.

## 2. The binding chain (DETERMINISTIC)

### 2.1 Type → effect (`common/weather.txt`, `weather_effects` block)

Each strategic-region weather *type* maps to one or two `weather_effect` rows,
each naming a foreground `entity` and (for precipitating types) a
`fading_entity` — the cloud layer that stays visible zoomed out. The second row
per type carries `small = yes`.

| type (`strategicregions` key) | big `fading_entity` | small `fading_entity` |
|---|---|---|
| `rain_light` | `rain_clouds_entity` | `rain_small_clouds_entity` |
| `rain_heavy` | `lightning_storm_clouds_entity` | `lightning_storm_small_clouds_entity` |
| `snow` | `snow_clouds_entity` | `snow_small_clouds_entity` |
| `blizzard` | `snow_storm_clouds_entity` | `snow_storm_small_clouds_entity` |
| `sandstorm` | *(none — no cloud layer)* | *(none)* |

(`no_phenomenon` draws nothing; `arctic_water` and `mud` are ground/water
post-effects, not airborne particles — `arctic_water`/snow ground is SNOW1's
`snow_law`, mud is the flooded static modifier.)

### 2.2 Entity → particle (`gfx/entities/weather_entities.asset`)

`entity = { name = "rain_clouds_entity" … state = { event = { … particle =
"rain_clouds_particle" … } } }`. The entity's `idle` state fires the named
`pdxparticle`.

### 2.3 Particle name → file (`gfx/entities/particles.gfx`)

`pdxparticle = { name = "rain_clouds_particle" type = "rain_clouds_file" }`. The
`type` is the `name=` of the `particle = { … }` block in the `.asset`.

### 2.4 File suffix convention

For the **weather cloud** files this law resolves, `<X>_clouds_file` →
`gfx/particles/environment/<X>_clouds.asset` (e.g. `rain_clouds_file` →
`rain_clouds.asset`). Verified for every vanilla `*_clouds_file`. NOTE
(Codex-flagged): this `environment/` directory convention is **not** universal
for all particle files — e.g. `cloud_effect_file` lives at
`gfx/particles/vehicles/cloud_effect.asset`, not `environment/`. The weather
resolver only ever follows `*_clouds_file` entries reached through the
`weather_effects` chain, so the convention holds for every path it takes; a
future non-weather particle would need its own `pdxparticle`-declared path.

## 3. The cloud subsystem (`gfx/particles/environment/*_clouds.asset`)

The first `subsystem` is the cloud billboard layer (the lightning variants add a
second `flash` additive subsystem — reported by `asset_subsystem_count`). The
render-relevant fields, per vanilla file:

| file | texture | shader | color (rgb/255) | base α/255 | α anim | size mean |
|---|---|---|---|---|---|---|
| `rain_clouds` | `cloud_6.dds` | `ParticleAdditive` | 110/120/150 | 180 | `smoke_fade` | 60.5 |
| `rain_small_clouds` | `cloud_6.dds` | `ParticleAdditive` | 110/120/150 | 180 | `smoke_fade` | 60.5 |
| `snow_clouds` | `snow_cloud.dds` | `ParticleAlphaBlend` | 200/200/200 | 60 | `smoke_fade` | 60.5 |
| `snow_small_clouds` | `snow_cloud.dds` | `ParticleAlphaBlend` | 255/255/255 | 30 | `smoke_fade` | 30.5 |
| `snow_storm_clouds` | `cloud_7.dds` | `ParticleAlphaBlend` | 220/220/220 | 60 | `smoke_fade` | 60.5 |
| `snow_storm_small_clouds` | `cloud_7.dds` | `ParticleAlphaBlend` | 220/220/220 | 50 | `smoke_fade` | 30.5 |
| `lightning_storm_clouds` (cloud layer) | `cloud_6.dds` | `ParticleAlphaBlend` | 160/160/160 | 150 | `cloud_fade` | 60.5 |
| `lightning_storm_small_clouds` (cloud layer) | `cloud_6.dds` | `ParticleAlphaBlend` | 180/180/180 | 120 | `cloud_fade` | 50.0 |

Sprite DDS are all **DXT5/BC3** (alpha in the texture): `cloud_6.dds`,
`cloud_7.dds` = 256×256; `snow_cloud.dds` = 128×128. Decoded via the crate's
`load_dds_rgba` (existing BC3 decoder).

**Draw-order note (Codex-flagged):** the `lightning_storm_clouds` subsystems
carry `sort="depth"` (`lightning_storm_clouds.asset:7`) — the engine depth-sorts
those particles back-to-front. The non-lightning cloud subsystems have no
`sort`. The `alpha`/`size` asset fields also carry a **variance** term (e.g.
`alpha={ 180,smoke_fade 20 }`, `size={ 60.5 20 }`); this law uses the **mean**
(first number) and treats the variance + per-particle spread as runtime spawn
state (U2). `snow_storm_clouds` additionally has a `planar` `sidewind` force —
motion sim, not render (U2).

## 4. The pixel-shader color law (`gfx/FX/particle.shader`) — EXACT

- Vertex: `Out.vColor = ToLinear( v.vColor )` (`particle.shader:151`).
  `ToLinear(float4) = float4( pow(rgb, 2.2), a )`
  (`standardfuncsgfx.fxh:53-56`) — gamma 2.2 on RGB, **alpha passed through**.
  The `v.vColor` `COLOR` semantic is a UNORM8. The shipped shader text does NOT
  contain the `/255` step — it receives an already-normalized `float4 vColor :
  COLOR`. The `/255` mapping from the 0..255 asset `color` into that UNORM
  vertex attribute is the engine's vertex-buffer fill, **inferred** (same
  convention as `color_lut.rs:128-130`), not read from a shader line. This is the
  one non-shader inference in the color law; it is the standard UNORM8 fill and
  is validated to the 3rd decimal by the resolved `linear_vertex_color` values.
- Pixel: `return tex2D( DiffuseMap, In.vUV0 ) * vColor` (`particle.shader:168`)
  — the linear sprite texel (RGB + its own alpha) times the linear vertex color.
- `FOG` define (on `ParticleAlphaBlend`, **off** `ParticleAdditive`) applies
  `ApplyDistanceFog` to RGB before return (`particle.shader:189-191`) — see
  UNTRACED U4.
- Blend states (`WriteMask = "RED|GREEN|BLUE"` — particles never write dest
  alpha):
  - `ParticleAlphaBlend` : Src=`SRC_ALPHA` Dst=`INV_SRC_ALPHA` (`:200-206`)
  - `ParticleAdditive` : Src=`SRC_ALPHA` Dst=`ONE` (`:208-214`)
  - `ParticlePreAlphaBlend` : Src=`ONE` Dst=`INV_SRC_ALPHA` (`:216-222`)

Ported in `ParticleBlend::blend_over` and unit-tested against the D3D equations.

## 5. Animation curves (`smoke_fade` / `cloud_fade`)

Each cloud `.asset` has `animation = { name="smoke_fade" curve={ x0 y0 x1 y1 … }
op="MUL" time="life" }`. The curve is **piecewise-linear**; it MUL-modulates the
base alpha over each particle's own life fraction. `evaluate_animation_curve`
samples it; `animation_curve_peak` returns its max. For the shipped clouds the
curve peaks at 1.0, so the representative (brightest) frame uses the base alpha.
The **live** per-particle age is runtime state (UNTRACED U2).

## 6. Emitter positions (`map/weatherpositions.txt`) — DETERMINISTIC

`region_id;x;y;z;{big|small}`, one emitter per line (588 vanilla rows, region
ids ≤ 288). `(x,y,z)` is the world position of a weather emitter on the same
`[0,5632] × [0,2048]` map plane in x/z that SNOW1 documents (y ≈ 9..15 height).
`big` selects the primary `weather_effect`, `small` the `small = yes` variant.
Parsed by the shared `map_static_data::weather_positions_from_source`; WEATHER1
decodes the `size` tag into `WeatherPositionSize` (`weather_emitters_from_*`).

## 7. Camera distance / zoom fade (`00_graphics.lua:1008-1015`)

Constants (cited exactly):

| define | value | meaning |
|---|---|---|
| `WEATHER_DISTANCE_CUTOFF` | 1500 | hidden beyond this camera distance |
| `WEATHER_DISTANCE_FADE_LENGTH` | 400 | far fade band length |
| `WEATHER_ZOOM_IN_CUTOFF` | 358 | most-faded when zooming in below this |
| `WEATHER_ZOOM_IN_FADE_LENGTH` | 220 | zoom-in fade band length |
| `WEATHER_ZOOM_IN_FADE_FACTOR` | 0.0 | residual factor at max zoom-in |
| `WEATHER_PLAYBACK_RATE` | 0.15 | (sim playback rate at max distance) |
| `WEATHER_PLAYBACK_RATE_CUTOFF` | 500 | |
| `WEATHER_PLAYBACK_RATE_LENGTH` | 200 | |

The `weather_camera_fade(camera_distance, defines)` function combines the far
and near bands into a 0..1 layer-visibility multiplier. **The constants are
real; the combining formula is a documented reconstruction** — the engine C++
that applies these has no shipped-text source (UNTRACED U3).

---

## UNTRACED list (honest — invented values are never acceptable)

| id | what | why untraced | plan / capture need |
|---|---|---|---|
| **U1** | per-region **current weather type + intensity** | pure runtime sim state; `common/weather.txt` gives only *probabilities* per period, and snow/mud is accumulated & seeded (see SNOW_LAW) | extractor feed `REPLAY_DATA_REQUIREMENTS` item 6: emit the active weather type per strategic region per tick |
| **U2** | live **per-particle age** → exact `smoke_fade` alpha, spawn count, turbulence offsets | continuous particle simulation, not a file | a `weather_pass` capture moment must record, for a region with active weather: the live particle instance buffer (per-particle position, size, rotation, color/alpha) OR accept the file-derived representative (peak-curve) frame |
| **U3** | the exact **camera fade curve** shape | constants cited but the C++ combine is not in any file | a capture at several zoom levels recording the applied per-layer fade factor vs. camera height would pin the shape; until then `weather_camera_fade` is flagged reconstruction |
| **U4** | `ApplyDistanceFog` RGB stage (`FOG` define) | needs the runtime fog constants (fog color/range CB), absent from shipped text | a `weather_pass` capture must record the particle VS/PS constant buffers incl. fog params; treated as identity until then |
| **U5** | exact **billboard perspective projection** | `particle.shader` billboards through the runtime `ViewProjectionMatrix` + `ProjectionMatrix` CB (per-frame camera state) | owned by the camera lane; the dump bin uses an explicit *orthographic top-down* projection instead of inventing a camera |
| **U6** | `sandstorm` visual | `sandstorm` has **no** `fading_entity` (no cloud layer); its foreground `sandstorm.asset` is a ground-level dust effect, out of the "clouds-from-altitude" overlay scope | if a sandstorm overlay is wanted, port `gfx/particles/environment/sandstorm.asset`'s foreground subsystem separately (same particle.shader law) |

### What a `weather_pass` capture moment must record (to close U2–U4)

For one map frame over a region with **active, known** weather:
1. The particle draw's **VS CB1/CB2/CB3** (`ProjectionMatrix`, `Scale`,
   `InstanceWorldMatrix`, `HalfPixelWH_RowsCols`, `vLocalTime`) — pins U5 and the
   tile atlas UV.
2. The bound **fog constants** used by `ApplyDistanceFog` — pins U4.
3. The live **per-particle instance buffer** (position/size/rotation/color) OR
   confirmation that the file-derived peak-curve frame is close enough — pins U2.
4. The **applied layer fade factor** at the capture's camera height — pins U3.
5. The bound **DiffuseMap** hash — cross-checks the file-derived sprite DDS
   (validation only; the law's sprite comes from the `.asset`).

---

## Files touched

- `replay_interface/native/map_viewer/src/weather_law.rs` (new) — the law.
- `replay_interface/native/map_viewer/src/lib.rs` — module registration +
  public exports only.

Reuses the shared `map_static_data` weatherpositions parser (no duplication).

## Gate summary

- `cargo fmt --check`: clean.
- `cargo test -p map_viewer --lib`: 477 passed, 0 failed (15 new weather tests).
- Assert-scan (`panic!`/`unwrap()`/`expect(`/`debug_assert` on non-test paths):
  clean in both new files.
- Dump PNGs produced for all 5 types (`map_viewer/out/weather_law_dump/`);
  params JSON per type cross-checks the resolved law.
- Flag-off byte-identical: **N/A by construction** — no `replay_app` render
  wiring was added (law + dump bin only), so the live render path is untouched.
