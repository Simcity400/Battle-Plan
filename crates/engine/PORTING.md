# Engine knowledge index

One row per old registry domain: what format 13 records for it, what is excluded and why, what remains, and where the evidence is. Status and acceptance live in `PLAN.md` section 1 and section 7; this file is the domain-level map. The long per-domain writer assessments written before 2026-09-11 remain in git history (`crates/engine/PORTING.md` at commit `e4a68197b`).

## Sources

- Old tree: `<local-path>` (tag `archive/pre-rewrite-2026-09-09`). Registry: `extractor/native/domain_registry.json`. Line ranges in the table below are relative to that root; most old walks are in `extractor/native/collector/src/attach_log.rs` (`attach_log`) or under `extractor/native/lanes/<lane>/src/` (`lanes/<lane>`).
- Decompiled a729 functions and everything already learned about them: `hoi4re` (source and guide in `tools/re/`). `fn`, `search`, `xrefs`, `class`, `field-users` read; `name`, `note`, `field`, `batch` save findings for the next agent. It reads `C:\hoi4_evidencegent_reunction_index.sqlite3` and writes `knowledge.sqlite3` beside it. RVAs in this file are image-relative (VA = `0x140000000` + RVA).
- Engine constants: `crates/engine/src/<struct>.rs`, one file per engine struct, every constant with a comment naming its decompiled evidence. Walkers live beside the constants; the collector packs are `crates/collector/src/extended/*.rs`; save parsers `crates/installed-content/src/save*.rs`; comparators `crates/recorder/src/oracle*.rs`.

## Rules for a table

1. Read the old walk and the harvested constants, then re-derive every width and meaning from the decompiled writers and serializers. An old reader's width is not evidence; several were wrong (below).
2. Read only at the accepted hourly boundary (full-tick return, RVA `0x1DCEF0`), on the game thread, with guarded reads, no engine calls, no retries, no waits, no cross-hour caches. Refuse rather than guess: a null element in a counted vector, a failed vtable or handle check, a header that changed, or an out-of-range count makes the table `Unavailable`; buffer pressure makes it `Truncated` with the unread count.
3. Assess the writers: place the identified writers relative to the tick return (inside the tick, in a joined job, or a serial command after the tick) and say what is not proved. Then a 200-check double-read window with zero mismatches. Both together are required: a stable double read cannot see a consistently half-updated world.
4. Extend the oracle (text-save parser plus comparator) or print the limitation. Never waive a difference.
5. Measure per-table read cost and bytes per hour in a shared capture (PLAN section 4, 2.1).

## Method facts that hold for every table

- Sample: the completed-tick state at the return of `0x1DCEF0`. Serial commands run afterwards through the session consumer `0x223DDC0` without another hour increment, so a save taken later in the same hour can differ. The clock row records thirteen engine allocation counters (header serializer `1F2A20`); the oracle prints them against the save header and states whether anything was allocated after the sample. It cannot see non-allocating changes.
- Consistency: each table is read twice per hour until 200 matching checks complete per attach; the war matrix has an independent 200-check census.
- Country references stay raw and resolve through `CountryLinks`; physical country slots can be reused for dynamic countries within a campaign, so a slot is meaningful only within its hour.

## Coverage summary

- 92 registry entries: 74 state (including two infrastructure entries), 11 presentation-only (excluded), 7 event-hook (excluded; four are future state candidates needing an independent source).
- Format 13 records 148 tables. Every state entry with a proven old walk has its state recorded, with the remaining fields listed per row below.
- Deliberately not recorded (state, but outside the minimum rows or size-excluded): D11 per-factory `factory_efficiencies` array (150 i64 per line; `CEquipmentProductionLine+0xC0/+0xCC`) and licensor reverse index; D17/D18 claims, cores, major-country and cosmetic names; D20 division experience, supply, fuel, entrenchment and routes; D22 province occupants; D43 purchase equipment and delivery subbuffers, prices, subsidies, meta history; D52/D66 per-state template overrides, occupation aggregate caches, garrison equipment allocations; D53 operation resources, requirements and phase contents, per-modifier opinion lists; D63 convoy and post-mortem counters; D102 viewer decay accumulator, area province membership on the save side, pool boundaries inside a container, the counterpart `CCountry+0x1410` vector; D98 alert categories and priority ordinals (presentation); D57 chat, pause, policy thresholds (defines, not campaign state).
- Never observed nonempty on the test campaign (rows unexercised): VolunteerHosts, TheatreAreas, StationedUnits, ProductionLicenseRecords.
- No save representation (decoded, not compared): RadarCoverage, AirBaseRegistrations, ThreatCountryAggregates, LexicalTokens, DeploymentLines, DeploymentManpower, DeploymentEquipment, ProvinceSupply, CountryResources and the other runtime caches the oracle names.

## Corrections that supersede old readers

Country scalars, division strength and organisation (`CArmy+0x420/+0x428`), resistance, building health, supply values and most fixed-point fields are signed i64; building levels i16; `CUnit+0x1F0` is the location province and `+0x28` a supply cache; `CGameState+0x490` is play time; `CCountry+0x338` is a mobilization ratio; supply province records have stride `0x20`; `CTaskForce` vtables are `0x294C480/0x294C660`; an air variant's `+0x38` is a string length; `CScientist+0x38` is a capacity (`+0x3c` is the count) and skills have a native count at `+0x11c`; `CMilitaryDeployment+0xB8` is the owning line; `0x03324598` is the process-wide invalid reference handle, not a counter; saved intel decimals are the raw i64 lanes over 100,000 and can be negative; a country's relation with itself is ordinary state; the theatre `area` save field is one array; `CSunkShipInfo+0x90` is a `CProvince*`; `CChat` layouts in the old walk were invalid.

## Registry

Columns: id, domain, class, what format 13 records (table names), status and limits, sources (old walk; engine files). "Live 2026-09-11" means a 200-check window with zero mismatches and the oracle result in that pass; earlier accepted tables carry their own pass in PLAN section 7.

| Id | Domain | Class | Recorded as | Status and limits | Sources |
|---|---|---|---|---|---|
| 1 | clock | State (infrastructure) | Clock (hour, play time, load signal, seed, speed, thirteen allocation counters) | Accepted 1.1; counters and speed live 2026-09-11 | attach_log 56247-56268; cgame_state.rs, save_allocation_counters.rs |
| 10 | fuel_live | State | CountryFuel | Live 2026-09-10/11; current and maximum fixed15, gain and cost fixed5 | attach_log 171883-171942, 151339-151364; ccountry_fuel_status.rs |
| 11 | production_live | State | ProductionLines, Stockpiles, ProductionLicenses, ProductionLicenseRecords, DeploymentConveyors, DeploymentLines, DeploymentManpower, DeploymentEquipment | Live 2026-09-11; licence records empty on test campaign; deployment lines, manpower and equipment have no save comparison; factory efficiencies and licensor reverse index not recorded (size, unserialized) | attach_log 172878-173138, 173198-173416, 172048-172376, 151366-151514; cproduction_status.rs, cdeployment_status.rs, cmilitary_deployment*.rs, clicensed_production_status.rs |
| 12 | focus_live | State | Focus, CompletedFocuses | Live 2026-09-10/11; progress i64; completion hooks excluded | attach_log 167363-167495, 152735-152836; cnational_focus_progress.rs |
| 13 | tech_live | State | ResearchTechnologies, ResearchSlots, TechnologyDefinitions | Live 2026-09-10/11; slot speed i64 fixed5; 242k instances per hour is the largest read (7.6 ms after prefetch hints) | attach_log 168103-168395, 168421-168780, 169217-169310; ctechnology_status.rs, ctechnology.rs, cresearch_slot.rs, prefetch.rs |
| 14 | politics_live | State | Politics, PoliticalParties, ActiveIdeas | Live 2026-09-10/11; party popularity i64; content-token classification of ideas | attach_log 170144-170446, 171631-171822; cpolitical_status.rs, cpolitical_party.rs, cidea.rs |
| 15 | politics_residuals | State | Politics (election fields), AdvisorRoles | Live 2026-09-10/11; last election as native hour | attach_log 170747-170862; cpolitical_status.rs, cadvisor.rs |
| 16 | decisions_live | State | DecisionCounts, Decisions | Live 2026-09-10/11; six collections, literal tokens, both raw target words; completion hooks excluded | attach_log 171120-171583; cdecision_status.rs, cdecision.rs, cdecision_cooldown.rs |
| 17 | diplomacy_live | State | WarRelations, Factions, FactionMembers | Accepted 2026-09-10 (7.12); casualties i64; owned-vector walk with independent matrix census | attach_log 167153-167296; cwar_relation.rs, cfaction.rs, cfaction_system.rs |
| 18 | country_identity | State | Countries, CountryLinks | Accepted 1.1/1.4; claims, cores, major and cosmetic names outside minimum rows | attach_log 136920-137162, 137206-137498; ccountry.rs, ccountry_tag.rs |
| 20 | divisions_live | State | Divisions (+ orders_group) | Accepted 2026-09-10 (7.10); experience, supply, fuel, entrenchment, routes outside minimum rows | attach_log 39758-39910, 108106-108213; carmy.rs, cunit.rs |
| 21 | land_combats_live | State | LandCombats | Accepted 2026-09-10 (7.13); save compares locations and participants only; participant lists differ in same-hour saves (open, PLAN 1) | attach_log 41125-41499, 41605-42424; cland_combat.rs, cland_combatant.rs |
| 22 | map_control_live | State | Provinces, States, StateFlags | Provinces/States accepted 1.1/1.4 and 2026-09-10 (7.9). Format-16 StateFlags adds the hourly native demilitarized byte using the shared States census and independent consistency window. Live2026-09-14:200/0 checks,1081-state fresh-save match; first cold/build-contended read p95 0.243ms, not warmed performance acceptance (PLAN7.37). Occupants outside minimum rows | attach_log 131847-131970, 135323-135598; cprovince.rs, cstate.rs |
| 23 | map_detail_live | State | StateBuildings, ProvinceBuildings, StateBaseResources | Live 2026-09-10/11; levels i16, health i64, resources i64 fixed5 | attach_log 132104-132160, 135069-135321, 165026-165254; cbuilding_status.rs, cbuilding.rs, cstate_template.rs |
| 24 | weather_live | State | WeatherLimits, WeatherRegions, WeatherProvinces | Live 2026-09-10/11; i64 fixed5; ceilings and custom mud have save limits | attach_log 134380-134480, 134505-135067; cweather_manager.rs, sweather_per_region.rs, sweather_per_province.rs |
| 26 | division_equipment_live | State | DivisionEquipment | Live 2026-09-10/11; bank at primary `+0x368`, amounts i64 | attach_log 108308-108510, 131376-131725; cequipment_variant_pool.rs |
| 27 | army_orders_live | State | ArmyOrders (with geometry) | Accepted 2026-09-10 (7.13); instance created after the tick can be missing from the same-hour save | attach_log 108512-108647, 43312-43853; lanes/army_battleplans_depth_live orders_group_live.rs 112-352; corders_group.rs, corder_instance.rs |
| 28 | oob_command_live | State | OrdersGroups, OrdersGroupMembers, ArmyGroupMembers, Fleets, TaskForces, Ships | Live 2026-09-11 (orders groups); naval OOB accepted 2026-09-10; group colour, name, border-war operand outside minimum rows | attach_log 109479-110228, 108649-108707, 47212-47291; corders_group.rs, carmy_group.rs, corders_group_member.rs, cfleet.rs, ctask_force.rs |
| 29 | ships_live | State | Ships | Accepted 2026-09-10 (7.13); scalars kept raw i64 | attach_log 110308-110435, 111950-112064; cship.rs |
| 30 | air_wings_live | State | AirWings | Accepted 2026-09-10 (7.13) | attach_log 112209-112241, 128930-129065; cair_wing.rs, cstrategic_air.rs |
| 31 | air_region_activity_live | State | derived off-thread from AirWings and WingEquipment | No separate table; aggregation is a consumer concern | attach_log 112243-112305 |
| 32 | air_wing_loadouts_live | State | WingEquipment | Live 2026-09-10/11 | attach_log 112307-112350, 129067-129240; cair_wing.rs |
| 33 | air_variant_defs | State | VariantDefinitions, VariantModules | Live 2026-09-10/11; registry census at `CGameState+0x708` | attach_log 122980-123080, 130041-130210; cequipment_variant.rs, cequipment_module.rs |
| 34 | air_base_status_live | State | AirBases, AirBaseRegistrations | Live 2026-09-10/11; registrations have no save representation | attach_log 128128-128234, 128409-128928; cair_base.rs, ccountry_air_container.rs |
| 35 | air_variant_stats | State | VariantStats | Live 2026-09-10/11; ten i64 cache values; no save comparison | attach_log 123221-123295, 130638-130701; cequipment_variant.rs |
| 36 | air_variant_reserves | State | derived from Stockpiles | No separate table | attach_log 127510-127618 |
| 37 | stockpile_variant_defs | State | VariantDefinitions | Same registry census as 33 | attach_log 123082-123219 |
| 38 | air_superiority_inputs | State | derived off-thread from AirWings, WingEquipment, VariantStats | No separate table | attach_log 127672-127978 |
| 39 | air_variant_upgrades | State | VariantUpgrades | Live 2026-09-10/11; native token and level | attach_log 124362-124398, 130703-130768; cequipment_upgrade.rs |
| 40 | stockpile_variant_upgrades | State | VariantUpgrades | Same as 39 | attach_log 124400-124483 |
| 41 | projects_live | State | ProjectPrograms, Projects, ProjectHistory, ProjectBreakthroughs, Scientists, ScientistSkills | Live 2026-09-10/11 (projects) and 2026-09-11 (scientists); trait tokens only counted; scientist roster ordinal unsaved | attach_log 124551-124925, 126612-127499, 160000-160608, 163468-164513; nproject_*.rs, cscientist.rs, scientist_roster.rs |
| 42 | air_combat_losses_live | State | WingCumulative, AirRegionHistory, AirRegionDamage | Live 2026-09-10/11; flattened damage lacks its parent ordinal (representation limit) | attach_log 112352-112731, 129242-130040; cair_region_combat_data.rs |
| 43 | market_trade_live | State | ResourceDefinitions, CountryResources, ResourceExchanges, PurchaseContracts | Live 2026-09-10/11; CountryResources has no direct save mapping; purchase subbuffers, prices, subsidies, history outside minimum rows | attach_log 113042-119760, 122194-123080; ccountry_resources.rs, cequipment_purchase_relation.rs |
| 44 | supply_logistics_attrition_live | State | ProvinceSupply, DisruptedSupply, ForeignHomebases | Live 2026-09-10/11; ProvinceSupply is a runtime cache with no save mapping | attach_log 119762-122192; ccountry_supply_system.rs, sprovince_supply_data.rs |
| 45 | focus_research_decisions_depth_live | State | shared with 12, 13, 16 | Not a separate source | attach_log 149372-150007 |
| 46 | air_variant_archetypes_live | State | AirAces (+ VariantDefinitions) | Live 2026-09-10/11; archetype classification is content-derived and excluded | attach_log 123297-123617; lanes/air_wings_variants_lifecycle_live ace_roster_live_roots.rs 62-325; cace.rs |
| 47 | characters_leaders_cabinet_live | State | Characters, CountryLeaderRoles, AdvisorRoles, UnitLeaders, UnitLeaderTraits, UnitLeaderTraitExperience | Live 2026-09-10/11; experience i64, skills i32; second most expensive read (11.6 ms) | attach_log 87613-87958; lanes/characters_leaders_cabinet_live adapter.rs 439-812; ccharacter.rs, cunit_leader.rs, ccountry_leader.rs |
| 48 | construction_buildings_state_damage_live | State | BuildingConstruction, RailwayConstruction (+ 23) | Live 2026-09-10/11; rail remaining cost is not hours | attach_log 101802-101913; lanes/construction_buildings_state_damage_live lib.rs 435-738; cbuilding_production_line.rs |
| 49 | naval_combat_lifecycle_live | State | NavalCombats | Accepted 2026-09-10 (7.13) | attach_log 44579-44617, 98333-98639; cnaval_combat.rs |
| 50 | naval_combat_members_live | State | NavalCombatParticipants | Accepted 2026-09-10 (7.13) | attach_log 99652-100163; cfex_member.rs |
| 51 | scripted_state_flags_variables_live | State | ScriptFlags, ScriptVariables, LexicalTokens | Live 2026-09-10/11; LexicalTokens is a guarded passive copy between mutation witnesses and may report unavailable; no save comparison for the dictionary | attach_log 75165-75385, 76339-76529, 105986-106191; cvariables.rs, cflag_manager.rs, cpdx_lexer.rs, ctext_lexer.rs |
| 52 | occupation_resistance_compliance_live | State | OccupationSettings, StateGarrisons, StateAssignedManpower, CountryOccupationReserveManpower, StateResistance, CountryCollaboration | Live 2026-09-10/11; per-state overrides, aggregate caches, garrison equipment not recorded | lanes/occupation_resistance_compliance_live collector_module.rs 469-1230; ccountry_occupation_*.rs, cstate_garrison_data.rs, cresistance.rs |
| 53 | diplomacy_intel_operatives_live | State | Subjects, Autonomy, IntelligenceAgencies, OperativeMemberships, Cryptology, IntelNetworks, Operations, OperationAssignments, OperationTokens, IncomingDiplomaticActions, RelationOpinions, RelationTuples, CountryIntel | Live 2026-09-10/11 and 2026-09-11; operation resources, requirements, phase contents and per-modifier opinions not recorded; action body dates rest on the archived reader | lanes/diplomacy_intel_operatives_live extractor.rs 3594-10846; crelation_status.rs, crelation.rs, ccountry_intel.rs, cincoming_diplomatic_action.rs, ccountry_operation_manager.rs |
| 54 | army_battleplans_depth_live | State | ArmyOrders geometry, OrdersGroups | See 27 and 28 | lanes/army_battleplans_depth_live lib.rs 252-510 |
| 55 | land_combat_detail_events_live | Event-hook | excluded | Combat-round hooks are not installed; derive honestly from hourly state or report unavailable | attach_log 105473-105984 |
| 56 | naval_missions_status_live | State | Fleets and TaskForces mission fields, NavalSupremacy | Live 2026-09-10/11; supremacy is a daily cache | lanes/naval_missions_status_live lib.rs 286-1046; cnaval_region_dominance.rs |
| 57 | session_multiplayer_metadata_live | State | Players, HumanControllers, WorldTension, ThreatSources, ThreatCountryAggregates, Clock.speed | Live 2026-09-10/11 and 2026-09-11; pause, chat, policy thresholds excluded (not campaign state); saved header roster is not the live census | attach_log 103078-103830, 104254-104576; chuman.rs, cworld_threat.rs, cthreat_source.rs |
| 58 | country_economy_counters_live | State (no walk) | none | Reservation only | lanes/country_economy_counters_live (JSON) |
| 59 | templates_equipment_catalog_live | State (no walk) | none | Static content catalog, not state | lanes/templates_equipment_catalog_live (Python gate) |
| 60 | capitulation_war_state_live | State | Capitulation, WarRelations | Live 2026-09-10/11; legitimacy i64 | lanes/capitulation_war_state_live extractor.rs 188-637; cdiplomacy_status.rs |
| 61 | air_loss_stats_live | State | AirStatistics | Live 2026-09-10/11; 24 of 89 channels, one ring bucket | lanes/air_loss_stats_live lib.rs 459-767; cloop_history.rs |
| 62 | hub_motorization_live | State | HubMotorization | Live 2026-09-10/11; settings differ in same-hour saves (open, PLAN 1) | lanes/hub_motorization_live lib.rs 331-766; ccountry_supply_system.rs |
| 63 | country_core_scalars_live | State | Countries scalars, CountryExperience | Accepted 1.4; experience fixed15 live 2026-09-10/11; convoy and post-mortem counters not recorded | attach_log 137775-137911; ccountry_experience_status.rs |
| 64 | delayed_events_live | Event-hook | excluded | Per-country update hooks are not installed | attach_log 79016-79635 |
| 65 | special_project_programs_live | State | ProjectPrograms | See 41 | attach_log 147602-148283 |
| 66 | occupation_state_extras_live | State | StateOccupationLaws (+ 52) | Live 2026-09-10/11; literal law names | attach_log 148399-149289 |
| 67 | country_dynamic_modifiers_live | State | DynamicModifiers | Live 2026-09-10/11; removal events excluded | attach_log 138451-138982; cdynamic_modifier_container.rs |
| 68 | session_header_live | State | Players (with 57) | Saved header roster is refreshed by user bytes and is not a complete live census; printed limit | attach_log 139223-139335 |
| 69 | tech_sharing_groups_live | State | TechSharingGroups, TechSharingMembers | Live 2026-09-10/11 | attach_log 140173-140218, 146905-147468; ctechnology_sharing_group.rs |
| 70 | sunk_convoy_history_live | State | SunkConvoys | Live 2026-09-10/11 | attach_log 140220-140438; csunk_convoy_info.rs |
| 71 | dynamic_collection_trace_live | Presentation | excluded | UI collection hooks | lanes/dynamic_collection_trace_live |
| 72 | resolved_tooltip_instances_live | Presentation | excluded | Tooltip hooks | lanes/resolved_tooltip_instances_live |
| 74 | power_balance_state_live | State | PowerBalances, PowerBalanceMembers, PowerBalanceRanges, PowerBalanceOverrides | Live 2026-09-10/11 | attach_log/d74_power_balance.rs 303-628; power_balance_manager.rs, power_balance_record.rs |
| 75 | fired_event_popups_live | Event-hook | excluded | Event hooks | lanes/fired_event_popups_live |
| 77 | production_render_submits_live | Presentation | excluded | Render capture | lanes/production_render_submit_live |
| 78 | design_catalog_context_live | Presentation | excluded | Designer UI | lanes/design_catalog_context_live |
| 79 | hud_surface_activation_live | Presentation | excluded | HUD hooks | lanes/hud_role_observer_live |
| 80 | production_map_content_snapshot_live | Presentation | excluded | No producer ever existed | compact_contract lib.rs 1977-1985 |
| 81 | rail_replay_state_live | State | RailEdges, RailCooldowns (+ HubMotorization) | Live 2026-09-10/11; directed edges compared through reconstructed neighbour order; rail-change hooks excluded | attach_log 144896-146449, 142880-143774; crailway_manager.rs |
| 82 | character_presentation_live | Presentation | excluded | Portrait and trait UI | lanes/character_presentation_live |
| 83 | nuke_stockpile_live | State | Nukes | Live 2026-09-10/11 | lanes/nuke_stockpile_live nuke_stockpile_exact.rs 80-211 |
| 84 | scripted_gui_runtime_live | Presentation | excluded | Scripted GUI hooks | lanes/scripted_state_flags_variables_live a729_scripted_gui_runtime.rs |
| 85 | selected_designer_identity_live | Presentation | excluded | Designer selection hooks | lanes/design_catalog_context_live selected_designer_identity |
| 86 | sunk_ship_history_live | State | SunkShips | Live 2026-09-10/11; append hooks excluded | lanes/sunk_ship_history_live a729_hook.rs 2441-2640; csunk_ship_info.rs |
| 87 | decision_panel_live | State | Decisions, DecisionCounts | State part recorded with 16; rendered availability, costs, text excluded | attach_log 170921-171560 |
| 88 | active_command_abilities_live | State | ActiveAbilities | Live 2026-09-10/11 (empty on test campaign); selected-controller hooks excluded | lanes/active_command_abilities_live a729_hook.rs 2421-2628; active_ability_record.rs |
| 89 | national_focus_presentation_live | Presentation | excluded | Focus view hooks | attach_log 167510-167687 |
| 90 | convoy_transport_attack_live | Event-hook (future state candidate) | excluded | Sink terminal hook; per-sink attribution cannot be reconstructed from hourly aggregates | lanes/naval_combat_live convoy_terminal_hook_windows.rs 1607-2176 |
| 91 | raid_instances_live | State | RaidCountries, Raids | Live 2026-09-10/11; end date presence only | attach_log 151721-152180; craid_system.rs |
| 92 | country_preferred_tactic_live | State | PreferredTactics | Live 2026-09-10/11 | attach_log 77432-77508, 78104-78236; ccombat_tactic.rs |
| 93 | country_army_upgrade_requests_live | State | ArmyUpgradeGroups, ArmyUpgradeRequests, ArmyUpgradeDeliveries, ArmyUpgradeProduced | Live 2026-09-10/11 | lanes/country_army_upgrade_requests_live lib.rs 841-1741; carmy_upgrades_status.rs |
| 94 | pov_province_visibility_live | Event-hook (future state candidate) | excluded | Producer hook; needs a persistent root before any fog view | lanes/fow_visibility_live a729.rs 859-1102 |
| 95 | pov_spotted_units_live | Event-hook (future state candidate) | excluded | Four UI hooks; needs an hourly state source | lanes/spotted_units_live a729.rs 747-1095 |
| 96 | country_preferred_tactic_weight_live | State | PreferredTacticOperands | Live 2026-09-10/11 | attach_log 77762-78100 |
| 97 | peace_conference_live | State (presence) | PeaceConferences | Live 2026-09-10/11 (empty on test campaign); active identity and bids came from excluded window hooks | lanes/design_catalog_context_live peace_conference_presence.rs 28-62; peace_conference.rs |
| 98 | theatre_selector_live | State | LandTheatres, LandTheatreGroups, LandTheatreMembers, AirTheatres, AirTheatreGroups, AirTheatreMembers | Live 2026-09-10/11; alert categories and priorities are presentation | attach_log/theatre_selector.rs 713-1479; ctheatre.rs, ctheater_group.rs, cair_theatre.rs |
| 99 | division_combat_stats_live | Event-hook (future state candidate) | excluded | Combat-round detour; round bonuses are transient | lanes/division_combat_stats_live a729_hook.rs 643-917 |
| 100 | xmap07_snow_mud_live | Presentation | excluded | Render staging grid; weather state comes from 24 | lanes/map_passive_carriers_live xmap07_grid.rs 60-211 |
| 101 | map_furniture_victory_points_live | State | Provinces.victory_points | Live 2026-09-10/11; signed i32, zero proved by omission | attach_log 133863-134036; cprovince.rs |
| 102 | fog_derivation_inputs_live | State | RadarCoverage, VolunteerHosts, TheatreAreas, StationedUnits, UnitIntel, AirBaseRegistrations | Live 2026-09-11; three tables empty on the test campaign (no volunteer theatre); RadarCoverage and AirBaseRegistrations have no save representation; the country-ordinal and unit-class families are already covered by CountryLinks and the authenticated vtable; scripted reveals, radar rebuild and theatre entrances are not placed relative to the tick | lanes/fog_derivation_inputs_live lib.rs 955-2232; attach_log 132866-133217; cradars_pool.rs, radar_group.rs, ctheatre.rs, ccontroller_area.rs, cunit.rs, cair_base.rs |
| 103 | naval_combat_results_live | State | NavalResults, NavalResultMembers, NavalResultCountries | Live 2026-09-10/11 (empty on test campaign) | lanes/naval_combat_results_live lib.rs 704-1212; cnaval_combat_results.rs |
| 1000 | frame | State (infrastructure) | frame envelope (per-table outcomes, callback costs, control records) | Not a table | attach_log 156596-156624 |

Excluded subpaths without a registry row: runtime text inputs, and the technology, character and rail change gates. They are hooks; the underlying state is read hourly by the rows above.

## Map-track ETA addition (format 17, not accepted)

DivisionEta154 is an hourly companion keyed by the full division:u64 handle. It records next/destination/path_total:u32 and next_remaining/destination_remaining:i64 fixed5. CUnit BECF90/BEDD50 supply the route/current-progress algorithm; CArmy C72EE0/C6FA00/C70670, CRailwayManager1954F90 and CNavalUnitTransfer1955DF0 supply passive effective-speed derivations. No native calls or additional hook are installed. Full routes over4096 entries are explicitly unavailable; the256-entry DivisionMovement prefix is not substituted for full coverage. Owned operands feed duration arithmetic off-thread.

Native UI at60831010 matches ordinary SOV54 next28h/full138h and naval ENG871 full188h. Strategic-redeployment FRA5 at60831510 matches next2h and army-list full71h. The map counter's146h is proved to be a different pathfinding-cache result (BED280/BED3A0, army+300, duration14516912), not current-route BECF90. No UI-dependent cached estimate is substituted. Save comparisons cover full census and route metadata only, because durations are not serialized. Artifacts and current cost/lifecycle verdict: PLAN section1 and `captures\m21_takeover_20260913\eta_native_reference.md`. Polling and viewer source remain unchanged; a third callback-bounded acquisition worker and per-pass modifier snapshots await live acceptance.


## Supply train inputs, format21 (code-only, 2026-09-15)

SupplyNodes162 records each canonical country's companion node index, province,
kind, flag, fixed5 available/consumed, province-keyed parent and immediate selected
route. TrainInventory163 stores ordered full variant references/signed i32 counts,
node census size and shared canonical-slot-indexed controller-access bytes per
country. Both are hourly. Nodes join Countries/Provinces, parents join the same
country's node provinces, and allocations join VariantDefinitions (existing
four-hour definition/name refresh). The native missing-route branch separately
records whether its two controller-companion subtype-zero node lookups emit a
synthetic naval break. Consumers retain native path duplicates/order and the
100-parent native traversal limit before reversing land portions.

Verified consumers: 141598790, 1411FC780, 141213EA0, 141213CA0, 140EB3740,
141213980, 1412040E0, 141598230, 141C4EB80, 141C4F3C0 and 1406E1B30.
The access check is in route validity, not the departure draw; inventory contains
variant references, not equipment-type tokens. Serializer14121C6D0 writes truck
allocation+148 and settings/disrupted/foreign-homebase containers, omitting the
allocated trains+120 and companion simulation nodes. The oracle checks saved
country/province/variant joins and prints these coverage limits; it does not
substitute truck counts, stockpiles or hub settings for the requested operands.

Engine layouts: supply_node.rs, supply_parent_map.rs, supply_route.rs,
supply_train_entry.rs, supply_companion.rs and ccountry_supply_system.rs.
Preallocated independent rows/arenas, 200-success table windows, per-table read
and spool costs, explicit work bounds and whole-row truncation are integrated.
No native calls or live pointers cross to conversion. No game or lock access;
live consistency, fresh-save oracle, actual cost and throughput remain unmeasured.
Contract, canonical evidence, bounds, gate outcomes and later-session procedure:
`captures\m21_takeover_20260913\recorder3_handoff.md`.
