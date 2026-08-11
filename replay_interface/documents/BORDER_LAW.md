# Thin-border production law

## Authority and document role

This is the current normative XMAP06 thin-border law for `hoi4.exe`
`1.19.2.0.a729`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
XMAP06 is an unconditional packet in the authenticated product map
composition. The legacy `--borders-from-files` tooling is not a product route
and cannot replace or disable either XMAP06 phase.

Durable engine and capture evidence is recorded once in
`replay_interface/re_evidence/rendering_laws/XMAP06/PRODUCTION_EVIDENCE.md`.
The required live selector schema is recorded once in
`replay_interface/re_evidence/rendering_laws/XMAP06/RUNTIME_BUCKET_CARRIER.md`.
This file states the resulting production law; it does not duplicate promotion
history or validation receipts.

## Effective content and geometry

Production reads the admitted effective-VFS winners for `default.map`,
`definition.csv`, `provinces.bmp`, `heightmap.bmp`, `adjacencies.csv`, state
history, border textures, graphics defines, and the complete effective
`border.shader` translation-unit closure. A vanilla file is never substituted
for a mod winner.

Positive-height BMP rows are engine-scanned south-up. Replay therefore:

1. converts the loader's top-down province raster to a south-up engine view;
2. performs connected-region discovery and contour traversal in that view;
3. preserves the ordered-key, newest-first contour ledger; and
4. converts emitted lattice corners back with
   `j_top_down = height - 2 - j_engine`.

exist only at greater-than-two-region junctions. Open contours do not invent
ordinary endpoints. Closed contours append their first stored station after
the scan-order station list; they are not re-anchored or rewound by a local
heuristic.

The retained `session_BORDERZOOM_3` pool contains 40,959 ordered records. The
authoritative evidence gate compares the production ledger directly with that
pool and passes 40,959/40,959 without a residual.

## Smoothing and wrap

Land and coast records use the recovered fixed-end box-three path. Open and
closed `WaterWater` records use two fixed-end wide passes. For station `i`:

`radius = min(i, row_count - 1 - i, 6)`

Each x/z component is accumulated left-to-right with scalar f32 `ADDSS`,
divided by the f32 window count with `DIVSS`, stored to f32, and then consumed
by the second pass.

For a wrap-seam record, production translates the first stored station into
`[-0.5, map_width-0.5)` before either smoothing pass. Translating the result
after smoothing is numerically different and is rejected.

## Vertex, material, and constant-buffer law

Each vertex is exactly 16 bytes:

| Offset | Semantic | Format |
| ---: | --- | --- |
| 0 | position | `R32G32B32_FLOAT` |
| 12 | integer UV | `R16G16_SINT` |

UV packing is fallible; a value outside a signed 16-bit lane is an error.

Pixel-shader constant-buffer slot 2 is a 32-byte allocation containing
`float4 vSelectionColor`, `float2 vTime_Transparency`, and normal trailing
padding. `.x` is the authenticated presentation clock and `.y` is the
per-draw alpha multiplier. Final shader alpha is:

`max(texture_alpha, pulse - 0.2) * vTime_Transparency.y`

The exact material order is:

1. province;
2. state;
3. country;
4. impassable;
5. sea/coast; and
6. sea-region.

Every material uses its admitted effective
`map/terrain/border_<material>_0.dds` resource, including the complete mip
chain. Exact-a729 evaluates province and state alpha from their respective
effective graphics-define windows:

`alpha = 1 - (clamp(height, near, far) - near) / (far - near)`

Country, impassable, sea/coast, and sea-region are opaque. Impassable does not
use the state window. Only reason-zero absolute bucket 0 uses the province
ramp, and only reason-zero absolute bucket 1 uses the state ramp. Any nonzero
reason and buckets 2 through 11 use alpha 1; secondary buckets 6 and 7 are
therefore opaque despite reusing province/state textures.

## Runtime partition and viewer ownership


| Bucket | Bank | Material | Phase |
| ---: | --- | --- | --- |
| 0 | base | province | early |
| 1 | base | state | early |
| 2 | base | country | early |
| 3 | base | impassable | early |
| 4 | base | sea/coast | late |
| 5 | base | sea-region | late |
| 6 | secondary | province | late |
| 7 | secondary | state | late |
| 8 | secondary | country | late |
| 9 | secondary | impassable | late |
| 10 | secondary | sea/coast | late |
| 11 | secondary | sea-region | late |

The secondary bank is a per-record selector result and a separate submission
phase. It is neither duplicate geometry nor a global switch. Carrier v4
authenticates, for every gameplay POV, the complete camera-independent record
census, pair ordinal, base material, post-sort bucket draw ordinal,
reason/palette index, full ordered selection palette, exact suppression-branch
inputs, and payload digest.

Replay owns the current camera and map mode. It selects the authenticated POV
row table, derives province/state alpha from its camera height and current
collision-safe built-in/scripted map-mode identity, submits every non-hidden
row for its single main view, and lets the GPU clip offscreen geometry. The
recorder's transient frustum mask and final alpha are not playback authority.

Cache, prepared-frame, phase-resource, and draw identities bind the current
viewer map mode. CB2 is refreshed whenever the reason or replay-derived alpha
changes. The actual-draw receipt binds the prepared frame, main-view identity,
ordered draw stream, CB2 update census, and issued indices.

If carrier v4 or its selected context is absent, the family is typed
unavailable as `XMAP06.runtime_bucket_carrier_unavailable`. There is no
base-bank fallback.

## GPU state and product sequence

The captured pass establishes:

- triangle-list IA with the 16-byte vertex layout;
- `BorderDiffuse` at PS slot 0;
- source-alpha/inverse-source-alpha blending;
- RGB write mask `0x7`;
- depth test `LESS_EQUAL` with depth writes disabled; and
- the 32-byte CB2 described above.

The authenticated core-map order is:

1. terrain;
2. early border buckets 0 through 3;
3. rivers and straits;
4. rails, when a rail batch is present;
5. trees;
6. XMAP07 water; and
7. late border buckets 4 through 11.

The early and late calls consume distinct centrally assigned stages. Their
XMAP06 receipts bind the assignment ordinal, topology hash, assignment hash,
prepared frame, and actual draw stream. The sequence above is compositor
ordering: late borders follow rivers and straits, any admitted rails, trees,
and water. The final XMAP22 receipt later authenticates complete same-target
family execution.

## Fail-closed boundary

Production rejects missing or malformed effective content, wrong-build or
wrong-hour carriers, incomplete POV rosters, duplicate or reordered records,
invalid palette or bucket ordinals, stale viewer/map identities, incorrect
vertex/CB layouts, and misordered phase receipts. It does not substitute a
captured camera, recorder-selected map mode, visually tuned width, fallback
texture, files-only bucket partition, or legacy diagnostic pass.
