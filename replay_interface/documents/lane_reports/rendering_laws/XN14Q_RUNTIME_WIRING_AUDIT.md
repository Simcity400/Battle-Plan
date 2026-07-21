# XN14Q - close-zoom weather runtime wiring audit

Status: **NON-HUMAN IMPLEMENTATION WIRED; CENTRAL VALIDATION AND SAME-SESSION LIVE PROOF PENDING**

Pinned binary: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

This audit does not promote a new rendering-law authority. It compares the frozen XN14/XN15 authority with the current extractor, portable carrier, and replay implementation. Claims below inherit only the retained frozen evidence named under **Authority**. The 2026-07-20 implementation update is statically wired but has not yet passed the primary agent's central Rust validation or the required same-session early-launch live observation.

## Result

No close-zoom weather law remains unknown in the retained evidence. The exact producer root, population, placement/order, authored-variant selection, camera-distance input, visibility branches, playback rate, lifecycle join, packet bytes, and final draw route are already frozen. The remaining residual is a production implementation gap:

1. `a729_hook.rs` observes code-change state, PRNG seed words, ordered factory attempts, live primary/fading pairs, child effect IDs, and the frozen named-registry lookup that binds an entity definition pointer to its case-sensitive registration token.
2. `production_render_submit_live::AuthenticatedWeatherCatalog` now builds or decodes a hash-pinned effective-VFS catalog for the exact code table, authored variants, dependency closures, and ordered positions. Resolution is exact-or-fail and implements only the producer's authenticated big-empty-to-small fallback; no other cross-class substitution is allowed.
3. The normal selected-Present collector path now drains the latest complete normal-update membership/checkpoint row per observed region, attaches the latest visibility row for each currently active region, promotes both through the authenticated weather catalog, and joins every child effect to the selected submit's complete ordered CPU output cover.
4. The portable carrier now includes frozen source-line/XYZ placement, pointer-erased variant/content joins, exact entity/effect membership, per-update visibility/write-mask phases, and weather checkpoint hashes for state, complete membership, active lifecycle, visibility, submitted packets, and composition order.
5. Replay admits every weather content winner through the effective-VFS resolver, validates the complete weather carrier and checkpoint fold, then executes only the authoritative particle/entity submissions in captured draw order. It never classifies weather from textures or resimulates the effect.

Production must not represent active state as an unauthenticated empty weather stream. The current hook retains the authenticated manager region count plus a per-region last-seen airborne-code discriminator and requires observed coverage and membership snapshots for every manager region, plus a non-stale visibility row for every active region, at the selected Present. Those rows and their pointer-free digest are bound into candidate selection and the selected receipt; incomplete coverage, missing ownership, or missing catalog/output joins reject the frame.

Wave 2 defines the pointer-free carrier side of the closure: source-line/XYZ position identity; one effective-VFS `map/weatherpositions.txt` identity per region epoch; deduplicated authored-variant entity/content chains; per-update visibility/write-mask records; membership payload hashes; strict output-reference validation; by-region checkpoint folding; and replay-time admission of every referenced content winner. The direct-definition path inherits only the exact-10 authenticated `0x14227e5f0` named-registry receipt; a missing receipt still rejects. Wave 3 now supplies the normal-update ownership, setter observations, Present binding, and selected-submit fold that were previously absent. Promotion status remains closed until central validation and a same-session live receipt prove that implementation.

## Exact-10 static authority admission

The four weather entity-registry/factory roots are statically admitted through
replacement root `SUP-02-D1-P1` only:

- actual job: `20260719T223856Z-wave2-r10-sup-02-d1-p1-ea60087c`
- actual shard: `shard-03`
- normalized request SHA-256: `46efa0aeb27817c3956f8a6b668cd95f4d61ad789389c2fc11aefdeb55b97a46`
- identity-manifest SHA-256: `cd33527ef2f8f5d73ac87272d13b3c6219927272875ffe379ec7f4e1f4f8f8fe`
- exact-10 function-evidence manifest SHA-256: `d50e88d325dc098c7e676de879563e99de6b689d6ab58e5cb094320b1c0903e4`
- job receipt SHA-256: `be93c49db0d0a5d7b44ddfd36bf19af38604a919c4474879447144f58d5a7a9a`
- ten-ready-shard pool-status receipt SHA-256: `df67a663059a645bf8f039f34659fed76274a7259af2fc137a2a60b5e8e41395`
- repository supplement audit SHA-256: `49f4937abb1bddaaa9cdb4ca463549098f6287d4f976240574c09605ea9cfcd7`

The manifest contains exact depth-1/p-code exports for `0x14227e5f0`,
`0x142278710`, `0x142277df0`, and `0x142273fb0`. Their decompile artifact
SHA-256 values are respectively
`e0d1de209dc5e05a802459e62728d55a38ee7802fa416a16a022395b1531e73c`,
`102de8d042162939ea2baa68c5945d428c90ec53cc7d825a0f296936d56e6292`,
`655d63fefd2610ec5fda27f75dea7bc34bb69fd4841f832f37ec9aaf418f3680`,
and `6bc8d1149a2a9ce76da3e24663f57410a4931d50c1a84f912ae10c9c69960963`.
These static receipts authorize the registry-token/definition/factory law only;
they do not authenticate any live weather state, update, or selected submit.

## Exhausted static roots

- Visual producer: `0x14166ec50`; strategic-region visual object owns the producer. The weather record is selected through the strategic-region ID at producer `+0x30`, and the live pair vector is producer `+0x48`, count `+0x54`, stride `0x10`.
- Population: the runtime walks the exact ordered strategic-region weather-position vector. The effective game-file census contains 588 rows. Runtime position stride is `0x10`; class flag is `+0x0c`.
- Placement: each created primary/fading entity receives an identity transform with the ordered position XYZ as translation and unit scale. Reordering or deduplicating authored positions is forbidden.
- Variant selection: the exact flag priority is light rain `2`, heavy rain `3`, snow `4`, blizzard `5`, sandstorm `7`, otherwise `0`. A big position selects the big vector only when that vector is nonempty and otherwise falls back to the small vector; a small position selects only the small vector. An empty selected vector creates no pair and consumes no PRNG draw. Every nonempty selected vector uses the wall-clock-seeded PRNG result modulo the exact vector count.
- Lifecycle: a code change destroys both members of every old pair, clears the vector, and recreates it in authored position order. Primary/fading entity IDs and their ordered child effect IDs are the only semantic join to final packets.
- Camera/LOD input: render/view context `+0x184` is the exact distance scalar. The frozen branch boundaries are `358`, `578`, `1100`, and `1500`; playback is `1` through `300`, linear to `0.15` over `(300,500)`, then `0.15`.
- Final output: wall-clock selection and particle simulation are not replay-deterministic from game date or region state. Exact submitted particle/entity outputs, pipeline/resource state, and final draw order are authoritative and must be carried rather than recomputed.

The retained render anchors were also re-inventoried: `session12_anchor` contains 9 weather-candidate draws, `session12_last_anchor` 74, `vanilla_anchor` 22, and `vanilla_late_anchor` 0. The zero late anchor is a retained negative observation, not evidence that weather output can be omitted. The positive anchors include both exclusive texture/technique pairs and shared/ambiguous atlases, confirming that texture-only classification cannot close the CPU membership join.

## Implemented non-human closure

All of these joins now exist in the normal selected-Present transaction, subject to central build/test validation and the live gate:

1. An effective-VFS-authenticated weather catalog resolver for all nine authored rows, 17 reachable entity/particle definitions, and all 588 ordered positions.
2. A portable position carrier containing source line and XYZ raw bits, plus the existing class, explicit-empty/instantiated result, selected authored index, variant token, entity IDs, and ordered effect IDs.
3. Producer observations on initial/checkpoint state and on changes to every frozen region-state field, not only airborne-code changes.
4. A per-admitted-update visibility record carrying the observed distance bits, exact branch, post-update entity state/write mask, fade bits, and playback-rate bits.
5. A process-local resolution from entity definition pointers to admitted content, followed by an exact join from ordered child effect IDs to the selected submit's particle/entity output references. The hook now records the frozen `0x14227e5f0` named-registry result as `definition_ptr -> token`, the direct factory copies that token into `RawWeatherEntityRef`, promotion rejects `None` or a catalog mismatch, and output joining requires the complete selected-submit particle cover. Process pointers are discarded before serialization.
6. Frame and checkpoint folding that maintains state/membership by region and hashes the actual weather stream, including cold-seek equality.
7. Replay validation of those joins without texture-based weather classification. Shared cloud atlases remain ambiguous without the CPU entity/effect identity.

## Selected-Present discriminator and snapshot

The runtime discriminator is exact and intentionally small:

- `observed_weather_region_count`: count of strategic regions reached at `0x14166ec50` since hook installation.
- `active_airborne_weather_region_count`: count whose last authenticated producer code is one of `2,3,4,5,7`.
- `queued_weather_change_count`: exact initial/code-change/state-checkpoint observations waiting for the compatibility process-local promotion drain.
- `weather_change_overflow_count`: bounded compatibility-history loss. A nonzero value makes the complete transition-history drain fail instead of returning a silently truncated sequence; it does not replace or invalidate the independently sealed per-region selected-Present snapshot.

Code values outside `0,2,3,4,5,7` reject the observation. At selection, `expected_weather_region_count` must be nonzero and equal both `observed_weather_region_count` and the ordered membership snapshot size; `active_airborne_weather_region_count` must equal the active-only visibility snapshot size. The hook recomputes a pointer-free snapshot digest, binds it and the latest completed weather update index into the candidate and selected-receipt domains, and the collector recomputes that receipt before promotion. Queued compatibility observations no longer cause a blanket rejection because they are not substituted for the selected snapshot.

The remaining manager-index ambiguity was also exhausted on the current exact-20 broker. Job `20260720T110300Z-w5-xn14q-weather-manager-index-law-c4fe6fac` ran on `shard-19`; its function-evidence manifest SHA-256 is `ffde1022a89d74b037ecb92aac744c8100c2600946a8427943fc2a10fdd8a60a`, and `func_140f04e70.decompile.c` has SHA-256 `39528b7ea9d33654b791842cd1cff111366398c58d3c1853497ecb7bb9847cc3`. The exact lookup accepts only `0 <= region_id < manager+0x4c` and returns `manager+0x40 + region_id*0x160`. This independently agrees with domain 24's retained 305-row, ID-`0..304` contract, so the hook's coverage count and row-address law have no one-based-ID gap.

That cross-check also closes the zero-pair active-region case. `0x14166ec50` admits a nonzero airborne code independently of whether the strategic region has a position whose selected vector is nonempty; with zero positions or only empty selected vectors it creates no pairs and executes no entity setters. Production now retains the exact completed camera/visibility phase with an empty entity list, while the catalog proves the exact explicit-empty position membership. Such an initial active row can be admitted without a historical PRNG seed only when catalog resolution proves that no draw or factory call was possible; any nonempty selected vector still requires the captured seed and factory/pair join. No placeholder entity is fabricated, no pair-backed setter check is relaxed, and a missing visibility phase remains incomplete.

## Wave 3 setter-chain admission boundary

The exact non-human hook seed is now exhausted and packaged in
`XN14Q/weather_update_setter_chain_promotion_request.json`. One consolidated
depth-1/p-code export must contain `0x14166ec50`, `0x14166cec0`,
`0x14227b200`, `0x142271b40`, and `0x14227b4a0`; its canonical semantic
request SHA-256 is
`d96e1b37a4087b19507cefe96823b8f3438afee91bfe9573870322b2dd22daf8`.
The three setter prologues require the companion dump request whose canonical
SHA-256 is
`792a4380f12fa38f6914750f8973085a8932e9c95e3fbd158edaf6c9084579e7`.
The Wave 3 exact-20 ledger now binds the actual function and data jobs. That
static custody authorizes implementation but does not replace the independent
same-session live activation gate.

The discovery bodies also expose one concrete dormant validator defect. The
producer does not use one common write mask for active and alpha. Fading
active is written in every branch, but fading alpha only when computed alpha
is positive. Primary active is written only in far-fade/far-hidden, and primary
alpha only in far-fade. Playback is written for both roles on every active
update. The extractor and portable validators now enforce independent active
and alpha occurrence masks for all five branches, plus field-specific
continuity when a setter was not called.

The same request pins the implemented transaction work: first-call code-zero
rows, normal-update ownership distinct from editor refresh, setter-scoped
occurrence plus double-read post-state, PresentEnd ownership, and complete CPU
effect-output cover. An empty effect-output subset is admitted only when the
complete authenticated selected-submit packet list proves that exact empty
set. Texture absence and RenderDoc absence remain non-authoritative.

## Human-input boundary

No human interpretation of weather assets or native behavior is required. Static mining, exact-20 receipt binding, and the non-human consumers above are exhausted. The definition-pointer receipt is implemented from exact-10 XN15L authority and still refuses inference from pointer value, factory order, texture, or position. The retained-observation census searched the retained RDC, XN14B anchors/corpus, XN14V fixtures, native custom-harness runs, and current hook surface; none contains the required same-session proof. The census is pinned at `replay_interface/re_evidence/rendering_laws/XN14Q/retained_observation_census.json` (SHA-256 `96243fdbc74ceb910118855e58d44399f70b48f55c35bfb30cf1012c70acdb60`) and summarized by `XN14Q_RETAINED_OBSERVATION_CENSUS.md` (SHA-256 `47914c3316fa29a224b5db4896e391dcd367d5050f23e6e93ff51309218fc7ec`).

Promotion therefore still requires the primary agent's central Rust validation and an authorized early-launch capture proving that the installed implementation sees every needed named-registry lookup, emits the first successful call per region even when unchanged, records exact per-update visibility and setter write masks for every instantiated pair (and an exact empty phase for a zero-pair active row), and owns the latest completed update plus complete effect-output cover in the same selected-Present transaction. The earlier 2026-07-18 registry/factory jobs are retained as discovery history only. No static receipt or unexecuted implementation enables active-weather promotion.

## Authority

- `replay_interface/re_evidence/rendering_laws/XN14A/runtime_contract.json`
- `replay_interface/re_evidence/rendering_laws/XN14A/weather_asset_census.json`
- `replay_interface/re_evidence/rendering_laws/XN14A/ghidra_provenance.json`
- `replay_interface/re_evidence/rendering_laws/XN14C/weather_interface_contract.json`
- `replay_interface/re_evidence/rendering_laws/XN14C/promotion_contract.json`
- `replay_interface/re_evidence/rendering_laws/XN14P/production_handoff.json`
- `replay_interface/re_evidence/rendering_laws/XN14V/fixture_corpus.json`
- `replay_interface/re_evidence/rendering_laws/XN15/recording_contract.json`
- `replay_interface/re_evidence/rendering_laws/XN15L/entity_selector_native_contract.json`
- `extractor/documents/WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json`
- `replay_interface/re_evidence/rendering_laws/XN14Q/exact10_weather_supplement_audit.json`
- `replay_interface/re_evidence/rendering_laws/XN14Q/retained_observation_census.json`
- `replay_interface/re_evidence/rendering_laws/XN14Q/weather_update_setter_chain_promotion_request.json`
- `replay_interface/re_evidence/rendering_laws/XN14Q/automated_selected_present_wiring.json`
- `replay_interface/documents/lane_reports/rendering_laws/XN14Q_RETAINED_OBSERVATION_CENSUS.md`
- `replay_interface/documents/lane_reports/rendering_laws/XN14Q_WEATHER_UPDATE_SETTER_CHAIN_PROMOTION_REQUEST.md`
