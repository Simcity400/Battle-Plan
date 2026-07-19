# XN13V — deterministic XN13I production fixtures

Disposition: **PROVEN — PROMOTION READY**

XN13V freezes a deterministic positive and rejection corpus for the already
frozen XN13I composition (XN13 main + XN13B + XN13C). It adds no rendering law
and does not replace XN13P's production schema/handoff work. Its purpose is to
make promotion mechanically testable at the consumer boundary.

Authority is `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
The immediate frozen boundary is XN13I manifest
`c16f512b09a0ae10fe270ba0c92ee1938f69882c568e1c9b338b7eefacf49f49`.

## Frozen fixture coverage

The corpus selects lexicographically first qualifying records from pinned
upstreams. All entity/state/event tokens, ordered payloads, effect/light
definitions, capture draw rows, and 3,200-byte palettes are copied from those
sources. Only clocks, stable recording ids, instance ids, and sequence numbers
are structural fixture values; none are claimed as engine constants.

| Domain | Frozen positive fixture |
|---|---|
| Identity and transition | `ENG_armored_signal_entity`; exact authored `idle` → `move` state indices, animations, and first resolved move event |
| Skinned three-pass pose | object `08bb6f50ea047d9e766764d520e7f2f06711669a00c6e4c197f2f54c417395c7`; exact independent `color_a`, `light_depth`, and `main_hdr` 3,200-byte palettes |
| Explicitly unskinned pose | object `0177c15143afd0eb827b7e7989dc0309078df117dd84f680beb7e34bdb68bc6f`; all three passes carry authoritative `not_skinned` tags |
| Ground particle + light | first resolved ground light row: `AFG_light_armor_0_entity`, `attack`, event 0, `mg_muzzle_flash` |
| Aircraft trail | first frozen `aircraft_trail`: `AST_supersonic_jet_plane_entity`, `air_trail_particle` |
| Naval wakes | first frozen front/back rows: `CHI_light_cruiser_entity`, `ship_moving_front_fleet_particle` and `ship_moving_back_fleet_particle` |
| Teardown | ordered draw removal, stop-emission, child destroy, entity destroy, and presentation/owner destroy fixtures |
| Checkpoint | domain hashes, aggregate hash, exact linear-state hash, and byte-identical cold-seek state |

The corpus covers ten positive domains and 25 negative mutations. Each mutation
must return its exact fail-closed code: unknown schema/binary/catalog, process
pointer identity, non-idempotent upstream, missing/duplicate/extra main or
ground rows, shared-identity mismatch, unknown entity/state/event, bad palette
length/digest/tag mixture, unresolved light, forbidden trail light, missing or
ambiguous packet join, reordered event payload, teardown reorder, surviving
child, checkpoint mismatch, and cold-seek inequality.

## Mechanical verification

`verify_xn13v.py`:

1. rebuilds the corpus in memory and requires canonical byte equivalence;
2. re-hashes all seven frozen inputs;
3. validates the positive specimen against the selected frozen source rows;
4. recomputes every palette digest from exact little-endian bytes;
5. verifies event payload order, identity joins, light/effect resolution,
   packet cardinality, teardown order, and checkpoint hashes; and
6. applies all 25 mutations independently and requires the exact rejection
   code for each.

Current result is **PASS**: 10 positive domains, 25/25 negative fixtures, zero
residuals. Consecutive rebuild/verification runs are required to remain
byte-identical before the packet tuple is frozen.

## Incidental useful finding

The explicitly unskinned `cigarette.mesh` object still has three different
observed VS-CB2 palette digests across the retained passes. Those bytes are
evidence of buffer state, not proof that the shader consumes a skinning
palette. The fixture therefore retains the observed digests for diagnostics
while requiring `not_skinned` as the production selection. A consumer that
infers skinning from CB2 population alone will fail this fixture.

## Federated Ghidra provenance

XN13V made no new binary claim. These decisive XN13I-owned tuples are federated
unchanged so the report remains independently traceable; exhaustive provenance
remains in XN13/XN13B/XN13C and XN13I.

| Function address / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14226ba00` animation runtime load | `20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034` | `C:\Users\Example\ghidra_agent_exports\20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034\evidence\func_14226ba00.decompile.c` | `7810719a8998b58d7f420784edf666d12e2cf4b45469784acf005f490bb199ca` |
| `0x142250010` runtime pose setup | `20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034` | `C:\Users\Example\ghidra_agent_exports\20260717T003253Z-xn13-animation-runtime-skeleton-load-v3-4c719034\evidence\func_142250010.decompile.c` | `7810719a8998b58d7f420784edf666d12e2cf4b45469784acf005f490bb199ca` |
| `0x14227b9a0` entity state lookup | `20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259` | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14227b9a0.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x1422755a0` state enter | `20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259` | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_1422755a0.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14226eb10` attached-event cleanup | `20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259` | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14226eb10.decompile.c` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14227ac70` node/attachment transform | `20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847` | `C:\Users\Example\ghidra_agent_exports\20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847\evidence\func_14227ac70.decompile.c` | `016d5dd925d197b6dd232b359cf059b8c6a8363c86898d42e0bbba76cb11602b` |
| `0x14227b4a0` attached-particle update | `20260717T023648Z-xn13b-entity-animation-event-update-c7a41f7c` | `C:\Users\Example\ghidra_agent_exports\20260717T023648Z-xn13b-entity-animation-event-update-c7a41f7c\evidence\func_14227b4a0.decompile.c` | `13ea856113cc00003a8645c35ecd1f767ce578dc283fa6367cd9a6b6d65cd39a` |
| `0x1409bc400` particle attachment transform | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_1409bc400.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x1423e8190` particle create | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_1423e8190.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x1423e8400` particle destroy | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_1423e8400.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x140082cb0` entity-event light grammar | `20260717T013324Z-xn15b-exact-light-key-xrefs-b0965d90` | `C:\Users\Example\ghidra_agent_exports\20260717T013324Z-xn15b-exact-light-key-xrefs-b0965d90\string-xrefs.txt` | `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe` |

## Frozen artifacts

| Artifact | SHA-256 |
|---|---|
| `replay_interface/re_evidence/rendering_laws/XN13V/build_fixture_corpus.py` | `839b2ad89ea0a406660c8448cbd9216c33456a897a2a94abd5ffee5c2fcee75b` |
| `replay_interface/re_evidence/rendering_laws/XN13V/verify_xn13v.py` | `c18ee3d6edc31e9e40e208c436d96426913816d0128be4dd7febc9bc76bc6f35` |
| `replay_interface/re_evidence/rendering_laws/XN13V/fixture_corpus.json` | `b7611ed9d660c5c166036ecd6c2d6e67294fbd1a04f58aed40d1e935fae3fe46` |
| `replay_interface/re_evidence/rendering_laws/XN13V/verification.json` | `f70287107e92497e4ddb5fcaf1eed210c05f4dd148008bc1f7dd84a805b8a01e` |

Residuals: **none**. No capture or data blocker remains in XN13V.
