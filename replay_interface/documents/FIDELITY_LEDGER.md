# FIDELITY EVIDENCE LEDGER — Battle Plan rendering stack

Standing directive this ledger serves (verbatim core): *"absolutely every single
part of the map and the replay [must be] engine-derived or game-file-derived. No
bullshitting, no making it up… If an agent couldn't derive it, [there must be] a
plan for how they're going to crack it."*

Classification key:

- **FILE** — game-file-derived, with a file:line / asset citation. The runtime
  value is computed from shipped files through the mod-aware `GameFileResolver`.
- **CAPTURE** — engine-capture-derived: the runtime value is a payload/constant
  read from a capture bundle (modset-bound), not composable from files.
- **FIT** — a reconstruction/approximation **not yet evidence-derived**. These are
  the debt items. Includes anything the code labels `DERIVED-BY-FIT`, `GUESS`,
  `approximation`, or "not reconstructable".
- **UNKNOWN** — identified gap, nothing implemented (rendered blank / deferred).

**Honesty rule applied throughout:** an item wrongly classed FILE/CAPTURE is worse
than an extra FIT. Where the law doc's *adjectives* claim files-first but the
*runtime default* or the *validation* is capture-anchored (or vacuous), the row is
downgraded and flagged. Two structural facts drive most downgrades:

1. **Flag-gating (partially closed — GRAD1).** Two of the strongest files-first
   paths are now **DEFAULT ON**: grading (`--postfx-restore-full`) and water
   reflection/refraction (`--water-full`) render by default, with explicit
   opt-outs (`--no-postfx-restore-full`, `--no-water-full`) that restore the old
   capture-anchored path byte-identically, and a fail-open downgrade so a
   recording without grade/resolver evidence still renders (an explicit flag
   still fails closed). Borders (`--borders-from-files`) and labels
   (`--map-labels-fallback`) REMAIN default-OFF (corner/placement laws pending
   the next capture; user rejected their current look). The FILE classification
   below for grading/water now describes the **default** path; for
   borders/labels it is still scoped "when the flag is on", and those two
   default-path rows stay capture-anchored (no borders / no labels).
2. **Camera freedom vs single-camera capture.** Every capture sits at essentially
   one camera (x≈2816, height≈2958). The replay camera pans/zooms freely. Several
   runtime behaviors (virtual-sun x, ambient, exposure) are frozen-or-fudged from
   that one camera and are FIT for any other camera.

---

## 1. Summary table — class counts per layer

Counts are per **parameter/behavior rows** enumerated in §2 (not per pixel).

| Layer | FILE | CAPTURE | FIT | UNKNOWN | Law doc |
|---|---:|---:|---:|---:|---|
| Grading (LUT / exposure / tonemap) | 4 | 2 | 2 | 1 | GRADING_LAW.md |
| Bloom chain | 5 | 0 | 1 | 0 | BLOOM_LAW.md |
| Water (reflection / refraction / glitter) | 4 | 3 | 3 | 2 | WATER_LAW.md |
| Light rig (sun/moon/ambient/shadow/daynight) | 7 | 3 | 3 | 4 | LIGHT_LAW.md |
| Borders (thin geometry + render) | 7 | 0 | 5 | 4 | BORDER_LAW.md |
| Map labels (country names) | 3 | 0 | 8 | 5 | MAP_LABEL_LAW.md |
| Snow / mud | 4 | 1 | 1 | 2 | SNOW_LAW.md |
| Political color (fill + secondary) | 5 | 1 | 1 | 3 | POLITICAL_COLOR_LAW.md |
| Terrain (chunks / IDMap / LOD / height) | 3 | 6 | 1 | 4 | — (ledger only) |
| Rivers / straits | 4 | 2 | 2 | 2 | — |
| Trees (T2) | 4 | 2 | 3 | 2 | — |
| Buildings preview (B2) | 2 | 0 | 2 | 3 | — |
| Day / night (runtime patch) | 2 | 1 | 3 | 2 | LIGHT_LAW.md §4-5 |
| Camera / projection / wrap | 3 | 1 | 1 | 0 | — |
| Gradient borders (political GB field) | 4 | 1 | 1 | 1 | — |
| UI composition / chrome | 5 | 0 | 0 | 3 | — |
| Overlay fonts | 3 | 0 | 1 | 1 | — |
| Units / counters / rail / arrows | 1 | 0 | 0 | 6 | — |
| **Totals (LEDGER2 — column sums)** | **70** | **23** | **38** | **45** | |

The FIT + UNKNOWN rows are the burn-down list (§3).

**LEDGER2 burn-down (2026-07-06).** Score line — **before** (git HEAD, per-layer
column sums): FILE 67 / CAPTURE 23 / FIT 39 / UNKNOWN 45 (the previous totals row
printed "43 FIT" but the layer column actually summed to 39 — a pre-existing
arithmetic slip in the header, corrected here). **After LEDGER2**: FILE **70** /
CAPTURE 23 / FIT **38** / UNKNOWN 45. Net movement: **+3 FILE, −1 FIT** (the FILE
gain exceeds the FIT loss because two moves SPLIT a FIT row into a new FILE row +
a narrowed FIT residual rather than deleting the FIT). No row moved to FILE without
a file:line citation LEDGER2 personally opened and verified. Reclassifications
(all cited in the rows): borders fade-window endpoints (FILE, `00_graphics.lua:774-777`);
river palette small/large split (FILE, `00_defines.lua:936-938`); tree fade window
(FILE, `00_graphics.lua:1068-1069`). Sharpened-not-reclassified (kept class,
tightened the cracking plan / citation): night mechanism (code-gated, not
capture-gated), ambient compose (inputs FILE), exposure clamp floor
(`00_graphics.lua:1038`), reflection cube (CONFIRMED still CAPTURE), moon `.z`
(CONFIRMED still ambiguous UNKNOWN), free-camera clamp inputs (FILE). Stale-fix:
the "virtual-sun camera-follow" debt (#1) and misclassification-risk #1 are CLOSED
(LIGHT2 already removed camera-follow from the render path; re-verified). A separate
`documents/RENDER_PATH_AUDIT.md` carries the mod-compat vanilla-literal sweep
(2 findings: `font.rs:38` hard-coded personal path; `constants.rs:5-143` uncited
constants). ~25 existing FILE citations spot-checked (grading/water/light/terrain/
snow/bloom/border/label) — **all accurate, zero stale FILE rows found**. (The
occupation-stripe row 2.8 correctly cites BOTH the `pdxmap.shader:328` callsite AND
the stripe LAW `standardfuncsgfx.fxh:979-987` / `CalculateOccupationMask :965`;
`:328` alone would be under-evidence.)

**Codex adversarial pass (mandated one-shot, read-only, xhigh, 2026-07-06).**
Codex verified LEDGER2's reclassifications against the shipped files and agreed
with #1 (border windows), #3 (HDR clamp bounds), #4 (river indices), #5 (moon `.z`
stays ambiguous UNKNOWN), and #6 (reflection cube stays CAPTURE — it cited the live
upload `d3d11_device.rs:3520-3545` + `:5069` vs the offline resolve
`bundle_gen.rs:36-37`). It caught ONE over-statement, now fixed above: the night
residual list must include `DayNight_Hour_SunDir.x` (hour phase, `fxh:349`) among
the driven inputs — it is file-law (driven by `--hour`), not omitted — so the
runtime residuals are the `.yzw` vector curve + the `FoW.w` gate, with `.x` a known
driver. It also added that the log-luminance chain starts at `downsample.shader:70`
(not only `downsample_luminance.shader`). Both corrections are folded into the rows.

---

## 2. Per-layer parameter rows

Format: **name | class | evidence citation or FIT/UNKNOWN description | cracking
plan + which instrument applies**. Instruments referenced: `restore_pass`
(restore_gate_1 RestoreScene capture), `mapname_pass` (a not-yet-existing capture
that dumps the mapname VB/cb0/cb1), `borders_pass` (clean live border capture —
never obtained clean), `SAMP1 cb0 sampler` (scene-CB0 mining via `dump_light_rig`),
`census` (per-frame census manifest), or a **NEW** instrument to build.

### 2.1 Grading — LUT / exposure / tonemap (`GRADING_LAW.md`, `grading_law.rs`, `render.rs`)

| Parameter | Class | Evidence / FIT description | Cracking plan |
|---|---|---|---|
| Volume selection table (which `posteffect_values`) | FILE | `posteffect_volumes.txt` selection law: height-volume `Base Highest`→`max_distance` for the captured camera y=2958 (`posteffect_volumes.txt:345-353`, `:94-110`) | Settled for the captured camera; per-frame recompose is a FIT (below). |
| ColorCube LUT bytes | FILE | Decode of `colorcorrection_farrange.tga` (uncompressed type-2 24bpp) matches captured ColorCube **byte-exact, residual 0** (`GRADING_LAW.md` LAW 1) | Solid. Loaded via resolver, mod-aware. |
| MiddleGrey | FILE | `posteffect_volumes.txt` `standard` `tonemap_middlegrey=0.55`, matches captured CB (`:17`) | Solid. |
| Tonemap curve (Uncharted2) | FILE | `restorescene.shader:155-164/194-202` ported into `render.rs::uncharted2_tonemap` | Solid. |
| **Which named value the engine used** | CAPTURE-weak / **FIT** | `farrange`/`midrange`/`standard` cubes are **byte-identical** (MD5 `655e09bb…`); the capture *cannot* prove the value NAME, only that height makes `max_distance` plausible (`GRADING_LAW.md` "Honest caveats") | Only a **NEW** capture on a mod that retunes one of those three LUTs disambiguates. Low visual impact (identical pixels), naming only. |
| Exposure / AverageLuminance | **FILE (per-frame, EXPO1)** — clamped branch capture-validated; unclamped branch shader-cited only | The `downsample_luminance.shader` chain is ported (`map_viewer/src/luminance_law.rs`) and computes AvgLum from the rendered scene each frame on `--postfx-restore-full` (`render.rs::update_exposure_from_scene_luminance`). The clamp bounds are file-exact (LEDGER2 verified): `MIN_HDR_ADJUSTMENT=0.5` (`00_graphics.lua:1038`, ACTIVE — the per-volume copies in `posteffect_volumes.txt:107` are commented `#`), `MAX_HDR_ADJUSTMENT=1.0` (`:1039`), `LUMINANCE_VECTOR=(0.2125,0.7154,0.0721)` (`constants.fxh:119`). The per-pixel log-luminance SOURCE is packed in `downsample.shader:70` (`logLuminance = log(dot(rgb,LUMINANCE_VECTOR)+1e-4)`, stored in `.a`), then averaged by `downsample_luminance.shader` and clamped via `MinHdr_MaxHdr` (`downsample_luminance.shader:128`) — Codex-confirmed the chain starts at `downsample.shader:70`, not only `downsample_luminance`. So the 0.5 FLOOR is a shipped define, not an arbitrary constant; the old `MIN_HDR_ADJUSTMENT` hard-code is retired (per-frame path derives the same value). Clamped branch is byte-exact vs the captured restore_gate_1 MainScene (geo-mean 0.0825 -> clamp 0.5 -> R16 0x3800 == captured). Flag-off byte-identical; restore-full wide-view A/B byte-identical (measured clamps to the same 0.5 the constant used) | Unclamped branch: NOT capture-validated and not triggered by any current replay view (linear political scene stays ~0.05-0.16, always clamps). Shader-cited + synthetic-scene unit test only. **NEW** bright close-up `restore_pass` capture (scene geo-mean > 0.5) to validate. Easing tau/delta-time still engine-timed (steady state used). |
| Per-frame recompose (camera/hour/season) | UNKNOWN | Grade composed **once at construction** for a high-altitude day view; live per-frame day/night does not recompose it (`GRADING_LAW.md` "Known scope boundaries"). `fade_distance` cross-fades ignored | Recompose grade per frame on camera-height / hour / winter change; fold `fade_distance` blends between adjacent volumes. |
| **Default-path grade** | **FILE (GRAD1: now default ON)** | `--postfx-restore-full` is **DEFAULT ON** — the default render composes the files-first grade (LUT + MiddleGrey + per-frame exposure + bloom). `--no-postfx-restore-full` restores the old capture-anchored default (equivalence gate B byte-identical). Fail-open: a missing resolver/TGA on the default path logs a downgrade and reverts to the old path; an explicit flag fails closed | Done (GRAD1). Residual = the unclamped-exposure capture (row above), not the default wiring. |

### 2.2 Bloom chain (`BLOOM_LAW.md`, `bloom.rs`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Bright-pass knee + downsample | FILE | `downsample.shader:64-72`; 2×2 box average pinned by `InvDownSampleSize` (`BLOOM_LAW.md` §2) | Solid. |
| `BLOOM_WIDTH`/`BLOOM_SCALE`/`BRIGHT_THRESHOLD`/MiddleGrey | FILE | `posteffect_volumes.txt:14-17` | Solid. |
| Gaussian kernel (Weight0/Weights/Offsets) | FILE | Reconstructed from `BLOOM_WIDTH`/`BLOOM_SCALE`; matches captured `ConstantBuffer(2,39)` to **7 sig figs** (§3.1) | Solid at σ=1.5. |
| Separable 9-tap linear-sampled blur + clamp edge | FILE | `bloom.shader:68/94-101` | Solid. |
| `EMISSIVE_BLOOM_STRENGTH` | FILE | `00_graphics.lua:1037` | Solid. |
| **σ-generality (wide mod sigma)** | **FIT (bounded)** | Verified only at σ=1.5; radius-8 truncation + normalization convention could drift for σ≈3 (§3.1 caveat) | **NEW** capture of a frame with retuned `BLOOM_WIDTH`; low priority (vanilla σ fixed). |

### 2.3 Water (`WATER_LAW.md`, `water_law.rs`, `underwater_pass.rs`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Reflection cube geometry (256²×6×9-mip) | FILE | Shipped `map/terrain/reflection.dds`; all 54 subresources byte-identical to captured cube (`WATER_LAW.md` §1) | Solid pixels. |
| Reflection cube upload payload | **CAPTURE (file-equivalent) — CONFIRMED OPEN by LEDGER2** | `--water-full` (`ShippedFileCube`, `d3d11_replay.rs:50-58`, `d3d11_device.rs:450`) uploads the **captured** cube payload at replay time (upload `d3d11_device.rs:3520-3545`, falling back to the verified frame-file read at `:5069`) — LEDGER2 + Codex confirmed it is NOT a resolver-decode of `reflection.dds`. Only the OFFLINE `bundle_gen.rs:36-37`/`:81-86` path resolver-decodes the DDS. A runtime CONTRACT check (`d3d11_device.rs:3701-3722`) fails closed if the uploaded cube's dimensions/format/`TEXTURECUBE` flag diverge from the `reflection.dds` contract — but that is a SHAPE check, not a pixel-source from mod files, so a mod that retextures `reflection.dds` (same shape) would still render the vanilla cube. Decoding the DDS through the resolver into the replay upload remains the **tracked follow-up** (§1) | Thread resolver into the water-pass prep; decode + byte-verify the DDS at replay time (not just bundle-gen). Pixels proven equal for vanilla; mod-safety needs the runtime decode. |
| Fresnel reflect/refract blend | FILE | `pdxwater.shader:319/334-337` | Solid. |
| Underwater seafloor render (refraction) | FILE | `pdxmap.shader` `Effect underwater` recompiled byte-exact; constants all cited (`WATER_LAW.md` §2a) | Mechanism solid; validation not (below). |
| Underwater constants (WATER_HEIGHT, clip, depth-fog, WATER_COLOR_LIGHTNESS…) | FILE | `constants.fxh:133/57/90`, `pdxmap.shader:424-425` | Solid. |
| Virtual-sun/glitter packing law (`.xwz` swizzle) | FILE | `standardfuncsgfx.fxh:634`; packing law reproduces captured CB0 **residual 0** (§3) | Solid. |
| RT clear `(0,0,0,0)` region | CAPTURE | Measured 150 959 px clear; shader cannot prove the API clear (§2a) | Correct observable reconstruction. |
| Underwater byte-exact validation | **UNKNOWN (blocked)** | `restore_gate_1` seafloor SRVs are descriptor-only/no payload; oracle vs `live_PAN2` is synthesized-geometry; WATER3 lifted exact channels 23.9→26.3%, mean abs err 20.8→12.4 (§2a-ter) | **CAPFIX1 `underwater_pass` moment** (in flight): capture the engine's underwater draw CB0/CB1/all SRVs incl. t0 + RT pre/post next session. |
| **Underwater CB0 light state** | **FILE/CAPTURE-CLOSED (WATER5)** | `session_CAPFIX2_GAPS_4` captured the engine's OWN underwater pass; its scene CB0 (slot 0) carries a WHITE un-absorbed sun `(1,1,1,1)`, shipped moon `(0.2176,0.5468,1,2.5)`, day-gate OFF — byte-identical to the same-frame terrain CB0. **Engine applies NO CB0 light absorption to the underwater pass.** Decode: bin `decode_underwater_state`; test `engine_underwater_cb0_carries_unabsorbed_white_sun` | CLOSED: the underwater pass consumes the scene light rig unchanged. |
| **Underwater brightness correction (0.133,0.234,0.570)** | **FIT (output-matching fallback; NOT an engine light law — WATER5-refuted)** | WATER3 read it as an engine CB0 light absorption; WATER5 REFUTES that (row above: engine CB0 is un-absorbed). Reframed as an empirical correction that darkens the app's ~5×-too-bright lit arm — a `TerrainColorTint`/effective-`vWaterAlpha` mismatch proxy (Codex-flagged residual), not a light model. KEPT because it improved the oracle (mean abs err 20.8→12.4) and `--water-full` is default ON; default pixels unchanged | Remove once a session renders the engine's own `TerrainColorTint`/`ShadowMap` through this pass and diffs vs captured `WaterRefraction`. |
| Default-path refraction | **FILE (GRAD1: now default ON; oracle-validated)** | `--water-full` is **DEFAULT ON** — the mechanism-correct underwater seafloor pass renders by default. `--no-water-full` restores the old app-owned `TERRAIN_COLOR_SNAPSHOT_HLSL` approximation `terrain·(0.0128,0.0225,0.0655)+bias` (equivalence gate B byte-identical). Fail-open: default without a resolver reverts to the approximation with a NOTICE; explicit `--water-full` fails closed | Done (GRAD1). Residual = byte-exact underwater validation (row below), not the default wiring. |
| Virtual-sun/moon **x** (longitude) + moon `.z` for glitter | CAPTURE / UNKNOWN | `.x` and moon `.z=1448` are engine-orbit runtime, read from captured CB0 (§3 "Tracked gaps") | Shared with light rig §2.4 — needs multi-hour capture. |
| Per-hour virtual-sun-longitude patch in water pass | UNKNOWN | Water pass reuses one canonical captured CB0; re-deriving the engine orbit is deferred (§3) | Same multi-hour capture as light rig. |

### 2.4 Light rig (`LIGHT_LAW.md`, `light_law.rs`, `shader_meta.rs`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Sun/moon heights, latitudes, water heights | FILE | `00_graphics.lua:1043-1050`; residual 0 vs 1212 CB0s (`LIGHT_LAW.md` §1) | Solid. |
| Sun/moon diffuse colors (HSV→linear) | FILE | `ToLinear(HSVtoRGBPost(...))`, residual <1e-4 (§6) | Solid. |
| Sun/moon/cubemap intensities | FILE | `SUN_INTENSITY`/`MOON_INTENSITY`/`CUBEMAP_INTENSITY` (§1) | Solid. |
| Day/night is per-pixel from one uniform (no texture) | FILE | `CalcGlobeNormal`+`DayNightFactor`, `standardfuncsgfx.fxh:347/358` (§4) | Solid mechanism. |
| GMT longitude anchor | FILE | `GMT_OFFSET=2793` (`00_graphics.lua:934`) (§2) | Solid. |
| `vVirtualSunPos.x` (const 2816) | CAPTURE | Engine runtime, in no file; read from captured CB0 (§1) | — |
| Moon `.z=1448` | UNKNOWN (ambiguity CONFIRMED by LEDGER2 — stays UNKNOWN) | Two file identities collide EXACTLY on 1448: `SUN_LATITUDE(848,`00_graphics.lua:1048`) + MOON_HEIGHT(600,:1043) = 1448` AND `MAP_SIZE_Y(2048) − MOON_HEIGHT(600) = 1448`. LEDGER2 verified both operands in the files; because both formulas yield the identical value the vanilla corpus cannot disambiguate — correctly UNKNOWN, do not upgrade | **NEW** capture on a mod retuning `MOON_HEIGHT`/`SUN_LATITUDE` or a different `MAP_SIZE_Y` (breaks the tie). |
| **Runtime sun rig longitudes** | **FILE+CAPTURE (LAW) — FIXED (LIGHT2)** | Camera-follow REMOVED; all four rig `.x` written from `light_law::virtual_rig_longitudes_x(hour24)` = `[sun, moon, moon, sun]`, the day-cycle law proven from unpaused session_FINAL_1 samples (residual <0.01); noon output `[2816,0,0,2816]` byte-equal to every captured CB0 rig; `.y/.z/.w` stay captured/file values; captured-verbatim replay paths untouched | Done. NOTE: the fix exposed that the old `baseline_2816.bmp` lock embedded the bug (captured base camera x=2770.12 → baked sun.x≈2861.9); lock re-baselined as `baseline_2816_v2.bmp` (old file retained as historical evidence). Night-side x(hour) + `.yzw(hour)` curve remain UNKNOWN (multi-hour night capture). |
| Ambient rig (`AmbientPos/Neg{X,Y,Z}`) | CAPTURE (inputs FILE; compose not ported) | The captured field is the engine-composed final ambient (already day/night-blended + boosted), NOT the raw defines. But ALL compose INPUTS are file-derived: day rig `DayAmbientMap*` (`constants.fxh:17-22`), night rig `NightAmbient*` (`constants.fxh:24-37`), `NIGHT_AMBIENT_BOOST=3.0` (`constants.fxh:14`) — LEDGER2 verified. So the compose is **code-portable now**; the row stays CAPTURE only because the day/night blend is not yet implemented, not because any input is missing | Port the ambient compose (day/night blend + `NIGHT_AMBIENT_BOOST`) from `constants.fxh` for per-frame recompose. **Code-gated, not capture-gated.** |
| Night path (`NightifyColor`, moon light) | UNKNOWN (mechanism FILE-portable; only validation capture-gated) | The whole night mechanism is shipped plain-text HLSL, portable NOW: `CalcGlobeNormal` (`standardfuncsgfx.fxh:347`, consumes `DayNight_Hour_SunDir.x` phase at `:349`), `DayNightFactor` (`:358/365`, dot with `.yzw` at `:360`, `FoW.w` gate at `:361`), `NightifyColor` (`:370`), `DayNightWithBlend` (`:393`); constants `NIGHT_OPACITY=0.85` (`:328`), `NIGHT_DARKNESS=0.5` (`:329`), `FEATHER_MIN/MAX=∓0.01` (`:324/325`) — LEDGER2 verified every line. Of the `DayNight_Hour_SunDir` float4: `.x` (hour phase) IS a known file-law and already driven by `--hour` (§2.13); the genuine runtime residuals are the `.yzw` sun-direction **vector curve** per hour and the `FoW.w` day/night **gate** (`vFoWOpacity…MaxGameSpeed.w`, 0 in all 1212 captures). So night is **CODE-gated** (port the shader) with a **capture-gated VALIDATION residual** (no night frame captured; `.yzw(hour)` unfittable) — NOT fundamentally capture-blocked (Codex-checked: the `.x` phase must be listed among the driven inputs, not omitted) | Port `NightifyColor`/`DayNightWithBlend` (all file); then a **NEW** game-speed capture (`FoW.w≠0` at several hours, §8 #1) to VALIDATE the look and fit `.yzw(hour)`. |
| `DayNight_Hour_SunDir.yzw(hour)` continuous curve | **CLOSED (NIGHT1): NO CURVE EXISTS — `.yzw` is HOUR-INVARIANT** | 468 samples hours 0..23 (session_NIGHT_SWEEP_6 + FULL_2 + GAPS_4): `.yzw` bit-identical across an unpaused 1→6 hour sweep at one date; rotation model REFUTED (residual 1.99); `.y≡0`, `.w≈1`; only `.z` moves — with sim-DATE, not hour (`light_law::render_sun_dir_yzw`, tests on real swept bytes) | Residual: exact `.z`(date) map (same-hour/different-date captures) + `FoW.w≠0` night-look validation. |
| ShadowMap (screen-space shadow+FoW) | CAPTURE | 1920×1080 `B8G8R8A8`, shadow in `.r`, FoW in `.g/.b`; proven for the captured water/main binding only (§7) | Light-space 2048² PCF path (`pdxmap.shader:462`) never captured → separate UNKNOWN. |

### 2.5 Borders — thin geometry + render (`BORDER_LAW.md`, `border_law.rs`, `border_pass.rs`, `border_world_synthesis.rs`)

**DEFAULT ON** (BORD5). `--no-borders-from-files` restores the borderless render
(verified byte-identical: the world lock with the opt-out reproduces the pre-BORD5
`4c3cb49c…` hash exactly).

BORD5 (2026-07-07, `session_BORDERZOOM_3`) captured a GENUINE `border.shader`
draw (state border Brittany 9495 | Loire 527, both FRA) **plus the engine's
entire resident border vertex pool** — 31,824,096 bytes decomposing into 40,959
ribbon strips covering 100.0% of the pool. The geometry law was derived from
that pool and verified strip-by-strip (decode harness: session scratchpad
`bord5_*.py`; adversarial review `/tmp/cdx_bord5law.txt`).

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| **Centerline construction** | **CAPTURED-ENGINE** | Chain = maximal run of pair-boundary lattice edges, split at pair-degree≠2 corners; stations at Manhattan arc `{0, 0.5, 1.5, …, M−0.5, M}`; ONE box-3 pass, ends fixed. EXACT (err ≤ 3.3e-4 = f32 noise) on 31,528/40,959 captured strips incl. 27,014/27,048 land/land and 3,901 land/water | Closed for land+coast. |
| **Ribbon extrusion** | **CAPTURED-ENGINE** | Row normal = perp central-diff `P_{k+1}−P_{k−1}` (one-sided ends); verts = P ± 0.75·n; all 40,959 captured strips measure width 1.5 exactly (max err 5e-4). `uv = (side 0\|1, row k)` integers (VS has exactly one `itof`; captured IA = `R16G16_SINT`) | Closed. |
| **Coordinate conventions** | **CAPTURED-ENGINE** | pixel `(x,y)` center = world `(x, (H−1)−y)`; corner `(i,j)` = world `(i+0.5, (H−1)−j−0.5)`; heightmap grid vertex `(i,j)` = texel `(i,(H−1)−j)`, h=raw·0.1, border verts BILINEAR-draped (all 32 captured verts ≤ 5e-4; triangle-split interpolations refuted at 0.012) | Closed. NOTE: `replay_app/render.rs map_to_screen_3d` uses `z = H − py` (0.5 off) — candidate LAB5 follow-up for label/furniture anchors. |
| **Answer-key gate** | **GATE (passing)** | `gate_vanilla_captured_border_strip_evidence`: synthesis from vanilla files reproduces ALL 32 vertices (pos ≤1e-3 incl heights, uv exact) of the captured drawn strip; + local `captured_brittany_centerline_law_reproduced_on_synthetic_staircase` (no installed content needed) | Standing regression. |
| **Per-class `BorderDiffuse` = per-class DDS** | **CAPTURED-ENGINE** | The captured state-border SRV payload is **byte-identical to shipped `map/terrain/border_state_0.dds` (all 7 mips)**, format `BC3_UNORM_SRGB` (78). The old diag23b "single runtime texture" FIT is dead | Closed for state; other classes inferred by the same mechanism (same shader, per-class DDS family). |
| **sRGB sampling** | CAPTURED-ENGINE | SRV format 78 = BC3_UNORM_**SRGB**; pass now uploads `R8G8B8A8_UNORM_SRGB` (CPU BC3 decode per D3D spec) | Residual: BC decode rounding vs GPU — negligible, tracked. |
| Blend / depth / raster / sampler states | **CAPTURED-ENGINE** | Captured live state matches shader blocks: blend 5/6/ADD mask 7; depth LESS_EQUAL write-off; raster solid/cull-BACK/front-CCW; s0 linear bias −0.5 maxlod 4 U-wrap V-clamp | Closed. |
| CB2 semantics | **CAPTURED-ENGINE** | Captured `vSelectionColor = 0`, `vTime_Transparency = (121.15 s, 1.0)` at camHeight 310.13 — consistent with fade windows (province gone >300, state full <400) | Closed. |
| Rank/water classification + impassable override | FILE | unchanged from BORD1/BORD3 (definition.csv, history/states, adjacencies.csv); coastline cross-check disagreement = 0 | Solid. |
| Fade **window** endpoints (province/state) | FILE (LEDGER2) | `00_graphics.lua:774-777` = 200/300, 400/500; no COUNTRY/IMPASSABLE fade defines exist | Settled. |
| **Fade ramp SHAPE inside [NEAR,FAR]** | **FIT (narrowed)** | Captured point (310.13 → 1.0) confirms the state boundary condition but does not discriminate the interior; linear ramp labeled | Multi-height `borders_pass` CB2 sweep inside 200–300/400–500. |
| **Impassable fade window** | **FIT** | Ranked as `state` window (no define) — tracked approximation | Same sweep. |
| **sea\|sea (sea_region) centerline** | **UNKNOWN — pinpointed gap** | 7,536 water\|water strips REFUTE the land law (box3 iterated ≤80× fails on 294/300 sampled); construction differs structurally. Render pass **skips SeaRegion** rather than drawing wrong geometry | Own derivation lane against the captured pool (all sea strips are on disk). |
| **Closed-loop chains** | **FIT (0.13% of land chains)** | Loop anchor rule unverified; ours = lex-smallest corner, fixed duplicated endpoint (counted in `closed_loop_chains`) | Match loop strips in the captured pool. |
| **Chain direction (row 0 end)** | FIT (visual-neutral) | Ours = north/west-first (reproduces the captured strip); a flip preserves winding and only mirrors the texture V profile | Compare more pool strips if the V-profile asymmetry ever shows. |
| **~1.4k strips not yet reproduced** | UNKNOWN (bounded) | 393 row-count mismatches + 880 trace fails + 422 side-sample fails in the harness = loops/x-seam/checkerboard specials, 3.4% of strips, dominated by water classes | Diagnose in the pool harness; land impact is 34 strips (0.13%). |

### 2.6 Map labels (`MAP_LABEL_LAW.md`, `map_label_law.rs`)

**DEFAULT ON since LABGRAD1 2026-07-07** (`--no-map-labels` opt-out = pre-graduation output byte-identical). Engine mapname VB CAPTURED (session_MAPNAME_7B: 23 draws; session_CAPFIX2_GAPS_4: held-out draw) and decoded EXACTLY (LAB3, `map_label_capture.rs`); placement arc law VALIDATED (mean RMS 0.158 wu / 23 labels, held-out 0.035); per-component labelling per engine refutation of one-per-country. **LAB4 2026-07-07 (user-reported)**: display strings are UPPERCASE (23/23 decoded engine labels are caps) and IDEOLOGY-RESOLVED — `ruling_party` from `history/countries` → loc key `<TAG>_<ideology>` ("GERMAN REICH"; engine VBs show "MANDATORY PALESTINE"/"BRITISH IRAQ") → `<TAG>` → tag; baselines project through the production 3D camera at per-point heightmap elevation; and ALL zoom cutoffs (label cutoff + furniture dot/text/capital) now compare against the production camera's REAL height (`MapRenderer::production_camera_height`) — the visible-world-height proxy FIT is RETIRED (it underestimated distance by the FOV factor, drawing capital stars at world zoom that vanilla culls). Residuals (§5.1): size floor & de-clutter DERIVED-BY-FIT, kerning, anchor rule, owner-vs-controller keying, runtime ideology flips (recording feed).

**NAME1 2026-07-07 (user-reported: India must be BRITISH RAJ)**: the display-name
chain now honors the 1936 `set_cosmetic_tag` (history/countries, brace-depth-0
only): `<cosmetic>_<ideology>` → `<cosmetic>` → `<tag>_<ideology>` → `<tag>` →
tag. FILE-proven: `RAJ - British Raj.txt:244` sets `RAJ_UK`;
`countries_cosmetic_l_english.yml` `RAJ_UK_neutrality: "British Raj"` (the plain
`RAJ_neutrality` loc is "India"). Cosmetic COLORS override the tag colors the
same way (`common/countries/cosmetic.txt`; see 2.8). Vanilla gate:
`gate_vanilla_cosmetic_tag_law` (RAJ→RAJ_UK, color 170/10/10, 13 top-level
countries present, 7 gated ones absent). **NAME1b — DLC-conditional cosmetics
LANDED (2026-07-08)**: `dlc_conditional_cosmetics_from_text` parses top-level
`if = { limit = { [NOT] has_dlc = "X" } set_cosmetic_tag = Y }` blocks (pure
has_dlc limit only, so civil-war/event cosmetics are rejected) and merges them
under a DLC predicate. Vanilla start-state cases: **CAN** → CAN_UK
(rgb 119,48,39 → COL3-baked 95,61,56 = the RenderDoc Canada gate target) with
Together for Victory, else CAN_ALY; **COG** → COG_belgian_congo with
Gotterdammerung (inert at 1936 — the Congo is BEL-owned). The bundle assumes
all DLC (the common install + byte target); the recording's real DLC set drives
the no-DLC path when carried (R-DLCLIST). CZE/MEN/SPR stay correctly skipped
(focus/civil-war/event effects, non-DLC limits). Gates:
`dlc_conditional_cosmetics_*` (unit) + `gate_vanilla_dlc_conditional_cosmetics_canada_congo`
(vs real files). Bundle colours refreshed via `refresh_country_colors`; world
lock re-based (Canada-only pixel delta). The name chain also lacks autonomy/subject loc forms
(`<cosmetic>_<autonomy_state>`, `_subject`, overlord forms) keyed by the 1936
`set_autonomy` state — RAJ's direct cosmetic+ideology key already matches
vanilla output, so precedence needs a capture referee, not a guess. Also to
verify: whether the map fill applies `NGraphics.COUNTRY_COLOR_*` modifiers to
the file color before upload (we use the raw `color`; `color_ui` is UI-only).

**NAME2 tracking law 2026-07-07 (user-reported: titles "not shaped properly")**:
ENGINE-MEASURED from the same 23 decoded `session_MAPNAME_7B` labels — the
engine JUSTIFIES a name across its baseline with uniform ADDITIVE letter
tracking per character SLOT (spaces included): per-gap pen deltas exceed the
font xadvance by a constant per label (IRAN +61 fnt px/gap, FRANCE +12, a tiny
island UNITED KINGDOM +121 whose across-the-space D→K gap measures +239 ≈ 2
increments — proving additive-per-slot, refuting a multiplicative stretch);
long names lay out at NATURAL advances (residuals ±2 px; the .fnt ships zero
kerning pairs; word gaps = the space xadvance exactly); tracking never
compresses. Also re-measured: per-label glyph scale is uniform (spread <1% on
clean labels) with sy/sx = 1, and glyph origins are terrain-draped (origin y
varies 9→15 wu across one label). Implemented (commit 06710a39a):
`BmFont::layout_text_tracked` + `draw_text_curved_tracked`; `map_labels`
computes `tracking = max(0, (arc_len_px − natural_px)/(slots−1))` — replaces
the old centre-the-capped-word behavior (the visible defect: short names
clustered at the component centre instead of spreading). Remaining glyph-law
residuals: exact target-span semantics vs our arc (single-capture-fitted),
per-glyph tilt near high-bow ends (chord-vs-tangent ≤6°), city/VP label shape
(separate UI-text system), the ±1-unit world-frame audit (own task).

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Font (`tahoma_60`/`hoi_mapfont4`) + metrics | FILE | `core.gfx:1679`; `hoi_mapfont4.fnt` (§1) | Solid. |
| Display strings (localisation) | FILE | `localisation/*/*.yml` (§1) | Solid. |
| Visibility cutoff `DRAW_COUNTRY_NAMES_CUTOFF=260` | FILE | `00_graphics.lua:781` (§1) | Solid (the cutoff value only). |
| Anchor = centroid of home component | **FIT** | `DERIVED-BY-FIT` (§3.3) | `mapname_pass` capture. |
| Home-landmass = VP-capital connected component | **FIT** | Peak-VP component rule (`map_label_law.rs:279`, §3.2) | `mapname_pass`. |
| Orientation = PCA principal axis | **ENGINE-VERIFIED** | NAME2 2026-07-08: best of all files-derivable laws over 38 frame4700 pairs — median 4.0°, mean 6.7° (max-extent 9.8°, percentile-extent 7.7°, VP-weighted 6.8° all worse); residual tail (Greece 29.5°) unfixable without regressions; `MAP_LABEL_LAW.md` §9 | Closed (PCA kept). |
| Glyph render (world-space projection, UNORM atlas, shipped mips) | **ENGINE-MEASURED** | NAME2b/2c 2026-07-08: world-space quads flat on the map plane, per-corner 3D projection; atlas UNORM fmt 87 (not sRGB); shipped 4-mip chain + −0.4 LOD bias (`mapname.shader`); glyph body 0.026 vs engine 0.033; `MAP_LABEL_LAW.md` §9 | Locks world_v19/europe_v11. |
| Length / extent fraction | **ENGINE-FITTED** | `LABEL_EXTENT_FRACTION=0.75` = engine arc/major median 0.752 (IQR 0.68–0.80, 38 frame4700 labels; old 0.62 was eyeballed) | Gate `frame4700_label_size_gate.rs`. |
| Curvature (quadratic bow) | **FIT** | Least-squares `v≈a0+a2·s²` (§3.6); engine spline UNKNOWN | `mapname_pass`. |
| Letterspacing ceiling | **ENGINE-OBSERVED** | NAME2 2026-07-08: `MAX_LETTERSPACE_STRETCH=2.0` = engine max over 68 labels (ITALY-boot 2.00); confines over-elongated names (CHILE 8.1×→2.0×). NORWAY partial (2.5×→2.0×); its southern anchor UNTRACED; `MAP_LABEL_LAW.md` §10 | Anchor law needs multi-elongated capture. |
| Size law | **ENGINE-FITTED** | NAME2 2026-07-07: `h = min(arc/W_nat×55, 0.22×extent-mask minor)`; extent mask includes owner-tagged impassable states (Libya proof); NO ceiling (SOVIET 131.9 refutes old 65); 89% of 38 engine labels within ±25%, median 0.988; IRAN residual 0.81 (engine breadth metric UNTRACED); `MAP_LABEL_LAW.md` §8 | Gate `frame4700_label_size_gate.rs`; observer far-zoom capture (B1) for Africa-frustum labels. |
| Visibility fade ramp (25% above cutoff) + 2D-height substitution | **FIT** | Ramp + 2D-height stand-in are `DERIVED-BY-FIT` (§3.8) | `mapname_pass` cb1 `Transp_OffsetX`. |
| Glyph RGBA passthrough (pre-coloured atlas) | FILE | `hoi_mapfont4.dds` pre-coloured; `AtlasMode::PreservedColor` mirrors `mapname.shader:77` (§3.9) | Solid (rendering, not placement). |
| Engine baseline spline / collision / de-overlap | UNKNOWN | §4 UNKNOWN-ENGINE-SIDE | `mapname_pass` + multi-country. |
| Owner vs controller keying | UNKNOWN | Law keys on owner; engine may use controller (§4) | Recording controllers + capture. |
| Day/night glyph dimming + `Transp_OffsetX.y` wrap | UNKNOWN | Not reproduced in 2D replay (§3.9) | `mapname_pass` cb1. |
| Centroid vs pole-of-inaccessibility | UNKNOWN | §4 | `mapname_pass`. |
| Raster 4-connectivity vs engine topology | UNKNOWN | §4 | `mapname_pass`. |

**`mapname_pass` = a NEW capture that must be built:** dump the mapname slot-0 VB
(float3 world positions), cb1 `Transp_OffsetX`, cb0 ViewProjection. Every session
so far skipped it for capture budget (`census` shows `captured:false,
uncaptured_reason:"budget"`).

### 2.7 Snow / mud (`SNOW_LAW.md`, `snow_law.rs`)

Analysis/law only — not yet render-wired.

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Addressing law (quarter-res, world extent, Y-flip) | FILE | `standardfuncsgfx.fxh:231-234`, FOW UV; Y-flip confirmed vs capture (§1) | Solid. |
| Channel packing + `SnowMudFade` blend (CB0 off 248) | FILE | `fxh:243/261`; §2 | Solid. |
| Per-region `min_snow_level` floor + date window | FILE | `strategicregions/*.txt` twelve monthly periods; year-wrap (§3) | Solid (floor policy only). |
| `MAP_SIZE`/`FOW_POW2` constants | CAPTURE | Engine-injected, read from bundle (=5632/2048/1.0) (§1) | Read from bundle; not in files. |
| **Actual per-texel snow/mud values** | **UNKNOWN** | Accumulated seeded sim state (`weather.txt`); 3 captures → 3 hashes, one all-zero (§4) | **Extractor feed** (§6): per-hour per-province snow/mud accumulators. Graceful degrade = the file-only floor. |
| Fade-0/fade-1 = prev/current month? | UNKNOWN | Only fade endpoints shader-proven (§7) | Capture across a month boundary. |
| Capture calendar date | FIT/UNKNOWN | Inferred warm/early-game, not recoverable from files (§5) | Record game-date capture field. |

### 2.8 Political color (`POLITICAL_COLOR_LAW.md`, `political_color_synthesis.rs`, `gb_field.rs`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Secondary map shape / sampler / slot | FILE+CAPTURE | 2816×1024 RGBA8, `pdxmap` slot 6, Linear/Wrap; two captures byte-match (`POLITICAL_COLOR_LAW.md` "Proves vs assumes") | Proven. |
| **1936 cosmetic-tag color overrides (NAME1)** | FILE | `set_cosmetic_tag` (history/countries, depth-0) → `common/countries/cosmetic.txt` color replaces the tag color in `country_colors.json`: RAJ→170/10/10 (British red), SYR/LEB→57/113/228 (French-mandate blue), BRM/JOR/KUW/MAL/PAL→201/56/93 (protectorate pink), INS/ICE/URG per file; 11 tags changed vs pre-NAME1. Gate: `gate_vanilla_cosmetic_tag_law` | Gap: DLC/event-gated cosmetics (7 vanilla countries, see 2.6). Runtime cosmetic-tag changes need an extractor feed. |
| Occupation-stripe shader semantics | FILE | `pdxmap.shader:328`, `standardfuncsgfx.fxh:979-987` (diagonal cosine stripe) | Proven from shaders. |
| Country colors (tag→file→colors.txt precedence) | FILE | `country_tags/*`, `countries/*.txt`, `colors.txt` (Static Composition Law) | Solid for 1936 static. |
| 1936 owner/controller from history | FILE | `history/states/*` `history={}` block | Solid for start state only. |
| Secondary map = all-transparent (both captures) | CAPTURE | Byte-scan: 0 non-zero (v2 verdict) — the SRV genuinely all-zero in default view | Observed + explained (user testimony: ordinary occupation = controller fill, not stripes). |
| **province-id → occupation-stripe (rgb+alpha) mapping** | **UNKNOWN (deprioritized)** | Validation is **VACUOUS** — synthesized AND captured both all-zero; no non-transparent texel ever exercised (v2). The controller≠owner→alpha-255 emission is WRONG per user testimony | LOW priority. Needs a capture where the stripe path actually fires (rare trigger: resistance/collaboration/specific mapmode). Ordinary occupation belongs in the PRIMARY fill. |
| Per-hour owner/controller changes | UNKNOWN | History = start only; replay changes at runtime | **Recording** R1 hourly snapshots. |
| Cosmetic/alias/civil-war tag colors | UNKNOWN | Dynamic state, not static file law ("Dynamic Data Needed") | Extractor resolved-tag feed. |
| Primary political fill SRV validation | UNKNOWN | Only `ProvinceSecondaryColorMap` ever captured; primary fill unvalidated | Capture a primary political-fill SRV / RT. |

### 2.9 Terrain (chunks / IDMap / LOD / height) — ledger-only, no law doc

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Terrain index / texture-id / heightmap / world-normal extract | FILE | `bundle_gen.rs:257-330`; `terrain.bmp`+`00_terrain.txt:320` palette | Solid as static bundle. |
| Atlas / normal mips | FILE | Shipped `atlas*.dds` (`GAME_FILES_ATLAS.md:495`) | Solid. |
| `map_visual` CPU composite | FILE | `cpu_compose.rs` | Solid. |
| **TerrainIDMap packing + 4-way blend** | CAPTURE | Native replay requires the **captured** TerrainIDMap; engine composes a packed runtime map (ledger gap 2) | **NEW** offline TerrainIDMap generator validated vs one captured map. |
| Terrain chunk mesh BYTES (VB/IB/CB1) / LOD tiles / CB1 scale | **FILE** (CHUNK1) | Offline generator `terrain_mesh_law.rs` regenerates slot0 (65×65 int grid + edge-stitch classes, pure law), the checkerboard index tessellation (pure law), the per-LOD slot1 height pool (`map/heightmap.bmp` via mod-resolver; `u=raw·10`, `v=`next-even-neighbour alt height for the T-junction snap), and CB1 (`QuadOffset_Scale_IsDetail=[min_x,min_z,tile_size/64,0]`, `vSnap` detail mask, `[0,0.1,scale,0]`). Grounded field-by-field in shipped `gfx/FX/pdxmap.shader` `VS_INPUT_TERRAIN_NOTEXTURE`/`main` + `pdxmap.fxh ConstantBuffer(1,32)`. **Byte-exact gate**: 49 captured chunk draws across live_PAN2 (44: LOD128×20, LOD256×17, LOD512×7) + tracked harness runs (5×LOD512), 49/49 byte-identical; full per-LOD slot1 pools (44/176/704 tiles) regenerate byte-exact (fnv1a64 == recorded). LOD law = only tile_size differs (scale=tile_size/64); slot0/index LOD-invariant. `--chunks-from-files` UPLOADS the generated buffers to the GPU (threaded into VB/IB/CB prep; flag-OFF byte-identical; flag-ON standard-view dump == captured-template dump == `baseline_2816_v3` 307b88b6, deterministic 2x). | **RESIDUAL**: plan/library-load still READS captured chunk files for scene-CB0/camera + per-LOD cb1_scale/detail metadata, so mesh-BYTE dependency is eliminated but the metadata template is not yet — follow-up: derive scale/camera from files. Mixed-LOD `vSnap` quadtree mask stays capture-sourced (runtime-not-file; uniform-LOD → 0). |
| Vertex/index buffers + input layout | **FILE** (CHUNK1) | slot0 `R16G16B16A16_SINT`/stride 8 + slot1 `R16G16_SINT`/stride 4 + index `R16_UINT`, regenerated byte-exact by `terrain_mesh_law.rs` (matches captured `input_layout` and the BORD3 `TerrainGridPatch` signature). Slot1 pool-tail is never fetched (max index 4224 = 16900-byte window; proven, satisfies the under-recorded-pool caveat). | Solid (49-draw byte-exact corpus gate). |
| Scene CB0 (per-frame scene state) | CAPTURE | Canonical captured CB0 (`d3d11_replay.rs:1973`); camera fields patchable, sun/time/snow/fog runtime | SAMP1 cb0 mining pins the law fields; the rest need multi-hour capture. |
| Compiled VS/PS bytecode + reflection | CAPTURE | Consumed from capture (ledger cap-surface) | Shipped-shader compile/define resolver (ledger gap 33; OFF-3 machinery already exists). |
| Height / world-elevation numeric scale | **FILE** (CHUNK1) | `world_y = raw_heightmap_byte · 0.1`, VALIDATED against captured chunk vertex data: the engine stores slot1 height as `i16 = raw · 10` and the shipped VS multiplies by `0.01` (`pdxmap.shader:204`), and `HeightScale=0.1` is CB1 row6.y (`pdxmap.fxh` `vBorderLookup_HeightScale_…`); reproduced byte-exact for all 49 captured chunk draws (`terrain_mesh_law.rs`). Palette is identity grayscale (raw byte = height index). Confirms the WATER_LAW `raw/10` precedent from the vertex buffers, not just headers. | Solid (byte-exact vs captured VBs). |
| Screen-space mip/LOD derivative behavior | UNKNOWN | Shader-derivative, not static content (gap 5) | Implement shader law + runtime evidence. |
| Output-merger / depth / MSAA target state | UNKNOWN | Runtime D3D state; byte-exact parity needs live evidence (gap 34) | Own all replay targets, or one live capture. |
| `LightIndexMap`/`LightDataMap` point lights | UNKNOWN | Runtime/generated textures (gap 18) | Engine-law + one validation frame. |

### 2.10 Rivers / straits — ledger-only

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| `rivers.r8` extraction + heightmap | FILE | `bundle_gen.rs:260-312` | Solid. |
| River mesh synthesizer (polyline→quads) | FILE | `river_mesh_synthesis.rs` reads `rivers.r8`+`heightmap.r8` | Mechanism present. |
| Height scale `0.1·height` | FILE | Matches river-world precedent | Same as border/tree height. |
| **Class-3 width ramp (0→1.1 over 11 stations)** | **FIT (capture-derived)** | `RIVER_SOURCE_WIDTH_RAMP_STEPS=11.0`, `FADE_STEPS=17.0` decoded from the **captured PAN2 river run** (`river_mesh_synthesis.rs:7-15`), not files | One live river validation capture or official palette docs (ledger gap 7). |
| Vertex stride/layout (32B, POSITION+TEXCOORD) | CAPTURE | Decoded from PAN2 river draw (`river_mesh_synthesis.rs:7`) | Same. |
| **`rivers.bmp` palette/class semantics** | **FILE (small/large split) + FIT (per-index detail) — LEDGER2** | The small-vs-large split IS named in shipped text: `00_defines.lua:936-938` `RIVER_SMALL_START_INDEX=0`, `RIVER_SMALL_STOP_INDEX=6`, `RIVER_LARGE_STOP_INDEX=11` (comment "color indices for rivers") ⇒ palette index 0-6 = small river, 7-11 = large. Also `RIVER_FADE_FROM=20.0`/`RIVER_FADE_TO=3.0` (`00_graphics.lua:993-994`). LEDGER2 verified; `river_mesh_synthesis.rs` tracks `river_class` but does NOT yet cite these define lines (see AUDIT note). Residual FIT: the exact per-index flow/source meaning beyond the small/large boundary | Adopt the `00_defines.lua:936-938` boundaries in the synthesizer; per-index detail via live river capture or docs. |
| River/strait mesh + `RiverData` convention | FIT | Synthesizes from images but needs convention validation (gap 8) | Offline generator + fixture compare. |
| Strait/adjacency geometry | UNKNOWN | `strait.shader`, adjacencies define sources not the VB | Offline strait generator. |
| Animated water time / camera scene | UNKNOWN | Runtime | Scene CB0 + time. |

### 2.11 Trees (T2) — ledger-only

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| `trees.r8` / `tree_tint` / `tree_season` extraction | FILE | `bundle_gen.rs:262-316` | Solid. |
| Instance position (cell-center→world, height·0.1, flipped z) | FILE | `tree_instance_synthesis.rs:99-102` | Derived rule. |
| Template mesh (120 verts, stride 56) | CAPTURE | Decoded from PAN2 tree bundle (`tree_instance_synthesis.rs:5-11`) | — |
| Instance stride (28B: pos+yrot+slopes) | CAPTURE | From tree VS signature (`tree_instance_synthesis.rs:11`) | — |
| **Exact scatter anchor (37-instance)** | **FIT** | *"not reconstructable from this bundle"* — no per-instance records (`tree_instance_synthesis.rs:123`) | Offline `.gfx`/`.asset`/`.mesh` scatter decoder (ledger gap 9). |
| **Rotation = deterministic cell hash** | **FIT** | `deterministic_tree_rotation` — a stand-in, not engine scatter | Same decoder. |
| **Model/template selection per index** | **FIT** | One-cell→one-instance; exact model/season selection open (ledger gap 9) | Tree model decoder. |
| Camera-height fade window | **FILE (LEDGER2)** | `00_graphics.lua:1068-1069`: `TREE_FADE_NEAR=250.0`, `TREE_FADE_FAR=350.0` — the tree fade window is file-exact (LEDGER2 verified; also `DRAW_MAP_OBJECTS_CUTOFF=1100.0` at `:757`) | Windows settled from files; interpolation shape (as with borders) is C++-side. |
| "Second tree sub-draw" (PS `6b77b6df9ba8f03d`) | **RESOLVED (TREES2)** | Byte-exact recompile (OFF-3): `6b77b6df` == `gfx/FX/tree.shader:tree_low_gfx` PS — the `LOW_END_GFX` variant of the SAME lit tree PS (cb0/cb1 only, no EnvironmentMap/LightData/LightIndex/SnowMud/cb2). It is **mutually exclusive** with the full `tree` PS (`277b7b5e`), selected by the LOW_END_GFX graphics setting, NOT a second same-frame draw. Every captured session (incl. `live_trees_measure`'s 8-draw full pass) binds only the full-detail family; the low-gfx PS is compiled-but-bound-to-no-draw. So there is **no unrendered second sub-draw** at the captured settings. Warning logic now evidence-gated (`tree_second_subdraw.rs`); opt-out anchor `--no-trees-second-subdraw` byte-identical. | Nothing to render lawfully unless a LOW_END_GFX-machine capture ever binds it to a draw (then `low_gfx_bound_unrendered` fires the seam note). |
| Season/snow tree state | UNKNOWN | Runtime season/snow | Season CB1 + snow feed. |

### 2.12 Buildings preview (B2) — ledger-only

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Placement locators (`buildings.txt`) | FILE | `bundle_gen.rs`; semicolon rows (`MAP_3D_OBJECTS_PLAN.md`) | Manifest only. |
| Building rules (`show_on_map`, meshes) | FILE | `common/buildings/00_buildings.txt:3-5` | Manifest only. |
| **`AtlasCoordinate = (0,0,1,1)`** | **FIT (GUESS)** | `building_files.rs:606` literally `"AtlasCoordinate = (0,0,1,1) GUESS"` | Capture a `map_objects_pass` OR `.mesh`/material importer. |
| **`vUVAnimSpeed = 0`** | **FIT (GUESS)** | `building_files.rs:606` `"vUVAnimSpeed = 0 GUESS"` | Same. |
| Building mesh geometry / material | UNKNOWN | `.mesh` = binary `pdxasset`, no decoder (ledger gap 19) | `.mesh` importer OR `map_objects_pass` capture (`MAP_3D_OBJECTS_PLAN.md` Option A/C). |
| Per-game level / damage / control | UNKNOWN | Runtime (ledger gap 20) | Recording R1/R2. |
| City scatter from `cities.bmp` | UNKNOWN | Engine scatter not reproduced ("No-fabrication implications") | Capture or scatter-rule impl. |

### 2.13 Day / night — runtime CB0 patch (`shader_meta.rs`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| `--hour` writes `DayNight_Hour_SunDir.x = h/24` | FILE | Longitude phase law (`shader_meta.rs:954-974`; LIGHT_LAW §4) | Correct mechanism for `.x`. |
| Factor gate `vFoW…MaxGameSpeed.w=1` | FILE | Un-gates the blend (`shader_meta.rs:976-996`; LIGHT_LAW §4-5) | Correct. |
| **`.yzw` sun-direction not repatched per hour** | **CLOSED (NIGHT1) — correct by law** | `.yzw` is HOUR-INVARIANT (proven bit-identical across hours at one date), so echoing the captured vector at every `--hour` IS the engine behavior (`write_sun_dir_yzw_hour_invariant_field`, full-buffer byte-noop test) | Residual: `.z`(date) drift map only. |
| **Night look (`NightifyColor`, moon cross-fade)** | **FIT/UNKNOWN (mechanism FILE-portable, LEDGER2)** | Night path never exercised in corpus (`FoW.w=0`), so unvalidated — but the port itself is fully file-derivable: `NightifyColor` (`standardfuncsgfx.fxh:370`) + `DayNightWithBlend` (`:393`) + `NIGHT_DARKNESS`/`NIGHT_OPACITY` (`:329/:328`) are shipped HLSL. Code-gated port; capture-gated validation | Port the shipped `NightifyColor`; multi-hour night capture (LIGHT_LAW §8 #1) to VALIDATE. |
| Default = keep captured CB0 (terminator off) | CAPTURE | `day_night_hour=None` keeps captured day state (`render.rs:1372`) | — |
| Continuous hour curve validation | UNKNOWN | 4 discrete hours only | Same night capture. |
| Sun/moon `x(hour)` orbit | UNKNOWN | Constant in corpus; engine orbit unknown | Same night capture. |

### 2.14 Camera / projection / wrap (`render.rs`, `camera.rs`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Zoom→camera-height grounding | FILE | the game map camera min/max defines; `hoi4_map_camera_defines_from_bundle` (`render.rs:1109`) | Solid. |
| East-west wrap offsets | FILE | `map_wrap_offsets`; `fmod(map_pos.x, map_w)` (`render.rs:311-317`, `:1407`) | Derived rule (world map). |
| ViewProjection patch | CAPTURE→recompute | Captured VP kept bit-exact if camera unchanged; recomputed on move (`shader_meta.rs:860-884`) | Correct. |
| **Free-camera parity vs engine pan/zoom** | **FIT (clamp inputs FILE, LEDGER2)** | `render.rs:306-313` marks the off-map letterbox/visibility as a PROXY needing "the game camera pan/zoom limits from common/defines". Those limits ARE file-derived (LEDGER2 verified): `CAMERA_MIN_HEIGHT=50.0`/`CAMERA_MAX_HEIGHT=3000.0` (`00_graphics.lua:1495-1496`), `CAMERA_OUTSIDE_MAP_DISTANCE_TOP/BOTTOM=200.0` (`:1184-1185`), `CAMERA_ZOOM_SPEED/KEY_SCALE/SPEED_DISTANCE_MULT` (`:1187-1189`). The residual FIT is the actual view/projection MATRIX (FOV, near/far, tilt) the engine builds per pan/zoom — engine C++, not in defines | Ground the off-map clamp from `common/defines` now; capture the engine camera CB0 view/proj at several pans for the projection matrix. |

### 2.15 Gradient borders — political GB field (`gb_field.rs`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| GB channel structure (measured) | FILE+CAPTURE | Measured channel layout (`gb_field.rs:1-32`); validated vs GB channel ground truth (MEMORY) | Solid for land political. |
| GB constants (`GB_CAM_MIN/MAX`, opacity, cutoffs) | FILE | `constants.fxh:195-210` | Solid. |
| Channel2 + country-layer synthesis | FILE | `gb_field.rs:90-116`, `:267-390` | Solid for land. |
| Owner/controller from recording | CAPTURE(recording) | `render.rs:1157-1199` scrub uses recording owners | Recording feed. |
| **Sea-zone / naval layer** | UNKNOWN (render-off at 1936 PROVEN correct, SEA1 2026-07-07) | Needs R6 naval supremacy (ledger gap 13). **SEA1 measured** (live_PAN2 = vanilla 1936 start): ch1-top carries the full sea-zone field (constant navy 51,63,89 + border-distance alpha over 76% of sea texels) but ch2-top has **green==0 at every texel** (full every-2nd-texel scan), and the shipped law gates each GB layer by `TexCh2.g` at that layer's own uv (`gradient_border_apply`, `standardfuncsgfx.fxh` — `TranspA = tex2D(TexCh2, vUV).g`) ⇒ the sea layer contributes NOTHING at the 1936 start; vanilla itself shows no sea lines then. Our zeroed top half is engine-faithful for static-1936. Residual data point: ch2-top has a RED (255,0,0, a=255) fill over 8.4% of sea texels — invisible to this composite (g=0); its consumer (Codex-identified, verified: `dominance_fx_apply` at `standardfuncsgfx.fxh:989` reads Ch2-top red as its `Control` input, called from `pdxwater.shader:404`) is the naval-DOMINANCE water FX, not a boundary line (evidence maps `w1check\gb_ch2top_red_map.pgm`, `gb_ch1top_borderband_map.pgm`) | **Recording** R6 per-hour tri-state → write ch2-top green fills; ch1-top geometry then synthesizes from `map/strategicregions` (static, file-derivable). |
| GB per-frame recompute vs captured | FIT | Generated field replaces captured SRV (`render.rs:755-758`) | Land proven; sea open. |
| **Map-fill country color transform (COL3 2026-07-07)** | FILE+CAPTURE | Baked GB country color = `trunc(255·hsv_to_rgb(h+HUE, min(1,s·SAT), min(1,v·BRIGHT)))` with `00_graphics.lua:1164-1166` `COUNTRY_COLOR_HUE/SATURATION/BRIGHTNESS_MODIFIER` (vanilla 0.0/0.6/0.8; `COUNTRY_UI_COLOR_*`=1.0 keeps UI raw). BYTE-EXACT vs the engine-generated GB1 country layer in TWO captures (live_PAN2 1936 + session_POLCOLOR_1 1940+): ENG (201,56,93)→(160,91,108), GER→(81,78,74), SOV→(100,46,51), BRA→(82,116,76), SWE→(96,142,197), RAJ cosmetic (170,10,10)→(136,59,59). round() misses 5/6 by +1 ⇒ engine truncates. Resolver-read (mod palette shifts honoured), f64 lua parse (an f32 round-trip perturbs truncation boundaries). Gate `gate_engine_country_color_bake_law` | Proven. Gaps: HUE modifier UNIT untraced (vanilla 0.0; applied as hue-circle fraction); `ProvinceSecondaryColorMap` occupation colors not yet checked against the same law; `IMPASSABLE_GRAY` 0x5c5c5c kept as measured baked-space constant (raw-space source untraced). |
| **Political-mapmode scene CB0 `vGBCamDistOverride_GBOutlineCutoff` (COL3)** | FILE+CAPTURE | `(1.0, 0.0, 0.98, 0.973)` in EVERY political CB0 sample: 3768 samples, 4 sessions (BORDERZOOM_1/2/3 + POLCOLOR_1), heights 50..3000 — HEIGHT-INVARIANT. `.zw` = defines `GRADIENT_BORDERS_OUTLINE_CUTOFF_STRATEGIC_REGIONS`(0.98)/`_COUNTRY`(0.973) (`00_graphics.lua:880-885`); `.x=1` forces the strategic (top) layer to the near-camera response, `.y=0` leaves the country layer on `cam_distance`. Written per camera update into every replayed pass CB0 (`shader_meta::write_political_scene_fields`; `None` on all byte-exact reference paths — QA frame parity still 100.000000%). Land-fill referee gate at the matched POLCOLOR camera: mean land \|err\| 0.0529→0.0172 linear RGB | Proven for political mapmode (the replay's only mode). Other mapmodes' overrides untraced (each needs its own capture). |
| **`ShadowFadeFactor` height law (COL3)** | FILE+CAPTURE | `clamp((DRAW_SHADOWS_CUTOFF − camheight)/DRAW_SHADOWS_FADE_LENGTH, 0, 1)` with defines 400/50 (`00_graphics.lua:860-861`); max err **0.000000** over all 1433 BORDERZOOM_1 sweep samples (h 50..3000; POLCOLOR h=387.386 → 0.252269 exact) | Law proven; PROVENANCE-GATED (Codex review): live_PAN2's captured ShadowMap SRV is NULL, so a nonzero factor against it would darken uniformly (`shadow.fxh:50`) — the written factor is collapsed to 0 via `PoliticalSceneCb0::shadow_map_bound=false` until a shadow pass is replayed (A/B at the POLCOLOR camera: pixel-identical, the term does not fire on this path). Replaying/synthesizing the shadow pass is a future lane; then flip the gate. |
| **Political sea BASE look** | UNKNOWN | Engine political frame (POLCOLOR borders-pass post-RT) shows a pale grey-green sea vs our (faithful-to-PAN2) dark reflective water — but the referee save has FOW over the visible ocean, so the pale wash is FOW-CONTAMINATED evidence, NOT a clean 1936 political-sea target. The GB strategic layer is NOT the pale fill (override `.x=1` ⇒ interior fill 0; ch2-top g=0 gates it off entirely) | Needs a NO-FOW political-mode capture (fresh 1936 game, mid zoom) to separate the political-water look from FOW; then decompose the water fresnel/refraction inputs vs our replay. |

### 2.15b Map furniture composite — VP dots / capital stars / city names (FURN1, `replay_app/src/map_furniture.rs`)

**DEFAULT ON since FURN1 2026-07-07** (`--no-map-furniture` opt-out byte-identical to the
pre-FURN1 default; baseline chain v7/v6/v5/v2). The MAPICON1 law (§ acceptance rubric
row 7) now renders in the frame. Live-oracle acceptance 2026-07-07 (solo vanilla
instance, gui_scale=1.0, `w1check\furn_oracle_*.png`): shape law CONFIRMED against
shipped VP values — Melbourne 20 VP → circle (proving the `>=20` bucket edge),
Adelaide 15 → pentagon, Canberra 30 + `capital = 285` → star, Hobart 1 VP culled at
the observed camera distance.

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Marker set / anchors / tiers / capitals | FILE + ENGINE-MEASURED | MAPICON1 law: `history/states` victory_points, `history/countries` capitals, `VICTORY_POINTS_<id>` localisation, provinces.bmp centroids (match shipped `map/unitstacks.txt` per-province positions within ~2px — verified Dublin/Tullamore/Wexford), `00_graphics.lua:947-963` tiers+cutoffs. **Capital law (user-reported fix)**: ONE star per country — the HIGHEST-VP province in the capital state (oracle: Canberra 30 stars, Sydney 20 same-state circles; single-state 1936 Ireland = one Dublin star, not three); ties → lowest id (engine tie rule UNTRACED). **Anchor projection**: through the production 3D camera at heightmap elevation (`map_to_screen_3d`; `world_y = raw*0.1`, CHUNK1 scale, cross-checked vs unitstacks heights) so markers sit ON the rendered terrain — the flat 2D mapping diverges from the tilted-perspective terrain render at close zooms (see the CAM1 residual below) | Solid; anchor = centroid ≈ unitstacks (verified). |
| Strip frame law (shape x color grid) | FILE+CAPTURE-VALIDATED | Decoded shipped strip (4 shapes x 5 colors: star/circle/pentagon/square = capital/t2/t1/t0, areas 353/255/197/116; evidence gate `gate_vanilla_vp_strip_grid_evidence`); shape↔tier mapping confirmed vs live oracle + shipped VP values | Solid. |
| Diplomatic color column | **FIT (documented default)** | 5 columns = relation palette (oracle shows RED for the viewing player's enemies); observer replay renders neutral GRAY (column 4). The engine's relation→column map is engine C++ | A 1936-observer screenshot (needs menu click-through) to pin the observer column; low risk. |
| Zoom gating (dot/text cutoffs) | FILE | `VpIconDefines::{dot_visible,text_visible}` from `00_graphics.lua:947-963` verbatim comments; consistent with oracle at two camera heights (Hobart culled, Melbourne text visible) | Interp shape + `<=` comparisons are law choices (UNTRACED engine C++). |
| Camera-distance proxy | FIT (shared) | `screen_h / zoom` visible-world-height stand-in — the SAME approximation the country-label cutoff uses | Engine camera capture at known zooms would calibrate both. |
| **Text font/scale/palette** | **ENGINE-MEASURED (refutes the naive .gui reading)** | The `.gui` names `hoi_20bs`, but measured on-screen text is `hoi_mapfont4` metrics at x0.40 (advance-sum exact on two words: 149/119 px vs 370/298 sums; glyph ink confirms), white body + dark rim = INVERSE of the label atlas palette, body alpha ~1.0 (atlas 0.5 → normalized x2). Constants VP_TEXT_MAPFONT_SCALE/PEN_DX/LINE_MID_DY cited in `map_furniture.rs` | The engine's substitution/scale pipeline is UNTRACED; a GUI-text draw capture would close it. Residual: our dark rim reads slightly softer than the oracle's. |
| **Icon draw scale** | **ENGINE-MEASURED** | On-screen dot = authored frame x1.25 (red-disk diameter match; bracket [1.2,1.3], `icon_scale_lineup.png`) | Residual: oracle highlight ring reads fully bright vs the authored asymmetric arc (engine shading pipeline, UNTRACED). |
| Port anchors / naval+air base icons | UNKNOWN (next furniture slice) | `naval_base_mapicon` container at `interface/mapicons.gui:1020-1040` (GFX_mapicon_naval_base / _large), `NAVALBASE_ICON_DISTANCE_CUTOFF=900`, `AIRBASE…=900` (`00_graphics.lua`); 1936 static levels in `history/states` buildings; positions `map/buildings.txt` locators | FURN2: same composite pattern; file-derivable for the static start. |
| **Synthesized-camera projection (CAM1)** | **ENGINE-MEASURED — CLOSED 2026-07-07** | The USER drove a wheel-zoom sweep (session_BORDERZOOM_1: 1414 CB0 samples at 250ms, heights 50.00..3000.00 = the exact CAMERA_MIN/MAX_HEIGHT defines, each sample carrying the engine's full ViewProjection matrix). Decomposition: `fov_y=44.977°`, `near=5`, `far=4000` HEIGHT-INVARIANT; yaw=roll=0; pitch varies 53.5°(h50)→89.4°(h3000) via ONE rule — the camera looks at a ground point `dz(h)` north of itself (37.0@h50 → 30.1@h3000). Implemented as the measured `ENGINE_CAMERA_TILT_TABLE` (camera.rs, 29 engine-sampled rows, linear interp — no fitted formula); `camera_for_ground_center`/`camera_from_position`/`height_for_screen_zoom` rebuilt on it. GATES: unit tests embed three RAW engine matrices (h=50/499/2999) and require per-cell reconstruction ≤0.3% (the old frozen-basis code fails by ~40% at h=50); Wexford-row coastline vs provinces.bmp went from MISSING to ±0.4px. Residual: Dublin-bay east edge ~7px vs the RASTER — visual-coast-vs-political-raster ambiguity; the referee is an engine RT at a matched camera (not yet captured) | Optional: capture one engine frame at a sweep camera for pixel-level referee; labels/furniture already project through the corrected VP. |

### 2.16 UI composition / chrome (`game_ui`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| `.gfx`/`.gui`/`.fnt`/localisation parse | FILE | `game_ui` probe (`probe.rs:151-196`) | Solid, QA-gated. |
| Topbar / HUD layout + flags | FILE | `topbar.rs`, `hud.rs`; flag precedence tries candidates | Solid layout. |
| DDS/TGA texture decode + sprite slicing | FILE | `assets.rs:112-229` | Solid. |
| Font coverage QA | FILE | `probe.rs:479-505` | Solid. |
| Flag selection | FILE | `topbar.rs:701-731` | Solid for tag/ideology. |
| Cosmetic/flag precedence | UNKNOWN | Exact precedence needs runtime census (ledger gap 12) | `census` of known tags. |
| Dynamic values / listbox / scrollbar | UNKNOWN | Runtime game-state binding (gap 27) | Recording + runtime probes. |
| Player-country mapping | UNKNOWN | Session metadata (gap 26) | Recording R4. |

### 2.17 Overlay fonts (`replay_app/font.rs`)

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| `fonts.json` + `.fnt` page load | FILE | `font.rs:153-170` | Solid. |
| `hoi_mapfont4` label atlas | FILE | Pre-coloured RGBA passthrough (labels) | Solid. |
| **Overlay clock font (`hoi_18mbs`)** | **FIT** | `replay_app` still uses a **TGA proxy** font because `hoi_18mbs` pages are DDS-only (`GAME_FILES_ATLAS.md:62`, ledger rec #7) | Reuse `game_ui` DDS decode in `replay_app`; switch overlay to `hoi_18mbs` (low effort). |
| Language / cosmetic string selection | UNKNOWN | Localisation language selection | Recording modset + language. |

### 2.18 Units / counters / rail / arrows — ledger-only

| Parameter | Class | Evidence / FIT | Cracking plan |
|---|---|---|---|
| Static locators (`unitstacks.txt`, `railways.txt`, `supply_nodes.txt`) | FILE | Bundled as source metadata (`game_data.rs:203-214`) | Manifest only. |
| Unit models / counters render | UNKNOWN | Not implemented (ledger gaps 21,25) | Interface-asset overlay + recording divisions (`MAP_3D_OBJECTS_PLAN.md` counters). |
| Rail / supply overlay | UNKNOWN | No render layer (gap 22) | Static graph draw from files + mapicons. |
| Map arrows / battle plans | UNKNOWN | No renderer; `.lua`→`.shader` alias engine-side (gaps 29,30) | Recording orders + arrow decoder. |
| Particles / effects | UNKNOWN | No parser (gap 31) | Particle asset decoder. |
| Naval supremacy display | UNKNOWN | Runtime (gap 13) | Recording R6. |

---

## 3. Prioritized debt list (FIT + UNKNOWN, by visual impact)

Ordered so the orchestrator can burn down top-to-bottom. Impact = how much a wrong
value changes what the viewer sees across the whole map.

| # | Debt item | Class | Layer | Cracking plan / instrument |
|---|---|---|---|---|
| 1 | ~~**Virtual-sun camera-follow**~~ **CLOSED (LIGHT2 — confirmed by LEDGER2)** — the render NO LONGER translates the sun x with the camera. `shader_meta.rs::write_virtual_light_rig_fields` (`:924-951`) now writes all four rig `.x` from the day-cycle law `light_law::virtual_rig_longitudes_x(hour24)` (`light_law.rs:447-451`); the old `camera.x - reference_x` FIT is gone (LEDGER2 grepped: zero remaining camera-follow in the render path). Noon output `[2816,0,0,2816]` is byte-equal to every captured rig. **Residual** (was conflated here): the DAY side is closed; the night-side `.x(hour)` and the `.yzw(hour)` sun-direction curve remain UNKNOWN (multi-hour night capture) — tracked at debt #6, not here | FILE+CAPTURE (LAW) | Light rig | Day side done. Night `.x/.yzw(hour)` folds into the debt-#6 multi-hour night capture. |
| 2 | ~~**Exposure = clamp floor**~~ **CLOSED (EXPO1)** — per-frame luminance adaptation ported (`downsample_luminance.shader` chain, `luminance_law.rs`); AvgLum computed from the rendered scene each frame on `--postfx-restore-full`. Clamped branch byte-exact vs capture; unclamped branch shader-cited + unit-tested only (open item: bright close-up `restore_pass` capture, since no current replay view exceeds the 0.5 floor) | FILE (per-frame) | Grading | Residual: capture the unclamped branch; add easing curve for frame sequences. |
| 3 | **Default render path is capture-anchored** — ~~grading/water~~/borders/labels default-OFF | FIT (structural) — **PARTIALLY CLOSED (GRAD1)** | ~~Grading~~/~~Water~~/Borders/Labels | **GRAD1 graduated `--postfx-restore-full` and `--water-full` to DEFAULT ON** (fail-open downgrade on missing evidence; explicit opt-outs restore the old path byte-identically — equivalence gates A/B). **Borders/labels REMAIN default-OFF** (placement/corner laws pending next capture; user rejected current look) — that is the residual of this debt. |
| 4 | **Snow/mud actual values** — accumulated seeded sim state; file floor is visibly wrong for Eastern-Front winters | UNKNOWN | Snow | Extractor per-hour snow/mud feed (`SNOW_LAW.md` §6); files-only floor is the graceful degrade. |
| 5 | **Per-hour owner/controller** — political fill/borders/labels/occupation all change at runtime; history gives start state only | UNKNOWN | Political/Borders/Labels | Recording R1 hourly snapshots + R2 deltas. |
| 6 | **Night path — PARTIALLY CLOSED (NIGHT1 2026-07-07)**: `.yzw(hour)` question RESOLVED (hour-invariant, no curve); hour-swept night CB0 corpus now exists; `FoW.w=1` un-gates the shipped-shader night blend | UNKNOWN→CAPTURE(residual) | Light/Day-night | Residual: `FoW.w≠0` never captured (all 1680 CB0s gate=0) — night LOOK still shader-port-validated only; needs a game-speed capture with the gate open + `.z`(date) map. |
| 7 | ~~**Default water refraction approximation**~~ **CLOSED (GRAD1)** — the mechanism-correct underwater seafloor pass (`pdxmap` `Effect underwater` port + WATER3 light absorption) now renders BY DEFAULT; the old mean-color/linear-fit approximation is opt-out only (`--no-water-full`) | FIT → FILE (default; oracle-validated, not yet engine byte-exact) | Water | GRADE2 hook landed + GRAD1 graduated `--water-full` to default-ON. Residual is the byte-exact underwater validation (debt row 2.3 "Underwater byte-exact validation" — still UNKNOWN/blocked pending the `underwater_pass` capture), NOT the default-path wiring. |
| 8 | **Terrain chunk mesh/LOD + TerrainIDMap = captured** — largest captured dependency; blocks arbitrary cameras/mods | CAPTURE→need generator | Terrain | Offline TerrainIDMap + chunk/LOD generator validated vs one capture (ledger gaps 2,3). |
| 9 | **Label placement — CLOSED (LAB3+LABGRAD1 2026-07-07)**: mapname VB captured (7B 23 draws + GAPS_4 held-out), decoded exactly (stride 20, world-space glyph quads), arc law VALIDATED (RMS 0.158/0.035 wu); per-component labelling matches engine; graduated DEFAULT ON | FIT→CAPTURE-VALIDATED | Labels | Residuals honestly open: size floor + de-clutter fraction DERIVED-BY-FIT, kerning, anchor rule, owner-vs-controller keying (need multi-zoom / owner-raster captures). |
| 10 | **Border mechanism — RESOLVED-BY-EVIDENCE (BORD4 2026-07-07)**: border.shader PS/VS are NEVER CREATED in normal play (zoom-swept at-launch session, zero creations while all other families observed) — vanilla borders = the GRADIENT-BORDER composite (already default-rendered; class table + defines law in BORDER_LAW.md; mod-compat defines now resolver-read fail-closed) | FIT→FILE/CAPTURE | Borders | border.shader path stays flag-only (other mapmodes); GB residuals: RAMP_D0 0.875 vanilla-fit, channel2 `.b` FX selector, outline-cutoff CB delta. |

Lower-impact tail (continue burn-down after top 10):

11. River class-3 width ramp + `rivers.bmp` palette semantics — FIT/UNKNOWN (live
    river capture or docs).
12. Tree scatter anchor + rotation + model selection + 2nd sub-draw — FIT/UNKNOWN
    (offline tree decoder + complete tree-pass capture).
13. Building `AtlasCoordinate`/`vUVAnimSpeed` GUESS + mesh geometry — FIT/UNKNOWN
    (`map_objects_pass` capture or `.mesh` importer).
14. Water reflection cube: upload captured payload not yet resolver-decoded from
    `reflection.dds` — CAPTURE(file-equivalent) (thread resolver into water prep).
15. Sea-zone / naval-supremacy gradient-border layer — UNKNOWN (recording R6).
16. Moon `.z=1448` identity ambiguity — UNKNOWN (mod-retune capture).
17. Free-camera parity vs engine pan/zoom law — FIT (capture engine camera CB0).
18. Overlay clock font TGA proxy → `hoi_18mbs` DDS — FIT (low effort).
19. Height/world-normal elevation scale — UNKNOWN (one known-coord capture).
20. Occupation-stripe write law — UNKNOWN, deprioritized (validation vacuous; rare
    trigger; ordinary occupation is primary-fill).
21. ShadowMap light-space PCF path, `LightIndexMap`/`LightDataMap`, screen-space
    mip/LOD derivatives, output-merger/depth/MSAA state — UNKNOWN (engine-law +
    live evidence).
22. Units/counters, rail/supply overlay, map arrows, particles — UNKNOWN
    (recording domains + asset decoders + interface-asset overlays).

---

## 4. Misclassification risks — claimed-evidenced in docs but actually untraced

These are the rows most likely to be **read** as FILE/CAPTURE-solid from the law
docs' prose but are, on the citations, FIT or vacuous. Flagged so a reader does not
over-trust them.

1. ~~**Light rig "camera-follow REFUTED" vs the actual render.**~~ **RESOLVED
   (LIGHT2; re-verified by LEDGER2 2026-07-06).** The law and the renderer now
   AGREE: `shader_meta.rs::write_virtual_light_rig_fields` (`:924-951`) writes the
   four rig `.x` from `light_law::virtual_rig_longitudes_x(hour24)`
   (`light_law.rs:447-451`) — the fixed-sun day-cycle law — and the old
   `camera.x - reference_x` camera-follow FIT has been removed (LEDGER2 grep found
   no camera-follow term in the render path). A reader of `LIGHT_LAW.md` now sees
   the shipped-file rig that actually renders on the day side. Residual (NOT a
   misclassification, an open gap): night-side `.x(hour)` and `.yzw(hour)` are
   still UNKNOWN pending a multi-hour night capture (debt #6). This is no longer
   the top misclassification risk; the top live risk is now the reflection-cube
   "files-first" reading (#5 below), which is still CAPTURE.

2. **Grading "wired as the runtime source."** ~~RESOLVED (GRAD1).~~ As of GRAD1
   the files-first grade is **DEFAULT ON**, so `GRADING_LAW.md`'s "wired as the
   runtime source in `replay_app`" now matches the default render path. The old
   default (captured restore bundle / GAME POST GAPS shader path) is reachable
   only via `--no-postfx-restore-full`. (The embedded 2D `render.rs` shader still
   carries the GAME-POST-GAPS naming for the opt-out path; that is now the
   non-default branch.) Residual misclassification risk here is gone; the
   remaining grading caveat is the value-NAME provenance (#3 below), not the
   default path.

3. **LUT "byte-exact residual 0."** True, but the three matching cubes are
   byte-identical, so the capture cannot prove the engine used `max_distance` — the
   *value-name* selection is a plausibility argument, not proof. The pixels are
   right; the selection provenance is not fully closed.

4. **Political secondary color "100% byte-exact / texel match 0."** The doc itself
   flags this as **VACUOUS** (`validation_is_vacuous:true`) — both synthesized and
   captured maps are all-zero. A skim of the 100% number without the vacuous flag
   would badly overstate fidelity. The province-id→stripe mapping is UNKNOWN.

5. **Water reflection "files-first."** The pixels are proven identical to
   `reflection.dds`, but the runtime uploads the **captured** payload; decoding the
   shipped file through the resolver is a tracked follow-up. It is CAPTURE with a
   file-equivalence proof, not yet a file-load. (Honestly labelled in `WATER_LAW.md`
   §1, but easy to read as FILE.)

6. **Underwater refraction "mechanism-correct / rides `--water-full`."** The
   mechanism is shader-exact, but byte-exact validation is **blocked** (seafloor
   SRVs absent in `restore_gate_1`; oracle exact-channels only 23.9%). Correct
   mechanism ≠ validated output.

7. **Border render law "files-first."** The geometry classification is files-first,
   but the per-class `BorderDiffuse`, the linear fade curve, impassable's window,
   the D3D vertex packing, and the depth/raster/sampler states are all
   FIT/UNKNOWN — the one border capture (diag23b) is unresolved and may be
   contamination. A "files-first" reading overstates the render fidelity.

8. **Snow `~90.5% exact byte match`.** The doc correctly calls this a "mostly-zero
   artifact" (both floor and capture ~99% zero). The percentage is not a meaningful
   match; the actual per-texel snow is UNKNOWN sim state.

9. **Bloom "byte-close / 99.26% exact."** Genuinely strong, but verified only at
   `σ=1.5`; a wide-sigma mod could drift. FILE with a bounded caveat, not
   universal.

---

Sources mined: all `replay_interface/documents/*_LAW.md`,
`GAME_FILES_EXTRACTION_LEDGER.md`, `GAME_FILES_ATLAS.md`, `MAP_3D_OBJECTS_PLAN.md`;
and the implementations `map_viewer/src/{grading_law,bloom,water_law,light_law,
border_law,map_label_law,snow_law,political_color_synthesis,tree_instance_synthesis,
river_mesh_synthesis,building_files,terrain_chunk_synthesis,shader_meta,d3d11_device,
d3d11_replay}.rs`, `replay_app/src/{render,main,map_labels,font,gb_field}.rs`.
Classifications trust citations, not adjectives; every FIT/UNKNOWN carries a
concrete cracking plan and the instrument (`restore_pass`, `mapname_pass`,
`borders_pass`, SAMP1 cb0 mining, `census`, or a named NEW instrument).

**LEDGER2 pass (2026-07-06)** additionally opened and cross-checked, in the
shipped vanilla files at `<local-path> game\`:
`common/defines/00_graphics.lua` (border/tree/camera/HDR/light defines),
`common/defines/00_defines.lua` (river indices), `gfx/FX/constants.fxh` (night
ambient rig, water/snow constants), `gfx/FX/standardfuncsgfx.fxh` (night
mechanism), `gfx/posteffect_volumes.txt` (grading/HDR), `gfx/FX/{border,pdxmap,
pdxwater,restorescene,downsample_luminance}.shader`, `map/strategicregions/*.txt`
(snow date windows), `common/terrain/00_terrain.txt` (palette); and the render
code `map_viewer/src/{light_law,luminance_law,border_law,river_mesh_synthesis,
constants,shader_meta,d3d11_replay,d3d11_device}.rs`,
`replay_app/src/{render,font,main}.rs`. Every LEDGER2 reclassification quotes the
exact file:line it verified. The mod-compat vanilla-literal sweep is in
`documents/RENDER_PATH_AUDIT.md`.
