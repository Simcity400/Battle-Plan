# XMAP10 bloom law

This document describes the current shipping product route. Historical CPU
prototypes and capture-only experiments are intentionally omitted.

## Authority and inputs

Bloom is resolved from the admitted effective VFS for the playback modset. The
runtime reads:

- `gfx/posteffect_volumes.txt` for the selected volume's inherited
  `BLOOM_WIDTH`, `BLOOM_SCALE`, `BRIGHT_THRESHOLD`, and `tonemap_middlegrey`;
- `common/defines/00_graphics.lua` for required fallback values, including
  `EMISSIVE_BLOOM_STRENGTH`;
- the exact per-frame grade program selected from viewer camera position,
  authenticated day/night state, and authenticated region-winter state.

No captured texture, constant buffer, hard-coded vanilla volume, recorder POV,
or selected-Present UI state is a product input. Missing, ambiguous, malformed,
or non-finite values fail the XMAP10 frame closed.

## Native frame order

`ProductMapCompositionSession` routes window, offscreen, and benchmark frames
through the same `ProductXmap10HandoffRuntime` and
`NativeXmap10PostFxExecutor`:

1. All map families render into the HDR `MainScene`
   (`R16G16B16A16_FLOAT`).
2. `postfx.bright_pass` downsamples MainScene to the half-resolution bright
   target and applies the shipped threshold/emissive law.
3. `postfx.luminance_downsample_chain` and `postfx.luminance_gather` update the
   authenticated temporal luminance resource.
4. `postfx.bloom.horizontal` and `postfx.bloom.vertical` produce
   `RestoreBloom`.
5. `postfx.restore_scene` consumes MainScene, RestoreBloom, the final LUT, and
   temporal luminance and writes the product target.

The executor seals target-bound receipts for all six stages. A missing stage,
wrong resource identity, wrong assignment, stale frame sequence, or incomplete
receipt rejects the frame.

## Blur kernel

The shipped blur is a separable Gaussian with one centre tap and four symmetric
linear-sampled tap pairs per axis. `BloomBlurKernel::from_gaussian` derives the
weights and fractional offsets from the selected `BLOOM_WIDTH` and
`BLOOM_SCALE`; the values are not tuned visually. Each pass applies the selected
scale, so the two-pass signal gain is `BLOOM_SCALE²`.

The bright pass and blur retain half-float targets. RestoreScene adds the
resulting bloom RGB to the linear MainScene before the remaining grade
operations. The CPU implementation in `map_viewer/src/bloom.rs` is a reference
and validation mirror; the shipping product executes the D3D11 path in
`map_viewer/src/xmap10_d3d_handoff.rs`.

## Discontinuities

Viewer presentation timing is an exact recurrence from the proved constructor
zero. The app retains a constant-size authenticated fold of every Presented
step, not the unbounded step history. Each new step advances that fold with
constant memory and work and is byte-equivalent to rebuilding the complete
ordered recurrence. A seek, date jump, scrub, backward step, forward jump, or
skipped-hour playback uses that bounded state; it does not carry a stale
accumulator or invent cadence for unpresented hours.

Temporal luminance cannot be reconstructed from absent intermediate pixels. On
a transport discontinuity the product therefore recreates MainScene and the
post-FX temporal resources and seals the existing engine-resource-reset
lifecycle. Consecutive frames continue from the prior completed temporal
receipt. Both policies are shared by window, offscreen, and benchmark routes.

## Evidence boundary

The preserved `restore_gate_1` capture validates the bright-pass/blur shape and
one set of vanilla parameters, including the captured kernel values. It is
validation evidence only and does not prove that those vanilla values apply to
another modset or per-frame volume. Product authority remains the admitted
effective VFS plus the exact XMAP10 receipts.

Current implementation:

- `replay_interface/native/replay_app/src/app.rs`
- `replay_interface/native/map_viewer/src/xmap10_d3d_handoff.rs`
- `replay_interface/native/map_viewer/src/xmap10_scene_cb0.rs`
- `replay_interface/native/map_viewer/src/bloom.rs` (reference/validation)
