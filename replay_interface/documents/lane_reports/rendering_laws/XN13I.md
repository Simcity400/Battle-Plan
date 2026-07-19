# XN13I — final dynamic-unit presentation integration

Status: **PROVEN — PROMOTION READY**

Pinned target: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XN13I mechanically composes the immutable XN13 main unit/pose wrapper, XN13B
air/naval trail and wake association, and XN13C ground-unit presentation. It
adds no replacement renderer or duplicated reverse engineering. Its only new
law is exact admission: the three typed populations, their joins, and their
checkpoints are accepted atomically or rejected together.

## Immutable inputs

| Lane | Report | Contract / census | Verifier / verification | Manifest |
|---|---|---|---|---|
| XN13 main | `e4f3373ed4e6165828775544746af5a9bef6b1ee0400a94c1081adc30afce9a8` | contract `2311f7cf00a3562f74c4f020ae25081549cbb19305596d8d57e95474c6222723`; Ghidra ledger `f28620f6aee22ea8e0660f3ed4fe60f956c97b5c22ba37c14502dd1bf926a62b` | verifier `80a9e5a26e12afa314c177ed3b7502a09b2fc8aca02780e62fd07319125aa090`; verification `58624047c8bb5f85b5399fd06c96703bec878653826700e1e64a10f298ba962d` | `1455b16cc28ac40eebaa2bb9994728f03a783c834851922a5e74a52abfe99587` |
| XN13B | `bc820cffd876656696c4334e3ad7a7c117be6f6248c406b09c8a58ee233ed0d9` | contract `f1720456e0b7a6cdcc7b06c59f1b1b5a5e0c79e0b9c1781d4f6fc29b86d98658`; census `de5b9fed2c080aede53b6b221173f9d4dcc900f35a9f9bfc95d87cc6ed523530` | verifier `63128186d9059c7ff35ae864aec99f478491a18d038ceb7b6b3a6e95e1ec9fc6`; verification `933a0cd2ab71c9ad48b30615b60f425e5c97d5716536f3dcc8cffe3cb93dbd6f` | integrated directly |
| XN13C | `262507e9fb57cc0dadd402b50c55ed3c58d6150165c33bc1b28462aacdf17d44` | contract `328650759b33cfeb535776c1190093d434ea996fb8b23b65d7d4f6377fe36c00`; census `8713b51eb331dd9558be60eb7139a4085cf82275f37b8e607fd6948c191a546a` | verifier `d5925992f4c83d3dbb3b06e53f8c6a4bf45ff034e9bfe27011212563cbee906f`; verification `6f07f76525f7f64f58e1c125934374e3b22bc0dce9db42e6e175d6191adfb831` | `ae4f0683b46272a6249f2178e47b9d7fd4d3922244cb2b0d72dbca4f3a6a3cc8` |

XN13 main passes 36/36 and its focused consumer tests pass 5/5. XN13C
passes 88/88. Their verification and manifest hashes are stable across two
complete consecutive runs. XN13B was also rerun twice; its verification SHA
remained byte-identical. Every upstream declares zero residuals.

## Integrated promotion law

The normative contract is
`replay_interface/re_evidence/rendering_laws/XN13I/integration_contract.json`.
Each frame envelope has one sequence/update/clock/checkpoint epoch and these
typed populations:

1. XN13 resolved unit rows and exact three-pass pose states;
2. XN13C ground-runtime association rows;
3. XN13B direct dynamic-effect associations;
4. references to the already-frozen XN12B particle packet stream.

Admission is ordered and fail-closed:

- verify binary, effective-VFS, catalog, upstream artifact, verifier, and
  idempotence identities;
- require XN13 `(clock, stable_unit_id)` keys to equal M01 admitted keys;
- require XN13C keys to equal the ground-classified XN13/M01 subset and match
  shared stable unit/entity/state identity;
- require each XN13B effect id to join the direct XN15A2 producer/entity/event
  association and its ordered XN12B packet references;
- validate the atomic checkpoint and cold-seek equality before emitting any
  XN13I contribution.

No process pointer is an identity. Stable `presentation_instance_id`,
`producer_instance_id`, `entity_instance_id`, and `effect_instance_id` values
bridge the lanes. Missing, extra, duplicated, reordered, or ambiguous joins
reject the full frame instead of producing a reduced or guessed population.

## Closed populations

XN13 main freezes 1,239 retained draws: 413 objects across `color_a`,
`light_depth`, and `main_hdr`. Sixty-four skinned objects produce 192 palettes
and 152 byte-unique palettes; 25 objects have two distinct pass palettes and 39
have three. All 1,047 unskinned draws are explicit `NotSkinned`, so stale CB2
bindings are never treated as pose data.

XN13B closes 171/171 dynamic trail/wake rows: 47 aircraft trails, 72 front
wakes, 42 back wakes, and the adjacent 9 flare plus 1 rocket trail rows. All
registrations and effect definitions resolve, all 171 association keys are
unique, and zero rows select a light. Exact node/attachment, visibility/phase,
stop/destroy, and XN12B packet order remain part of the stable effect join.

XN13C closes 4,238 ground entities, 18,796 states, 12,765 authored events,
8,815 attachments, and 10,392 visual rows. Its 10,391 particle rows and all
609 light rows resolve. It retains 4,694 movement-state rows and 641 death-state
rows. The ground grammar contains zero authored selection/highlight/outline
fields, so `selection_effect` is a mandatory observed XN13 runtime output.

## Ownership and deterministic checkpoint

XN13 owns entity/model/state/animation/transition, exact phase, recorded LOD
decision and comparison inputs, material/damage/selection variants, and each
pass's explicit `NotSkinned` or exact 3,200-byte 50-matrix palette. XN13C owns
ground catalog admission and stable ground presentation/entity/producer
association. M01 owns A14 visibility and ordered draw/state membership. XN13B
owns air/naval trail/wake semantics and stable effect association. XN12/XN15
remain the effect/light simulation, lifecycle, packet, and submission authority.

The XN13I checkpoint nests every upstream checkpoint, source sequence range,
next stable id and RNG state under one epoch. Cold seek plus forward replay must
equal linear playback for entity/state/pose/LOD/variant, ground association,
effect attachment/lifecycle, packet membership/order, and next ids. Any
inequality rejects the checkpoint.

## Federated Ghidra evidence

XN13I made no new binary claim. The following decisive upstream tuples are
federated without changing their ownership; exhaustive per-lane tables remain
in XN13, XN13B, and XN13C.

| Address / finding | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14226ba00` animation runtime load | `20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034` | `C:\Users\Example\ghidra_agent_exports\20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034\evidence\func_14226ba00.decompile.c` | `7810719a8998b58d7f420784edf666d12e2cf4b45469784acf005f490bb199ca` |
| `0x14224d6a0` skeleton runtime load | same job | `C:\Users\Example\ghidra_agent_exports\20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034\evidence\func_14224d6a0.decompile.c` | `7810719a8998b58d7f420784edf666d12e2cf4b45469784acf005f490bb199ca` |
| `0x142250010` pose setup | same job | `C:\Users\Example\ghidra_agent_exports\20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034\evidence\func_142250010.decompile.c` | `7810719a8998b58d7f420784edf666d12e2cf4b45469784acf005f490bb199ca` |
| `0x14224f250` mesh `loddist` parser | `20260717T002015Z-xn13-pdxmesh-loddist-parser-v1-9e5c31f9` | `C:\Users\Example\ghidra_agent_exports\20260717T002015Z-xn13-pdxmesh-loddist-parser-v1-9e5c31f9\evidence\func_14224f250.decompile.c` | `0900ab7ea74710940c7cf9c268ad43380898c84ef8bb969ae558f7ffb7f0ad19` |
| `0x14227b9a0` state lookup | `20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259` | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14227b9a0.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14227b5e0` duplicate-state RNG | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14227b5e0.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x1422755a0` state enter | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_1422755a0.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14226eb10` attached-event cleanup | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14226eb10.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14227b200` child visibility | `20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847` | `C:\Users\Example\ghidra_agent_exports\20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847\evidence\func_14227b200.decompile.c` | `016d5dd925d197b6dd232b359cf059b8c6a8363c86898d42e0bbba76cb11602b` |
| `0x14227ac70` node/attachment transform | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847\evidence\func_14227ac70.decompile.c` | `016d5dd925d197b6dd232b359cf059b8c6a8363c86898d42e0bbba76cb11602b` |
| `0x14227b4a0` attached-particle update | `20260717T023648Z-xn13b-entity-animation-event-update-c7a41f7c` | `C:\Users\Example\ghidra_agent_exports\20260717T023648Z-xn13b-entity-animation-event-update-c7a41f7c\evidence\func_14227b4a0.decompile.c` | `13ea856113cc00003a8645c35ecd1f767ce578dc283fa6367cd9a6b6d65cd39a` |
| `0x14239c860` animation time store | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023648Z-xn13b-entity-animation-event-update-c7a41f7c\evidence\func_14239c860.decompile.c` | `13ea856113cc00003a8645c35ecd1f767ce578dc283fa6367cd9a6b6d65cd39a` |
| `0x1422c2ba0` object visibility transition | `20260717T024504Z-xn13b-lod-and-child-visibility-58b7504e` | `C:\Users\Example\ghidra_agent_exports\20260717T024504Z-xn13b-lod-and-child-visibility-58b7504e\evidence\func_1422c2ba0.decompile.c` | `d9dd757471db511288c1bf89c4c56a3ece706cef6c25d11da4548772ac8ca9c9` |
| `0x1422824a0` particle-token resolver | `20260717T023823Z-xn13b-particle-event-attachment-candidat-de3a5d8f` | `C:\Users\Example\ghidra_agent_exports\20260717T023823Z-xn13b-particle-event-attachment-candidat-de3a5d8f\evidence\func_1422824a0.decompile.c` | `e1f8add7e35a70dcafc02a1e70ac5301f79367639948d7806436cd7e902ba103` |
| `0x1409bc400` particle attachment | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_1409bc400.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x1423e8190` particle create | same job | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_1423e8190.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x1423e8400` particle destroy | same job | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_1423e8400.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x140082cb0` entity-event light grammar | `20260717T013324Z-xn15b-exact-light-key-xrefs-b0965d90` | `C:\Users\Example\ghidra_agent_exports\20260717T013324Z-xn15b-exact-light-key-xrefs-b0965d90\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x142264cc0` ordered mesh shadow submission | `20260716T084950Z-l03-shadow-model-list-producer-v1-bbb15676` | `C:\Users\Example\ghidra_agent_exports\20260716T084950Z-l03-shadow-model-list-producer-v1-bbb15676\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `5c3e7986b4350f612b2eb908ef1a6fa533cda1523ac9b2a42f91621a21da3e4f` |

## Verification and disposition

Run `python replay_interface/re_evidence/rendering_laws/XN13I/verify_xn13i.py`.
The verifier pins every upstream and integration artifact, validates all three
PASS dispositions and two-run idempotence proofs, checks shared catalog hashes,
rechecks exact population closure, and validates the complete admission schema.

XN13I has no data, capture, reverse-engineering, or human-input residual. It is
ready for production replay promotion under the atomic fail-closed contract.
PROGRAM_STATUS and collector source were not modified.
