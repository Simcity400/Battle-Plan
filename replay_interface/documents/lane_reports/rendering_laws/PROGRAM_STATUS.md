# Rendering-laws RE program status

This is the orchestrator-owned live status surface for
`RENDERING_LAWS_RE_EXECUTION_PLAN.md`. Investigator lanes must not edit it.
Canonical renderer and fidelity documents remain unchanged until a packet has
passed its evidence, verifier, composition, and final-review gates.

## Frozen authority

- Workspace issue tree: current workspace HEAD recorded in
  `replay_interface/re_evidence/rendering_laws/AUTHORITY.json`.
- Replay authority: `C:\Users\Example\battle-plan_main` at
  `6aab56e455b39bdb9f17a38078f59b5a6a8d4755` (read-only for this wave).
- Program: `hoi4.exe 1.19.2.0.a729`.
- SHA-256: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Image base: `0x140000000`.
- Broker: read-only, no-analysis, serialized `hoi4_current/hoi4.exe` only.
- Plan SHA-256: `af1f48a8ceed22c017def9323816addfa3199132384ca365d301d6d3e939e0c0`.

## State vocabulary

- `ACTIVE`: owner is investigating and may enqueue bounded broker jobs.
- `QUEUED`: packet is issued but intentionally waiting for capacity or a
  producer contract.
- `EVIDENCE`: report exists and is undergoing mechanical reproduction.
- `CAPTURE`: static and retained-corpus work is exhausted; a discriminating
  capture/live-session card remains.
- `PROVEN`, `PARTIAL`, `REFUTED`, `BLOCKED`: terminal investigator disposition.
- `PROMOTED`: exact law passed composition and the final review wave.

`PROVEN` is not equivalent to `PROMOTED`.

## Non-UI packet register

| Packet | Owner | State | Primary dependency / next gate |
|---|---|---|---|
| B01 | `cont_b01` | ACTIVE | Endpoints/CB byte bound; runtime class mapping and ramp writer unknown |
| B02 | `b02_thin_water` | PARTIAL | 8,578/8,578 retained oracle exact; byte-bearing producer loop unknown |
| B03 | `b03_contours` | PARTIAL | Four retained records reproduced; exact construction/assignment producer unknown |
| B04 | `b04_gradient_topology` | PARTIAL | Static kernel exact; sort/writers/DatDump/engine fixture open |
| B05 | `b05_gradient_area` | PARTIAL | Cache/sum/interpolation proven; field producers and held-out join open |
| B06 | `b06_gradient_pack` | PARTIAL | Carrier descriptors/refresh bridge exact; alpha pack/upload/cadence open |
| L01 | `l01_moon_z` | CAPTURE | Three isolated retunes distinguish the two colliding formulas |
| L02 | `l02_shadow_matrix` | PARTIAL | 129-chain matrix composition exact; CPU owner/fit/update leaves open |
| L03 | `l03_shadow_drawset` | PARTIAL | 129 ordered draw sets exact; CPU list/filter/entity joins open |
| L04 | `l04_exposure` | CAPTURE | Static/order/ping-pong exact; retained values non-discriminating for easing |
| L05 | `l05_bloom` | CAPTURE | Static consumer verified; wide-width C03 selects normalization/host writer |
| L06 | `l06_northern_ice` | CAPTURE | Latitude/shader inputs exact; CPU activation/update join requires C04 |
| C00 | `cont_c00` | PROVEN | CAMERA-ROOT published; 12/12 jobs and 596/596 artifacts verify with zero open leaves |
| C01 | `cont_c00` | ACTIVE | Generic SIMD matrix law exact; canonical inputs/DatDump/viewport gate open |
| C02 | `c02_frontend_camera` | PARTIAL | 4/4 loader joins; 0/12 temporal leaves and CAMERA-ROOT unavailable |
| C03 | `c03_graphics_camera` | PARTIAL | Key arithmetic bound; wheel/clamp/state/order READ paths open |
| C04 | `c04_wrap_resize` | PARTIAL | Plane arithmetic exact; wrap/resize/runtime-state capture open |
| T00 | `t00_terrain_cull` | PARTIAL | 89-frame W14 reproduced; actual state/planes/flags 0/89 retained |
| T01 | `t01_offscreen_snap` | PARTIAL | Stateful law exact; T00 consecutive-owner composition dependency |
| R01 | `r01_river_builder` | PARTIAL | Full retained VB census exact; generator semantics/export unavailable |
| R02 | `r02_river_palette` | PARTIAL | FILE law exact; runtime meanings/consumers/order require mod capture/export |
| R03 | `r03_strait_mesh` | CAPTURE | 241 retained quads/UV law exact; filtering/generated endpoints/wrap open |
| O01 | `resume_ui01` | PROVEN | Exact PRNG seed/key/state/order and conversion pass 3,985/3,985 |
| O02 | `o02_tree_slope` | PROVEN | Exact producer/sampler law; 3,985/3,985 bit-equal slope pairs |
| O03 | removed | PROVEN | Closed by mining: `clamp((350-cameraY)/100,0,1)`, ULP-exact over 1,080 draws |
| O04 | `o04_city_generator` | PARTIAL | Static generator structure exact; TLW/ordered-instance C07 gate open |
| O05 | `cont_o05` | ACTIVE | Selector and 150/150 TLW exact; semantic/transform/pixel gate open |
| O06 | `cont_o06` | ACTIVE | R8 payload/slot-14 consumer exact; source transform/writer/cadence open |
| N01 | `n01_label_bbox` | PROVEN | Impassable-inclusive bbox is semantic-only; no renderer change |
| N02 | `n02_label_concavity` | PARTIAL | Only Catmull-Rom family; retained controls missing and port parameter mismatch found |
| N03 | `n03_label_topology` | PARTIAL | Consumer exact; graph admission policy requires export/C08 |
| N04 | `n04_label_alpha` | CAPTURE | 34/34 draws/conflict exact; selector causation/CPU producer open |
| M01 | `m01_units` | PARTIAL | 413 objects/1,239 draws exact; stable semantic/A7/A14 selection joins open |
| M02 | `m02_combat` | PARTIAL | 60 TLW chains exact; stable combat/progress/direction/result joins open |
| M03 | `m03_arrows` | PARTIAL | Geometry/FILE/shader sublaws exact; comparator/resources/A48/pixels open |
| M04 | `m04_rails` | CAPTURE | 153 VBs exact; raw province anchors and fractional-alpha sample open |

## Shared services

| Service | Owner | State | Deliverable |
|---|---|---|---|
| Retained-corpus inventory | `corpus_inventory` | PROVEN | 35/35 packets; 160 RDCs, 87,449 artifacts, 211 issue-bearing components routed |
| Existing-evidence registry | `evidence_registry` | PROVEN | 35/35 packets and 45 durable-job snapshot records reconciled |
| Serialized broker queue | orchestrator | ACTIVE | Exact-a729 10-shard pool verified; durable jobs are deduplicated before submission |

## UI packet register

Packet reports and packet-local findings ledgers hold detailed addresses,
artifacts, hashes, refutations, and incidental discoveries. This master surface
records only state and the single promotion gate.

| Packet | State | Primary dependency / next gate |
|---|---|---|
| UI01 | PROVEN | Exact oscillator/PRNG/event/replay-input law passes the pinned promotion verifier |
| UI02 | PROVEN | Exact event/state/frame/callback/sound law and retained oracle pass the promotion gate |
| UI03 | ACTIVE | Visibility/child propagation exact; draw suppression and animation arithmetic remain |
| UI04 | ACTIVE | `CInGameIdler+0x18` consumer/injection exact; per-frame writer and clock policy remain |
| UI05 | ACTIVE | Pulse/object state and inert census exact; selected-state-to-Channel2 pack bridge remains |
| UI06 | ACTIVE | FILE families and retained settled case bound; exact thumb fork/value/clip law across edge families remains |
| UI07 | ACTIVE | Input/repeat/rail/drag law exact; nested competing-target arbitration remains |
| UI08 | PROVEN | Two parsers agree on 157/157 records across 453 files; no authored scrollbar repeat timer |
| UI09 | PARTIAL | Authored focus/tech/doctrine dimensions exact; pan/clip/zoom/order/resize and doctrine row pixels require C11 |
| UI10 | PROVEN | 10,888 focus nodes, 438 tech edges, 155 doctrine edges and typed A22/A23 subsets reconcile exactly |
| UI11 | PROVEN | Exact frame/strip/font/glyph/pixel laws pass the pinned promotion verifier |
| UI12 | ACTIVE | Level-to-frame premise refuted and retained frames bound; locator transform and cutoff equality remain |
| UI13 | DATA_BLOCKED | Static law exact; authenticated per-field call/output/MXCSR CPU capture is absent |
| UI14 | PARTIAL | Product loose-file resolver gates pass; two-order exact-a729 loader trace still required |
| UI15 | PARTIAL | Static binder supports ZIP but renderer skips archive-only mods; engine mount collision matrix required |

## Review policy

No adversarial review runs during discovery, corpus mining, Ghidra export,
capture preparation, law synthesis, or initial integration. The sole review
wave starts only after all packet dispositions, evidence hashes, focused
verifiers, and cross-packet composition results are frozen.
