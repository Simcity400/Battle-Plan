# XN13P — ground-unit and building production replay handoff

Status: **PROVEN — PRODUCTION HANDOFF READY / FAIL CLOSED**

Pinned binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XN13P freezes the production replay envelope for dynamic units and buildings. It consumes final XN13I as an opaque, already-verified composition of XN13 main, XN13B, and XN13C; it adds O05 building/fixed-object presentation, XN15B2 building effect/light lifecycle, and the exact B8/X6 extractor building registry. It does not duplicate any upstream renderer, simulation, selection law, or packet producer.

The normative schema is `replay_interface/re_evidence/rendering_laws/XN13P/production_handoff_contract.json`.

## Immutable unit contribution

The final XN13I tuple is admitted only at these exact SHA-256 identities:

| Artifact | SHA-256 |
|---|---|
| `XN13I.md` | `a4ff7a8c38210cfef61d34085add658779b4c513a7866f95f9019d877e516812` |
| `integration_contract.json` | `5d55c81a395b98f778947dcf3b758b382163b47d0a024c434ad873ead71616fb` |
| `manifest.json` | `c16f512b09a0ae10fe270ba0c92ee1938f69882c568e1c9b338b7eefacf49f49` |
| `verification.json` | `7357ae5494a3c4d9e693a8186d29acfba64d5cfcc78d07a790d0b933b7613cc8` |

XN13P mechanically rechecks the XN13I manifest's embedded XN13 main, XN13B, and XN13C hashes, PASS dispositions, zero-residual declarations, shared binary/catalog identity, and two-run idempotence proofs. It neither edits nor re-serializes their internal row schemas. The unit contribution is one nested `xn13i_dynamic_unit_frame` joined to the XN13P clock, sequence range, and checkpoint epoch.

The inherited exact closure remains 1,239 retained unit draws / 413 objects; 152 unique pose palettes; 171 dynamic trail/wake associations; 4,238 ground entities; 18,796 ground states; 10,392 ground visual events; 10,391 resolved ground particle rows; and 609/609 resolved ground lights.

## Exact building presentation population

The building census combines O05 semantic/draw evidence, XN15B2 visual-event producers, and B8/X6 numeric type identity:

| Item | Exact result |
|---|---:|
| O05 semantic static objects | 281 |
| O05 fixed objects admitted | 4 |
| vanilla static source rows parsed | 66,664 |
| retained O05 objects / draws | 50 / 150 |
| retained static buildings / landmarks / furniture | 46 / 1 / 3 |
| building/landmark visual events | 358 |
| building particle events | 357 |
| building light events | 11 |
| resolved / explicit-null lights | 11 / 0 |
| light-only building events | 1 |
| resolved entity tokens | 63 |
| destroyed-entity-token events | 266 |
| B8/X6 registry entries | 55 |
| observed exact / independently uninstantiated mappings | 43 / 12 |
| registry ambiguity / mismatch | 0 / 0 |

The 358 visual events are selected by resolved effective-VFS source identity: `gfx/entities/buildings.asset` contributes 146 and `gfx/entities/landmarks.asset` contributes 212. Token prefixes are not the classifier. Their complete canonical SHA-256 is `c4411ad70abb77303f60e400bcfb8f4f06179e8fe2e1a12743d3827292d28cb0`.

The only particle-null row is valid and positive: `building_mega_gun_emplacement.idle`, state ordinal 0/event ordinal 2, resolves `alarm_flash`. Replay emits the observed light lifecycle/output and no particle; it never aliases or fills the absent particle.

## Building identities, selection, and transforms

Every live row has stable `building_instance_id` and the exact O05 semantic key:

`map/buildings.txt@<raw-resolved-source-sha256>#L<one-based-source-line>`

The row additionally retains the extractor's exact `runtime_building_type_id` and `building_type_token`. These identities are complementary: the 55-entry registry names the engine type while the O05 source key identifies the placed object. Neither replaces the other.

Resolved presentation records ordinal, entity/model, health or destroyed variant, material/shape, source insertion ordinal, authored XYZ/yaw bits, exact 64-byte world matrix, scale-in/out bits, and transform update index. Visibility records POV/FoW decision, raw camera distance, maximum-visibility input where applicable, exact camera shrink-gate result, inclusion, and explicit exclusion reason. Replay consumes the recorded results; it does not infer health variants, FoW, camera gates, or transforms from aggregate state.

O05 admits four fixed objects only when all eight frozen source hashes match. Any missing/changed source rejects the complete fixed set. Every other unproven fixed/ambient/landmark candidate remains absent, not approximated.

## Effects, draw order, teardown, and checkpoint

Building effect/light rows bind `building_instance_id` to stable entity/effect/light IDs, exact entity/state/event catalog identity, ordered authored payload, actual trigger sequence/update/tick, and resolved node/world matrix. XN15B2 remains lifecycle and submitted-output authority.

Draw rows retain O05 insertion ordinal, three pass references, exact matrix, mesh/shape/material/resource hashes, and pipeline-state dictionary IDs. The 50-object retained sequence is identical across all three passes. Sorting by type ID, semantic key, entity, model, health, effect, or spawn time is forbidden.

Teardown retains draw-membership removal, effect stop/destroy, graphics-handle destruction, and building-instance destruction in observed engine order. No child or draw survives implicitly.

Each XN13P checkpoint nests the complete XN13I checkpoint and freezes every active building identity/type/source join, health/variant/visibility state, transform, effect/light association, draw/state reference, next stable ID, and source/catalog hash. Checkpoint decode plus forward replay must be byte-identical to linear playback. Unit and building populations are admitted or rejected atomically at one update boundary.

## Federated Ghidra evidence

XN13P adds no binary claim and uses immutable upstream artifacts. Each decisive row records address, job, artifact path, and manifest hash.

| Address / finding | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14227b9a0` exact unit state-token lookup | `20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259` | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14227b9a0.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14227ac70` animated node/attachment transform | `20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847` | `C:\Users\Example\ghidra_agent_exports\20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847\evidence\func_14227ac70.decompile.c` | `016d5dd925d197b6dd232b359cf059b8c6a8363c86898d42e0bbba76cb11602b` |
| `0x1409bc400` particle attachment | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_1409bc400.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x14165f630` building record/handle order catalog | `20260716T050259Z-o05-runtime-record-handle-order-catalog--b3869614` | `C:\Users\Example\ghidra_agent_exports\20260716T050259Z-o05-runtime-record-handle-order-catalog--b3869614\evidence\func_14165f630.decompile.c` | `01a3e41e6541f9f11fc35ddb0dbbd7350521f16b0f91271343aa421c0663df29` |
| `0x14165d5c0` desired-count/validity selection | `20260716T041136Z-o05-runtime-desired-count-validity-speci-7b04be6e` | `C:\Users\Example\ghidra_agent_exports\20260716T041136Z-o05-runtime-desired-count-validity-speci-7b04be6e\evidence\func_14165d5c0.decompile.c` | `2e744d3b5e94f2af4ad72e0ca1259c4808d3d0dc0684854dca3189626953e438` |
| `0x141663e70` live graphics-handle destructor caller | `20260716T052658Z-o05-live-graphics-handle-destructor-call-707b7e02` | `C:\Users\Example\ghidra_agent_exports\20260716T052658Z-o05-live-graphics-handle-destructor-call-707b7e02\evidence\func_141663e70.decompile.c` | `b4f7c807f5af526716e2fc55af47e1324f3a42ef6e172503e3e09467b86f008c` |
| `0x1416648d0` graphics-handle destruction | same job | `C:\Users\Example\ghidra_agent_exports\20260716T052658Z-o05-live-graphics-handle-destructor-call-707b7e02\evidence\func_1416648d0.decompile.c` | `b4f7c807f5af526716e2fc55af47e1324f3a42ef6e172503e3e09467b86f008c` |
| `0x14127b2e0` map-building production update loop | `20260716T052901Z-o05-mapbuilding-production-update-loop-v-ea7ac957` | `C:\Users\Example\ghidra_agent_exports\20260716T052901Z-o05-mapbuilding-production-update-loop-v-ea7ac957\evidence\func_14127b2e0.decompile.c` | `bcca6ce2eb8491d2df2c8a0562d14c29dcd7bc4486d1fff019235c42879cbb6b` |

No new export was semantically required. All relied-on tuples and upstream reports are independently hash-pinned; no guessed address, fitted rule, or decompile-only promotion was introduced.

## Promotion result

The verifier rebuilds the building census, validates the exact final XN13I tuple and its embedded XN13/XN13B/XN13C identities, checks O05 and B8/X6 PASS evidence, verifies building-event/draw populations and all schema/checkpoint fields, and requires every Ghidra tuple above. There are no XN13P data, capture, RE, or human-input residuals.

Run:

```text
python replay_interface/re_evidence/rendering_laws/XN13P/verify_xn13p.py
```
