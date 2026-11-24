use super::*;
pub(super) fn expected_tables(family: Family) -> &'static [TableId] {
    use TableId::*;
    match family {
        Family::Portrait
        | Family::CombatLogPrune
        | Family::Opinion
        | Family::Advisor
        | Family::Relation
        | Family::History
        | Family::Flags
        | Family::Buildings
        | Family::AdvisorPools
        | Family::PartyLeaders
        | Family::Variables
        | Family::Parties
        | Family::Wings
        | Family::Roster
        | Family::PoliticalStatus
        | Family::Rails
        | Family::Territory
        | Family::ArmyStats
        | Family::CountryInputs
        | Family::Missions
        | Family::Raid
        | Family::NavalTrees
        | Family::UnitLeaders
        | Family::Census
        | Family::OrdersGroups
        | Family::OrderMembership
        | Family::AirRecords
        | Family::LocalSubunits
        | Family::ShipModels
        | Family::ProvinceUnits
        | Family::ProvinceCombats
        | Family::AirBaseCarry
        | Family::Fuel
        | Family::PlayerCommands
        | Family::Propaganda => &[],
        Family::EquipmentDestruction => &[EquipmentDestruction, DivisionEquipmentDeletions],
        Family::Land => &[LandCombatResults, DestroyedDivisions],
        Family::Construction => &[ConstructionLineOutput],
        Family::LineOutput => &[ProductionLineOutput, RocketProductionOutput],
        Family::WorldInputs => &[
            DoctrineFolders,
            DoctrineTracks,
            DoctrineLeaderMastery,
            DoctrineCostReductions,
            DoctrineMasteryGrants,
            DoctrineEquipmentBonuses,
            DoctrineCountries,
        ],
        Family::Templates => &[
            DivisionTemplates,
            DivisionTemplateIcons,
            CountryTemplateState,
            OrdersGroupCounterInputs,
        ],
        Family::Experience => &[CountryExperienceGains],
        Family::OperationOutcomes => &[OperationCompletions],
        Family::Politics => &[
            CountryPowerDaily,
            CountryPowerChanges,
            CountryPowerDailyCosts,
            GovernmentChanges,
        ],
        Family::Logistics => &[LogisticsEquipment],
        Family::Peace => &[PeaceSettlements],
        Family::NavalResultSources => &[NavalResultSources],
        Family::AirLosses => &[AirLossEvents],
        Family::BuildingBombing => &[BuildingBombingDamage],
        Family::CombatEngagementRefreshes => &[CombatEngagementRefreshes],
        Family::NuclearStrikes => &[NuclearStrikes],
        Family::MapEvents => &[MapEvents],
        Family::EventFeed => &[EventFeedHistory],
        Family::IntelSources => &[IntelContributions, IntelSources],
        Family::NetworkAuxiliary => &[IntelNetworkAuxiliaries],
        Family::NavalManagement => &[
            NavalRepairCountries,
            NavalRepairFleets,
            NavalRepairTaskForces,
            NavalTaskForceComposition,
            NavalRepairBases,
            NavalRepairQueue,
            NavalRefitShips,
            NavalRefitProgress,
            NavalRepairChildren,
        ],
        Family::WarParticipation => &[
            WarParticipation,
            WarCapturedProvinces,
            WarLendLeaseHistories,
        ],
        Family::Designs => &[
            VariantChanges,
            DesignCountryModifiers,
            DesignCountryTemplates,
            DesignOrganisationLiveness,
        ],
        Family::FactionGoals => &[FactionGoalStatus, FactionGoalEntries],
        Family::FactionCompanions => &[
            FactionRules,
            FactionPrograms,
            FactionManpower,
            FactionUnlocks,
            FactionAccounts,
            FactionProjection,
            FactionResources,
            FactionAdvisorModifiers,
            FactionMinimumUpgrades,
            FactionDoctrine,
        ],
        Family::Market => &[MarketEvents],
        Family::Hourly | Family::CommandBatch | Family::WorldLifetime => &[],
    }
}

fn constructor_sites(family: Family) -> Vec<engine_sites::Rva> {
    use engine_air_history::{air_history_mutations, air_loss_sites, air_record_mutations};
    use engine_characters::ccharacter_portraits;
    use engine_events::{
        building_damage_sites, diplomacy_mutations, equipment_destruction, event_feed_sites,
        experience_gain_sites, map_event_sites, nuclear_strike_sites, operation_outcome_sites,
    };
    use engine_fuel::fuel_awards;
    use engine_land_inputs::{clogistics_status, land_events, land_wipe, ncombat_log_cmanager};
    use engine_mutations::{
        advisor_mutations, advisor_pool_mutations, air_base_mutations,
        air_wing_equipment_mutations, army_stat_mutations, building_mutations, census_mutations,
        character_role_mutations, character_roster_mutations, country_input_mutations,
        local_subunit_mutations, naval_tree_mutations, opinion_mutations, orders_group_mutations,
        party_leader_mutations, political_party_mutations, political_status_mutations,
        province_combat_mutations, province_unit_mutations, raid_mutations, railway_mutations,
        regional_mission_mutations, script_flag_mutations, script_variable_mutations,
        ship_model_mutations, template_mutations, territory_mutations, unit_leader_mutations,
        world_input_mutations, world_lifetime_sites,
    };
    use engine_native::cgame_state;
    use engine_naval_history::naval_result_sources;
    use engine_peace::peace_finalizer_site;
    use engine_production::line_output_sites;
    macro_rules! sites {
        ($items:expr, $site:pat => $rva:expr) => {
            $items.into_iter().map(|$site| $rva).collect::<Vec<_>>()
        };
    }
    let mut sites = match family {
        Family::PlayerCommands => engine_native_air::command_inputs::SITES
            .iter()
            .map(|site| site.rva)
            .collect(),
        Family::Hourly => vec![cgame_state::HOURLY_UPDATE_RVA],
        Family::CommandBatch => vec![cgame_state::COMMAND_BATCH_RVA],
        Family::EquipmentDestruction => sites!(equipment_destruction::SITES, s => s.rva),
        Family::Land => [
            land_events::FINALIZE_RVA,
            land_events::DESTROY_RVA,
            land_events::BORDER_FINALIZE_RVA,
            land_events::BORDER_TERMINATE_RVA,
            land_events::DISPATCH_RVA,
            land_events::DESTRUCTION_RVA,
            land_wipe::LIFECYCLE_RVA,
        ]
        .into_iter()
        .chain(land_events::DISPATCH_LEADER_CALLS)
        .collect(),
        Family::Portrait => sites!(ccharacter_portraits::MUTATIONS, s => s.rva),
        Family::Opinion => opinion_mutations::MUTATIONS
            .iter()
            .chain(&opinion_mutations::RELATIONS)
            .map(|s| s.rva)
            .collect(),
        Family::Construction => sites!(collector_output::construction_ic::SITES, s => s.rva),
        Family::LineOutput => vec![
            line_output_sites::CREDIT.rva,
            line_output_sites::DELIVERY.rva,
            line_output_sites::ROCKET_ADVANCE.rva,
        ],
        Family::CombatLogPrune => vec![ncombat_log_cmanager::PRUNE_RVA],
        Family::Advisor => advisor_mutations::MUTATIONS
            .iter()
            .chain(&character_role_mutations::MUTATIONS)
            .map(|s| s.rva)
            .collect(),
        Family::Relation => sites!(diplomacy_mutations::MUTATIONS, s => s.rva),
        Family::History => sites!(air_history_mutations::MUTATIONS, s => s.rva),
        Family::Flags => sites!(script_flag_mutations::MUTATIONS, (s, _) => s.rva),
        Family::Buildings => sites!(building_mutations::MUTATIONS, (s, _) => s.rva),
        Family::AdvisorPools => {
            sites!(&advisor_pool_mutations::MUTATIONS[..advisor_pool_mutations::MUTATIONS.len()-1], (s, _) => s.rva)
        }
        Family::PartyLeaders => sites!(party_leader_mutations::MUTATIONS, (s, _) => s.rva),
        Family::Variables => sites!(script_variable_mutations::MUTATIONS, (s, _, _) => s.rva),
        Family::Parties => sites!(political_party_mutations::MUTATIONS, s => s.rva),
        Family::Wings => sites!(air_wing_equipment_mutations::MUTATIONS, (s, _) => s.rva),
        Family::Roster => sites!(character_roster_mutations::MUTATIONS, (s, _) => s.rva),
        Family::PoliticalStatus => {
            sites!(political_status_mutations::MUTATIONS, (s, _, _) => s.rva)
        }
        Family::Missions => sites!(regional_mission_mutations::MUTATIONS, s => s.rva),
        Family::Rails => sites!(railway_mutations::MUTATIONS, (s, _) => s.rva),
        Family::Territory => sites!(territory_mutations::MUTATIONS, (s, _) => s.rva),
        Family::ArmyStats => sites!(army_stat_mutations::MUTATIONS, s => s.rva),
        Family::CountryInputs => sites!(country_input_mutations::MUTATIONS, (s, _, _) => s.rva),
        Family::Raid => raid_mutations::MUTATIONS
            .iter()
            .map(|(s, _)| s.rva)
            .chain([raid_mutations::DAILY_CAPACITIES.rva])
            .collect(),
        Family::NavalTrees => {
            sites!(&naval_tree_mutations::MUTATIONS[naval_tree_mutations::COUNTRY_SITES..], s => s.rva)
        }
        Family::UnitLeaders => sites!(unit_leader_mutations::MUTATIONS, s => s.rva),
        Family::Census => sites!(census_mutations::MUTATIONS, s => s.rva),
        Family::OrdersGroups => sites!(orders_group_mutations::MUTATIONS, (s, _) => s.rva),
        Family::OrderMembership => {
            sites!(engine_mutations::order_membership_mutations::MUTATIONS, (s, _) => s.rva)
        }
        Family::WorldInputs => sites!(world_input_mutations::MUTATIONS, (s, _) => s.rva),
        Family::AirRecords => sites!(air_record_mutations::MUTATIONS, s => s.rva),
        Family::LocalSubunits => sites!(local_subunit_mutations::MUTATIONS, (s, _) => s.rva),
        Family::Templates => sites!(template_mutations::MUTATIONS, (s, _) => s.rva),
        Family::ShipModels => sites!(ship_model_mutations::MUTATIONS, s => s.rva),
        Family::ProvinceUnits => sites!(province_unit_mutations::MUTATIONS, s => s.rva),
        Family::ProvinceCombats => sites!(province_combat_mutations::MUTATIONS, s => s.rva),
        Family::AirBaseCarry => sites!(air_base_mutations::MUTATIONS, (s, _) => s.rva),
        Family::Experience => sites!(experience_gain_sites::SITES, s => s.rva),
        Family::Fuel => [
            fuel_awards::BEGIN,
            fuel_awards::END,
            fuel_awards::NAVAL,
            fuel_awards::AIR,
            fuel_awards::RESET,
        ]
        .into_iter()
        .chain(fuel_awards::LAND)
        .collect(),
        Family::OperationOutcomes => vec![operation_outcome_sites::COMPLETION_RVA],
        Family::Politics => collector_politics::politics_changes::entry_sites()
            .iter()
            .map(|s| s.rva)
            .chain(
                collector_politics::politics_changes::TAPS
                    .iter()
                    .map(|s| s.rva),
            )
            .collect(),
        Family::Logistics => clogistics_status::BALANCE_CALLS
            .into_iter()
            .map(|(rva, _)| rva)
            .chain(clogistics_status::HISTORY_CALLS.map(|(rva, _)| rva))
            .collect(),
        Family::Peace => vec![peace_finalizer_site::RVA],
        Family::NavalResultSources => sites!(naval_result_sources::SITES, s => s.rva),
        Family::AirLosses => sites!(air_loss_sites::SITES, s => s.rva),
        Family::BuildingBombing => vec![building_damage_sites::APPLY_DAMAGE_RVA],
        Family::CombatEngagementRefreshes => {
            sites!(engine_mutations::combat_engagement_mutations::MUTATIONS, s => s.rva)
        }
        Family::NuclearStrikes => sites!(nuclear_strike_sites::SITES, s => s.rva),
        Family::MapEvents => vec![
            map_event_sites::FAKE_REVEAL_VISIBILITY_CALL_RVA,
            map_event_sites::GENERAL_CAPTURE_CALL_RVA,
            map_event_sites::PARADROP_LANDING_CALL_RVA,
            map_event_sites::RAILWAY_OVERRUN_RVA,
            map_event_sites::RESISTANCE_ACTIVITY_RVA,
            map_event_sites::BUILDING_NOTIFICATION_RVA,
            map_event_sites::CRITICAL_BOMBARDMENT_TAP_RVA,
        ],
        Family::EventFeed => [event_feed_sites::WAR.rva, event_feed_sites::ANNEX.rva]
            .into_iter()
            .chain(event_feed_sites::provenance::ENTRIES.iter().map(|s| s.rva))
            .collect(),

        Family::Propaganda => vec![
            engine_intelligence::propaganda_history::DAILY_UPDATE_RVA,
            engine_intelligence::propaganda_history::RESET_RVA,
        ],
        Family::IntelSources => engine_intelligence::intel_source_sites::SITES
            .iter()
            .map(|s| s.rva)
            .collect(),
        Family::NetworkAuxiliary => engine_intelligence::network_auxiliary::SITES
            .iter()
            .map(|s| s.rva)
            .collect(),
        Family::NavalManagement => collector_naval_management::changes::patch_sites()
            .map(|(_, rva)| rva)
            .collect(),
        Family::WarParticipation => engine_mutations::war_participation_sites::SITES
            .iter()
            .map(|s| s.rva)
            .collect(),
        Family::Designs => engine_native::cequipment_variant::mutations::MUTATIONS
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                !engine_native::cequipment_variant::mutations::FORWARDED_SITES.contains(i)
            })
            .map(|(_, s)| s.rva)
            .chain(
                engine_native::cequipment_variant::source_mutations::MUTATIONS
                    .iter()
                    .map(|s| s.rva),
            )
            .chain(
                engine_native::cequipment_variant::source_mutations::ORGANISATIONS
                    .iter()
                    .map(|s| s.rva),
            )
            .chain(
                engine_native::cproduction_status::design_templates::MUTATIONS
                    .iter()
                    .map(|s| s.rva),
            )
            .collect(),
        Family::FactionGoals => engine_mutations::faction_goal_mutations::MUTATIONS
            .iter()
            .map(|(s, _, _)| s.rva)
            .collect(),
        Family::FactionCompanions => engine_mutations::faction_companion_mutations::MUTATIONS
            .iter()
            .map(|(s, _, _)| s.rva)
            .collect(),
        Family::Market => collector_observers::market_changes::OBSERVED_SITES
            .iter()
            .map(|&i| engine_production::market_mutations::SITES[i].rva)
            .collect(),
        Family::WorldLifetime => vec![
            world_lifetime_sites::RESET.rva,
            world_lifetime_sites::CONSTRUCT.rva,
            world_lifetime_sites::DELETE.rva,
            world_lifetime_sites::STATE_RELOAD.rva,
            world_lifetime_sites::STRAITS_RELOAD.rva,
        ],
    };
    match family {
        Family::Missions => {
            sites.extend(
                engine_native_naval::naval_mine_inputs::WRITERS
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != 5)
                    .map(|(_, (_, rva))| *rva),
            );
            sites.extend(engine_naval_history::cnaval_accident_report::WRITERS);
            sites.extend(
                engine_mutations::navy_ledger_mutations::MUTATIONS
                    .iter()
                    .map(|site| site.rva),
            );
            sites.extend(
                collector_observers::fleet_region_changes::SITES
                    .iter()
                    .map(|s| s.rva),
            );
            sites.extend([
                engine_map_air::map_air_sortie_inputs::DETECTION_OVERRIDE_WRITER_RVA,
                engine_map_air::radar_group::UPDATE_RVA,
                engine_map_air::map_air_country_region_modifier::GROUND_CREWS_ENABLE_RVA,
                engine_map_air::map_air_country_region_modifier::GROUND_CREWS_DISABLE_RVA,
                engine_map_air::map_air_country_region_modifier::GROUND_CREWS_CONSTRUCTOR_RVA,
            ]);
        }
        Family::CombatLogPrune => sites.extend([
            engine_land_inputs::combat_log_sites::GROUP_CREATE_CALL_RVA,
            engine_land_inputs::combat_log_sites::OWN_EQUIPMENT_RVA,
            engine_land_inputs::combat_log_sites::OWN_VARIANT_RVA,
            engine_land_inputs::combat_log_sites::ENEMY_RVA,
            engine_land_inputs::combat_log_sites::RECOVERED_RVA,
            engine_land_inputs::combat_log_sites::MANPOWER_RVA,
            engine_land_inputs::combat_log_sites::RESULT_RVA,
            engine_land_inputs::combat_log_sites::GROUP_PRUNE_RVA,
            engine_land_inputs::combat_log_sites::GROUP_DESTROY_RVA,
            engine_land_inputs::combat_log_sites::BATTLE_ALLOCATE_RVA,
            engine_land_inputs::combat_log_sites::BATTLE_RELEASE_RVA,
            engine_land_inputs::combat_log_sites::BATTLE_RESET_RVA,
            engine_land_inputs::combat_log_sites::BATTLE_LOAD_RVA,
            engine_land_inputs::combat_log_sites::TEMPLATE_COUNTER_RVA,
        ]),
        Family::ArmyStats => sites.extend(
            engine_mutations::division_panel_mutations::MUTATIONS
                .iter()
                .map(|s| s.rva),
        ),
        Family::Wings => {
            sites.extend(
                engine_mutations::air_wing_equipment_mutations::MAP_MUTATIONS
                    .iter()
                    .filter(|s| {
                        s.rva
                            != engine_mutations::country_input_mutations::MUTATIONS[0]
                                .0
                                .rva
                    })
                    .map(|s| s.rva),
            );
            sites.extend(
                engine_native_air::cair_wing::replacement::MUTATIONS
                    .iter()
                    .map(|s| s.rva),
            );
        }
        Family::CountryInputs => sites.extend(
            engine_native::cwar_relation::goals::mutations::SITES
                .iter()
                .map(|s| s.rva),
        ),
        Family::NavalTrees => sites.extend(
            engine_occupation::cresistance::changes::SITES
                .iter()
                .map(|s| s.rva),
        ),
        Family::WorldInputs => {
            sites.extend(
                engine_mutations::doctrine_mutations::SITES
                    .iter()
                    .map(|s| s.rva),
            );
            sites.extend(engine_mutations::doctrine_mutations::EFFECTIVE_GAIN_STORES);
        }
        Family::Politics => {
            sites.push(engine_events::script_event_sites::decisions::TARGETED_SELECTION_RESULT.rva)
        }
        Family::EventFeed => {
            sites.extend(
                engine_events::script_event_sites::TAPS
                    .iter()
                    .map(|s| s.rva),
            );
            sites.extend(
                engine_events::script_event_sites::REGISTRY_ENTRIES
                    .iter()
                    .map(|s| s.rva),
            );
            sites.push(engine_events::script_event_sites::ACTIVATE_TARGETED.rva);
            sites.extend(
                engine_decisions::cdecision::EVENT_FEED_ENTRIES
                    .iter()
                    .map(|(_, rva)| *rva),
            );
            sites.extend(
                engine_events::script_event_sites::LIFECYCLE_TAPS
                    .iter()
                    .chain(engine_events::script_event_sites::decisions::TAPS.iter())
                    .filter(|s| {
                        s.rva
                            != engine_events::script_event_sites::decisions::TARGETED_SELECTION_RESULT.rva
                            && s.rva
                                != engine_events::script_event_sites::decisions::CLEAR_CANDIDATES.rva
                    })
                    .map(|s| s.rva),
            );
            sites.extend(
                engine_events::script_event_sites::DELAYED_ENTRIES
                    .iter()
                    .take(2)
                    .chain(engine_events::script_event_sites::LIFECYCLE_ENTRIES.iter())
                    .chain(engine_events::script_event_sites::decisions::ENTRIES.iter())
                    .map(|s| s.rva),
            );
            sites.extend(
                engine_events::script_event_sites::decisions::CALLS
                    .iter()
                    .map(|s| s.rva),
            );
        }
        Family::Variables => {
            sites.push(engine_events::script_event_sites::DELAYED_PASS.rva);
            sites.extend(
                engine_events::script_event_sites::DELAYED_TAPS
                    .iter()
                    .map(|s| s.rva),
            );
        }
        Family::MapEvents => sites.push(engine_events::map_event_sites::SCRIPTED_REFRESH_CALL_RVA),
        _ => {}
    }
    sites
}

#[test]
fn every_constructor_site_has_its_actual_family_registration() {
    let patches = crate::hook_inventory::patches();
    let mut failures = Vec::new();
    for family in Family::ALL {
        let mut expected = constructor_sites(family);
        let mut actual: Vec<_> = patches
            .iter()
            .filter(|p| family.owns_patch_family(p.family))
            .map(|p| p.rva)
            .collect();
        expected.sort_unstable();
        actual.sort_unstable();
        if actual != expected {
            failures.push(format!(
                "{} actual {actual:?}; expected {expected:?}",
                family.name()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "native ownership differs from constructors: {failures:#?}"
    );
}

#[test]
fn omitted_or_wrong_family_native_site_is_a_registration_failure() {
    use collector_hooks::hook::installation::{self, Kind};
    let (_, report) = installation::capture(|| installation::site(1));
    assert_eq!(
        audit_report(crate::hook_inventory::patches, Family::Portrait, 0, report)
            .unwrap()
            .unwrap()
            .kind,
        Kind::Registration
    );
    let at = engine_events::equipment_destruction::SITES[0].rva;
    let (_, report) = installation::capture(|| installation::site(at.0));
    assert_eq!(
        audit_report(crate::hook_inventory::patches, Family::Portrait, 0, report)
            .unwrap()
            .unwrap()
            .kind,
        Kind::Registration
    );
}

pub(crate) fn isolated(test: &str) -> bool {
    const CHILD: &str = "BATTLEPLAN_CONTAINMENT_REVIEW_TEST";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    false
}
pub(crate) fn reset_session() {
    collector_primitives::build_gate::record(true, None);
    reset_failures_for_test();
}
struct Unreadable;
impl engine_native_memory::Memory for Unreadable {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        false
    }
}

#[test]
fn real_silent_constructors_refuse_and_propagate_their_family() {
    if !isolated(
        "hook_failures::contract_tests::real_silent_constructors_refuse_and_propagate_their_family",
    ) {
        return;
    }
    reset_session();
    let directory =
        std::env::temp_dir().join(format!("hoi4-real-hook-refusal-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    configure(&directory, 1, 1000, crate::status_file::write_status_bytes);
    let status_path = directory.join("status.json");
    std::fs::write(&status_path, b"{}").unwrap();
    let reader = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
            .open(&status_path)
            .unwrap()
    };
    let equipment =
        FamilyHooks::<collector_equipment_destruction::equipment_destruction::Hooks>::construct(
            0,
            || {
                Ok::<_, &'static str>(unsafe {
                    collector_equipment_destruction::equipment_destruction::Hooks::new(
                        &Unreadable,
                        0,
                    )
                })
            },
        )
        .unwrap();
    drop(reader);
    assert!(
        std::fs::read_to_string(directory.join("recorder.log"))
            .unwrap()
            .contains("hook failure status not published")
    );
    assert!(
        equipment.inner.is_some(),
        "exercise the API that internally returned an empty object"
    );
    assert!(failed(Family::EquipmentDestruction));
    assert!(failed(Family::Wings));
    assert!(failed(Family::NuclearStrikes));
    let status = serde_json::to_value(status()).unwrap();
    let root = status
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["family"] == "equipment_destruction")
        .unwrap();
    assert_eq!(root["stage"], "sentinel");
    assert!(
        root["site"]
            .as_str()
            .unwrap()
            .contains(engine_events::equipment_destruction::SITES[0].name)
    );
    assert!(root["cause"].as_str().unwrap().contains("withheld"));
    reset_session();
    let building =
        FamilyHooks::<collector_construction::building_changes::Hooks>::construct(0, || unsafe {
            collector_construction::building_changes::Hooks::new(&Unreadable, 0)
        })
        .unwrap();
    assert!(building.inner.is_some());
    assert!(failed(Family::Buildings));
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&status_path).unwrap()).unwrap();
    assert!(
        persisted["failed_hook_families"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["family"] == Family::Buildings.name())
    );
    reset_session();
    let opinion =
        FamilyHooks::<collector_observers::opinion_changes::Hooks>::construct(0, || unsafe {
            collector_observers::opinion_changes::Hooks::new(&Unreadable, 0)
        })
        .unwrap();
    assert!(opinion.inner.is_some());
    assert!(failed(Family::Opinion));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn template_rollback_failure_stays_fatal_with_fresh_read_fallback() {
    use replay_frame::frame::Metrics;
    if !isolated(
        "hook_failures::contract_tests::template_rollback_failure_stays_fatal_with_fresh_read_fallback",
    ) {
        return;
    }
    reset_session();
    let result =
        FamilyHooks::<collector_division_templates::template_changes::Hooks>::construct(0, || {
            collector_hooks::hook::installation::refused(
                collector_hooks::hook::installation::Kind::Rollback,
                None,
                "synthetic unsafe template rollback",
            );
            Err("synthetic template construction failure")
        });
    assert!(result.is_err());
    assert!(fatal_installation_error().is_some());
    assert!(
        !collector_division_templates::template_changes::Changes::fresh_only(
            &collector_division_templates::template_changes::Observer
        )
    );
    let mut input = FrameInput {
        branch: 1,
        hour: 1,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(replay_data::rows::TableRows::empty(TableId::ALL[index])).into()
        }),
    };
    assert!(apply(&mut input).is_err());
}

#[test]
fn initial_internal_rollback_failure_enters_teardown_and_stays_terminal() {
    if !isolated(
        "hook_failures::contract_tests::initial_internal_rollback_failure_enters_teardown_and_stays_terminal",
    ) {
        return;
    }
    reset_session();
    let mut hooks = FamilyHooks::new(Family::Construction, Ok(())).unwrap();
    assert!(!failed(Family::Construction));
    let directory =
        std::env::temp_dir().join(format!("hoi4-initial-rollback-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    configure(&directory, 1, 1000, crate::status_file::write_status_bytes);
    let progress = crate::progress::Progress::open(&directory);
    let stage = crate::progress::Stage::new(&progress, Family::Construction.name());
    let mut later_installed = false;
    let error = crate::runtime::install_attach_batch(
        &stage,
        &mut [
            (Family::Construction.name(), &mut || {
                hooks.toggle(true, |_, _| {
                    collector_output::output_hooks::rollback_after_publication_error(
                        "initial enable failed",
                        || Err("initial rollback failed".into()),
                    )
                    .map(|()| true)
                    .map_err(Into::into)
                })
            }),
            (Family::Hourly.name(), &mut || {
                later_installed = true;
                Ok(true)
            }),
        ],
    )
    .unwrap_err();
    assert!(
        !later_installed,
        "attach continued after its first failed rollback"
    );
    assert!(
        !failed(Family::Construction),
        "the initial failure preceded central family refusal"
    );
    let first = install_failed(
        crate::hook_inventory::patches,
        Family::Construction,
        error,
        installation_message,
    )
    .unwrap_err();
    assert!(first.contains("initial rollback failed"));
    assert!(failed(Family::Construction));
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("status.json")).unwrap()).unwrap();
    assert!(
        persisted["failed_hook_families"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["family"] == "construction_ic" && f["stage"] == "rollback")
    );
    hooks.toggle(false, |_, _| Ok(true)).unwrap();
    assert_eq!(
        fatal_installation_error(),
        Some(first),
        "later disable erased the first rollback failure"
    );
    assert!(
        install_failed(
            crate::hook_inventory::patches,
            Family::Portrait,
            "later error",
            installation_message
        )
        .is_err()
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn later_shared_rollback_is_latched_before_an_earlier_reporting_error() {
    if !isolated(
        "hook_failures::contract_tests::later_shared_rollback_is_latched_before_an_earlier_reporting_error",
    ) {
        return;
    }
    use collector_hooks::hook::{Hook, TestCode, shared};
    reset_session();
    let _guard = collector_hooks::hook::FAMILY_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let code = TestCode::new();
    unsafe extern "system" fn original() -> usize {
        7
    }
    unsafe extern "system" fn replacement() -> usize {
        19
    }
    let make = |offset| {
        let (site, sentinel) = code.stub(offset, original as *const () as usize);
        vec![unsafe { Hook::new(site, replacement as *const () as usize, sentinel) }.unwrap()]
    };
    let mut air = FamilyHooks::new(Family::AirLosses, Ok(make(0))).unwrap();
    let mut bombing = FamilyHooks::new(Family::BuildingBombing, Ok(make(64))).unwrap();
    fn fail_bombing_flush() {
        collector_hooks::hook::fail_flushes(1);
    }
    let directory =
        std::env::temp_dir().join(format!("hoi4-later-shared-rollback-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    configure(&directory, 1, 1000, crate::status_file::write_status_bytes);
    let progress = crate::progress::Progress::open(&directory);
    let stage = crate::progress::Stage::new(&progress, Family::AirLosses.name());
    shared::fail_reopens(3, 1);
    let mut air_reports = 0;
    let first = crate::runtime::install_attach_batch(
        &stage,
        &mut [
            (Family::AirLosses.name(), &mut || {
                if shared::attach_reporting() {
                    air_reports += 1;
                }
                air.toggle(true, |hooks, enabled| {
                    Hook::set_family_enabled(hooks, enabled).map_err(Into::into)
                })
            }),
            (Family::BuildingBombing.name(), &mut || {
                bombing.toggle(true, |hooks, enabled| {
                    Hook::set_family_enabled_shared_prepared(hooks, enabled, fail_bombing_flush)
                        .map_err(Into::into)
                })
            }),
        ],
    )
    .unwrap_err();
    assert_eq!(
        air_reports, 0,
        "fatal groups must be latched before the first reporting error can stop traversal"
    );
    assert_eq!(stage.get(), Family::BuildingBombing.name());
    assert!(
        !air.inner.as_ref().unwrap()[0].is_enabled(),
        "first group's rollback succeeded"
    );
    assert!(
        bombing.inner.as_ref().unwrap()[0].is_enabled(),
        "later group's rollback failed"
    );
    assert!(first.contains("building_bombing"));
    assert!(first.contains("rollback page preparation failed"));
    assert!(failed(Family::BuildingBombing));
    assert!(
        !failed(Family::AirLosses),
        "earlier nonfatal outcome was not the fatal family"
    );
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("status.json")).unwrap()).unwrap();
    assert!(
        persisted["failed_hook_families"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["family"] == "building_bombing" && f["stage"] == "rollback")
    );
    for hooks in [&mut air, &mut bombing] {
        hooks
            .toggle(false, |hooks, enabled| {
                Hook::set_family_enabled(hooks, enabled).map_err(Into::into)
            })
            .unwrap();
        assert!(!hooks.inner.as_ref().unwrap()[0].is_enabled());
    }
    assert_eq!(fatal_installation_error(), Some(first));
    let retry = crate::runtime::install_attach_batch(
        &stage,
        &mut [(Family::AirLosses.name(), &mut || Ok(true))],
    );
    assert_eq!(
        retry,
        Err(first),
        "a successful retry erased the later group's first rollback failure"
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn peace_finalizer_refusal_keeps_independently_polled_bids() {
    if !isolated(
        "hook_failures::contract_tests::peace_finalizer_refusal_keeps_independently_polled_bids",
    ) {
        return;
    }
    reset_session();
    assert_eq!(Family::Peace.tables(), &collector_peace::tables::TABLES);
    assert_eq!(collector_peace::peace_bids::TABLES, [TableId::PeaceBids]);
    let mut source = replay_frame::frame::FrameInput {
        branch: 1,
        hour: 10,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(replay_data::rows::TableRows::empty(TableId::ALL[index])).into()
        }),
    };
    let bids = TableView::Complete(replay_data::rows::TableRows::PeaceBids(vec![
        replay_data::rows::PeaceSettlementRow {
            sequence: 7,
            conference: Some(9),
            hour: Some(10),
            winners: vec![23],
            ..Default::default()
        },
    ]));
    source.tables[TableId::PeaceBids.index()] = bids.clone().into();
    refuse(
        crate::hook_inventory::patches,
        Family::Peace,
        "publication",
        "finalizer refused",
    )
    .unwrap();
    apply(&mut source).unwrap();
    let replay_frame::frame::TableInput::Sampled(actual_bids) =
        &source.tables[TableId::PeaceBids.index()]
    else {
        panic!("bid payload was replaced");
    };
    assert_eq!(actual_bids, &bids);
    assert!(
        matches!(&source.tables[TableId::PeaceSettlements.index()], replay_frame::frame::TableInput::Sampled(TableView::Unavailable { reason }) if reason.contains("peace_settlements"))
    );
}

#[test]
fn every_hook_capture_producer_table_has_an_explicit_classification() {
    use TableId::*;
    use std::collections::BTreeSet;
    let dependent: BTreeSet<_> = Family::ALL
        .into_iter()
        .flat_map(|f| f.tables().iter().map(|t| t.index()))
        .collect();
    let mut covered = BTreeSet::new();
    let mut unclassified = Vec::new();
    macro_rules! producer {
        ($($module:ident)::+; [$($independent:ident),*]) => {{
            let actual: BTreeSet<_> = $($module)::+::TABLES.iter().map(|t| t.index()).collect();
            let independent: BTreeSet<usize> = [$($independent.index()),*].into_iter().collect();
            assert!(independent.is_subset(&actual), "{} removed an independently captured table", stringify!($($module)::+));
            assert!(independent.is_disjoint(&dependent), "{} independently captured table was refused", stringify!($($module)::+));
            let classified: BTreeSet<_> = actual.intersection(&dependent).copied().chain(independent).collect();
            if classified != actual { unclassified.push(format!("{} missing {:?}", stringify!($($module)::+), actual.difference(&classified).map(|i|TableId::ALL[*i]).collect::<Vec<_>>())); }
            covered.extend(actual.intersection(&dependent).copied());
        }};
    }
    producer!(collector_air::extended::air; [WingCumulative,AirAces,AirStatistics,AirReferenceWitnesses,AirMissionInputs,AirSortieBaseCountries,AirSortieModifiers,AirSortieSpecialBases,AirSortieModifierMetadata,AirNavalPatrolCombats]);
    producer!(collector_air_histories::air_histories; []);
    producer!(collector_air_records::air_records; []);
    producer!(collector_air_relations::air_relations; []);
    producer!(collector_ledger_tables::air_losses; []);
    producer!(collector_ledger_tables::building_bombing; []);
    producer!(collector_characters::extended::character_inputs; []);
    producer!(collector_characters::extended::character_portraits; []);
    producer!(collector_characters::extended::characters; []);
    producer!(collector_combat_log::combat_log; [LendLeaseEquipment]);
    producer!(collector_land_events::extended::combat_progress; [LandCombatProgress,CombatLogObservers]);
    producer!(collector_construction::construction; [StateBaseResources,BuildingConstruction,RailwayConstruction,OccupationSettings,StateOccupationLaws,StateGarrisons,StateAssignedManpower,CountryOccupationReserveManpower,CountryCollaboration]);
    producer!(collector_country_counters::country_appearance; [ReservedCountryNaming]);
    producer!(collector_country_counters::country_counters; [CountryCivilWars]);
    producer!(collector_output::construction_tables; []);
    producer!(collector_stability::country_stability; []);
    producer!(collector_divisions::extended::division_presentation; [DivisionMovement,MovementStateTerms,CoreSupplyAccess,RiverRelations,DivisionTransportSpeeds]);
    producer!(collector_division_templates::extended::division_templates; []);
    producer!(collector_economy::economy; [CountryExperience,ProvinceSupply,HubMotorization,DisruptedSupply,ForeignHomebases,ResourceDefinitions,CountryResources,ResourceExchanges,PurchaseContracts,CountryFuelConsumers,FuelDemandParts]);
    producer!(collector_equipment_destruction::extended::equipment_destruction; []);
    producer!(collector_experience_gains::tables; []);
    producer!(collector_ledger_tables::event_feed; []);
    producer!(collector_air::extended::fog; [VolunteerHosts,TheatreAreas,StationedUnits,UnitIntel,SentVolunteers]);
    producer!(collector_governance::governance; [ResearchSlots,DecisionCounts,Decisions,PreferredTactics,TechSharingGroups,TechSharingMembers,PowerBalances,PowerBalanceMembers,PowerBalanceRanges,PowerBalanceOverrides,PowerBalanceModifierBreakdowns,TimedIdeas,DecisionRandomItems]);
    producer!(collector_advisors::hired_advisors; []);
    producer!(collector_intel::intel; [IntelNetworkSummaries,Capitulation,Subjects,Autonomy,IntelligenceAgencies,OperativeMemberships,Cryptology,IntelNetworks,Operations,OperationAssignments,OperationTokens,IncomingDiplomaticActions,CountryIntel,CapturedOperativeReferences,AgencyUpgradeLevels,OperativeMissions,StrategicOperatives,AgencyOperationBuffers]);
    producer!(collector_land_events::extended::land_events; []);
    producer!(collector_leaders::leader_paths; []);
    producer!(collector_leaders::leader_portraits; []);
    producer!(collector_model_inputs::model_inputs; [CountryUnitBonuses]);
    producer!(collector_naval_details::naval_details; [NavalCombatDetails,NavalCombatShips,NavalCombatAirBaseWingHandles]);
    producer!(collector_division_templates::extended::group_counters; []);
    producer!(collector_naval_counters::naval_counters; [TaskForceCounterInputs,TaskForceMapMotion,TaskForceMissions]);
    producer!(collector_naval_details::naval_combat_sources; [NavalCombatTransfers,NavalTransferUnits]);
    producer!(collector_output::line_tables; []);
    producer!(collector_session::map_names; []);
    producer!(collector_slow_tables::logistics; []);
    producer!(collector_naval_history::extended::naval_history; [NavalSupremacy,SunkConvoys,LandTheatres,LandTheatreGroups,LandTheatreMembers,AirTheatres,AirTheatreGroups,AirTheatreMembers,NavalResults,NavalResultMembers,NavalResultCountries,SunkShips,OrdersGroups,OrdersGroupMembers,ArmyGroupMembers]);
    producer!(collector_naval_missions::extended::naval_mission_inputs; [NavalMissionTaskForces,NavalMissionLedgers,NavalPatrolMissionInputs,NavalDominanceTaskForceRanges]);
    producer!(collector_naval_results::extended::naval_result_sources; []);
    producer!(collector_ledger_tables::combat_engagement_refreshes; []);
    producer!(collector_ledger_tables::nuclear_strikes; []);
    producer!(collector_capture::extended::map_events; []);
    producer!(collector_operation_outcomes::extended::operation_outcomes; []);
    producer!(collector_peace::peace_bids; [PeaceBids]);
    producer!(collector_peace::tables; []);
    producer!(collector_politics::politics; []);
    producer!(collector_production::production; [ProductionLines,Stockpiles,DivisionEquipment,ArmyUpgradeGroups,ArmyUpgradeRequests,ArmyUpgradeDeliveries,ArmyUpgradeProduced,ProductionLicenses,DeploymentConveyors,DeploymentLines,DeploymentManpower,DeploymentEquipment,ProductionLicenseRecords]);
    producer!(collector_projects::projects; [ProjectBreakthroughs,Nukes,RaidCountries,PeaceConferences,RaidTargets,RaidCooldowns,RaidPathDistances,RaidPlayerAllowedTypes,RaidEquipment,RaidFacilities,RaidIntelInputs,RaidTooltipOperands]);
    producer!(collector_divisions::extended::province_unit_order; []);
    producer!(collector_session::session; []);
    producer!(collector_ship_models::ship_models; []);
    producer!(collector_production_daily::industrial_organisations; []);
    {
        let actual = collector_slow_tables::doctrines::TABLES;
        assert!(actual.iter().all(|t| dependent.contains(&t.index())));
        covered.extend(actual.map(|t| t.index()));
    }
    producer!(collector_ledger_tables::market; []);
    for table in collector_naval_management::forces::TABLES
        .into_iter()
        .chain(collector_naval_management::bases::TABLES)
    {
        assert!(dependent.contains(&table.index()));
        covered.insert(table.index());
    }

    producer!(collector_visibility::visibility; [SimulationFeatures,StateVisibility,CountryWars,LocalPlayer,LandUnitOrder]);
    producer!(collector_war_overview::war_overview; []);
    for table in collector_factions::faction_companions::TABLES
        .into_iter()
        .chain(collector_factions::faction_goals::TABLES)
    {
        assert!(dependent.contains(&table.index()));
        covered.insert(table.index());
    }
    covered.extend([States.index(), Provinces.index(), WarRelations.index()]);
    assert!(
        unclassified.is_empty(),
        "unclassified capture tables: {unclassified:#?}"
    );
    assert_eq!(
        covered, dependent,
        "refusal table has no capture-source contract"
    );
}

#[test]
fn map_exclusive_intel_hooks_install_nothing_without_reads_or_failure_counters() {
    if !isolated(
        "hook_failures::contract_tests::map_exclusive_intel_hooks_install_nothing_without_reads_or_failure_counters",
    ) {
        return;
    }
    reset_session();
    struct NoRead;
    impl engine_native_memory::Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("an omitted hook family read native memory");
        }
    }
    let mode = replay_data::scope::CaptureMode::Map;
    macro_rules! omitted {
        ($module:path, $family:ident) => {{
            let capture =
                collector_extended_support::outcomes::records_any(mode, Family::$family.tables());
            assert!(!capture);
            assert!(collector_extended_support::outcomes::records_any(
                replay_data::scope::CaptureMode::Full,
                Family::$family.tables()
            ));
            let mut hooks =
                FamilyHooks::construct(0, || unsafe { <$module>::new(&NoRead, 0, capture) })
                    .unwrap();
            assert!(hooks.inner.is_some());
            for _ in 0..100 {
                assert_eq!(hooks.set_enabled(true), Ok(true));
                assert_eq!(hooks.executable_ranges().count(), 0);
                assert_eq!(hooks.set_enabled(false), Ok(true));
            }
            assert!(!failed(Family::$family));
        }};
    }
    omitted!(collector_intel::propaganda_changes::Hooks, Propaganda);
    omitted!(collector_intel::source_changes::Hooks, IntelSources);
    omitted!(collector_intel::network_auxiliary::Hooks, NetworkAuxiliary);
    assert!(
        serde_json::to_value(status())
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}
