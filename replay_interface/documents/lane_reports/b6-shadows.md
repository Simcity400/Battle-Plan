# LANE b6-shadows — engine-law ledger tail (plan item B6)

Branch `lane/b6-shadows`. Scope: close the ledger tail #16/#17/#20/#21 —
shadow/PCF path (priority 1, gates C7 h=50), camera interpolation shape,
occupation stripes, moon `.z`/height scale. **Files-first**: every law below is
derived from shipped shader/define text; no binary RE was needed (Ghidra project
left untouched — the shipped plain-text HLSL + defines fully specify these
laws). No-eyeball: gates are hand-computed shader-reference values + resolver
FILE-citation substring gates (the NIGHT2/AMBIENT1 standard), NOT pixel
validation of runtime textures.

New modules (all `mod` + `pub use` in `map_viewer/src/lib.rs`):
`shadow_law.rs`, `occupation_stripe_law.rs`, `camera_interp_law.rs`.

Build/test: `cargo build -p map_viewer` **green**; `cargo test -p map_viewer
--lib` **665 passed / 0 failed / 1 ignored** (baseline 639 passed → **+26** new
gates), identical with and without `BATTLE_PLAN_GAME_DIR` (FILE-citation gates skip
cleanly when the game dir is unset; all 26 pass against the shipped vanilla
files when set to `<local-path> game`).

No `REPLAY_DATA_REQUIREMENTS.md` / `EXTRACTION_NEEDS.md` change: none of these
laws add a recording-side dependency (shadows/camera/moon are engine+camera
state; the occupation-stripe DATA mapping rides the already-listed per-hour
controller feed, requirements item 5 — see §3 below).

---

## 1. Shadow / PCF path — ledger #21 UNKNOWN → FILE (priority 1, gates C7)

**The whole shadow term is file-derived.** The screen-space `ShadowMap` SRV
(pdxmap PS slot 12) carries the *final* shadow in `.r` (FoW in `.g/.b` — C8, not
touched). What was OPEN — how that `.r` is PRODUCED, filtered, faded, and
consumed — is now fully cited. Four passes:

1. **Light-space depth map (2048²).** Per-frame ortho projection
   `ShadowMapTextureMatrix` (`pdxmap.shader:209`, CB0 `standardfuncsgfx.fxh:2-33`)
   aimed along `LIGHT_SHADOW_DIRECTION = (-5,-8,5)`
   (`00_graphics.lua:769-771`; the `constants.fxh:177-179` comment's X=-8 is
   stale, the define X=-5 wins). **Single** map, NOT cascaded — the only cascade
   path (`CalculateShadowCascaded`, `pdxmesh.shader:499`) is commented out. Size
   pinned by the PCF texel `0.5/2048` (`shadow.fxh:17`).
2. **Producer** — `Effect terrainunlit` (`pdxmap.shader:501-505` →
   `PixelShaderTerrainUnlit:457-468`) writes `(CalculateShadow(vShadowProj,
   ShadowMap), FogColorFactor, FogAlphaFactor, 1)`. `CalculateShadow`
   (`shadow.fxh:6-42`) = **5-tap PCF cross kernel**: offsets `{−t,0}{0,+t}{+t,0}
   {0,−t}{0,0}` with `t=0.5/2048`; each tap `A < 0.99 && A < (receiverZ − 0.001)`
   → 0.1 (occluded) else 1.0; averaged /5. Result ∈ {0.1,0.28,0.46,0.64,0.82,1}.
   The `A < 0.99` guard suppresses self-shadowing of fully-lit texels; the 0.001
   bias suppresses acne.
3. **Blur** — `shadowblur.shader` `ShadowBlurHorizontal` then `…Vertical`:
   separable **3-tap gaussian on `.r` only**, preserving `.g/.b` (FoW) and
   forcing `.a=1`. Offsets/weights `vGaussianOffsetsWeights[2]` are the per-frame
   sigma-derived CB (like the bloom kernel — structure FILE, weights runtime).
4. **Consumer** — terrain/water/tree/river/border read the blurred screen-space
   shadow via `GetShadowScaled(SHADOW_WEIGHT_*, vScreenCoord, ShadowMap)`
   (`shadow.fxh:44-54`): `(1−w) + w·saturate(screen_r + (1−ShadowFadeFactor))`.
   Weights `constants.fxh:170-175`: TERRAIN/MAP/BORDER/TREE **0.7**, WATER
   **0.5**, RIVER **0.4**. Terrain adds a `max(…,0.1)` floor
   (`pdxmap.shader:359`). The scaled shadow multiplies **only** the direct
   sun/moon diffuse, gated by day+night (`CalculateSunLight`,
   `standardfuncsgfx.fxh:642-652`) — never the ambient term.

**Camera-height fade (was engine/CAPTURE → FILE).** `ShadowFadeFactor`
(CB0 scalar `standardfuncsgfx.fxh:19`) `= saturate((DRAW_SHADOWS_CUTOFF 400 −
camY) / DRAW_SHADOWS_FADE_LENGTH 50)` (`00_graphics.lua:860-861`). This matches
the previously mined runtime value `clamp((400−h)/50)` (col3-political memory)
and the shipped define names/values are literally "shadows". **Consequence
(key visual law): shadows are fully faded out above 400 wu camera height** — so
they only matter at close zoom, exactly the C7 h=50 target band.

**Port + gates** — `shadow_law.rs`, 16 gates, all pass:
`gate_pcf_tap_offsets_match_shadow_fxh`, `gate_pcf_all_lit_returns_one`,
`gate_pcf_all_shadowed_returns_floor`, `gate_pcf_partial_two_shadowed` (0.64),
`gate_pcf_099_self_shadow_guard`, `gate_pcf_bias_prevents_acne`,
`gate_shadow_fade_factor_endpoints` (h=50→1, 375→0.5, 400→0),
`gate_shadow_fully_faded_above_cutoff` (h=500 → shadow term 1.0),
`gate_get_shadow_scaled_full_shadow_near_camera` (0.37),
`gate_get_shadow_scaled_lit_is_one`, `gate_terrain_floor_binds_only_below_min`,
`gate_shadow_gated_by_daynight`, `gate_shadow_blur_3tap_hand_computed` (0.7),
`gate_vanilla_shadow_constants_are_shipped` (resolver substring citation of
`constants.fxh`/`shadow.fxh`/`00_graphics.lua`/`pdxmap.shader`).

**Open remainder + exact closure path.** RUNTIME residual only:
`ShadowMapTextureMatrix` (per-frame ortho fit to the visible scene along the
fixed light dir) and the light-space depth-map **contents** (which terrain
mountains/meshes cast). These are engine per-frame state, not fitted constants;
pixel parity for them is capture-gated. **Closure:** a low-h (h≤250) mined
RenderDoc frame that captures the light-space depth pass RT + the screen-space
`ShadowMap` pre-blur; diff the ported `CalculateShadow` against the engine's
producer `.r` at matching `vShadowProj`. The oracle already holds low-h frames
WITH shadows — the missing piece is the intermediate depth/producer RT, not a
new session. Until then the term is validated as a shader port, not pixel-exact.

---

## 2. Camera interpolation shape — ledger #17 residual FIT → FILE (gates E2)

`CAMERA_INTERPOLATION_SPEED = 0.19` (`00_graphics.lua:1498`) is the interpolation
function: **exponential ease-out**, `current = lerp(current, target, 0.19)` per
tick, so remaining distance decays `0.81^n` — asymptotic, no overshoot, not a
fixed-time or bezier curve. Zoom is **distance-proportional**: `CAMERA_ZOOM_SPEED
=50` (`:1187`), `CAMERA_ZOOM_KEY_SCALE=0.02` (`:1188`),
`CAMERA_ZOOM_SPEED_DISTANCE_MULT=6.0` (`:1189`, "when max zoomed out, zoom-in
speed = 100% of the mult") → geometric (constant-perceptual-rate) zoom. Clamps
re-cited: `CAMERA_MIN_HEIGHT=50`, `CAMERA_MAX_HEIGHT=3000` (`:1495-1496`).

Port + gates — `camera_interp_law.rs`, 5 gates:
`gate_interp_step_eases_toward_target` (100→300 @0.19 = 138),
`gate_exponential_decay_shape` (0.81, 0.6561, monotone no-overshoot),
`gate_height_clamp`, `gate_zoom_is_distance_proportional`,
`gate_vanilla_camera_defines_are_shipped`.

**Open remainder.** Only the `dt`/fixed-tick normalization of the 0.19
coefficient and the key-pan speed constant are engine C++ — these rescale the
time-constant, not the curve family. **Closure:** read the engine camera-update
tick from `hoi4.exe` (or accept the shipped per-tick coefficient as the playback
default — the shape is exact regardless of tick rate).

---

## 3. Occupation stripes — ledger #20: render law FILE (already), now PORTED

`CalculateOccupationMask` (`standardfuncsgfx.fxh:965-977`): diagonal cosine
grating at angle `t=π/8` (22.5°), spatial period `SEC_MAP_TILE=6000`
(`constants.fxh:216`), camera fade `cam_distance(300,1200)` lifted by
`camDist·1.5` then `smoothstep(0,1, ·1.7) · lerp(1.0, 0.3, camDist)` — i.e.
stripes go near-solid AND drop to 0.3 intensity when zoomed out. Blend
`secondary_color_mask` (`:979-987`): `occ = mask·colorMask.a`,
`color = lerp(color, colorMask.rgb, occ)`, `bloom *= (1−occ)`.

Port + gates — `occupation_stripe_law.rs`, 6 gates:
`gate_stripe_crest_near_camera_is_full` (1.0),
`gate_stripe_trough_near_camera_is_zero` (0.0),
`gate_stripe_far_camera_solid_at_reduced_intensity` (crest→0.3),
`gate_secondary_blend_and_bloom_suppression`, `gate_cam_distance_clamped_ramp`,
`gate_vanilla_stripe_law_is_shipped`.

**Open remainder = DATA mapping, not render law.** The province-id →
`colorMask.rgb/.a` (which provinces stripe, in what color) is recording-fed:
it derives from owner≠controller + the specific secondary map mode. The
controller feed is **already** requirements item 5 (per-hour owner/controller),
so **no new extractor requirement** — the stripe color is computed replay-side
from controller vs owner, exactly like the primary fill. Validation stays
vacuous until a capture fires the stripe path (vanilla secondary SRV is
all-transparent); this is the pre-existing ledger #20 UNKNOWN, unchanged.

---

## 4. Moon `.z` / height scale — ledger #16/#19 (confirmed, no change)

- **Moon `.z=1448` stays UNKNOWN.** Both colliding file identities re-confirmed
  in the shipped defines: `SUN_LATITUDE=848` + `MOON_HEIGHT=600` = 1448 AND
  `MAP_SIZE_Y(2048) − MOON_HEIGHT(600)` = 1448 (`00_graphics.lua:1043/1048`).
  Vanilla cannot break the tie. **Closure:** a mod retuning `MOON_HEIGHT` or
  `MAP_SIZE_Y` (unchanged from LEDGER2's plan).
- **Terrain vertex height scale = 0.01** is file-cited (`pdxmap.shader:204`,
  `vHeight *= 0.01f`) — flagged for the orchestrator: ledger line ~491 lists a
  "0.1·height" scale for border/tree/river world height; the terrain-pass scale
  is 0.01, so the two are different constants (per-geometry), not a conflict, but
  worth reconciling in a follow-up. No code change made here (single constant,
  low value; item #19's "world-normal elevation scale" is the HeightNormal
  lighting normal, a separate capture-gated item).

---

## Ledger updates (this change)

- Summary table: Light rig **7/3/3/4 → 8/3/3/3**; Camera **3/1/1/0 → 4/1/1/0**;
  Totals **FILE 78→80, UNKNOWN 46→45** (net +2 FILE / −1 UNKNOWN).
- §2.4: `ShadowMap` row re-pointed; new **Shadow term pipeline = FILE** row.
- §2.8: occupation-stripe semantics row cites the new module + gates.
- §3 tail: #16 operands confirmed; #17 interp shape FILE; #20 render law ported;
  #21 UNKNOWN → FILE (shader port), runtime residual scoped.

## Gate summary (numbers)

| Item | Module | Gates | Status |
|---|---|---|---|
| Shadow/PCF #21 | `shadow_law.rs` | 16/16 pass | FILE (port); runtime matrix/depth capture-gated |
| Camera interp #17 | `camera_interp_law.rs` | 5/5 pass | FILE (shape); dt-norm engine residual |
| Occupation #20 | `occupation_stripe_law.rs` | 6/6 pass | FILE render law; DATA mapping UNKNOWN (item 5 feed) |
| Moon.z #16 | — | — | UNKNOWN confirmed (mod-retune closure) |

`map_viewer --lib`: **665 passed / 0 failed / 1 ignored** (+26).
