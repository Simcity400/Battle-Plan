# Lighting and shadow production law

## Authority and document role

This is the current normative lighting/shadow law for XMAP09. Durable
exact-a729 engine evidence, job identities, hashes, and capture-oracle
boundaries are recorded in
`replay_interface/re_evidence/rendering_laws/XMAP09/PRODUCTION_EVIDENCE.md`.
This file states the product contract and does not retain superseded LIGHT
experiments or capture plans.

The binary authority is `hoi4.exe 1.19.2.0.a729`, image base
`0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Semantic input

XMAP09 consumes `xmap09_lighting` with schema
`hoi4.xmap09.semantic-lighting-input.a729.v1`. It is a complete current-hour
semantic payload containing:

- the raw game hour and date-specific engine light outputs;
- the exact point-light lifecycle rows;
- effective mod-aware shadow cutoff/fade values; and
- the complete ordered 64x64 light-cell census.

The envelope binds the product modset, effective-VFS identity, exact-a729
writer RVA and executable, source observation, ordered census, and canonical
payload digest. It contains no D3D resource snapshot and no recorder camera.

Missing semantic input is a typed unavailable family. XMAP09 does not replace
it with an empty point-light lifecycle, captured-camera shadow, fixed daylight
rig, or neutral shadow texture.

## Scene lighting

epoch/hour law, and writes the runtime light positions and normalized scene
direction. XMAP09 turns those authenticated semantic outputs into a named-field
SceneCB0 patch. It never fabricates a standalone 416-byte SceneCB0.

The patch validates the reflected effective layout and preserves every byte
owned by XMAP03 camera, XMAP05 FoW/political state, XMAP07 snow/mud, and XMAP10
post-light presentation. Its receipt binds the base payload, applied field set,
final payload, reflected descriptor, and contributor identity.

`LightIndexMap` is a 64x64 RGBA8 resource. `LightDataMap` is a 128x1
`R32G32B32A32_FLOAT` resource. The app-device handoff uploads both resources
and the exact 16-byte `PointLightsCB2` payload on the replay application's
D3D11 device.

## Viewer-owned shadow camera

shadow camera from the four viewer fit points, stabilizes it in 10-unit steps,
and applies the exact 30-unit eye margin. The resulting light
view-projection/shadow-texture matrix is bound to the current camera-frame and
canonical XMAP04 replay-frame identities.

XMAP09 creates the 2048x2048 R32 typeless depth resource with a D32_FLOAT DSV
and R32_FLOAT SRV and clears it to exact depth 1.0. The authenticated execute
stage dispatches each ordered projected submission to the sole caster that
registered the matching canonical submission digest.

XMAP11, XMAP12, and XMAP13 own tree, object, and unit caster eligibility and
order. XMAP09 owns the shared depth target, registration join, execution, and
receipt; those packets do not create independent shadow maps.

The execution receipt binds the centrally assigned stage, prepared lighting
frame, XMAP03 camera, XMAP04 replay frame, target descriptor, caster
registrations, ordered submissions, clear value, and final sampled resource.
An inactive semantic state produces the typed inactive execution form with no
target binding.

## Screen-space shadow ownership

The viewport-sized BGRA8 `ShadowMap` is not a neutral XMAP09 fallback. XMAP10
owns the terrainunlit producer, horizontal/vertical blur, and downstream
publication because that path requires the aggregate reflected SceneCB0 and
fog state. XMAP10 consumes XMAP09's authenticated depth view and execute
receipt.

The exact blur factor supplied by the XMAP09 camera law is:

`min(camera_y / 700, 1)`

Provider views retain the packet owner, stable resource identity,
payload/execution digests, descriptor digest, live same-device pointer, camera
identity, and replay-frame identity. A consumer resolves its own reflected SRV
register; XMAP09 does not publish a false universal slot.

## Assigned-stage order

The central schedule authenticates:

`(xmap09.lighting_shadow.prepare, lighting_shadow.prepare)`

before:

`(xmap09.shadow_map.execute, shadow_map.depth.draw)`

The prepare receipt binds the ordered semantic suboperations and does not claim
a draw. Only same-device light-depth execution, or the typed inactive result,
can complete the execute stage.

## Fail-closed boundary

Production rejects wrong-build or wrong-hour payloads, incomplete/duplicate or
reordered light cells, invalid float lanes, effective-content drift, stale
camera or replay-frame identities, unregistered or reordered casters,
same-device mismatches, reflected-layout drift, and stage/receipt mismatches.
Retained selected-submit captures remain structural comparison oracles only;
they cannot supply product camera, light lifecycle, resource, or draw authority.
