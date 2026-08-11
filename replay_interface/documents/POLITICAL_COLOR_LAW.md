# Political map-mode production law

## Authority and document role

This is the current normative law for the XMAP05 political, occupation,
GradientBorder, IntelMap, FoW, visibility, and SceneCB0-patch family. Durable
exact-a729 engine jobs, hashes, retained capture observations, and the exact
producer law are recorded in
`replay_interface/re_evidence/rendering_laws/XMAP05/PRODUCTION_EVIDENCE.md`.
Required replay inputs are specified in
`replay_interface/re_evidence/rendering_laws/XMAP05/EXTRACTOR_DATA_REQUIREMENTS.md`.

The product path is the XMAP05 foundation adapter and
`NativeXmap05PoliticalHandoff`. Captured political resources and the legacy
`--gb-cpu` visualizer are comparison/debug surfaces, not product inputs or
fallbacks.

The binary authority is `hoi4.exe 1.19.2.0.a729`, image base
`0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Political epoch

One XMAP05 epoch joins:

- admitted effective `definition.csv`, `provinces.bmp`, state membership,
  country tags, country colors, and graphics defines;
- measured current-hour state owner, state controller, cosmetic tag, and
  province-controller tables;
- the complete current D18 country roster;
- current map, country, visibility, and selected-viewer-POV payloads; and
- the exact MAPC1 control-authority envelope.

MAPC1 binds the exact executable and evidence manifest, proven province
controller field and country-remap function, strict state/province identity
orders, nonzero capture sequence, complete-table assertions, and canonical
census/roster/presentation digests. Save-shaped or static-history rows cannot
satisfy this contract.

Before reading content, XMAP05 independently enumerates the effective
non-recursive state-history, country-tag, and country-file censuses and rejects
an admission that omitted a winning path. Every subsequent read remains
allowlist-bound to the admitted effective VFS.

The epoch digest changes with replay hour, viewer country, owner, controller,
cosmetic identity, MAPC1 authority, visibility authority, or effective
content. The app-device handoff replaces all XMAP05 resources at a semantic
epoch boundary; no stale ownership epoch is carried forward.

## Political and occupation resources

`ProvincePrimaryColorMap` and `ProvinceSecondaryColorMap` are derived from the
selected replay epoch and admitted country colors. The half-resolution lookup
uses the shader's vertically flipped half-texel UV law. Occupation-secondary
RGB is the stripe target and alpha is stripe strength, consumed by
`secondary_color_mask` and the procedural diagonal occupation mask.

Static 1936 state history is not a runtime fallback. Captured all-transparent
secondary maps are negative comparison frames, and the retained
nontransparent Resistance capture is an additional producer oracle. Shipping
authority remains the current typed control epoch plus effective files.

## GradientBorder law

Country GradientBorder generation uses the exact-a729 full-resolution
province-edge topology. The engine groups 12-byte province/neighbor edge rows,
orders integer coordinate chains, marks the complete country region, and keeps
only chains whose neighboring province lies outside that region. Internal
same-controller joins and diagonal-only contacts are not country edges.

The fill worker finds the minimum squared Euclidean distance to the integer
edge-point set. Spatial bins only prune candidates. After the square root it
writes:

```text
if distance > center:
    byte = 255
         - trunc((distance - center) * (1 / thickness) * 255)
         - trunc(center)
else:
    byte = 255 - trunc(distance)
byte = clamp(byte - 8, 0, 255)
```

`center` and thickness interpolation come from the admitted effective graphics
defines and exact country-area law. The literal subtract-eight is engine law,
not a tuned adjustment. Production contains no chamfer metric, captured
`RAMP_D0`, reduced-raster majority vote, seam invention, or CPU-visualizer
fallback.

Channel1 and Channel2 use the proved `2 * layer_height + 2` packing: top layer,
zero separator, country layer, zero pad. Country RGB and distance occupy the
Channel1 country layer. Channel2 country-layer `.g` is 255 only on
controller-owned land.

`GradientBorderChannel3` is the admitted effective
`map/terrain/naval_dominance_fx.dds`: 2x1, one effective mip,
`B8G8R8A8_UNORM_SRGB`, point sampled, clamped, non-array, non-cube. The two
mod-resolved texels are uploaded exactly. Vanilla bytes are an oracle, not a
shipping constant.

## FoW and disclosure

`IntelMap` uses the exact six-by-six province coverage average and bitwise
inverse law. Omniscient mode disables FoW only after the complete D18 POV
census is authenticated. A country POV requires its current D94 vector and
current D95 land/task-force disclosure surfaces with nonzero matching
identities.

D95 raw bytes drive disclosure tier and reason. Threshold-excluded entities
are labeled as such; they are not falsely called engine-confirmed hidden.
Exact SPU1 authenticates the absence of an air-wing spotting surface, and
XMAP05 exposes `air_wing_spotting_unavailable = true`. No third spotting
threshold or other visibility rule is invented.

## SceneCB0 patch

XMAP05 publishes a typed `Xmap05SceneCb0Patch`; XMAP10 owns the aggregate
416-byte buffer. The patch binds the effective content identities, replay hour,
viewer camera, political epoch, exact executable, and authenticated
viewer-owned XMAP10 recurrence.

- `vGBCamDistOverride_GBOutlineCutoff` comes from effective graphics defines.
- `FOWFadeFactor` is
  `square(clamp((DRAW_FOW_CUTOFF-camera_height)/DRAW_FOW_FADE_LENGTH,0,1))`.
- FoW time comes from the authenticated viewer-owned XMAP10 presentation
  recurrence. Legacy opacity is sealed to the engine reset value `+0.0` and is
  not recorder-presentation authority.
- Camera height comes from the validated viewer-owned XMAP03 current eye.
- `MinMeshAlpha` is exact positive zero.
- `GB_TextureHeight` is the logical province-raster height.

The patch validates the reflected layout and preserves fields owned by every
other packet.

## Native handoff

`NativeXmap05PoliticalHandoff` creates and retains six real D3D11 textures and
SRVs on the replay application's device:

1. `ProvincePrimaryColorMap`;
2. `ProvinceSecondaryColorMap`;
3. `GradientBorderChannel1`;
4. `GradientBorderChannel2`;
5. `GradientBorderChannel3`; and
6. `IntelMap`.

Each upload binds the prepared payload digest, descriptor digest, exact format,
extent, row pitch, and mip contract. Consumers receive typed owner/resource
views rather than unlabelled SRVs. XMAP04/XMAP07 water and terrain draws resolve
their reflected slots and reject missing, stale, wrong-hour, wrong-content, or
same-device-mismatched providers.

## Fail-closed boundary

Production rejects incomplete control or POV censuses, missing MAPC1/D18/D94/
D95 authority, wrong hours or identities, content-admission omissions,
unresolved colors, topology disagreement, malformed DDS resources, stale
political epochs, reflected-layout drift, and upload/receipt mismatches. It
does not use initial-history ownership, save parsing, captured textures,
recorder POV, neutral political colors, guessed occupation alpha, or legacy
GradientBorder output as a substitute.
