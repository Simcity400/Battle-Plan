# XMAP10 replay data requirements

XMAP10 does not require a live the game process, recorder country/POV, RenderDoc
payloads, or captured GPU textures in production. RenderDoc and Ghidra are
evidence for the engine law; product frames are rebuilt from replay state,
viewer state, and the effective installed content.

## Replay/state inputs

The replay-facing typed providers must supply:

- exact game date and replay hour identity;
- the XMAP07 snow/mud input and its deterministic receipt;
- the XMAP09 lighting/shadow input, authenticated
  `ShadowMapTextureMatrix`, and completed light-depth receipt;
- the XMAP05 political/FoW SceneCB0 patch, including its authenticated map
  clock;
- the selected-frame content manifest, product modset, and effective-VFS
  identities.

XMAP10 does not infer unavailable world semantics from the recording country.
All dependencies are global game state or independently authenticated visual
providers.

## Viewer-owned inputs

The replay viewer, not the extractor, owns:

- viewport dimensions;
- main/west/east map-copy identity and exact map-copy offset;
- the authenticated camera/view matrices and camera height;
- the authenticated HDR MainScene color descriptor and its paired
  full-resolution D24S8 depth descriptor, both derived from the XMAP10 target
  plan;
- a chained presentation clock carrying application/process elapsed seconds
  for SceneCB0 animation phase;
- frame delta and the viewer-local `day_night_toggle` state;
- playback frame delta and sequential frame continuity;
- output/app render-target identity.

Temporal exposure depends on render-frame history. Exact-a729 resets its two
1x1 R16_FLOAT targets only when HDR resources are allocated/recreated or HDR
mode changes; otherwise it consumes the immediately preceding rendered frame.
A random replay seek is not an engine reset. The viewer must either:

1. retain the authenticated preceding temporal state;
2. deterministically replay the post-FX history from an authenticated resource
   reset/checkpoint; or
3. report the requested frame unavailable.

No live production capture is needed. An optional replay-side temporal
checkpoint is a performance optimization, not a prerequisite for correctness.

The presentation clock and day/night gate are not extractor country/POV
fields. `vGlobalTime` is process elapsed time. Exact-a729 RE proves that the
field historically named `MaxGameSpeed` starts at zero and advances as:

```text
target = day_night_enabled && (recorded_paused || recorded_speed_raw < 4)
alpha  = clamp(frame_delta * 12.0, 0.001, 1.0)
gate   = clamp(old_gate + (target - old_gate) * alpha, 0.0, 1.0)
```

The existing authenticated replay clock supplies pause and raw speed. The
viewer supplies frame delta and the same local day/night toggle that it already
uses to select a day/night hour. XMAP10 chains these into its presentation-state
receipt; it never derives them from replay date/hour or UX playback speed.
Missing recorded clock state fails closed. A fresh MainScene/compositor starts
the gate at exact zero. An arbitrary historical seek must preserve the live
viewer recurrence or replay it from an authenticated zero-state boundary; it
must not silently reset and call the result historical continuity.

## Effective game-file inputs

The content resolver supplies and authenticates the selected effective winners
and transitive include closure for:

- `gfx/posteffect_volumes.txt`;
- `common/defines/00_graphics.lua`;
- `gfx/FX/downsample.shader`;
- `gfx/FX/downsample_luminance.shader`;
- `gfx/FX/bloom.shader`;
- `gfx/FX/restorescene.shader`;
- `gfx/FX/lut_blender.shader`;
- `gfx/FX/pdxmap.shader` and its `terrainunlit` include closure;
- `gfx/FX/shadowblur.shader`;
- the XMAP04-admitted terrain textures and sampler declarations consumed by
  `terrainunlit`;
- every transitive `.fxh`/platform include;
- every LUT path reachable from the active inherited height, day/night, winter,
  and region volume endpoints.

Production fails closed when the active shader/define/LUT closure cannot be
resolved, compiled, reflected, or joined to the frame's content identities.
It does not substitute vanilla files, filename-pattern LUT discovery, captured
bytecode, or a neutral/identity grade.

## Production compositor boundaries

XMAP10 publishes three authenticated boundaries rather than claiming ownership
of the whole frame:

1. The pre-particle boundary proves that the file-derived content admission,
   XMAP09 lighting admission, exact 416-byte SceneCB0 binding, and the viewer's
   MainScene color/depth targets agree. XMAP20 consumes that boundary before
   drawing particles.
2. The screen-shadow boundary consumes XMAP04's exact terrain geometry/tile
   rows, XMAP09's current viewer-camera shadow matrix and completed light-depth
   target, plus the matching per-map-copy SceneCB0 buffers. It executes
   `terrainunlit`, horizontal blur, and vertical blur, and publishes the
   viewport-sized BGRA8 `ShadowMap` only after all three passes complete. When
   XMAP09 reports the exact inactive path, XMAP10 performs no draw or clear and
   retains the current `ShadowMap` resource contents through a chained receipt.
   An inactive fresh allocation fails closed because it has no defined prior
   contents.
3. The post-FX boundary consumes XMAP20's completed MainScene receipt and the
   shared assigned-stage schedule. It executes bright pass, the exact
   target-derived luminance downsample chain, luminance gather, horizontal and
   vertical bloom, and RestoreScene.

The native post-FX executor is constructed only over the exact
`NativeProductionMainSceneTarget` retained by the viewer. It validates the
texture/device pairing and records that allocation's pointer-free resource
identity in both passes that sample MainScene. A descriptor-identical second
HDR allocation is not an admissible substitute.

The downsample chain is resolution-dependent. Each authenticated subpass names
its input and output dimensions; the chain repeatedly uses ceil-half extents
and stops when both next axes are at most three texels. The aggregate schedule
still contains one `postfx.luminance_downsample_chain` node, so target size does
not change the global map-stage topology.

RestoreScene writes RGB only and unbinds its shader resources. XMAP22 owns the
final target, UI composition on that target, and Present. XMAP10 never presents
the swapchain and never includes UI work in its completion receipt.

## No extractor-layer changes in this packet

This document is the Replay Layer's dependency contract. XMAP10 does not modify
Extractor Layer code or claim that a missing typed provider has already been
recorded. Provider availability is checked at replay preparation time.
