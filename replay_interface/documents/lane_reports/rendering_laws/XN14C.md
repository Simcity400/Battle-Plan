# XN14C — weather promotion integration contract

Status: **PROVEN - PROMOTION READY**

Pinned binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XN14C joins the exact XN14A producer/asset law, frozen XN14 weather-interface slice, frozen XN14B retained draw/composition census, frozen XN15B authored trigger census, frozen XN15B2 positive runtime records and frozen XN15 lifecycle schema. Every dependency is hash-pinned and the combined contract is ready to promote.

## Exact promotion boundary

Static extraction pins the nine ordered `weather_effect` variants, 17 reachable entities/particle definitions, 588 ordered weather positions, effective-VFS identities, textures/shaders, and the eight `WEATHER_*` graphics defines.

Runtime extraction records five linked streams:

1. Region state: the six flags, exact airborne runtime code, nullable weather-effect token, `+0x138` as signed fixed-point `i64 / 100000`, and `+0x150` as absolute `i32` game-hour expiry. Code zero requires a null token.
2. Membership epoch: every ordered position, runtime XYZ bits, big/small class, instantiated-or-explicit-empty resolution, the actually selected authored variant when present, both entity instance IDs, and ordered child effect-instance IDs.
3. Lifecycle: the positive runtime producer, parent anchor, and strict spawn/activate/deactivate/stop-emission/destroy sequence for every entity/effect instance.
4. Visibility/phase: view-distance bits, exact camera branch, post-evaluation alpha/active flags, playback-rate bits, and effect/particle phase in the lifecycle and packet streams.
5. Submission: ordered final packet bytes plus full shader/material/CB/SRV/blend/depth/raster/sampler/target/viewport state.

The wall-clock-selected authored variant and wall-clock-seeded particle simulation are recorded, never recomputed. Texture identity is not weather identity: shared cloud atlases retain CPU effect identity and exact packet state, and capture-local RenderDoc ResourceIds are never serialized across captures.

## Corrected state semantics

The current binary proves two important corrections to the earlier XN15 draft:

- `SWeatherPerRegion +0x138` is fixed-point integer state, not `f32` bits; `+0x150` is an absolute game-hour `i32`, not a generic tick counter.
- The airborne producer has no weather intensity scalar or interpolated weather-transition phase. A runtime-code change destroys and recreates the entity pairs. Effect elapsed/time-scale and particle simulation phase remain exact in their own lifecycle/packet records.

Any implementation that emits the rejected field types or synthesizes the absent values fails closed.

## Membership, composition, and order

Membership joins in this direction only:

`region state -> selected authored variant -> primary/fading entity instances -> child effect instances -> submitted packets`

The CPU instance/effect join is authoritative. Exclusive texture/technique pairs provide a capture-side carrier cross-check, but shared atlas payloads never assign semantic identity.

Total order is `sequence_u64` for lifecycle, `update_index_u64` for simulation, then `submit_index_u64 / pass_ordinal_u32 / packet_ordinal_u32 / draw_order_u32` for composition. The replay must retain exact packet bytes, pipeline transitions, targets, viewport, and particle placement after map objects and before arrows/state restore.

Checkpoints contain complete region state, position membership, active entity-to-effect joins, lifecycle state, ordered weather packets, and separate state/membership/lifecycle/packet/composition hashes. Cold seek must be byte-identical to linear playback.

## Dependency gate

| Dependency | Current state | Required frozen tuple |
|---|---|---|
| XN14A | frozen | report `4cf5087e…`, manifest `f37b7bd9…`, runtime contract `7bc3f5ee…` |
| XN14 interface | frozen | contract `b6d5db71…2ad9`, report `ec41591e…e304`, verification `622c4d42…0947`, manifest `096f91f2…8773` |
| XN14B | frozen | report `27a0bdcb…24e7`, census `2667f81b…88d`, verification `993a6e15…fb8`, manifest `0c52445a…e66` |
| XN15B static | frozen | report `a1d22ecd…`, census `23f8373c…`, verification `4dda72ea…` |
| XN15B2 runtime | frozen | report `0343b6d5…5eaf`, contract `64139661…5269`, verification `c63bb79d…ffc0`, manifest `b275e02d…8a85` |
| XN15 core | frozen | report `fdba5781…76f9c`, contract `053f0675…2d95`, ledger `2d483491…2527`, verifier `6a7d2591…cf60`, verification `9ec7ad09…f173` |

Every dependency in [promotion_contract.json](../../../re_evidence/rendering_laws/XN14C/promotion_contract.json) is frozen and mechanically hash-checked. Unknown or mismatched dependencies continue to fail closed.

## Inherited executable provenance

XN14C issued no duplicate Ghidra jobs. Its executable claims inherit these frozen tuples:

| Address / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x140f0bd60` region transition/type/expiry | `20260717T010100Z-xn14a-weather-state-transition-workers-b3770842` | `C:\Users\Example\ghidra_agent_exports\20260717T010100Z-xn14a-weather-state-transition-workers-b3770842\evidence\func_140f0bd60.decompile.c` | `97ee87421ce17726a486eb4529cb7422cb70d2b1942fe927bee335c9f97929e8` |
| `0x14166ec50` membership/visibility/playback producer | `20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833` | `C:\Users\Example\ghidra_agent_exports\20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833\evidence\func_14166ec50.decompile.c` | `fd99de197bf7699319a522e0d2e68dbd63a36d9a778ded8ca368597d9679d9c8` |
| `0x1424dbe60` wall-clock seed source | `20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d` | `C:\Users\Example\ghidra_agent_exports\20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d\evidence\func_1424dbe60.decompile.c` | `faabb041c2eaa89bdc9a93a81535073aa5189d0e09a9322f46ea613b8b9ad1d5` |
| `0x1423dd730` packet emission | `20260717T004108Z-xn12b-particle-instance-render-update-co-75672612` | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423dd730.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |
| `0x14227f420` bind/draw submit | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `C:\Users\Example\ghidra_agent_exports\20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19\evidence\func_14227f420.decompile.c` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |

The positive XN15B2 weather-producer tuple is frozen and hash-pinned in the promotion contract.

## Evidence and verification

- `replay_interface/re_evidence/rendering_laws/XN14C/promotion_contract.json`
- `replay_interface/re_evidence/rendering_laws/XN14C/verify_xn14c.py`
- `replay_interface/re_evidence/rendering_laws/XN14C/preflight.json`

Preflight command:

`python replay_interface/re_evidence/rendering_laws/XN14C/verify_xn14c.py --preflight`

Final result: `XN14C_VERIFY_OK`, 71 checks. The verifier writes `verification.json` and `evidence_manifest.json` only after every dependency hash and the exact XN14-interface/XN15 lifecycle field binding pass.
