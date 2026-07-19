# XN17I — final non-UI display-variant parity integration

Status: **PROMOTION READY / FAIL CLOSED — integration law proven; XN13, XN14, XN15, and XN16 contributions remain typed rejects until immutable tuples are admitted**

XN17I mechanically combines the immutable XN17A display/admission law, XN17B exhaustive DLC/VFS law, and corrected XN12 particle-production law. It does not re-derive an upstream equation or treat a directory, report status, or passing narrower check as dependency admission. The normative contract is `replay_interface/re_evidence/rendering_laws/XN17I/contract.json`.

The integration gate itself is complete and promotion-ready. It can be implemented now without guesses: an input either satisfies its frozen law or returns a stable typed rejection at the earliest boundary. A full non-UI frame is not currently admitted because XN13–XN16 have not supplied complete immutable tuples to this integration packet. This is a deliberate production decision, not a partial result.

## Admitted immutable inputs

| Packet | Report SHA-256 | Contract/census SHA-256 | Verification SHA-256 | Manifest SHA-256 |
|---|---|---|---|---|
| XN17A | `6d5543cf7c34917d7bca135b55cf6c87eb2b83962e0a76824622bc0106c72b30` | `e005bfd6ce4a22d443ec533ef4731e1daa9c3056c8a935c4d6fdf52461a197a3` | `a16ba4c79056b554766e28db9bb4d56de841cd64abb13448d7d424c7cd4869a9` | `8f24e29d74dce7d49229c73e397e340b8ec21238ce290388054b0c22f2671823` |
| XN17B | `43c1abe8b57d5ac5655c16f501f6102da907229c160a88989a48be2e1993af00` | `63e8f133ccfa62614cd1c241c7704d0e19ec3763ebc478047da4c67ca125a3ee` | `91c06363b060bdc13862b4cab642772b2b3e73b42121b1400d1c8d4d204bf85c` | `3927bcb42a99632049224f8bfde821f1e347764a4d2faf24d685af4439dbd67e` |
| XN12 | `5ce0d731938aca2de05bbc5d35f8e869aaeea4811ecea5a40e9b77e5809d47bb` | `6206ca801b20778f21afac0678e90e6b8dcfc5a04d9ff1bbbf34f2b6aaceda46` | `797e2eddbeb99729162f9ff30f1f260db23041e98bbb0904833f3fa0c8d1e1d3` | `29bef75c09f9c0eaf9afb18aae14f1eaaa85d340f483ae548be0ff77dd4f07d9` |
| XN16P component | `ebf5f0222e67ef0153ef91205c4d517b776c7f1fcdad03adef46ac479074526e` | recovery `822270cf55eaccbddca57b000781666990322f3375b9eac58cbc432e1afc1079` | `097863555017caa94b7ce7d739fa2a15548c199bd39475d54930eaa51ed354db` | `3c79b7b89492b16201c5d21f128ab10d1e03e5d8d1d8b5a76d746ac3cc269f95` |

XN16P admits only the PAN2 clear-provenance component. It cannot admit its XN16 parent.

## Final production decision law

Validate in this order:

1. Require `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, and executable SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
2. Recompute XN17B's ordered namespace: base; four integrated DLC layers; active product plus descriptorless asset layers; then mods in launcher order. Reject unsafe, ambiguous, invalid, drifted, or hash-mismatched inputs before packet decode.
3. Enforce XN17A's 5632×2048 map geometry, finite camera domain, construction-time viewport/aspect law, engine registry map-mode identity, and all six recorded display toggles.
4. Submit XN12's exact post-simulation particle packets at the `0x14227f420` seam. Preserve 112-byte packet order, 48-byte instances, stable effect/asset identity, and source-local payload joins. Never resimulate from semantic events or transport capture-local handles.
5. Evaluate requested producers in the fixed order XN13, XN14, XN15, then mandatory XN16 final composition. An unadmitted producer returns its typed code; it never emits a fallback.
6. Reject any missing required asset, packet, palette, sequence, pass predecessor, render target, or semantic input. Only producer-proven optional emptiness, inactive optional DLC, exact null/no-spawn lookup, and explicit `NotSkinned` are valid absence.

The fixed display composition order remains: semantic population filters; terrain/city lighting; FoW; distance fog; day/night; optional radar; M03 arrows in recorded engine order; then counter quads. GUI registration order is never substituted for render order.

## Active producer gates

| Packet | Required contribution | Current result |
|---|---|---|
| XN13 | dynamic entity presentation, semantic animation state, exact per-pass skinning palette | `XN17I_E_XN13_ENTITY_PRESENTATION_UNFROZEN` |
| XN14 | weather state, membership, lifecycle, and ordered particle composition | `XN17I_E_XN14_WEATHER_COMPOSITION_UNFROZEN` |
| XN15 | semantic effect/entity/direct-producer/light/weather recording stream | `XN17I_E_XN15_SEMANTIC_STREAM_UNFROZEN` |
| XN16 | synchronized base-map pass graph and frozen final compose check | `XN17I_E_XN16_FINAL_COMPOSE_UNFROZEN` |

Each gate requires four explicitly pinned artifacts: report, normative contract, successful verification result, and evidence manifest. New files or changed status prose have no effect until XN17I is rebuilt with those verified hashes. XN16 additionally requires same-round synchronized terrain/trees/water/rivers, an authenticated rivers pass-close render target, and the final compose-check result; PAN2 is already closed by XN16P.

## Controlled absence and scope

Aliases, visually similar assets, guessed defaults, tuned colours, synthesized populations, event-only particle resimulation, cross-capture `ResourceId`/pointer transport, and host-setting inference are forbidden. The four user removals remain outside the exact domain: tree camera-height fade, border chain direction/texture-V, diplomacy fill RGB, and resource fill.

There are no XN17I partial leaves, tool-blocked leaves, data requests, new capture requests, or new reverse-engineering operations. The current XN13–XN16 results are stable typed admission outcomes, not approximations.

## Inherited Ghidra provenance

XN17I makes no new binary claim. It freezes the complete integration-boundary tuples consumed from XN17A and XN12:

| Address | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x14124bf10` | `20260716T040552Z-c00-camera-class-pre-rebuild-neighborhoo-a3312ec3` | `C:\Users\Example\ghidra_agent_exports\20260716T040552Z-c00-camera-class-pre-rebuild-neighborhoo-a3312ec3\evidence\func_14124bf10.decompile.c` | `f60c8c6e12a6afcfaa9295cec0541ff4b66a85192a4decaf311ecff4a03ed633` |
| `0x1424e8ca0` | `20260716T004707Z-c01-exact-matrix-core-v1-70d9a3d8` | `C:\Users\Example\ghidra_agent_exports\20260716T004707Z-c01-exact-matrix-core-v1-70d9a3d8\evidence\func_1424e8ca0.decompile.c` | `46b22c5f59a0ef708efd539fb4a47b12afe51ba0b3276c3e359217563aac8511` |
| `0x14124e290` | `20260716T090609Z-c03-wheel-clamp-update-promotion-v1-1f4090a4` | `C:\Users\Example\ghidra_agent_exports\20260716T090609Z-c03-wheel-clamp-update-promotion-v1-1f4090a4\evidence\func_14124e290.decompile.c` | `e194ce0d243e5bf94d3c0371035b7b4a7f5f8a39e9919599557ff35dbdc9290b` |
| `0x14124c280` | `20260716T084427Z-c03-camera-scalar-writer-clamp-v1-384d67eb` | `C:\Users\Example\ghidra_agent_exports\20260716T084427Z-c03-camera-scalar-writer-clamp-v1-384d67eb\evidence\func_14124c280.decompile.c` | `1f357289fc4e4860233fa3c14fbd7d20ede31717bbd69b53980dcd2edb803a93` |
| `0x14124cdd0` | `20260716T090609Z-c03-wheel-clamp-update-promotion-v1-1f4090a4` | `C:\Users\Example\ghidra_agent_exports\20260716T090609Z-c03-wheel-clamp-update-promotion-v1-1f4090a4\evidence\func_14124cdd0.decompile.c` | `e194ce0d243e5bf94d3c0371035b7b4a7f5f8a39e9919599557ff35dbdc9290b` |
| `0x14124c8c0` | `20260716T092319Z-c04-current-wrap-updater-and-view-constr-5d046294` | `C:\Users\Example\ghidra_agent_exports\20260716T092319Z-c04-current-wrap-updater-and-view-constr-5d046294\evidence\func_14124c8c0.decompile.c` | `8379801854757045986f695acb91bca67bb03f8b3bf7c5f28fbae26813eb974a` |
| `0x140dedfc0` | `20260717T000245Z-xn08-mapmode-registry-102f3ba3` | `C:\Users\Example\ghidra_agent_exports\20260717T000245Z-xn08-mapmode-registry-102f3ba3\evidence\func_140dedfc0.decompile.c` | `418ff9caa5584d42625a137d4fd56bf69d99335d44b799e9f6da46058edfa903` |
| `0x141563c80` | `20260716T235833Z-xn10-toggle-registration-and-radar-color-069992c0` | `C:\Users\Example\ghidra_agent_exports\20260716T235833Z-xn10-toggle-registration-and-radar-color-069992c0\evidence\func_141563c80.decompile.c` | `1ed289663731268cb6f1354afd1a598863864be1fd3e3e45c40f7a02feec3d9b` |
| `0x1423e8590` | `20260717T002544Z-xn12a-particle-global-registry-lookup-d021786f` | `C:\Users\Example\ghidra_agent_exports\20260717T002544Z-xn12a-particle-global-registry-lookup-d021786f\evidence\func_1423e8590.decompile.c` | `8ac0771e940cc091900793046deee7242dc4f7875a02bde6c85d6c227e6a8757` |
| `0x14227ea10` | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_14227ea10.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x14227ec50` | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_14227ec50.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x14227ecd0` | `20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e` | `C:\Users\Example\ghidra_agent_exports\20260717T003401Z-xn12a-cpdxparticleobject-lifecycle-famil-9ce8888e\evidence\func_14227ecd0.decompile.c` | `45770eba51fc2d6ac64c490f364310c5140e5f1823cf39628b5bbdae2a02de9a` |
| `0x1423ddaa0` | `20260717T005200Z-xn12b-particle-update-sort-workers-7b289392` | `C:\Users\Example\ghidra_agent_exports\20260717T005200Z-xn12b-particle-update-sort-workers-7b289392\evidence\func_1423ddaa0.decompile.c` | `4a8e7d57f6ba78d656c55098eea51249c41877ab7bbfc55a0ac3bf16bbfb41a6` |
| `0x1423dfd30` | `20260717T005200Z-xn12b-particle-update-sort-workers-7b289392` | `C:\Users\Example\ghidra_agent_exports\20260717T005200Z-xn12b-particle-update-sort-workers-7b289392\evidence\func_1423dfd30.decompile.c` | `4a8e7d57f6ba78d656c55098eea51249c41877ab7bbfc55a0ac3bf16bbfb41a6` |
| `0x14227f420` | `20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19` | `C:\Users\Example\ghidra_agent_exports\20260717T004345Z-xn12a-cpdxparticleobject-update-callers-98b14e19\evidence\func_14227f420.decompile.c` | `a64495ae47dfaaa2a6f752ca1b5d96ec1f764041b68074ea8e79d88cea51f6d1` |

## Mechanical closure

`build_xn17i.py` validates all 16 frozen upstream artifacts before emitting the contract and snapshot. `verify_xn17i.py` checks the complete dependency policy, all 15 inherited Ghidra tuples, exact display/VFS/particle invariants, eight typed rejection classes, all 16 combinations of the four active gates, and every manifest file. The frozen result passes **124/124** checks.

No collector, replay production source, upstream packet, or `PROGRAM_STATUS.md` file was modified. No adversarial review was performed.
