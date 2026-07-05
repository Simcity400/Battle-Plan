# Master Replay Extraction Contract

The active top-level checklist for deciding whether the recording layer is done for a full 1:1
Battle Plan. This is the **only** active completion ledger; older plans/notes/packets are evidence
only under `extractor/documents/archive/`.

**Where things live (single-source, don't duplicate here):**
- Per-domain IDs / status / schedulability → `extractor/native/domain_registry.json` (code-authoritative mirror `compact_contract::ASSIGNED_COMPACT_DOMAINS`).
- Exhaustive per-datum census (every save path, per-country schema, memory-only/UI state) → `extractor/documents/EXTRACTION_ATLAS.md`.
- **What is cracked but not yet wired** → `extractor/native/runs/crack_fanout/CRACKED_LEDGER.md`.
- **What is NOT yet cracked** → the OPEN ITEMS REGISTRY below (the single working backlog).

This contract holds the normative rules (completion, gate, done-means), the open backlog, and a
coverage map. If every open item is closed or dispositioned, the extractor has no known missed
replay-relevant data.

## Current state (2026-07-05)

Recent commits land in git; the CRACKED_LEDGER tracks wiring-pending slices. Highlights this session:
E5 land-combat attribution (domain-55 v3, live-validated 202/202) and D23 map_detail speed (~3x on
the hot loop) COMMITTED; a 10-agent discovery fan-out cracked ~25 more fields/joins (pending wiring
packets W1–W5) and promotes RESERVED **domain 59** (division→template_id join). Earlier this session:
domains **61** (air_loss_stats) + **62** (hub_motorization) scheduled; D43/D53/D56 trailers (MTRF /
DIAQ / INGR / NSUP), E12 naval OOB fix, E4/D20 division redeploy+manpower+losses — all committed and
melt-validated. Do **not** call the fan-out "complete": a real fraction came back UNRESOLVED / PARTIAL /
fixture-blocked — enumerated below.

**Remaining engineering acceptance gate:** a fresh speed-5 end-to-end capture proving every scheduled
domain emits at <100ms/frame with zero skipped hours (all validation to date is against the PAUSED
D04 1944.2.11.18 fixture).

## OPEN ITEMS REGISTRY — NOT YET CRACKED (single source of truth)

The one place to work through everything still uncracked. Cracked-but-not-wired lives in
`CRACKED_LEDGER.md`, not here. Check an item off only when cracked+validated (move it to the ledger)
or consciously dispositioned. Categories: **A** needs a user fixture; **B** needs another RE pass (no
fixture, but hard); **C** derived → replay layer, not a DLL crack; **D** committed-code bug to fix;
**E** deprioritized/low-value tail.

### A. FIXTURE-BLOCKED — needs a live/save game session (cannot crack from paused D04)
- [ ] A1. Hub motorization LEVEL/settings — toggle ≥2 hubs to distinct levels (d62 node membership done; per-level value needs variance)
- [ ] A2. Focus-paused bit — pause one national focus (d45; D04 has no paused variance)
- [ ] A3. Game-speed enum root — 10s speed tap at each speed 1–5 (d57, task #22)
- [ ] A4. Air combat lifecycle: sortie lifecycle / kill attribution / last_killer (d42)
- [ ] A5. Ace kill_type / wounded variance (d46)
- [ ] A6. Convoy/port disrupted-supply PRESSURE (nonzero) — blockaded island / cut pocket (d44)
- [ ] A7. Convoy combat loss route/context — active convoy battle (d44)
- [ ] A8. Naval mission enum / home base / repair / retreat / route semantics (d56)
- [ ] A9. Naval combat lifecycle-end + member join/leave/sinking — battle teardown (d49-50)
- [ ] A10. Running-operation owner/target contrast (operative loc ≠ target) (d53)
- [ ] A11. Spy-network / intel-map percentages, same-tick (d53)
- [ ] A12. Decision numeric state/province targets + cooldowns (d45)
- [ ] A13. Scripted GUI open/state (d51) — or a proof-backed non-replay rejection
- [ ] A14. Construction repair semantics/speed — damaged building repairing (d48)
- [ ] A15. MP chat/messages, hotjoin, tag-switch — 2-player same-build MP (d57)
- [ ] A16. Subject autonomy CHANGE dynamics (static levels DONE by D4; change needs a fixture) (d53)
- [ ] A17. Nukes leaf offset — a country with ≥1 built nuke (D1 container `CCountry+0x1310→CNuke` ready) (d20)
- [ ] A18. Dynamic_modifier per-instance value/expiry — a country carrying a timed modifier (D2 id-set done) (d20/d53)
- [ ] A19. Delayed-event `+0xC4` decrement-over-time + originator-vs-scope-country separation (D6 static done) (d51)
- [ ] A20. Weather region-phenomenon-flags drift confirmation (rain_heavy/arctic_water 305/305 now) — unpaused drift (d24)
- [ ] A21. Finest per-entity combat killer label (per-convoy / per-sunk-event / per-wing) — one controlled combat (combat)

### B. DEEP-CRACK FOLLOW-UP — no fixture, but needs another RE pass (genuinely uncracked)
- [ ] B1. Market stockpile OWNER chain — crack the global `CInternationalMarket` singleton + per-country keying (D7 proved it is NOT off CCountry; task #28)
- [ ] B2. Convoys available (owned count) — walk the equipment-stockpile map (equipment type 70) and sum amounts (D1 UNRESOLVED)
- [ ] B3. Garrison equipment id/type/amount — deeper equipment-bank (D5: 0 hits depth-4 BFS; oracle saved)
- [ ] B4. sunk_convoys_history container-header stride — finish the container walk (D8: element decode 441/441 ready, header PARTIAL)
- [ ] B5. War threat / instigator — read off the capitulation lane's already-discovered war-relation row addresses (D4; no new scan needed)
- [ ] B6. Naval anti_air — member→CShip AA aggregate (E3: matches nowhere as a direct side field)
- [ ] B7. raids per-raid decode — `CCountryRaidStatus` internal layout (D8: container `RVA 0x29B6198` proven, per-raid UNRESOLVED)
- [ ] B8. Construction building token/name — resolve numeric type id → token via game-file map (D5 PARTIAL; may be replay-derived)
- [ ] B9. Subject ratio — `CSubjectRelation+0x60` has no melt oracle to validate against (D4 UNVALIDATED)
- [ ] B10. `CProgram` owner-tag offset — to attribute the 5 aliased special-project facilities that d65 currently fail-closed drops (W3: production has no oracle to disambiguate shared `CProgramStatus` facility vectors; d65 emits 21/26 single-owner). Needs live D04.

### C. DERIVED / RECOMPUTE — replay layer, NOT a DLL crack (track the replay-side work)
- [ ] C1. World tension / average_major_ic / tension_scaling_base_country — derived decayed sum of the threat event list (D3)
- [ ] C2. Realized province temperature/water/snow — derived/non-resident (D6)
- [ ] C3. Outgoing diplomatic queue — transpose of incoming (no distinct structure; D4)
- [ ] C4. Market sibling pools produced/imported/used/to_export/base_export — derive from states + trade routes (MTRF)
- [ ] C5. Country factory/resource/consumer-goods UI counters — RESERVED d58; derive from state buildings/offmap + inputs
- [ ] C6. Alerts/notifications/outliner/message UI — derive alert-equivalents from replay facts (not pollable without hooks)

### D. COMMITTED-CODE FIXES — bugs found by discovery, apply during wiring
- [ ] D1. d53 subject-relation reads 0 subjects on 870ebf — committed inline-vtable scan misses; add the `CRelationStatus+0x288` pointer hop → 38/38 (found by D4)
- [ ] D2. Naval positioning read emits a bogus constant 1.0 (`f32@NavalCombatSide+0x288`) — make it fail-closed / ABSENT (found by E3)

### E. DEPRIORITIZED / LOW-VALUE TAIL — disposition consciously (don't silently drop)
Save-backed but judged low replay value or redundant; each needs a one-line decision (crack-if-cheap /
redundant / derived / skip), not silent omission: ai/ai_strategy (skip), buildings per-country
(redundant w/ d27), cached_navy_strength (derive from OOB), id counters (theatre/navy/land_combat/
industry_org/instances/id_counter_store/debug_current_ref_id — skip), history/history_queue, tokens,
collaboration, claims, cores, deployment, reinforcement, upgrades, name_group, external_rules,
focus_tree/continuous_focus, preferred_tactic, is_top_ic/major/capital/original_capital (session/AI
context — decide per item).

## Completion Rule

A gameplay concept is not complete just because one compact domain passes. A row is **closed** only
when every replay-relevant field is either (1) **captured** through the production path
(`hoi4.exe → one injected DLL collector → typed compact records → Rust recorder → .battleplan`),
(2) **rejected with proof** (exact candidates tested, not to be retried without new evidence), or
(3) **blocked with proof** (an exact fixture/tooling/root wall documented, no unblocked work remains).
Python/saves/heap-scans support research and validation only — they never close a production row.

**Status legend** (full per-domain status is in `domain_registry.json`):
`PRODUCTION_SLICE_DONE` (passed the DLL/compact/Rust/.battleplan gate at hourly cadence, zero skipped
hours) · `PRODUCTION_COMPLETE` (infra domain accepted) · `SHARED_INTEGRATION_PENDING` (lane proof
exists; not yet in the shared collector/recorder) · `VALIDATED_SLICE_OPEN_GAPS` (schedulable for named
fields; others open) · `FIXTURE_BLOCKED` (needs a live/save fixture) · `ROOT_DISCOVERY_OPEN` (matters,
but no production-safe root pinned) · `REJECTED_NO_RETRY` (candidate conclusively rejected) ·
`CONCEPT_COMPLETE` (whole row captured/rejected/blocked with proof — the only "done").

## Production Gate (required for every captured row)

DLL-only capture in the hot path · compact binary records (not JSON frames) · Rust recorder decodes ·
`.battleplan` verifies · hourly-or-better cadence at speed-5/current-equivalent · zero skipped game
hours · no untolerated source-unavailable/partial/worker-panic/bridge failure flags · proven under the
recorder `--production` profile (not an ad-hoc `--scheduled-domain` subset) · fail-closed on invalid
layouts.

**Expected-partial policy:** the gate stays fail-closed. The only tolerated partials are the
domain/flag pairs in `compact_contract::COMPACT_EXPECTED_PARTIAL_TOLERANCES`, absorbing
`COMPACT_STATUS_PARTIAL_DATA` only for the named fixture-blocked sub-scopes — never source-unavailable,
control-invalid, or worker/bridge bits. The recorder manifest surfaces each applied tolerance
(domain, flags, reason, frame count).

## Domain status — open / not-yet-production-complete

Full list + status: `domain_registry.json`. Everything below `PRODUCTION_SLICE_DONE` (the rest are
production-proven or existing compact coverage):

| Domain | Status | What remains (→ registry ref) |
| --- | --- | --- |
| `supply_logistics_attrition_live` / 44 | SHARED_INTEGRATION_PENDING | advancing speed-5 hourly proof; exact province supply; hub level (A1); convoy/disrupted pressure (A6-7); attrition reason |
| `characters_leaders_cabinet_live` / 47 | SHARED_INTEGRATION_PENDING | advisor roster/appointee/portrait/unit-leader-trait rows blocked pending current-build `CAdvisor`/`CUnitLeaderTrait` revalidation; speed-5 proof |
| `construction_buildings_state_damage_live` / 48 | SHARED_INTEGRATION_PENDING | created_date (D5 cracked→ledger); token/name (B8); exact target location; repair (A14); speed-5 proof |
| `naval_combat_lifecycle_live` / 49 | SHARED_INTEGRATION_PENDING | serial-integrate into shared collector; lifecycle-end fixture (A9); speed-5 proof |
| `naval_combat_members_live` / 50 | SHARED_INTEGRATION_PENDING | serial-integrate; member-change/sinking fixture (A9); speed-5 proof |
| `diplomacy_intel_operatives_live` / 53 | VALIDATED_SLICE_OPEN_GAPS | factions/subjects/queues (D4 cracked→ledger + fix D1); war lifecycle (B5); operations (A10); intel-map (A11) |
| `army_battleplans_depth_live` / 54 | SHARED_INTEGRATION_PENDING | speed-5 proof; activation/execution, stance, redraw lifecycle, non-path geometry |
| `land_combat_detail_events_live` / 55 | SHARED (E5 v3 committed) | speed-5 proof; tactics/reserves/width/casualty/terrain fields still open |
| `naval_missions_status_live` / 56 | SHARED_INTEGRATION_PENDING | mission/homebase/repair/retreat (A8); positioning fix (D2); anti_air (B6); speed-5 proof |
| `session_multiplayer_metadata_live` / 57 | VALIDATED_PARTIAL_SLICE_R4 | game-speed root (A3); hotjoin/tag-switch + chat (A15); player-mapping (D3 cracked→ledger) |
| `country_economy_counters_live` / 58 | ROOT_DISCOVERY_OPEN | factory/resource counters are recomputed, not stored → DERIVE in replay (C5) or before/after delta fixture |
| `templates_equipment_catalog_live` / 59 | SHARED (join cracked) | static catalog packed; division→template_id live join cracked by D7 (2359/2359)→ledger; production acceptance awaits wiring + speed-5 |

## Gameplay coverage map (14 areas)

Base slices are `PRODUCTION_SLICE_DONE` unless noted; per-field detail is in `EXTRACTION_ATLAS.md`,
open items in the registry above.

| # | Area | Coverage | Open (registry refs) |
| --- | --- | --- | --- |
| 1 | Session / clock / stream / container | DONE (infra PRODUCTION_COMPLETE) | game-speed root A3 |
| 2 | Static definitions & enrichment | DONE (loc/tech/focus/decisions/ships packed) | GFX/assets (unaudited, UI-owned); character defs (d47) |
| 3 | Country core & economy | DONE base (fuel/stockpile/production/market slice) | market owner B1; factory/resource counters C4/C5; licensing; MTLL |
| 4 | Politics / leaders / cabinet | DONE base | advisors/cabinet/characters (d47 revalidation) |
| 5 | Focus / research / decisions | DONE base | focus-paused A2; decision targets/cooldowns A12; direct hooks |
| 6 | Special projects / scientists | DONE base (+ D6 owner-join/reward cracked→ledger) | project lifecycle events; breakthrough |
| 7 | Divisions / army / OOB / land combat | DONE base (+E5 attribution committed; D20 manpower/losses) | battleplan lifecycle (d54); land-combat detail fields (d55) |
| 8 | Navy | DONE base | naval missions A8; combat lifecycle-end/members A9; convoy losses A7; positioning fix D2 / anti_air B6 |
| 9 | Air | DONE base | sortie/combat lifecycle A4; kill attribution A4/A21; ace death A5; reinforcement reason |
| 10 | Map / weather / construction / occupation | DONE base (+D5 created_date/occupation-law/garrison cracked→ledger) | construction repair A14; garrison equipment B3; weather internals A20/C2; terrain (UI) |
| 11 | Supply / logistics / attrition | DONE base | exact province supply; hub level A1; convoy/disrupted pressure A6; attrition reason |
| 12 | Diplomacy / wars / factions / intel | DONE base + d53 slices (+D4 queues/factions/subjects cracked→ledger) | war lifecycle B5; operations A10; intel-map A11; fix D1 |
| 13 | Scripted state / events / variables | DONE base d51 (+D6 delayed-events cracked→ledger) | scripted GUI A13; on-action hooks; mod-added state |
| 14 | Player / multiplayer / session | PARTIAL (pause + player rows; D3 session header cracked→ledger) | game-speed A3; hotjoin/tag-switch/chat A15; alerts derived C6 |

## Static artifacts packed by recorder

Deterministic JSON artifacts the production recorder packs as `role="static_artifact"` (present/absent
recorded in `index.json`, fail-closed when a flag is omitted):

| artifact | recorder flag | contents |
| --- | --- | --- |
| `templates_equipment_catalog` | `--static-templates-equipment-catalog` | 306 templates / 1180 unit defs / 59 equipment defs (domain 59) |
| `localisation_join` | `--static-localisation-join` | english loc-key → {name,raw,source} map + `resolved_joins` (focus/state/VP/country/cosmetic id→name); 139,950 keys, mod-over-vanilla |
| `idea_tech_loc_xref_static` | `--static-idea-tech-loc-xref` | idea/tech token → loc-key cross-reference (6,522 ideas / 557 techs) |
| `decisions_static_definitions` | `--static-decisions-definitions` | 9,525 decisions / 611 categories with costs/targets/cooldowns/visibility |
| `ship_definitions_static` | `--static-ship-definitions` | 80 hulls / 116 modules |
| `gfx_icon_asset_manifest_static` | `--static-gfx-icon-manifest` | sprite/icon/flag manifest over vanilla+dlc+mod layers with texture hashes |

## Closure sweep (before calling the extractor done)

Run each, update the registry/coverage map: (1) **save-visible parity** — every replay-relevant save
field maps to a compact row or a proof-backed blocker; (2) **UI-visible parity** — walk every ledger/
map-mode/tooltip/combat/production/diplomacy/intel/navy/air/army/project screen; (3) **runtime-event
sweep** — state transitions invisible to hourly snapshots that need sub-hour event rows; (4)
**static-definition sweep** — every emitted token/id has a deterministic loc/def/asset join or an
explicit omitted reason; (5) **fixture backlog** — every fixture-blocked item (registry §A) names its
exact fixture + accepting gate; (6) **rejection ledger** — every `REJECTED_NO_RETRY` candidate listed
with offset/root evidence; (7) **combined speed-5 proof** — schedule every enabled production domain,
require hourly coverage, zero skipped hours, no untolerated failures, Rust decode, `.battleplan` verify.

## Done means

The extraction app is done only when every coverage-map area + registry item is `CONCEPT_COMPLETE`
(captured/rejected/blocked with proof) or explicitly out-of-scope with accepted proof, **and** the
final combined speed-5 proof passes with all enabled production domains, zero skipped game hours, no
untolerated failures, Rust decode, and `.battleplan` verification.
