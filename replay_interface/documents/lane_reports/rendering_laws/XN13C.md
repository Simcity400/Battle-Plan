# XN13C — ground-unit dynamic presentation integration

Status: **PROVEN — PROMOTION READY / FAIL CLOSED**

Pinned binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XN13C closes the ground-only presentation packet between M01 admission/draws, XN13 resolved animation/pose/LOD, and XN15B/B2 authored and runtime particle/light producers. It does not duplicate XN13's palette evaluator boundary, XN13B air/naval trails and wakes, or XN12 particle submission. The normative machine contract is `replay_interface/re_evidence/rendering_laws/XN13C/ground_dynamic_presentation_contract.json`.

## Exact closure

The independently rebuilt ground projection is exhaustive under the frozen XN15B classification law:

| Item | Exact result |
|---|---:|
| ground entities | 4,238 |
| source files containing them | 138 |
| authored states | 18,796 |
| all authored state events | 12,765 |
| authored attachments | 8,815 |
| entities with attachments | 2,730 |
| ground visual events | 10,392 |
| ground particle events | 10,391 |
| ground light events | 609 |
| resolved ground lights | 609 |
| explicit-null ground lights | 0 |
| movement-state rows / visual events | 4,694 / 4,642 |
| death-state rows / visual events | 641 / 3 |
| authored selection/highlight/outline fields | 0 |

The canonical SHA-256 of the complete projected entity/state/attachment population is `2f97707c1e1ec901fefc20ef8988de50119121509520e0934e6bb51291416a72`; the canonical ground visual-row SHA-256 is `105712c1add528c70629616b16bf7a0f0ea6fc71743fda8931857c000da4c264`. These digests bind the full rows without copying 28 MB of source evidence into this packet.

## Association, transforms, and selection

At each clock, the XN13C key set must equal the admitted M01 ground-unit key set on `(clock, stable_unit_id)`. One row records stable `presentation_instance_id`, `producer_instance_id`, and `entity_instance_id`, plus the exact effective-VFS entity/model selection. Rebind and teardown are ordered operations; process pointers are never serialized identities.

M01 supplies the exact 4×4 root world matrix. XN13/XN15B2 supply the observed node index, animated-node matrix, local child matrix, and final child world matrix. The 8,815 authored attachment rows are candidates only: replay never guesses a node or transform from the asset declaration. Successful particle attachment retains the frozen `0x1409bc400` runtime seam and stable effect identity.

XN13C consumes the XN13 resolved semantic state, state-entry clock/sequence, clip or transition, phase/speed bits, selected LOD, exact distance/loddist comparison inputs, material variant, damage variant, and selection effect. It consumes XN13's explicit `NotSkinned` tag or exact 3,200-byte, 50-matrix palette separately for each pass. Palettes are never coalesced by object or frame.

The movement domain is exactly `move`, `move_2`, `move_mud`, `move_snow`, `retreat`, and `retreat_2`. The shipped ground damage/death-name census contains only `death`; damage material variants remain separate observed runtime outputs. Because the authored ground grammar has zero selection-like fields, `selection_effect` is necessarily recorded from the resolved runtime presentation and cannot be inferred from installed content or a selected-unit flag alone.

## Effects, lights, visibility, and draws

Every ground visual event retains entity/state/event identity, the ordered authored payload digest, actual trigger sequence/update/tick, resolved particle or light identity, stable effect/light instance ID, and exact parent anchor. The 10,391 particle rows and 609 light rows all resolve. XN15B2 owns create/attach/stop/destroy and submitted light payloads; XN12 owns simulated particle packets.

Visibility is exactly the M01 A14 admission tuple: POV tag, visibility/intel state, spotted decision, inclusion flag, and explicit exclusion reason. LOD and animation phase are exact XN13 recorded decisions. Replay must not apply a new distance, FoW, damage, movement, or phase heuristic.

The retained M01 proof contains 1,239 draws: 413 objects in each of `color_a`, `light_depth`, and `main_hdr`, identical object order, and exactly three fixed state dictionaries. Ground runtime rows preserve that admitted order and exact pass events/state/palette references. Sorting by stable ID, entity, model, state, LOD, effect, or spawn time is forbidden.

## Teardown and checkpoints

Lifecycle records retain presentation/entity spawn, bind/rebind, state entry, transform/visibility/LOD updates, authored event triggers, effect/light spawn and stop/destroy, entity destruction, and presentation destruction in engine order. Owner destruction first removes draw membership and then preserves the observed child stop/destroy sequence; children never survive implicitly.

A deterministic checkpoint contains all next IDs and sequence/update/submit indices; the effective-VFS/catalog identities; every active ground association; semantic state, phase and transition; root/attachment matrices; A14/LOD/damage/selection decisions; per-pass pose references and exact palette digests/bytes; active effect/light joins; and ordered M01/XN12/XN15 references. Cold seek plus forward replay must be byte-identical to linear playback for every recorded field and order.

## Ghidra evidence ledger

XN13C uses immutable, previously frozen exact-a729 artifacts. No function was re-exported and no worker/project was selected. Every relied-on executable row records its address, job, artifact path, and manifest hash.

| Address / finding | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14226ba00` animation runtime load | `20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034` | `C:\Users\Example\ghidra_agent_exports\20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034\evidence\func_14226ba00.decompile.c` | `7810719a8998b58d7f420784edf666d12e2cf4b45469784acf005f490bb199ca` |
| `0x14224d6a0` skeleton runtime load | same job | `C:\Users\Example\ghidra_agent_exports\20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034\evidence\func_14224d6a0.decompile.c` | `7810719a8998b58d7f420784edf666d12e2cf4b45469784acf005f490bb199ca` |
| `0x142250010` runtime pose setup | same job | `C:\Users\Example\ghidra_agent_exports\20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034\evidence\func_142250010.decompile.c` | `7810719a8998b58d7f420784edf666d12e2cf4b45469784acf005f490bb199ca` |
| `0x14224f250` `pdxmesh` `loddist` parser | `20260717T002015Z-xn13-pdxmesh-loddist-parser-v1-9e5c31f9` | `C:\Users\Example\ghidra_agent_exports\20260717T002015Z-xn13-pdxmesh-loddist-parser-v1-9e5c31f9\evidence\func_14224f250.decompile.c` | `0900ab7ea74710940c7cf9c268ad43380898c84ef8bb969ae558f7ffb7f0ad19` |
| `0x1409bc400` particle attachment transform | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `evidence/func_1409bc400.decompile.c` (artifact SHA `a137233b3854434deda5624ae0d32115c57fafd49317f0d2abb95ace9ee7475a`) | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x1423e8190` particle create | same job | `evidence/func_1423e8190.decompile.c` (artifact SHA `35c70015b603ae24289cfa4fa4ce4581ca1b7eb5def5f1c138cc7f709f4d8c7f`) | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x1423e8400` particle destroy | same job | `evidence/func_1423e8400.decompile.c` (artifact SHA `69a05e32081a6f33e762227a5f87b1032f983ce037a90bd3d35c8c90359e588b`) | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x140082cb0` entity-event light grammar | `20260717T013324Z-xn15b-exact-light-key-xrefs-b0965d90` | `string-xrefs.txt` (artifact SHA `dd245d25f15e29346b43cf1ce7efb64295578d1e2c2affa22565497c47d3e057`) | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |
| `0x142264cc0` ordered generic PdxMesh shadow submission | `20260716T084950Z-l03-shadow-model-list-producer-v1-bbb15676` | `C:\Users\Example\ghidra_agent_exports\20260716T084950Z-l03-shadow-model-list-producer-v1-bbb15676\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `5c3e7986b4350f612b2eb908ef1a6fa533cda1523ac9b2a42f91621a21da3e4f` |

No new export was semantically required: every executable claim is covered by the immutable tuples above, and every relied-on tuple is independently hash-pinned through the frozen upstream contracts. XN13C introduced no guessed address or decompile-only semantic claim.

## Promotion result

The verifier rebuilds the ground census, pins every upstream hash, checks the full association/transform/state/effect/light/visibility/LOD/draw/lifecycle/checkpoint schema, validates the retained M01 population/state counts, and requires all Ghidra tuples above. XN13C has no data, capture, RE, or human-input residual. Missing synchronized runtime rows fail closed to zero ground presentation output; they are not approximated.

Run:

```text
python replay_interface/re_evidence/rendering_laws/XN13C/verify_xn13c.py
```
