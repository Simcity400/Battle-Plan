# XUI14 — intelligence and operative presentation

Status: **PROVEN / PROMOTION-READY REPLAY CONSUMER / FAIL-CLOSED EXTERNAL INPUT GATES**

Authority: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Result

The production replay layer now has one typed, exact-a729 admission boundary for agency state, agency upgrades, operative rosters and assignments, operations, ciphers, four-lane intel levels, intelligence networks, seven per-surface visibility gates, and final controller order. Its typed panel renderer emits all seven surfaces in frozen order, suppresses content for hidden and unavailable panels, preserves both gate and producer reasons for unavailable data, and keeps visible measured-empty panels distinct. The consumer rejects inferred identities, final-pixel order, incomplete joins, invalid fixed values, and unavailable surfaces.

XUI14 is ready to promote as a replay consumer. It does not fabricate producer data that remains absent; every missing field remains an external extractor-owned dependency represented by explicit `Unobtained` data and an `Unavailable` visibility gate.

## Effective game-file evidence

The installed-layer composition reuses the frozen XN17B playset and the XUI01 effective VFS. All five effective intelligence GUI files resolve to vanilla:

- `countryintelledger.gui`: 80 windows, 18 collections;
- `countryintelligenceagencyview.gui`: 40 windows, 8 collections;
- `operationoverview.gui`: 25 windows, 4 collections;
- `operative.gui`: 2 windows, 1 collection;
- `operativeleader.gui`: 13 windows, 3 collections.

The exact combined total is **5 GUI files, 160 windows, and 34 dynamic collections**. The active static definition census parses **24 files with zero failures** and records authored order for 70 agency logo/name records, 21 agency upgrades, 21 operation definitions, 82 operation-phase definitions, and five operation tokens.

Authored order is retained only as static catalog evidence. It is not promoted to live UI order; every measured list depends on the authenticated XUI03/A28 collection-controller trace.

## Retained runtime and save evidence

The retained intelligence save oracle is an exact join/order fixture containing:

- 45 countries with intelligence state and 45 named agencies;
- 56 operatives and zero captured operatives;
- three running and 22 finished operations;
- 37 cryptology targets, 25 actively decrypting;
- six operation slots, all six joined to an operative.

The strategic save parser independently preserves 144 operative-pool entries, 16 entries with a network, and 38 total networks. These are exact saved-state fixtures, not same-tick a729 UI-controller producers.

The current a729 D53 path is reused without overclaiming it. It carries agency raw scalars/capacity, operative identity and numeric lifecycle values, cipher target/amount/decrypting state, and sorted network state topology. The current proof mask does not admit operation detail or four-lane intel-map percentages; the candidate operation owner field remains rejected. The typed replay `IntelView` currently exposes only agency name/created/upgrade progress, operatives, and running-operation count.

The retained capture census contains 214 RDCs totalling 69,298,844,274 bytes and the UI13 corpus contains 13 capture roots with 7,063,660 CPU events. Neither contains an authenticated intelligence controller trace, and no intelligence-named RDC was found. Further ordinary RenderDoc capture or manual pixel annotation cannot prove hidden rows, stable identities, reciprocal assignments, owner-target percentages, or controller order, so no new capture is requested.

Useful incidental output was retained rather than discarded: the same save parse produced `strategic_air.json` with 337 strategic air bases and `raids.json` with 2,404 raid targets. They are outside XUI14 admission but remain in its save-oracle directory for their owning future lanes.

## Production admission laws

`game_ui::intelligence_ui` is publicly exported and accepts only `ExactA729ExtractorController` traces. Its production laws are:

- all seven surfaces—agency, upgrades, operatives, operations, ciphers, intel ledger, and networks—have explicit visible/hidden/unavailable dispositions; every non-visible surface carries a reason;
- every measured list carries an exact `DynamicCollectionTrace`; producer rows equal its included source identities and final order comes only from `ordered_item_ids`;
- operative, operation, and network identities are unique, and operation/operative plus network/operative assignments must join reciprocally in both directions;
- agency current-upgrade identity joins the measured upgrade catalog;
- operation targets, phase bounds, duration, fixed progress, mission targets, cipher progress, operative experience, all four intel levels, network strength/coverage, and sorted-unique network state IDs are validated;
- `Unobtained` is admitted only with an explicit reason and an unavailable surface, while measured-empty remains a distinct authenticated collection state;
- final panel rendering emits agency, upgrades, operatives, operations, ciphers, intel ledger, and networks in frozen surface order; hidden panels expose no rows, and unavailable panels preserve both controller and producer reasons;
- all rejections use the `XUI14_FAIL_CLOSED` prefix.

Focused tests cover exact order, collection/cross-reference drift, reciprocal network drift, visibility completeness, invalid operation targets and fixed operative experience, explicit unobtained state, typed seven-panel rendering, hidden-row suppression, and unavailable-versus-measured-empty behavior. They pass **14/14**.

## External producer dependency boundary

The remaining producer work is intentionally outside this replay-only closure:

- **A10 / operations:** authenticated owner, exact definition and target/location, phase/progress/status, start/duration, assigned operative joins, and lifecycle;
- **A11 / intelligence and networks:** same-tick owner-target civilian/army/navy/air intel levels plus authenticated network owner, operative joins, strength, and national coverage;
- **A28 / controller order:** the exact 20-field population/filter/order/group/materialization/clip/scroll trace for every measured XUI14 collection;
- **A30 / intelligence and operative frame:** complete agency semantics, upgrade/build state, operative identity/presentation/traits/nationalities/assignment/capture/recruitable state, cipher display denominator and flags, all seven visibility gates, and the joins to A10/A11/A28.

The replay verifier does not read extractor sources or dossiers. Missing producer families are never filled from saves, static definitions, hashes, localized names, pixels, draw order, or renderer-side script evaluation.

## Binary provenance

No new executable claim or Ghidra operation was made for XUI14. Therefore the new function address, job ID, Ghidra artifact path, and Ghidra manifest hash are all **N/A**. Reused D53 binary findings remain owned by their existing dossier and are not relabelled here.

## Mechanical freeze

The deterministic replay-only evidence builder reproduces the static census, retained runtime inventory, and promotion contract without reading extractor sources or dossiers. Two consecutive manifest-aware verifier runs pass **38/38** with byte-identical verification output; each verifier run deletes its isolated Cargo target.

- static census: `5146a1ea46b0f326e71898023178415c3dd488624c1b4cdfea8344aef5ab1c8f`;
- retained runtime evidence: `36abc5d09685ab4c349712193b7e655392df19b510068b44c41a25abba3c1c59`;
- promotion contract: `b621d00fd15cd1344dc0bb1caf22cc2b43cf24da881b41513f5b157ef62321cd`;
- verification, both runs: `77f188da2cd594829731157b8a095ca028a8b2a7e16af190cacda53465773e1b`;
- production source: `e43cef2e0113c29d7d6f91b54649551a9b25fce93e2cc63b6e21ff6c89a6cd34`;
- focused tests: `dc5307e7e16247e1927f9a739dcf937b516bec84f72ef53e60b65a5fe88bf935`;
- evidence manifest: rebuilt after this report is frozen; its own hash is intentionally not self-embedded.

No `PROGRAM_STATUS.md` edit, adversarial review, new Ghidra operation, or new capture was performed.
