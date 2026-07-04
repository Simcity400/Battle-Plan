# Master Replay Extraction Contract

This is the active top-level checklist for deciding whether the recording layer
is actually done for a full 1:1 Battle Plan.

It is the only active extraction completion ledger. Older completion plans,
scoped-status summaries, research notes, and lane handoff packets are archived
under `extractor/documents/archive/` and are evidence only.

- `extractor/documents/DOMAIN_REGISTRY.md` assigns compact domain IDs.
- This file is the master contract: if every row here is closed, the extractor
  should have no known missed replay-relevant data.

## 2026-07-04 SESSION STATUS DELTA (authoritative — supersedes stale rows below)

The rows in the checklist below predate the 2026-07-04 crack/wire session. The
following statuses changed; where a row below still reads `FIXTURE_BLOCKED`/
`NOT_STARTED` for one of these, THIS DELTA is authoritative. Evidence docs cited.

CRACKED + WIRED into production this session (committed):
- Naval missions: mission enum `CTaskForce+0x364`, home base `CFleet+0xD8`, repair
  `+0x4B0` → `FIXTURE_BLOCKED`→**PRODUCTION_SLICE_DONE** (D56, commit 88b8f756a; WAVE4 doc).
- Aces: identity/death/killer/wing-assignment (`CAceEntry` +0x0C/+0xF0/+0x158/+0x164/
  +0x168, `CAirWing+0x2DC`) → **PRODUCTION_SLICE_DONE** (D46, 9e3ec2d50).
- Production licensing (`CCountry+0xF68` mgr) → **PRODUCTION_SLICE_DONE** (D11, c6e969d39).
- Division disrupted_supply (`CDivision+0xA0`) + supply_gain → **PRODUCTION_SLICE_DONE**
  (D20, 6c495fd92).
- Project start/completion lifecycle (`CProject` +0x1BC/+0x1BD via CCountry+0xFA8 chain)
  → `FIXTURE_BLOCKED`→**PRODUCTION_SLICE_DONE** (D41, 05e9a0af1; WAVE5 doc §2).

CRACKED (validated, WIRING queued — not yet in the container):
- Operations: running-op target `COperationInstance+0x50` (vt 0x29A76C0) → **cracked,
  wiring in progress** (D53 sidecar; WAVE5 §3). Owner +0x10 REJECTED.
- Diplomacy action queue: `CCountry+0xFF8` queue (corrects the wrong +0x4E0) → **cracked
  5/5 recipients / 7/7 actions** (CRACK_QUEUE_RESULTS §C3).
- Market stockpile + per-country owner chain: `CCountry+0xFB8`→wrapper→pool (vt 0x27330A0),
  i64 amounts → **cracked 70/70 by pointer** (CRACK_QUEUE_RESULTS §C1; WAVE5 §1).
- Cosmetic-tag live stream (`CCountry+0x1488`) → **PRODUCTION_SLICE_DONE** (D18,
  commit 5f7c75666; 29/29 live-exact vs melt; tri-state string|null|absent — schema
  delta to replay layer that §3.12 becomes nullable).
- Strategic-redeploy (`CDivision+0x29F`, 220==220), division current manpower
  (`CDivision+0x3D0→+4`, 2359/2359) + cumulative losses (`+0x668`) → **cracked**
  (WALL_CRACK_RESULTS).
- Combat attribution at COUNTRY granularity: `CStatsObserver→SDamageDone` dealer→victim→
  damage matrix (vt 0x2946F68/0x2946EC8), + loss stores CSunkConvoyInfo/CSunkShipInfo/
  SAirWingCombatData → **cracked** (WALL_CRACK_RESULTS). Finest per-entity killer = NEEDS-USER.

RE-CLASSIFIED (no longer memory rows):
- Exact province supply, resource flow, attrition reason, offmap factory → **DERIVED**
  (replay recomputes; no stored field — WALL_CRACK_RESULTS).
- Garrison child scalars → **RESOLVED** (wave-3: +0x88/+0xF0/+0xF8).

STILL OPEN / IN PROGRESS:
- Intel-network/spy-map graph (C4) → **CRACKED + verified**: CStrategicOperative+0x10 →
  66 CCountryIntelNetwork (vt RVA 0x298AB48) → +0x08 target tag, +0x18 graph → +0x40
  vertices / +0x38 state array (66/66, ENG-14/SOV-43 exact; CRACK_QUEUE_RESULTS §C4).
  Wiring queued.
- Game-speed root (C2) → **DEFERRED to fixture B0 (or deeper disasm)**: mechanism found
  (CSet/Increase/DecreaseGameSpeedCommand + GAME_SPEED_SECONDS), +0x66C retired as a
  false candidate; value has no paused oracle. Cheapest close = the 10s B0 speed-tap.
- Capitulation/war-detail, land/naval/air combat DETAIL, reinforcement reason,
  breakthrough state, scripted-state audit → **DONE (FINAL_SWEEP_RESULTS_2026-07-04.md)**:
  capitulation CRACKED + orchestrator-verified (`D=u64[CCountry+0xF88]`, capitulated u8
  @D+0x2E0 = exact 15/15 melt set; legitimacy/dates; war pool 958/958); land combat
  CRACKED (CCombatSide 0x29913C0, 100/100); naval combat CRACKED (CNavalCombatant
  0x29CD298, 8/8); reinforcement-reason + surrender_progress DERIVED; peace_conference
  absent; breakthrough/land-tactic PARTIAL (bounded no-user RE); D51 scripted-state
  coverage complete. No un-attempted memory category remains.
- Genuine USER-fixture floor (4): hub motorization level, focus-paused bit, convoy/port
  PRESSURE, MP chat/hotjoin. Plus the fine per-entity combat killer (1 event).
- Architecture: capture is HOURLY SNAPSHOTS; a real-time delta/event stream is a deferred
  slice (archive/plans/RAW_DELTA_EVENT_STREAM_PLAN.md), NOT built.

## Completion Rule

A gameplay concept is not complete just because one compact domain passes.

For a row in this document to be considered closed, every replay-relevant field
in that row must be one of:

1. Captured through the production path:
   `hoi4.exe -> one injected DLL collector/coordinator -> typed compact records
   -> Rust recorder -> .battleplan`.
2. Rejected with proof:
   exact candidates/offsets/functions were tested and should not be retried
   without new evidence.
3. Blocked with proof:
   an exact fixture/tooling/root wall is documented and no unblocked production
   work remains for that row.

Python, saves, JSON root frames, heap scans, and bridge rows may support
research and validation. They do not close a production row unless the runtime
recording path itself is DLL-only and compact.

## Status Values

| Status | Meaning |
| --- | --- |
| `CONCEPT_COMPLETE` | The whole row is captured, rejected, or blocked with exact proof. This is the only status that means "done" for 1:1 replay coverage. |
| `PRODUCTION_COMPLETE` | The named runtime infrastructure/domain contract is accepted in the shared DLL/Rust path, but parent gameplay concepts may still have other rows open. |
| `PRODUCTION_SLICE_DONE` | A scoped production slice passed the DLL/compact/Rust/.battleplan gate at hourly-or-better speed-five/current-equivalent cadence, with zero skipped hours and no untolerated partial/source/worker/bridge failures. |
| `VALIDATED_PARTIAL_SLICE_R4` | A shared validated partial slice is schedulable and folded for its explicitly named fields, but broader domain/category fields remain open or fixture-blocked. |
| `PARTIAL:PAUSE_VALIDATED_SPEED_OPEN` | Pause state is validated and integrated, while speed/session timing semantics remain open. |
| `SHARED_INTEGRATION_PENDING` | Lane-local implementation/proof exists, but the shared one-DLL collector and Rust recorder have not accepted it. |
| `LANE_LOCAL_PROVED` | A lane-local mini stack or proof exists, but it is not yet a shared production slice. |
| `FIXTURE_BLOCKED` | A specific live/UI/save fixture is required before the row can be captured, rejected, or semantically named. |
| `ROOT_DISCOVERY_OPEN` | The row matters for replay, but a production-safe root/path/hook is not pinned. |
| `REJECTED_NO_RETRY` | A candidate was conclusively rejected. This status closes only that candidate, not the whole row unless all candidates/fields are covered. |
| `NOT_STARTED_OR_UNAUDITED` | The row has not been fully inventoried against current live memory, static definitions, UI-visible state, and save-visible state. |

## Production Gate Required For Captured Rows

Captured rows must prove all of the following:

- DLL-only capture in the hot recording path.
- Compact binary records, not full JSON frames.
- Rust recorder decodes the records.
- `.battleplan` verifies.
- Hourly-or-better cadence at speed five/current equivalent.
- Zero skipped game hours.
- No untolerated source-unavailable, partial-data, worker-panic,
  bridge-unavailable, or equivalent failure flags.
- Captured rows must be proven under the recorder `--production` profile, not
  only under an ad hoc subset of `--scheduled-domain` flags.
- Fail-closed behavior for invalid layouts.

### Expected Partial Tolerance Policy

The production gate remains fail-closed. The only expected partials are the
domain/flag pairs in `compact_contract::COMPACT_EXPECTED_PARTIAL_TOLERANCES`.
Those entries may absorb `COMPACT_STATUS_PARTIAL_DATA` only for the named
fixture-blocked, skip-counted sub-scopes; they never absorb source-unavailable,
control-invalid, worker/bridge, or other failure bits. The recorder manifest
must surface each applied tolerance with its domain, flags, reason, and affected
frame count.

## Current Production / Profile-Scheduled Domains

These domains currently have production-proven or existing compact coverage, or
are profile-scheduled pending the batched live proof (see each row's status).
Neither automatically makes their whole gameplay category complete.

| Domain | Status | Scope |
| --- | --- | --- |
| `fuel_live` / 10 | `PRODUCTION_SLICE_DONE` | Country fuel rows. |
| `production_live` / 11 | `PRODUCTION_SLICE_DONE` | Stockpile, production lines, naval lines, construction queues as currently encoded. |
| `focus_live` / 12 | `PRODUCTION_SLICE_DONE` | Base focus tree/current/completed/progress fields as currently encoded. |
| `tech_live` / 13 | `PRODUCTION_SLICE_DONE` | Base research/completed-tech/active-slot fields as currently encoded. |
| `politics_live` / 14 | `PRODUCTION_SLICE_DONE` | Base politics, active ideas/laws, ruling-party/leader fields as currently encoded. |
| `politics_residuals` / 15 | `PRODUCTION_SLICE_DONE` | Last election, elections allowed, appointed-advisor residual fields where contracts are supplied. |
| `decisions_live` / 16 | `PRODUCTION_SLICE_DONE` | Base decision status and timed-decision rows as currently encoded. |
| `diplomacy_live` / 17 | `PRODUCTION_SLICE_DONE` | Base country diplomacy rows as currently encoded. |
| `divisions_live` / 20 | `PRODUCTION_SLICE_DONE` | Division identity, location, movement, org/strength, combat references, owner, parent army, template id, XP, supply/fuel, deletes. |
| `land_combats_live` / 21 | `PRODUCTION_SLICE_DONE` | Active land-combat lifecycle snapshots and participant vectors; folded into REPLAY_SCHEMA §7 `combats[]` snapshots. |
| `map_control_live` / 22 | `PRODUCTION_SLICE_DONE` | State ownership/control and province controller/state rows. |
| `map_detail_live` / 23 | `PRODUCTION_SLICE_DONE` | State resources and state/province building levels; folded into REPLAY_SCHEMA section 4 `states[*].resources`, `states[*].buildings`, `states[*].province_buildings`, with an R2 section 8 `map.map_detail_live` mirror. |
| `weather_live` / 24 | `PRODUCTION_SLICE_DONE` | Visual weather drivers including sandstorm slice; projected under `map._r2_weather_live` as an R2-documented additive key pending schema-owner acceptance because the schema has no first-class weather home; non-visual internals remain open. |
| `division_equipment_live` / 26 | `PRODUCTION_SLICE_DONE` | Division held equipment entries as currently encoded; folded onto `divisions[*].equipment` only through a unique owner/division id join against D20, with unmatched/ambiguous rows dropped and counted in provenance. |
| `army_orders_live` / 27 | `PRODUCTION_SLICE_DONE` | Active army orders reachable from live divisions, type-5 target-state slices, deletes; projected under `order_of_battle.battle_plan_orders` as an R2-documented recorder shape pending schema-owner acceptance. |
| `oob_command_live` / 28 | `PRODUCTION_SLICE_DONE` | Active land OOB plus widened naval hierarchy rows where accepted; projected under `order_of_battle` command/fleet arrays with `_r2_oob_command_capture` evidence, pending schema-owner acceptance of the live raw shape. |
| `ships_live` / 29 | `PRODUCTION_SLICE_DONE` | Ship telemetry, owner attribution, task-force linkage, definition/category, strength/org/XP, deletes; projected under `order_of_battle.ships` and task-force ship lists as an R2-documented recorder shape pending schema-owner acceptance. |
| `air_wings_live` / 30 | `PRODUCTION_SLICE_DONE` | Air-wing rows as currently encoded; D30 owns `order_of_battle.air_wings` existence with `_r2_air_wing_capture` evidence. |
| `air_region_activity_live` / 31 | `PRODUCTION_SLICE_DONE` | Air-region activity rows as currently encoded; folded into REPLAY_SCHEMA sections 13.1/14 `_r2_air_region_activity`. |
| `air_wing_loadouts_live` / 32 | `PRODUCTION_SLICE_DONE` | Air-wing loadout rows as currently encoded; attaches equipment/loadout annotations only to D30-owned `order_of_battle.air_wings` rows, with current-hour orphan rows pruned and noted as R2 provenance. |
| `air_variant_defs` / 33 | `PRODUCTION_SLICE_DONE` | Air variant definition rows as currently encoded; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live` definitions. |
| `air_base_status_live` / 34 | `PRODUCTION_SLICE_DONE` | Air-base status rows as currently encoded; folded into REPLAY_SCHEMA section 14 `strategic_air.air_bases`. |
| `air_variant_stats` / 35 | `PRODUCTION_SLICE_DONE` | Air variant stat rows as currently encoded; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live.live_stats`. |
| `air_variant_reserves` / 36 | `PRODUCTION_SLICE_DONE` | Air reserve rows as currently encoded; folded into REPLAY_SCHEMA section 11 `designs._r2_air_variant_reserves`. |
| `stockpile_variant_defs` / 37 | `PRODUCTION_SLICE_DONE` | Stockpile variant definition rows as currently encoded; folded into REPLAY_SCHEMA section 11 `designs._r2_stockpile_variant_defs`. |
| `air_superiority_inputs` / 38 | `PRODUCTION_SLICE_DONE` | Air superiority input rows as currently encoded; folded into REPLAY_SCHEMA sections 13.1/14 `_r2_air_superiority_inputs`. |
| `air_variant_upgrades` / 39 | `PRODUCTION_SLICE_DONE` | Air variant upgrade rows as currently encoded; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live.upgrade_levels`. |
| `stockpile_variant_upgrades` / 40 | `PRODUCTION_SLICE_DONE` | Stockpile variant upgrade rows as currently encoded; folded into REPLAY_SCHEMA section 11 `designs._r2_stockpile_variant_upgrades`. |
| `projects_live` / 41 | `PRODUCTION_SLICE_DONE` | Facilities, scientists, progress/completed-bit slice folded into top-level `special_projects`; raw handles that need name resolution stay in clearly named `_r2_*` leaves; project lifecycle/rewards still open. |
| `air_combat_losses_live` / 42 | `PRODUCTION_SLICE_DONE` | Air loss/reinforcement delta slice; folded only into REPLAY_SCHEMA sections 13.1/14 `_r2_air_combat_losses_live`, not `events[]`; post-failure-gap deltas may aggregate changes across the gap and are not exact per-hour proof; full air combat remains open. |
| `market_trade_live` / 43 | `PRODUCTION_SLICE_DONE` | Purchase-contract slice plus dynamic D-tag buyer handling and MTD1 detail trailer; folded into REPLAY_SCHEMA section 7b `diplomacy_world.trade_contracts`, per-country `diplomacy.lend_lease` projections, and `r2_market_trade_live` raw detail/progress leaves; full trade economy remains open. |
| `supply_logistics_attrition_live` / 44 | `SHARED_INTEGRATION_PENDING` | Supply/logistics subset is shared integrated through the anchored `CGameState+0x3D8` root and folded into REPLAY_SCHEMA section 13.2 `supply.by_province` with `r2_logistics_attrition` badges. Remaining: advancing speed-five/current-equivalent hourly proof before this slice can be counted `PRODUCTION_SLICE_DONE`; exact province supply wall remains open. |
| `focus_research_decisions_depth_live` / 45 | `PRODUCTION_SLICE_DONE` | Transition-depth slice plus active targeted-timed country-target rows folded as R2 focus/research/decision annotations, `countries[*].decisions._r2_depth_targets`, and provenance badges from the active decoder; D12 owns first-class focus fields, with D45 only supplying an absent-only fallback for `focus.current`, `focus.current_continuous`, and `focus.completed_count`; paused focus, numeric targets, decision body/cooldown fields, unmatched transitions, and event hooks remain open/blocked. |
| `air_variant_archetypes_live` / 46 | `PRODUCTION_SLICE_DONE` | Active air variant archetype slice; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live` archetype/R2 metadata; full air lifecycle/ace linkage remains open. |
| `scripted_state_flags_variables_live` / 51 | `PRODUCTION_SLICE_DONE` | Shared registered country/global/state variables and token/date/value flags folded into `world`, country `extra`, and state `_r2_scripted_state` leaves; opaque-root statuses are provenance notes, while partial/source-unavailable frames remain fail-closed unknown hours. |
| `occupation_resistance_compliance_live` / 52 | `PRODUCTION_SLICE_DONE` | Occupation/resistance/compliance state rows folded into REPLAY_SCHEMA section 4 `states[*].occupation`; law-token and garrison-template/scalar extras remain under `_r2_occupation_resistance_compliance_live` pending verified joins. |

## Shared Or Lane-Ready Domains Not Accepted As Production Complete

| Domain | Status | Why it is not closed |
| --- | --- | --- |
| `characters_leaders_cabinet_live` / 47 | `SHARED_INTEGRATION_PENDING` | Shared registration now schedules the honest `ValidatedSliceOpenGaps` descriptor in the production profile for the validated offline slice: live character identity, unit-leader identity/stats, country-leader role edges, and active ruler rows. Fold writes only `countries[*].commanders.leader` where REPLAY_SCHEMA defines it plus `_r2_characters_leaders_cabinet_live` for richer character detail; domain 14 keeps ownership of `politics.*` leader/ruling-party/advisor fields. Blocked pending current-build revalidation: advisor roster rows, appointed advisor rows, character portrait rows, and unit-leader trait-token rows (`CAdvisor` / `CUnitLeaderTrait`). Full live speed-five proof and broader leader/cabinet closure remain open. |
| `construction_buildings_state_damage_live` / 48 | `SHARED_INTEGRATION_PENDING` | Shared compact integration exists for the validated offline slice: construction queue rows, all-or-none `target_state_id`, numeric state/province building health rows, and numeric state resource rows. Fold projects the state/province overlap under domain-owned `construction_buildings_state_damage_live` sidecars rather than D23-owned `states[*].resources` / `buildings` / `province_buildings`. Fixture-blocked: token/name resolution, created-date semantics, exact target province/location beyond state, repair semantics/speed, and live speed-five replay proof. |
| `naval_combat_lifecycle_live` / 49 | `SHARED_INTEGRATION_PENDING` | Lane live gate PASSED 2026-07-03 on the paused D04 1944 4-battle fixture (`runs/nav49_50_port_validation/live_gate/`: harness `status=passed`, `validated_combat_count=4`, Rust proof `passed`, `.battleplan` byte-verified). `front+0xA4` (u32 province) is PROMOTED: compact `location_id` emitted 4/4 (live == save `naval_combat.location` for 2804/6434/1032/8819) plus static `region_id` join via `map/strategicregions`; `STATUS_LOCATION_CANDIDATE_UNVALIDATED` now fires only as the fail-closed missing-field signal. Live `combat_id` (body+0x18 high u32, type 0x1269) matched save `naval_combat.id.id` 4/4. All pinned vtable RVAs are RTTI-name-verified at attach (fail closed, `vtable_rtti_mismatch`). Remaining: orchestrator serial-integration of compact 49/50 emission into the shared collector/recorder, lifecycle-end (battle-teardown) fixture, and speed-five proof. |
| `naval_combat_members_live` / 50 | `SHARED_INTEGRATION_PENDING` | Lane live gate PASSED 2026-07-03 with the CORRECTED member chain (prior side+0x2A0/+0x2A8/+0x2B0 vector REJECTED): heap-scan CFEXGroup vtable 0x2A42D18 (RTTI `.?AVCFEXGroup@@`, attach-verified) and bucket groups by the AUTHORITATIVE owning-side back-ref `group+0x38`; group `+0x08` member array / `+0x10` cap / `+0x14` count (empty groups valid); the `side+0x20` forward child route (circular `+0x20` next-chain spanning both sides) is an advisory subset cross-check only — the child is absent/unreadable for some live battles. Save-oracle result (`runs/nav49_50_port_validation/`): 117/117 member rows, 8/8 sides' ship-id sets, convoy counts (packed CFEXMember handle 0 == save `convoy={...}` member), and member `+0xE0` state / `+0x120` slot multisets matched the same-tick save; 101/101 non-convoy members resolved through CShip (task-force/fleet/hull/sunk-convoy telemetry, `cship_unresolved_member_count=0`). Remaining: orchestrator serial-integration into the shared collector/recorder, member join/leave + sinking fixture, and speed-five proof. |
| `diplomacy_intel_operatives_live` / 53 | `VALIDATED_SLICE_OPEN_GAPS` | Re-enabled 2026-07-03 after live 870ebf root re-validation (`runs/domain53_revalidation_870ebf_20260703/`, constant-by-constant audit in the lane's `REVALIDATION_870EBF.md`: 44 VALIDATED / 27 STRUCTURAL-ONLY / 0 unvalidated). Enabled sub-scopes: agencies (name/icon/defense/upgrade, raw E4/F0/F4 capacity cluster — old max/usable semantic assignment REJECTED by probe, fields carry raw candidates only), cryptology, operatives + lifecycle groups, relations/opinions (heap+bounds gated), relation flags (exact current vtable equality), subjects (field sanity gates), factions (multi-condition runtime gate: bounded reads, >=40 countries covered, >=1 nonzero + >=1 shared group, else `STATUS_FACTION_GROUP_UNVALIDATED` and no rows), war tag-pair/casualty rows (lifecycle-unvalidated badges kept). Build-identity gate: MSVC RTTI walk must yield `CCountry` @ 0x27D0E90 and `CIntelligenceAgency` @ 0x296B338 before `SOURCE_PROOF_BUILD_SHA_870EBF` is stamped; wrong/garbled RTTI fails the domain closed. Descriptor schedulable + in the production profile. Fixture-blocked/fail-closed: operation detail, spy-network/intel-map percentages, incoming action queues, full war grouping/lifecycle semantics (all emit explicit resisted/absent markers, never plausible empties). |
| `army_battleplans_depth_live` / 54 | `SHARED_INTEGRATION_PENDING` | Shared collector/recorder/fold wiring exists for the validated partial slice: active-army order rows, exact type-5 target-state rows, exact save/live-paired non-type-5 `order+0x70/+0x78` path-geometry rows, and same-frame assignment/linkage status. Offset provenance is current-build live `870ebf` evidence, not older-build carryover (`army_battleplans_depth_live_save_geometry_870ebf_20260701_v1`). Shared speed-five `.battleplan` proof and live redraw/toggle fixture closure remain. Fixture-blocked/fail-closed: direct empty/inactive roots, activation/execution values, stance/aggressiveness values, redraw/toggle lifecycle, non-path geometry semantics, and full save-shaped OOB reconstruction. |
| `land_combat_detail_events_live` / 55 | `SHARED_INTEGRATION_PENDING` | Lane-local compact/Rust/.battleplan proof exists for phase lifecycle and participant join/leave-unclassified records; domain 55 is now registered in shared collector/recorder and folded onto active §7 combat snapshots. Remaining: live speed-five proof and the still-incomplete combat-detail field set. |
| `naval_missions_status_live` / 56 | `SHARED_INTEGRATION_PENDING` | Lane-local compact/Rust packet plus collector/recorder/fold registration for the validated fleet/task-force/ship-root slice, folded as an R2-projected `order_of_battle.extra.naval_missions_status_live` annotation: task-force id, fleet id, owner tag-db id, current province, optional derived region, and aggregate sunk-convoy count. Mission, home-base, repair/retreat, and convoy route/context semantics remain fixture-blocked; shared speed-five/.battleplan proof still required. |
| `session_multiplayer_metadata_live` / 57 | `VALIDATED_PARTIAL_SLICE_R4` | Shared collector/recorder/fold integration schedules the validated partial slice only: player-country human TAG rows from `CGameState=u64[module+0x3316E90]` via `+0x394` count / `+0x388` human byte array / `+0x310` handles (`handle+0x08` slot index) / `+0x358` TAG records stride `0x20`, plus pause state from clock singleton slots `module+0x33172E0` and `module+0x33172E8`, vtable `module+0x2952510`, bool `+0x6C1`. Live proof: 2026-07-02 read-only strict probe, SP oracle exactly one human row IRE/Ireland index 29, pause byte `[1,1,1]`. Fold home is top-level `_r2_session_multiplayer_metadata_live` with normalized `human_marker` player rows and per-hour pause; the raw `+0x388` status byte is not carried. Fail-closed/open: game-speed root remains unpinned; player_name/player_key/session_slot/host/connected and MP session-event hooks remain fixture-blocked and are not emitted. |
| `country_economy_counters_live` / 58 | `ROOT_DISCOVERY_OPEN` | 2026-07-03 live investigation (5 probe cycles, 1943 IRE save, `INTEGRATION_PACKET.md`): anchors VALIDATED and reusable — `CGameState@module+0x3316E90`, country table `CGameState+0x310`/count `+0x31C`, `CCountry` vtable `module+0x27D0E90`, `CCountry+0x8` tag echo, `CProductionManager` chain, 394 production lines. Factory/dockyard/consumer TOTALS are NOT a directly-readable per-country memory scalar: a melted-save derived-field oracle (v3, 490 rows) and a raw-stored-totals oracle (v4, 506 rows) BOTH validated 0/8 cross-country; candidate offsets hold garbage/zero (`0x1750`, `0x17E0`) or real-but-unmapped per-country data (`0xF08`). Consistent with the game recomputing factory counts from state building levels each tick. Resources also 0/8 (v2 resource-vector). Fuel=domain 10, stability/war-support=domain 14, market/licensing=domain 43 (overlaps, excluded). | Orchestrator/user decision: (a) DERIVE factory totals in the replay from domain-23 state building levels (already extracted) plus a small offmap-factory extraction, OR (b) before/after DELTA fixture (change a country's factory count by a known amount in-game, re-probe for the offset that moved by that delta — may find nothing if purely recomputed). Resources need a resource-flow delta fixture or better oracle. |
| `templates_equipment_catalog_live` / 59 | `SHARED_INTEGRATION_PENDING` | Strict vanilla+mod static catalog gate passed 2026-07-02 after the local vanilla install root was located from the running process: 306 division templates, 1180 unit definitions, 59 equipment definitions, zero warnings, `.battleplan` byte-verified. Production packing shipped: `recorder_host` (`collector_record --static-templates-equipment-catalog <path>`) byte-copies the catalog into the production `.battleplan` as a `role="static_artifact"` entry, hashes it (sha256), and records its presence + redacted mod-identity keying in `index.json`'s top-level `static_artifacts` roster (absence fail-closed when the flag is omitted); open+verify_all round-trips the payload byte-identically. Remaining: live division/template-id joins from catalog rows (open); full production-slice acceptance awaits the batched live proof. |

## Master Gameplay Checklist

### 1. Session, Clock, Stream, And Replay Container

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Executable/build profile | Build hash, layout profile, checksum/mod identity, accepted offsets. | `PRODUCTION_SLICE_DONE` | Current 870ebf profile proofs. | Mark `CONCEPT_COMPLETE` only after profile migration/rejection procedure is documented for every supported build. |
| Game clock | Date/hour, monotonically ordered game-hour raw, pause/speed context where needed. | `PRODUCTION_SLICE_DONE` | Frame domain / compact perf gate. | Add explicit pause/speed-state row if replay UI requires exact playback timing beyond game-hour cadence. |
| Compact stream framing | Sequence id, game hour, domain id, record type, payload length, ordering. | `PRODUCTION_COMPLETE` | Domain 1000 / compact gate. | Keep complete as long as new domains use the same gate. |
| Rust decode and validation | Decode every domain, reject corrupt rows, expose counters/failures. | `PRODUCTION_COMPLETE` | Recorder host gates. | Every new domain must add decoder/gate coverage before acceptance. |
| `.battleplan` packing | Raw compact stream plus manifest packaged and verified. | `PRODUCTION_COMPLETE` | `pack_replay verify`. | Keep complete as long as new roles/domains verify. |
| Skipped-hour detection | Detect missing frame hours and missing scheduled domain records. | `PRODUCTION_COMPLETE` | Compact replay verification gates. | Every new scheduled domain must be covered by the same gate. |
| Debug JSON | Debug visibility only. | `CONCEPT_COMPLETE` | Architecture decision. | Must never be used as production feasibility proof. |

### 2. Static Definitions And Replay Enrichment

Static definitions are not necessarily live-memory rows, but they are required
for an authentic replay because compact records often carry token ids or handles.

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Country/tag definitions | Tags, names, colors, dynamic D-tags, cosmetic tags. | `PRODUCTION_SLICE_DONE` | Country table and dynamic D-tag handling. | Audit dynamic/cosmetic tag lifecycle and exile/civil-war variants. |
| State/province/map definitions | State ids, province ids, terrain, continents, strategic regions, adjacency, supply areas. | `PRODUCTION_SLICE_DONE` | `map_control_live`, `map_detail_live`, game defs. | Full map definition parity audit against the game files and replay renderer. |
| Localisation | Names for countries, focuses, ideas, techs, decisions, equipment, characters, states. | `PRODUCTION_SLICE_DONE` | `extractor/game_defs/parse_localisation.py` (2026-07-02): 139,950 english keys, mod-over-vanilla override order, packed+byte-verified into a `.battleplan` static artifact via `localisation_gate.py` (domain-59 pattern). Direct id→loc resolves cleanly for focuses, country TAGs, equipment archetypes, state names. Production packing shipped: `recorder_host --static-localisation-join <path>` byte-copies the R3 bundle into the production `.battleplan` as a hashed `role="static_artifact"` entry and records its presence + redacted mod-identity keying in `index.json`'s `static_artifacts` roster (fail-closed absence when omitted). Definition-side cross-refs shipped 2026-07-02: `parse_idea_tech_loc.py` (ideas: scalar `name=` override wins, else id; techs: id-derived title + separate `desc_loc_key`; 6,522 ideas / 557 techs) and `parse_decisions.py` (9,525 decisions, category-attributed), wired into `build_all.py`; gate now emits per-category loc coverage with a no-path-leak self-assertion. The idea/tech cross-reference bundle is production-packed 2026-07-03 via `recorder_host --static-idea-tech-loc-xref` (hashed `role="static_artifact"` `idea_tech_loc_xref_static`, fail-closed shape validation, whitelisted provenance, production-required). Coverage vs catalog (corrected accounting, runtime-required tokens bucketed separately from the static denominator): state 100%, focus 99.92%, equipment 99.68%, decision 98.32%, country TAG 97.06% (75 dynamic D-tags runtime-required; only EFR/WST unresolved), tech 92.82% (source-ordered `enable_equipments`/module loc joins reported as `resolved_via_equipment_loc`, tech's own key wins), character 83.57% (quoted literal names distinguished from key-shaped strings; key-shaped-without-catalog-hit buckets as `quoted_key_missing_loc`, never fake-resolved), idea 83.41%. | Close remaining tails (ideas: legacy advisor ids without names; techs: 40 hidden/special-project; characters: 487 missing `name` fields) or reject exactly; dynamic D-tag names come from runtime capture (domain 18 `country_identity` / cosmetic-tag work). |
| GFX/assets/icons | Unit icons, leader portraits, idea icons, equipment icons, focus icons, map assets. | `NOT_STARTED_OR_UNAUDITED` | UI/game asset work exists separately. | Define which assets must be bundled/referenced for replay. |
| Unit templates and equipment definitions | Template catalog, battalions/support companies, equipment archetypes, stats. | `SHARED_INTEGRATION_PENDING` | Domain 59 strict vanilla+mod static gate passed 2026-07-02: parses `common/units`, `common/units/equipment`, and `history/units` across both layers, emits a SHA-256 source inventory, and byte-verifies the replay-packed catalog (306 templates / 1180 unit defs / 59 equipment defs, zero warnings). Production packing shipped: `recorder_host --static-templates-equipment-catalog <path>` packs the catalog into the production `.battleplan` as a hashed `role="static_artifact"` entry with a `static_artifacts` roster in `index.json` (present/absent + redacted mod-identity keying, fail-closed on a missing/corrupt path). | Join catalog rows to live division/equipment rows; full production-slice acceptance awaits the batched live proof. |
| Technology definitions | Tech names, categories, costs, prerequisites, effects. | `PRODUCTION_SLICE_DONE` | `tech_live`, static tech maps where supplied. | Audit effect/prerequisite parity needed by replay. |
| Focus definitions | Focus ids, tree structure, icons, prerequisites, rewards. | `PRODUCTION_SLICE_DONE` | `focus_live`. | Reward semantics and completed-token closure must join cleanly. |
| Decision definitions | Decision ids, categories, costs, targets, cooldowns, visible/available metadata. | `PRODUCTION_SLICE_DONE` | Static definitions shipped 2026-07-03: `parse_decisions.py` full-catalog parser (9,525 decisions / 611 categories, fail-closed, `decisions_static_gate.py` green, commit `0faf7e88c`) with costs, targets, cooldown/removal and visibility/availability metadata plus loc joins; production packing shipped via `recorder_host --static-decisions-definitions` (hashed `role="static_artifact"` `decisions_static_definitions`, fail-closed shape validation, whitelisted provenance, production-required). Live decision STATE stays domains 16/45 (own rows). | Live-token join (`countries[*].decisions.timed_live[*].decision` ↔ `decisions.<id>`) is asserted-not-verified — confirm on a decode with active decisions. |
| Idea/law/advisor definitions | Idea groups, law buckets, national spirits, advisor slots/ledgers. | `PRODUCTION_SLICE_DONE` | `politics_live`, `politics_residuals`. | Cabinet/advisor hired-state and character joins must close under domain 47. |
| Character definitions | Character ids, portraits, roles, traits, advisor/general/admiral/scientist metadata. | `ROOT_DISCOVERY_OPEN` | Scientist route and domain 47 lane. | Shared production character/cabinet route plus static portrait/trait joins. |
| Ship/air variant definitions | Hull/airframe categories, modules, upgrades, stats. | `PRODUCTION_SLICE_DONE` | `ships_live`, air variant domains. | Audit name/module/stat closure and token fallback rows. |

### 3. Country Core And Economy

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Country identity/rooting | Country object table, tag db id, tag string, dynamic tags. | `PRODUCTION_COMPLETE` | `CGameState+0x310` country table and market D-tag proof. | Complete if civil-war/exile/cosmetic variants are audited. |
| Fuel | Current fuel and scale metadata. | `PRODUCTION_SLICE_DONE` | `fuel_live` / 10. | Audit capacity/gain/use if not already in production/economy rows. |
| Stockpile | Equipment stockpile rows and variant joins. | `PRODUCTION_SLICE_DONE` | `production_live`, stockpile variant domains. | Audit captured/lost/obsolete/equipment-type semantics. |
| Military production | Lines, assigned factories, output/progress where encoded. | `PRODUCTION_SLICE_DONE` | `production_live` / 11. | Close any missing efficiency, resource shortage, conversion, and line-history fields. |
| Naval production | Ship construction lines and related counters. | `PRODUCTION_SLICE_DONE` | `production_live` / 11. | Join to ship/hull definitions and construction completion events. |
| Construction queue | Queue rows as currently encoded. | `PRODUCTION_SLICE_DONE` | `production_live` / 11. | Full construction/state-damage domain 48 must be shared accepted. |
| Factories/industry capacity | Civilian/military/naval factories, damaged/used/free, consumer goods. | `ROOT_DISCOVERY_OPEN` | Production/map detail slices. | Audit all UI-visible economy counters and capture or reject each. |
| Resources | Produced, imported, exported, used, deficit, market/trade effects. | `ROOT_DISCOVERY_OPEN` | `map_detail_live`, `market_trade_live` residuals. | Close resource import/export flows and market stockpile rows. |
| International market contracts | Seller/buyer, contract id/type, equipment variant, amount/delivery fields. | `PRODUCTION_SLICE_DONE` | `market_trade_live` / 43, including MTD1 detail rows. | Keep as slice; full trade rows below remain open. |
| Market price/speed/completion | Price, speed, completed state, delivery route/progress. | `PRODUCTION_SLICE_DONE` | Domain 43 shared MTD1 detail trailer from current-build 870ebf proof: 63 same-tick detail rows; price uses the shared equipment-subbuffer variant amount root, amount uses the delivery-subbuffer amount, and validated direct offsets are speed `CPurchaseContract+0xC0`, completed `+0x25C`, days `+0x1D0`, delivery-country tag id `+0x5C`, convoys `+0x118/+0x11C`, efficiency `+0x1C0`, factory CIC progress `+0x1E0`, and equipment-transfer progress `+0x1E8`. | Keep raw progress/detail values under R2 until schema first-class fields exist; full route/path, stockpile, MTLL, and broader market semantics remain open. |
| Market stockpile/sellable equipment | Sellable equipment and market inventory. | `ROOT_DISCOVERY_OPEN` | `CCountry+0x1230` candidate rejected no-retry by same-tick save/live fixture. | Find an alternate production root, or prove-backed rejection for remaining market-stockpile semantics. |
| Resource trade/import/export | Non-market resource flow, imports, exports, convoys, trade law impact. | `ROOT_DISCOVERY_OPEN` | Market residual docs. | Production root or proof-backed rejection. |
| Lend lease | Directed lend-lease contracts, equipment, convoys, delivery progress. | `PRODUCTION_SLICE_DONE` | Domain 43 projects purchase-contract rows plus MTD1 detail fields into per-country `diplomacy.lend_lease`; MTLL-specific mapping remains lane-local/open. | Capture or reject MTLL-specific semantics, diplomacy context, and full lend-lease lifecycle before concept completion. |
| Licensing | Active license production relationships and details. | `ROOT_DISCOVERY_OPEN` | Rejected old vectors; active-license fixture needed. | New root or proof-backed rejection. |

### 4. Politics, Leaders, Cabinet, And Internal Country State

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Political power | Current value. | `PRODUCTION_SLICE_DONE` | `politics_live` / country political status. | Audit gain/spend history if required for replay timeline. |
| Ruling party and popularity | Ruling party plus party popularity rows. | `PRODUCTION_SLICE_DONE` | `politics_live` / 14. | Complete if all ideology groups and dynamic party names are joined. |
| Laws | Economy, trade, mobilization, conscription/other law buckets. | `PRODUCTION_SLICE_DONE` | `politics_live` idea classification. | Audit every law group emitted by static definitions. |
| National spirits and ideas | Active ideas, idea groups, national spirits. | `PRODUCTION_SLICE_DONE` | `politics_live`. | Close token fallback/missing static-contract rows. |
| Stability and war support | Current values and changes. | `PRODUCTION_SLICE_DONE` | Existing politics/country rows if encoded. | Verify explicit counters in compact manifest or add rows. |
| Elections | Last election, elections allowed, frequency. | `PRODUCTION_SLICE_DONE` | `politics_residuals` / 15. | Audit election event/lifecycle rows. |
| Country leader | Active leader character rows. | `PRODUCTION_SLICE_DONE` | `politics_live` leader extraction. | Close multi-entry ruling-party leader vector semantics. |
| Advisors/cabinet | Hired advisors, slots, ledgers, availability, traits/effects. | `ROOT_DISCOVERY_OPEN` | `politics_residuals`, domain 47 lane. | Domain 47 advisor roster/appointee/portrait rows remain blocked pending current-build `CAdvisor` / portrait-feed revalidation; static joins still open. |
| Military high command/theorists/designers | Hired state, roles, effects, availability. | `ROOT_DISCOVERY_OPEN` | Domain 47 lane. | Keep excluded until advisor/appointee roots and role semantics are revalidated or exactly blocked. |
| Characters/leaders/officers | Generals, field marshals, admirals, traits, XP, assignment, portraits. | `SHARED_INTEGRATION_PENDING` | `characters_leaders_cabinet_live` / 47. | Shared production-profile slice now covers live character identity and unit-leader identity/stats; unit-leader trait-token rows and portraits are blocked pending current-build `CUnitLeaderTrait` / portrait-feed revalidation, with assignment/role closure and full speed-five proof still open. |
| Scientist characters | Scientist roster, skill/profile rows. | `PRODUCTION_SLICE_DONE` | `scientists_live` under 41. | Treat as scientist slice only; broader character relationships remain domain 47. |

### 5. Focus, Research, Decisions, And Timed Country Actions

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Current focus | Focus id, progress, tree id. | `PRODUCTION_SLICE_DONE` | `focus_live` / 12. | D12 owns first-class `focus.current`-style fields; D45 may fill them only when absent and mirrors its values under R2 depth keys. Keep joined to focus static definitions. |
| Completed focuses | Completed count and tokens where encoded. | `PRODUCTION_SLICE_DONE` | `focus_live`, domain 45 exact-completed slice. | Ensure direct completed-token closure for all cases. |
| Continuous focus | Current continuous focus state. | `PRODUCTION_SLICE_DONE` | `focus_live`, domain 45. | Audit mutual exclusivity with normal focus. |
| Focus paused state | Whether focus progress is paused. | `FIXTURE_BLOCKED` | Domain 45 pause probes rejected candidates. | Same-build paused-true oracle or proof-backed rejection. |
| Research slots | Active slot, progress, live progress, base cost, tech handle/name. | `PRODUCTION_SLICE_DONE` | `tech_live` / 13. | Audit effective modifiers and completion events. |
| Completed technologies | Completed tech count/tokens. | `PRODUCTION_SLICE_DONE` | `tech_live`. | Join every token to tech definition. |
| Research ETA/speed | ETA from live deltas and speed fields where encoded. | `PRODUCTION_SLICE_DONE` | `focus_research_decisions_depth_live` / 45. | Close exact engine speed/modifier fields if replay needs them. |
| Active decisions | Decision status, timed decisions, counts. | `PRODUCTION_SLICE_DONE` | `decisions_live` / 16 and domain 45. | Complete only when target/cooldown/body fields are captured or blocked. |
| Decision targets/body/cooldowns | State/country targets, remaining cooldowns, body detail fields. | `PRODUCTION_SLICE_DONE` | Domain 45 shared country-target subset from current-build 870ebf proof: `CDecisionStatus+0xE8` / vtable `0x27D5038` / `record+0x20` country-target `tag_db_id`, 1061 exact save/live matched rows across 180 decision tokens, folded under `countries[*].decisions._r2_depth_targets`; D45 target rows use an H4CR record-header schema-v2 flag with an `FRD2` payload sanity marker, while legacy v1 headers remain decoder-supported. Numeric state/province targets, cooldown/day/state fields, body layout fields, and unmatched live transition rows remain open/blocked. | Close or reject numeric targets, cooldowns, body fields, and unmatched transitions with exact proof; promote `_r2_depth_targets` only after schema ownership is widened. |
| Direct focus/research/decision events | Start/finish/cancel/complete decision/focus/research hooks. | `ROOT_DISCOVERY_OPEN` | Domain 45 hook candidates rejected/blocked. | Exact function/hook contract or proof-backed no-go. |

### 6. Special Projects, Scientists, And Facilities

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Facilities | Facility rows and currently worked projects. | `PRODUCTION_SLICE_DONE` | `projects_live` / 41. | Audit facility type/slot/location joins. |
| Scientists | Scientist char id, skill/profile rows, country roster route. | `PRODUCTION_SLICE_DONE` | `scientists_live` under 41. | Join to full character/portrait/role data under domain 47. |
| Project progress | Progress/state fields where validated. | `PRODUCTION_SLICE_DONE` | `projects_live` / 41. | Full state-machine semantic naming remains open. |
| Project completed bit | Completed-bit slice where validated. | `PRODUCTION_SLICE_DONE` | Project progress lane. | Completion lifecycle event semantics still open. |
| Project start/completion lifecycle | Started, completed, cancelled, repeatable state, history. | `FIXTURE_BLOCKED` | Project fixture docs. | Controlled before/after project fixture and shared compact rows. |
| Rewards/reward history | Reward object, selected reward, fired reward history. | `FIXTURE_BLOCKED` | Project reward fixture docs. | Before/after reward-option fixture and semantic root. |
| Breakthrough state | Breakthrough compact state and deltas. | `LANE_LOCAL_PROVED` | Project progress lane. | Shared integration and semantic specialization names where possible. |

### 7. Divisions, Army, Orders, OOB, And Land Combat

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Division root discovery | Stable anchored DLL-native division discovery. | `PRODUCTION_COMPLETE` | `divisions_live` / 20. | Keep per-build rediscovery/validation docs current. |
| Division state | Id, owner, location, org, strength, XP, supply, fuel, movement, combat ref, deletes. | `PRODUCTION_SLICE_DONE` | `divisions_live` / 20. | Add exact manpower and assignment/detail rows below. |
| Division template id | Template id joined from live division. | `PRODUCTION_SLICE_DONE` | `divisions_live` widened contract. | Full template catalog required separately. |
| Division template catalog | Full template composition, support companies, battalions, equipment requirements. | `SHARED_INTEGRATION_PENDING` | Domain 59 strict vanilla+mod static catalog proof passed 2026-07-02: 306 division templates, 1180 unit definitions, 59 equipment definitions, `.battleplan` byte-verified with zero warnings. | Integrate domain 59 into production replay packing and join catalog ids to live division/template rows; live-created/edited templates remain a live-capture concern under divisions. |
| Division equipment | Held equipment entries. | `PRODUCTION_SLICE_DONE` | `division_equipment_live` / 26. | Folded only by unique owner/division id join to D20 because D26 rows lack `div_type`; audit exact shortages/reinforcement/upgrade history if replay needs it. |
| Division manpower | Current manpower and losses. | `ROOT_DISCOVERY_OPEN` | Division docs list exact manpower as future work. | Pin field/root and compact rows. |
| Division assignment/order linkage | Army, order, front/battleplan assignment, strategic redeploy state. | `ROOT_DISCOVERY_OPEN` | `army_orders_live`, domain 54. | Full order geometry and assignment rows. |
| Active army orders | Order identity, generation, slot, type, deletes, type-5 target-state slices. | `PRODUCTION_SLICE_DONE` | `army_orders_live` / 27. | Non-type-5 geometry and redraw/toggle fixtures remain. |
| Battleplan geometry | Frontline/offensive line/fallback/garrison geometry, redraws, activation/execution state. | `SHARED_INTEGRATION_PENDING` | `army_battleplans_depth_live` / 54; shared wiring covers exact type-5 target states and exact non-type-5 `order+0x70/+0x78` path rows from the live-`870ebf` save/live paired slice (`army_battleplans_depth_live_save_geometry_870ebf_20260701_v1`); the old type-3 `order+0xC0/+0xC8` path candidate is rejected. | Pass shared DLL/Rust/.battleplan proof; close or exactly reject direct roots, redraw lifecycle, activation/execution, stance/aggressiveness, and non-path geometry semantics. |
| Army OOB | Active armies, groups, leaders, membership, owner rows. | `PRODUCTION_SLICE_DONE` | `oob_command_live` / 28. | Empty armies, theatres, inactive commands, full manager roots. |
| Land combat lifecycle | Active battle rows, participants, sides, combat state. | `PRODUCTION_SLICE_DONE` | `land_combats_live` / 21; active rows are folded into REPLAY_SCHEMA §7 `combats[]` snapshots. | Audit tactics, reserves, width, casualties, terrain, modifiers if not encoded. |
| Land combat detailed events | Reinforce/retreat/overrun/casualty/equipment loss/phase events. | `SHARED_INTEGRATION_PENDING` | Domain 55 lane-local compact/Rust/.battleplan proof covers phase started/updated/ended, participant joined, and participant left-unclassified records from domain-21-shaped `CLandCombat` snapshots; shared collector/recorder registration and world-fold enrichment are wired. | Pass shared DLL/Rust/.battleplan speed-five proof, then capture or exactly reject tactics, reserves, width, casualty, equipment-loss, retreat, overrun, terrain, and modifier fields. |

### 8. Navy

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Ship rows | Ship id, task force, owner, definition/category, strength/org/XP, sunk convoys, deletes. | `PRODUCTION_SLICE_DONE` | `ships_live` / 29. | Direct ship names, modules, repair state, mission/home base rows below. |
| Fleet/task-force hierarchy | Fleets, task forces, membership, task-force province ids where encoded. | `PRODUCTION_SLICE_DONE` | `oob_command_live`, `ships_live`. | Empty/inactive fleets and full command roots. |
| Ship definitions/modules | Hull, variant, modules, stats, names. | `PRODUCTION_SLICE_DONE` | Static definitions shipped 2026-07-03: `parse_ship_definitions.py` (80 hulls / 116 modules, content/token-based discovery, module selection by ship-slot category, `ship_definitions_gate.py` green, commit `0faf7e88c`); production packing shipped via `recorder_host --static-ship-definitions` (hashed `role="static_artifact"` `ship_definitions_static`, fail-closed shape validation, whitelisted provenance, production-required). Live ship state stays domain 29; join key `order_of_battle.ships[*].definition` ↔ `ship_hulls.<token>`. | Live per-ship VARIANT names/stats (design-level) are not in the static catalog — direct live variant/name extraction remains open. |
| Naval missions | Patrol/strike/convoy/repair/training/hold, region assignment, mission state. | `FIXTURE_BLOCKED` | Domain 56 shared integration pending for current task-force location/region and aggregate sunk-convoy counters only; mission enum/state semantics are not promoted. | Controlled mission-change fixture, then compact mission rows or exact rejection. |
| Home base/repair/retreat | Port, home base, repair queue, retreat state. | `FIXTURE_BLOCKED` | Domain 56 validates current location only; `CTaskForce+0x18` is not home base. | Controlled port-change, damaged-repairing, and retreat fixtures, then shared compact rows or exact rejection. |
| Naval combat lifecycle | Start/update/end battle events, regions, sides. | `SHARED_INTEGRATION_PENDING` | `naval_combat_lifecycle_live` / 49; lane live gate passed 2026-07-03 with promoted `location_id`/`region_id` (4/4 D04 save-oracle match). | Orchestrator serial-integration into the shared collector, lifecycle-end fixture, speed-five proof. |
| Naval combat members | Ship/task-force members, join/leave, damage/sinking linkage. | `SHARED_INTEGRATION_PENDING` | `naval_combat_members_live` / 50; lane live gate passed 2026-07-03 via the CFEXGroup back-ref walk (117/117 members, 101/101 CShip joins vs the D04 save oracle). | Orchestrator serial-integration into the shared collector, sinking/member-change fixture, speed-five proof. |
| Convoy combat/losses | Convoy involvement, losses, route/mission context. | `FIXTURE_BLOCKED` | Domain 56 carries aggregate `CShip+0x3C` sunk-convoy counters only. | Active convoy battle/loss route fixture, then compact convoy-event rows or exact rejection. |

### 9. Air

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Air wings | Wing identity, owner, size, mission, and region where encoded; base/equipment annotations come from `air_wing_loadouts_live` / 32. | `PRODUCTION_SLICE_DONE` | `air_wings_live` / 30 plus D32 annotations. | Audit create/delete/rebase and mission lifecycle. |
| Air regions/activity | Region activity counters and mission activity. | `PRODUCTION_SLICE_DONE` | `air_region_activity_live` / 31. | Audit every UI-visible activity/loss/sortie counter. |
| Air wing loadouts | Equipment/variant loadouts. | `PRODUCTION_SLICE_DONE` | `air_wing_loadouts_live` / 32. | Join to variant definition/stats. |
| Air bases | Base status/capacity/occupancy where encoded. | `PRODUCTION_SLICE_DONE` | `air_base_status_live` / 34. | Audit damage/repair/overcrowding if not captured. |
| Aircraft variants | Defs, stats, reserves, upgrades, archetypes. | `PRODUCTION_SLICE_DONE` | Domains 33, 35, 36, 39, 46. | Close rich role/base-airframe strings and fallback tokens. |
| Air superiority inputs | Superiority input rows. | `PRODUCTION_SLICE_DONE` | `air_superiority_inputs` / 38. | Audit exact UI superiority output/modifiers. |
| Air combat losses/reinforcements | Loss, reinforcement, delete deltas, mission region, equipment amount deltas. | `PRODUCTION_SLICE_DONE` | `air_combat_losses_live` / 42. | R2 sidecar only; recovered post-failure-gap deltas may aggregate changes across the gap, so full exact per-hour air combat rows remain open. |
| Sortie/combat lifecycle | Sortie start/end, combat pairings, tactical engagement state. | `ROOT_DISCOVERY_OPEN` | Domain 42 residual probes rejected region/base scalar helpers `0xC44FA0`, `0xC448F0`, `0xE0CD40`, `0xE0CF00`, and `0xF71050` as event sources; full lifecycle root/hook remains open. | Deeper instrumentation/root or proof-backed rejection. |
| Kill/cause attribution | Who killed what, loss reason/source, AA/accident/combat cause. | `ROOT_DISCOVERY_OPEN` | Domain 42 residual probes rejected the `0xF51060` subpath through `0xF51A60` and `0x1004BF0/0x1004C40/0x1004C50/0x1004C80/0x1004E70` helper family as attribution sources. | Production attribution rows or exact blocker. |
| Ace identity/assignment | Ace roster, wing assignment, participation, death/wounding. | `FIXTURE_BLOCKED` | Air/ace lane-local roster proof exists; domain 42 narrowed ace-management command `0x2FB5` at `0xC52840` through dispatch block `0x6FA8AC` / callsite `0x6FA8BA`, with `rcx` resolved as selected-country strategic-air container via `GameState+0x690`/`0x6C2D80`; `rdx=r14` payload remains unvalidated. | Shared roster registration and speed-five proof, plus controlled command `0x2FB5` ace assignment/loss fixture for assignment, participation, death, and wounding semantics. |
| Reinforcement reason | Production reinforcement, transfer, attrition replacement, deleted-wing semantics. | `ROOT_DISCOVERY_OPEN` | Loss delta slice only; the same domain 42 helper-family rejection removes one candidate route for reinforcement/cause semantics. | Root or event hook. |

### 10. Map, Weather, Construction, Damage, Occupation

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| State ownership/control | Owner/controller per state. | `PRODUCTION_SLICE_DONE` | `map_control_live` / 22. | Audit civil-war/exile/occupation edge cases. |
| Province controller/state | Controller and state link per province. | `PRODUCTION_SLICE_DONE` | `map_control_live` / 22. | Audit impassable/water/special provinces. |
| Buildings | State/province building levels. | `PRODUCTION_SLICE_DONE` | `map_detail_live` / 23. | Domain 48 covers damage/repair/target state next. |
| Resources/deposits | State resource deposits. | `PRODUCTION_SLICE_DONE` | `map_detail_live` / 23. | Join to trade/economy resource flows. |
| Construction/building damage | Construction target state, building damage/repair/health. | `SHARED_INTEGRATION_PENDING` | `construction_buildings_state_damage_live` / 48 shared compact slice. | R2-projected sidecars cover construction queue, all-or-none target state, and numeric building/resource health rows; live speed-five proof plus token/name, created-date, exact target-location, repair semantics, and speed closure remain. |
| Resistance/compliance/occupation | Occupation/resistance/compliance rows. | `PRODUCTION_SLICE_DONE` | `occupation_resistance_compliance_live` / 52. | Garrison scalar/collaboration detail remains open. |
| Garrisons | Garrison law/template/manpower/equipment/suppression details. | `ROOT_DISCOVERY_OPEN` | Occupation residuals. | Production root or exact blocker. |
| Weather visual drivers | Region weather, province displayed temp/change/water/snow/mud, recolor/LUT, sandstorm. | `PRODUCTION_SLICE_DONE` | `weather_live` / 24. | Projected under `map._r2_weather_live` as an R2-documented additive key pending schema-owner acceptance; close enum/particle parity and non-visual internals where replay-relevant. |
| Weather causal internals | Internal weather-system cause/effect, unobserved enums, close-zoom particle/entity state. | `ROOT_DISCOVERY_OPEN` | Weather residuals. | Extract, reject as non-replay-relevant, or fixture-block with proof. |
| Terrain/map rendering inputs | Terrain, water, map modes, borders, supply overlays, battle indicators. | `NOT_STARTED_OR_UNAUDITED` | UI/map agent owns presentation separately. | Extraction must expose every state needed by replay renderer. |

### 11. Supply, Logistics, And Attrition

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Province logistics/buildings | Rails, hubs, naval hubs, infrastructure-derived rows. | `PRODUCTION_SLICE_DONE` | `supply_logistics_attrition_live` / 44. | Keep joined to map/building domains. |
| Division supply/fuel proxy | Current/max/gain supply, deficit, fuel proxy. | `PRODUCTION_SLICE_DONE` | Domain 44. | Exact engine tooltip supply below remains open. |
| `CSupplySystem` province inputs | Province input array rows. | `FIXTURE_BLOCKED` | Shared integrated via anchored `CGameState+0x3D8`; `supply_csupply_directroot_shared_20260701T195744` decoded 13,396 rows with no failure flags and `read_time_us=76065`, but the game clock did not advance. | Rerun the shared compact gate with the game actually advancing at speed five/current equivalent and require zero skipped hours. |
| Exact province supply tooltip | Remaining supply amount and UI tooltip parity. | `ROOT_DISCOVERY_OPEN` | Supply residuals. | Root/probe that matches UI/save oracle or exact blocker. |
| Hub motorization | Motorization level/settings/effects per hub. | `FIXTURE_BLOCKED` | Domain 44 validated `CCountrySupplySystem+0x78 -> CCountry` and `CCountrySupplySystem+0x20` `0x30`-stride country/node/tag records; `ccountry_node_records_870ebf_20260701_v1` rejected record `+0x04` as exact level (`21` matches, `96` mismatches), and `hub_motorization_level_870ebf_20260701_v1` proved the save oracle is single-level only (`490` pairs, all level `2`; best live candidate `u8@record+0x2D` mismatched `20/117`). | Same-frame hub toggle fixture with at least two known levels, ideally one level `1` and one level `3`, then capture or reject the exact level field/function output through DLL compact rows. |
| Convoy/logistics pressure | Convoy supply, port pressure, foreign/homebase pressure. | `ROOT_DISCOVERY_OPEN` | Domain 44 `foreign_pressure_records_870ebf_20260701_v1` validates a foreign-homebase candidate at `CCountrySupplySystem+0x1E0`: `0x70`-stride records with province `+0x00`, supply `i32/100000@+0x08`, penalty `i32/100000@+0x18`, duplicate province `+0x50`; stale oracle matched `25/25` joined provinces and `20` exact raw supply+penalty pairs. | Same-frame foreign-homebase/convoy pressure fixture, plus roots for convoy/port/disrupted pressure; current save has zero disrupted-supply anchors and tag `D04` does not join live. |
| Attrition reason/flag | Attrition active state, reason/source, affected unit/province. | `ROOT_DISCOVERY_OPEN` | Supply residuals. | Capture exact reason rows or proof-backed blocker. |

### 12. Diplomacy, Wars, Factions, And Intelligence

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Base diplomacy state | Base country diplomacy rows as currently encoded. | `PRODUCTION_SLICE_DONE` | `diplomacy_live` / 17. | Audit every UI-visible relation/diplomacy field. |
| Relations/opinions | Opinion, tension, guarantees, access, non-aggression, attaches, faction relations. | `VALIDATED_SLICE_OPEN_GAPS` | Domain 53 re-enabled 2026-07-03 on live-870ebf re-validated roots (manager 0xF88 / table 0x08 / opinion 0x300, manager_ok=435, opinions bounded ±100; relation-flag vtables gated by exact current-build vtable equality, RTTI-derived). Structural-only constants run behind per-frame runtime gates. | Exact rejections for every remaining UI/save-visible relation field; attache/licence-production flag rows had no live instances in the fixture (vtable RTTI-derived only). |
| Factions | Faction membership, leader, name, joins/leaves, faction diplomacy. | `VALIDATED_SLICE_OPEN_GAPS` | Domain 53 faction-group ids (CCountry+0x598) emitted behind a multi-condition runtime gate (bounded reads, >=40 countries, >=1 nonzero + >=1 SHARED nonzero group; else no rows + `STATUS_FACTION_GROUP_UNVALIDATED`). Membership derives from shared group ids. | Faction leader/name semantics and join/leave event timeline still open — needs a faction-change fixture or static join. |
| Subjects/autonomy | Puppet/subject relation, autonomy progress, overlord, subject levels. | `VALIDATED_SLICE_OPEN_GAPS` | Domain 53 subject rows re-enabled 2026-07-03: subject relation vtable 0x2949F18 (74 live hits), overlord/subject/state-db/ratio fields behind sanity gates, on live-870ebf re-validated roots. | Subject level/static joins and autonomy-change event semantics. |
| War lifecycle | War ids/names, attacker/defender groups, start/end, participants, casualties. | `ACTIVE_INTEGRATION` | Domain 53 compact/offline war rows require lifecycle-unvalidated badging; the previous heap-scan live path is not production-valid, and pinned current-build war roots must be revalidated. | Live root re-validation plus war grouping/name/date closure or exact blocker. |
| Capitulation/peace/conferences | Capitulation state, surrender progress, peace conference state, treaty results. | `NOT_STARTED_OR_UNAUDITED` | Not accepted as production slice. | Full inventory and roots. |
| Intel agency | Agency scalars, upgrades, cryptology, capacity, recruitment. | `ACTIVE_INTEGRATION` | Domain 53 compact/offline slice covers agency scalars, cryptology rows, and probe-backed capacity slots `agency+0xE4` max operatives / `agency+0xF0` usable slots; `+0xF4` is not treated as validated. | Live root re-validation plus semantic fixtures for exact capacity labels, upgrades, recruitment, and cryptology state. |
| Operatives | Operative identity, owner, skill/XP, detected state, lifecycle flags. | `ACTIVE_INTEGRATION` | Domain 53 compact/offline operative rows require revalidation of the exact current-build discriminators before live publication. | Live root re-validation plus mission/captured/pool/nationality/portrait/trait/operation-assignment closure. |
| Operations | Running operation, target, stage/progress, participants, outcome. | `FIXTURE_BLOCKED` | Domain 53 rejected current operation-definition table candidates; no operation-instance root is accepted. | Same-tick running-operation fixture and shared rows, or exact rejection. |
| Spy networks/intel map | Network strength, coverage, intel percentages, target states/countries. | `FIXTURE_BLOCKED` | Domain 53 state-event rows do not cover graph/percentage spy-network state; no production graph root is accepted. | Same-tick intel-map fixture and shared rows. |
| Diplomacy action queues | Incoming/outgoing diplomatic actions and timed requests. | `FIXTURE_BLOCKED` | Domain 53 has no accepted pending-action queue root. | Live fixture and compact event rows. |

### 13. Scripted State, Events, Variables, And Runtime-Only State

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Country/global/state flags | Token/date/value flags where currently encoded. | `PRODUCTION_SLICE_DONE` | `scripted_state_flags_variables_live` / 51. | Folded with opaque-root statuses preserved as R2 provenance; opaque roots below remain open. |
| Variables | Country/global/state variables where currently encoded. | `PRODUCTION_SLICE_DONE` | Domain 51. | Folded into `world`, country `extra`, and state `_r2_scripted_state` leaves; audit all scopes and dynamic variable types. |
| Scripted GUI open/state | Scripted GUI runtime state, open panels, selected scopes if replay-relevant. | `FIXTURE_BLOCKED` | Scripted-state residual fixture contracts. | Controlled fixture or exact rejection as non-replay-relevant. |
| Delayed events/event queues | Scheduled delayed events, pending country/state/global events. | `FIXTURE_BLOCKED` | Scripted-state residual fixture contracts. | Controlled delayed-event fixture and compact rows. |
| On-action/effect runtime state | Runtime-only transition events that alter replay-visible state. | `ROOT_DISCOVERY_OPEN` | No broad accepted event hook domain. | Hook/root audit or proof-backed rejection. |
| Mission/decision scripted state | Runtime state behind missions/decisions not visible in base rows. | `ROOT_DISCOVERY_OPEN` | Decision residuals. | Capture or reject field by field. |
| Mod-added dynamic state | Any mod/scripted variable/flag that changes replay-visible behavior. | `NOT_STARTED_OR_UNAUDITED` | Private current vanilla focus. | Build mod-aware inventory and generic compact encoding rules. |

### 14. Player, Multiplayer, And Session Metadata

| System / field group | Required for 1:1 replay | Current status | Evidence / domain | Remaining closure condition |
| --- | --- | --- | --- | --- |
| Player-country mapping | Which human controlled which country at each time. | `VALIDATED_PARTIAL_SLICE_R4` | Domain 57 shared collector/recorder/fold now emits validated human->TAG rows from the 2026-07-02 live-870ebf passive probe chain (`CGameState` count, human byte array, handle slot index, TAG records), folded under `_r2_session_multiplayer_metadata_live.players_by_country` with a normalized `human_marker` only. | Keep MP-only lobby fields and raw `+0x388` status-code semantics unpopulated until an MP fixture validates them; pass broader shared speed-five proof before declaring the whole session category complete. |
| Hotjoin/tag switches | Player join/leave/tag-switch timeline. | `FIXTURE_BLOCKED` | Domain 57 lane-local hotjoin/tag-switch event schema and replay proof exist (`Hotjoin=1`, `TagSwitch=2`, `SOURCE_FLAG_COMMAND_HOOK_VALIDATED`); the `CGameLobby`/`CPlayerLobby` machinery is MP-only and cannot be validated from an SP save. Disposition 2026-07-03: `SESSION_EVENT_DISPOSITION.md`. | Same-build MP session (2+ players) with a controlled join/hotjoin and a tag-switch/player-country reassignment; validate session-slot/player-array deltas. |
| Game speed/pause commands | Speed changes and pauses for playback timing. | `PARTIAL:PAUSE_VALIDATED_SPEED_OPEN` | Domain 57 shared path emits pause state from the validated 2026-07-02 clock singleton/vtable/bool oracle under `_r2_session_multiplayer_metadata_live.pause`. | Game-speed root remains unpinned and is not folded; validate a live speed root/hook before exposing speed/session timing fields. |
| Chat/messages | Multiplayer chat, system messages, notifications. | `FIXTURE_BLOCKED` | Domain 57 lane-local chat/system-message schema and replay proof exist; chat is MP-runtime and cannot be validated from an SP save. Disposition 2026-07-03: `SESSION_EVENT_DISPOSITION.md`. | Same-build MP session with unique chat text, `--expect-text` oracle, and a stable pollable message container with count/text delta. |
| Alerts/notifications | UI alerts, important message log, popups. | `REJECTED` | Proof-backed rejection for v1 polling extraction 2026-07-03 (`SESSION_EVENT_DISPOSITION.md`): with active wartime alerts on screen, broad current-build RTTI found alert/message/notification classes but ZERO live heap instances, and a reachability walk from validated `CGameState`/human `CCountry`/child roots found no pollable container; RTTI shows an `NInterfaceMessages` listener/dispatch architecture, not stored state. Alerts are derived UI state reconstructable in the replay from underlying facts (domains 14/16/17/21/45/55). | Not required for 1:1 replay (derive alert-equivalents from replay facts). Future revisit only via code-hook instrumentation of the `NInterfaceMessages` dispatch path — outside the polling architecture, needs explicit sign-off. |

## Closure Sweep Required Before Calling The Extractor Done

Before the recording layer can be called complete, run these sweeps and update
every affected row above:

1. Save-visible parity sweep:
   compare every save-visible field relevant to replay against either a compact
   production row or a proof-backed blocker/rejection.
2. UI-visible parity sweep:
   walk every major the game ledger, map mode, tooltip, combat panel, production
   screen, diplomacy screen, intel screen, navy/air/army screen, and project UI;
   every displayed replay-relevant datum must map to a row here.
3. Runtime-event sweep:
   identify state transitions that are not visible from hourly snapshots alone
   and require sub-hour event rows.
4. Static-definition sweep:
   every token/id emitted by compact records must have a deterministic
   localisation/definition/asset join or an explicit omitted-field reason.
5. Fixture backlog sweep:
   every `FIXTURE_BLOCKED` row must name the exact fixture needed and the
   command/gate that will accept or reject it.
6. Rejection ledger sweep:
   every `REJECTED_NO_RETRY` candidate must be listed with offset/function/root
   evidence so it is not rediscovered by another lane.
7. Combined speed-five proof:
   schedule every production domain that is enabled for the current build,
   require hourly-or-better coverage, zero skipped hours, no untolerated failure
   flags, Rust decode, and `.battleplan` verification.

## Done Means

The extraction app is done only when:

- every row in `Master Gameplay Checklist` is `CONCEPT_COMPLETE`, or
- the row is explicitly out of scope with proof and that out-of-scope decision
  is accepted in this document, and
- the final combined speed-five proof passes with all enabled production domains,
  zero skipped game hours, no untolerated source/partial/worker/bridge failures,
  Rust decode, and `.battleplan` verification.
