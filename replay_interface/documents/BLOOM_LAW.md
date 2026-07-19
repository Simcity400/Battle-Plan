# the game Map Bloom Chain Law

Status: files-first, validated byte-close against one captured
MainScene->RestoreBloom pair. Every runtime stage is derived from shipped
shader/define/config files; the captured `restore_gate_1` pair is validation
only (it is never a runtime input). The CPU port is
`replay_interface/native/map_viewer/src/bloom.rs`; the validator is
`map_viewer/src/bin/dump_bloom_validation.rs`.
> **STORE LOST (audited 2026-07-19):** the `restore_gate_1` capture no longer
> exists on this machine. Recorded validation results stand as the evidence of
> record; re-running the validator requires a fresh capture. See
> `GRADING_LAW.md` and the store index in
> `lane_reports/rendering_laws/PROGRAM_STATUS.md`.

Vanilla game root used for citations:
`<local-path> of Iron IV`. All game-file reads go
through the mod-aware `GameFileResolver`, so a mod that ships a different
`gfx/posteffect_volumes.txt`, `common/defines/00_graphics.lua`,
`gfx/FX/downsample.shader`, or `gfx/FX/bloom.shader` retunes the chain
automatically.

## 0. Where the bloom sits in the frame

The map draws its HDR scene into a `R16G16B16A16_FLOAT` "MainScene" target,
then the post chain runs:

1. `gfx/FX/downsample.shader` — bright-pass + 2x downsample of MainScene into a
   half-resolution bloom buffer.
2. `gfx/FX/bloom.shader` — separable Gaussian blur of that buffer (horizontal
   pass then vertical pass), producing the "RestoreBloom" texture.
3. `gfx/FX/restorescene.shader` `Effect RestoreBloom` — reads MainScene and the
   RestoreBloom texture and composites `bloom + scene`. This lane implements
   stages 1-2 (the bloom chain that *produces* RestoreBloom); the composite in
   stage 3 is the color-grade lane's territory (see `post_effect.rs`).

The captured `RestoreBloom` SRV (`restore_gate_1`, 960x540, DXGI 10) is the
**output** of stage 2, bound as the input to stage 3. Its alpha is `1.0`
everywhere, which is `bloom.shader`'s `return float4(color, 1.0)`
(`gfx/FX/bloom.shader:101`) and not the downsample output (which writes a
negative log-luminance into alpha). That pins RestoreBloom as the blurred
buffer, not the raw bright pass.

## 1. Shipped parameters (source of truth)

The bloom parameters actually in effect come from the active posteffect volume,
`gfx/posteffect_volumes.txt`, `name = standard`
(`gfx/posteffect_volumes.txt:6`):

| Parameter | Value | Citation |
| --- | --- | --- |
| `BLOOM_WIDTH` | `1.5` | `gfx/posteffect_volumes.txt:14` |
| `BLOOM_SCALE` | `0.9` | `gfx/posteffect_volumes.txt:15` |
| `BRIGHT_THRESHOLD` | `0.9` | `gfx/posteffect_volumes.txt:16` |
| `tonemap_middlegrey` | `0.55` | `gfx/posteffect_volumes.txt:17` |

`common/defines/00_graphics.lua` carries the same keys but with the **day**
`BRIGHT_THRESHOLD = 0.4` (`00_graphics.lua:1035`); the `0.9` value is the
commented night value on `00_graphics.lua:1036`. The posteffect volume is the
authoritative override and matches the captured PostEffectBase constant buffer
exactly (`BrightThreshold = 0.9`, `MiddleGrey = 0.55`), so the resolver reads
the volume first and falls back to the defines only if a key is absent.
`EMISSIVE_BLOOM_STRENGTH = 1.0` is only in the defines file
(`00_graphics.lua:1037`), so it always comes from there. Resolution logic:
`bloom.rs::resolve_bloom_params_for_volume`.

**Which volume.** `standard` is the `inherit` root and the correct volume for the
captured `restore_gate_1` frame and the default map view, so
`resolve_bloom_params` uses `BLOOM_DEFAULT_VOLUME = "standard"`. It is **not**
universal: the same shipped file has active (non-commented) bloom overrides for
other volumes — e.g. `green_night` (`gfx/posteffect_volumes.txt:147`),
`green_night_asia` (`:168`), `medither_night` (`:331`). A frame rendered under a
different active volume must resolve that volume by name via
`resolve_bloom_params_for_volume`; supplying the frame's active volume is the
render-wiring job's responsibility.

## 2. Stage 1 — bright pass + downsample

Source: `gfx/FX/downsample.shader`. `MainScene` is sampled Linear/Linear,
`MipFilter = "None"`, `AddressU/V = "Clamp"` (`downsample.shader:11`-`:19`). The
pixel shader is:

```
float4 vColor = tex2Dlod0( MainScene, Input.uv );                       // :64
float vMax = max(0, max( max( vColor.r, vColor.g ), vColor.b ) - BrightThreshold ); // :66
vMax /= (0.5 + vMax);                                                   // :67
vMax += vColor.a * EmissiveBloomStrength;                               // :68
float logLuminance = log(max(0.0, dot(vColor.rgb, LUMINANCE_VECTOR)) + 0.0001f);    // :70
return float4( vColor.rgb * vMax, logLuminance );                      // :72
```

`LUMINANCE_VECTOR = float3(0.2125, 0.7154, 0.0721)`
(`gfx/FX/constants.fxh:119`).

**Downsample factor.** The bloom buffer is `960x540` = MainScene / 2 in each
axis. Evidence: the captured RestoreBloom is `960x540`, and the captured
`PostEffectBase.InvDownSampleSize = [1/960, 1/540]` while
`InvWindowSize = [1/1920, 1/1080]`. At an exact 2x reduction, hardware bilinear
(Linear MinFilter, no mip) sampled at each destination texel center lands on the
shared corner of a 2x2 source block, so the fetch is a **2x2 box average** of
the four source texels. The CPU port implements this directly
(`bloom.rs::bright_pass_downsample`); a shifted-box or vertical-flip alternative
was measured far worse against the captured pair. `FIX_FLIPPED_UV` is identity
on DX11 (`gfx/FX/defines_hlsl_dx11.fxh`) and the DX9 half-pixel branch
(`downsample.shader:49`) is inactive, so no half-texel correction is applied.

The bright-pass RGB is `rgb * vMax`; alpha carries `logLuminance` (retained by
the port for a faithful intermediate, but discarded by the blur).

## 3. Stage 2 — separable linear-sampled Gaussian blur

Source: `gfx/FX/bloom.shader`. `BloomSource` is Linear/Linear/Linear,
`AddressU/V = "Clamp"` (`bloom.shader:11`-`:19`). The blur constant buffer is
`ConstantBuffer( 2, 39 )` with `InvBloomSize`, `Axis`, `Weight0`, `float4
Weights`, `float4 Offsets` (`bloom.shader:40`-`:47`).

The vertex shader builds one center UV plus four symmetric offset pairs, where
the per-axis step is `vAxisOffset = InvBloomSize * float2(Axis, 1 - Axis)`
(`bloom.shader:68`) so `Axis = 1` is a horizontal pass and `Axis = 0` a vertical
pass. The pixel shader accumulates a center tap plus four symmetric tap pairs
(`bloom.shader:94`-`:99`):

```
color  = tex(center).rgb * Weight0;
color += Weights[i] * ( tex(center + off_i).rgb + tex(center - off_i).rgb );  // i = 0..3
return float4(color, 1.0);
```

That is **9 texture samples per pass** (1 center + 4 pairs), run twice (H then
V). The offsets are fractional (`Offsets[i]` are non-integer texel
displacements), i.e. this is the standard **linear-sampled Gaussian**: adjacent
discrete Gaussian taps are folded into one bilinear fetch.

### 3.1 The weights/offsets are derived from `BLOOM_WIDTH` and `BLOOM_SCALE`

The numeric `Weight0`/`Weights`/`Offsets` are **not** stored in any shipped text
file — the engine computes them host-side from `BLOOM_WIDTH` (Gaussian sigma)
and `BLOOM_SCALE` (per-pass gain) and uploads them into `ConstantBuffer(2,39)`.
The port reconstructs them exactly (`bloom.rs::BloomBlurKernel::from_gaussian`):

1. Discrete Gaussian `w(x) = exp(-x^2 / (2*sigma^2))`, `sigma = BLOOM_WIDTH`,
   for `x = 0..8`.
2. Normalize over the full symmetric 17-tap kernel
   (`tap0 + 2*sum(tap1..8)`).
3. Fold each adjacent discrete pair `(1,2),(3,4),(5,6),(7,8)` into one
   linear-sampled tap: combined weight `w_a + w_b`, offset
   `(a*w_a + b*w_b)/(w_a + w_b)`.
4. Multiply every weight by `BLOOM_SCALE` so the per-pass weight sum equals
   `BLOOM_SCALE` (not 1). Two passes therefore multiply the signal by
   `BLOOM_SCALE^2 = 0.81`.

For `sigma = 1.5`, `scale = 0.9` this reproduces the engine's own captured
`ConstantBuffer(2,39)` to 7 significant figures:

| Term | Derived from files | Captured engine CB |
| --- | --- | --- |
| `Weight0` | `0.239365370` | `0.239365369` |
| `Weights[0]` | `0.290074851` | `0.290074825` |
| `Weights[1]` | `0.039232172` | `0.039232172` |
| `Weights[2]` | `0.001005666` | `0.001005666` |
| `Weights[3]` | `4.627006e-6` | `4.627004e-6` |
| `Offsets` | `[1.339244, 3.174285, 5.079846, 7.034445]` | `[1.339244, 3.174285, 5.079846, 7.034445]` |
| per-pass sum | `0.900000` | `0.900000` |

(The captured CB is used here only to confirm the derivation; the runtime path
never reads it.) Because the kernel is a pure function of the two shipped
constants, a mod that changes `BLOOM_WIDTH`/`BLOOM_SCALE` gets the correct
kernel with no capture.

**Sigma-generality caveat.** The match is verified at `sigma = 1.5`. At that
sigma, a discrete radius of 8 and an effectively-infinite normalization are
indistinguishable (tap 8 already contributes `~5e-6`). For a much wider mod sigma
(e.g. `~3`), the radius-8 truncation and the exact normalization convention could
drift by more than half-float noise. Closing that fully would need a capture of a
frame with a retuned `BLOOM_WIDTH`. The port fixes the structure the shader
mandates (center + four pairs = 9 samples), so the only open freedom is the
discrete radius/normalization the engine used to fill those nine slots.

### 3.2 Sampling and edge behavior

Each blur pass samples along one axis with bilinear interpolation at the
fractional offset and **Clamp (edge-replicate) addressing**
(`bloom.rs::sample_bilinear_axis`). Between the horizontal and vertical passes
the engine writes a `R16G16B16A16_FLOAT` render target; the port re-quantizes
the intermediate through half-float to mirror that
(`bloom.rs::quantize_half`). The final output alpha is forced to `1.0`
(`bloom.shader:101`).

## 4. Stage 3 — composite (not in this lane)

`gfx/FX/restorescene.shader` `Effect RestoreBloom` (`restorescene.shader:236`,
`Defines = { "BLOOM" }` at `:240`) reads the RestoreBloom texture at
`Input.uv * BloomToScreenScale` (`restorescene.shader:211`) and adds it to the
scene: `color = bloom.rgb + color` (`restorescene.shader:212`). For the captured
frame `BloomToScreenScale = [1, 1]`. This additive composite lives in the
color-grade lane (`replay_app/src/post_effect.rs` / `render.rs`) and is out of
scope here; this document/law only produces the RestoreBloom texture that stage
consumes.

## 5. Validation

`dump_bloom_validation` runs the file-derived chain on the captured MainScene
(`restore_gate_1`) and compares to the captured RestoreBloom, RGB per channel,
at half-float precision:

- **99.26 %** of channels are half-float **exact** (bit-identical after
  re-quantizing to `R16G16B16A16_FLOAT`): 1,543,688 / 1,555,200.
- **100.0 %** of channels are within one 8-bit level (`1/255`).
- max abs error `8.06e-4` (`0.21` of an 8-bit level, `15` half-float ULPs);
  mean abs error `3.86e-8`.
- The chain is deterministic: two runs produce byte-identical stats and image
  outputs.

### Divergence localization (byte-exact not fully reached)

The gate target is byte-exact. The residual `0.74 %` of non-exact channels is
**not** a stage error. `dump_bloom_validation` records the localization bins in
`bloom_validation_stats.json`:

- The kernel matches the engine's own constant buffer to 7 sig figs (Section
  3.1), so the blur math is correct, not guessed.
- In bloom **background** regions (`513,546` pixels, captured magnitude
  `< 1e-6`) the max error is `9.9e-8` — the bright pass and blur produce exact
  zeros where the engine does.
- The entire residual lives in high-magnitude bloom **cores** (`145` pixels,
  magnitude `> 0.01`): max error `8.06e-4`, mean `1.6e-4`, `<=15` half-float
  ULPs. That is the irreducible floor from D3D11 bilinear-filter arithmetic
  ordering and the `R16G16B16A16_FLOAT` interpass round-trip, which a full-float
  CPU port cannot reproduce bit-for-bit.

Falsifier tested (Codex-proposed): re-quantizing the bright-pass output through
half-float *before* the horizontal blur — modeling the downsample render target
as `R16G16B16A16_FLOAT` — does **not** improve exactness (max error moves from
`8.06e-4` to `8.99e-4`, half-exactness unchanged). So the residual is genuinely
the filter/half-float floor, not a missing render-target round-trip; the port
keeps the bright pass full-float into the first blur pass and only round-trips
between the two blur passes.

So the divergence is localized to half-float/filter rounding in the blur, not to
the bright-pass knee, the downsample, the scale, or the kernel shape.

## 6. Fail-closed behavior

Every unknown fails closed with an explicit error: a missing shader/define/volume
source, a `posteffect_volumes.txt` volume with no `standard` block or a cyclic
`inherit`, a non-`R16G16B16A16_FLOAT` MainScene/RestoreBloom, a payload whose
size or row pitch does not match its declared dimensions, a non-finite texel, an
odd MainScene dimension, or a file-derived `BrightThreshold`/
`EmissiveBloomStrength` that disagrees with the captured PostEffectBase CB (which
would mean the resolved installed content describe a different frame than the capture).

## 7. Follow-up: render wiring

The render-wiring job (after this lane and the color-grade lane merge) needs:

- Call `map_viewer::resolve_bloom_params(resolver)` once per modset to get
  `BloomParams`, then `run_bloom_chain(main_scene, params)` per frame; the
  result's `.bloom` is the RestoreBloom SRV for the restorescene composite.
- The bloom buffer is MainScene / 2 in each axis; the composite samples it at
  `uv * BloomToScreenScale` (`[1,1]` in the captured frame).
- Stage 3 adds `bloom.rgb + scene.rgb` with no extra bloom scale
  (`restorescene.shader:212`); `BLOOM_SCALE` is already baked into the blur
  passes, so the composite must not multiply again.
- The blur runs on the half-float bright-pass; keep the interpass buffer at
  `R16G16B16A16_FLOAT` to match engine precision.
- Kernel/params are pure functions of `BLOOM_WIDTH`/`BLOOM_SCALE`, so no captured
  texture or constant buffer is needed at runtime — only the resolver.
