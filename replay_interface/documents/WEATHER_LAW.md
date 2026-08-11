# WEATHER_LAW — weather overlay rendering law (WEATHER1)

> Ranks under `documents/DECISIONS.md`, which is the authority on intent; where this
> document disagrees with it, `DECISIONS.md` wins. Status: **XMAP19 production
> implementation** — exact-a729 runtime selection/visibility laws, a
> presentation-independent cold-seek carrier, effective-VFS content closure,
> foundation preparation, and normal app admission are implemented.

Mechanism-correct **render** law for the game airborne weather overlay (clouds,
rain, snowfall, blizzard, lightning storm, sandstorm), derived **files-first**
from the shipped installed content via the mod-aware `GameFileResolver`. This is the
RENDERING law: given a weather *type* per strategic region, how the engine
*draws* the overlay. Exact D24 supplies the current per-region gameplay state.
The only additional runtime input is the two-word host-seeded visual epoch
recorded on initial baseline and airborne-code changes. Recorder camera and
selected-Present submissions are forbidden production authorities.

Code: `replay_interface/native/map_viewer/src/weather_law.rs`, exported by
`replay_interface/native/map_viewer/src/lib.rs`.

Vanilla examples (game root
`<local-path> of Iron IV\`):
`common/weather.txt`, `gfx/entities/weather_entities.asset`,
`gfx/entities/particles.gfx`, `gfx/particles/environment/*_clouds.asset`,
`gfx/particles/*.dds`, `gfx/FX/particle.shader`, `gfx/FX/standardfuncsgfx.fxh`,
`map/weatherpositions.txt`, `map/strategicregions/*.txt`,
`common/defines/00_graphics.lua`.
Production does not bind the example entity, particle-registration, or shader
filenames. It enumerates the admitted effective `gfx/entities/**/*.asset`,
`gfx/entities/**/*.gfx`, `gfx/particles/**/*.asset`, and `gfx/FX/**/*.shader`
namespaces, resolves every referenced authored name exactly once, and retains
the real winning `UsedContentKey` for each source.

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
- **Runtime inputs:** D24 supplies exact current flags/intensity/expiry; the
  XMAP19 lifecycle carrier supplies only the host-seeded visual epoch; XMAP03
  supplies viewer camera; XMAP09 supplies lighting; compile-time distance fog
  comes from the admitted particle-shader closure; XMAP10 owns final
  composition. XMAP19 supplies XMAP20 an exact authored-trigger index; XMAP20
  owns generic particle simulation and execution. Particle age/variance remains
  normal renderer temporal state, not captured presentation authority.

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

### 2.2 Entity → particle (effective `gfx/entities/**/*.asset`)

`entity = { name = "rain_clouds_entity" … state = { event = { … particle =
"rain_clouds_particle" … } } }`. The entity's `idle` state fires the named
`pdxparticle`.

### 2.3 Particle name → file (effective `gfx/entities/**/*.gfx`)

`pdxparticle = { name = "rain_clouds_particle" type = "rain_clouds_file" }`. The
`type` is the `name=` of the `particle = { … }` block in the `.asset`.

### 2.4 Particle-definition discovery

Production does not infer a filename from `<type>_file`. It first resolves the
named entity and every `pdxparticle` registration uniquely across their complete
admitted effective namespaces. It then indexes admitted
effective `gfx/particles/**/*.asset` definitions by their authored
`particle.name`, counts every matching block (including duplicates in one
file), requires exactly one definition, then closes the referenced texture and
uniquely resolved shader-Effect translation-unit graph. Shader Effects are
likewise counted by definition rather than by containing file. This preserves
mods that move or replace any definition without relying on a vanilla directory
convention, while refusing ambiguous same-file duplicates. Before parsing, the
adapter compares every admitted namespace with an independent effective-VFS
path enumeration and refuses an incomplete admission; an omitted mod file
therefore cannot hide a duplicate or overriding definition. Effective
Clausewitz namespace sources use the repository's lossless single-byte decode,
so valid mod bytes are retained rather than rejected as non-UTF-8. The prepared
`hoi4.xmap19.weather-atmosphere.a729.v2` program keeps the actual entity source,
particle-registration sources, particle-asset paths, shader Effect names,
shader paths, and admitted closure identities.

### 2.5 Authored trigger identity supplied to XMAP20

For each selected primary/fading producer ID, XMAP19 indexes the exact selected
entity `default_state`. `authored_event_index` is zero-based among every
`event={}` block, including sound-only blocks. `authored_effect_index` is
zero-based among the particle and sound invocations inside that event, in source
order. A particle trigger is therefore queried by the exact triple
`(producer_instance_id, authored_event_index, authored_effect_index)`.

The result preserves the admitted entity-file identity, entity/state names,
particle token, and optional authored `node`, `time`, `keep_particle`, and
`trigger_once` values. Omitted values remain absent; XMAP19 does not manufacture
a root node, time, or boolean default. The trigger is bound to the XMAP19
prepared-frame digest, and XMAP20 validates the returned row against its
unconditional lifecycle event before simulating it.

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
`alpha={ 180,smoke_fade 20 }`, `size={ 60.5 20 }`). The older static-overlay
diagnostic can summarize the first number, but it is not a production renderer
or promotion authority. Production XMAP19 closes the complete admitted asset
and XMAP20 applies its authored variance, forces, emission, and temporal
simulation without a representative mean/peak substitute. `snow_storm_clouds`
additionally has a `planar` `sidewind` force inside that generic simulation.

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
  `ApplyDistanceFog` to RGB before return (`particle.shader:189-191`). Its
  `FOG_COLOR`, `FOG_BEGIN`, `FOG_END`, and `FOG_MAX` are exact compile-time
  values from the admitted effective `constants.fxh`.
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
and `animation_curve_peak` remain diagnostic inspection helpers only.
Production XMAP20 evaluates the complete admitted curve at each simulated
particle age; it never substitutes the maximum or a representative phase.

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

Exact-a729 `CAirInterface::GetWeatherEffectForAirRegion`
(`hoi4.exe+0x0166ec50`) proves two role-specific alpha outputs:

- primary precipitation: alpha `1` through distance `1100`, then
  `(1500-distance)/400` through `1500`, then `0`;
- fading/cloud entity: alpha `0` below `358`,
  `(distance-358)/220` over `[358,578)`, `1` through `1100`, then the same far
  fade as primary through `1500`, then `0`.

Playback is `1` through distance `300`, linear to `0.15` over `(300,500)`,
then `0.15`. Production generalizes those operations through the effective
defines and rejects invalid or overlapping bands.

---

## Exact runtime boundary

| boundary | production treatment |
|---|---|
| D24 current phenomenon/intensity/expiry | exact 305-region census; named flags use engine priority `rain_light`, `rain_heavy`, `snow`, `blizzard`, `sandstorm` even when several are true; XMAP19 cross-checks visual epoch code and expiry |
| Authored variant choice | exact PRNG mix and file-order modulo; a measured-empty weather vector is valid and consumes no seed; big falls back to small only when the big vector is empty |
| Host-time seed | not reconstructible from game hour/files; minimal presentation-independent two-word epoch carrier required |
| Particle source identity | per-region code-change ordinal plus exact global weather-position ordinal and primary/fading role; full census is recorded and recomputed |
| Live particle ages/variance | renderer temporal state derived from admitted particle assets; no captured Present bytes or visually tuned phase |
| Billboard camera | viewer-owned XMAP03 matrices |
| Lighting | XMAP09 dependency; XMAP19 never supplies neutral constants |
| Distance fog | exact effective `constants.fxh` plus viewer/world positions |
| Final target/order | XMAP10 composition dependency |
| Snow/mud/ice | excluded; owned exclusively by XMAP07 |
| Observer FoW | excluded; owned exclusively by XMAP05 |

---

## Production files

- `replay_interface/native/replay_core/src/world.rs` — exact visual-epoch
  carrier admission.
- `replay_interface/native/map_viewer/src/weather_law.rs` — engine/file laws.
- `replay_interface/native/map_viewer/src/production_features/xmap19.rs` —
  effective-VFS catalog, exact selection, cold-seek preparation, family output.
- `replay_interface/native/map_viewer/src/production_foundation.rs` and
  `replay_interface/native/replay_app/src/app.rs` — normal product route and
  typed-unavailable enforcement.

## Promotion gate

Wrapper validation and the sole cold review are recorded with the promotion
commit. Recordings without the independent visual-epoch carrier remain
explicitly unavailable; they never fall back to `production_render`.
