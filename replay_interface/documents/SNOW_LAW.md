# SNOW_LAW — snow / mud "winter" law (SNOW1)

Mechanism-correct law for the game `SnowMudData` runtime texture (the
`SnowMudTexture` SRV that feeds terrain/tree/border/water snow & mud), derived
**files-first** from the shipped installed content (via the mod-aware
`GameFileResolver`) and validated against captured evidence. Analysis +
synthesis only — no render wiring (a later job threads the synthesized floor
into the terrain pass).

Code: `replay_interface/native/map_viewer/src/snow_law.rs` (+ validation bin
`src/bin/dump_snow_validation.rs`, exports in `src/lib.rs`).

Shipped sources of truth (game root
`<local-path> of Iron IV\`):
`gfx/FX/pdxmap.shader`, `gfx/FX/standardfuncsgfx.fxh`, `gfx/FX/constants.fxh`,
`map/strategicregions/*.txt`, `common/weather.txt`, `map/provinces.bmp`,
`map/definition.csv`, `common/defines/00_graphics.lua`.

---

## TL;DR

- The `SnowMudData` texture is **quarter-resolution** (`1408×512` =
  `5632/4 × 2048/4`), `B8G8R8A8_UNORM`, sampled once per terrain pixel via a
  world→UV law identical to the Fog-of-War texture.
- Its four channels pack `r=mud_fade0`, `a=mud_fade1`, `b=snow_fade0`,
  `g=snow_fade1`; the shader cross-fades fade0→fade1 by scene `SnowMudFade`.
- **Deterministic from files + date:** the addressing law, the packing/blend
  law, and each strategic region's `min_snow_level` floor for the active
  monthly weather period. The synthesized floor is a files-only climate /
  min-snow *policy* raster (a candidate lower bound), NOT a reproduction of the
  runtime `SnowMudData` bytes.
- **NOT derivable from files + date alone (extractor item):** the actual
  per-texel snow/mud values. Per `common/weather.txt`, snow is an *accumulated,
  seeded, pseudo-random* quantity (gain on snowing/blizzard, melt by
  temperature). Three captures give three distinct texture hashes (one entirely
  zero), and the captured snow is a small Arctic-only permanent-ice band — the
  full `min_snow_level` floor is not even visibly encoded in the analysed
  captures.

---

## 1. The texture and its addressing law (DETERMINISTIC)

**Declaration & sample.** `pdxmap.shader:77` declares `SnowMudData` at sampler
`Index = 7` (Linear, Wrap). It is sampled once per terrain pixel
(`pdxmap.shader:319`):

```
float4 vMudSnow = GetMudSnowColor( Input.prepos, SnowMudData );
```

`Input.prepos` is the world-space terrain position `(x, height, z)`.

**UV law.** `standardfuncsgfx.fxh:231–234`:

```
float4 GetMudSnowColor( float3 vPos, in sampler2D MudSnowTexture ) {
  return tex2D( MudSnowTexture,
    float2( ((vPos.x + 0.5f) / MAP_SIZE_X) * FOW_POW2_X,
            ((vPos.z + 0.5f) / MAP_SIZE_Y) ) * FOW_POW2_Y );
}
```

This is the **same** UV as the Fog-of-War texture (`standardfuncsgfx.fxh:193`).
`MAP_SIZE_X/Y` and `FOW_POW2_X/Y` are engine-injected (present in no shipped
text file). Measured from the captured runtime constants
(`live_PAN2/map_frame_bundle.json`): `MAP_SIZE_X = 5632`, `MAP_SIZE_Y = 2048`,
`FOW_POW2_X = FOW_POW2_Y = 1.0`. So the world spans `[0,5632] × [0,2048]` in
`(x, z)` and the SnowMud UV is simply `((x+0.5)/5632, (z+0.5)/2048)`.

> Adversarial note (Codex, addressed): a `FOW_POW2_X = 0.6875` (i.e.
> `5632/next_pow2 = 1408/next_pow2`) would only sample the left 68.75% of a
> `1408`-wide SRV. The **runtime constant is `1.0`** in every measured bundle,
> so the full texture is sampled; treat `FOW_POW2` as a runtime constant read
> from the bundle, not derived from the SRV size.

**Resolution.** The texture is `1408×512`, `source_dxgi_format = 87`
(`B8G8R8A8_UNORM`), `byte_len 2883584` in every capture. `1408 = 5632/4` and
`512 = 2048/4` → **quarter map resolution per axis**; the Linear sampler blends
between quarter-res texels.

**Orientation (Y-FLIP required).** `GetMudSnowColor` samples the **raw world
position** `Input.prepos` with `v = (z+0.5)/MAP_SIZE_Y`. Per `CalcGlobeNormal`
(`standardfuncsgfx.fxh:347–355`), `y = vWorldXZ.y/MAP_SIZE_Y` then
`y = SOUTH_POLE_OFFSET + (NORTH_POLE_OFFSET − SOUTH_POLE_OFFSET)·y` with
`SOUTH_POLE_OFFSET = 0.17`, `NORTH_POLE_OFFSET = 0.93` (fxh:330–331) — so
**world z=0 is SOUTH and z=MAP_SIZE_Y is NORTH**. Therefore SnowMud texture row
0 (v=0) = world z=0 = SOUTH and row 511 = NORTH. `load_provinces_bmp` yields
`ids` with row 0 = image top = NORTH, so the top-down raster must be
**flipped** into world-z order (`SnowFloorInputs::flip_source_y = true` in
production). Confirmed empirically: with the flip, NH-winter (January) Siberia
floors land in the high (north) texture rows ~270–511 (centroid 442) and
overlap the captured **Arctic** snow at rows 497–511. (Without the flip they
would incorrectly land in the low/south rows, disjoint from the capture.) Unit
test `floor_synthesis_flip_source_y_toggle_controls_orientation` pins the
toggle.

Rust: `SnowMudAddressing::world_to_uv` / `world_to_texel`,
`world_to_snow_mud_texel`.

---

## 2. Channel packing / temporal-blend law (DETERMINISTIC)

`standardfuncsgfx.fxh` decodes the sampled `float4 vMudSnowColor`:

- `GetMud` (`fxh:243`): `lerp( vMudSnowColor.r, vMudSnowColor.a, SnowMudFade )`.
- `GetSnow`/`GetWinter` (`fxh:261`):
  `lerp( vMudSnowColor.b, vMudSnowColor.g, SnowMudFade )`.

So the RGBA channels pack, per texel:

| HLSL | stored byte (BGRA order 87) | logical meaning |
|---|---|---|
| `.r` | byte 2 | `mud_fade0` (shown at `SnowMudFade=0`) |
| `.a` | byte 3 | `mud_fade1` (shown at `SnowMudFade=1`) |
| `.b` | byte 0 | `snow_fade0` |
| `.g` | byte 1 | `snow_fade1` |

**What the shader proves is only the fade endpoints.** The common gloss of
`(.b→.g, .r→.a)` as "previous month → current month" keyframes is an inference
about the C++ writer; `snow_law` names channels by *fade endpoint* to stay
honest.

`SnowMudFade` is `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.z`. The float4
sits at scene CB0 offset 240 (derived from the `ConstantBuffer(0,0)` layout in
`standardfuncsgfx.fxh`: `2×float4x4`(128) + `vGB…`(16) + 4 virtual-light
float4s(64) + `vCamPos`+`HdrRange`(16) + `vCamLookAtDir`+`vGlobalTime`(16) =
240), so `SnowMudFade` is at **CB0 offset 248**. Measured `SnowMudFade = 0.0`
in `restore_gate_1` and `live_PAN2` (FoWTime.y ≈ 22.8 / 23.3).

Rust: `SnowMudChannels`, `decode_snow_mud_texel`, `encode_snow_mud_texel`,
`SnowMudChannels::displayed_snow`/`displayed_mud`, `snow_mud_fade_from_cb0`,
`CB0_OFFSET_SNOW_MUD_FADE`.

---

## 3. Per-region snow floor — `min_snow_level` (DETERMINISTIC)

`map/strategicregions/*.txt` carry, per region, `weather = { period = { … } … }`
with **twelve monthly** `period` sub-blocks. Each period:

- `between = { START END }` — a `DAY.MONTH0` window (month 0-based; day 0-based
  on a nominal 31-slot month). `{ 0.0 30.0 }` = Jan, `{ 0.10 29.10 }` = Nov,
  `{ 0.11 30.11 }` = Dec.
- `temperature = { MIN MAX }` — stochastic per-day range.
- `snow`, `blizzard`, `rain_light`, `rain_heavy`, `mud`, `sandstorm`,
  `arctic_water`, `no_phenomenon` — **probabilities** (NOT deterministic).
- `min_snow_level` — a deterministic snow floor in `[0,1]` for that region &
  period.

Distribution of `min_snow_level` across all 304 vanilla regions (period count
by value): `0.000 ×3513`, `0.100 ×59`, `0.050 ×32`, `0.010 ×20`, `0.200 ×16`,
`0.300 ×8`. **Max 0.3**, reached only in the deep-Siberian regions
(`147-Siberian Region`, `256-Transbaikal`, `259-Sakha`, …) in January
(`between={0.0 30.0}`, `temperature={-40 -12}`, `snow=0.35`, `blizzard=0.30`).

`snow_law` parses these directly (`region_weather_from_resolver`,
`parse_region_weather`, `WeatherPeriod`, `WeatherPoint`), selects the active
period for a date with **year-wrap** handling
(`RegionWeather::period_for_date`), and rasterizes the floor into a
quarter-res texture keyed by `provinces.bmp` + each region's province list
(`synthesize_snow_floor_texture` / `synthesize_snow_floor_from_resolver`).
Fail-closed: a region with no period for the date contributes no floor (counted
in `regions_without_period`).

> The floor is written into **both** the fade-0 and fade-1 snow channels so the
> displayed snow equals the floor for any `SnowMudFade`
> (`SnowFloorTexture::to_snow_mud_payload`). Mud has **no** deterministic floor
> (mud is entirely sim state).

---

## 4. Snow/mud VALUES are accumulated simulation state (NOT files+date)

`common/weather.txt` proves the snow channel is accumulated sim state. Its
`snowing` block (`weather.txt:90`):

```
snow_gain_on_snowing  = 1.0
snow_gain_on_blizzard = 5.0
snow_gain_max         = 80.0
snow_visual_min       = 128   # "Visual fix, so even tiny amount of snow can be visible. (0-255)"
snow_melt_temperature = { 0.0 12.0 }
snow_melt_multiplier  = { 0.001 0.9 }
```

and `duration = { snow = { 5 20 } }` (random-length snow spells, weighted low).
Game bookmarks call `randomize_weather` with a seed (1936 = `12345`, 1939 =
`22345`). So the map snow at
any instant is the running **accumulate/melt integral of pseudo-random daily
rolls** — it needs the runtime/save weather state, RNG history, elapsed ticks,
and fade phase. `snow_visual_min = 128` explains why captured snow bytes
cluster at 128. This is a stateful seeded simulation, so an exact `SnowMudData`
is **not a closed function of (shipped files, date)**.

Evidence it is simulation state, not a files-only function of the date:

- Three captures → three distinct `SnowMudData` content hashes:
  `restore_gate_1` = `cd2693a5d522646d`, `live_PAN2` = `a94188fa5baafe80`,
  archived harness capture (`head_quick_20260702_211446`) = `17b9454249922325`.
- The archived harness capture is **entirely zero** — a legal game frame no
  deterministic floor can reproduce.
- `min_snow_level` maxes at `0.3`, but captured snow (`.g`) reaches `255` (1.0)
  only in the far-NORTH rows (497–511 = Arctic per `CalcGlobeNormal`). The
  captured snow is an Arctic permanent-ice band; the broader `min_snow_level`
  land floors (all of Siberia in January) show **zero** snow — i.e. the full
  floor is not even visibly encoded in the analysed captures (they are warm /
  early-game, only the small permanent Arctic band is present).

---

## 5. Validation numbers

Tool: `dump_snow_validation --game <dir> --date MM.DD --capture <session>…`.
Determinism verified: identical output on repeated runs (same date → identical
floor: `07.01` → 2223 floored texels both runs; `01.15` → 68198 both runs).

Captured `SnowMudData` channel stats (all `1408×512`, `SnowMudFade=0`):

| capture | hash | snow_fade0 (.b) | snow_fade1 (.g) | mud_fade0 (.r) | mud_fade1 (.a) |
|---|---|---|---|---|---|
| `restore_gate_1` | `cd2693a5…` | 0.00%/0 | 0.61%/255 | 0.00%/0 | 0.81%/255 |
| `live_PAN2` | `a94188fa…` | 0.00%/0 | 0.56%/255 | 0.00%/0 | 1.07%/255 |
| harness (`17b94542…`) | `17b94542…` | 0.00%/0 | 0.00%/0 | 0.00%/0 | 0.00%/0 |

(`nonzero% / max byte`.) The fade-0 channels (`.b`/`.r`, what is *displayed* at
`SnowMudFade=0`) are **zero everywhere** in all three; any snow/mud sits in the
fade-1 target (`.g`/`.a`) and is confined to the Arctic band (rows 497–511).

Deterministic-floor residual vs captures (correct orientation, Y-flip = true):

| date | floored texels | capture | floor respected | violations | max shortfall | exact displayed-snow byte match | mean abs snow diff |
|---|---|---|---|---|---|---|---|
| 01.15 | 68198 (9.46%) | `restore_gate_1` | 183 (0.27%) | 68015 | 0.300 | 90.54% | 0.0197 |
| 01.15 | 68198 (9.46%) | `live_PAN2` | 227 (0.33%) | 67971 | 0.300 | 90.54% | 0.0197 |
| 01.15 | 68198 (9.46%) | harness zero | 0 (0.00%) | 68198 | 0.300 | 90.54% | 0.0197 |

**Interpretation (honest).** With the correct orientation the January land
floor (Siberia, north rows ~270–511) DOES overlap the captured Arctic snow, so
`183`/`227` texels respect the floor. The remaining ~68k "violations" are
**expected and correct**: these captures are NOT from January — they are warm /
early-game, carrying only the small permanent Arctic ice band, so the full
January Siberian floor has no matching accumulated snow. This is not a
refutation of the *floor law* — it is positive evidence that the texture is
accumulated sim state at a different date. The `~90.5%` exact byte match is a
**mostly-zero artifact** (both floor and capture are ~99% zero); it is not a
meaningful "match" and the tool says so. Better sparse-map metrics to prefer:
under-floor violation rate, nonzero-mask precision/recall/IoU, residuals only
on floor-positive texels, per-channel nonzero%, and content hashes.

### Inferred capture dates

No date field is stored in the capture metadata, so the date is inferred:

- **`SnowMudFade = 0`** in both `restore_gate_1` and `live_PAN2` (the fade phase
  is mid-cross-fade indeterminate, not itself a date).
- The **tree-season CB1** (`live_PAN2/trees_frame_draw000_ps_constant_buffer_slot1.bin`,
  offsets 64/68) reads `vSeasonLerp = 0, vSeasonColumn = 0`. Column 0 is the
  `tree_winter` block, BUT the local capture census (per `seasons.rs` docs)
  found ALL valid 80-byte tree captures reading `(0, 0)`, so this is a
  **degenerate/default** value and a weak proxy, not a reliable date.
- The **SnowMudTexture content** is the strongest signal: snow appears only in
  the far-NORTH rows (497–511 = Arctic permanent ice per `CalcGlobeNormal`)
  with essentially no broader Northern- or Southern-Hemisphere seasonal
  accumulation. That is consistent with a **warm / uninitialized / not-a-
  validated-winter** state — only the small permanent Arctic band is present,
  no accumulated land snow.

**Inference (bounded):** `restore_gate_1` and `live_PAN2` are NOT in a
validated NH-winter snow state (no accumulated land snow beyond the permanent
Arctic band). The exact calendar day is **not recoverable** from the
shipped-file laws alone — a precise date needs a reliable game-date capture
field (the extractor item in §6); "warm / early-game" is a plausible but
unproven reading. The archived harness capture is a fully zero SnowMud frame.

---

## 6. Extractor item (proposed `REPLAY_DATA_REQUIREMENTS` wording)

`REPLAY_DATA_REQUIREMENTS.md` §6 already asks for per-region weather. SNOW1
sharpens what the **map snow/mud visual** specifically needs. Proposed refined
wording to fold into §6 (orchestrator to apply — this lane does not edit
`REPLAY_DATA_REQUIREMENTS.md`):

> **§6 Weather / ground snow-mud state (map visual).** The `SnowMudData`
> texture is accumulated, seeded weather-sim state, not a function of the date
> (see `SNOW_LAW.md`). Shipped files give only the addressing/packing law and
> per-region `min_snow_level` floors. To reproduce the map snow/mud 1:1, the
> replay needs, per hour (or on-change), the **per-province (or per-strategic-
> region) accumulated ground snow and mud amounts** the sim tracks — ideally
> the same `snow_gain`/`mud` accumulators (`0..snow_gain_max=80`, with the
> `snow_visual_min=128` visual floor) plus the current `SnowMudFade` phase and
> its fade-0/fade-1 endpoints. Minimum viable feed:
> `{region_or_province_id, snow_level_0_255, mud_level_0_255}` per hour; the
> replay rasterizes it into the quarter-res `SnowMudData` via the §1 addressing
> law and blends by `SnowMudFade`.
> **Graceful degrade (files-only, no feed):** synthesize the deterministic
> `min_snow_level` floor for the replay date via
> `snow_law::synthesize_snow_floor_from_resolver` (this lane). This is visibly
> wrong for Eastern-Front winters (no accumulated snow, only the small regional
> floor) but is deterministic, mod-aware, and never crashes.

---

## 7. What is proven vs open

**Proven (files-first, capture-cross-checked):**

- Addressing law (quarter-res, world extent, Y-flip) — §1.
- Channel packing & `SnowMudFade` blend, CB0 offset 248 — §2.
- Per-region `min_snow_level` parse + date-window selection (year-wrap) — §3.
- Snow is accumulated seeded sim state (`weather.txt`) — §4.

**Open / not files-derivable:**

- Exact per-texel snow/mud values (extractor item — §6).
- Whether the C++ writer's fade-0/fade-1 endpoints are literally
  previous-month / current-month keyframes (only the fade endpoints are
  shader-proven).
- The precise calendar date of the `restore_gate_1` / `live_PAN2` captures
  (inferred warm/early-game; not recoverable from shipped-file laws alone).

---

## Gate outputs

- `cargo fmt -p map_viewer` — clean.
- `cargo test -p map_viewer` — 368 passed / 0 failed (20 in `snow_law`,
  incl. weather-period date matching + year-wrap, region→texel + Y-flip
  orientation, channel decode/fade, fail-closed parse & synthesis).
- `dump_snow_validation` stats on `restore_gate_1`, `live_PAN2`, and the
  archived harness capture — see §5; determinism verified 2×.
- Scope: only `map_viewer/src/snow_law.rs`, `src/bin/dump_snow_validation.rs`
  (new), `src/lib.rs` (one-line exports), and this document. No forbidden files
  touched.
