# Replay Native Compact Domain Registry

This is the orchestrator-owned source of truth for compact domain IDs. Lane
subagents must not choose IDs independently. If a lane needs a new domain, the
orchestrator assigns it here first.

For scope-sensitive status, read `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`
alongside this registry. The registry status is a compact label; the master
contract lists what each validated slice actually covers and what remains open
for full replay fidelity.

Registration rule: lane implementation lives in lane-owned collector/decoder
modules. Shared `collector/src/lib.rs` and `recorder_host/src/main.rs` changes
should be registration hooks only unless the orchestrator explicitly creates or
migrates the module interface.

Status vocabulary:

- `validated slice` means the named scope passed the DLL/compact/Rust replay
  gate at hourly-or-better cadence. It does not mean the whole gameplay concept
  is captured.
- `concept complete` is reserved for full replay coverage of the concept: every
  replay-relevant extractable field is captured, or each remaining field has an
  exact blocker or conclusive rejection.
- Open walls stay in the status label. Do not compress them back to
  `production-complete`.

## Active ID Map

| ID | Domain | Status | Owner lane |
| --- | --- | --- | --- |
| 10 | `fuel_live` | existing production compact domain | country economy |
| 11 | `production_live` | existing production compact domain | country economy |
| 12 | `focus_live` | existing base compact domain | focus base |
| 13 | `tech_live` | existing base compact domain | research base |
| 14 | `politics_live` | existing base compact domain | politics base |
| 15 | `politics_residuals` | existing base compact domain | politics base |
| 16 | `decisions_live` | existing base compact domain | decisions base |
| 17 | `diplomacy_live` | existing base compact domain | diplomacy base |
| 18 | `country_identity` | active integration: per-frame `tag_db_id` -> TAG map for world_fold country re-keying; cosmetic_tag/color omitted | country identity |
| 20 | `divisions_live` | existing compact domain; E4/D20 row-growth (2026-07-05, commit 08f7fa137): +strategic_redeployment (`CDivision+0x29F`), current_manpower (`+0x3D0→u32@+0x4`), cumulative_losses (`+0x668`) — row now 104B with presence bits, absent stays null; D04 live-vs-melt 220/220 SR, 2359/2359 manpower, losses non-negative 0..131347 | divisions |
| 21 | `land_combats_live` | existing compact domain; folded into REPLAY_SCHEMA §7 `combats[]` snapshots | land combat |
| 22 | `map_control_live` | existing compact domain | map control |
| 23 | `map_detail_live` | existing compact domain; folded into REPLAY_SCHEMA section 4 state resources/buildings/province_buildings plus R2 `map.map_detail_live` mirror | map detail |
| 24 | `weather_live` | validated slice: weather visual drivers plus sandstorm flag; projected under `map._r2_weather_live` as an R2-documented additive key pending schema-owner acceptance; enum/particle parity and internals open | weather visual drivers |
| 26 | `division_equipment_live` | existing compact domain; folded into `divisions[*].equipment` only when owner/division id joins uniquely to D20, with unmatched/ambiguous rows dropped and counted in provenance | divisions |
| 27 | `army_orders_live` | existing compact domain; projected under `order_of_battle.battle_plan_orders` as an R2-documented recorder shape pending schema-owner acceptance | army orders |
| 28 | `oob_command_live` | existing compact domain; projected under section 12 `order_of_battle` command/fleet arrays with `_r2_oob_command_capture`, pending schema-owner acceptance of the live raw shape | army OOB |
| 29 | `ships_live` | existing compact domain; projected under `order_of_battle.ships` and task-force ship lists as an R2-documented recorder shape pending schema-owner acceptance. E12 fix (2026-07-05, commit d667480b1): corrected D28/D29 task-force UNDER-reporting (expand direct CFleet subobject candidates + accumulate all valid CTaskForce body deltas); D04 1690/1690 active ships, 206/206 task forces, 0 mismatches | naval OOB |
| 30 | `air_wings_live` | existing compact domain; D30 owns section 12 `order_of_battle.air_wings` existence with `_r2_air_wing_capture` evidence | air wings |
| 31 | `air_region_activity_live` | existing compact domain; folded into REPLAY_SCHEMA sections 13.1/14 `_r2_air_region_activity` | air activity |
| 32 | `air_wing_loadouts_live` | existing compact domain; attaches section 12 air-wing loadout/equipment annotations only to D30-owned air-wing rows, with orphan rows dropped and noted as R2 provenance | air wings |
| 33 | `air_variant_defs` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live` definitions | air variants |
| 34 | `air_base_status_live` | existing compact domain; folded into REPLAY_SCHEMA section 14 `strategic_air.air_bases` | air bases |
| 35 | `air_variant_stats` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live.live_stats` | air variants |
| 36 | `air_variant_reserves` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs._r2_air_variant_reserves` | air reserves |
| 37 | `stockpile_variant_defs` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs._r2_stockpile_variant_defs` | stockpile |
| 38 | `air_superiority_inputs` | existing compact domain; folded into REPLAY_SCHEMA sections 13.1/14 `_r2_air_superiority_inputs` | air superiority |
| 39 | `air_variant_upgrades` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live.upgrade_levels` | air variants |
| 40 | `stockpile_variant_upgrades` | existing compact domain; folded into REPLAY_SCHEMA section 11 `designs._r2_stockpile_variant_upgrades` | stockpile |
| 41 | `projects_live` | partial validated slice: facilities/scientists/progress/completed bit folded into top-level `special_projects`; raw unresolved handles preserved under `_r2_*`; start/reward fixture blocked | projects |
| 42 | `air_combat_losses_live` | validated slice: loss/reinforcement delta; direct event root rejected; folded only into REPLAY_SCHEMA sections 13.1/14 `_r2_air_combat_losses_live`; not full air combat | air combat losses |
| 43 | `market_trade_live` | validated slice: purchase contracts plus MTD1 purchase-contract detail trailer (price/amount/speed/completed/days/delivery country/convoys/efficiency/progress); folded into REPLAY_SCHEMA section 7b `diplomacy_world.trade_contracts`, per-country `diplomacy.lend_lease`, and `r2_market_trade_live`; omitted candidates/full trade economy, MTLL, market stockpile, non-market contracts, and route/path semantics remain open. MTRF exported resource-flow trailer (2026-07-05, commit 3211e5fcd): `CCountryResources=u64[CCountry+0x11F8]`→exported pool `+0x5B0` (`/1e5`, oil/aluminium/rubber/tungsten/steel/chromium/coal); 95/95 oracle countries, 665/665 exported fields, 0 mismatches (orchestrator re-verified 436 countries/3052 fields). Sibling pools produced/imported/used/to_export/base_export OMITTED — value-validation failed, stay DERIVED/OPEN | market/trade |
| 44 | `supply_logistics_attrition_live` | validated slice: supply/logistics subset folded into REPLAY_SCHEMA section 13.2 `supply.by_province` with `r2_logistics_attrition` badges; speed-five proof and exact province supply wall open | supply/logistics |
| 45 | `focus_research_decisions_depth_live` | validated slice: transition depth plus active targeted-timed country-target rows in D45 schema-v2 records selected by the H4CR record-header schema flag and sanity-checked with `FRD2`, folded as R2 focus/research/decision annotations and `countries[*].decisions._r2_depth_targets`; legacy v1 headers remain decoder-supported; D12 owns first-class focus fields, with D45 providing only an absent-only fallback for `focus.current`-style fields; numeric state/province targets, cooldowns, body fields, unmatched transitions, paused focus, and direct event hooks remain open/blocked | focus/research/decisions depth |
| 46 | `air_variant_archetypes_live` | validated slice: variant archetypes; folded into REPLAY_SCHEMA section 11 `designs.equipment_variants_live` archetype/R2 metadata; token `20378` resolved; wave-4 ace-lifecycle slice: `ACE1` ace-roster trailer on the same data record (identity + wing assignment + alive/killer death record, full-replace rows) folded onto `order_of_battle.air_wings[*].ace` plus per-country `extra._r2_air_ace_roster_live`; ace kill_type/wounded blocked (no D04 variance); shared token hook/full air lifecycle open | air wings/variants lifecycle |
| 47 | `characters_leaders_cabinet_live` | shared integration pending: registered as a schedulable validated-slice/open-gap compact domain; folds validated live character identity, unit-leader identity/stats, country-leader role edges, and active ruler rows into `countries[*].commanders.leader` plus `_r2_characters_leaders_cabinet_live`; advisor roster, appointed advisor, character portrait, and unit-leader trait-token rows are blocked pending current-build `CAdvisor` / `CUnitLeaderTrait` revalidation; live speed-five proof and broader closure remain open | characters/leaders/cabinet |
| 48 | `construction_buildings_state_damage_live` | shared integration pending: validated offline compact slice for construction queue rows, all-or-none target state, numeric state/province building health rows, and numeric state resources; folded as R2/domain-owned sidecars to avoid D23 state-field ownership collisions; token/name, created date, exact target location, repair semantics/speed, and live proof remain fixture-blocked/open | construction/buildings/state damage |
| 49 | `naval_combat_lifecycle_live` | fixture-blocked: no active battle in current process; current CShip route validated | naval combat |
| 50 | `naval_combat_members_live` | fixture-blocked: no active battle in current process; current CShip route validated | naval combat |
| 51 | `scripted_state_flags_variables_live` | validated lane slice: shared registered and folded into `world`, country `extra`, and state `_r2_scripted_state` leaves; opaque-root statuses remain R2 provenance, not hard failures | scripted state/flags/variables |
| 52 | `occupation_resistance_compliance_live` | validated slice: shared registered occupation state rows folded into REPLAY_SCHEMA section 4 `states[*].occupation`; law/garrison extras retained under `_r2_occupation_resistance_compliance_live`; garrison scalar/collaboration detail open | occupation/resistance/compliance |
| 53 | `diplomacy_intel_operatives_live` | active integration, not production-scheduled: compact/offline contract exists for relation/opinion/subject rows, agency scalars plus probe-backed E4/F0 capacity slots, operative lifecycle rows, cryptology, compact state-event observations, and war tag-pair/casualty rows badged lifecycle-unvalidated, but current-build live root offsets are unvalidated and must not claim 870ebf source proof; operation detail, spy-network/intel-map percentages, incoming action queues, and full war/agency/operative semantics remain fixture-blocked/fail-closed. wave-5 running-operations sidecar (`OPIN` trailer: per-instance {instance_id, def_ref_ok, associated_country badged PROVISIONAL}, from COperationInstance vtable 0x29A76C0 via the existing discovery-cache heap scan; +0x10 owner REJECTED) is BLOCKED-ADDITIVE plumbing — wired, unit-proven, and offset-verified (8/8 targets), but NOT live-emitted on 870ebf because the d53 base extract is fail-closed here; it lights up when the d53 base root source-availability lands. TRAILERS (2026-07-05, commit b1924589b): DIAQ diplomacy-queue trailer (`CCountry+0xFF8` incoming action queue, action vtable `mod+0x2A209A0`; D04 5/5 recipients CZE2/AUS2/LIT1/EST1/LAT1 = 7/7 actions, 0 false positives) + INGR intel-graph trailer (`CStrategicOperative+0x10`→`CCountryIntelNetwork` vt `mod+0x298AB48`→target `+0x08`, graph `+0x18`→vertices `+0x38`/count `+0x40`; D04 66/66 network rows exact target+sorted-state-set multisets). d53 base relations separately VALUE-PROVEN (commit 3170465e4): cached_sum `rel+0x300` 10354/10354, border_friction `rel+0x308/1e5` 44382/44382, 0 mismatches | diplomacy/intel/operatives |
| 54 | `army_battleplans_depth_live` | shared integration pending: schedulable validated slice for active-army order rows, exact type-5 target states, live-`870ebf` save/live paired non-type-5 `order+0x70/+0x78` path geometry, and same-frame assignment/linkage status; direct roots, redraw/toggle lifecycle, activation/execution, stance/aggressiveness, non-path geometry, and full OOB reconstruction remain fixture-blocked/fail-closed | army OOB/orders/battleplans depth |
| 55 | `land_combat_detail_events_live` | shared integration pending: registered in collector/recorder and folded onto active combat snapshots. E2 (2026-07-04) added the per-side CCombatSide DETAIL scalars (schema v2, current-state row family): units(+0x2C)/committed(+0xD4)/reserve(+0xEC), air_kills(+0x140), org_loss_summary_index(+0x1A8), num_org_losses(+0x1AC), and 7 i64×1e5 damage accumulators (anti_air +0x138, air str/org +0x148/+0x150, ground str/org +0x158/+0x160, prevented str/org +0x168/+0x170) — all 50/50 both sides vs the melt (FINAL_SWEEP §T2). Folded under section-7 `combats[*].land_combat_detail_events_live.{attacker,defender}` — ALL-OR-NOTHING per combat (if either side fails its read, NEITHER side is emitted; explicit `side_detail_completeness` badge: both / both_counts_incomplete / attacker_only_suppressed / defender_only_suppressed). Collector DOUBLE-READS the units/committed/reserve triple (both passes must agree) to distinguish a torn read (dropped) from a genuine mid-combat transient (RETAINED + `counts_incomplete` badged, not hard-dropped — combat transitions are the moments worth showing); committed+reserve==units enforced on clean rows collector+decoder, relaxed only under the badge. tactic = PARTIAL (pointer/table-backed, badged null), losses = omitted (all-zero oracle), width/terrain/phase = DERIVED. v1 event streams still decode | land combat details |
| 56 | `naval_missions_status_live` | shared integration pending: registered validated task-force status slice folded as R2 `order_of_battle.extra`; full-replace rows for task_force_id/fleet_id/owner/current province, optional derived region, and aggregate sunk-convoy count; mission/home-base/repair/retreat/route fixtures open. TRAILERS (2026-07-05): NSUP naval-supremacy trailer (commit 684b46d43, closes R6/N2): `CStrategicRegion` (vt `mod+0x29569A8`/`+0x29569F8`)→naval dominance `+0xE8`→`CDominanceValues` (vt `mod+0x296DFC0`, stride `0x50`, dup-echo tag guard, current/target/base_target/previous/decline_from/individual_ratio); D04 47/47 region-controller rows, 282/282 fields exact. Plus D29-derived `ship_count`+`dominant_hull_class` sidecar (commit d667480b1, null when D29 unavailable) | naval missions/status |
| 57 | `session_multiplayer_metadata_live` | validated partial slice (R4): schedulable player-country human TAG rows plus pause state; folded under `_r2_session_multiplayer_metadata_live` with normalized `human_marker` rows only; raw `+0x388` status-code semantics, game-speed root, and MP-only player/session-event fields remain fail-closed/open | session/multiplayer metadata |
| 58 | `country_economy_counters_live` | assigned; no accepted lane output after usage-limit interruption; factory/resource/consumer-goods UI counter closure open | country economy counters |
| 59 | `templates_equipment_catalog_live` | strict vanilla+mod static catalog proof passed (2026-07-02); production replay packing shipped (`recorder_host --static-templates-equipment-catalog`: hashed `role="static_artifact"` entry + `index.json` `static_artifacts` roster, fail-closed absence); live template-id joins open | templates/equipment catalog |
| 60 | `capitulation_war_state_live` | E1 validated slice (2026-07-04): per-country capitulation state (`D=u64[CCountry+0xF88]`: capitulated u8 +0x2E0 orchestrator-verified 15/15 vs melt over 437 countries; capitulated/last-surrender/faction-join dates, legitimacy x1e5, exile army leaders — all 0-mismatch) + global war pool rows {tag pair, start_date_raw @row+0x10 (958/958 melt-exact), casualty accumulators} via its own discovery-cache heap scan (validated-row addresses cached, revalidated per frame, background refresh). Deliberately INDEPENDENT of the fail-closed d53 base extract. Fold: countries[*].capitulation + section 7b `diplomacy_world.wars` (nullable when unknown). Active-war casualty fields badged `STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR` (frozen-war values byte-exact; active wars exceed the save snapshot — semantic caveat, offsets valid). surrender_progress/peace_conference remain DERIVED/absent | capitulation/war state |
| 61 | `air_loss_stats_live` | validated slice (2026-07-05, commit 71d6d89d6): cumulative air-loss statistic-23 record via `CGS→registry+0x690→mgr_array+0x30→mgr+0x158→CLoopHistory[23]` with the YEARLY window `+0x20` (the `+0x10` daily-ring bug was the load-bearing fix), guards `u64[lh]==mod+0x29BBE58` / `u32[lh+0x34]==89` / self-ref `lh+0x28==lh` / cap `window+0x20==5`; emits 6 value-pinned channels (our/enemy fighters/bombers/cas lost + disrupted), raw 89-field record carried but other channels unlabeled. 23/23 countries × 89 = 2047/2047 vs D04 melt, 0 mismatches | air loss stats |
| 62 | `hub_motorization_live` | validated slice (2026-07-05, commit 12109ae92): per-country motorized supply-hub node membership from the persistent `CCountrySupplySystem` settings object (vtable pair `mod+0x298BBF8`/`+0x298BC48`, country `+0x78`, node array `+0x188`/count `+0x194`, per-record province `+0`/state `+4`/motor_map_ptr `+0x10`/map_size `+0x1c`; motorized ⇔ map_size>0). 223/223 rows, 189/189 provinces, 483/483 map entries, 69/69 per-country counts vs D04 melt, 0 mismatches. Node-level membership + map_size only; inner per-tag warlord attribution intentionally unpromoted | hub motorization |
| 1000 | `frame` | compact stream framing/perf records | recorder infrastructure |

## Static Artifacts Packed By Recorder

The production recorder packs pre-built deterministic JSON artifacts as
`role="static_artifact"` entries and records present/absent status in
`index.json`. These W2 `game_defs` artifacts are required alongside the existing
domain-59 templates/equipment catalog and R3 localisation join bundle:

| artifact name | recorder flag | payload path | schema | contents |
| --- | --- | --- | --- | --- |
| `decisions_static_definitions` | `--static-decisions-definitions` | `extractor/game_defs/out/decisions_static/decisions_static_definitions/decision_definitions_catalog.json` | `decisions_static_definitions.static_artifact.v1` | Static decision and category definitions for replay enrichment. |
| `idea_tech_loc_xref_static` | `--static-idea-tech-loc-xref` | `extractor/game_defs/out/idea_tech_loc_xref/idea_tech_loc_xref_static/idea_tech_loc_xref_catalog.json` | `idea_tech_loc_xref_static.static_artifact.v1` | Idea and technology token cross-reference to localisation keys. |
| `ship_definitions_static` | `--static-ship-definitions` | `extractor/game_defs/out/ship_definitions_static/ship_definitions_static/ship_definitions_catalog.json` | `ship_definitions_static.static_artifact.v1` | Ship hull and ship module static definitions. |
| `gfx_icon_asset_manifest_static` | `--static-gfx-icon-manifest` | `extractor/game_defs/out/gfx_icon_manifest/gfx_icon_asset_manifest_static/gfx_icon_manifest_catalog.json` | `gfx_icon_asset_manifest_static.static_artifact.v1` | Sprite/icon/flag asset manifest over vanilla+integrated_dlc+enabled dlc+mod layers with texture hashes; recorder rejects non-gate-passing manifests (unresolved refs, stale allowlist, parse gaps). |
| `localisation_join` | `--static-localisation-join` | `extractor/game_defs/out/localisation_gate/static_localisation_join_r3/localisation_catalog.json` | `static_localisation_join_r3.static_artifact.v1` | English loc key -> {name, raw, source_layer, source_file} mod-over-vanilla map + per-category resolution coverage. E11 (2026-07-04) added `resolved_joins` (schema `static_localisation_join_r3.resolved_joins.v1`): the consumable id -> display-name tables the replay UI needs — `focus`{focus_id:name}, `state`{state_id:name}, `victory_point`{provid:name}, `country`.{`tags`{TAG:name}, `cosmetic_tags`{cosmetic_tag:name}} — mod-over-vanilla, no fabrication (an id whose best loc candidate is absent is OMITTED). Country display contract: `cosmetic_tags[active] else tags[country_tag] else the raw tag`; dynamic (D01..) tags resolve at runtime. Additive/open-object (existing `keys`/`coverage` unchanged; recorder reads only schema/layers + whole-file hash). Live vanilla counts: 10602 focus / 1081 state / 1704 VP / 364 tags / 344 cosmetic. NOTE the full gate's per-category coverage step FAILURE on `common/ideas/SOV.txt` was FIXED 2026-07-05 (commit b7cae4740): the shipped file legitimately omits its final top-level `ideas={` wrapper brace; the strict precheck now tolerates a single-unclosed-root when nested braces are balanced and the key is real (re-raises operator artifacts / unmatched closers). Full active modset now parses clean: ideas 6522, focus 7526, localisation 139950 keys, 0 failures. |

## Lane Acceptance Under This Registry

- A lane may not report a new compact domain ID as free unless this registry
  assigns it.
- A lane-local proof must use its assigned final ID.
- A lane-local packet that still needs major shared collector/recorder
  implementation is not merge-ready.
- Final integration should be registration plus combined proof. If a lane cannot
  reach that shape, it must return an exact blocker or conclusive rejection.
- Recorder production-profile membership is derived from descriptor
  status/schedulability in `compact_contract`; do not maintain hand-copied
  profile lists in runners or docs.
