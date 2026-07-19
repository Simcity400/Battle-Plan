# XN14P — weather production replay handoff

Status: **PROVEN - PRODUCTION REPLAY HANDOFF FROZEN**

Pinned binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XN14P is the final fail-closed implementation handoff for the weather lane. It independently hash-checks the frozen XN14A producer/assets, XN14B retained capture census, standalone XN14 interface, and the final 71/71 XN14C integration tuple. The normative machine-readable handoff is `replay_interface/re_evidence/rendering_laws/XN14P/production_handoff.json`; the complete field-level authority remains the pinned XN14C promotion contract.

## Production boundary

Promote the lane as five linked streams in decode order: region weather state, ordered membership epoch, effect lifecycle, per-entity visibility/phase, and final submitted weather packets. Join only in this direction:

`region state -> selected authored variant -> primary/fading entity instances -> ordered child effect instances -> submitted packets`

Static extraction supplies the nine ordered authored variants, 17 reachable entities/particle definitions, 588 ordered weather positions, and exact effective-VFS payload identities. Runtime capture records the six region flags, exact code/token, fixed-point scalar and absolute expiry; every instantiated-or-explicit-empty position; stable entity/effect IDs and lifecycle; post-evaluation visibility/playback; and exact ordered packet bytes plus full composition state.

Do not recompute wall-clock-selected authored variants or wall-clock-seeded particle results. Do not infer weather identity from a shared atlas. Do not serialize or compare capture-local RenderDoc ResourceIds. A checkpoint plus forward replay must be byte-identical to linear playback for state, membership, lifecycle, packet-order, and composition hashes. All 14 fail-closed cases and all eight production acceptance tests are inherited exactly from the frozen XN14C contract.

## Frozen inputs

- XN14A: report `4cf5087efdcd6cba9d9fe4cdfe23a88f13bec5d69c1c6045c8bbb91f72f790d8`, manifest `f37b7bd97fd7024c4d7b374c51ccf4c4b7c326d212958ad9e4f91216b7245110`, runtime contract `7bc3f5eef4aaf28641e33fccb87d3e72c616697a354cd944148ec209cb6677da`.
- XN14B: report `27a0bdcb1825d8497d7f2d2c8a4b271f3b942e53e7348cf7639f434cb4f724e7`, census `2667f81b3e57d83657cc7f6adabee111b0cdcfb32cc2fe51cdcc98cfe49f688d`, verification `993a6e151b6f66de7fa4e8fa1ef331e4d8c2b87ceba58591c39c35b55b203fb8`, manifest `0c52445a3b0bca92676fd344a8197304e658e0f29a44dce44d5e8e7a5d203e66`.
- XN14 interface: report `ec41591ecef30ee8027f29dd40d297c34a0e91634d3042e0ee4b06b1a554e304`, contract `b6d5db716be4a177a1fe4174d1ba657460ad7a6dd322baa2b87ca81fb8b52ad9`, verification `622c4d4251a0fd51954d574a52e023bacf5258074dc940399cc7c23652650947`, manifest `096f91f2be911761fade30bd09644716ed445c2e83870e699f53e165d1068773`.
- XN14C final: final report `08ba827a0adcd9da058e3f488de52ac43ee06439aaf21a143fadc5b7cd066eed`, integration report `65dd9a12cc5fcf73d68b8a2ddd3c18452af3ed900880c1502a68685115a22875`, contract `0bca70715444feae56ac97777e19d5d695b082c1938f2730a37f4258a571e616`, verification `866df6efa232b2d6bce5c5fabf381d2734b72bfd9e7416e774d32cd44f79d901`, manifest `e3bf28865acb17e21ef67768744f8e6a729967e057af33b0e9673ae45f32015b`.

There is no standalone XN14B2 packet to consume. XN14B's frozen `session12_last_anchor.json` and `vanilla_late_anchor.json` close the late-load capture coverage; XN15B2 is the distinct frozen positive runtime producer dependency already consumed by XN14C. This distinction is enforced mechanically so an invented packet cannot enter the handoff.

## Inherited executable provenance

XN14P issued no duplicate Ghidra jobs. Every executable claim inherits a complete frozen tuple:

| Function address / role | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x140f0bd60` region transition/type/expiry | `20260717T010100Z-xn14a-weather-state-transition-workers-b3770842` | `C:\Users\Example\ghidra_agent_exports\20260717T010100Z-xn14a-weather-state-transition-workers-b3770842\evidence\func_140f0bd60.decompile.c` | `97ee87421ce17726a486eb4529cb7422cb70d2b1942fe927bee335c9f97929e8` |
| `0x14166ec50` membership/visibility/playback producer | `20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833` | `C:\Users\Example\ghidra_agent_exports\20260717T012439Z-xn14a-weather-region-getter-all-callers-e6227833\evidence\func_14166ec50.decompile.c` | `fd99de197bf7699319a522e0d2e68dbd63a36d9a778ded8ca368597d9679d9c8` |
| `0x1424dbe60` wall-clock seed source | `20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d` | `C:\Users\Example\ghidra_agent_exports\20260717T004246Z-xn12b-particle-prng-seed-init-min-14613a5d\evidence\func_1424dbe60.decompile.c` | `faabb041c2eaa89bdc9a93a81535073aa5189d0e09a9322f46ea613b8b9ad1d5` |
| `0x1423dd730` packet emission | `20260717T004108Z-xn12b-particle-instance-render-update-co-75672612` | `C:\Users\Example\ghidra_agent_exports\20260717T004108Z-xn12b-particle-instance-render-update-co-75672612\evidence\func_1423dd730.decompile.c` | `b8482288abf63d70128ddd097186c3cba5c7e9e8eb48d906df3cd5b00670192d` |
| `0x14227f420` bind/draw submit | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `C:\Users\Example\ghidra_agent_exports\20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19\evidence\func_14227f420.decompile.c` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |

## Mechanical verification

Run:

`python replay_interface/re_evidence/rendering_laws/XN14P/verify_xn14p.py`

The verifier hashes every frozen input, checks the complete XN14C dependency gate and 71/71 result, validates the exact stream/order/type/packet/checkpoint seams, confirms both late-load anchors are frozen inside XN14B, checks every provenance tuple against this report, and emits the XN14P verification and manifest only on success.

There is no data, capture, Ghidra, or human-input blocker for promotion. No collector, rendering implementation, or `PROGRAM_STATUS.md` file was edited. No excluded rendering scope was investigated.
