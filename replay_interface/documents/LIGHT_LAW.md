# LIGHT_LAW — the map light rig (LIGHT1)

Mechanism-correct laws for the game's map scene lighting: the virtual sun/moon
positions, the day-night terminator, the ambient rig, the diffuse/specular
colors, and the ShadowMap. Derived **files-first** from the shipped installed content
(via the mod-aware `GameFileResolver`) and validated against captured scene
constant-buffer 0 (CB0) evidence. This is an analysis/law job — no render-path
wiring; the module is a pure, unit-tested law (`map_viewer/src/light_law.rs`)
plus a CB0-mining bin (`map_viewer/src/bin/dump_light_rig.rs`).

Sibling of `WATER_LAW.md`, which owns the water sun/moon **packing** (`.xwz`
swizzle) law. This doc owns the **rig** (positions, day/night, colors, shadow).

Shipped sources of truth (game root
`<local-path> of Iron IV\`):
`gfx/FX/standardfuncsgfx.fxh` (CB0 declaration + day/night + sun-dir),
`gfx/FX/constants.fxh` (ambient maps, NIGHT_AMBIENT_BOOST, SHADOW_WEIGHT_*),
`gfx/FX/shadow.fxh` (CalculateShadow/GetShadowScaled),
`gfx/FX/fow.fxh` (ApplyFOW), `gfx/FX/pdxwater.shader`, `gfx/FX/pdxmap.shader`,
`common/defines/00_graphics.lua`, `map/heightmap.bmp` (5632×2048).

Every claim below was cross-checked by an adversarial Codex pass (session
2026-07-06); its two substantive findings — a competing moon-`.z` identity and
scoping the ShadowMap verdict to the captured binding — are folded in.

---

## 0. Evidence base — the mined corpus

`dump_light_rig` walks capture session dirs, finds every
`*_ps_constant_buffer_slot0.bin` that decodes as a 416-byte **scene CB0** (its
`vVirtualSunPos.y == SUN_HEIGHT == 600` and `.w == SUN_HEIGHT_WATER == 5000`
fingerprint the scene CB0, excluding post-effect/UI slot-0 CBs), and tabulates
the rig fields per bundle.

- **1212 scene CB0s** from **59 sessions** (of 86 capture dirs; 27 carry no map
  bundle — e.g. `session_LIVE_A2`/`A4` are shader-only dumps).
- Bundle kinds covered: terrain (`map_frame_draw*`), water (`water_frame_*`),
  rivers (`rivers_frame_*`), trees (`trees_frame_*`), terrain chunks.
- The bin is **deterministic** (sorted session/file traversal; output is a pure
  function of inputs — two runs byte-identical) and writes JSON+markdown to a
  scratch `--out` dir. **Payloads are never committed.**

### CB0 lighting-field layout (standardfuncsgfx.fxh:2-33, HLSL packing)

Total scene CB0 = **416 bytes** (26 float4 registers), matching every capture.
Offsets pinned by HLSL cbuffer packing; **no offset error found** in the audit.

| offset | field | notes |
|---|---|---|
| 144 | `vVirtualSunPos` | (x, y=600, z=848, w=5000) |
| 160 | `vVirtualMoonPos` | (x=0, y=600, z=1448, w=550) |
| 176 | `vSecondVirtualSunPos` | (0, 600, 100, 5000) seam-hiding fake |
| 192 | `vSecondVirtualMoonPos` | (2816, 600, 100, 550) seam-hiding fake |
| 208 | `vCamPos` (f3) | + 220 `HdrRange` share reg 13 |
| 224 | `vCamLookAtDir` (f3) | + 236 `vGlobalTime` share reg 14 |
| 240 | `vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed` | `.w` gates day/night |
| 256 | `DayNight_Hour_SunDir` | the entire day-night driver |
| 272 | `AmbientPosX` (f3) | + 284 `ShadowFadeFactor` |
| 288 | `AmbientNegX` (f3) | + 300 `FOWFadeFactor` |
| 304 | `AmbientPosY` (f3) | + 316 `MinMeshAlpha` |
| 320 | `AmbientNegY` (f3) | + 332 `NegFogMultiplier` |
| 336 | `AmbientPosZ` (f3) | reg 21 .xyz |
| 352 | `AmbientNegZ` (f3) | reg 22 .xyz (fresh reg: f3 cannot use prev .w) |
| 364 | `CubemapIntensity` | reg 22 .w |
| 368 | `SunDiffuseIntensity` | rgb color, a = intensity |
| 384 | `MoonDiffuseIntensity` | rgb color, a = intensity |
| 400 | `GB_TextureHeight` | reg 25 .x |
| 404 | `SunSpecularIntensity` | reg 25 .y |

---

## 1. The rig-law table — field → source / law / unknown

Residuals are `captured − law`, max over the 1212 mined samples.

| CB0 field | value(s) observed | source / law | residual |
|---|---|---|---|
| `vVirtualSunPos.y` | 600 (const) | `SUN_HEIGHT` (00_graphics.lua:1044) | 0 |
| `vVirtualSunPos.z` | 848 (const) | `SUN_LATITUDE` (line 1048) | 0 |
| `vVirtualSunPos.w` | 5000 (const) | `SUN_HEIGHT_WATER` (line 1046) | 0 |
| `vVirtualSunPos.x` | **2816 (const)** | **engine runtime** (not in any file) | — |
| `vVirtualMoonPos.y` | 600 | `MOON_HEIGHT` (line 1043) | 0 |
| `vVirtualMoonPos.w` | 550 | `MOON_HEIGHT_WATER` (line 1045) | 0 |
| `vVirtualMoonPos.z` | **1448 (const)** | **UNKNOWN** — two competing identities (§3) | — |
| `vVirtualMoonPos.x` | 0 | engine runtime | — |
| `vSecondVirtualSunPos` | (0,600,100,5000) | `.z=SECOND_SUN_LATITUDE=100` (line 1050); rest as sun | 0 |
| `vSecondVirtualMoonPos` | (2816,600,100,550) | `.z=SECOND_MOON_LATITUDE=100` (line 1049); `.x` runtime | 0 |
| `DayNight_Hour_SunDir.x` | swept 0..23/24 (NIGHT1) | hour/24 (§4) — engine runtime (time-of-day) | — |
| `DayNight_Hour_SunDir.yzw` | `(0, z_date, ~1)` unit vec | sun direction — **HOUR-INVARIANT** (§4b, NIGHT1); `.z=dir.y` drifts with DATE only | `|yzw|−1` < 2e-7; `.y≡0`; `1−.w` < 3e-3 |
| `vFoW…MaxGameSpeed.w` | **0 (const)** | engine runtime; gates day/night → **full daylight** (§5) | — |
| `SunDiffuseIntensity.rgb` | (1,1,1) | `ToLinear(HSVtoRGBPost(SUN_DIFFUSE_COLOR))` (§6) | 0 |
| `SunDiffuseIntensity.a` | 1.0 | `SUN_INTENSITY` (line 1061) | 0 |
| `MoonDiffuseIntensity.rgb` | (0.2176,0.5468,1.0) | `ToLinear(HSVtoRGBPost(MOON_DIFFUSE_COLOR))` (§6) | < 1e-4 |
| `MoonDiffuseIntensity.a` | 2.5 | `MOON_INTENSITY` (line 1064) | 0 |
| `SunSpecularIntensity` | 1.0 | `SUN_SPECULAR_INTENSITY` (line 1062) | 0 |
| `CubemapIntensity` | 1.0 | `CUBEMAP_INTENSITY` (line 1066) | 0 |
| `Ambient{Pos,Neg}{X,Y,Z}` | per-frame | **engine-composed** final ambient (NOT the raw `DayAmbientMap*` defines; already day/night-blended + boosted) | — |
| `vCamPos`, `vCamLookAtDir`, `vGlobalTime`, `HdrRange`, `Shadow/FOWFadeFactor`, `MinMeshAlpha`, `NegFogMultiplier`, `GB_TextureHeight` | vary | engine runtime per-frame state (not lighting-law) | — |

**Every files-derivable field reproduces its capture with residual 0** (colors
< 1e-4 through the gamma step). All variation lives in engine-runtime fields.

---

## 2. Camera-follow — REFUTED

The spec's open question and prior project memory: does `vVirtualSunPos.x`
follow the camera (a "camera-follow light rig")? **No.** Across the 1212 mined
CB0s:

- `vVirtualSunPos.x` is a **fixed 2816** in every sample.
- `vCamPos.x` **varies** (2958 in the bulk; also 2714–2833 across pans/zoom).

Since the sun x is constant while the camera x moves, the sun does **not** track
the camera. The prior "sun.x ≈ camera.x" reading was a coincidence: in one early
capture a camera happened to sit near x≈2816. The shader's true longitude anchor
is `GMT_OFFSET = 2793` (00_graphics.lua:934, "X position of Greenwich GMT+0"),
consumed by `CalcGlobeNormal` as `(worldXZ.x − GMT_OFFSET)/MAP_SIZE_X`
(standardfuncsgfx.fxh:349) — not the camera. (`2816 − 2793 = 23`, numerically
near `vGlobalTime ≈ 23`, but this is not load-bearing and is treated as
coincidental.)

Encoded in `light_law.rs::camera_follow_hypothesis_is_refuted` (asserts
`|sun.x − cam.x| > 100`, ≈ 142 here).

### 2a. The renderer now drives the rig from the law (LIGHT2, camera-follow removed)

For a period the **product renderer contradicted this law**: it translated all
four virtual-rig `.x` fields by `delta_x = camera.x − reference_camera_x`, so the
sun followed the pan camera (FIDELITY_LEDGER debt item #1; the code's own comment
admitted "in-game the rig did NOT track the pan camera"). That FIT is **removed**.
`shader_meta.rs::write_virtual_light_rig_fields` now writes the four `.x` lanes
from the day-cycle law (`light_law::virtual_rig_longitudes_x`, §4a) evaluated at
the render hour — the **same** `day_night_hour` fraction that feeds
`DayNight_Hour_SunDir.x`, so the rig and the terminator share one clock. Only
`.x` is written; `.y/.z/.w` stay at the captured/file constants. The write fires
only when a render hour is set (`day_night_hour = Some`); byte-exact
captured-frame replay / reference / validation renders (`hour = None`) keep the
captured rig verbatim.

**Baseline-noon equivalence (proof of the invariant):** `--hour` is a
fraction-of-day, so `--hour 0.5 → hour24 = 12.0`, and
`virtual_sun_longitude(12, 5632) = 5632·(1 − 0.5) = 2816.0` **exactly in f32**
(bits `0x45300000`); `moon.x = (2816 + 2816) mod 5632 = 0.0` (`+0.0`). So at noon
the law writes `[2816, 0, 0, 2816]` — bit-for-bit the values every paused/noon
capture carries. The law is therefore a byte-exact drop-in for the captured rig
at the noon hour the paused corpus sits on.

**Caveat on the locked byte-identity oracle
(`hoi4_captures/w1check/baseline_2816.bmp`):** that BMP was rendered by the OLD
camera-follow code at dump-center 2816 where the captured base camera x was
`2770.12`, i.e. `delta_x = +45.88` — so the baseline embeds `sun.x ≈ 2861.9`, the
**camera-follow bug**, NOT the law's noon value. The corrected law writes
`sun.x = 2816.0`, so the new noon dump differs from that baseline by a spread of
±1-per-pixel lighting deltas (a ~46-unit sun-x shift at whole-world zoom). This is
expected and correct: the baseline is an oracle for the refuted FIT, not for the
law. It is read-only evidence and was not regenerated. (Root cause confirmed
empirically by instrumenting `delta_x`, and by an adversarial Codex pass,
2026-07-06.)

---

## 3. Moon `.z` = 1448 — UNKNOWN, two competing file-grounded identities

`vVirtualMoonPos.z` is a constant **1448** in every capture and is **not** any
single shipped define — `MOON_LATITUDE = 0` is commented "NOT USED"
(00_graphics.lua:1047). Two arithmetic identities from shipped values both hit
1448 exactly, and the corpus (one unvarying value) **cannot choose between
them**:

1. `SUN_LATITUDE + MOON_HEIGHT = 848 + 600 = 1448`
2. `MAP_SIZE_Y − MOON_HEIGHT = 2048 − 600 = 1448` (map height 2048, from the
   shipped `map/heightmap.bmp` = 5632×2048)

Both are **hypotheses**, not proven source assignments. The Rust
`defines::MOON_Z_HYPOTHESIS` carries identity (1) only so a residual can be
reported; it is numerically equal to identity (2). See §8 for the capture that
would disambiguate.

---

## 4. Day/night — a per-pixel terminator from ONE uniform, no texture

Night is applied **per-pixel**, driven **entirely** by the single CB0 uniform
`DayNight_Hour_SunDir` (offset 256). **There is no terminator/day-night
texture.** (Verified: no texture appears anywhere in the day/night path.)

- `CalcGlobeNormal(worldXZ)` (standardfuncsgfx.fxh:347) maps a pixel's world XZ
  onto a virtual globe, adding `DayNight_Hour_SunDir.x` as an **east/west
  longitude phase**: `fmod_loop((worldXZ.x − GMT_OFFSET)/MAP_SIZE_X +
  DayNight_Hour_SunDir.x, 1)`. So `.x` is the **hour-of-day fraction** that
  rotates the terminator. Observed `.x ∈ {0.0417 = 1/24, 0.6667 = 16/24}` →
  in-game **hours 1 and 16** only.
- `DayNightFactor(globeNormal)` (fxh:358) =
  `saturate((dot(globeNormal, DayNight_Hour_SunDir.yzw) − FEATHER_MIN) /
  (FEATHER_MAX − FEATHER_MIN)) * vFoW…MaxGameSpeed.w`, with
  `FEATHER_MIN/MAX = ∓0.01` (fxh:324-325). So `.yzw` is the **sun-direction unit
  vector** (measured `|yzw| = 1.0000`); its dot with the per-pixel globe normal
  is the terminator.
- `DayNightWithBlend`/`NightifyColor` (fxh:370-399) produce the night look:
  `NightifyColor` desaturates and recolors, then multiplies by
  `NIGHT_DARKNESS = 0.5`; the day→night blend is `lerp(dayColor,
  NightifyColor(dayColor), DayNightFactor * NIGHT_OPACITY)` with
  `NIGHT_OPACITY = 0.85`. (Night is **not** a plain `×0.5` — it recolors
  first; a faithful port must run the full `NightifyColor`.)
- The sun/moon **light** selection (`CalculateSunLight`, fxh:642) uses
  `vDayFactor = 1 − DayNightFactor(...)` and `vNightFactor = DayNightFactor(...,
  MOON_FEATHER)` to cross-fade `SunDiffuseIntensity` and `MoonDiffuseIntensity`.

### How the replay `--hour` daynight factor maps onto this

The engine mechanism is: `--hour h` sets `DayNight_Hour_SunDir.x = h/24` (the
terminator longitude phase); the sun-direction `.yzw` is **hour-invariant**
(NIGHT1, §4b) so it is left at the captured value. A replay `--hour` control
therefore **writes `.x = h/24`, echoes the captured `.yzw` verbatim**, and —
to actually exercise night — sets `vFoW…MaxGameSpeed.w = 1` to *un-gate* the
blend (see §5), because the mined corpus only ever contains the full-daylight
case (`.w = 0`). Wired in `shader_meta::terrain_scene_cb0_bytes_with_app_scene_layout`
(→ `write_day_night_hour_field` for `.x`,
`write_sun_dir_yzw_hour_invariant_field` for the hour-invariant `.yzw` echo, and
`write_day_night_factor_gate_field` for the `.w=1` un-gate).

### 4b. NIGHT1 — the sun-direction `.yzw` is HOUR-INVARIANT (curve CLOSED)

The prior corpus captured only two hours (1 and 16), so `.yzw` looked like "4
discrete unit vectors" and the `.yzw(hour)` curve was documented UNKNOWN (§8 #1
below, now superseded). Tonight's **UNPAUSED hour-swept** capture
(`session_NIGHT_SWEEP_6` + `session_CAPFIX2_FULL_2` + `session_CAPFIX2_GAPS_4`,
**468 decoded samples spanning in-game hours 0..23**) resolves it: **`.yzw`
does not rotate with the hour.** Mined residuals (`bin/mine_cb0_sweep`):

| quantity | value | reading |
|---|---|---|
| `max\|.y\|` (dir.x) | **0.000000** (exact) | dir.x is identically 0 at every hour |
| `max\|1−.w\|` (dir.z) | 0.00300 | dir.z ≈ 1 at every hour |
| `max\|\|yzw\|−1\|` | 1.2e-7 | a unit vector throughout |
| CLEAN control `.z` spread | **0.000001** | session_CAPFIX2_FULL_2, hours **1→6** at ONE date: `.z` flat to 1e-6 as the hour sweeps a quarter-day |
| total `.z` spread across dates | 0.112 | the ONLY movement in `.yzw`; tracks the advancing in-game DATE |
| REFUTED rotation recon | 1.99 | a single-day `.y=r·cosθ,.w=r·sinθ` model is inadmissible (`.y≡0`, `.w≈1` const) |

The decisive control is `session_CAPFIX2_FULL_2`: the game hour advances
MONOTONICALLY 1→2→3→4→5→6 (unpaused) while `.yzw` is **bit-for-bit identical**
(only `DayNight_Hour_SunDir.x` changes). In `session_NIGHT_SWEEP_6` the hour
bounces wildly while `.z` instead tracks **wall-clock / simulation time** and
FREEZES the instant the game pauses — i.e. `.z` is a date/season term, decoupled
from the hour. This is exactly what the shader implies: `CalcGlobeNormal`
(fxh:349) folds the hour into the terminator via `+ DayNight_Hour_SunDir.x`;
`DayNightFactor` (fxh:360) dots the globe normal against a per-date sun vector.

**Consequence (files-first, byte-safe):** the render writes `.x = hour/24` and
**leaves `.yzw` at the captured value** (the hour-invariant sun direction for
the recording's own date). Encoded in `light_law::render_sun_dir_yzw` (+
`night_yzw_residual`) and unit-tested on the real swept bytes
(`night_sweep_yzw_is_bit_identical_across_hours_at_one_date`,
`render_sun_dir_yzw_echoes_captured_vector_independent_of_hour`) and through the
full write path (`day_night_hour_patch_leaves_sun_dir_yzw_bit_identical_at_every_hour`).
Because the `.yzw` write is a byte-echo of the captured vector, it is a no-op on
the bytes and cannot perturb the `--hour 0.5` byte-identity gate.

**Still capture-gated:** the *specific driver* of the `.z` date-drift (is it
precisely a seasonal declination? the exact date→`.z` map?) is not pinned — what
is proven is that `.z` moves with elapsed sim-days and is hour-independent. And
the **FoW.w≠0 night blend** itself is STILL never captured (FoW.w = 0 in all 468
samples), so the night *darkening amount* remains validated only by the shipped
shader port, not by a nonzero-gate capture (§5, §8 #1).

### 4a. The virtual-rig longitude law `x(hour)` — PROVEN and now wired

The virtual **sun/moon `.x` longitudes** (distinct from the terminator phase
`DayNight_Hour_SunDir.x` above) were closed as a law from **session_FINAL_1's
unpaused CB0 samples** (5 samples across game hours 0.00..15.38, residual < 0.01
map units on every moving sample). With `hour24 = DayNight_Hour_SunDir.x · 24`:

```
sun.x         = MAP_SIZE_X · (1 − hour24/24)   (mod MAP_SIZE_X)   [east→west]
moon.x        = sun.x + MAP_SIZE_X/2           (mod MAP_SIZE_X)   [antipodal]
second_sun.x  = moon.x   (seam-hiding fake bodies are the SWAPPED pair)
second_moon.x = sun.x
```

`MAP_SIZE_X = 5632` (shipped `MAP_SIZE_X` shader define / `heightmap.bmp` width).
Encoded in `light_law::virtual_sun_longitude` / `virtual_moon_longitude` /
`virtual_rig_longitudes_x`, unit-tested against the session_FINAL_1 tuples and
against the noon byte-exact invariant. **This law is now the renderer's rig
mechanism** (§2a); it supersedes the earlier "`x(hour)` UNKNOWN pending capture"
status — that gap is **closed** by session_FINAL_1 for the daylight-side hours it
sampled. (The continuous **`.yzw(h)` sun-direction** curve and the **night** side
remain UNKNOWN per §5/§8 #1 — they are a *different* dimension than the rig `.x`
longitude closed here.)

---

## 5. The whole corpus is full-daylight — a capture limitation

`vFoWOpacity_FoWTime_SnowMudFade_MaxGameSpeed.w` (the `.w` = `MaxGameSpeed`
component) is **0 in all 1212 daylight captures AND all 468 NIGHT1 hour-swept
samples** (1680 total). Because `DayNightFactor` multiplies its result by this
`.w` (fxh:361), **every captured frame renders full daylight** —
`DayNightWithBlend` returns the day color, sun light uses day-factor 1 / moon
0, and ambient uses the day ambient. So the mined evidence **cannot validate the
night *darkening amount*** (the `NightifyColor` blend behind a nonzero gate) —
that path is validated only by the shipped-shader CPU port, not by capture.
The all-`.w==0` fact is capture evidence, not a file law.

**What NIGHT1 DID close (despite `.w=0`):** the `DayNight_Hour_SunDir.x`
hour-sweep and the `.yzw` **hour-invariance** are independent of the FoW.w gate —
they are the *inputs* to `DayNightFactor`, read straight from CB0 regardless of
whether the gate multiplies the output to 0. So the swept corpus fully closes the
`.yzw(hour)` curve (§4b) even though the night *look* stays gate-blocked.

Encoded in `light_law.rs::corpus_is_full_daylight_only`.

---

## 6. Diffuse-color law — HSV define → linear RGB (exact)

A shipped `*_DIFFUSE_COLOR` HSV define (H on the 0..1 dial) becomes the CB0
`*DiffuseIntensity.rgb` via **`ToLinear(HSVtoRGBPost(H*6, S, V))`**:

- `HSVtoRGBPost` (standardfuncsgfx.fxh:102) does HSV→RGB with H on the 0..6
  sextant dial (`HuePost`), clamping RGB to [0,1].
- `ToLinear` (fxh:48) is `pow(rgb, 2.2)` (alpha preserved on float4).
- The 0..1→0..6 scaling is confirmed by the sibling `HSVtoRGB` 0..1 hue path in
  the same file.

Verified exactly:

| light | HSV define | HSVtoRGBPost | `^2.2` (linear) | captured `.rgb` |
|---|---|---|---|---|
| moon | {0.58, 0.5, 1.0} | (0.5, 0.76, 1.0) | **(0.21764, 0.54675, 1.0)** | (0.21764, 0.54675, 1.0) |
| sun | {0.14, 0.0, 1.0} | (1,1,1) (S=0→vvv) | **(1, 1, 1)** | (1, 1, 1) |

`.a` = the matching `*_INTENSITY` (`SUN_INTENSITY = 1.0`,
`MOON_INTENSITY = 2.5`); `SunSpecularIntensity = SUN_SPECULAR_INTENSITY = 1.0`;
`CubemapIntensity = CUBEMAP_INTENSITY = 1.0`. Encoded in
`light_law.rs::diffuse_color_rgb_from_hsv_define` and
`diffuse_color_hsv_to_linear_law_matches_captured_rgb`.

---

## 7. ShadowMap — a screen-space shadow+FoW RGBA target (for the captured path)

**What water SRV slot 15 "ShadowMap" actually contains, measured** (live_PAN2,
restore_gate_1 water bundles, active draw):

| property | captured `ShadowMap` |
|---|---|
| slot / name | 15 / `ShadowMap` (reflection: `ShadowMap → 15`) |
| dims | **1920×1080** (= screen resolution, = `WaterRefraction` slot 5) |
| format | DXGI **87** = `B8G8R8A8_UNORM` |
| mips | 1 |

`pdxwater.shader` declares it `Type = "Shadow"` (line 150) and the water pixel
shader consumes it **in screen space** at `Input.vScreenCoord`:

- `GetShadowScaled(SHADOW_WEIGHT_WATER, vScreenCoord, ShadowMap)` (line 386)
  reads the shadow term from `.r` (shadow.fxh:44).
- `ApplyFOW(vOut, ShadowMap, vScreenCoord)` (line 398) samples the **same**
  texture and reads fog from `.y` (fog alpha) and `.z` (fog color) via
  `tex2Dproj` (fow.fxh:85).

So the slot-15 texture is a **combined screen-space shadow+FoW render target**
(shadow in R, FoW in G/B), produced by an engine pre-pass at screen resolution —
**not** a light-space depth shadow map, despite the `Type="Shadow"` declaration
and the `0.5/2048` PCF texel constant in `shadow.fxh`.

**Scope caveat (Codex):** this verdict is proven for the **captured water/main
binding**. The engine still ships a separate *light-space* PCF path —
`CalculateShadow` (shadow.fxh:6) with the hardcoded `0.5/2048` (a 2048² design),
still referenced by a `pdxmap.shader` variant (line 462) via
`vShadowProj = mul(ShadowMapTextureMatrix, …)` (pdxmap.shader:209,
`ShadowMapTextureMatrix` in pdxmap.fxh:4). We have **not** captured a binding
that resolves to a 2048² depth texture; in the terrain (`pdxmap`) bundles the
`ShadowMap` SRV (slot 12) is **descriptor-only** (0×0, unresolved runtime RT).
So: do not claim *every* `ShadowMap` use is screen-space — only the captured
water/main path is proven so.

---

## 8. What a future capture must vary (the capture spec)

The rig positions and colors are fully pinned. The **day/night dimension is
not** — every capture is full daylight (`FoW.w = 0`, §5) at just 2 discrete
hours. Sessions are cheap; a targeted capture would close these gaps:

1. **~~Fit the `.yzw(hour)` curve.~~ — CLOSED (NIGHT1, §4b).** The hour-swept
   corpus (468 samples, hours 0..23) proves `.yzw` is **HOUR-INVARIANT**: dir.x≡0,
   dir.z≈1, and the only mover (dir.y) tracks the DATE, not the hour (clean
   single-date control `.z` spread = 1e-6 across hours 1→6). No continuous
   `.yzw(hour)` curve exists to fit; the render echoes the captured `.yzw`.
   **STILL OPEN — the night *look* behind a nonzero gate.** Every capture
   (1680 total) has `vFoW…MaxGameSpeed.w = 0`, so the night *darkening amount*
   (`NightifyColor`, moon-light cross-fade, `NIGHT_AMBIENT_BOOST`) is validated
   only by the shipped-shader CPU port, never by a nonzero-gate capture. A
   capture with `.w ≠ 0` at a few night hours (0, 3, 21) would validate the night
   path pixels for the first time. It would ALSO pin the `.z` date-drift driver:
   capture the SAME hour on several distinct in-game DATES (or a fixed date
   across hours) to map `.z(date)` and confirm the seasonal-declination reading.
2. **Disambiguate moon `.z` (§3).** The two identities differ only if a term
   changes. Capture on a **mod that retunes `MOON_HEIGHT` or `SUN_LATITUDE`**
   (or a map with a different `MAP_SIZE_Y`): if `moon.z` tracks
   `SUN_LATITUDE + MOON_HEIGHT` it confirms identity (1); if it tracks
   `MAP_SIZE_Y − MOON_HEIGHT` it confirms identity (2).
3. **~~Pin `vVirtualSunPos.x` / `vVirtualMoonPos.x` (longitude).~~ — CLOSED.**
   This was UNKNOWN when only the paused corpus (constant 2816 / 0) existed. It is
   now **PROVEN** from session_FINAL_1's unpaused CB0 samples: the virtual
   sun/moon **x** DO move with time-of-day, per `x(hour) = MAP_SIZE_X·(1 −
   hour24/24)` (moon antipodal, second bodies swapped). See §4a; encoded and
   unit-tested in `light_law.rs` and wired as the renderer's rig mechanism (§2a).
   A future multi-hour night capture (#1) would additionally cross-check the
   `.x(hour)` curve on the night side, but the daylight-side longitude law is
   settled.

The `.yzw(hour)` sun-direction curve is now **CLOSED** as hour-invariant
(NIGHT1, §4b) — the module echoes the captured `.yzw` and does not synthesize an
hour curve. What remains **capture-gated** is the **night *look*** (the pixels
behind a nonzero `FoW.w` gate) and the exact **date→`.z` driver**; the module
deliberately does not over-fit either from the always-`FoW.w=0` corpus. (The rig
**`x(hour)` longitude** — #3 — and the **`.yzw(hour)` curve** — #1 — are both out
of the UNKNOWN set now; only the nonzero-gate night look and the `.z` date map
remain.)

---

## 9. Deliverables

- `map_viewer/src/light_law.rs` — pure, dependency-free law module
  (`SceneLighting` CB0 decoder, `defines`, `offsets`,
  `diffuse_color_rgb_from_hsv_define`, `validate_lighting_law_against_cb0`),
  8 unit tests against the mined CB0 values (all green).
- `map_viewer/src/bin/dump_light_rig.rs` — the deterministic CB0-mining bin.
- `map_viewer/src/lib.rs` — one-line module + export wiring only.

No render-path wiring (that is a later job). No forbidden files touched.
