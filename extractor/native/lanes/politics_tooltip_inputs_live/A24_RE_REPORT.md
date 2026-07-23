# A24 politics-tooltip upstream RE and d73 carrier

Status: **exact diagnostic detours, pointer-free owner capture, and the typed extractor-to-replay/UI carrier are complete; the production semantic PTI1 row producer is not implemented, so d73 remains reserved, unschedulable, and fail closed.**

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

The recovery pass found that the earlier graph was insufficient to authenticate
the six PTI1 families. Exact replacement evidence was exported from the pinned
20-shard broker:

| Exact producer | Proven runtime output | Member SHA-256 |
|---|---|---|
| `0x14128DC50` | Compact advisor PP/CP/allocation costs, affordability coloring, unavailable icon, and final resolved string | `de88afe804eae0f97dadb2854494a770811607209230f8cf4d71862a34d52c1f` |
| `0x14128A780` | Full advisor availability, ordered `CANT_TAKE_IDEAS` reasons, PP/CP/allocation costs, modifiers, affordability, and final strings | `d398e741796715399d0b06cf2031642a0841d6eeb6b5787ccffbfae2a6fef272` |
| `0x140376270` | Exact displayed ideology identity and raw popularity-change line | `f938c228ea9bb38b96c062ca5e2718d13d869a6755e519b38e4450f1be6fc5a3` |
| `0x141C7D4D0` | Full timed-idea tooltip and removal date from `remaining_days * 0x18 + CGameState+0x468` | `0e1bc4dce0c4efbccae14933b6d47d69217bccc5787cd28d31f6b3087ea73de2` |
| `0x141C2B640` | Active-idea runtime description, custom fields/modifiers, removal-trigger text, and `TIMED_IDEA_DATE` | `2ac20a759232db390a88ad29c02c182e500dec2d03e13bd23a23c08bc4bafb0e` |
| `0x141C302E0` | Political-idea remove permission, remove cost, affordability, reasons, and effects | `e1f46c2eda5cd5a8ea9c9df9815e1653d38ee26c4d54a033b754c0561e7bf512` |
| `0x141C2E8C0` | Category occupancy/capacity and exact add/change/no-more-ideas reasons | `553b16a36ff274dcef74c44f6708be36d207126c1c3bbfe4269847d088fcf0b6` |
| `0x141C6AA70` | Boost-party current popularity, raw change, and ordered `WHY` breakdown | `635788349d94fe9ee784744317ffe49084268d7e6c4e873fda15a86dbf709bbf` |
| `0x141BFE110` | Selection disabled-reason owner and compact-cost producer call | `8d839d60b64b317e574a215f5df5f3743b4392ec7214f54733037a23c36d6638` |
| `0x141BFC990` | Compact-cost refresh owner | `fb361e038fd7c708ab0ea3ad5540f46fb3963f3758febfba4532688eafa58d54` |

These ten members share job
`20260720T130541Z-w5-a24-exact-ui-producers-da7088d4`, manifest SHA-256
`18bac19dc5de7fc0ef90a9341e0bc5ed1ac73c745a1dc96f6177ed08c8f70b35`.
The helper graph shares job
`20260720T124332Z-w5-a24-recover-runtime-producers-6a870988`, manifest
SHA-256
`989783832d3fe63a4f87bef2e2ba4e3c34c2048f11e87d55afb15164073e1503`.
It proves exact idea availability at `0x140FBEB00` (member
`065550044d6bc499d88c905e9549c3fd73310352fbe9bf01b4fef289d903aca3`),
effective fixed-point cost accumulation at `0x140B9A060` (member
`7abf1a652c32fa3f3835f0b744610936e35a63afe6bddc90c7eb40d2da1b57ab`),
`CCountry+0xF90 -> CPoliticalStatus` at `0x1406C2D50`, country-handle
resolution at `0x140BA56A0`, and idea modifier-description/stat production at
`0x141C35770`.

The exact token xref receipt is
`20260720T130531Z-w5-a24-runtime-string-producers-426c66d5`, artifact SHA-256
`8ddf4ac6308c965a393a295d0f3e51e72bd51a6b7b56cb4800ebfaeee3d6e2df`.
It binds `POLITICS_IDEA_COST`, `POLITICS_ADVISOR_COST`,
`COMMAND_POWER_ADVISOR_COST`, `POLITICS_IDEOLOGY_POPULARITY_CHANGE`,
`TIMED_IDEA_DATE`, `POLITICS_IDEA_NO_REMOVE`,
`POLITICS_ADVISOR_PP_COST_SHORT`, and `BOOST_PARTY_SELECT_TT` to the
producers above. Exact 16-byte entry sentinels are frozen by jobs
`20260720T131310Z-w5-a24-exact-producer-entry-sentinels-ae52331e`
(artifact SHA-256
`1bcf5829cd3a6af2ca1720ee26f63285ec374fdbc609adb27152769122203270`)
and
`20260720T131506Z-w5-a24-modifier-producer-entry-sentinels-7284f717`
(artifact SHA-256
`7f78ebae425582b1be1ab5ae9b3cd7f0d2c7a174cd4d526d0188b22f5d4e34fa`).

## Incidental findings retained

These were encountered while closing A24 and are preserved without promoting them beyond what the named artifact proves. All four decompiles share Ghidra job `20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362`, artifact manifest `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\_FUNCTION_EVIDENCE_MANIFEST.json`, manifest SHA-256 `aec720fe9371b012bfb6a8f000f74ab39232f3482a766c0edfac7582ce489ce5`.

| Function | Retained finding | Artifact path | Member SHA-256 |
|---|---|---|---|
| `0x141d0bd80` | Rebuilds the political-ideas window item for a country/controller, then invokes the exact switch, focus and politics refresh paths including `0x141c3b870`. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141d0bd80.decompile.c` | `4c9bfce72c5857987c3c0038aa6a1b86ba40ab5f623f2ec1520f12882526bb87` |
| `0x141d0c970` | Refreshes three mutually selected politics-window widgets from the live country mode at the object reached from `param_1+0x10`, then triggers the attached callback. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141d0c970.decompile.c` | `8abc471c57a71c9c3600e4efe742c07ad3f481af94a6c56afa56980474ec139c` |
| `0x140b52390` | Global UI-manager construction includes the `CCountryPoliticsView` allocation/constructor at `0x14156a2f0`; it also exposes the surrounding view-construction order useful to later producer scheduling. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_140b52390.decompile.c` | `f304ca13aa649a99ba45d0d03d8f7476a44777f818ba290a6473373fe5c642f4` |
| `0x141c47e90` | Incidental A37 topology: constructs `CPowerBalanceView`, resolves authored `powerbalanceview`, and installs its callbacks/state. | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-a24-politics-tooltip-producers-v1-c7bba362\evidence\func_141c47e90.decompile.c` | `89e347253650cd2f4d54ba60c746b505342d81deea603cd410455973e7db3c75` |

The string-xref artifact above also binds `MODIFIER_BASE_VALUE` to `0x141d1afd0` and multiple generic modifier producers. This is an anchor only: function `0x141d1afd0`, job `20260717T103021Z-a24-politics-tooltip-anchors-v1-4af7c984`, artifact `C:\Users\Example\ghidra_agent_exports\20260717T103021Z-a24-politics-tooltip-anchors-v1-4af7c984\string-xrefs.txt`, artifact SHA-256 `4096785ffe2701327a1702ddc5e4640255f8f0505c050cf684376ddbbc07a3c6`, identity-manifest SHA-256 `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`.

## Superseded infrastructure incidents

The pre-existing helper export `20260717T103804Z-a24-politics-runtime-helpers-v1-1a23f7fd` requested `0x140fbeb00`, `0x140b9a060`, `0x1406c2d50`, `0x140ba56a0`, `0x141c350f0`, and `0x141c35770`. Its only readable artifact is `job.json`, SHA-256 `58078f2f6a995e02dbe5a14887b696b01f50d55d5653ed4ce79d6bb1a46e7331`; it remains non-evidence. The 2026-07-20 helper job above supersedes that failed attempt with authenticated function members.

Three failed runtime-text searches are recorded for completeness and are not semantic evidence: `20260717T105856Z-a24-politics-runtime-text-v2-b4b31899`, `20260717T110320Z-a24-politics-runtime-text-v2-retry-dad5addf`, and `20260717T110352Z-a24-politics-runtime-text-v2-retry2-2b4cf2b8`.

Further non-evidence attempts were retained rather than silently discarded: advisor producer export `20260717T123539Z-a24-advisor-disabled-reason-producer-v1-10fefac4` for `0x14128f730`; modifier-breakdown export `20260717T125539Z-a24-modifier-breakdown-producer-v1-2b4115fd` for `0x141d1afd0`; and string searches `20260717T124000Z-a24-politics-party-advisor-runtime-ancho-349bd79f`, `20260717T124237Z-a24-popularity-change-anchor-v2-41072875`, `20260717T124245Z-a24-timed-party-drift-anchor-v2-bd631263`, `20260717T124254Z-a24-advisor-cost-anchor-v2-883d97a3`, `20260717T124301Z-a24-advisor-short-cost-anchor-v2-757386d1`, and `20260717T124310Z-a24-advisor-cp-cost-anchor-v2-a12e1373`. Each failed during the identity/export phase and produced no semantic artifact or manifest; no claim is based on them.

## Game-file and retained-capture evidence

The effective installed GUI is `<local-path> of Iron IV\interface\countrypoliticsview.gui`, SHA-256 `811051a83ef087b87be4de9b226fb831f6d6d8f20cd440371619501b5710c183`. It contains the idea category/grid, political-ideas window, selectable entries, and modifier container used by the producer graph.

The English politics localisation is `<local-path> of Iron IV\localisation\english\politics_l_english.yml`, SHA-256 `ea6346f4e10dab0b7bec0fb55f165390bdd9158eb3c7b33207379ba8687b4423`. It pins `REMOVE_IDEA_POLITICAL_POWER`, `IDEA_WINDOW_SWITCH`, `IDEA_WINDOW_CANNOT_SWITCH`, `IDEA_COST_POLITICAL_POWER`, `IDEA_NOT_AVAILABLE_ICON`, `POLITICS_IDEA_STAT_TRADE`, and `POLITICS_IDEA_STAT_RECRUITABLE`.

The XUI11 retained-store receipt covers 214 RDCs and 69,298,844,274 bytes
with zero politics/advisor/minister/national-spirit filename hits. That is an
exhaustive path/filename result, not visual OCR. Those captures cannot replace
resolved trigger results, aggregate terms, evaluation context, or a
completeness witness. They therefore neither close A24 nor establish a present
human-input requirement.

## Production carrier and fold

Reserved unscheduled domain 73 uses `PTI1` schema v2. Its exact row key is `{frame_hour, country_tag, target_kind, target_token_or_slot, line_kind}`. It carries measured/not-applicable/unobtainable availability, exact raw integer plus scale, source identity, evaluation context, a producer-manifest SHA-256, the selected-Present candidate/ordinal/QPC/cutoff/receipt SHA-256, completeness-record identity, and expected row count.

The six typed families are trigger result/reason tree, effective hire/removal cost, conditional-modifier aggregate with exact source terms, advisor-slot capacity/vacancy/eligibility/disabled reasons, timed dynamic modifier value/source/expiry, and displayed party-popularity change sources. Validation rejects duplicate keys, mismatched typed variants, malformed reason trees, nonmonotonic source ordinals, aggregate sum disagreement, scale disagreement, inconsistent advisor vacancy, invalid expiry order, unauthenticated source admission, and cross-hour snapshot attachment.

The collector exposes `encode_a24_exact_politics_tooltip_frame`.
`recorder_host` folds clean frames to `_r2_politics_tooltip_inputs_live`,
preserves status flags and provenance, distinguishes absent from authenticated
measured-empty, emits null on failure or frame-hour mismatch, and never carries
a prior hour's politics tooltip input. `replay_core` now strictly decodes PTI1,
checks exact hour/authority/manifest/count/key/value/reason/source-term
invariants, recomputes and joins the PTI1 selected-Present receipt against the
production-render receipt, rejects unknown country joins, and projects measured rows to
`PoliticsDetailView::tooltip_inputs`. `game_ui::politics_tooltips` exposes only
exact target-key selection and preserves absent versus unobtained versus
measured-empty. It does not evaluate triggers or infer missing rows.

Prior lane, contract, collector, and recorder test results predate this recovery
and are retained only as historical evidence. They do not validate the new
producer-manifest binding or live taps. The primary agent must run the central
validation battery after the shared Wave5 edit set stabilizes.

## Live source disposition and activation

`src/a729_hook.rs` installs 23 sentinel-gated diagnostic detours: the politics entry
refresh, all nine exact UI producers, the three refresh/binding owners, the
five internal value/text producers, the availability/cost/affordability/remove
helpers, and the nested raw modifier-value lookup. Entry and return are paired
on a thread-local native stack with monotonically allocated invocation IDs.
Only nested helper calls under an A24 root are retained. Durable observations
contain module RVAs, fixed-point values, predicates, copied engine strings,
QPC entry/return times, frame hour, and only statically proven pointer-free
engine identities; an unproven object path remains `null` rather than probing
plausible offsets. Raw process pointers are never serialized.

If the isolated legacy diagnostic workflow is explicitly opted into, an
evidence epoch must be armed before UI choreography. Sealing rejects an
hour mismatch, any observation outside the arm-to-selected-Present interval,
an unknown/non-image stack RVA, missing parent invocation, duplicate sequence
or invocation, incomplete tap census, active callback, queue eviction, or any
producer/scheduler diagnostic. Teardown disables all detours, waits for callback
quiescence, removes every trampoline, and clears process-local state before the
collector DLL may unload. The collector exposes install/arm/receipt-copy/
uninstall harness ABI functions, an installed-source PTI1 encoder, and the
one-argument `battle-plan_a24_capture_command_json` remote-owner ABI. That ABI
drains the production-render selected-Present inside the collector and accepts
no Present payload from the caller. The legacy clean-launch owner is
`extractor/native/scripts/Invoke-A24PoliticsTooltipEvidence.ps1`; it authenticates
the frozen D04 save, centrally attested recorder/DLL, loaded module, paused raw
hour, country handle, window ownership, and 1920x1080 client before delegating
the six create-new cases to `capture_a24_autonomous_scenes.py`. The frozen
profile `A24_AUTONOMOUS_SCENES_1920X1080.json` drives physical click, wheel, and
hover sweeps without prompts, advances exactly one hour between cases, and
repause-checks every boundary. `verify_a24_autonomous_admission.py` joins the
six receipt hashes, exact 23-tap union, consecutive hours, regrade result,
executable, and central Release collector attestation into one immutable
diagnostic receipt. A tap-only receipt explicitly does not authorize semantic
source admission, cannot schedule d73, and cannot change its activation latch.
The scene driver is isolated behind the explicit process-local
`BATTLE_PLAN_CLIENT_DRIVING_OPT_IN=I_ACKNOWLEDGE_THIS_TOOL_WILL_CONTROL_HOI4` gate and
is not part of production completion or validation.

The exact UI producer manifest is
`18bac19dc5de7fc0ef90a9341e0bc5ed1ac73c745a1dc96f6177ed08c8f70b35`.
The internal-value job
`20260720T134130Z-w5-a24-internal-value-producers-722157a2` has manifest
`ea5a7659b80b58cfc3f7e41d518da67b1d9114d9f966a66fe7c069a463f79eb1`;
the raw modifier-term closure, slot census, and timed value/expiry jobs have
manifests `a6c5582e6073445b2d1d9402f757ce8b5ae463862ab96bf2e44d34cda58b5a69`,
`23e9625c8f49e2aaeeeb96ed1f57f6e1df081e83b86f34c680b2d4c5b0d0c0d0`,
and `7c254928e9c7571dc11a2d9369c6f053c9b6159d8c29756f8b9acbf27b7a7562`.

Production admission remains explicitly disabled by
`LIVE_PRODUCER_TAPS_COMPLETE=false`; `is_exact_a729_politics_source` remains
false outside tests and d73 remains reserved/unschedulable. This is an
implementation boundary: `begin_exact_politics_frame`,
`submit_exact_politics_batch`, and `seal_exact_politics_frame` have no production
callers. The current detours retain final localized strings and selected scalar
values, but they do not produce the schema's ordered reason nodes, exact
modifier/popularity source-term lists, or a complete target roster including
measured-empty targets. A selected-Present tap matrix cannot reconstruct those
typed values and therefore cannot justify flipping the latch.

## Final exhaustive static producer closure (2026-07-22)

The nominated pre-localisation roots and every immediate exported caller were
re-exported at depth two from the current exact-a729 broker. All three jobs ran
read-only on `shard-07` against image base `0x140000000` and executable SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`:

| Scope | Job | Manifest SHA-256 |
|---|---|---|
| availability/removal trigger roots and callers | `20260722T071342Z-final-a24-trigger-reason-producer-consum-860727f3` | `378eba48ba1bffaec27c4c0f5bbac8be3a92f5f52031eb8965eaa8b7f36b2768` |
| country gate and party-change source arithmetic | `20260722T071400Z-final-a24-party-source-term-producer-con-c8fdacbc` | `b595f4b0ce20df02e2fa7fb2e39653cbfbad4e31ddcfa7bcd14887b3044ca9e4` |
| modifier-tree and slot-roster roots and callers | `20260722T071409Z-final-a24-modifier-roster-producer-consu-825a8c29` | `a3b42976692f4f583ea8e4cd620020e4bfd83d9aadabaf87843209c80423f715` |

The final mechanical disposition is:

- `0x140FBEB00` and `0x140FBEA50` each construct a trigger evaluation
  context, call the trigger object's virtual evaluator at vtable offset
  `+0x18`, and return one byte. Neither ABI accepts a reason sink, node vector,
  callback, or parent/ordinal output. Their direct command callers consume only
  that byte. The immediate politics UI callers (`0x141C32790`,
  `0x141C33FE0`, `0x141C2E8C0`, and `0x141C302E0`) independently append generic
  or already-localized strings; they do not receive structured trigger nodes.
- `0x140B98A20` looks up modifier `0x22A`, optionally invokes
  `0x14054D0F0`, appends the returned flat string, and returns whether the
  aggregate is zero. Its six direct callers either branch on the byte or append
  the same flat string. There is no structured country-gate reason output at
  this boundary.
- `0x14054D0F0` is the exact recursive modifier-source enumerator. Children are
  at container `+0x18`, count at `+0x24`, and each child contributes its
  container at `child+0x10`. Leaf formatter `0x14054C770` proves the authored
  source label at node `+0x48` and calls `0x140551820` for the raw value. The
  lookup detour now retains `modifier_id`, exact source label when the native
  stack contains the unique post-call RVA `0x0054C93B`, and raw value. Other
  lookup callers remain ID-only because their `+0x48` field is not proven to be
  a string. This closes the pointer-free diagnostic identity path without
  inventing offsets. The enumerator still returns only one concatenated string,
  not a durable typed vector.
- `0x140D05A40` exposes exact final party-change raw value and flat text. Its
  arithmetic combines a base value, modifier `0xFC`, clamping, and the
  `0x140B99EB0` drift-dimension return. Consequently the nested modifier leaves
  are not by themselves the schema's sum-complete party-change term list; the
  function has no typed term-list output ABI.
- `0x141C32A50` is called only by constructor `0x141C2DE30`. It allocates three
  persistent widget vectors at offsets `0x38`, `0x50`, and `0x68` from authored
  category slot definitions and constructs advisor, idea, or MIO entry widgets.
  It is a constructor-time layout roster, not a selected-country, frame-local
  enumeration, and it cannot attest measured-empty runtime targets.

Member hashes for the decisive roots are: `0x140FBEB00`
`065550044d6bc499d88c905e9549c3fd73310352fbe9bf01b4fef289d903aca3`,
`0x140FBEA50`
`144ea54ab4572d0b8797daa2bd2145ca2a77255a39bc9e6929deafad230f5c0d`,
`0x140B98A20`
`202ead3c949711027bb42d7fdbc72e694584cb11b6503ad941d9dee7df14733a`,
`0x140D05A40`
`76df5f30809bd1a6ef7d98b3aec85f39b740343cb9662bbeb18190744bc774db`,
`0x140551820`
`5f73b28680133810cce0615586e254291305e3102f2f0413661b4e286e84ad14`,
`0x14054D0F0`
`d740867206f1f505aa458f89cfdf8ca4c8f0e6a36c48460516d8d2deb502bd7c`,
`0x14054C770`
`cba933262d3acd9219d655826a30bdc17b2120325556a902e2d538d6b2ad2d59`,
`0x141C2DE30`
`390f830706e74edb98101fa62387f33ef4f307ee4f6d729ee0f7bafeaf9ede70`,
and `0x141C32A50`
`1dc9f0a49d5e3e4607e68bbb4b70f7227ca282f0da9ac46a1de43a6171405759`.

There is therefore no further static producer wiring available at the nominated
exact-a729 boundaries. The exact engine owns bools, fixed-point aggregates, and
localized strings there, but not the typed reason-tree, sum-complete source-term,
or frame-complete roster objects required by PTI1 v2. Fabricating those objects
from strings, residual arithmetic, or constructor widgets would violate the
lane's exact-or-fail contract. The production-safe disposition is to keep d73
unscheduled and omit the optional politics-tooltip semantic leaf. No human
input, passive capture, or client-driving workflow can change that static ABI
fact; any later observation is regression validation only.
