# CAP4-MINING lane report (2026-07-09)

Offline mining of the preserved `session_WAVE1_CAP4` corpus
(`C:\Users\Example\hoi4_capture_corpus_preserved\session_WAVE1_CAP4_rounds`,
read-only) for three capture-gated residuals. No game session, no Ghidra.

Corpus structure per round (`fow_off_observer_front`, `fow_on_pov_front`,
`counters_close`): map terrain/water/rivers/trees draw payloads (IA/CB/SRV),
plus 1920×1080 `R16G16B16A16_FLOAT` `om_pre_draw`/`om_post_pass` render targets
(16,588,800 B each, headerless raw). `counters_close` is budget-truncated (no RT
payloads). **No `restore_pass`, no UI/furniture/sprite pass anywhere in the
corpus.**

---

## TARGET 1 — B12 sea base look → CLOSED (clean referee established)

**Verdict:** the no-FOW `fow_off_observer_front` round yields a provably
fog-clean open-sea referee. The old "pale grey-green sea" was FOW contamination;
there is no separate pale political-sea law.

Method (`map_viewer/src/cap4_sea_referee.rs`): sea mask = pixels the water draw
changed (`|post−pre|₁ > 1e-3` on the water pass RTs); measure post-water HDR sea
colour + water `ShadowMap` FoW-alpha (`.b`, byte 0) over that mask.

Gate numbers (`gate_wave1_cap4_no_fow_sea_base_referee`):
- Sea mask: **1,009,749 px** (48.7% of frame).
- Sea `FogAlphaFactor` byte-mean **0.48/255 ≈ 0** → fow `lerp` (`pdxmap.shader:388`)
  is a byte no-op there → **sea is fog-clean** (the key cleanliness proof). Shadow
  term byte-mean 254.6/255 (unshadowed).
- Clean open-sea base HDR = **(0.1007, 0.1314, 0.1919)** linear — dark reflective
  **blue** (B > G > R), NOT pale grey-green.
- Cross-check vs independent `live_PAN2` clean sea **(0.079, 0.101, 0.145)**:
  same blue hue shape, ~1.3× uniform brightness delta (different camera/hour, not
  a hue change) → same water law.

**Residual (not offline-closable):** byte-exact pixel parity of OUR pdxwater
output vs this referee needs the water D3D11 pass re-rendered with the captured
SRVs/CBs (a render op, not a mining reduction), decomposing fresnel / reflection
cube / refraction arms. The referee itself is now clean and established.

Ledger: "Political sea BASE look" row reclassified **UNKNOWN → CAPTURE**
(clean referee; full parity open). Summary GB layer 4/1/1/1 → 4/2/1/0; totals
CAPTURE 24→25, UNKNOWN 46→45. Plan §4 CAP8 updated.

---

## TARGET 2 — B11 VP diplomatic color columns 1/3 → NOT CLOSABLE (sharpened ask)

**Verdict:** the CAP4 corpus does not contain the VP-icon draw pass, so the
FURN-REL law (columns 0/2/4 reachable, 1/3 unreached) cannot be capture-confirmed
here. It stays static-RE-only.

Evidence: the `fow_on_pov_front` / `fow_off_observer_front` rounds captured ONLY
the map terrain/water/rivers/trees passes. Corpus-wide search found NO
`onmap_victorypoints_strip` / VP-icon / map-furniture / UI / sprite draw or SRV;
every captured SRV declared name is a terrain/water/tree/river map. "Enemy
territory + VPs on screen" (plan CAP2) describes what was visually on screen, not
what the capture recorded — the on-map icon pass was not in the recorded draw set.

**Sharpened capture ask:** a player-POV frame that **captures the on-map
furniture / VP-icon draw pass** (draws binding `GFX_onmap_victorypoints_strip` /
`onmap_victorypoints_strip.dds`, with the per-VP strip frame index or UV
sub-rect), spanning at-war-enemy / self / same-faction / neutral VP owners so
columns 0/2/4 each appear and 1/3 are confirmed absent, byte-exact.

Ledger "Diplomatic color column" row + plan §4 CAP2 updated with the CAP4 miss
and the sharpened ask.

---

## TARGET 3 — B9 unclamped exposure branch → NOT CLOSABLE (sharpened ask)

**Verdict:** CAP4 never exercised the unclamped exposure branch (raw
geometric-mean scene luminance > 0.5). Both frames clamp to the 0.5 floor.

Evidence: CAP4 captured NO `restore_pass` bundle (no MainScene /
AverageLuminanceTexture payload anywhere). Reducing the last captured map-pass
HDR RT (trees `om_post_pass`, the MainScene proxy) through the shipped luminance
chain (`luminance_law::scene_average_luminance`) gives geometric-mean luminance:
- `fow_off_observer_front`: **0.23671** → clamps to 0.5.
- `fow_on_pov_front`: **0.23654** → clamps to 0.5.
- Only ~0.35% of pixels exceed 0.5 linear (max ~0.62); the unclamped branch needs
  the *geometric mean* > 0.5.

Gate: `luminance_law::gate_wave1_cap4_scene_clamps_to_floor_unclamped_branch_unexercised`.

**Sharpened capture ask:** a frame that FILLS the view with high-albedo terrain
(snow / desert) at close zoom with minimal dark ocean/shadow so
`exp(mean(log(luminance)))` > 0.5, AND that dumps the `restore_pass` bundle
(MainScene + AverageLuminanceTexture) so the branch is both triggered and
byte-validatable.

Ledger "Exposure / AverageLuminance" row + burn-down #2 context + plan §4 CAP4
updated with the CAP4 miss and the sharpened ask.

---

## Build / test

- `cargo build -p map_viewer` — green (only pre-existing `light_law`/`test_support`
  dead-code warnings; none from new code).
- `cargo test -p map_viewer --lib` — **634 passed, 0 failed, 1 ignored** (the 3
  new gates included; corpus gates run against the preserved corpus, skip-if-absent).

## Files touched (all in `map_viewer` / documents; no extractor/replay_core/fixtures/corpus)

- `map_viewer/src/cap4_sea_referee.rs` (new module + gate) — Target 1.
- `map_viewer/src/luminance_law.rs` (new CAP4 gate in tests) — Target 3.
- `map_viewer/src/lib.rs` (`mod` + `pub use` for the new module).
- `documents/FIDELITY_LEDGER.md` (sea row reclass + summary/totals + exposure &
  diplomatic-column row notes).
- `documents/REPLAY_PRODUCTION_PLAN.md` (§4 CAP2 / CAP4 / CAP8 rows).
