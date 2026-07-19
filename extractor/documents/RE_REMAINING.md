# Extractor reverse-engineering promotion ledger

This is the extractor-only master list. It is intentionally separate from the
rendering-law reverse-engineering corpus under `replay_interface/`.

Pinned program: `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

A row is complete only when the exact memory law is recorded, the production
reader is fail-closed, tests pass, and the report records every supporting
function address, Ghidra job ID, artifact path, and manifest hash. `PARTIAL` and
`TOOL_BLOCKED` are not completion states. A capture dependency is retained only
when the existing capture corpus genuinely cannot supply the discriminating data.

| ID | Extractor producer | Exact promotion target | Status |
|---|---|---|---|
| A1 | d62 hub motorization | Inner country-to-level hashmap layout and attribution | **PROMOTION READY** — schema v2 and the joint collector/contract/recorder matrix pass |
| A2 | d45 focus/decisions | Stable focus-paused flag root | **PROMOTION READY** — exact `CNationalFocusProgress+0xB0` pause state is wired and verified |
| A12 | d45 focus/decisions | Decision target state/province, cooldown/days, and re-enable timer roots | **PROMOTION READY** — exact target union and both cooldown vectors are wired and verified |
| A4 | d42 air combat | Sortie/combat lifecycle, pairings, and reinforcement reason | **PROMOTION READY** — exact stored country/region/date/mission history is wired; persistent wing pairing and reinforcement-source enum are proven absent and fail closed |
| A21 | d42 air combat | Killer/cause attribution and convoy/sunk variants | **PROMOTION READY** — typed damage causes plus dedicated sunk-convoy history are wired and pass collector/contract/recorder folds |
| A14 | d48 construction | Repair-cause semantics and target province/location | **PROMOTION READY** — exact line `+0x78` repair counter, building `+0x48` partial health, reason enum 0..5, and typed location pass the joint matrix |
| A22 | d61 air-loss statistics | Exact labels for all 24 display channels | **PROMOTION READY** — schema v2, all 24 field/label/divisor mappings, and the joint collector/contract/recorder matrix pass |
| A15 | d57 session/MP | Pollable message root and source flags for chat/system/hotjoin/tag-switch | **PROMOTION READY** — exact d57 poll root/store and typed chat/system/hotjoin/tag-switch disposition are wired and lane-verified |
| B8/X6 | building-token registry | Exact id-to-token registry mapping | **PROMOTION READY** — shared 55-entry registry is wired and verified across 7,456 rows: 43 observed, 12 independently proven absent, zero ambiguity/mismatch |

## Remaining fixture and deep-root work

These rows are extractor-owned memory/static-definition work. They describe the
authoritative facts that may enter `.battleplan`, not how the replay renderer or
UI will consume or display them.

| ID | Extractor producer | Exact promotion target | Status |
|---|---|---|---|
| A3 | d57 session | Exact raw game-speed mapping, transitions, and pause orthogonality | **FIXTURE REQUIRED** — retain a real speed-1 through speed-5 cycle plus pause/resume evidence |
| A5 | d46 air/aces | Ace kill type, wounded/death state, and lifecycle semantics | **RE + FIXTURE REQUIRED** |
| A6 | d44 supply/logistics | Nonzero convoy/port disrupted-supply pressure and attribution | **RE + FIXTURE REQUIRED** — blockaded-island or cut-pocket fixture |
| A7 | d44 convoy/logistics | Convoy-combat loss route, location, context, and cause | **RE + FIXTURE REQUIRED** — active convoy-battle fixture |
| A8 | d56 naval missions | Mission enum, home base, repair, retreat, route, and lifecycle semantics | **RE + FIXTURE REQUIRED** |
| A9 | d49-d50 naval combat | Combat end plus participant join/leave/sinking lifecycle | **RE + FIXTURE REQUIRED** |
| A10 | d53 intelligence operations | Running-operation owner, exact definition/target, location, phase/progress/status, assigned operative joins, and lifecycle | **DATA BLOCKED — EXACT PRODUCER CONTRACT FROZEN** — current a729 rows prove only definition-valid instance identity plus provisional associated country; the required live operation carrier is specified in `extractor/documents/dossiers/A30_INTELLIGENCE_OPERATIVES_UI_INPUTS.json` |
| A11 | d53 intelligence | Owner-bound spy-network strength/coverage and all four owner-target intel-map percentages with same-tick provenance | **DATA BLOCKED — EXACT PRODUCER CONTRACT FROZEN** — current a729 extraction preserves network topology but no strength/coverage, while intel-map percentages remain resisted; exact requirements are in `extractor/documents/dossiers/A30_INTELLIGENCE_OPERATIVES_UI_INPUTS.json` |
| A13 | d51/d64 scripted state | Authoritative scripted flags, variables, delayed state, and direct state-changing facts | **RE REQUIRED** — host-local panel state is explicitly excluded |
| A16 | d53 subjects | Subject-autonomy transition direction and `CSubjectRelation+0x60` ratio semantics | **RE + FIXTURE REQUIRED** |
| A17 | country core | Built-nuke container leaf and exact count | **RE + FIXTURE REQUIRED** — country with at least one nuke |
| A18 | d67 dynamic modifiers | Per-instance modifier value and expiry | **RE + FIXTURE REQUIRED** — timed-modifier fixture |
| A19 | d64 delayed events | Countdown/decrement, scope, originator, and execution lifecycle | **RE + FIXTURE REQUIRED** |
| A20 | d24 weather | Region phenomenon flags and unpaused transition/drift semantics | **FIXTURE REQUIRED** — static flag agreement is not temporal proof |
| A23 | d65 special projects | `CProgram` owner identity, location join, and exact save-owner equality | **FIXTURE REQUIRED** — candidate offsets require the frozen 26/26 live gate |
| A24 | politics evaluated-tooltip inputs | Per-frame, per-country resolved politics tooltip rows: evaluated trigger result/reason tree; effective hire/removal cost; aggregated conditional modifiers; advisor-slot capacity/vacancy/eligibility/disabled reason; timed/dynamic modifier value/source/expiry; displayed popularity-change sources | **EXTRACTOR CONTRACT REQUIRED** — current replay payload has no equivalent resolved-computation leaf; exact field/provenance contract is frozen in `extractor/documents/dossiers/A24_POLITICS_TOOLTIP_INPUTS.json`; renderer evaluation or static-presence inference is forbidden |
| A25 | runtime localized text inputs | Exact engine-selected scripted-localisation/plural/grammar result; complete formatted substitutions and data-function/dynamic-colour results; runtime wrap/truncation/ellipsis/clip/alignment; fallback-font and inline-icon runs for admitted replay UI draws | **EXTRACTOR CONTRACT REQUIRED** — current replay payload has no generic runtime text/glyph-run carrier; exact contract is frozen in `extractor/documents/dossiers/A25_RUNTIME_TEXT_INPUTS.json`; XUI06 consumes these values but must not synthesize them |
| A26 | resolved tooltip instances | Exact engine-resolved screen rectangle plus ordered, evaluated tooltip rows with source ordinal/identity/context and explicit availability; includes dynamic/scripted breakdown rows | **PROMOTION READY — D72 SCHEDULABLE** (updated 2026-07-19; the prior "carrier reserved/unschedulable" wording was stale) — the 13 authenticated retained CPU streams prove the topbar tooltip producer at `0x1418845C0`, exact formatting sites `0x14188963C`/`0x1418897B4`, and virtual-dispatch chain beginning `0x14162961E`; the generic a729 producer implements `ExactTooltipInstanceSource` (`extractor/native/collector/src/lib.rs`, `extractor/native/lanes/resolved_tooltip_instances_live/`) and d72 is `validated_slice`/schedulable in `domain_registry.json` (dossier status `PROMOTION_READY_EXACT_A729_D72`). Contract: `extractor/documents/dossiers/A26_RESOLVED_TOOLTIP_INSTANCES.json` |
| A27 | live design catalog inputs | Complete live division/ship/tank/aircraft/equipment definitions, modules, displayed stats, upgrade/lifecycle state, and exact displayed order | **PROMOTION READY** - the frozen same-tick a729 producer supplies 5,281 variants and 1,126 templates across 440 countries with exact native definition fields, lifecycle state, pointer/ordinal identity, and authenticated producer order; the independent 12/12 promotion gate passes twice with zero blockers (`extractor/native/lanes/design_catalog_live/proofs/a27_full_reader_fixture/live_promotion_gate.json`) |
| A28 | dynamic collection controller trace | Per-frame/per-transition source population, stable identity, filter disposition, final order/grouping, row-template and materialized-slot binding, clipping/extents, and fixed scroll state for every admitted list/grid/overlap instance | **EQUIPMENT-VARIANT GRID ZERO-GAP PRODUCER PROVEN; REMAINING FAMILIES REQUIRED** — reserved d71 carries all 20 fields and its recovered fold passes 1/1. The equipment-designer fixture now closes source order, stable IDs, every filter outcome, accepted order, row reuse/binding, final grid geometry, and fixed-point scroll synchronization. Unsourced list/grid/smooth-list/overlap instances remain fail-closed pending their own authenticated controller joins and zero-gap fixtures. Exact provenance is in `extractor/documents/dossiers/A28_DYNAMIC_COLLECTION_TRACE.json` and `replay_interface/re_evidence/rendering_laws/XUI03/A28_GHIDRA_HANDOFF.json`. |
| A30 | intelligence/operative UI state | Exact agency semantics, operative roster/assignments/lifecycle, recruitable pool, cipher display state, and per-surface visibility; joins A10 operations, A11 intel/networks, and A28 controller ordering | **DATA BLOCKED — EXTRACTOR CONTRACT REQUIRED** — XUI14's typed consumer is promotion-ready and retained save/a729 slices are reusable, but no current producer supplies the complete authenticated frame contract frozen in `extractor/documents/dossiers/A30_INTELLIGENCE_OPERATIVES_UI_INPUTS.json` |
| A29 | active-research effective ahead-of-time inputs | Per active slot and replay hour: same-tick slot/technology identity, effective ahead years, displayed penalty ratio, applied reduction years, reduced displayed penalty ratio, and explicit availability/provenance | **DATA BLOCKED — EXTRACTOR CONTRACT REQUIRED** — live numerator/base-denominator progress is already carried, but effective modifier aggregation is not; XUI09 rejects every active row lacking the exact carrier frozen in `extractor/documents/dossiers/A29_RESEARCH_AHEAD_INPUTS.json`; deriving it from `BASE_YEAR_AHEAD_PENALTY_FACTOR` alone is forbidden |
| A40 | XUI08 designer and peace-conference snapshots | Exact inert designer selection/order/rows for division, equipment, tank, plane, ship and variant-upgrade views; exact active peace participant/allocation/result graph | **EXTRACTOR CONTRACT REQUIRED** — save/static evidence closes catalogs, definitions, GUI chrome and authenticated inactive peace state; active/selected runtime fields are frozen in `extractor/documents/dossiers/A40_XUI08_READONLY_INPUTS.json`; `game_ui::readonly_design_peace` rejects every inferred or incomplete surface |
| A31 | national-focus presentation inputs | Engine-evaluated available set, final visible order, effective display progress, and runtime glow/marker/ownership visibility | **INTEGRATED EXACT FIELDS — RUNTIME PRODUCER LEAVES REMAIN** — selected tree, completed order/count, measured-empty current/continuous sets, progress days, pause, all 22 exact background-style families, all 93 authored overlays (25 sprites), and fail-closed admission are wired and tested; the genuinely absent leaves are frozen in `extractor/documents/dossiers/A31_NATIONAL_FOCUS_PRESENTATION_INPUTS.json`; trigger evaluation, base-cost progress, and authored-order inference remain forbidden |
| A32 | display/language/input launch variant | Exact physical viewport, `min_gui`, GUI-scale f32 bits, refresh rates, fullscreen/borderless/display mode, settings SHA-256 and localisation language from one stable prelaunch settings read; pointer-family input admission | **PROMOTION READY — INTEGRATED** — recorder_host emits the closed `display_variant`, fold/repack and replay_core validate/inject it, and XUI25 constructs an exact key. Keyboard/controller traversal is explicitly unsupported rather than guessed; A25 remains the owner of runtime-selected fallback glyph/text runs. Evidence and tests are frozen in `extractor/documents/dossiers/A32_DISPLAY_LANGUAGE_INPUT_VARIANTS.json` and `replay_interface/re_evidence/rendering_laws/XUI25/`. |
| A36 | selected-context active command abilities | Per-frame complete ordered rows with identity, availability, activation, cooldown, effective cost, target, duration, icon, resolved tooltip and disabled reason; plus affected-division identities | **DATA BLOCKED - EXTRACTOR CONTRACT REQUIRED** - authored definitions and replay consumers are complete, but saves and retained replay streams do not carry named live activation state; exact fail-closed carrier is frozen in `extractor/documents/dossiers/A36_ACTIVE_ABILITY_INPUTS.json` |
| A39 | XUI17 character/leader/trait-tree/ace presentation residuals | Formation display name when absent from OOB/save; dynamic trait-node state/progress; unresolved portrait fallback; ace modifier semantics and unresolved wing/equipment presentation | **EXTRACTOR CONTRACT REQUIRED** - d47 identity/stats/traits, d28 assignment/reserve, exact skill-cost XP, static character portraits and literal named-ace portraits are replay-wired; A25 owns resolved text and A28 owns visible collection order; only irreducible fallback leaves are frozen in `extractor/documents/dossiers/A39_XUI17_CHARACTER_PRESENTATION_INPUTS.json` |
| A33 | XUI20 combat detail, losses, log, and war overview | Exact land/naval/air result and loss rows; leaders/tactics/progress/modifiers/width; destroyed-entity semantics; global combat-log order; war-side grouping and participant aggregates | **INTEGRATION + DEEP RE + FIXTURE REQUIRED** — current authenticated d55/d49-d50/d42/D60 fields are typed in the replay layer; remaining producer inputs and fail-closed boundaries are frozen in `extractor/documents/dossiers/A33_XUI20_COMBAT_DETAIL_LOSSES.json`; M02's exact five-state human capture remains a separate rendering dependency |
| XUI21-E1 | fired-event history and popup producer | Exact per-firing identity, recipient, dispatch tick/order, visible definition-option ordinals, chosen ordinal, selected desc/picture/subtype, timeout/resolution, A25 text join, and zero-gap completeness | **DEEP RE REQUIRED** — XUI21 replay integration is promotion-ready and rejects legacy save IDs/polling inferences; the exact callback/completeness contract is frozen in `extractor/documents/dossiers/XUI21_FIRED_EVENT_POPUP_INPUTS.json` |
| A34 | XUI19 decision/mission panel state | Complete ordered visible cards with evaluated availability/activation, semantic lifecycle, typed target, effective cost, timers/cooldowns, exact runtime text, and both alert memberships/counts | **DEEP RE + INTEGRATION IN PROGRESS** — D45 v4 now carries both exact targeted families (`+0x100` targeted, `+0xE8` targeted-timed) with source kind, typed target, lifecycle, ignore, and days; cooldown identity+days are also live. Authenticated evidence freezes a 22-row materialization, one exact 100-PP/180-day activation, 5,042 save records, and exact alert-widget bindings. Generic ordered/evaluated rows, effective custom costs/text, and alert memberships/counts remain under `extractor/documents/dossiers/A34_XUI19_DECISION_PANEL_INPUTS.json`; trigger evaluation and authored-order inference are forbidden |
| A35 | d52 occupation/garrison UI completion | Save garrison/equipment/history producer plus global `alert_not_enough_garrison` aggregate shortage law | **PROVEN/PROMOTION-READY** - exact equipment, reinforcement and 3,974-row resistance history carriers are parsed, joined, typed and tested; Ghidra exports prove slot `0x3f`, false-on-shortage membership, positive aggregate manpower deficit, positive equipment children after exact `/100000`, and instant/detail localization paths; A28 order and A25 text remain shared producers |
| A37 | XUI18 power-balance resolved runtime state | Exact active side/range, effective side sprites, stable/moving bar state, range-indicator frame/position, change cadence/magnitude, active modifier order, BOP decision subset/order, and multi-balance display ordinal | **EXTRACTOR CONTRACT REQUIRED** — existing `world.power_balances[]` and static `common/bop` definitions remain reusable, but shared range boundaries, scripted/runtime sprite changes, range-indicator anchor, trend magnitude, dynamic modifiers, and decision-grid state are not reconstructible; exact fail-closed fields are frozen in `extractor/documents/dossiers/A37_XUI18_POWER_BALANCE_INPUTS.json`; A25/A28/XUI19 remain the owners of resolved text, collection materialization, and general decision state |
| A38 | generic scripted-GUI engine residuals | Exact per-country, per-effective-GUI final results for engine-only visibility/enabled predicates, named triggers, dynamic properties, and effect-enabled gates | **EXTRACTOR CONTRACT REQUIRED** - d51 already carries country/world variables and named flags, and XUI22 evaluates their supported variable/flag trigger/effect subset generically. Arbitrary engine scopes/functions are irreducible renderer-side; the authenticated, mod-extensible residual carrier is frozen in `extractor/documents/dossiers/A38_SCRIPTED_GUI_EVALUATION_INPUTS.json`. Missing results remain unavailable; per-panel schemas and inferred defaults are forbidden. |
| B1 | d43 market | Per-country market-stockpile owner plus authoritative nested serializer/iterator | **DEEP RE REQUIRED** |
| B2 | country convoys | Convoys-owned total, authoritative inner iterator, and admitted static-variant join | **PROMOTION READY** (updated 2026-07-19; prior DEEP RE REQUIRED was stale) — exact `CConvoys` at `CCountry+0x1200` producer, pool iterator/stride laws, owned/assigned/available semantics, static `convoy_1` join, and production-stockpile negative control are frozen under `extractor/re_evidence/B2_COUNTRY_CONVOYS/` (2026-07-17 Ghidra jobs; `verification.json` result PASS, 8/8 checks) |
| B3 | d52 occupation/garrison | Garrison equipment id, type, amount, and completeness | **PROVEN/INTEGRATED BY A35** - save parser, `(id,type)` design join, measured-empty completeness, typed replay carrier and direct XUI16 admission are tested |
| B4 | global combat history | `sunk_convoys_history` live root/iterator equality and production integration | **EVIDENCE** — static law closed; exact paused acceptance remains |
| B5 | d60 wars | Threat, instigator, hostility reasons, typed bounds, and all-row equality | **DEEP RE + FIXTURE REQUIRED** |
| B6 | d49 naval combat | Exact side anti-air raw decoding and side-root join | **EVIDENCE** — serializer member known; live eight-side equality remains |
| B7 | raids | Exact raid child population, identity, ordering, and dummy classification | **EVIDENCE** — roots/iterators closed; live acceptance/integration remains |

## Remaining coverage-sweep RE

| ID | Extractor producer | Exact promotion target | Status |
|---|---|---|---|
| X1a | d27/d54 orders | Carry stable order-instance identity, correct membership meaning, and omission telemetry through production rows/folds | **INTEGRATION REQUIRED** — no new root RE claimed |
| X1b | d54 orders | Empty/inactive order groups, activation/execution, stance/aggressiveness, and geometry beyond validated target/path snapshots | **DEEP RE REQUIRED** |
| X1c | d54 orders | Stable-identity redraw/resize/delete and assignment add/remove/tombstone lifecycle | **FIXTURE REQUIRED** |
| X2a | d55 land combat | Integrate exact tactic identity/name and side-progress fields | **INTEGRATION REQUIRED** — cracked fields must retain exact provenance |
| X2b | d55 land combat | Manpower/equipment losses and reserve-division identities | **DEEP RE REQUIRED** |
| X2c | d55 land combat | All-zero losses plus retreat, overrun, outcome, and ended-combat transitions | **FIXTURE REQUIRED** |
| X2d | d55 land combat | Exact combat-width law or evidence-backed rejection; terrain remains derived | **RE/DISPOSITION REQUIRED** |
| X3 | d43 market | Licensing and military-trade/licence relation joins | **DEEP RE REQUIRED** |
| X4 | production/deployment/MIO | D11 line variant id, requested factories, typed subtype, efficiency state, MIO assignment, zero-vector completeness; plus deployment queues | **X4-P1..P6 PROMOTION READY; DEPLOYMENT QUEUES REMAIN** — strict `PVJ1`, `PFR1`, `PLS1`, `PEF1`, `PMI1`, and `PCP1` contracts are frozen; authority: `extractor/native/lanes/country_economy_production_stockpile_live/X4_PRODUCTION_INPUT_CONTRACT.json` |
| X5 | d48 construction | Exact repair/location semantics | **COVERED BY A14** — promotion-ready; do not duplicate RE |
| X6 | static/live registries | Building and equipment-variant deterministic live-id joins | **COVERED BY B8/X6** — promotion-ready; do not duplicate RE |
| X8 | d27/d54 order membership | Complete `(order_instance_id, division_id)` child table for divisions assigned to multiple orders | **DEEP RE + FIXTURE REQUIRED** |
| X9 | d41/d65 special projects | Start/completion transitions, owner, fired reward history/options, and breakthrough semantic-name joins | **RE + FIXTURE REQUIRED** — joins A23 |
| X10 | decisions/events/on-actions | Direct hooks and mod-added authoritative state that hourly polling cannot preserve | **DEEP RE REQUIRED** |
| X11 | d26/d27/d28/d54 completeness | Narrow expected-partial scopes and prove sparse-fold identity or mark the domain/hour unknown | **RECONCILIATION REQUIRED** |
| X12 | session/timeline | Day progress and sub-hour tick/event timing required for transition fidelity | **RE/DISPOSITION REQUIRED** |
| X15 | d43 classic resource trade | Integrate the already-proven six `CCountryResources` pools; add exact ordered display rows, partner deals, factory/convoy costs, route efficiency, and zero-row completeness | **INTEGRATION + DEEP ROOT REQUIRED** - exact X15-P1..P5 producer contract: `extractor/documents/dossiers/X15_RESOURCE_TRADE_INPUTS.json`; resource-pool root RE is complete and must not be repeated |
| P3 | session identity | Stable campaign seed/id across save/load, relaunch, copy/rename, campaign changes, multiplayer, observer, and rehost | **FIXTURE MATRIX REQUIRED** — candidate fields exist; production admission awaits the complete identity matrix |

## Extractor coverage-tail dispositions

These save-backed families are not automatically replay-file fields. Each must
be mechanically proven redundant, exactly derived, static-derived, irrelevant,
or uniquely replay-relevant. A uniquely replay-relevant result becomes a new RE
row in this ledger.

| ID | Extractor producer | Exact promotion target | Status |
|---|---|---|---|
| E12 | `history` | Prove whether it contains rows not covered by event/stat domains | **DISPOSITION REQUIRED** |
| E13 | `history_queue` | Prove whether it contains unique authoritative transitions | **DISPOSITION REQUIRED** |
| E14 | `tokens` | Enumerate token families and bind each to live/static coverage or rejection | **DISPOSITION REQUIRED** |
| E15 | `collaboration` | Classify observable occupation/diplomacy semantics or reject | **DISPOSITION REQUIRED** |
| E16 | `claims` | Reconcile against authoritative map/country claim coverage | **DISPOSITION REQUIRED** |
| E17 | `cores` | Reconcile against authoritative map/country core coverage | **DISPOSITION REQUIRED** |
| E18 | `deployment` | Reconcile against production/deployment-queue coverage | **DISPOSITION REQUIRED** |
| E19 | `reinforcement` | Prove unique reason/state beyond division-equipment deltas | **DISPOSITION REQUIRED** |
| E20 | `upgrades` | Reconcile against equipment/variant upgrade domains | **DISPOSITION REQUIRED** |
| E21 | `name_group` | Prove a static-definition join or evidence-backed omission | **DISPOSITION REQUIRED** |
| E22 | `external_rules` | Identify authoritative user-visible semantics or reject as engine configuration | **DISPOSITION REQUIRED** |
| E23 | `focus_tree` | Reconcile static tree identity against live focus rows | **DISPOSITION REQUIRED** |
| E24 | `continuous_focus` | Prove whether current selection/state is already captured | **DISPOSITION REQUIRED** |
| E25 | `preferred_tactic` | Determine whether it is authoritative visible player intent | **DISPOSITION REQUIRED** |
| E26 | `is_top_ic` | Prove derived/session-context treatment or capture unique state | **DISPOSITION REQUIRED** |
| E27 | `major` | Reconcile current dynamic-major status with country/session rows | **DISPOSITION REQUIRED** |
| E28 | `capital` | Reconcile against authoritative country/state ownership and capital changes | **DISPOSITION REQUIRED** |
| E29 | `original_capital` | Prove whether unique historical state must be captured | **DISPOSITION REQUIRED** |

## Explicitly excluded from extractor RE and `.battleplan`

- selected objects;
- open screens, panels, and dialogs;
- active map mode and display toggles;
- camera state;
- hover targets and tooltips;
- outliner expansion, filtering, and scrolling;
- UI scroll positions, drag state, and local preferences.

The replay viewer owns those presentation states. Their graphical and behavioral
parity is tracked only in the replay-layer rendering/UI program and is not mixed
into this extractor ledger.

Per-lane findings and evidence stay beside their extractor implementation under
`extractor/native/lanes/<lane>/`. This ledger receives only mechanically verified
promotion changes and genuine remaining extractor-owned dependencies. Closing a
row requires its evidence path and mechanical gate; prose assertion alone does
not remove it.

Frozen promotion tuple for A1/A2/A4/A12/A14/A15/A21/A22/B8-X6:
`extractor/documents/production_promotion/contract.json`.
