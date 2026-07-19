# XUI07 — runtime UI graphics

Disposition: **PROVEN — READY TO PROMOTE**

This packet is bound to `hoi4.exe` 1.19.2.0.a729, image base
`0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

XUI07 implements a fail-closed replay seam for runtime sprite/frame selection,
button state shading, animation clock/order joins, clipping, opacity, and final
composition. Exact built-in shading is limited to shipped, hash-bound
`buttonstate.shader`, `buttonstate_blendframes.shader`, `static_button.shader`,
and `sprite_animation.fxh`. Every other effect family requires finite
producer-resolved shaded RGBA plus source-record provenance; no visual tuning or
shader substitution is admitted.

The independent shipped-file census covers 162 `.gfx` files, 3,863 effect
assignments, 14 normalized aliases, and 14 resolved shader files. No effect
alias is data-blocked. The full assignment/file ledger is retained in
`replay_interface/re_evidence/rendering_laws/XUI07/gamefile_census.json`.

UI02 supplies exact raw button states, one-based frames 1..4, non-current-hover
fallback to frame 1, and transition order. UI03 supplies complete
visibility/traversal and position-animation law. UI04 supplies exact UI delta
bits and consume-previous/write-current ordering. Retained RenderDoc evidence
corroborates the final pass but is not overclaimed as source-to-compiled-shader
identity.

Production requires an exact nonempty producer clip and finite opacity in
`[0,1]`. Clips intersect with half-open pixel bounds. Opacity multiplies source
alpha before the shader-declared fixed blend. Source-alpha/inverse-source-alpha
is used only for shader files that declare it; blend-disabled or otherwise
unimplemented states require producer-resolved final RGBA.

Implementation:

- `native/game_ui/src/runtime_graphics.rs`, SHA-256
  `6ba74d29ce03374630b6042574784ad9ce2b1f2728cbf4a497959a68a54bf44c`;
- `native/game_ui/tests/xui07_runtime_graphics.rs`, SHA-256
  `36568d09db0973d30ded7f31fe25e868f7e7f45add5682720b6e82611843bb86`;
- 15/15 focused tests pass; the combined full `game_ui` suite passes;
- promotion contract SHA-256
  `4da2f8bb74099af37607325b89064874f16349112ed3685ac0df4040eeebb7b0`;
- two byte-identical verifier results SHA-256
  `35c7cc70292644d3863c1054a540ac2c0ac23c4c616c6af2f96bf0b211e5fed5`.

Open leaves: none. Human capture request: none.

## Primary Ghidra provenance

| Function address(es) | Job ID | Artifact path | Manifest hash |
|---|---|---|---|
| `0x14229b650`, `0x14229bd10`, `0x14229b4b0` | `20260716T022530Z-ui02-button-event-state-helper-chain-d0--36127cb0` | `evidence/_FUNCTION_EVIDENCE_MANIFEST.json` | `bd8244fa5b3e73604e1648e641011241b3c20ba64135a4ea36325c313d59ed19` |
| `0x1422bbb70`, `0x1422bcd30` | `20260716T031244Z-ui02-cbuttonstandard-state-sound-consume-34ce1666` | `evidence/_FUNCTION_EVIDENCE_MANIFEST.json` | `b53d3e968c14bcc40414da2094fc5f600e0cfd42e31a9dd13533f5d312fb4a68` |
| `0x1422bc940`, `0x1422bca30`, `0x1422bc8a0` | `20260716T031411Z-ui02-cbuttonstandard-down-over-disable-c-819a8b85` | `evidence/_FUNCTION_EVIDENCE_MANIFEST.json` | `c17af1d73f6e145c44e29e70fd5a67ae25968ce244df0f0eff0004b1b5ba9212` |
| `0x1422b01e0` | `20260716T052828Z-ui03-container-state-scan-b0000-b0fff-8277b271` | `evidence/func_1422b01e0.decompile.c` | `5a5848e827371cd48996853196a37f88324f31c099da065c0314722d37b6de43` |
| `0x1422af780`, `0x14254c2a0`, `0x14254b3d0` | `20260716T054324Z-ui03-animation-easing-helper-exact-v1-8931dd93` | `evidence/func_1422af780.decompile.c` | `7da8758aa9e0a978f9120381dfc25ab4f218191a9b0a2fed6214d9d1d5c057d8` |
| `0x1402a0290` | `20260716T040648Z-ui04-cgameidler-idler-core-vmethods-d2858e50` | `evidence/func_1402a0290.instructions.txt` | `5b946cc516d91e4f3b5575d34c28df11eff9f47a9dc0f98d7e0210d1b13d13ee` |
| `0x142238c90`, `0x1401f7b60`, `0x142238ce0` | `20260716T041225Z-ui04-qpc-wrapper-and-normal-ui-dispatch-a1d8ae98` | `evidence/_FUNCTION_EVIDENCE_MANIFEST.json` | `2d71967392902965fcd2c5d1447414422358afcdd54dc52f26cb3dfd5f601cfd` |
| `0x140b594d0`, `0x1412218d0` | `20260716T040936Z-ui04-updateable-dispatch-order-7e4923bb` | `evidence/_FUNCTION_EVIDENCE_MANIFEST.json` | `1f09fe9f8946103d257d4d083ecbc8d55792027826684d3c16b3a9820c78e538` |

All paths are relative to the immutable broker job directory. No new Ghidra
operation was used; upstream incidental, negative, and failed findings remain
preserved in their original ledgers.
