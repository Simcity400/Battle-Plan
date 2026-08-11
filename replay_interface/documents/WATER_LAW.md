# Water and ice production law (XMAP07)

Status: shipping through the viewer-owned XMAP07/XMAP22 route.

## Authenticated inputs

XMAP07 builds water from current replay state and admitted effective game/mod
content. Required content includes the effective `gfx/FX/pdxwater.shader`, its
include closure, `common/defines/00_graphics.lua`, and
`map/terrain/reflection.dds`. The manifest-restricted VFS selects each winner;
host install paths and hard-coded vanilla catalogues are not inputs.

The exact `_xmap07_snow_mud_runtime` carrier described in `SNOW_LAW.md`
supplies the shared snow/mud texture and fade authority. Semantic province
weather rows cannot substitute for it.

The other typed providers are:

- XMAP03 camera/view state and XMAP04 map-copy/frustum partitions;
- for the high-GFX `water` effect, XMAP04 underwater terrain execution into
  XMAP07's retained refraction target;
- XMAP05 political/FOW resources;
- XMAP09 light/depth and scene-shadow authority; and
- XMAP10 current scene constant-buffer authority.

Every provider is joined to the same hour, replay-frame identity, viewer state,
map-copy receipt set, effective-VFS/content identity, assigned stage, target,
and device.

## Shader and resource law

The effective water effect determines its exact program, states, static slots,
samplers, blend/depth/raster configuration, and constant-buffer layout. XMAP07
uses `water` or `water_low_gfx` according to the authenticated graphics profile;
an effect whose interface violates the proved contract is refused.

`map/terrain/reflection.dds` is the reflection-cube authority. It is decoded
through the effective VFS into the proved cubemap subresource order; no captured
cube or mean-colour approximation participates.

For the high-GFX `water` effect, the refraction resource is a separate
viewport-sized target owned by XMAP07. XMAP04 renders the authenticated
underwater terrain partitions into it; XMAP07 then samples that completed
target. Target identity, viewport, format, map-copy ordering, and the underwater
execution receipt must agree. The `water_low_gfx` effect has no underwater
program and does not consume `WaterRefraction`, so that provider is not required
for the low-GFX route.

The surface geometry is the proved four-vertex triangle strip. The exact
vertex-fetch bounds are derived from the admitted input layout, strides,
binding offsets, and draw arguments; unknown formats, unsafe origins, or
insufficient bytes fail closed. No blanket padding or guessed vertex origin is
accepted.

The selected effective effect owns its refraction/reflection, shoreline alpha,
glitter, virtual-sun packing, ice/snow sampling, and blending laws. Replay
consumes XMAP10's authenticated scene values and does not invent an orbit,
gloss clamp, light attenuation, or visual correction.

## Execution and composition

Preparation completes before D3D state changes. The retained XMAP07 owner
uploads immutable content resources once per exact content/program closure and
binds current-frame providers separately.

In MainScene order, XMAP07 makes its snow/mud resources available to earlier
consumers, executes the selected profile's map-copy work (including underwater
terrain for high GFX), then draws the water surface after XMAP11 trees and
before XMAP06 late borders. Its native receipt binds the program, resources,
provider receipts, surface draws, target, and final scene operation. XMAP22 v4
separately binds both
`xmap07_water_execution_sha256` and
`xmap07_snow_mud_authority_sha256`.

## Fail-closed conditions

Water is unavailable for missing or inconsistent effective content, map extent,
snow/mud carrier, provider, stage, device, target, vertex-fetch law, shader
interface, resource identity, or receipt. Product rendering never uses captured
draws, RenderDoc payloads, diagnostic switches, environment overrides,
approximation shaders, prior frames, or a visually tuned substitute.

## Evidence and implementation

Exact-a729 program/resource evidence is retained under
`re_evidence/rendering_laws/XMAP07/` and the XMAP04 underwater contract under
`re_evidence/rendering_laws/XMAP04/`. Production preparation is in
`map_viewer/src/production_features/xmap07.rs`; native ownership and execution
are in `map_viewer/src/xmap07_d3d_handoff.rs`; the shared compositor is
`replay_app/src/app.rs`.
