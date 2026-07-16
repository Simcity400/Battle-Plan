# Retained-corpus inventory for non-UI rendering-law packets

Date: 2026-07-16  
Lane: `corpus_inventory`  
Disposition: offline retained-corpus inventory only; no Ghidra, capture, canon, renderer, or review work.

## Authority and scope

This inventory is subordinate to `RENDERING_LAWS_RE_EXECUTION_PLAN.md`,
`re_evidence/rendering_laws/AUTHORITY.json`, and the orchestrator-owned
`PROGRAM_STATUS.md`. It covers the 35 non-UI packets B01-B06, L01-L06,
C00-C04, T00-T01, R01-R03, O01, O02, O04-O06, N01-N04, and M01-M04.
O03 is outside the active roster because its retained-corpus law is closed;
O06 replaces it as the `trees.bmp` to runtime slot-14 R8 generator trace.

The executable authority is `hoi4.exe 1.19.2.0.a729`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, image
base `0x140000000`. Corpus reuse is fail-closed: a file is reusable only for
the facts its retained identity, event provenance, consumer binding, and
true-last-write coverage actually establish.

## Registry

The machine-readable source and packet registry is
`replay_interface/re_evidence/rendering_laws/CORPUS_INVENTORY/registry.json`.
It records the retained source classes and their authoritative per-artifact
registries; this report summarizes packet routing and exact capture gaps.

## Packet routing

### Retained source classes

| Source | Identity hash | Contents and reusable boundary | Event / provenance / true-last-write |
|---|---|---|---|
| 160 retained RDCs: 88 in `session_2026-07-10`, 72 in `session_2026-07-12` | corpus registry SHA-256 `9715a81dababc7c6d888a957ec102e73f2d5cc01b3cd93d2f995facd56d23e4d`; embedded artifact-manifest SHA-256 `f9549cb692c4b394a1f2869a3b70c3a78bbf039b4d065ff3dd98e4a94614919f` | Whole-frame source, `resources.json`, `events.jsonl.gz`, CB0 decodes, CB blobs, initial contents, shaders and content-addressed blobs. Per-RDC SHA-256 and byte length are in the registry. | Event streams exist, but the generic mines are `EVENT_STREAM_AVAILABLE_NOT_LAST_WRITE_VALIDATED`; 38 legacy mines have no retained source RDC. Reuse them for discovery, never as consumer-scoped last-write proof. |
| W20 lighting/shadow/postfx referee | SHA-256 `658ea962861dfdc70c2676f3dcd3a30ae679a0bbeb2c7d250701dbd1a149a90a` | Corrected census: 129 full shadow chains at 28 sun positions, hash-bound to RDCs; postfx order/resources also indexed. | Full event order and preceding writes for the shadow chain. Postfx is partial: `previous_exposure_current_frame_last_write` is absent and ColorCube producer provenance remains unproven. |
| Camera CB0 referee | same RDC/index roots above | Corrected census: 2,820 matrix-complete CB0 samples. The old “1414 sweep” name is quarantined. | CB write and draw-event context are reusable. All retained views are fixed-size session captures; no labelled viewport-resize case is established. |
| Border pool `session_BORDERZOOM_3` | VB SHA-256 `45878ff7c0699833340bb73b3b669ba2058a2b399467d110b024b6e39508228a`; bundle SHA-256 `32034a43c643d21a221fb49f1614d4790507db98e8520915617b8f74817318a2` | 31,824,096-byte stride-16 VB, 40,959 strips; current exact census land 27,024/27,024, water 8,578/8,578, closed 924/928 with residual records 13673, 40330, 40345, 15029. | Direct draw binding and pass-boundary provenance exist; no CPU-builder event or preceding-write history. Geometry bytes are authoritative; allocation/emission producer claims are not. |
| Native PAN2 map / tree / river bundles | map SHA-256 `e1078500910e127bbdf780f1bbe7d0c191711dfa10acaf4a33bca20559879adb`; trees `d293ff9b5f9d53f37e9266bd490dd262574bc288f987e00f15172d9a1ac6d934`; rivers `e6d8afe47dd819b456b92f20206376a29c2385189ae63de8bfb0b93dab905756` | Bound buffers, CBs, SRVs, descriptors, shaders and pre/post OM payloads. | Capture-moment/pass provenance and payload FNVs exist, but no API event identity or consumer-scoped preceding writer. Runtime textures captured at `dwell_end` are contents, not producer proof. |
| Terrain chunk libraries PAN2 / PAN-chunks / FINAL | SHA-256 `5467e764601e0ee9c86b6719b718b2cc05c1f55e53a4426369a48976ff211685`, `f09fc0e14fa7a295da3eb56853fcc096c341875f4cbbaf32ec4f8554b3dd7a1e`, `37c45078d1836012cd56e392ac389d3c2fc134f8a2b069acb4a7359e3ca2f88c` | 44-owner PAN2 library plus the PAN/FINAL vSnap answer keys. Historical direct-edge gates are PAN-chunks 188/188 and FINAL 28/28; PAN2 duplicates remain separately classified. | Draw/CB binding provenance only. They contain no `map+0x4970`, actual three plane arrays, root flags, or consecutive CPU updates. |
| CAP9 `hoi4_frame54162` | RDC SHA-256 `887a8593398281f5937929b7fdf054ec836128aa30544030df695f4d88717104` | Eight immutable tree instance buffers (3,985 records), 153 immutable rail VBs, object-family candidates, full event stream and content-addressed payloads. | Source identity is pinned, but the generic frame mine remains `UNVERIFIED_DERIVATION`; immutable creation contents are reusable while semantic identity and per-draw true-last-write joins remain packet-specific work. |
| Tree runtime-state export | stale checked-in summary SHA-256 `d470f7cfb4823f5a539388341aef6de12431b778e00aa1ea8d9ccbd348d0c17f` | The checked-in summary records the earlier 984-draw census. The orchestrator’s corrected retained result supersedes it: 1,080/1,080 ULP-exact for `clamp((350-cameraY)/100,0,1)`. | The summary carries RDC, event, CB0/CB1 last-write and payload SHA-256 fields. A refreshed artifact hash for the corrected 1,080-row export is not yet present in this checkout and must be added before frozen synthesis. |
| W11c label mines | source RDC SHA-256 `05a7f20a676ef5b546a80103d6960955d4a520eceffdb3671ea34b4030c8f7d2`, `a589740ae1849e757f87111a2a01ba5b0ec7a80c0d81284d589774a7d4969093`, `13c899162a0c9787ef14caee39222fa39302fa8d017a27a699d4fa9de5bbe754`; artifact trees `c00f0970f6648b43e76ab6c5dca0010ec1f32f99970885fa1405f10fcc91011b`, `c2f54268898b42bacafe3be1f8a31bfa5130572c98d1f2fc9dcd83d0e583d51a`, `9497976adf5769e35157f2cd5d0f72da9195def0d3cdcf03824b83636655ffc8` | Country-font VBs/atlas/CBs and creation contents. | Event and last-write markers exist but are not semantically validated. |
| Map-label oracle | SHA-256 `7b454870048d6dcf75b77d4b940307db2d8119c73993d5b368ad0f35d94cee63`; placement gate `4a079557d7c163cec6e8395eef16661c2b24a0b6cc56c96582aacbef1bb98bea`; provenance `4d563cca2195965a86d2edd2c4f5b90f52d41f4e4dd84fdcf9e0f705f0b49992` | 63 decoded engine labels and held-out placement/VB fixtures. | Fixture provenance, not runtime producer history. |
| W13 focused mines | rivers artifact-tree SHA-256 `f91082ecdb86c10a29225d59def37d97eb45684279c7362d792717ed6ee154dc`; city artifact-tree `c050f34897fa2f0d869481f9f0eb3da362b16bd452c94e3d01f931bd7bf97401` | River event-state diagnostics and a city/object candidate. | Rivers have no event-scope proof. City has event markers only and no last-write proof; the prominent candidate was later classified as a tank. |

`E/P/T` below means event identity, source/binary provenance, and true-last-write
coverage. `yes` applies only to the named reusable leaf, not to the packet as a
whole.

### Borders, gradients, light and camera

| Packet | Reusable retained evidence | E/P/T | Exact retained-corpus result and smallest missing fact |
|---|---|---|---|
| B01 | 160-RDC height ladder has border draws and all small CB writes; native border CB2 exists. | partial / yes / no | No extracted per-type `vTime_Transparency.y` trace or proven CPU writer/offset. Re-mine CB writes before C06; only if type/ramp discrimination is still absent capture one height-only sweep with draw ID, 32-byte CB2 pre/post, slot/offset and preceding writer. |
| B02 | Border-pool VB and 8,578/8,578 engine-oriented water strips. | draw / yes / no CPU writer | Exact U/direction and monotone chain-step V are closed from the VB. The remaining task is offline ordered-f32 seed/instruction order for the thin-water rows; no new capture fact is identified. |
| B03 | Same 40,959-strip VB and current four-record census. | draw / yes / no CPU writer | Exact bytes/identities exist for residual records 13673, 40330, 40345 and 15029. Missing contour record construction, region assignment and junction/closure state are producer facts, not a new visual capture ask. |
| B04 | Gradient Channel1/2 payloads across native and RDC stores. | partial / yes / no | Candidate-set/distance equality cannot be certified because retained textures lack their edge-list/candidate inputs and preceding writes. Re-mine RDC writes first; no capture is authorized merely to obtain another final texture. |
| B05 | Gradient payloads, FILE define inputs and province/state maps. | no / yes / no | Per-province `+0xc0` area, country aggregate, `+0x428/+0x410` values and f32 interpolation are absent. These are RE/data exports; no new capture fact is identified. |
| B06 | Exact Channel1/2 bytes, 2816x2050 descriptors, shader slots and draw consumers. | partial / yes / no | Pack/resolve bytes are reusable; alpha producer, guard rows, resource allocation, dirty/refresh cadence and consumer-scoped preceding write are absent. Re-mine first; a controlled owner/selection change is justified only if no retained write event exists. |
| L01 | 1,231 retained light-rig writes plus the corrected 129-chain referee; moon z is invariant 1448. | yes / yes / yes CB0 | The corpus cannot distinguish `SUN_LATITUDE+MOON_HEIGHT` from `MAP_SIZE_Y-MOON_HEIGHT`. C01 is required: three otherwise-identical cases retuning only moon height, sun latitude, then map height, with producer inputs and CB write. |
| L02 | 129 full chains/28 sun positions and 2,820 matrix-complete CB0 samples. | yes / yes / yes CB0+chain | Matrices and consumers are present; controlled fixed-camera/varied-sun and fixed-sun/varied-camera grouping plus resize are not established. Do not reshoot generic shadows; use C05/C06 only for the missing independent-variable and resize cases. |
| L03 | Every active chain carries ordered light-depth draw ranges/counts and draw-set SHA-256. | yes / yes / yes resources | Draw-set bytes/order are reusable, but stable semantic producer identities and inclusion predicates are absent. C05 is needed only if retained draw resources cannot be joined offline to semantic entities. |
| L04 | Postfx event order, luminance gathers, 1x1 resources and restore consumers. | yes / yes / partial | `previous_exposure_current_frame_last_write` is null; no controlled over-range dark/bright time series, first-frame state, pause or ping-pong transition. C02 must capture pre/post luminance, both 1x1 states and timestamps before/after update. |
| L05 | Default bright/downsample/bloom H/V/restore chain and payloads. | yes / yes / partial | No one-variable wide-`BLOOM_WIDTH` case or proven kernel CB generation. C03 is required with default control and width-only retune, kernel CB, intermediates and final target. |
| L06 | SnowMud/water payloads and winter/partial-winter frames. | partial / yes / no activation writer | Retained water is open; the legacy winter oracle lacks its source RDC. C04 must show visible northern ice across date/latitude boundary with A2 SnowMud inputs, slots 6/7/9 and water output. |
| C00 | 2,820 CB0 samples expose final camera/matrix values. | yes / yes / yes CB0 | Current/target object root, layout and writer/read graph are not in GPU corpus. Offline RE only; no new capture fact unless a later trajectory cannot be related to fields. |
| C01 | 2,820 matrix-complete samples; old “1414 sweep” quarantined. | yes / yes / yes CB0 | Strong matrix oracle, but no labelled viewport/aspect resize. C06 must add one resized viewport; existing samples can gate all non-resize composition work. |
| C02 | Final CB0 heights across 50..3000. | yes / yes / yes CB0 | No labelled NFrontend target/current trajectory, tick delta, pause, or boundary event. C06 must record exact input event, target/current writes and clock across both clamps. |
| C03 | Same height/camera corpus. | yes / yes / yes CB0 | Wheel/key signs and per-event NGraphics writes are not labelled. C06 must record wheel/key events at interior and both boundaries with update order. |
| C04 | CB0 matrices and retained wrap draws. | yes / yes / yes CB0 | No captured root wrap-enable bytes, canonical/west/east plane arrays or resized-map case. C06 must atomically record flags, all three views/planes, map width and resize. |

### Terrain, rivers, objects, labels and semantic draws

| Packet | Reusable retained evidence | E/P/T | Exact retained-corpus result and smallest missing fact |
|---|---|---|---|
| T00 | 89-frame draw census and PAN/PAN2/FINAL libraries. | draw / yes / no CPU state | The 98 engine-only false culls are preserved. `map+0x4970`, flags and three actual plane arrays are absent. C06 must record them in the same two consecutive stable updates. |
| T01 | PAN-chunks 188/188 and FINAL 28/28 direct-edge answers; PAN2 duplicates classified separately. | draw / yes / no CPU state | Prior/current signed owner records and all-subviews-culled history are absent. Same C06 atomic consecutive-update payload as T00; no independent shot. |
| R01 | 5,784,192-byte retained river VB and W20 event-state diagnostic. | partial / yes / incomplete | 14/22 W20 resources lack a preceding writer; W13 river export has no event scope. Re-export true last writes from retained RDC before any shot. Builder allocation/order and height-sample provenance remain absent. |
| R02 | FILE palette/defines and default river draws. | partial / yes / no | No one-variable palette-index/define reassignment. C09 is required only after RE cannot prove loader branch/order; it must vary one palette/define at a time. |
| R03 | Adjacency FILE data and retained draw candidates. | partial / yes / no | No labelled land/sea/impassable/through-province/wrap set with byte-equal geometry and inclusion. C09 must label those types if retained draws cannot be classified offline. |
| O01 | Eight immutable CAP9 buffers, 3,985 `[x,y,z,yaw,slope_x,slope_z]` records, source RDC hash. | creation+draw / yes / immutable creation only | Yaw values/order are sufficient for offline PRNG work; prior mismatched candidates are refuted. No new capture fact. |
| O02 | Same 3,985 records plus height/normal FILE assets. | creation+draw / yes / immutable creation only | Slope pairs are sufficient for offline kernel/order work. No new capture fact. |
| O04 | `cities.bmp`/FILE data plus W13 city candidate. | markers / partial / no | Candidate was a tank; no event-authoritative city instance list, terrain snap, province association or model/yaw order. C07 must isolate a labelled city region and preserve instance buffers/matrices if retained frame54162 cannot be reclassified. |
| O05 | CAP9 object-family resources and FILE mesh/material census. | yes / yes / no semantic TLW | Exact runtime object set, model-to-world, LOD/shape/variant, bindings and compositor order are absent. C07 is required unless retained frame54162 yields event-scoped true-last-write plus stable object identity. |
| O06 | Tree draw slot-14 runtime R8 resource and shipped `trees.bmp` FILE input. | draw / yes / no producer | Shipped-file absence is proven: slot 14 is runtime-generated R8, not a missing bitmap. Re-mine retained creation/write history and trace `trees.bmp` to generator offline; do not request a shipped file or generic tree recapture. |
| N01 | 63-label oracle and country-font VBs. | draw / yes / immutable creation | Output-equivalent bbox cases do not expose `param_11` producer. Offline caller provenance first; capture only if RE proves a smaller-than-passable bbox case can occur. |
| N02 | 63 labels plus held-out VBs/placement fixtures. | draw / yes / immutable creation | Existing fixtures can test current Catmull-Rom output; strong-concavity branch coverage is not certified. C08 only if static branch census finds an alternate branch or the existing oracle lacks three qualifying shapes. |
| N03 | FILE province graph and label components. | no / yes / no | Diagonal/sea/island/impassable/strait truth is absent as an exhaustive labelled runtime set. Prefer exhaustive FILE-derived synthetic graph; C08 only for unresolved engine-vs-file topology cases. |
| N04 | Hash-bound same-height pair: frame129740 SHA `85fd67f72fcd02e6e1630709956380a018636077aef2413b6f3e8d7c957a4003` gives 1.0; frame129921 SHA `4ac312064932e5043ed6170ba00d1e04c1e372943c9abe47631c8d783f323fee` gives 0.6331649422645569. | yes / yes / yes CB1 | Height-only is refuted. Upstream selector identity and a controlled transition are absent. Re-mine setters first; C08 must change only the surviving selector if offline dataflow cannot identify it. |
| M01 | Hash-bound frame54162 plus unit/model/counter candidates. | yes / yes / no semantic TLW | No stable unit ID joins A7/A14 to ordered draws, transforms, LOD/animation/material/FoW state. Re-export retained frame first; C10 needs labelled stable IDs only if that join is impossible. |
| M02 | War-cluster/CAP11 candidates and FILE combat sprites/layouts. | yes / yes / no semantic TLW | Stop identity is unverified; no stable combat ID, progress/direction/result state or labelled land/naval case. Re-mine signatures first; C10 must label those state transitions if absent. |
| M03 | Retained war/postfx arrow draws and W21 fixed-point producer chain. | yes / yes / partial | Draw pipelines exist, but named order IDs, VB/IB preceding writes, width/join/cap/head state and comparator ties are not fully joined. Re-export before C10; capture only missing named/tie cases. |
| M04 | 153 immutable rail VBs from frame54162. | creation+draw / yes / immutable creation only | Width 1.0, Bezier centerline, U/direction and monotone chain-step V/atlas bands are closed. Exact province anchors, true mid-fade alpha/update order, A8 built/damaged state and hub frame are absent. C07 needs a labelled isolated rail plus temporal mid-fade only after retained re-mine. |

## Closed facts that must not trigger new capture

- O03 tree camera-height fade is closed as
  `clamp((350 - cameraY) / 100, 0, 1)`, ULP-exact over 1,080 draws.
- Border U/direction and monotone chain-step V are closed from the retained VB.
- Resource mode has no vanilla political fill.
- Slot-14 world/tree mask has no shipped-file source: it is a runtime-generated
  R8 resource. The remaining work is an RE packet tracing `trees.bmp` to the R8
  generator, not a request to find a missing shipped bitmap.
- The shadow referee contains 129 full-chain frames at 28 sun positions.
- The camera referee contains 2,820 matrix-complete CB0 samples.
- The old “1414 sweep” camera label is quarantined; it must not replace the
  2,820-sample count.
- Rail/border-ribbon U direction and monotone chain-step V are already closed
  from the retained VB; M04 must not recapture them.
- O03 is not one of the 35 active packets. Its corrected 1,080-draw result is
  retained only as a closed producer input.

## Exact missing capture facts

The registry distinguishes `RE_MINE_FIRST` from `CAPTURE_REQUIRED`. Generic
scene reshoots are not justified. The surviving exact shot-card facts are:

| Capture | Exact absent fact; required evidence |
|---|---|
| C01 / L01 | Formula discriminator for moon z: moon-height-only, sun-latitude-only and map-height-only cases; producer inputs and CB preceding write. |
| C02 / L04 | Over-range dark/bright temporal series with frame timestamps, pre/post luminance, every reduction, previous/current 1x1 values before/after update, first-frame and pause state. |
| C03 / L05 | Default versus `BLOOM_WIDTH`-only wide case with kernel CB, weights/offsets/taps, bright-pass input, both blur intermediates, restore consumer and final target. |
| C04 / L06 | Visible northern ice on both sides of date/latitude boundary, camera inside 100-350, A2 SnowMud input, slots 6/7/9 and water output. |
| C05 / L02-L03 | Only controlled independence and semantic joins missing from the 129-chain/28-sun referee: fixed camera with changed sun/date, fixed sun/date with changed camera, stable draw identities. Do not capture another uncontrolled shadow frame. |
| C06 / C01-C04,T00-T01 | One resize and labelled wheel/key/clamp/wrap sequence; atomically record target/current camera, clock, canonical/west/east matrices and planes, root flags, map width, and `*(map+0x4970)` for two consecutive stable updates. |
| C07 / O04-O05,M04 | Separate labelled city, map-object and isolated rail scopes with matrices/instance buffers, states/SRVs/CBs and true-last-writes; rail additionally needs a true mid-fade temporal sample. O01/O02/O03 are excluded. |
| C08 / N02-N04 | Only surviving discriminators: three strong-concavity cases if absent, topology edge cases unresolved by FILE graph, and one controlled N04 selector transition. |
| C09 / R02-R03 | One-variable river palette/define changes and labelled adjacency types, only after retained river/strait re-mine. |
| C10 / M01-M03 | Stable unit/combat/order IDs and named state transitions only where frame54162/war-cluster re-export cannot supply semantic joins. |

B01, B04, B06, R01, O04-O06, N04 and M01-M04 still have retained event
streams or bound native payloads to mine. Their capture status is therefore not
an unconditional authorization. B02, B03, B05, C00, O01, O02 and N01/N03
primarily lack offline producer/data facts, not pixels.

## Corpus-wide fail-closed finding

The integrity registry covers 87,449 artifacts in 211 components and is
`FAIL_CLOSED`: all 211 components carry at least one identity or event-scope
issue; 198 have an event stream without validated last-write semantics, 38
legacy components lack the source RDC, and only five focused components carry
last-write markers (still not semantically validated). The retained corpus is
large and highly reusable, but generic mined presence never satisfies the
capture-law gate by itself.

## Limits

This lane does not promote a rendering law. Artifact presence is not semantic
proof, event markers are not event identity, and resource contents without a
consumer-scoped preceding writer are not true-last-write evidence.
