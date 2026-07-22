# XUI15 — country resource-trade state

Status: **PROVEN / EXACT PIXEL ROUTE IMPLEMENTED / LIVE ADMISSION RECEIPT PENDING**

This packet is `XUI15`, not canonical archive-mount packet `UI15`. The Wave 4 producer integration updates the extractor carrier/fold and this program-status row without changing canonical archive-mount packet `UI15`.

## Promotion result

The replay owns a typed country trade screen over `replay_core::TradeView` and the shipped `countrytradeview.gui`. Exact-a729 RE now identifies `CCountryTradeView::Open` at RVA `0x015EC520` and `Close` at RVA `0x015EAF40` as the authored `countrytradeview` lifecycle. The extractor emits that independent observation through `MTRA v1`, replay validates it as `MarketTradeActivationView`, and the production renderer admits XUI15 only while the exact active country agrees with the selected POV. No always-on, semantic-presence, or selected-POV activation inference is reachable.

Implemented replay behavior:

- complete resource pools require all seven positions in exact order: oil, aluminium, rubber, tungsten, steel, chromium, coal;
- resource and deal arrays require contiguous zero-based `visible_order`; a wrong order rejects the whole section;
- resource rows require the exact a729 integer display semantics, including oil current/maximum and non-oil balance/need/type agreement;
- deal IDs are unique and positive in replay, packed identity is checked in the fold, and each world deal must have exactly one agreeing importer/exporter half;
- controller values remain independent: balance, produced, imported, exported, special-project use, production-line use, ordinary need, and oil current/maximum need;
- classic deals preserve ID, partner, direction, resource, requested and delivered amounts, available export, civilian-factory cost, per-resource factory cost, convoy counts, route kind and efficiency;
- complete-only replay totals cover imported, exported, requested, delivered, civilian factories, assigned convoys and required convoys;
- partners are stable-unique in measured deal order, and `no_convoys` is exactly `assigned < required`;
- factory total, used and available values are admitted independently and are never recomputed from generic factory totals;
- domain-absent, domain-unavailable, country-absent, incomplete witness, missing row, arithmetic overflow and missing producer leaves have distinct fail-closed outcomes;
- `resources_info_entry` and `country_trade_entry` are instantiated from the shipped GUI, populated in recorded order and rendered through the production compositor;
- missing resource/deal sections render explicit unavailable messages rather than empty tables.
- every X15 trailer is current-hour full replacement; a missing family clears instead of projecting an older trade screen.
- `MTRA v1` is omitted unless the active view's engine country pointer joins the current exact country census; malformed, unjoined, absent and torn activation observations fail closed;
- a closed exact lifecycle carrier produces no pixel surface, while an active carrier without matching selected-country semantic d43 state is rejected rather than rendered.

## Producer boundary

The current carrier supplies structurally complete six-pool `MTRF v2`, controller rows through `MTRR`, factory headers through `MTRH`, and classic deals through `MTRD v2`. Classic resource deals remain distinct from D43 equipment-market purchase contracts.


Ghidra authority is job `20260722T013354Z-x15-open-close-lifecycle-521d1851`, manifest SHA-256 `de07756d379ffa51de678e5083ac8f4c6efb7453bfc20306499c44b18fba78e2`. Hook sentinels are pinned by job `20260722T013608Z-x15-activation-hook-sentinels-feaea801`, artifact SHA-256 `7dff1c3afb8d48be2987c5ce1e9f9ade92cf805094ef634fef75b444d835c62b`.

`MTRD v2` now carries the exact route `+0xE8` no-influence byte and route `+0xE9` no-resources byte in the former per-row reserved word. The collector admits only raw byte values zero or one, reciprocal halves must agree, the fold emits measured booleans, and the existing typed replay/UI path consumes them. `MTRD v1` remains accepted for old recordings but projects neither field, so absence cannot silently become false. Because the schema version belongs to the single world trailer, replay and the portable schema now reject any complete snapshot that mixes v1 and v2 rows across otherwise valid reciprocal deals.

## Wave 5 hardening

Fresh exact-a729 Ghidra job `20260720T140456Z-wave5-x15-d43-classic-resource-trade-rea-fccf30e0` exported 18 relevant functions on `shard-10` under manifest SHA-256 `562b1f247c21d25099501d7cea93023d0c38cf5e6f2028a4b622f4abbd0ac077`. Every fresh function artifact hash matched the frozen X15 authority, so the binary laws did not drift.

The remaining production audit fixed torn six-pool reads, stale current-hour trailer retention, descriptor alias/order acceptance, packed deal-identity acceptance, fail-open replay row/value/reciprocal validation, and the dormant selected-POV semantic consumer. The final adversarial pass also preserved explicit `complete=false` as a measured witness while withholding dependent values, poisoned all derived pool values when pool validation fails, enforced signed-32-bit factory header values in both replay implementations, and made the MTRD v1/v2 selection world-atomic. `XUI15` is no longer classified as producer-authority blocked; only authenticated window activation/root binding blocks its pixel canvas.

## Historical mechanical verification

The Wave 4 MTRD v2 change adds static carrier, compatibility, reciprocal-parity, fold, and projection tests. Root-owned centralized validation is required before the hashes and historical pass counts below are re-frozen; this agent did not run Cargo, tests, verifiers, compilers, or formatters.

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

## Historical frozen hashes

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

A fresh 18-function re-audit matched the frozen artifacts byte-for-byte at the decompile-artifact hash level. The full provenance ledger and fresh job receipt remain in `extractor/documents/dossiers/X15_RESOURCE_TRADE_INPUTS.json`.

| Function | Job ID | Artifact path | Manifest hash |
|---|---|---|---|
| `0x141CA44A0` | `20260717T101421Z-x15-trade-view-row-controller-producers-323ae188` | `evidence/func_141ca44a0.decompile.c` | `3ab1714810f4cf7e5d8d484b438b90c175c832d0149d590a7bdef66ea06f626e` |
| `0x140C9F7A0` | `20260717T102907Z-x15-classic-route-and-header-value-produ-c7885736` | `evidence/func_140c9f7a0.decompile.c` | `f255d6515f654915c315680215049580f1cecc9d40b1e621114c8a69e5bf72bb` |
| `0x141CA5C60` | `20260717T102907Z-x15-classic-route-and-header-value-produ-c7885736` | `evidence/func_141ca5c60.decompile.c` | `f255d6515f654915c315680215049580f1cecc9d40b1e621114c8a69e5bf72bb` |
| `0x1415ED1C0` | `20260717T102907Z-x15-classic-route-and-header-value-produ-c7885736` | `evidence/func_1415ed1c0.decompile.c` | `f255d6515f654915c315680215049580f1cecc9d40b1e621114c8a69e5bf72bb` |

No human-input request is required for the carrier, replay route, or autonomous activation action: the authored `shortcut = "r"` on `trade_button` provides an exact non-coordinate route. Final promotion still requires running the acquisition against the already loaded exact-a729 D04 game and authenticating a closed-active-closed MTRA sequence plus the corresponding replay pixel output. This document deliberately does not claim that unrun live receipt.
