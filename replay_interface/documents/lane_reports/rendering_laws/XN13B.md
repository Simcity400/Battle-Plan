# XN13B — aircraft-trail and naval-wake dynamic presentation

Status: **PROVEN — PROMOTION READY**

Pinned target: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
The broker reported `ready=true` with ten execution shards. Existing jobs were
listed before address exports.

XN13B consumes the frozen XN15A2 aircraft/naval/convoy producers and closes the
dynamic presentation association between a direct producer, its live entity,
the exact authored trail/wake event, the attached particle instance, and the
XN12B post-simulation render packet. It does not duplicate M01/XN13A mesh
animation or palette work, nor XN12A/XN12B generic particle simulation and
submission.

## Promotion ledger

| ID | Proven law |
|---|---|
| XN13B-01 | The effective-VFS catalog contains exactly 171 trail/wake rows: 47 aircraft trails, 72 front wakes, 42 back wakes, plus 9 flare and 1 rocket trail retained as adjacent useful evidence. |
| XN13B-02 | All 171 rows resolve one particle registration and at least one exact effect definition; zero rows select an authored light. |
| XN13B-03 | Association identity is the direct XN15A2 producer/entity/effect id tuple plus effective source hash, authored state/event indices, registration, and node—not a token-only or texture-only match. |
| XN13B-04 | State lookup is case-sensitive FNV-1a32. Consecutive duplicate state variants are selected with the entity RNG counter/seed and must record the chosen variant. |
| XN13B-05 | State entry first executes prior-state cleanup, stores the new state, initializes animation phase/time, and consumes RNG where the authored state requires it. |
| XN13B-06 | Node attachment evaluates the live animated/skinned node matrix and multiplies the authored attachment matrix; exact replay records the resolved node index and all resulting matrices. |
| XN13B-07 | Owner visibility propagates to every attached child object. Aircraft producer phase 1 is hidden, phase 2 visible and advancing, and phase 3 hidden/expiring. |
| XN13B-08 | Trail/wake event rows author no LOD key. Exact draw presence is observed XN12B packet membership; no replay-side distance threshold or visual approximation is permitted. |
| XN13B-09 | Active attached particles receive the entity animation time each update. `keep_particle=yes` avoids immediate free on ordinary state exit and transitions the retained particle lifecycle; `no` destroys the object and attachment resources immediately. |
| XN13B-10 | Draw state/order is exactly XN12B bucket, packet ordinal, ribbon/trail flag, state dictionary entry, and submit index. XN13B adds the effect-id join without re-sorting. |
| XN13B-11 | Damage, death, hide, stop-emission, expiry, particle destruction, and owner destruction are ordered producer operations. Aggregate combat/loss counters cannot synthesize any of them. |
| XN13B-12 | Checkpoints freeze active association membership, stable ids, state/event/phase, node transforms, visibility, RNG, lifecycle sequences, and the referenced XN12B packet checkpoint. Cold seek must equal linear playback. |

## Exhaustive game-data closure

`dynamic_effect_census.json` is a lossless projection of the 171 matching rows
from XN15's 18,134-row visual-trigger census, cross-checked as a duplicate-aware
multiset against XN13 and joined to the XN15 registration/definition source
hashes. Closure is:

- `air_trail_particle`: 47;
- `ship_moving_front_fleet_particle`: 72;
- `ship_moving_back_fleet_particle`: 42;
- `flare_trail_particle`: 9;
- `rocket_trail_particle`: 1;
- unresolved registrations/effect definitions: 0/0;
- authored light selections: 0;
- unique association catalog keys: 171/171.

The rows span 112 `move`, 49 `idle`, 8 `death`, and 2 `attack` states. The
death rows are ship flare trails and retain their exact `keep_particle=no`
semantics; they are not wakes. Of all rows, 161 author `keep_particle=yes` and
10 author `no`; all 114 wakes author `trigger_once=yes`.

One authored anomaly is preserved rather than normalized away:
`USA_carrier_0_entity`, state `idle`, event index `0`, has ordered duplicate
node keys `idle_foam` then `flying_plane_loc` for `air_trail_particle`. The
catalog retains both values and exact event payload. Production records the
runtime-resolved node index, so replay never guesses duplicate-key resolution.

## Producer, entity, and attachment association

XN15A2 already freezes stable `producer_instance_id`, `entity_instance_id`, and
`effect_instance_id` values at direct aircraft, naval-combat, and trade-route
producer operations. XN13B extends that record with the exact effective-VFS
entity source hash, authored state/event indices, 171-row association key,
registration/effect definition, ordered node values, resolved node index, and
attachment matrices.

`0x14227b9a0` performs the exact state-token lookup. `0x14227b5e0` resolves
duplicate state variants with the entity RNG and applies the chosen state to
children. `0x1422755a0` exits the previous state, stores the new state, chooses
the animation start/phase and initializes state time. `0x1422824a0` resolves an
authored particle token to `PdxParticleType`, returning null with a diagnostic
when missing. Generic particle allocation and simulation remain XN12A/XN12B
authority; XN13B's stable effect id bridges that lifecycle to the producer.

For attachments, `0x14227ac70` handles the authored attachment kind. Its node
path reads the animated/skinned node transform when available, falls back to
the entity world transform when required, and multiplies the authored 4x4
attachment matrix. `0x14227b4a0` stores current entity animation time, updates
each active attached particle object with that time, and recurses into child
entities. Therefore exact association requires observed node resolution and
matrices, not reconstruction from entity position alone.

## Visibility, LOD, phase, and selection

`0x14227b200` stores owner visibility and walks the attached-object vector.
`0x1422c2ba0` changes the child object's visibility bit and invokes its virtual
visibility transition only when the bit changes. Trails and wakes cannot remain
independently visible after the owner is hidden.

XN15A2's aircraft lifecycle is authoritative for phase gating: phase 1 hides
the actor during pre-roll, phase 2 shows it, selects/advances state and
attachment paths, and phase 3 hides it through expiry. Naval and convoy
visibility follows their direct producer lifecycle. State timing is recorded
as state-enter sequence plus exact animation-phase bits; authored `time` text
is catalog metadata, not a substitute for the observed trigger operation.

The LOD search found `0x14224f250` parsing mesh `loddist`, which belongs to the
mesh/object pipeline rather than the 171 particle event rows. `0x1402a1d40`
builds `LOD %d` diagnostic/display entries and is likewise not a particle event
gate. Since the trail/wake rows have no LOD field, exact replay uses the
post-simulation `submitted_bool` and XN12B packet reference observed at the
render boundary. Absence is preserved as absence; no hidden threshold is
invented.

Every row resolves particle registration and effect definition from the pinned
effective VFS. All 171 have `light_token=null` and empty light definitions, so
the production replay must not synthesize dynamic lights for this packet.

## Draw order and retained captures

XN12B remains the renderer authority. Per frame, XN13B records the semantic
join from `effect_instance_id` to submitted flag, packet bucket, packet ordinal,
render submit index, and render-state dictionary id. The renderer consumes the
captured packet bytes and keeps XN12B order, including the ribbon/trail tail.
Sorting by producer/entity/effect identity, texture, state, or spawn time is
forbidden.

The retained capture census is hash-pinned into the XN13B evidence:

| Capture | Draws | Instances | First–last event |
|---|---:|---:|---:|
| `hoi4_frame35240` | 28 | 416 | `25706–26057` |
| `hoi4_frame69242` | 41 | 614 | `28456–28949` |
| `postfx_frame8467` | 427 | 6,287 | `40386–45550` |

Those captures prove packet/state/order behavior, not semantic identity by
texture. The producer/effect-id join supplies semantics and avoids an ambiguous
capture-side inference.

## Damage, death, removal, and checkpoint law

`0x14226eb10` proves state-exit behavior over the entity's active attached
particle vector. On ordinary exit, a `keep_particle` child is not immediately
freed and its retained particle lifecycle is invoked. Other entries destroy the
particle object, release attachment resources, remove the vector entry, and
clean the remaining state event/sound objects. Owner teardown may force all
entries through destruction.

XN15A2's direct operations remain the source of truth:

- aircraft phase hide, expiry, destructor, and scenario teardown gate/remove
  aircraft trails;
- naval damaged/death state entry selects authored events, while effect expiry
  and ship teardown own removal;
- `westerngfx_traderoute_convoy` has exactly two time-zero retained wakes in
  default `idle`, zero particle events in authored `death`, and its producer
  hides, reuses, or destroys the visual object rather than synthesizing a sunk
  visual.

The frozen contract is `presentation_contract.json`. A checkpoint contains
every active association and next id/sequence; exact state/event identity,
phase, visibility, node matrices, keep/trigger/life fields, producer RNG,
stop/destroy sequence, and referenced XN12B particle checkpoint. Any missing or
ambiguous field fails closed.

## Frozen artifacts

| Artifact | SHA-256 |
|---|---|
| `XN13B/dynamic_effect_census.json` | `de5b9fed2c080aede53b6b221173f9d4dcc900f35a9f9bfc95d87cc6ed523530` |
| `XN13B/presentation_contract.json` | `f1720456e0b7a6cdcc7b06c59f1b1b5a5e0c79e0b9c1781d4f6fc29b86d98658` |
| `XN13B/build_dynamic_effect_census.py` | `752bbaf7790c40492bed01c83282b3d539736acc3e124cb062beed78a17cf0d9` |
| input `XN13/gamefile_census.json` | `d450832d98630a3c470a5898949836c366c86ea541f58fc117ecaf2ae49413cd` |
| input `XN15/gamefile_trigger_census.json` | `23f8373c6bc620146667b5f61a026a49155de3a7940375448a49c97c13728053` |
| input `XN15A2/producer_census.json` | `c16632d98f3911f49a7fcccc7350fb9634e6dd64be404a214eb89a0903e5b93c` |
| input `XN12B/mechanical_result.json` | `9ea07a4618a2ccb652a5cd5baf1ad2ba73f8804f7cb0f540fc7455be47ca8ee1` |

## Ghidra function evidence

Every function row records address, job id, artifact path, artifact hash, and
function-evidence manifest hash.

| Function / finding | Job ID | Artifact path | Artifact SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|
| `0x14227b9a0` exact state-token lookup | `20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259` | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14227b9a0.decompile.c` | `eba772a0bc6b2c7d03903ed4282f0a38782061bacaac1456ed9811f400a4c4a7` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14227b5e0` duplicate-state RNG selection | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14227b5e0.decompile.c` | `ac29c400c4855f786f51c0376cb8e2c9a722b7d2b267945f90e2869ad43e2c89` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x1422755a0` state exit/enter and phase initialization | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_1422755a0.decompile.c` | `d1f8f817c2698056512cc41dcfb8fced27224ac9e7e4e0955d9303c309d23c5a` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14226eb10` attached-event cleanup/keep behavior | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023441Z-xn13b-entity-state-event-runtime-2ced8259\evidence\func_14226eb10.decompile.c` | `c86bd5bf57cc6c416431de89d5af6bb099814357ef24177a4af4a8c47f453b9a` | `eb46e2e95d1b7067f58139783c8fcd61c48536bfe74e1218ec0fe22134ddb725` |
| `0x14227b200` owner-to-child visibility propagation | `20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847` | `C:\Users\Example\ghidra_agent_exports\20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847\evidence\func_14227b200.decompile.c` | `b41eafb431e7c947d0fd6585d325e2415c26b5659e6a09b9075bca921f5f42e2` | `016d5dd925d197b6dd232b359cf059b8c6a8363c86898d42e0bbba76cb11602b` |
| `0x14227ac70` animated node/attachment transform | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023459Z-xn13b-entity-visibility-node-transform-42ca1847\evidence\func_14227ac70.decompile.c` | `4e5a00637535a7675c9a328bc1cbeb9aaf53a1ca017d172a053f04c86ca7f761` | `016d5dd925d197b6dd232b359cf059b8c6a8363c86898d42e0bbba76cb11602b` |
| `0x14227b4a0` active attached-particle time update | `20260717T023648Z-xn13b-entity-animation-event-update-c7a41f7c` | `C:\Users\Example\ghidra_agent_exports\20260717T023648Z-xn13b-entity-animation-event-update-c7a41f7c\evidence\func_14227b4a0.decompile.c` | `a1daf799cfc2af5439e1498bd4e4d06930dcd79d142fd13f1fe52e76ed51e57b` | `13ea856113cc00003a8645c35ecd1f767ce578dc283fa6367cd9a6b6d65cd39a` |
| `0x14239c860` animation time store | same job | `C:\Users\Example\ghidra_agent_exports\20260717T023648Z-xn13b-entity-animation-event-update-c7a41f7c\evidence\func_14239c860.decompile.c` | `a9a23dd3156315cdae14970ca30b97260a5a39cf63f95f6c1b57f2d8de62a0e9` | `13ea856113cc00003a8645c35ecd1f767ce578dc283fa6367cd9a6b6d65cd39a` |
| `0x1422c2ba0` child visibility transition | `20260717T024504Z-xn13b-lod-and-child-visibility-58b7504e` | `C:\Users\Example\ghidra_agent_exports\20260717T024504Z-xn13b-lod-and-child-visibility-58b7504e\evidence\func_1422c2ba0.decompile.c` | `487f62b97df0e53b3a6d72ad75bd1ecd72f1048b1cb0a0b5c3d1f3bba398a637` | `d9dd757471db511288c1bf89c4c56a3ece706cef6c25d11da4548772ac8ca9c9` |
| `0x14224f250` investigated mesh `loddist` parser boundary | same job | `C:\Users\Example\ghidra_agent_exports\20260717T024504Z-xn13b-lod-and-child-visibility-58b7504e\evidence\func_14224f250.decompile.c` | `2a4c183429020f28269bdc655d042636bc9c8ad933422761513d27f7ee310f8e` | `d9dd757471db511288c1bf89c4c56a3ece706cef6c25d11da4548772ac8ca9c9` |
| `0x1402a1d40` investigated `LOD %d` diagnostic/display path | same job | `C:\Users\Example\ghidra_agent_exports\20260717T024504Z-xn13b-lod-and-child-visibility-58b7504e\evidence\func_1402a1d40.decompile.c` | `978ae13b9e6579d996bd3df9ff929356afe243816e2001e7b8437abe3b9642b5` | `d9dd757471db511288c1bf89c4c56a3ece706cef6c25d11da4548772ac8ca9c9` |
| `0x1422824a0` particle-token resolver | `20260717T023823Z-xn13b-particle-event-attachment-candidat-de3a5d8f` | `C:\Users\Example\ghidra_agent_exports\20260717T023823Z-xn13b-particle-event-attachment-candidat-de3a5d8f\evidence\func_1422824a0.decompile.c` | `96e25d1aeab544e56fc981870191f41bc2f3c14d58221147453786add0a20495` | `e1f8add7e35a70dcafc02a1e70ac5301f79367639948d7806436cd7e902ba103` |

## Mechanical verification

Run:

`python replay_interface/re_evidence/rendering_laws/XN13B/build_dynamic_effect_census.py`

then:

`python replay_interface/re_evidence/rendering_laws/XN13B/verify_xn13b.py`

There are no remaining data-blocked, capture-blocked, or Ghidra-blocked XN13B
residuals. PROGRAM_STATUS and production collector source were not modified.
