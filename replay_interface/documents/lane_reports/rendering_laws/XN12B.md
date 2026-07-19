# XN12B — particle/effect timeline, simulation, sorting, and render law

Pinned target: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

Status: **PROVEN — promotion-ready**. No binary, game-data, or retained-capture
dependency remains for the production extractor/replay contract. Exact replay
must use the post-simulation packet stream below; effect name plus start tick is
not equivalent.

## Promotion ledger

| ID | Proven law |
|---|---|
| XN12B-01 | Authored time modes are exactly `spawn=0`, `system=1`, `life_abs=2`, `life=3`; parser default is `spawn`, and unknown spelling is invalid. |
| XN12B-02 | Runtime instance is `0x58`: RNG `u32[2]` at `+0x20`, active `+0x54`, update-selected `+0x55`, scale float3 `+0x48`. Creation seeds both RNG words from the high-resolution wall clock, not game tick or effect id. |
| XN12B-03 | Static subsystem stride is `0x8f8`; runtime stride is `0x88`. Runtime owns `max_amount * 0x70` particle records, `max_amount * 0x30` transforms, and `child_count * max_amount * 0x88` child runtimes. |
| XN12B-04 | `0x142219c10` supplies elapsed seconds and camera vectors to `0x1423e9500`. It filters active instances, substeps the whole elapsed interval, parallel-updates, then depth-sorts. |
| XN12B-05 | Recursive simulator `0x1423ddaa0` advances ages, removes expired records, accumulates/caps emission, initializes spawns, evaluates authored curves/forces, integrates position, applies colour/alpha, writes live counts, and updates child systems. |
| XN12B-06 | Turbulence/noise uses a deterministic counter+seed integer hash. Nondeterminism is limited to capture of the initial wall-clock-derived words and the exact frame/substep schedule. |
| XN12B-07 | Camera sort is `0x1423dfd30`. Traversal preserves subsystem/child recursion and assigns a monotonically increasing packet ordinal. |
| XN12B-08 | Render packet stride `0x70`: matrix `+0x00`, live count `+0x40`, particle pointer `+0x48`, record stride `+0x50`, max `+0x54`, transform stride `+0x58=0x30`, flags `+0x5c/+0x5d`, ordinal `+0x60`. |
| XN12B-09 | SIMPLEPARTICLE GPU stride is 48: `position.xyz,size` @0, yaw/pitch/roll degrees @16, `R8_UINT tile_x` @28, `B8G8R8A8_UNORM` @32, 12 padding. Quad UVs are `(0,0),(0,1),(1,0),(1,1)`; R16 indices `0,1,2,2,1,3`. |
| XN12B-10 | Billboard uses roll and projection-diagonal clip offset; no-billboard uses yaw/pitch/roll plus instance-world matrix. RGB is raised to 2.2, alpha unchanged; atlas V flips; texture RGBA multiplies vertex colour. |
| XN12B-11 | Blend variants are alpha `SrcAlpha/InvSrcAlpha`, additive `SrcAlpha/One`, pre-alpha `One/InvSrcAlpha`, RGB mask only. Depth is enabled/write-off/`LESS_EQUAL`; cull is back for billboard and none otherwise; sampler is linear/wrap. |
| XN12B-12 | `0x14227f420` binds matrix/material/texture/pipeline and submits instanced packets, then its ribbon/trail tail. Captures place particles after map objects and before arrows/restore. |

## Production extractor/replay contract

Capture a **post-simulation particle packet stream** after sorting and at the
`0x14227f420` submit boundary. For every frame, bucket, and packet, serialize in
engine order:

1. stable effect/definition identity plus material, pipeline, and texture IDs;
2. packet ordinal, bucket, ribbon/trail flags, and exact 4x4 matrix;
3. live count and exactly `live_count * 48` GPU-instance bytes;
4. atlas rows/columns, `Scale`, fog/blend branch, cull/depth/sampler state;
5. frame camera/projection constants used by the billboard branch.

Replay submits the static six-index quad once per packet, consumes the captured
instance bytes without re-simulation/re-sorting, and preserves packet/bucket
order including the ribbon/trail tail. Identical state can be dictionary-coded
and instance blocks delta-compressed, but each checkpoint must be independently
decodable.

Event-only cold seek would additionally require both initial RNG words, every
elapsed-seconds/substep value, active/scale and attachment-transform transitions,
all `0x88` subsystem state, live `0x70` records, `0x30` transforms, spawn
accumulators, child runtimes, and camera vectors. Thus effect token + start tick
plus nominal seed is not exact. The packet stream is the fail-closed contract.

## Retained-capture closure

Mechanical re-mining of the exact SIMPLEPARTICLE VS hashes gives:

| Capture | Draws | Instances | First–last event |
|---|---:|---:|---:|
| `hoi4_frame35240` | 28 | 416 | `25706–26057` |
| `hoi4_frame69242` | 41 | 614 | `28456–28949` |
| `postfx_frame8467` | 427 | 6,287 | `40386–45550` |

The evidence result byte-decodes one 48-byte instance from each frame and checks
the 12-byte padding. Shipped `particle.shader` and the retained W10 capture-law
report are hash-pinned; their static quad, shader/state law, and 26 exact atlas
joins remain authoritative. Texture identity alone is never effect identity.

## Ghidra evidence ledger

Every finding records address, job, artifact, and manifest hash. Full tuples are
also machine-readable in `XN12B/mechanical_result.json`.

| Address / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14231aa80` time enum | `20260717T001427Z-xn12b-particle-time-mode-readers-v2-fdfd7871` | `C:\Users\Example\ghidra_agent_exports\20260717T001427Z-xn12b-particle-time-mode-readers-v2-fdfd7871\evidence\func_14231aa80.decompile.c` | `6535b878b9fa8234741743b390efc6fcaaebe8eea7220aaf0e82279046bb9a4b` |
| `0x142317980` time parser | same job | `C:\Users\Example\ghidra_agent_exports\20260717T001427Z-xn12b-particle-time-mode-readers-v2-fdfd7871\evidence\func_142317980.decompile.c` | `6535b878b9fa8234741743b390efc6fcaaebe8eea7220aaf0e82279046bb9a4b` |
| `0x1423e7720` definition loader | `20260717T003149Z-xn12b-particle-manager-neighborhood-map-fb43af7e` | `C:\Users\Example\ghidra_agent_exports\20260717T003149Z-xn12b-particle-manager-neighborhood-map-fb43af7e\evidence\func_1423e7720.decompile.c` | `89ac3effd686701889451d1eb5041bf26cb11da1b13b8a7354aa8a1135e78db4` |
| `0x1423e8190` instance constructor | same job | `C:\Users\Example\ghidra_agent_exports\20260717T003149Z-xn12b-particle-manager-neighborhood-map-fb43af7e\evidence\func_1423e8190.decompile.c` | `89ac3effd686701889451d1eb5041bf26cb11da1b13b8a7354aa8a1135e78db4` |
| `0x1424dbe60` wall-clock seeding | `20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d` | `C:\Users\Example\ghidra_agent_exports\20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d\evidence\func_1424dbe60.decompile.c` | `faabb041c2eaa89bdc9a93a81535073aa5189d0e09a9322f46ea613b8b9ad1d5` |
| `0x1423dcc80` runtime allocation | `20260717T004108Z-xn12b-particle-instance-render-update-co-75672612` | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423dcc80.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |
| `0x1423deb60` runtime rebuild | same job | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423deb60.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |
| `0x142219c10` dt/camera caller | `20260717T005102Z-xn12b-particle-runtime-neighborhood-grid-b4bc7186` | `C:\Users\Example\ghidra_agent_exports\20260717T005102Z-xn12b-particle-runtime-neighborhood-grid-b4bc7186\evidence\func_142219c10.decompile.c` | `623dbe4e353e668f9d039192763102a682ebb6d76f8f633f379224f2c8688d91` |
| `0x1423e9500` update/sort root | same job | `C:\Users\Example\ghidra_agent_exports\20260717T005102Z-xn12b-particle-runtime-neighborhood-grid-b4bc7186\evidence\func_1423e9500.decompile.c` | `623dbe4e353e668f9d039192763102a682ebb6d76f8f633f379224f2c8688d91` |
| `0x1423dabe0` update worker | `20260717T005200Z-xn12b-particle-update-sort-workers-7b289392` | `C:\Users\Example\ghidra_agent_exports\20260717T005200Z-xn12b-particle-update-sort-workers-7b289392\evidence\func_1423dabe0.decompile.c` | `4a8e7d57f6ba78d656c55098eea51249c41877ab7bbfc55a0ac3bf16bbfb41a6` |
| `0x1423ddaa0` simulator/live writer | same job | `C:\Users\Example\ghidra_agent_exports\20260717T005200Z-xn12b-particle-update-sort-workers-7b289392\evidence\func_1423ddaa0.decompile.c` | `4a8e7d57f6ba78d656c55098eea51249c41877ab7bbfc55a0ac3bf16bbfb41a6` |
| `0x1423e9ab0` deterministic hash/noise | `20260717T004108Z-xn12b-particle-instance-render-update-co-75672612` | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423e9ab0.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |
| `0x1423dfd30` camera sort | `20260717T005200Z-xn12b-particle-update-sort-workers-7b289392` | `C:\Users\Example\ghidra_agent_exports\20260717T005200Z-xn12b-particle-update-sort-workers-7b289392\evidence\func_1423dfd30.decompile.c` | `4a8e7d57f6ba78d656c55098eea51249c41877ab7bbfc55a0ac3bf16bbfb41a6` |
| `0x1423dd730` packet emission | `20260717T004108Z-xn12b-particle-instance-render-update-co-75672612` | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423dd730.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |
| `0x1423e8b90` render traversal | `20260717T003149Z-xn12b-particle-manager-neighborhood-map-fb43af7e` | `C:\Users\Example\ghidra_agent_exports\20260717T003149Z-xn12b-particle-manager-neighborhood-map-fb43af7e\evidence\func_1423e8b90.decompile.c` | `89ac3effd686701889451d1eb5041bf26cb11da1b13b8a7354aa8a1135e78db4` |
| `0x14227f420` bind/draw/ribbon submit | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `C:\Users\Example\ghidra_agent_exports\20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19\evidence\func_14227f420.decompile.c` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |

Mechanical verification:
`python replay_interface/re_evidence/rendering_laws/XN12B/verify_xn12b.py`.

There are no XN12B data-blocked or capture-blocked residuals.
