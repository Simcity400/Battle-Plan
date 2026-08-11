# XMAP12 replay data requirements

XMAP12 is Replay-owned. It does not authorize edits to `extractor/native` and
does not consume recorder-camera draw lists or selected-Present
`EntitySubmission` rows as product authority.

Static cities require no extractor data. Replay regenerates them from the
admitted effective winners for:

- `map/cities.txt` and its `types_source` indexed BMP;
- `map/terrain.bmp` plus `common/terrain/*.txt` `spawn_city` definitions;
- `map/rivers.bmp`;
- `map/provinces.bmp`, `map/definition.csv`, and the exact land flag;
- `map/heightmap.bmp`;
- selected entity, PdxMesh, mesh, material, texture, and shader files.

Building placement and content come from admitted `map/buildings.txt`,
`common/buildings`, entity/PdxMesh declarations, and their mesh/material
chains. Current semantic level comes from the existing
`WorldFrame::buildings_by_state()` D48 projection. Exact O05 presentation also
requires the pointer-free `Xmap12BuildingVisualFrame`, in placement-file order:
the engine-selected normal/destroyed entity token after fallback, destroyed and
under-construction state, and current grow/shrink scale. D48 values are
cross-checked when measured, but absent D48 presentation leaves are not guessed.
This is the exact supplemental boundary the Extractor Layer must populate; the
Replay Layer does not modify the extractor.
Replay admits it from the authenticated per-frame WorldFrame domain
`xmap12_building_visual`; malformed or cross-frame values fail closed.

Ambient/fixed objects use the effective `map/default.map` winner. The engine
unconditionally resolves the authored `ambient_object` token as the literal
`map/` plus the full raw token; it does not normalize slashes or rescue an
already-prefixed value. Type and object source order, default idle animation,
height/yaw/type scale, DLC, duration, presentation enablement, and the unusual
`max_visible` counter law are reproduced exactly. Seeking requires persistent
viewer-owned per-type counters and per-object active/remaining-duration state;
recomputing every frame from an all-inactive initial state is forbidden. The
lifecycle carrier is bound to the exact replay-frame identity, and Replay
rejects a valid carrier from any prior or later hour.
Replay admits this carrier from the authenticated per-frame WorldFrame domain
`xmap12_fixed_runtime`.

The reusable XMAP12 PdxMesh materializer accepts stable semantic identity,
resolved effective entity/PdxMesh declarations, a world transform, authored
scales. XMAP12 admits only the proven complete 48-byte static
position/normal/tangent/UV0 layout; UV1 and XMAP13 skinning fail closed. It
authenticates every
payload through `ValidatedContentAdmission`, parses the effective mesh,
regenerates generic bounding-sphere/frustum/LOD selection, emits exact geometry
and material identities for the main scene, and supplies the unchanged XMAP09
L03 caster primitive. It never accepts a recorder `EntitySubmission`.
`.gfx` `meshsettings.index` addresses the raw authored shape/group ordinal,
before drawable groups and submeshes are compacted; it is not an inner
submesh index.

Native execution requires exact compiled effect objects, material textures,
shared map resources, samplers, partition SceneCB0s, pixel CB2, and the real
MainScene target. Those are typed provider inputs; a missing provider makes the
XMAP12 feature explicitly unavailable. Neutral textures, guessed constants,
black-clear substitution, and recorder-selected resources are forbidden.
Only samplers present in compiled-bytecode reflection are materialized. The
object CB1 is the retained 96-byte PdxMesh layout and is bound to VS/PS; L03
receives a separate standard-layout CB0 populated with the authenticated
selected-light view-projection, never the viewer camera matrix. Shadow
partition bindings cover exactly the partitions that contain XMAP12 caster
submissions. Each retained object owns a distinct CB1 for every selected map
copy: primary uses the authored transform, west-wrap mask 2 adds `map_width` to
world X, and east-wrap mask 4 subtracts `map_width`, matching XMAP04's exact
map-copy offsets in both MainScene and L03.
The effective mod-selected effect bytecode is reflected at admission time, so
each shared texture is joined by its authored semantic name and shader register
rather than a vanilla register guess. Every reflected constant-buffer layout,
material/shared texture, and sampler must match the complete set that XMAP12
binds. Main shaders are restricted to canonical SceneCB0, PdxMesh CB1, and
PointLightsCB2 plus reflected material/shared resources; shadow shaders are
restricted to canonical light SceneCB0, PdxMesh CB1, and the proven optional
diffuse-alpha t0/s0 path. Any effective mod shader outside that supported
contract is typed unavailable before a draw, never executed with stale state.
Reflected SRV dimension is part of every requirement: material slots must be
`Texture2D`, and each shared provider's live `GetDesc().ViewDimension` must
equal the effective DXBC requirement (including the environment `TextureCube`).
The shipping replay caller provides
XMAP09 light data, XMAP07 snow/mud, XMAP10 shadow-map, XMAP05 generated map
resources, and admitted XMAP04 static resources on the same D3D11 device.
XMAP12 additionally owns the three static map-graphics resources supplied by
the exact-a729 loader: `FOWNoise_Texture` from the one selected
`map/terrain/fow_noise_{texture_quality}.dds` winner,
`FOWHeight_Texture` from `map/terrain/fow_rgb_waterspec_a.dds`, and
`EnvironmentMap_Texture` from the real six-face
`map/terrain/reflection.dds` cube. All three are admission-bound effective-VFS
winners sealed into the prepared-frame digest; no bare-vanilla fallback or
quality-tier guess is permitted. A semantic not owned by one of those exact
providers remains typed unavailable.
The product map is advertised complete only after XMAP12 returns a validated
native receipt; a validated zero-draw receipt is authenticated empty, while a
missing carrier or provider is unavailable and clears retained XMAP12 state.
