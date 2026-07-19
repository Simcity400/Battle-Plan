# GRADING_LAW — files-first map color grading (RestoreScene post-process)

Status: derived from shipped installed content, validated byte-exactly against a live
GPU capture, and wired as the **DEFAULT** runtime source in `replay_app`. The
installed content are the source of truth so mods work; captured evidence VALIDATES the
law and is never a runtime source of constants or textures.

**Default policy.** The files-first grade renders BY DEFAULT and fails closed. A
missing `--game`/resolver, missing `colorcorrection_*.tga`, or missing selection
key is a HARD ERROR: the product renderer does not silently downgrade to an
identity grade. `--no-postfx` and `--no-postfx-restore-full` are blocked at the
CLI because raw captured render targets / ungraded captures are not the game's
final on-screen image. `--postfx-validate-capture` remains validation-only: it
loads a captured restore bundle, reports file-law deltas, then exits.

All game-file reads go through the mod-aware `GameFileResolver`
(`map_viewer/src/mod_resolver.rs`, see `MOD_COMPATIBILITY.md`), never bare paths.

Historical ground-truth capture:
`C:\Users\Example\AppData\Local\hoi4_captures\restore_gate_1\` (vanilla,
captured 2026-07-06). That directory remains absent and has not been
reconstructed or relabelled.

> **RETAINED SUBSTITUTE RECOVERED (2026-07-19):** intact, hash-bound RDCs at
> `<local-path>`
> and `postfx_frame8467.rdc` now provide a rerunnable RestoreScene t0..t3 plus
> output boundary. The high-altitude frame independently matches the shipped
> far-range LUT byte-exactly, carries MiddleGrey 0.55, and re-clamps its
> scene-derived luminance to the captured f16 0.5. The close frame carries a
> distinct volume cube and MiddleGrey. See
> `re_evidence/rendering_laws/RESTORE_GATE_RETAINED/`. This is new substitute
> evidence, not proof that either RDC is the lost `restore_gate_1` frame.

Implementation: `replay_app/src/grading_law.rs` (law),
`replay_app/src/render.rs` (`restore_full_postfx_from_files`,
`restore_full_postfx_validate_capture`).

---

## Inferred capture state (date / time-of-day)

The `map_capture_session.json` manifest carries no save date. The state is
inferred from the recorded terrain scene constant buffer and the grade the
engine selected:

- Terrain scene CB0 (`map_frame_bundle.json` draw 0, `dx11_cb0`) vCamPos =
  **(x 2816, y 2958, z 1400)** — offsets 192 / 208 / 216. Camera height 2958 is
  near the 3000 max (`00_graphics.lua:1496`): a wide, high-altitude view.
- The engine selected the **day** grade values (see below), so the captured
  moment was **daytime**. No winter/snow cube was selected, so the region was
  **not in active winter**.
- Exact in-game calendar date is NOT recorded in this capture; only
  day/non-winter is inferable. (Reported honestly as a residual unknown.)

---

## LAW 1 — LUT composition

### Selection mechanism (cited)

`gfx/posteffect_volumes.txt` is the selection table:

- `posteffect_values` presets carry `lut = "<tga>"`, `tonemap_middlegrey`, and an
  optional `inherit = <name>`. Unset scalars fall back through the inherit
  chain; `#`-commented lines are inert.
  - `standard` (`:5-18`): `lut = "gfx/world/colorcorrection.tga"`,
    `tonemap_middlegrey = 0.55`, `BRIGHT_THRESHOLD = 0.9`.
  - `default` (`:21-33`): `inherit = standard`, overrides
    `lut = "gfx/world/colorcorrection_standard.tga"`; its `#tonemap_middlegrey`
    is commented, so it inherits 0.55.
  - `max_distance` (`:94-110`): `inherit = standard`,
    `lut = "gfx/world/colorcorrection_farrange.tga"`; its
    `#tonemap_middlegrey = 0.65` is commented, so it inherits 0.55.
- `posteffect_volumes { ... }` holds:
  - `posteffect_height_volume` entries chosen by camera height. `Base Highest`
    (`:345-353`, `height 1600`) maps day -> `max_distance`. `Base` (`:365-373`,
    `height 0`) maps day -> `default`.
  - `posteffect_volume` region boxes chosen by 3D `position`/`size` containment
    (e.g. `EuropeandRussia` `:601-612`, `posteffect_values_day = green`).

### Selection rule

If the camera point is inside a region `posteffect_volume` box, that volume's
value wins; otherwise the `posteffect_height_volume` with the greatest
`height <= camera.y` wins. Day/night (and winter) pick the matching
`posteffect_values_*` field.

For the captured frame: camera y = 2958 exceeds every region box's vertical
extent (e.g. `EuropeandRussia` spans y in `275 +/- 400 = -125..675`), so no
region volume applies. The tallest height volume `Base Highest` (1600) wins ->
`max_distance` (day) -> `lut = gfx/world/colorcorrection_farrange.tga`, MiddleGrey
inherited 0.55.

### Fit result (byte-exact)

Decoding all 25 shipped `gfx/world/*colorcorrection*.tga` (uncompressed type-2
24bpp, bottom-up, 1024x32) to top-down RGB and comparing to the captured
ColorCube (DXGI 88 B8G8R8X8, 131072 bytes) after BGRX -> RGB swizzle:

| cube | max per-channel delta vs captured |
|------|-----------------------------------|
| `colorcorrection_farrange.tga` | **0** |
| `colorcorrection_midrange.tga` | **0** |
| `colorcorrection_standard.tga` | **0** |
| `colorcorrection.tga` | 59 |
| next closest (`2colorcorrection_neutral`, ...) | >= 46 |

The three exact-match cubes are **byte-identical** (MD5 `655e09bb10be30198d50476613d8d52f`,
98348 bytes each). So the composed-from-files LUT matches the captured ColorCube
**byte-exactly (residual 0)** — no `lut_blender` blend is required; the blend
factor is effectively 0/1 onto a single cube.

Gates: `grading_law::gate_files_lut_matches_captured_color_cube_byte_exact`
(automated), and `replay_app --postfx-validate-capture` (CLI) both report
LUT max delta 0.

### Honest caveats (from adversarial review)

- The captured cube alone cannot prove the selected value NAME was
  `max_distance`: `farrange` / `midrange` / `standard` are byte-identical, so
  camera height only makes `max_distance` the plausible route.
- `gfx/FX/lut_blender.shader` exists and blends LUTs with `lerp`. The capture
  proves one shipped cube reproduces this frame; it does not prove the engine
  skipped an earlier blender pass in general (a blend of identical endpoints, or
  factor 0/1, is observationally identical).

---

## LAW 2 — adaptation / exposure

### Formula (cited)

`gfx/FX/restorescene.shader` `Exposure()` under `#define ADJUSTED_EXPOSURE`
(`:135-140`, sample+multiply at `:138-139`):

```
float AverageLuminance = tex2Dlod0(AverageLuminanceTexture, vec2(0.5)).r;
return inColor * (MiddleGrey / AverageLuminance);
```

`MiddleGrey` comes from the PostEffectBase CB (`gfx/FX/posteffect_base.fxh:1-11`),
sourced at runtime from the volume `tonemap_middlegrey` — **not** the define
`TONE_MAP_MIDDLE_GREY = 0.5` (`common/defines/00_graphics.lua:1041`). Corroborating
this: the captured CB `BrightThreshold = 0.9` matches `posteffect_volumes.txt`
`standard`'s `BRIGHT_THRESHOLD = 0.9` (`:16`), not the define default 0.4
(`00_graphics.lua:1035`).

With captured `MiddleGrey = 0.55` and captured `AverageLuminance = 0.5`, the
exposure scale is **0.55 / 0.5 = 1.1**.

### Where AverageLuminance comes from (cited)

Not engine-internal — the shipped luminance pass computes it. The **replay now
ports this chain and computes AverageLuminance from the rendered scene each frame
on the `--postfx-restore-full` path** (`map_viewer/src/luminance_law.rs`,
`render.rs::update_exposure_from_scene_luminance`). The constant `MIN_HDR_ADJUSTMENT`
floor was retired on that path (FIDELITY_LEDGER debt #2 closed; see below).

The shipped chain (ported line-by-line):

1. `gfx/FX/downsample.shader` PixelShader (bright pass, full-res MainScene ->
   half-res): writes, in the **alpha** channel,
   `logLuminance = log( max(0, dot(rgb, LUMINANCE_VECTOR)) + 0.0001 )` (`:70`).
   `LUMINANCE_VECTOR = (0.2125, 0.7154, 0.0721)` (`constants.fxh:119`).
2. `gfx/FX/downsample_luminance.shader` `PixelShaderDownsample` resamples that
   log-luminance from the bright-pass alpha into a dedicated luminance target
   (`#ifdef LUMINANCE_SAMPLE_ALPHA` -> `.a`; `:87-93`). *(The `LUMINANCE_SAMPLE_ALPHA`
   define is not itself present in the shipped `gfx/FX` tree — it is inferred from
   the bright-pass-writes-log-to-alpha wiring, not capture-proven. The port reads
   log-luminance directly from the linear scene color, which is what that alpha
   channel holds, so the inference does not affect the ported math.)*
3. `gfx/FX/downsample_luminance.shader` `PixelShaderGather`:
   ```
   vSum /= GatherSize_PixelSize.x * GatherSize_PixelSize.y;                  // :126 (mean of logs)
   float vCurrentLuminance = clamp(exp(vSum), MinHdr_MaxHdr.x, MinHdr_MaxHdr.y);  // :128
   // then eases LastLuminance toward vCurrentLuminance (tau/delta-time)     // :99-104
   ```

`MinHdr_MaxHdr` = `{ MIN_HDR_ADJUSTMENT 0.5, MAX_HDR_ADJUSTMENT 1.0 }`
(`00_graphics.lua:1038-1039`), eased at `HDR_ADJUSTMENT_SPEED 15`
(`00_graphics.lua:1040`).

So the fully-adapted (steady-state) average luminance is
`clamp( exp( mean( log( max(0, dot(rgb, LUMINANCE_VECTOR)) + 1e-4 ) ) ),
MIN_HDR_ADJUSTMENT, MAX_HDR_ADJUSTMENT )`.

**Where AvgLum comes from on each render path (post-EXPO1):**

- `--postfx-restore-full` (files-first): **per-frame measured.** The composite
  reads back the rendered HDR MainScene (`R16G16B16A16_FLOAT`), reduces it through
  the ported chain (2x2 box-average the linear color like the bright-pass tap,
  then log -> mean -> `exp` -> clamp), quantizes the result to an R16 half
  (matching the engine's 1x1 `R16_FLOAT` `AverageLuminanceTexture` precision), and
  writes it into the exposure constant's AvgLum field. MiddleGrey and every other
  grade constant are untouched.
- Default product render: same files-first path, because restore-full is default
  ON. If required evidence is missing, startup fails closed instead of using an
  identity grade.
- Validation-only disabled path: internal unit tests can construct the composer
  disabled to prove it is inert, but the product CLI no longer exposes that as a
  render mode.

**Temporal easing (honest):** `CalculateAdaptedLuminance` (`:99-104`) eases
`LastLuminance` toward `vCurrentLuminance`, converging to it over time. A static
dump has no previous frame, so the fully-adapted steady state IS
`vCurrentLuminance` — the port uses that and invents no tau/delta-time. A future
frame-sequence path would persist the previous adapted luminance and apply the
shipped easing (`HDR_ADJUSTMENT_SPEED = 15`).

**Order-of-operations fidelity (honest):** the bright pass computes
`log(luminance_of_box_average)`, not `mean(log(per-pixel luminance))`; since `log`
is nonlinear these differ. The port reproduces the engine's order (box-average the
color first, then one `log`). The one stage it does NOT reproduce is the engine's
bilinear resample of the half-res log field to the small gather grid before the
sum; averaging over all half-res texels instead is an exact substitute only for
uniform resample weights. Irrelevant whenever `exp(mean_log)` sits below the 0.5
floor (the captured corpus, ~6x margin).

### Clamp-branch validation (byte-exact vs capture)

The captured restore_gate_1 `AverageLuminance` texel was exactly **0.5 == half
0x3800 == MIN_HDR_ADJUSTMENT** — that frame CLAMPED to the floor. Running the
ported chain on the **captured restore_gate_1 MainScene** (the exact HDR scene
input the engine's restore pass graded, 1920x1080 `R16G16B16A16_FLOAT`) yields:

```
geo-mean luminance 0.082545 -> clamp[0.5,1.0] = 0.500 -> R16 0x3800 == captured 0x3800 (0.5)   [BYTE-EXACT]
```

The geometric mean (0.0825) is ~6x below the 0.5 floor, so the clamp engages with
a large margin and reproduces the captured f16 texel byte-for-byte
(`luminance_law::gate_captured_scene_reproduces_captured_average_luminance`). This
is the clamped branch and it is capture-validated.

### Unclamped branch — NOT capture-validated (open item)

The captured corpus only exercises the CLAMPED branch. Historical replay probes
against the now-removed CPU scaffold also never triggered the unclamped branch:
that political-map view was dim in **linear** space, with no bright HDR content
like sunlit 3D terrain with specular, so the measured
geometric-mean luminance stays ~0.05-0.16 across the whole map — probed at the
wide view and at bright desert/ice close-ups up to zoom 8 (all clamp to 0.5).
The unclamped (bright-scene) branch is therefore validated ONLY by:

- the ported math being line-cited to the shipped shader, and
- unit tests over synthetic bright scenes
  (`luminance_law::uniform_bright_scene_does_not_clamp`: a 0.7 scene -> AvgLum
  0.7, exposure scale drops from 1.1 to 0.786).

It is NOT presented as capture-validated. **Capture plan:** a bright close-up
`restore_pass` capture whose MainScene geometric-mean luminance exceeds 0.5 (e.g.
a sunlit 3D-terrain close-up with the full lighting/bloom the engine renders, not
the replay's flat political fill), recording both the MainScene SRV and the 1x1
`AverageLuminanceTexture`, so the ported reduction can be checked against a
captured unclamped AvgLum.

**Perf:** the reduction is ~12 ms/frame over 1920x1080 (release, CPU) plus the
fixed D3D11 staging readback; within the <100 ms speed-5 frame budget for static
dumps. A real-time sequence at speed 5 would move the reduction onto the GPU (a
small mip/downsample chain, as the engine does) if per-frame cost matters.

---

## Runtime rewire

`replay_app --postfx-restore-full` now composes the grade from FILES:

- LUT: `file_derived_grade` selects the volume value via LAW 1, loads the
  `colorcorrection_*.tga` through the resolver (mod-aware), swizzles to RGBA8.
- Constants: neutral HSV / ColorBalance (the base values carry no HSV/balance
  block); MiddleGrey from LAW 1; exposure = MiddleGrey / **per-frame measured**
  average luminance (LAW 2 — the ported `downsample_luminance.shader` chain over
  the rendered scene, no longer the `MIN_HDR_ADJUSTMENT` constant). The replay
  shader gates the LUT block on
  `RestoreColorBalanceUseLut.w > 0.5`, the replay-local mirror of the shipped
  shader's `#ifdef COLOR_LUT` (`restorescene.shader:114`). `HSV.w` is unused by
  both shaders, so the captured HSV.w=1 (a LUT-enable marker) has no render
  effect and neutral `[0,1,1,0]` reproduces the captured grade.
- Fail policy: with the grade DEFAULT ON, absence of `--game`/resolver or a
  required TGA / selection key is a hard startup error. There is no silent
  identity-grade fallback, and `--no-postfx-restore-full` is blocked at parse
  time.

The captured bundle is now a **validation-only** path:
`replay_app --postfx-validate-capture` loads the captured restore bundle and
reports file-law vs captured deltas (LUT max delta, MiddleGrey / AverageLuminance
diffs). It is a regression gate, never a render input. This decoupling means
grading applies to ANY session (and mods), even one whose capture recorded no
restore pass (e.g. `live_PAN2`).

### Validation output (restore_gate_1)

```
LUT max per-channel delta: 0
MiddleGrey: file 0.550000 vs captured 0.550000 (delta 0.000000)
AverageLuminance: file 0.500000 vs captured 0.500000 (delta 0.000000)
RESULT: LUT EXACT, MiddleGrey EXACT, AverageLuminance EXACT
```

---

## Known scope boundaries

- **Per-frame recompose LAW implemented + gated (GRADE2, 2026-07-09); render
  wiring pending.** `grading_law.rs` now carries the per-frame composition:
  `resolve_grade_per_frame` selects the height volume for the frame camera and
  folds the `fade_distance` cross-fade (shipped 200/200/0 at
  posteffect_volumes.txt:352/362/372) — the frame's grade is
  `lerp(lower_volume, winner, t)` with `t = (camY − winner.height) /
  winner.fade_distance`, continuous across the floor (`t = 0` equals the lower
  volume exactly). LUT blending uses the engine's own `lut_blender.shader`
  law (`lerp(base, blend, vBlendFactor2)`, lut_blender.shader:94-104) on the
  CPU (`blend_luts_to_rgba8`, endpoint-bit-exact); MiddleGrey rides the same
  lerp. The t-anchor (fade upward from the floor) is a documented law choice —
  the engine's C++ blend-factor source is UNTRACED; region-box `fade_distance`
  (spatial) is parsed but NOT implemented (hard containment retained).
  GATES: `gate_per_frame_recompose_is_byte_identical_at_captured_camera`
  (byte-noop at the captured camera 2958 vs the once-at-construction grade +
  a REAL shipped-file fade `t = 0.5` at height 1700);
  `gate_height_sweep_cross_fades_between_adjacent_volumes_with_file_values`
  (sweep across both floors, file values only);
  `blend_luts_reduces_to_endpoints_bit_exactly`.
  **Residual:** `MapRenderer::new` still composes ONCE at construction for the
  high-altitude view (`RESTORE_GRADE_VIEW_CAMERA_HEIGHT = 2958`); wiring
  `file_derived_grade_per_frame` into the render loop (LUT texture re-upload +
  bloom re-resolve on volume change) is the remaining step, tracked in
  FIDELITY_LEDGER §2.1.

## Residual engine-side unknowns

- Exact in-game calendar date of the capture (only day / non-winter inferable).
- The per-frame `AverageLuminance` easing trajectory (tau/delta-time) is
  engine-timed; only its clamp bounds and the steady-state target
  (`vCurrentLuminance`) are files-derived. The replay uses the steady state (the
  fully-adapted eye), correct for a static dump; a frame sequence would need the
  easing curve, which requires a multi-frame capture with timestamps.
- The **unclamped exposure branch** is not capture-validated and is not exercised
  by any current replay view (see LAW 2 "Unclamped branch"): needs a bright
  close-up `restore_pass` capture whose scene geometric-mean luminance exceeds
  0.5.
- Which specific value NAME the engine used is not uniquely provable from the
  captured cube (identical-content cubes), only the height-consistent route.
