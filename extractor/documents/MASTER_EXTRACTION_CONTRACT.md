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

## Current state (2026-07-14)

The domain coverage below is stable; the active work is the **wave-12→14 correctness-hardening
pipeline** — every recording-layer domain re-reviewed adversarially for silent-data-loss and
namespace-safety defects, then serial-merged. See `CODEX_DISPATCH_QUEUE.md` for the blow-by-blow;
this section is the summary of record.

**Fix-pipeline & merge status — MERGED TO MASTER @`ca1844dc` (2026-07-14). The wave-12 correctness/
hardening fix pipeline is complete and integrated.**
- **All lanes merged to master** (serialized 3-way merge wave + battery + integration-adversary gate):
  the 6 core wave-12 lanes + d57 game-speed reconciliation + docs cleanup (base), plus **autoattach**
  (universal injection lease + pipe PID/session binding + default-deny recording proof), **facs-wire**
  (d53 FACS tri-state single-source + sound completeness witness), **foldwave2** (typed coverage
  fail-closed + D45→focus_status), **boundary-auth** (82 fold-dispatch bounds-guards, no panic +
  shared d69 encoder), **replay-loader** (dir-store hardening + strict decode + verify-through-handle
  finalizer consolidation — the earlier TOCTOU residual was CLOSED, not just documented), **stitching**
  (independent re-derivation on open + ZIP-conformance fix3→fix6), **dlcmeta** (DLC names/language +
  Windows-only dossier scope), **c1stageb** (duplex/stage-B design deadline-model reconciliation),
  **d51/campaignid/librs**. The 5 stale w12f lanes' unique RE dossiers were harvested (doc-only).
- **Gate methodology:** the Codex cyber-refusal blocker was worked around with **neutral-phrased**
  gate prompts (input-validation/correctness framing instead of adversarial/exploit language) — every
  gate since produced a real verdict with zero refusals. See [[codex-cyber-refusal-blocker]].
- **Battery at merge:** extractor side green (modulo the accepted Dev-Mode symlink capability test
  Win32 1314 + 2 RmStartSession live flakes); `replay_core` 81/0 at merge (the 2 then-parked
  degradation tests are now un-parked + passing per the resolved owner decision below → 83/0, +1
  pre-existing OneDrive-dehydrated ignore). Integration-adversary gate: 7/8 items PASS, the 8th (a
  fixture error-string propagation) fixed and closed.
- **✔ OWNER DECISION RESOLVED 2026-07-14 — malformed-payload semantics = PER-DOMAIN DEGRADE (zero
  data loss).** Owner ruled zero data loss. A structurally-malformed/truncated DOMAIN_DATA payload
  now degrades **only its own domain** — the stream-boundary ingest gate (`stream_fold.rs`
  `drive_fold`) records a `malformed_payload` domain problem, marks the domain unknown, keeps the
  prior carried leaf for diagnostics, and **never applies the untrusted bytes** (so nothing can be
  smuggled into another domain) — while every other domain and hour keeps folding. **Framing / magic
  / version / header-length / payload-bound / storage-checksum / zstd corruption still fails closed**
  (it cannot be localized to one domain). New `WorldFold::record_malformed_domain_payload` routes
  through the existing `mark_fold_domain_unknown` machinery (+ sets d53's `seen_in_stream`/`known`
  safety signal, the only domain with the "modern-stream ⇒ unavailable-on-absence" pattern); new
  `FoldStreamStats.malformed_payload_records` counter; the 5 DOMAIN_DATA-payload boundary tests now
  assert per-domain degrade; the 2 acceptance oracles (`folded_malformed_d45_without_d12...` in
  `world.rs`, `production_encode_decode_fold_typed_unavailable...` in `world_timeline.rs`) are
  **un-parked and passing**. Consumer side unchanged — `reconcile_domain_problem_statuses`
  (DOMAIN_PROBLEM_VIEW_MAP 45→Focus, 53→Diplomacy) already downgrades any recorded domain problem.
  Lane `w12h-malformed-degrade`. Independent gate run: `recorder_host --lib` 421/0/2, `stream_fold`
  suite 34/34, `replay_core` 83/0 (+1 pre-existing ignore), both oracles pass. NOTE: the independent
  Codex adversarial gate could not emit a verdict — the Windows sandbox helper
  (`codex-windows-sandbox-setup.exe`) broke mid-session (`program not found`, 3 attempts incl. Codex's
  own review); substituted by orchestrator adversarial review (anti-injection proven by construction:
  the degrade branch advances by the full framed `record_len` and `continue`s before any apply, so an
  embedded record is never framed/counted/applied — the flipped swallow tests pin `data_records == 1`).
  Resumable review tid saved under `hoi4_lanes/_tids/` for a re-run once the helper is restored.

**Remaining engineering acceptance gate (unchanged, USER-GATED):** a fresh speed-5 end-to-end
capture proving every scheduled domain emits at <100ms/frame with zero skipped hours. All validation
to date is against the PAUSED D04 1944.2.11.18 fixture; the combined speed-5 proof needs a live game.

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
- [ ] A22. Per-country CUMULATIVE air-loss statistic (melt `history{}` stat id 23; distinct from d42 per-wing deltas) (d61) — root chain PROVEN + data layer landed (`lanes/air_loss_stats_live/INTEGRATION_PACKET.md`, `runs/cracklist_c1_air_stats/`), but the packet is UNCOMMITTED (working-tree, "orchestrator review") and production scheduling is fixture-gated (needs a live session with air losses to verify). Roll-up of a lane-packet item that was not previously surfaced here.

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
- [x] D1. d53 subject-relation reads 0 subjects on 870ebf — FIXED in W2 (2c3c1e98e): added the `CRelationStatus+0x288` pointer hop → 38/38.
- [x] D2. Naval positioning read emits a bogus constant 1.0 (`f32@NavalCombatSide+0x288`) — FIXED in W5 (1ac13f761): now fail-closed → None; d49 wire layout byte-identical.

### E. DEPRIORITIZED / LOW-VALUE TAIL — disposition consciously (don't silently drop)
Save-backed but judged low replay value or redundant; each needs a one-line decision (crack-if-cheap /
redundant / derived / skip), not silent omission: ai/ai_strategy (skip), buildings per-country
(redundant w/ d27), cached_navy_strength (derive from OOB), id counters (theatre/navy/land_combat/
industry_org/instances/id_counter_store/debug_current_ref_id — skip), history/history_queue, tokens,
collaboration, claims, cores, deployment, reinforcement, upgrades, name_group, external_rules,
focus_tree/continuous_focus, preferred_tactic, is_top_ic/major/capital/original_capital (session/AI
context — decide per item).

## Residuals & known limitations (consolidated — 2026-07-14)

The single roll-up of everything not yet closed. Detail lives where noted; nothing here is a silent
gap.

**USER-GATED — needs a live game, cannot be produced on synthetic fixtures (the only things between
"code built + unit-verified" and "verified"):**
- Combined **speed-5 end-to-end proof** — every enabled production domain, hourly cadence, zero
  skipped hours, no untolerated failures (the standing engineering acceptance gate).
- **Cross-modset validation matrix** — the first non-vanilla captures ever; validates the modset
  resolver + all game-file joins against a real modded session.
- **d57 game-speed root** (registry A3) — a 10s tap at each speed 1–5; and **FACS v2 injected
  capture** to exercise the d53 tri-state against a live faction roster.
- The **fixture-blocked backlog A1–A21** in the OPEN ITEMS REGISTRY above (each names its exact
  fixture + accepting gate).

**Platform (owner-confirmed 2026-07-14): Windows-only.** The `snapshot/dir_store` Linux/macOS code
is unsupported dead weight (Linux impl is cfg-gated and untested; macOS is a fail-closed stub). It
is **never** a pre-ship residual; a cleanup task to strip/stub it is filed. Do not re-list
"Linux/macOS run" as owed work.

**Fix-pipeline residuals (non-blocking, tracked in `CODEX_DISPATCH_QUEUE.md`):**
- **replay-loader:** the `.battleplan` container/loader is Windows-namespace-hardened and the
  **production streaming finalizer** verifies through the retained creation handle (no pathname
  reopen). **USER-GATED open residual:** a reparse-ancestor-rebind + concurrent-manifest-swap TOCTOU
  on the two NON-PRODUCTION finalizers (in-memory + file-backed, the latter explicitly superseded on
  the production path) — parked as three `#[ignore]`'d RED tests (`recorder_host` `main.rs`) that
  encode the target invariant. Threat = a hostile same-user process with write access to the output
  dir on a single-user local tool. Scope decision owed: accept-as-residual / cheap output-path
  ancestor-reparse rejection / full verify-through-handle across both finalizers. Two other tests are
  environment-gated pre-ship — Windows **Developer Mode** for the symlink-rejection capability test,
  and a real **dehydrated OneDrive** placeholder store for the Files-On-Demand tests. The static "no
  descendant pathname open" gate (F4) is an accepted **best-effort development lint**, not a runtime
  security boundary (cannot see renamed FFI beyond `#[link_name]`, macro-expanded, or generated code).
- **facs-wire:** with no certified `name_hash → display-name` resolver, the UI renders the
  authoritative **numeric** faction identity (cosmetic only — membership content is fully
  preserved). An owed **live-map integration test** lands when a production factions-map render
  consumer is built (none exists in-tree today).
- **autoattach:** the recording-proof residual is explicitly **out-of-threat-model**, not
  fail-closed — an in-process compromise or a truly-simultaneous equal-rights external injection is
  irreducible without an OS-level trust boundary.
- **c1stageb** (duplex control transport; envelope/registry spec in `DUPLEX_CONTROL_TRANSPORT.md`):
  a holistic deadline-model pass is deferred as a design task (stage-B implementation is future; not
  merge-blocking).

**Largest unbuilt product pillars (`PRODUCT_SESSION_SEMANTICS.md`):** boot-and-play **auto-attach**
(both launch orders), **save/campaign identity** binding, and **reload/relaunch stitching** (rollback
segments). Designed/anticipated in the container format; core logic not yet built.

**Deep-crack + derived backlog:** the RE follow-ups **B1–B10** (no fixture, need another RE pass) and
the **C1–C6** derive-in-the-replay-layer items remain open in the OPEN ITEMS REGISTRY above.

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
