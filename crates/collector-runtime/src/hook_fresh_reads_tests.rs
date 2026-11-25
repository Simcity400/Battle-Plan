use super::*;
use replay_data::rows::*;
use replay_frame::frame::{Metrics, TableInput};

pub(super) fn source(hour: i32) -> FrameInput {
    FrameInput {
        branch: 1,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
    }
}

fn recoverable<T: RegisteredFamily>(name: &str, fresh: fn(i32) -> TableRows) {
    if !contract_tests::isolated(name) {
        return;
    }
    contract_tests::reset_session();
    let broken =
        FamilyHooks::<T>::construct(0, || Err("synthetic relay allocation failure")).unwrap();
    assert!(broken.inner.is_none());
    assert!(failed(T::FAMILY));
    let first = fresh(13);
    let second = fresh(29);
    assert_ne!(first, second, "fixture must change a recorded value");
    let table = first.table();
    assert!(T::FAMILY.preserves_capture(table));
    for (hour, rows) in [(10, first), (11, second)] {
        let mut frame = source(hour);
        frame.tables[table.index()] = TableView::Complete(rows).into();
        let expected = frame.tables[table.index()].clone();
        apply(&mut frame).unwrap();
        assert_eq!(frame.tables[table.index()], expected);
        for refused in contract_tests::expected_tables(T::FAMILY) {
            assert!(
                matches!(&frame.tables[refused.index()], TableInput::Sampled(TableView::Unavailable { reason }) if reason.contains("synthetic relay allocation failure"))
            );
        }
    }
    for view in [
        TableView::Truncated {
            rows: fresh(31),
            missing: 2,
        },
        TableView::Unavailable {
            reason: "synthetic native field read failure".into(),
        },
    ] {
        let mut frame = source(12);
        frame.tables[table.index()] = view.clone().into();
        apply(&mut frame).unwrap();
        assert_eq!(frame.tables[table.index()], TableInput::Sampled(view));
    }
    let mut frame = source(13);
    let shared = TableInput::SharedComplete(std::sync::Arc::new(fresh(37)));
    frame.tables[table.index()] = shared.clone();
    apply(&mut frame).unwrap();
    assert_eq!(frame.tables[table.index()], shared);
    for kind in [
        collector_hooks::hook::installation::Kind::Rollback,
        collector_hooks::hook::installation::Kind::Registration,
    ] {
        contract_tests::reset_session();
        assert!(
            FamilyHooks::<T>::construct(0, || {
                collector_hooks::hook::installation::refused(
                    kind,
                    None,
                    "synthetic unsafe installation",
                );
                Err("synthetic unsafe installation")
            })
            .is_err()
        );
        assert!(fatal_installation_error().is_some());
        assert!(apply(&mut frame).is_err());
    }
}

macro_rules! fresh_fallback {
    ($name:ident, $hooks:ty, $fresh:expr) => {
        #[test]
        fn $name() {
            recoverable::<$hooks>(
                concat!("hook_failures::fresh_reads_tests::", stringify!($name)),
                $fresh,
            );
        }
    };
}

fresh_fallback!(
    portrait_install_failure_keeps_fresh_portraits,
    collector_characters::portrait_changes::Hooks,
    |value| {
        TableRows::CharacterPortraits(vec![CharacterPortraitRow {
            character: 7,
            portraits: vec![CharacterPortrait {
                identity: value.to_le_bytes().to_vec(),
                ..Default::default()
            }],
        }])
    }
);
fresh_fallback!(
    opinion_install_failure_keeps_fresh_opinions,
    collector_observers::opinion_changes::Hooks,
    |value| {
        TableRows::RelationOpinions(vec![RelationOpinionRow {
            owner_country: 1,
            target_country: 2,
            opinion: value,
        }])
    }
);
fresh_fallback!(
    advisor_install_failure_keeps_fresh_roles,
    collector_characters::advisor_changes::Hooks,
    |value| {
        TableRows::CharacterRoleTraits(vec![CharacterRoleTraitRow {
            character: 7,
            tokens: vec![value as u32],
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    relation_install_failure_keeps_fresh_relations,
    collector_air_relations::relation_changes::Hooks,
    |value| {
        TableRows::AirCountryRelations(vec![AirCountryRelationRow {
            country: 1,
            other: value as u32,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    history_install_failure_keeps_fresh_histories,
    collector_air_histories::history_changes::Hooks,
    |value| {
        TableRows::AirNavalPatrolHistories(vec![AirNavalPatrolHistoryRow {
            country: 1,
            enemy_ships_sunk_raw: i64::from(value),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    flag_install_failure_keeps_fresh_flags,
    collector_intel::flag_changes::Hooks,
    |value| {
        TableRows::ScriptFlags(vec![ScriptFlagRow {
            scope_id: 1,
            token: 7,
            value: value as i16,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    building_install_failure_keeps_fresh_buildings,
    collector_construction::building_changes::Hooks,
    |value| {
        TableRows::StateBuildings(vec![BuildingRow {
            location: 1,
            level: value as i16,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    advisor_pool_install_failure_keeps_fresh_pools,
    collector_observers::advisor_pool_changes::Hooks,
    |value| {
        TableRows::AvailableAdvisors(vec![AvailableAdvisorRow {
            country: 1,
            character: value as u32,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    party_leader_install_failure_keeps_fresh_leaders,
    collector_governance::party_leader_changes::Hooks,
    |value| {
        TableRows::PartyLeaders(vec![PartyLeaderRow {
            country: 1,
            party: 7,
            leader_id: value,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    variable_install_failure_keeps_fresh_variables,
    collector_intel::variable_changes::Hooks,
    |value| {
        TableRows::ScriptVariables(vec![ScriptVariablesRow {
            scope_id: 1,
            seed: Recorded::Value(Some(value as u32)),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    party_install_failure_keeps_fresh_parties,
    collector_governance::party_changes::Hooks,
    |value| {
        TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 1,
            id: 7,
            popularity: i64::from(value),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    wing_install_failure_keeps_fresh_equipment,
    collector_observers::wing_changes::Hooks,
    |value| {
        TableRows::WingEquipment(vec![WingEquipmentRow {
            wing: 7,
            amount_raw: i64::from(value),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    roster_install_failure_keeps_fresh_rosters,
    collector_observers::roster_changes::Hooks,
    |value| {
        TableRows::CountryCharacters(vec![CountryCharacterRow {
            country: 1,
            character: Some(value as u32),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    political_status_install_failure_keeps_fresh_politics,
    collector_observers::political_status_changes::Hooks,
    |value| {
        TableRows::Politics(vec![PoliticsRow {
            country: 1,
            last_election: value,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    rail_install_failure_keeps_fresh_rails,
    collector_observers::rail_changes::Hooks,
    |value| {
        TableRows::RailCooldowns(vec![RailCooldownRow {
            province: 7,
            cooldown_ticks: value,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    territory_install_failure_keeps_fresh_territory,
    collector_observers::territory_changes::Hooks,
    |value| {
        TableRows::States(vec![StateRow {
            id: 7,
            manpower_available: value,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    army_stat_install_failure_keeps_fresh_inputs,
    collector_division_parts::army_stat_changes::Hooks,
    |value| {
        TableRows::DivisionLandInputs(vec![DivisionLandInputsRow {
            division: 7,
            naval_transfer: value == 29,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    country_input_install_failure_keeps_fresh_inputs,
    collector_observers::country_input_changes::Hooks,
    |value| {
        TableRows::CountryStabilityInputs(vec![CountryStabilityInputRow {
            country: 1,
            stability_raw: i64::from(value),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    naval_tree_install_failure_keeps_fresh_trees,
    collector_observers::naval_tree_changes::Hooks,
    |value| {
        TableRows::NavalModifierTrees(vec![NavalModifierTreeRow {
            owner: 1,
            trees: vec![NavalModifierKeyTree {
                key: value as u32,
                nodes: vec![],
            }],
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    unit_leader_install_failure_keeps_fresh_experience,
    collector_observers::unit_leader_changes::Hooks,
    |value| {
        TableRows::UnitLeaderTraitExperience(vec![UnitLeaderTraitExperienceRow {
            leader_reference: 7,
            experience: i64::from(value),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    census_install_failure_keeps_fresh_roles,
    collector_observers::census_changes::Hooks,
    |value| {
        TableRows::CharacterRoleTraits(vec![CharacterRoleTraitRow {
            character: 7,
            tokens: vec![value as u32],
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    orders_group_install_failure_keeps_fresh_selections,
    collector_observers::orders_group_changes::Hooks,
    |value| {
        TableRows::DivisionRetreatBans(vec![DivisionRetreatBanRow {
            division: 7,
            owner: 1,
            bans: value as u8,
        }])
    }
);
fresh_fallback!(
    air_record_install_failure_keeps_fresh_records,
    collector_air_records::air_record_changes::Hooks,
    |value| {
        TableRows::AirRegionCombat(vec![AirRegionCombatRow {
            country: 1,
            region: 7,
            owner: value as u32,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    local_subunit_install_failure_keeps_fresh_requirements,
    collector_model_inputs::local_subunit_changes::Hooks,
    |value| {
        TableRows::CountryLocalSubunits(vec![CountryLocalSubunitRow {
            country: 1,
            token: 7,
            requirements: vec![LocalSubunitRequirement {
                archetype_token: 8,
                amount_raw: i64::from(value),
            }],
        }])
    }
);
fresh_fallback!(
    ship_model_install_failure_keeps_fresh_models,
    collector_ship_models::ship_model_changes::Hooks,
    |value| {
        TableRows::ShipModels(vec![ShipModelRow {
            ship: 7,
            model_number: value,
            air_capacity_kind: Recorded::Value(None),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    province_unit_install_failure_keeps_fresh_order,
    collector_divisions::province_unit_changes::Hooks,
    |value| {
        TableRows::ProvinceUnitOrder(vec![ProvinceUnitOrderRow {
            province: 7,
            mixed: vec![value as u64],
            naval: vec![],
        }])
    }
);
fresh_fallback!(
    air_base_install_failure_keeps_fresh_registrations,
    collector_observers::air_base_carry_changes::Hooks,
    |value| {
        TableRows::AirBaseRegistrations(vec![AirBaseRegistrationRow {
            base: 7,
            slot: 1,
            capacity_raw: i64::from(value),
            ..Default::default()
        }])
    }
);

fresh_fallback!(
    raid_install_failure_keeps_fresh_raids,
    collector_observers::raid_changes::Hooks,
    |value| {
        TableRows::Raids(vec![RaidRow {
            id: 7,
            preparation_hours: value,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    mission_install_failure_keeps_fresh_regional_ordinals,
    collector_missions::mission_changes::Hooks,
    |value| {
        TableRows::AirRegionalMissions(vec![AirRegionalMissionRow {
            wing: 7,
            ordinal: value as u32,
            active_order: None,
        }])
    }
);
fresh_fallback!(
    world_input_install_failure_keeps_fresh_province_names,
    collector_observers::world_input_changes::Hooks,
    |value| {
        TableRows::ProvinceNames(vec![ProvinceNameRow {
            province: 7,
            name: value.to_le_bytes().to_vec(),
        }])
    }
);

fresh_fallback!(
    equipment_observer_install_failure_keeps_fresh_rosters,
    collector_equipment_destruction::equipment_destruction::Hooks,
    |value| {
        TableRows::AvailableEquipmentVariants(vec![AvailableEquipmentVariantRow {
            country: 1,
            variant: (value as u64) << 32 | 70,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    prune_install_failure_keeps_fresh_logs_and_false_coverage,
    collector_combat_log::combat_log_prune::Hooks,
    |value| {
        TableRows::CombatLogState(vec![CombatLogStateRow {
            viewing_country_raw: value,
            prune_observed: 0,
        }])
    }
);
fresh_fallback!(
    fuel_install_failure_keeps_fresh_stock,
    collector_fuel_awards::fuel_awards::Hooks,
    |value| {
        TableRows::CountryFuel(vec![CountryFuelRow {
            country: 1,
            current_raw: i64::from(value),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    air_loss_install_failure_keeps_fresh_rosters,
    collector_observers::air_losses::Hooks,
    |value| {
        TableRows::AirDailyRoster(vec![AirDailyRosterRow {
            country: 1,
            eligible: value == 29,
        }])
    }
);
fresh_fallback!(
    propaganda_install_failure_keeps_fresh_bootstrap,
    collector_intel::propaganda_changes::Hooks,
    |value| {
        TableRows::PropagandaEfforts(vec![PropagandaEffortRow {
            slot: 1,
            country: 1,
            stability: i64::from(value),
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    market_install_failure_keeps_fresh_full_snapshot,
    collector_observers::market_changes::Hooks,
    |value| {
        TableRows::CountryMarket(vec![CountryMarketRow {
            country: 1,
            subsidy_total_raw: i64::from(value),
            ..Default::default()
        }])
    }
);

#[test]
fn missing_fuel_awards_keep_fresh_stock_and_the_real_window_refusal() {
    if !contract_tests::isolated(
        "hook_failures::fresh_reads_tests::missing_fuel_awards_keep_fresh_stock_and_the_real_window_refusal",
    ) {
        return;
    }
    contract_tests::reset_session();
    FamilyHooks::<collector_fuel_awards::fuel_awards::Hooks>::construct(0, || {
        Err("synthetic relay allocation failure")
    })
    .unwrap();
    let mut awards = collector_fuel_awards::tables::Captured::vacant();
    let mut windows = collector_fuel_awards::tables::Windows::default();
    for (hour, current) in [(10, 130_000), (11, 290_000)] {
        awards.drain();
        let mut frame = source(hour);
        frame.tables[TableId::CountryFuel.index()] =
            TableView::Complete(TableRows::CountryFuel(vec![CountryFuelRow {
                country: 1,
                current_raw: current,
                ..Default::default()
            }]))
            .into();
        windows
            .assemble(1, hour, &awards, &mut frame.tables)
            .unwrap();
        let expected = frame.tables[TableId::CountryFuel.index()].clone();
        apply(&mut frame).unwrap();
        assert_eq!(frame.tables[TableId::CountryFuel.index()], expected);
        let TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(rows))) =
            &frame.tables[TableId::CountryFuel.index()]
        else {
            panic!("fresh fuel stock was discarded")
        };
        assert_eq!(rows[0].current_raw, current);
        let Recorded::Value(day) = &rows[0].award_day else {
            panic!("missing fuel window refusal")
        };
        assert!(!day.reason.is_empty());
    }
}

fresh_fallback!(
    land_install_failure_keeps_incomplete_coverage,
    collector_land_events::land_events::Hooks,
    |value| {
        TableRows::LandEventCoverage(vec![LandEventCoverageRow {
            complete: false,
            to_hour: value,
            lost_events: 1,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    politics_install_failure_keeps_unobserved_coverage,
    collector_politics::politics_changes::Hooks,
    |value| {
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 1,
            streams: 0,
            through_hour: value,
            ..Default::default()
        }])
    }
);
fresh_fallback!(
    design_install_failure_keeps_independent_fog_context,
    collector_production::production::design_changes::Hooks,
    |value| {
        TableRows::DesignObserverContext(vec![DesignObserverContextRow {
            fog: u8::from(value == 29),
        }])
    }
);

#[test]
fn missing_land_observer_keeps_actual_incomplete_coverage_and_refuses_events() {
    if !contract_tests::isolated(
        "hook_failures::fresh_reads_tests::missing_land_observer_keeps_actual_incomplete_coverage_and_refuses_events",
    ) {
        return;
    }
    contract_tests::reset_session();
    FamilyHooks::<collector_land_events::land_events::Hooks>::construct(0, || {
        Err("synthetic relay allocation failure")
    })
    .unwrap();
    let mut capture = collector_land_events::extended::land_events::Captured::vacant();
    for hour in [10, 11] {
        // SAFETY: Isolated contract test owns this capture and the land-event observer boundary.
        unsafe { capture.drain(hour, false) };
        let mut frame = source(hour);
        for (table, value) in collector_land_events::extended::land_events::TABLES
            .into_iter()
            .zip(capture.tables())
        {
            frame.tables[table.index()] = value.into();
        }
        let expected = frame.tables[TableId::LandEventCoverage.index()].clone();
        apply(&mut frame).unwrap();
        assert_eq!(frame.tables[TableId::LandEventCoverage.index()], expected);
        let TableInput::Sampled(TableView::Complete(TableRows::LandEventCoverage(rows))) =
            &frame.tables[TableId::LandEventCoverage.index()]
        else {
            panic!("lost incomplete interval coverage")
        };
        assert!(!rows[0].complete);
        assert_eq!(rows[0].to_hour, hour);
        for table in [
            TableId::LandCombatResults,
            TableId::LandCombatResultObservers,
            TableId::DestroyedDivisions,
        ] {
            assert!(matches!(
                &frame.tables[table.index()],
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
    }
}

fresh_fallback!(
    mission_install_failure_keeps_current_owner_and_unknown_ground_crew_flag,
    collector_missions::mission_changes::Hooks,
    |value| {
        TableRows::AirGroundCrews(vec![AirGroundCrewRow::without_supply(
            value as u32,
            7,
            None,
        )])
    }
);

pub(super) struct Native(pub(super) Vec<u8>);
impl Native {
    pub(super) fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }
    pub(super) fn word(&mut self, at: usize, value: usize) {
        self.put(at, &(value as u64).to_le_bytes());
    }
    pub(super) fn array(&mut self, at: usize, data: usize, count: u32) {
        self.word(at, data);
        self.put(at + 8, &count.to_le_bytes());
        self.put(at + 12, &count.to_le_bytes());
    }
}
impl engine_native_memory::Memory for Native {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        if at == engine_native_parts::csub_unit_definition::DATABASE_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&0x9800_u64.to_le_bytes());
            return true;
        }
        engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &self.0,
        }
        .copy(at, output)
    }
}

#[test]
fn failed_local_observer_reads_changed_native_requirements_and_keeps_dependent_inputs() {
    use collector_model_inputs::local_subunit_changes::Changes;
    use engine_deployment::cdeployment_status;
    use engine_native::{ccountry, cgame_state};
    use engine_native_parts::{cequipment_type, csub_unit_definition};
    if !contract_tests::isolated(
        "hook_failures::fresh_reads_tests::failed_local_observer_reads_changed_native_requirements_and_keeps_dependent_inputs",
    ) {
        return;
    }
    contract_tests::reset_session();
    FamilyHooks::<collector_model_inputs::local_subunit_changes::Hooks>::construct(0, || {
        Err("synthetic relay allocation failure")
    })
    .unwrap();
    assert!(failed(Family::LocalSubunits));
    assert!(failed(Family::ArmyStats));
    assert!(
        collector_model_inputs::local_subunit_changes::Observer
            .quiet()
            .is_none()
    );
    assert!(
        collector_model_inputs::local_subunit_changes::Observer
            .sequence(0x5400)
            .is_none()
    );
    let mut memory = Native(vec![0; 0x14000]);
    let (state, country, status, global, clone, archetype) =
        (0x1000, 0x4000, 0x5400, 0xb000, 0x11000, 0xa000);
    memory.array(state + cgame_state::COUNTRIES, 0x2000, 2);
    memory.array(state + cgame_state::COUNTRY_LINKS, 0x2100, 2);
    memory.word(0x2008, country);
    memory.put(0x2104, &1_u32.to_le_bytes());
    memory.word(country, ccountry::VTABLE_RVA.0);
    memory.put(country + ccountry::ID, &1_u32.to_le_bytes());
    memory.word(country + ccountry::DEPLOYMENT_MANAGER, status);
    memory.word(status, cdeployment_status::VTABLE_RVA.0);
    memory.word(status + cdeployment_status::OWNER, country);
    memory.array(status + cdeployment_status::LOCAL_SUBUNITS, 0x5500, 1);
    memory.word(0x5500, clone);
    memory.array(0x9800 + csub_unit_definition::DATABASE_VECTOR, 0x9900, 2);
    memory.word(0x9908, global);
    memory.word(archetype, cequipment_type::VTABLE_RVA.0);
    memory.put(archetype + cequipment_type::TOKEN, &71_u32.to_le_bytes());
    memory.put(archetype + cequipment_type::ACTIVE, &[1]);
    for (pointer, amount) in [(global, 100_000_i64), (clone, 200_000)] {
        memory.word(pointer, csub_unit_definition::VTABLE_RVA.0);
        memory.put(pointer + csub_unit_definition::TOKEN, &5_u32.to_le_bytes());
        memory.put(pointer + csub_unit_definition::IS_REAL, &[1]);
        memory.put(
            pointer + csub_unit_definition::DATABASE_INDEX,
            &1_i32.to_le_bytes(),
        );
        memory.array(
            pointer + csub_unit_definition::ARCHETYPE_REQUIREMENTS_DATA,
            pointer + 0x700,
            1,
        );
        memory.word(pointer + 0x700, archetype);
        memory.put(pointer + 0x708, &amount.to_le_bytes());
    }
    let mut capture = collector_model_inputs::model_inputs::Captured::vacant();
    let mut store = collector_model_inputs::model_inputs::LocalStore::default();
    for (hour, amount) in [(10, 200_000_i64), (11, 300_000)] {
        memory.put(clone + 0x708, &amount.to_le_bytes());
        capture.capture_carried(&memory, 0, state, hour, false, Some(&mut store));
        let mut frame = source(hour);
        frame.tables[TableId::CountryLocalSubunits.index()] = capture.tables()[0].clone().into();
        frame.tables[TableId::DivisionLandInputs.index()] =
            TableView::Complete(TableRows::DivisionLandInputs(vec![DivisionLandInputsRow {
                division: 7,
                naval_transfer: hour == 11,
                ..Default::default()
            }]))
            .into();
        let dependent = frame.tables[TableId::DivisionLandInputs.index()].clone();
        apply(&mut frame).unwrap();
        let TableInput::Sampled(TableView::Complete(TableRows::CountryLocalSubunits(rows))) =
            &frame.tables[TableId::CountryLocalSubunits.index()]
        else {
            panic!("fresh local requirements were discarded")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].requirements[0].amount_raw, amount);
        assert_eq!(frame.tables[TableId::DivisionLandInputs.index()], dependent);
    }
    memory.word(country + ccountry::DEPLOYMENT_MANAGER, 0);
    capture.capture_carried(&memory, 0, state, 12, true, Some(&mut store));
    let mut frame = source(12);
    frame.tables[TableId::CountryLocalSubunits.index()] = capture.tables()[0].clone().into();
    let refused = frame.tables[TableId::CountryLocalSubunits.index()].clone();
    assert!(!matches!(
        refused,
        TableInput::Sampled(TableView::Complete(_))
    ));
    apply(&mut frame).unwrap();
    assert_eq!(frame.tables[TableId::CountryLocalSubunits.index()], refused);
}
