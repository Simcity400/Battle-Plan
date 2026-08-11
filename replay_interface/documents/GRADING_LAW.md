# XMAP10 grading law

This document describes the current shipping product route. Superseded
single-camera, static-LUT, CPU-only, and capture-derived runtime descriptions
have been removed.

## Per-frame authority

The product loads the admitted mod-aware effective VFS once per composition
session:

- `gfx/posteffect_volumes.txt` supplies inherited grade and bloom values;
- `common/defines/00_graphics.lua` supplies the exact grading defines;
- the effective region-weight manifest joins camera position to authenticated
  passive province snow for winter selection;
- every selected LUT path is admitted through `Xmap10ContentClosure` and the
  playback content manifest.

For each frame `ProductXmap10HandoffRuntime` computes:

1. viewer camera position from the authenticated XMAP03 camera;
2. exact-a729 day/night factor from the XMAP09 sun direction, map dimensions,
   and XMAP10 viewer-owned day/night gate;
3. region-winter factor from the effective region weights and the current
   passive global snow carrier;
4. the nested region-winter, day/night, height, and spatial grade program via
   `PostEffectVolumeTable::resolve_exact_grade_program`;
5. the selected LUT content, MiddleGrey, bloom values, and other constant-buffer
   fields used by the native executor.

The replay never guesses a calendar date, camera height, active region, volume
name, LUT, blend factor, or winter state. Missing or contradictory authority is
a feature-blocking XMAP10 error, not an identity-grade fallback.

## Native execution

The shared window/offscreen/benchmark compositor executes this ordered D3D11
chain:

1. LUT copy/blend program to a sealed final color-cube content identity;
2. bright pass and luminance downsample chain;
3. temporal luminance gather;
4. horizontal and vertical bloom;
5. RestoreScene using MainScene, RestoreBloom, final LUT, and temporal
   luminance.

Every program, target, constant buffer, resource transition, final LUT content,
and output target participates in the XMAP10 completion receipts. The frame is
not complete unless RestoreScene is the aggregate tail and consumes the exact
final LUT identity.

Consecutive frames continue temporal luminance from the prior completed receipt.
A seek, jump, scrub, backward step, or skipped-hour playback recreates MainScene
and the post-FX temporal resources and seals the engine-resource-reset path; it
does not reuse a pre-seek luminance value or synthesize missing intermediate
pixels.

## Formula sources

The shader-visible laws follow the effective shipped files:

- RestoreScene exposure uses `MiddleGrey / AverageLuminance`;
- luminance is reduced through the shipped bright/downsample/gather sequence and
  retained in the temporal luminance target;
- LUT application and blending use the selected effective LUT program;
- bloom uses the per-frame selected parameters described in
  `BLOOM_LAW.md`.

These are executed on the native product path. The CPU grading, luminance, and
bloom modules remain reference/validation implementations and are not authority
for product pixels.

## Evidence boundary

The preserved `restore_gate_1` and retained RenderDoc observations validate one
vanilla RestoreScene boundary, including a byte-matching LUT and a clamped
average-luminance sample. They are not runtime inputs and do not make a universal
claim about other cameras, frames, mods, LUT transitions, or the unclamped
branch. Those cases are governed by the effective-file law and native receipts.

Current implementation:

- `replay_interface/native/replay_app/src/app.rs`
- `replay_interface/native/map_viewer/src/grading_law.rs`
- `replay_interface/native/map_viewer/src/posteffect_region_weights.rs`
- `replay_interface/native/map_viewer/src/xmap10_d3d_handoff.rs`
- `replay_interface/native/map_viewer/src/bloom.rs` and
  `luminance_law.rs` (reference/validation)
