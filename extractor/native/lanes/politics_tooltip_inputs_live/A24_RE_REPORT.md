# A24 politics-tooltip upstream RE and d73 carrier

Status: **promotion-ready: exact a729 native source, d73 carrier, collector seam, registry entry, and production fold are installed and fail closed.**

This report belongs only to the extractor A24 lane. It does not amend or reuse the rendering-laws `PROGRAM_STATUS` surface.

## Frozen authority

- Program: `hoi4.exe 1.19.2.0.a729`
- Image base: `0x140000000`
- Executable SHA-256: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
- Identity manifest SHA-256: `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`

## Exact producer findings

All rows below bind the function address, Ghidra job, exact artifact, whole-job manifest hash, and artifact-member hash. The common job is `20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362`; its manifest is `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\_FUNCTION_EVIDENCE_MANIFEST.json`, SHA-256 `aec720fe9371b012bfb6a8f000f74ab39232f3482a766c0edfac7582ce489ce5`.

| Function | Exact finding | Artifact path | Member SHA-256 |
|---|---|---|---|
| `0x141c33fe0` | Core politics idea-entry refresh. Reads the `CIdea` pointer at entry `+0x30`; evaluates availability (`0x140fbeb00`) and active removal permission (`0x140fbea50`); computes effective cost through `0x140ba56a0 -> 0x1406c2d50 -> 0x140b9a060`; checks affordability with `0x140fb8f30`; resolves runtime name/description with `0x140fbcf90`; emits dynamic rows through `0x141c350f0`/`0x141c35770`; then calls `0x141c346f0`. It also emits exact mobilization/trade-law stat rows from the current idea category. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141c33fe0.decompile.c` | `3abb0bcb748c83cffb2258e03a115ba5efdb40a9435e145c56bcd43898cb9f7b` |
| `0x141c346f0` | Final effective-cost formatter. Formats the already-computed exact value, rechecks affordability, and emits the unavailable path through `IDEA_NOT_AVAILABLE_ICON`. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141c346f0.decompile.c` | `4cf5baca00d6d2ff30fae8f497ef65dad84c871caa8a3bfcb4c157d044da622b` |
| `0x140fb8f30` | Exact resource affordability predicate. Types 0/1 compare against political power at status `+0xE0`; types 2/3/4 compare the three XP resources at `+0x10`, `+0x28`, and `+0x40`. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_140fb8f30.decompile.c` | `55c84e73535629898d4e6150b706b34982f158b4523d1662b6bf5490baee75c9` |
| `0x140fbea50` | Exact `allowed_to_remove` evaluation. The no-trigger case succeeds; otherwise the current evaluation context invokes the idea-side trigger object. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_140fbea50.decompile.c` | `144ea54ab4572d0b8797daa2bd2145ca2a77255a39bc9e6929deafad230f5c0d` |
| `0x140fbcf90` | Resolves idea name/description through the live runtime context, not a static token-only lookup. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_140fbcf90.decompile.c` | `44ab839934419b48a07d0f85400455b07d6daac0f8a4139ef66fd31563d176a0` |
| `0x141c3b870` | Political-ideas window switch path. Uses current idea at view `+0x568` and country/context at `+0x560`, selecting `IDEA_WINDOW_CANNOT_SWITCH` versus `IDEA_WINDOW_SWITCH`. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141c3b870.decompile.c` | `6ade199cc854c6e1a8f93919952f98435d5aadcac4133dcac2feb225e58d56b0` |
| `0x141c2de30` | Constructs `CPoliticsIdeaCategoryEntry`, binding the authored category entry to its live country/view state. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141c2de30.decompile.c` | `390f830706e74edb98101fa62387f33ef4f307ee4f6d729ee0f7bafeaf9ede70` |
| `0x14156a2f0` | Constructs `CCountryPoliticsView` and binds `countrypoliticsview` plus `political_ideas_window`. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_14156a2f0.decompile.c` | `1ada38d5b0554abddf1943b705cb83a3edc0530ee08783563098bd2b150295c9` |
| `0x140fbf9c0` | Generic modifier formatter; selector `0x1b` prefixes politics modifiers with `MODIFIER_POLITICS_PREFIX`. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_140fbf9c0.decompile.c` | `91e8d3be46f0164f18ae6c6ad5f1a8ebc974b959dc2586f0b6505d8ed0c48cc5` |
| `0x141c32a50` | Enumerates politics slot/category definitions, constructs advisor/category entries, assigns slot-family codes, and contains the `politicalitems.cpp:0x36a` `unknown slot type` assertion. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141c32a50.decompile.c` | `1dc9f0a49d5e3e4607e68bbb4b70f7227ca282f0da9ac46a1de43a6171405759` |
| `0x141c43430` | Incidental widget binding: resolves progress, modifier container, and related presentation handles for the politics item. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141c43430.decompile.c` | `93e52c9caeffaeb462cb53f12688584cf6dfb1764ebe06e8c4933e25bb1fed10` |

The string-xref job is `20260717T103021Z-a24-politics-tooltip-anchors-v1-4af7c984`. Artifact `C:\Users\Example\ghidra_agent_exports\20260717T103021Z-a24-politics-tooltip-anchors-v1-4af7c984\string-xrefs.txt` has SHA-256 `4096785ffe2701327a1702ddc5e4640255f8f0505c050cf684376ddbbc07a3c6` and identity-manifest SHA-256 `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`. It binds `countrypoliticsview`, `political_ideas_window`, `country_politics_idea_category_entry`, `modifier_container`, `IDEA_WINDOW_CANNOT_SWITCH`, `IDEA_NOT_AVAILABLE_ICON`, and `MODIFIER_POLITICS_PREFIX` to the functions above.

## Incidental findings retained

These were encountered while closing A24 and are preserved without promoting them beyond what the named artifact proves. All four decompiles share Ghidra job `20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362`, artifact manifest `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\_FUNCTION_EVIDENCE_MANIFEST.json`, manifest SHA-256 `aec720fe9371b012bfb6a8f000f74ab39232f3482a766c0edfac7582ce489ce5`.

| Function | Retained finding | Artifact path | Member SHA-256 |
|---|---|---|---|
| `0x141d0bd80` | Rebuilds the political-ideas window item for a country/controller, then invokes the exact switch, focus and politics refresh paths including `0x141c3b870`. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141d0bd80.decompile.c` | `4c9bfce72c5857987c3c0038aa6a1b86ba40ab5f623f2ec1520f12882526bb87` |
| `0x141d0c970` | Refreshes three mutually selected politics-window widgets from the live country mode at the object reached from `param_1+0x10`, then triggers the attached callback. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141d0c970.decompile.c` | `8abc471c57a71c9c3600e4efe742c07ad3f481af94a6c56afa56980474ec139c` |
| `0x140b52390` | Global UI-manager construction includes the `CCountryPoliticsView` allocation/constructor at `0x14156a2f0`; it also exposes the surrounding view-construction order useful to later producer scheduling. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_140b52390.decompile.c` | `f304ca13aa649a99ba45d0d03d8f7476a44777f818ba290a6473373fe5c642f4` |
| `0x141c47e90` | Incidental A37 topology: constructs `CPowerBalanceView`, resolves authored `powerbalanceview`, and installs its callbacks/state. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141c47e90.decompile.c` | `89e347253650cd2f4d54ba60c746b505342d81deea603cd410455973e7db3c75` |

The string-xref artifact above also binds `MODIFIER_BASE_VALUE` to `0x141d1afd0` and multiple generic modifier producers. This is an anchor only: function `0x141d1afd0`, job `20260717T103021Z-a24-politics-tooltip-anchors-v1-4af7c984`, artifact `C:\Users\Example\ghidra_agent_exports\20260717T103021Z-a24-politics-tooltip-anchors-v1-4af7c984\string-xrefs.txt`, artifact SHA-256 `4096785ffe2701327a1702ddc5e4640255f8f0505c050cf684376ddbbc07a3c6`, identity-manifest SHA-256 `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`.

## Non-evidence infrastructure incidents

The pre-existing helper export `20260717T103804Z-a24-politics-runtime-helpers-v1-1a23f7fd` requested `0x140fbeb00`, `0x140b9a060`, `0x1406c2d50`, `0x140ba56a0`, `0x141c350f0`, and `0x141c35770`. Its only readable artifact is `job.json`, SHA-256 `58078f2f6a995e02dbe5a14887b696b01f50d55d5653ed4ce79d6bb1a46e7331`; it remains marked running and produced no function manifest. Therefore none of those helper addresses is claimed from that job, and the operation was not duplicated.

Three failed runtime-text searches are recorded for completeness and are not semantic evidence: `20260717T105856Z-a24-politics-runtime-text-v2-b4b31899`, `20260717T110320Z-a24-politics-runtime-text-v2-retry-dad5addf`, and `20260717T110352Z-a24-politics-runtime-text-v2-retry2-2b4cf2b8`.

Further non-evidence attempts were retained rather than silently discarded: advisor producer export `20260717T123539Z-a24-advisor-disabled-reason-producer-v1-10fefac4` for `0x14128f730`; modifier-breakdown export `20260717T125539Z-a24-modifier-breakdown-producer-v1-2b4115fd` for `0x141d1afd0`; and string searches `20260717T124000Z-a24-politics-party-advisor-runtime-ancho-349bd79f`, `20260717T124237Z-a24-popularity-change-anchor-v2-41072875`, `20260717T124245Z-a24-timed-party-drift-anchor-v2-bd631263`, `20260717T124254Z-a24-advisor-cost-anchor-v2-883d97a3`, `20260717T124301Z-a24-advisor-short-cost-anchor-v2-757386d1`, and `20260717T124310Z-a24-advisor-cp-cost-anchor-v2-a12e1373`. Each failed during the identity/export phase and produced no semantic artifact or manifest; no claim is based on them.

## Game-file and retained-capture evidence

The effective installed GUI is `<local-path> of Iron IV\interface\countrypoliticsview.gui`, SHA-256 `811051a83ef087b87be4de9b226fb831f6d6d8f20cd440371619501b5710c183`. It contains the idea category/grid, political-ideas window, selectable entries, and modifier container used by the producer graph.

The English politics localisation is `<local-path> of Iron IV\localisation\english\politics_l_english.yml`, SHA-256 `ea6346f4e10dab0b7bec0fb55f165390bdd9158eb3c7b33207379ba8687b4423`. It pins `REMOVE_IDEA_POLITICAL_POWER`, `IDEA_WINDOW_SWITCH`, `IDEA_WINDOW_CANNOT_SWITCH`, `IDEA_COST_POLITICAL_POWER`, `IDEA_NOT_AVAILABLE_ICON`, `POLITICS_IDEA_STAT_TRADE`, and `POLITICS_IDEA_STAT_RECRUITABLE`.

The XUI11 retained-store receipt covers 214 RDCs and 69,298,844,274 bytes with zero politics/advisor/minister/national-spirit filename hits. That is an exhaustive path/filename result, not visual OCR. A capture cannot replace resolved trigger results, aggregate terms, evaluation context, or the completeness witness, so A24 requires no new human capture.

## Production carrier and fold

Reserved unscheduled domain 73 uses `PTI1` schema v1. Its exact row key is `{frame_hour, country_tag, target_kind, target_token_or_slot, line_kind}`. It carries measured/not-applicable/unobtainable availability, exact raw integer plus scale, source identity, evaluation context, a producer-manifest SHA-256, completeness-record identity, and expected row count.

The six typed families are trigger result/reason tree, effective hire/removal cost, conditional-modifier aggregate with exact source terms, advisor-slot capacity/vacancy/eligibility/disabled reasons, timed dynamic modifier value/source/expiry, and displayed party-popularity change sources. Validation rejects duplicate keys, mismatched typed variants, malformed reason trees, nonmonotonic source ordinals, aggregate sum disagreement, scale disagreement, inconsistent advisor vacancy, invalid expiry order, unauthenticated source admission, and cross-hour snapshot attachment.

The collector exposes `encode_a24_exact_politics_tooltip_frame`. `recorder_host` folds clean frames to `_r2_politics_tooltip_inputs_live`, preserves status flags and provenance, distinguishes absent from authenticated measured-empty, emits null on failure or frame-hour mismatch, and never carries a prior hour's politics tooltip input.

Mechanical gates:

- `politics_tooltip_inputs_live_lane`: 7/7 tests passed.
- `compact_contract`: 117/117 tests passed, including registry parity.
- `collector`: cargo check passed.
- dedicated production-library recorder integration: 2/2 tests passed, covering all six families and cross-hour rejection.

The old monolithic `recorder_host` lib-test target currently fails before selecting A24 because unrelated FRD test helpers reference absent `FRD_HEADER_LEN_V2` and `FRD_SCHEMA_VERSION_V2`. The dedicated `recorder_host/tests/a24_d73.rs` target compiles the production recorder library and passes; A24 does not modify those unrelated FRD symbols.

## Authenticated native source

`src/a729_hook.rs` implements `A729PoliticsHookSource` directly behind the existing `ExactPoliticsTooltipSource` trait and therefore feeds the existing `collector::encode_a24_exact_politics_tooltip_frame` seam without a parallel carrier or ABI. Installation verifies the exact 16-byte a729 sentinel `48895c24185556574154415541564157` before placing a MinHook at RVA `0x01C33FE0` / absolute `0x141C33FE0`. The hook records the completed producer tuple `(entry, *idea_id_pointer, country_context)` from the function's three proven arguments.

Frame admission is intentionally stricter than the PTI1 carrier alone. Each producer tuple is consumed once; null tuple parts, reuse, rejection, or bounded-queue eviction poison completeness. Submitted batches must identify either exact formatter callsite RVA `0x0188963C` or `0x018897B4` and the common retained chain tail `0x0162961E -> 0x00DB7668 -> 0x002A4F1C -> 0x00DC16E9 -> 0x0221A1D5`. That identity is backed by 258 authenticated formatter events in `extractor/native/lanes/resolved_tooltip_instances_live/retained_cpu_tooltip_analysis.json`, SHA-256 `7fe5897afe038acd9e7050e50c73b6f37a527e109a4a6b784d185610f16fa016`.

A frame cannot seal until the exact expected producer-tuple count agrees, at least one authenticated dispatch identity is present, every row passes the complete PTI1 invariant set, and all six required semantic families are represented. The source returns only sealed frame-local rows, publishes producer manifest `aec720fe9371b012bfb6a8f000f74ab39232f3482a766c0edfac7582ce489ce5`, and fails closed on any missing frame or completeness violation. Tests cover the successful source-to-d73 encode path plus unobserved tuple, bad chain, and missing-family rejection.
