# XN12A — particle/effect registration, resolution, ownership, and lifetime

Disposition: **PROMOTION READY / FAIL CLOSED**

## Scope and authority

This packet closes the non-UI producer side of particle/effect rendering: game-file registration, effective resource resolution, entity-event submission identities, runtime allocation/ownership/attachment, and destruction. Per-frame particle simulation, PRNG, sorting, vertex production, render-state, and draw ordering are XN12B. Semantic trigger scheduling is XN15. Weather-manager selection is XN14. Those are typed consumers of this law, not XN12A residuals.

The executable authority is `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`. The Ghidra pool was ready with ten shards before work began. The full machine-readable address/job/artifact/hash ledger is `replay_interface/re_evidence/rendering_laws/XN12A/ghidra_provenance.json`; it is normative where this report keeps the prose concise.

## Production law

1. Resolve the active game namespace with the UI15 ordered-VFS contract before parsing any particle file. Archives are materialized first; later layers win case-folded relative-path collisions; `replace_path` removes only lower-layer files in the exact immediate-parent directory. DLC folder enumeration order in the census is inventory-only and must never substitute for the launcher-provided active layer order.
2. Parse entity event fields `{time, node, particle, keep_particle, trigger_once}`. Resolve `particle` through a `pdxparticle = { name, type, scale }` registration, then resolve `type` to the `name` of a `particle = {...}` definition and its subsystem texture/shader resources.
3. Registry lookup is case-sensitive FNV-1a32 over the effect-name bytes: seed `0x811c9dc5`; for each byte, `(hash XOR byte) * 0x01000193` modulo `2^32`. The manager vector is at `+0x20`, count at `+0x2c`, stored type hash at `+0x80`; the engine returns the first hash match and performs no collision string comparison. The exact installed vanilla-plus-integrated-DLC-plus-DLC inventory has 180 unique registration names and no FNV collision. Production must repeat the collision check after resolving the active VFS, and reject any post-VFS duplicate name or FNV collision unless it also retains the exact engine registry insertion stream.
4. `CPdxParticleObject` owns one runtime instance pointer at object `+0x70`. Construction passes the resolved type to the manager creator; a null type returns no instance. A successful spawn allocates `0x58` bytes, constructs the subsystem instances, initializes the active byte at instance `+0x54` to one, and appends the pointer to the manager vector at `+0x8`/count `+0x14`.
5. Attachment is pointer identity, not a copied transform: runtime instance `+0x40` points to the owning particle object's transform at object `+0x10`. The declared `pdxparticle.scale` is copied to runtime instance `+0x48`.
6. Submission places the owner object into one of six render buckets and sets runtime byte `+0x55`. The per-frame bucket walker later passes the same object `+0x70` pointer into the particle update/render entry. XN12B owns everything after that handoff.
7. Destruction is exact ownership teardown. The object destructor calls the manager destroyer on object `+0x70`, clears the owner field, removes the exact pointer from the manager vector, destroys instance/subsystem buffers, and frees the instance. Registered particle-type removal is separately proven at `0x1423e82c0`.
8. Missing effects fail closed. The parser stores the requested name, resolves a null pointer, and emits `Could not find particle effect [NAME] near line N in file PATH`. A null type cannot spawn. Production must not guess, alias, or substitute another effect.

## Complete a729 census

`build_xn12a_census.py` scanned vanilla, all four shipped `integrated_dlc`
layers, and every installed loose DLC `gfx` namespace with the shared Paradox
parser. This matters: the parallel `dlc/dlc023_man_the_guns` path is an empty
licence placeholder, while the runtime particle payload is under
`integrated_dlc/dlc023_man_the_guns`. Every source row is preserved separately
so precedence is explicit rather than silently guessed.

| Item | Count |
|---|---:|
| particle-bearing `.gfx` / `.asset` files parsed | 280 |
| `pdxparticle` registrations / unique names | 180 / 180 |
| particle definitions / unique names | 147 / 146 |
| subsystems | 511 |
| texture references | 511 |
| entity particle events | 18,238 |
| parse errors | 0 |
| unresolved registration types | 0 |
| unresolved texture paths | 0 |
| unresolved event particles | 0 |
| FNV registry collisions | 0 |

Family counts are row counts and may overlap because one effect can legitimately be smoke plus fire, or wake plus trail:

| Family | Rows |
|---|---:|
| combat | 7,836 |
| trail / exhaust / tracks | 6,361 |
| smoke | 2,810 |
| fire | 2,226 |
| bombing | 1,181 |
| wake / water | 1,044 |
| other | 669 |
| weather handoff to XN14 | 132 |

The 511 subsystem texture bindings resolve to `ParticleAdditive` 282 times,
`ParticleAlphaBlend` 228 times, and `ParticlePreAlphaBlend` once. The exact
event-flag census is: `keep=no, trigger=no` 205; `keep=no, trigger=yes` 6,240;
`keep=yes, trigger=no` 11,130; `keep=yes, trigger=yes` 663. XN12A preserves
these authored flags losslessly as trigger inputs; XN15 owns their state-machine
scheduling. No temporal behavior is invented from the field names.

The only particle-relevant relative-path collision is `gfx/particles/infantry/mg_muzzle_tommy.asset`: vanilla and DLC040 define `mg_muzzle_tommy_file` with different bytes. This is an ordinary UI15 later-layer file override, not two definitions in one effective namespace.

## Integrated-DLC reconciliation

The earlier 16,695-row census incorrectly classified four
`amphibious_move_water_particle` events as null/no-spawn because its source
enumerator omitted `integrated_dlc/*`. That finding is **refuted and preserved
as a corrected negative finding**, not erased.

The globally unique registration is
`integrated_dlc/dlc023_man_the_guns/gfx/entities/MTG_particles.gfx`, SHA-256
`94fbf5338bb1d70df4f9ba4adbf3f06332661e2aa802a407245e190bc7815723`:
name `amphibious_move_water_particle`, type
`amphibious_move_water_effect`, scale `1`. Its definition is
`gfx/particles/vehicles/amphibious_move_water.asset`, SHA-256
`ff79d062c25c4315424da028e54cdb3d8240d7a96e87353848b9c4e4f6501189`.
The two DLC049 and two DLC052 consumers retain time `0`, node
`front_moving_water_effect`, `keep_particle=yes`, and `trigger_once=yes`; all
four now resolve exactly. `vfs_reconciliation.json` freezes their consumer
paths, line numbers, hashes, the direct registration/definition evidence, and
the XN15B integration cross-check. XN12A now has zero unresolved event names.

## Capture and game-file audit

The retained RenderDoc corpus was inspected through the frozen M01 corpus audit: 88 eligible immutable mined frames, 24 with authenticated unit-signature matches, and no scan errors; source SHA-256 `59d51a8a22133ab7e8588ad6bb3009508831ea0fb457adfdc55a9aed2e4b8288`. It corroborates that the retained capture corpus is extensive and usable. XN12A does not infer semantic effect names from pixels: registry identity, resource paths, and event flags come exactly from the effective installed content, while runtime ownership comes from Ghidra. XN12B/XN14 may bind captured draw signatures to this registry without changing the identity law.

## Ghidra proof surface

Every proof row below records the required function address, job, artifact, artifact hash, and manifest hash. The JSON ledger contains the complete set, including bounded grammar xrefs and incidental diagnostics.

| Law | Function | Job | Artifact and SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|
| missing-effect resolution and diagnostic | `0x1422824a0` | `20260717T001401Z-xn12a-particle-lookup-and-missing-4c34b662` | `...\evidence\func_1422824a0.decompile.c` — `96e25d1aeab544e56fc981870191f41bc2f3c14d58221147453786add0a20495` | `b8bd2f1b4575afc30af71a0f34b3f8278ddef4dcc8c8ada6d988ef4d73d006cb` |
| global manager | `0x1423e8b80` | `20260717T002544Z-xn12a-particle-global-registry-lookup-d021786f` | `...\evidence\func_1423e8b80.decompile.c` — `43a1ec11443fe61f4d9949151ae14fb54215b5c4d4c1aa9ac4828841f36335e6` | `8ac0771e940cc091900793046deee7242dc4f7875a02bde6c85d6c227e6a8757` |
| FNV registry lookup | `0x1423e8590` | same | `...\evidence\func_1423e8590.decompile.c` — `c2f2885360e342e0dc1f24841137f9dae3b2d7c195bf97808e00a52e025e681b` | same `8ac0771e...` |
| type identity construction | `0x142281a80` | same | `...\evidence\func_142281a80.decompile.c` — `6d39036862c8f67fd370bb2a14a15755e65eeb63bea4cf0e7f0eebb2f780efd7` | same `8ac0771e...` |
| null re-resolution / broken-type diagnostic | `0x142282600` | same | `...\evidence\func_142282600.decompile.c` — `0d238eab12c429ecd5219b163a4257018582feadc066f53b662a02aafe3cd02f` | same `8ac0771e...` |
| type finalization and renderer variants | `0x142281b00` | `20260717T003118Z-xn12a-particle-type-finalize-and-runtime-8dae0847` | `...\evidence\func_142281b00.decompile.c` — `7490a2e152ac8745f7396babb4b1d4013c9bf987de9581b907792f09358bc459` | `5cbe768b68a8fb98b42f6d576f42622fc9a19113290ecacc9e3ef0051ee76bfa` |
| runtime instance create | `0x1423e8190` | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `...\evidence\func_1423e8190.decompile.c` — `35c70015b603ae24289cfa4fa4ce4581ca1b7eb5def5f1c138cc7f709f4d8c7f` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| runtime instance destroy | `0x1423e8400` | same | `...\evidence\func_1423e8400.decompile.c` — `69a05e32081a6f33e762227a5f87b1032f983ce037a90bd3d35c8c90359e588b` | same `45770eba...` |
| object construction and ownership | `0x14227ea10` | same | `...\evidence\func_14227ea10.decompile.c` — `d874682980aaec407421102083e831421349a8a4cf0c156cf7defe75bb87795a` | same `45770eba...` |
| transform attachment pointer | `0x1409bc400` | same | `...\evidence\func_1409bc400.decompile.c` — `a137233b3854434deda5624ae0d32115c57fafd49317f0d2abb95ace9ee7475a` | same `45770eba...` |
| declared scale to runtime | `0x1423e9190` | same | `...\evidence\func_1423e9190.decompile.c` — `933243ec48a19fe4f9c9463f184999e49b191665ff5f7d1fde55fe939b473ceb` | same `45770eba...` |
| object destruction | `0x14227ec50` | same | `...\evidence\func_14227ec50.decompile.c` — `74c51abd23fda7f572e0c4154dc736c0f732314a9474ae1294f5f7b5ad61d63c` | same `45770eba...` |
| render-bucket submission | `0x14227ecd0` | same | `...\evidence\func_14227ecd0.decompile.c` — `4e440b78e865f0122291483af26e1a14527f688e7aedeec97738baaf79ffcce9` | same `45770eba...` |
| active/deactivated/live counters | `0x1423e8500` | `20260717T002857Z-xn12a-particle-manager-instance-counts-12f58cd5` | `...\evidence\func_1423e8500.decompile.c` — `226abf5e05ae633b68cd006b2259cb53f69446f6d59bbc9179e94da76fdebb2d` | `88bfec71f5cffc1bf04c0cdbef2a045b5491da02cd1648de6b9dd8912cfffc64` |
| type deregistration/destruction | `0x1423e82c0` | `20260717T003149Z-xn12b-particle-manager-neighborhood-map-fb43af7e` | `...\evidence\func_1423e82c0.decompile.c` — `7852d9e251ede8dece50c4b0f5eea5fc1ddae3fa26a6dbc58e8ce85823bb0017` | `89ac3effd686701889451d1eb5041bf26cb11da1b13b8a7354aa8a1135e78db4` |
| object-to-per-frame handoff | `0x14227f420` | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `...\evidence\func_14227f420.decompile.c` — `97c2fa8184aa73fc1aa5e6994fd563fbc563ea5c61a30a75a1b4f8286e954d8f` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |

The monolithic parser function `0x140082cb0` is too large for the decompiler and produced a failed export log rather than a function manifest. This is recorded in the JSON ledger as an incidental tool result, not used as a missing proof: exact string xrefs bind its grammar, and the separately decompiled lookup, constructor, manager, and destructor consumers close the runtime law.

## Mechanical gate and promotion handoff

`verify_xn12a.py` passes **22/22** checks on the corrected integrated-DLC census
with status `PROMOTION_READY_FAIL_CLOSED`. `verification.json` SHA-256 is
`306430bc5643d2db4813f371650e2b9122286e00525e22f5269e71dfe8886eec`;
`census.json` SHA-256 is
`c736085bbea45ba6688cccbaef3810b11d3d4590f435ed9ea6f1768f514d0b0c`.

The production replay handoff must retain:

- exact effective-VFS identity and content digests for the event asset, registration file, particle definition, textures, and shader selection;
- the exact effect name plus verified FNV key;
- producer/owner stable identity, state-event ordinal, `time`, `node`, `keep_particle`, and `trigger_once`;
- parent/node world-anchor identity and transform stream;
- explicit spawn, deactivation/stop, destruction, and checkpoint lifetime events supplied by XN15/XN12B;
- deterministic null records for unresolved names, with no aliasing.

There is no XN12A data blocker and no XN12A capture blocker. Once XN12B and XN15 supply their independently owned temporal state, this registry/lifetime packet is ready to promote unchanged into the production replay layer.
