# XN14B — retained weather/atmosphere draw and composition census

Pinned target: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Status: **PROVEN — promotion-ready**. XN14B is the capture-side integration
packet for XN14A. It does not replace XN14A's producer/timing law or XN12C's
static asset correlation. No new capture and no human input are required.

Frozen XN14A dependencies: report SHA-256
`4cf5087efdcd6cba9d9fe4cdfe23a88f13bec5d69c1c6045c8bbb91f72f790d8`;
evidence-manifest SHA-256
`f37b7bd97fd7024c4d7b374c51ccf4c4b7c326d212958ad9e4f91216b7245110`.

## Result

The exhaustive retained corpus is **199/199 RDCs**: 88 July-10 frames, 72
July-12 frames, and 39 vanilla frames. The machine census records source RDC,
frame/event hashes, event and pass order, shader hashes, atlas payload hashes,
CB payload hashes, SRV/resource dimensions, targets, draw/instance counts, and
the last blend/depth/raster transition for every matched draw. Three raw anchor
frames additionally record the complete D3D11 descriptors and bound payloads.

The frozen corpus result contains 22,133 weather-or-shared-atlas draws and
805,249 instances. Of these, 5,759 draws and 562,284 instances use a
texture/technique pair whose entire shipped namespace is weather-only. The
final values and every row live in `XN14B/corpus_census.json`. Its XN12C input
is the immutable 198/198 per-RDC correlation SHA-256
`2ca48fca12e46dc64f6f37a37cae12a55c9cd59529d9e8ed9e5cc07b8ae4fffc`.

| Exact carrier | Namespace law | Draws | Instances | Retained frames |
|---|---|---:|---:|---:|
| `rain_03.dds` / additive | rain or lightning-storm precipitation; identity inside that set is CPU-side | 1,451 | 174,256 | 159 |
| `snow.dds` / alpha | snow / small / storm / small-storm | 1,165 | 59,916 | 106 |
| `snowflake.dds` / alpha | snow / small / storm / small-storm | 2,713 | 325,455 | 115 |
| `mist_stripes.dds` / alpha | sandstorm only | 66 | 1,579 | 23 |
| `dust_streaks.dds` / additive or alpha | snow-storm or small snow-storm | 364 | 1,078 | 92 |

These are exact lower-level render carriers, not guesses at the authored effect
name. `rain_03` cannot distinguish rain from lightning storm, for example, so
the production extractor carries the stable CPU effect identity required by
XN12B alongside the exact packet.

## Shared atlases and proven absences

Texture identity is not effect identity. `cloud.dds`, `cloud_2.dds`,
`cloud_6.dds`, `cloud_7.dds`, `gradient_circle2.dds`, and `snow_cloud.dds` are
shared with non-weather fire, smoke, explosion, nuclear, track, or exhaust
definitions. Their matched draws are recorded, ordered, and replayed through
the unified XN12B particle packet stream, but XN14B never relabels one as
weather from its DDS alone. This closes the otherwise dangerous false-positive
path without dropping a real draw.

The known `cloud_effect_file` / `rain_smoke.dds` false positive and vehicle
snow-track rows are mechanically excluded. The exact 18-definition XN12A
weather chain contains no standalone fog/visibility particle carrier: scene
fog is the `de579ae13f1dfa35` FOG pixel-shader variant plus scene constants, not
a separate weather draw. Lightning has no unique GPU effect-id payload; its
cloud/precipitation packets remain exact through CPU identity plus packet data.

`hoi4_vanilla_frame3088546.rdc`, the one retained RDC without a compact mine,
was converted and parsed directly. It has 4,343 draws, zero SIMPLEPARTICLE quad
draws, and therefore zero matching weather/shared-atlas draws. That is a proven
per-frame absence, not `PARTIAL` and not a tool blocker.

## Exact composition state

Raw anchors cover vanilla at 1920×1080 and July-12 at 2752×1152. Across them:

| Field | Exact law |
|---|---|
| Draw | `DrawIndexedInstanced`, R16 indices `0,1,2,2,1,3`, six indices, captured instance count |
| VS | billboard `b88e161b8297ce4b`; no-billboard `edcd722913015e88` |
| PS | FOG `de579ae13f1dfa35`; unlit `abe38fd19ed78391` |
| Instance | 48 bytes: position/size, yaw-pitch-roll, tile, BGRA8 colour, padding exactly as XN12B |
| CBs | bound VS/PS resource id, size, last-write event, bytes and SHA-256; anchors preserve decoded float payloads through 416 bytes |
| SRVs | every bound PS slot's view/resource id and dimensions; immutable payload FNV is recorded where present; slot 0 equals the shipped DDS payload hash |
| Alpha blend | RGB `SrcAlpha/InvSrcAlpha`, additive `SrcAlpha/One`, pre-alpha `One/InvSrcAlpha`; RGB write mask `0x7` |
| Depth | enabled, write mask zero, `LESS_EQUAL`, stencil disabled |
| Raster | solid, depth clip on, clockwise front; billboard culls back and no-billboard culls none |
| Sampler | min/mag/mip linear and wrap U/V/W for the particle atlas |
| Target/order | exact target view/resource/format/dimensions, viewport, frame ordinal, pass id, pass ordinal, and state-transition events |

Anchors also show the late-load requirement directly: the first July-12 frame
exposes three weather-related immutable texture resources, while the last
exposes seven, including snow, snowflake, snow-cloud, storm-cloud, and
dust-streak carriers. These are direct state/payload examples only. RenderDoc
ResourceIds are capture-local opaque identities and are **never** transported
between RDCs; every corpus label comes from XN12C's payload bytes for that same
RDC.

Particles retain XN12B's proven placement after map objects and before
arrows/state restore. Replay must preserve the captured packet ordinal, pass
ordinal, blend branch, camera/projection constants, targets, and instance bytes.

## Promotion contract

At `0x14227f420`, serialize each post-simulation, post-sort particle packet in
engine order with stable effect/definition identity, material/pipeline/texture
ids, exact matrix, live count, the `live_count * 48` instance block, atlas
layout, bound CB/SRV state, blend/fog/cull/depth/sampler branch, target and
viewport. Replay the static quad and captured packet bytes without resimulation
or semantic reclassification. State dictionaries and delta compression are
allowed only when every checkpoint remains independently decodable.

This contract covers rain, storm, snow, sandstorm, cloud and lightning carriers
plus scene-fog participation. There is no XN14B capture/data/human residual.

## Ghidra provenance inherited by the capture join

XN14B issued no duplicate binary jobs. It uses the frozen XN12B submit and
packet-builder findings; both full tuples are retained here as required:

| Address / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14227f420` bind/draw/ribbon submit | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `C:\Users\Example\ghidra_agent_exports\20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19\evidence\func_14227f420.decompile.c` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |
| `0x1423dd730` packet emission | `20260717T004108Z-xn12b-particle-instance-render-update-co-75672612` | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423dd730.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |

The full inherited function ledger remains in `XN12B/mechanical_result.json`.

## Evidence and verification

- `replay_interface/re_evidence/rendering_laws/XN14B/corpus_census.json`
- `replay_interface/re_evidence/rendering_laws/XN14B/vanilla_anchor.json`
- `replay_interface/re_evidence/rendering_laws/XN14B/session12_anchor.json`
- `replay_interface/re_evidence/rendering_laws/XN14B/session12_last_anchor.json`
- `replay_interface/re_evidence/rendering_laws/XN14B/vanilla_late_anchor.json`
- `replay_interface/re_evidence/rendering_laws/XN14B/verification.json`
- `replay_interface/re_evidence/rendering_laws/XN14B/evidence_manifest.json`

Mechanical verification:
`python replay_interface/re_evidence/rendering_laws/XN14B/verify_xn14b.py`.
