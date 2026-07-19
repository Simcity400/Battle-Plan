# XUI15 — country resource-trade state

Status: **PROVEN / REPLAY PROMOTION-READY / EXTERNAL ROUTE-FLAG LEAVES FAIL CLOSED**

This packet is `XUI15`, not canonical archive-mount packet `UI15`. No extractor file or `PROGRAM_STATUS.md` was changed by this replay lane.

## Promotion result

The replay owns a typed country trade screen over `replay_core::TradeView` and the shipped `countrytradeview.gui`. It admits and renders exact measured state without simulating trade, substituting equipment-market contracts, sorting controller rows, or converting absent data to zero.

Implemented replay behavior:

- complete resource pools require all seven positions in exact order: oil, aluminium, rubber, tungsten, steel, chromium, coal;
- resource and deal arrays require contiguous zero-based `visible_order`; a wrong order rejects the whole section;
- controller values remain independent: balance, produced, imported, exported, special-project use, production-line use, ordinary need, and oil current/maximum need;
- classic deals preserve ID, partner, direction, resource, requested and delivered amounts, available export, civilian-factory cost, per-resource factory cost, convoy counts, route kind and efficiency;
- complete-only replay totals cover imported, exported, requested, delivered, civilian factories, assigned convoys and required convoys;
- partners are stable-unique in measured deal order, and `no_convoys` is exactly `assigned < required`;
- factory total, used and available values are admitted independently and are never recomputed from generic factory totals;
- domain-absent, domain-unavailable, country-absent, incomplete witness, missing row, arithmetic overflow and missing producer leaves have distinct fail-closed outcomes;
- `resources_info_entry` and `country_trade_entry` are instantiated from the shipped GUI, populated in recorded order and rendered through the production compositor;
- missing resource/deal sections render explicit unavailable messages rather than empty tables.

## Producer boundary

The current carrier supplies structurally complete six-pool `MTRF v2`, controller rows through `MTRR`, factory headers through `MTRH`, and classic deals through `MTRD`. Classic resource deals remain distinct from D43 equipment-market purchase contracts.

Two exact route status bytes are not yet carried by `MTRD`: route `+0xE8` no-influence and route `+0xE9` no-resources. Replay schema and typed rows accept them when produced and otherwise retain `Absent`/`Unobtained`; they are never inferred from delivery values or convoy state. This is the only external producer dependency in the promoted replay surface.

## Mechanical verification

Two consecutive complete passes succeeded:

- focused Python schema: 53 passed, 0 failed, twice;
- focused `replay_core` XUI15: 3 passed, 0 failed, twice;
- focused `game_ui` XUI15 admission/rendering: 4 passed, 0 failed, twice;
- full Python replay suite: 236 passed, 0 failed, 24 skipped, twice;
- full `replay_core`: 110 passed, 0 failed, 1 environment-only test ignored, twice;
- full `game_ui`: 354 passed, 0 failed, 1 environment-only test ignored, plus the shipped-file acceptance probe, twice;
- acceptance census per pass: 453 GUI files, 9,153 windows, 27,123 elements, 162 GFX files and 25,364 sprites;
- XUI15 verifier: 25/25 passed twice with byte-identical manifest SHA-256 `f7bfc6bd646b27b38ad077ef3e6dfa460080874c57949757a9e5a8446124dba4`.

Cleanup removed only the two XUI15 Cargo targets and replay Python bytecode caches. Exact reclaimed logical bytes: `6,854,565,535`. All four paths were verified absent afterward. Source, proof PNGs, manifests, Ghidra exports, RenderDoc captures and shared evidence were preserved.

## Frozen hashes

- `game_ui/src/trade_state.rs`: `1c53fc58bfa89fefbc716dbe11072ac52cbda28afaa04414b189812f64f33be6`
- `game_ui/tests/xui15_trade_state.rs`: `f835f4226c08cc449af1fe92654de9ece85a1fb630e5e7ac4df8c501fd208cbf`
- `replay_core/src/world.rs`: `0d16aa03c664213e0e35a0bfa31488f6cec4cd3f15decfc47f55b21b0400450f`
- Python schema: `a9f880a7368bbb9fe6f4aabe20eee9287bc66dd2187c918bb362597a5ee83e2e`
- Python schema tests: `de009a1befa552e97d3e7eb0de854a722ac17cb772359167eab5df7acb11afb7`
- production contract: `c14d0b6d078257980ab52fecd269536bbdc49c69a3cf9b9c35be69c93e1d7d72`
- verifier: `864cc1ff4b65eb482027a952b1b758e1dc6c7c35a5bae9b7101392301c6bd0df`
- verification result: `5399e08cab400ff0789462c38151a4dbb173e23d0228a1b5cd5511945bfce82c`
- retained capture census: `7f5f4ffd1022cde265fd7099744f5e59674d60c62dfdf2cde10689981c0b89a4`
- evidence manifest: `f7bfc6bd646b27b38ad077ef3e6dfa460080874c57949757a9e5a8446124dba4`

## Binary provenance

Authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

No repeat Ghidra operation was necessary; replay consumed the frozen X15 evidence. The full provenance ledger remains in `extractor/documents/dossiers/X15_RESOURCE_TRADE_INPUTS.json`.

| Function | Job ID | Artifact path | Manifest hash |
|---|---|---|---|
| `0x141CA44A0` | `20260717T101421Z-x15-trade-view-row-controller-producers-323ae188` | `evidence/func_141ca44a0.decompile.c` | `3ab1714810f4cf7e5d8d484b438b90c175c832d0149d590a7bdef66ea06f626e` |
| `0x140C9F7A0` | `20260717T102907Z-x15-classic-route-and-header-value-produ-c7885736` | `evidence/func_140c9f7a0.decompile.c` | `f255d6515f654915c315680215049580f1cecc9d40b1e621114c8a69e5bf72bb` |
| `0x141CA5C60` | `20260717T102907Z-x15-classic-route-and-header-value-produ-c7885736` | `evidence/func_141ca5c60.decompile.c` | `f255d6515f654915c315680215049580f1cecc9d40b1e621114c8a69e5bf72bb` |
| `0x1415ED1C0` | `20260717T102907Z-x15-classic-route-and-header-value-produ-c7885736` | `evidence/func_1415ed1c0.decompile.c` | `f255d6515f654915c315680215049580f1cecc9d40b1e621114c8a69e5bf72bb` |

No adversarial review or capture request was created.
