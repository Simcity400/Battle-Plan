use super::*;
use collector_extended_support::outcomes::failed_capture;
use engine_native::carmy;

pub(super) fn stage_fixture_acquisitions(
    captured: &mut Captured,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    hour: i32,
    force_full: bool,
) {
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
    macro_rules! staggered {
        ($ticks:expr, $($pack:ident),+ $(,)?) => {$(
            scatter(
                &$pack::TABLES,
                captured.$pack.capture(memory, base, game_state, hour, force_full),
                $ticks,
            );
        )+};
    }
    macro_rules! stage {
        ($slot:ident, $($pack:ident),+ $(,)?) => {
            if captured.$slot.is_none() {
                let mut ticks = [0; TABLE_COUNT];
                staggered!(&mut ticks, $($pack),+);
                captured.$slot = Some(ticks);
            }
        };
    }
    if captured.slow_ticks.is_none() {
        let mut ticks = [0; TABLE_COUNT];
        staggered!(
            &mut ticks,
            peace_bids,
            map_names,
            faction_theatres,
            model_inputs,
            modifier_definitions,
            country_tactic_unlocks,
            logistics,
            country_factories,
            state_building_inputs,
            visibility,
        );
        // SAFETY: Fixture staging runs under isolated fixture control, as the sole capture pass.
        let cost = unsafe { captured.hired_advisors.capture(memory, base, game_state, hour, force_full) };
        scatter(&hired_advisors::TABLES, cost, &mut ticks);
        // SAFETY: Fixture staging runs under isolated fixture control, as the sole capture pass.
        let cost = unsafe { captured.combat_log.capture(memory, base, game_state, hour, force_full) };
        scatter(&combat_log::TABLES, cost, &mut ticks);
        captured.slow_ticks = Some(ticks);
    }
    if captured.stability_ticks.is_none() && captured.records_any(&country_stability::TABLES) {
        let mut ticks = [0; TABLE_COUNT];
        scatter(
            &country_stability::TABLES,
            captured
                .country_stability
                .capture(memory, base, game_state, hour, force_full, None),
            &mut ticks,
        );
        captured.stability_ticks = Some(ticks);
    }
    if captured.production_daily_ticks.is_none()
        && ProductionDailyPacks::recorded(captured.capture_mode)
        && ProductionDailyPacks::due(hour, force_full)
    {
        let mut ticks = [0; TABLE_COUNT];
        captured.assemble_production_daily(
            memory, base, game_state, hour, force_full, &mut ticks, true,
        );
        captured.production_daily_ticks = Some(ticks);
    }
    if captured.records_any(&war_overview::TABLES) {
        stage!(war_overview_ticks, war_overview);
    }
    if captured.template_ticks.is_none() {
        // SAFETY: Fixture staging runs under isolated fixture control, as the sole capture pass.
        captured.template_ticks = Some(unsafe {
            captured
                .division_templates
                .capture(memory, base, game_state, hour, force_full)
        });
    }
    if captured.raid_state_ticks.is_none() {
        let mut ticks = [0; TABLE_COUNT];
        staggered!(&mut ticks, raid_state_inputs);
        scatter(
            &resource_map::TABLES,
            captured
                .resource_map
                .capture(memory, base, game_state, hour, force_full, None),
            &mut ticks,
        );
        captured.raid_state_ticks = Some(ticks);
    }
    if captured.records_any(&raid_target_statuses::TABLES) {
        stage!(raid_targets_ticks, raid_target_statuses);
    }
    stage!(raid_launch_ticks, raid_launch_inputs);
    stage!(gated_ticks, fog, construction, projects);
    if captured.character_inputs_ticks.is_none() {
        captured.character_inputs_ticks = Some(
            captured
                .character_inputs
                .capture(memory, base, game_state, hour, force_full),
        );
    }
    if captured.supply_trains_report.is_none() && captured.records_any(&supply_trains::TABLES) {
        let context = supply_trains::CaptureContext {
            base,
            game_state,
            hour,
            force_full,
        };
        captured.supply_trains_report = Some(captured.supply_trains.capture_owned(memory, context));
    }
    if captured.character_portraits_report.is_none() {
        let context = character_portraits::CaptureContext { base, game_state };
        captured.character_portraits_report =
            Some(captured.character_portraits.capture_owned(memory, context));
    }
    if captured.province_unit_order_report.is_none() {
        let context = province_unit_order::CaptureContext { base, game_state };
        captured.province_unit_order_report = Some(
            captured
                .province_unit_order
                .capture_owned(memory, context, None, None),
        );
    }
}

pub(super) fn stage_fixture_late(
    captured: &mut Captured,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    hour: i32,
    force_full: bool,
    orders: &mut Orders,
) {
    if captured.late_ticks.is_some() {
        return;
    }
    let _phase = collector_memory::terminal::enter(collector_memory::terminal::Phase::Extended);
    let mut packs = captured.late_spare.take().expect("late spare present");
    captured.swap_late(&mut packs, orders);
    packs.capture(
        memory,
        base,
        game_state,
        hour,
        force_full,
        captured.unit_inputs.as_deref(),
        Some(&captured.divisions),
        Some(&mut engine_raid_inputs::raid_air_scores::Held::default()),
        None,
    );
    packs.ticks[TableId::DivisionMovement.index()] += captured.plans.capture_unit_transports(
        memory,
        base,
        game_state,
        captured.unit_inputs.as_deref(),
        &packs.orders,
    );
    captured.late_ticks = Some(packs.ticks);
    captured.swap_late(&mut packs, orders);
    captured.late_spare = Some(packs);
    captured
        .plans
        .install_order_transport(&mut captured.division_presentation);
}

#[test]
fn periodic_live_input_sizing_keeps_land_events_prompt_and_skips_worker_hints() {
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let mut capacities = Capacities::default();
    assert!(
        // SAFETY: This is the session's sizing pass, with its observer storage still live.
        unsafe { capacities.presize_live_inputs_periodic(&Unreadable, 0, false) }.is_err()
    );
    assert!(
        // SAFETY: The sizing owner prepares the session's live storage before callbacks or in its
        // sizing pass.
        unsafe { capacities.presize_live_inputs(&Unreadable, 0) }.is_err()
    );
}

#[test]
fn final_worker_unrun_continuation_restores_storage_and_raw_verdicts_without_capture() {
    let mut captured = Captured::with_capacities(&Capacities {
        division_sources: 5_000,
        ..Default::default()
    });
    captured.characters.raw_active = [true; characters::N];
    captured.session.raw_mode = true;
    captured.intel.capitulation_raw_mode = true;
    captured.intel.relations_raw_mode = true;
    let source_storage = captured.divisions.first.as_ptr();
    let bytes = captured.allocated_bytes();
    let outcomes = captured.outcomes();
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    collector_test_support::with_allocations_denied(|| {
        let request = captured.take_final_inputs_request(&countries, 0, 0, 41, true);
        captured.return_final_inputs_request(request);
    });
    assert_eq!(captured.divisions.first.as_ptr(), source_storage);
    assert_eq!(captured.allocated_bytes(), bytes);
    assert_eq!(captured.outcomes(), outcomes);
    assert_eq!(captured.characters.raw_active, [true; characters::N]);
    assert!(captured.session.raw_mode);
    assert!(captured.intel.capitulation_raw_mode && captured.intel.relations_raw_mode);
    assert!(captured.final_inputs_ticks.is_none());
}

#[test]
fn final_worker_loan_preserves_mod_sized_storage_without_callback_allocation() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.divisions = Divisions::new(5_000);
    let source_storage = captured.divisions.first.as_ptr();
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    collector_test_support::with_allocations_denied(|| {
        let request = captured.take_final_inputs_request(&countries, 0, 0, 41, true);
        assert_eq!(captured.divisions.first.capacity(), 0);
        assert_eq!(captured.economy.allocated_bytes(), 0);
        assert_eq!(captured.characters.allocated_bytes(), 0);
        captured.restore_final_inputs_request(
            request,
            FinalInputsReport::default(),
            Some("final worker admission failed"),
        );
    });
    assert_eq!(captured.divisions.first.as_ptr(), source_storage);
    assert_eq!(
        captured.new_divisions.outcomes(),
        [Outcome::Unavailable {
            reason: "final worker admission failed",
        }]
    );
}

#[test]
fn vacant_spares_restore_storage_and_verdicts_after_success_and_failure() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let mut orders = Orders::vacant();
    orders.geometry = vec![11, 13, 17];
    orders.geometry_len = 3;
    orders.root_outcome = Outcome::Rows { len: 3 };
    let order_storage = orders.geometry.as_ptr();
    let division_storage = captured.divisions.first.as_ptr();
    let box_bytes = [
        size_of::<OrdinaryPacks>() + OrdinaryPart::spare_box_bytes(),
        size_of::<SlowPacks>(),
        size_of::<LatePacks>(),
        size_of::<GatedPacks>(),
        size_of::<air::Captured>(),
        size_of::<RaidStatePacks>(),
        size_of::<RaidLaunchPacks>(),
        size_of::<RaidTargetPacks>(),
        size_of::<ScoresRailsParts>(),
        size_of::<StabilityPacks>(),
        size_of::<WarOverviewPacks>(),
        size_of::<ProductionDailyPacks>(),
        size_of::<engine_map_air::map_air_supply::Capture>(),
    ];
    let assert_vacant = |captured: &Captured| {
        assert_eq!(
            [
                captured.ordinary_spare.as_ref().unwrap().allocated_bytes()
                    + captured
                        .ordinary_part_spares
                        .iter()
                        .flatten()
                        .map(OrdinaryPart::allocated_bytes)
                        .sum::<usize>(),
                captured.slow_spare.as_ref().unwrap().allocated_bytes(),
                captured.late_spare.as_ref().unwrap().allocated_bytes(),
                captured.gated_spare.as_ref().unwrap().allocated_bytes(),
                size_of::<air::Captured>()
                    + captured.missions_spare.as_ref().unwrap().allocated_bytes(),
                captured
                    .raid_state_spare
                    .as_ref()
                    .unwrap()
                    .allocated_bytes(),
                captured
                    .raid_launch_spare
                    .as_ref()
                    .unwrap()
                    .allocated_bytes(),
                captured
                    .raid_targets_spare
                    .as_ref()
                    .unwrap()
                    .allocated_bytes(),
                captured
                    .scores_rails_spare
                    .as_ref()
                    .unwrap()
                    .allocated_bytes(),
                captured.stability_spare.as_ref().unwrap().allocated_bytes(),
                captured
                    .war_overview_spare
                    .as_ref()
                    .unwrap()
                    .allocated_bytes(),
                captured
                    .production_daily_spare
                    .as_ref()
                    .unwrap()
                    .allocated_bytes(),
                size_of::<engine_map_air::map_air_supply::Capture>()
                    + captured
                        .air_supply_spare
                        .as_ref()
                        .unwrap()
                        .allocated_bytes(),
            ],
            box_bytes
        );
        assert_eq!(captured.spare_bytes(), box_bytes.iter().sum::<usize>());
        assert_eq!(
            captured
                .allocated_by_pack()
                .unwrap()
                .iter()
                .map(|(_, bytes)| bytes)
                .sum::<usize>(),
            captured.allocated_bytes()
        );
    };
    assert_vacant(&captured);
    let bytes = captured.allocated_bytes();
    let mut removed = [0; 4];
    for completed in [true, false, true] {
        captured.drop_and_reset();
        let ordinary_box = std::ptr::from_ref(&**captured.ordinary_spare.as_ref().unwrap());
        let parts = captured.take_ordinary_parts(
            &countries,
            11,
            0x1000,
            0,
            Ok(()),
            crate::extended::NavalMissionLoan::default(),
        );
        assert!(captured.ordinary_part_spares.iter().all(Option::is_none));
        let request = captured.take_ordinary_request(&countries, 11, 0x1000, 0);
        assert_eq!(std::ptr::from_ref(&*request.packs), ordinary_box);
        let part_bytes = parts
            .iter()
            .map(|part| part.allocated_bytes())
            .sum::<usize>();
        assert_eq!(
            captured.allocated_bytes() + request.allocated_bytes() + part_bytes,
            bytes
        );
        removed[0] = request.allocated_bytes() + part_bytes - box_bytes[0];
        captured.restore_ordinary_request(
            request,
            (!completed).then_some("fixture failure"),
            parts.map(|part| (part, None)),
        );
        assert_eq!(
            std::ptr::from_ref(&**captured.ordinary_spare.as_ref().unwrap()),
            ordinary_box
        );
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);

        let slow_box = std::ptr::from_ref(&**captured.slow_spare.as_ref().unwrap());
        let request =
            captured.take_slow_request(&countries, 11, 0x1000, 9, false, SlowStores::default());
        assert_eq!(std::ptr::from_ref(&*request.packs), slow_box);
        assert_eq!(
            captured.allocated_bytes() + request.packs.allocated_bytes(),
            bytes
        );
        removed[1] = request.packs.allocated_bytes() - box_bytes[1];
        captured.restore_slow_request(request, completed);
        assert_eq!(
            std::ptr::from_ref(&**captured.slow_spare.as_ref().unwrap()),
            slow_box
        );
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);

        let stability_box = std::ptr::from_ref(&**captured.stability_spare.as_ref().unwrap());
        let request = captured.take_stability_request(&countries, 11, 0x1000, 9, false, None);
        assert_eq!(std::ptr::from_ref(&*request.packs), stability_box);
        assert_eq!(
            captured.allocated_bytes() + request.packs.allocated_bytes(),
            bytes
        );
        captured.restore_stability_request(request, completed);
        assert_eq!(
            std::ptr::from_ref(&**captured.stability_spare.as_ref().unwrap()),
            stability_box
        );
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);

        let war_overview_box = std::ptr::from_ref(&**captured.war_overview_spare.as_ref().unwrap());
        let request = captured.take_war_overview_request(&countries, 11, 0x1000, 9, false);
        assert_eq!(std::ptr::from_ref(&*request.packs), war_overview_box);
        assert_eq!(
            captured.allocated_bytes() + request.packs.allocated_bytes(),
            bytes
        );
        captured.restore_war_overview_request(request, completed);
        assert_eq!(
            std::ptr::from_ref(&**captured.war_overview_spare.as_ref().unwrap()),
            war_overview_box
        );
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);

        let daily_box = std::ptr::from_ref(&**captured.production_daily_spare.as_ref().unwrap());
        let mio = collector_test_support::with_allocations_denied(|| {
            captured.take_industrial_organisations_request(&countries, 11, 0x1000, 8, false)
        });
        let mut request = captured.take_production_daily_request(&countries, 11, 0x1000, 8, false);
        assert_eq!(std::ptr::from_ref(&*request.packs), daily_box);
        assert_eq!(
            captured.allocated_bytes()
                + request.packs.allocated_bytes()
                + mio.captured.allocated_bytes(),
            bytes
        );
        collector_test_support::with_allocations_denied(|| {
            captured.restore_industrial_organisations_request(
                mio,
                completed,
                [0; industrial_organisations::N],
                &mut request.packs,
            );
        });
        captured.restore_production_daily_request(request, completed);
        assert_eq!(
            std::ptr::from_ref(&**captured.production_daily_spare.as_ref().unwrap()),
            daily_box
        );
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);
        let mio = collector_test_support::with_allocations_denied(|| {
            captured.take_industrial_organisations_request(&countries, 11, 0x1000, 8, false)
        });
        let mut request = captured.take_production_daily_request(&countries, 11, 0x1000, 8, false);
        captured.restore_industrial_organisations_request(
            mio,
            completed,
            [0; industrial_organisations::N],
            &mut request.packs,
        );
        captured.restore_production_daily_request(request, completed);
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);

        captured.fog.radar_raw_mode = true;
        captured.fog.intel_raw_mode = true;
        captured.fog.registrations_raw_mode = true;
        captured.projects.raw_mode = true;
        captured.construction.rest_raw_mode = true;
        let gated_box = std::ptr::from_ref(&**captured.gated_spare.as_ref().unwrap());
        let request = captured.take_gated_request(
            11,
            0x1000,
            9,
            false,
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            None,
        );
        assert_eq!(std::ptr::from_ref(&*request.packs), gated_box);
        assert_eq!(
            captured.allocated_bytes() + request.packs.allocated_bytes(),
            bytes
        );
        removed[3] = request.packs.allocated_bytes() - box_bytes[3];
        assert!(request.packs.fog.radar_raw_mode);
        assert!(request.packs.fog.intel_raw_mode);
        assert!(request.packs.fog.registrations_raw_mode);
        assert!(request.packs.projects.raw_mode);
        assert!(request.packs.construction.rest_raw_mode);
        assert!(!captured.fog.radar_raw_mode);
        assert!(!captured.projects.raw_mode);
        assert!(!captured.construction.rest_raw_mode);
        captured.restore_gated_request(request, completed);
        assert_eq!(
            std::ptr::from_ref(&**captured.gated_spare.as_ref().unwrap()),
            gated_box
        );
        assert!(captured.fog.radar_raw_mode);
        assert!(captured.fog.intel_raw_mode);
        assert!(captured.fog.registrations_raw_mode);
        assert!(captured.projects.raw_mode);
        assert!(captured.construction.rest_raw_mode);
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);

        let missions_box = std::ptr::from_ref(&**captured.missions_spare.as_ref().unwrap());
        captured.air.stats_raw_mode = true;
        let request = captured.take_air_missions_request(&countries, 11, 0x1000, None);
        assert_eq!(std::ptr::from_ref(&*request.pack), missions_box);
        assert!(request.pack.stats_raw_mode && !captured.air.stats_raw_mode);
        assert_eq!(request.histories.len(), air_histories::PARTS);
        captured.restore_air_missions_request(request, completed);
        assert_eq!(
            std::ptr::from_ref(&**captured.missions_spare.as_ref().unwrap()),
            missions_box
        );
        assert!(captured.air.stats_raw_mode);
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);

        captured.naval_history.raw_mode = true;
        let order_bytes = orders.allocated_bytes();
        let late_box = std::ptr::from_ref(&**captured.late_spare.as_ref().unwrap());
        let request =
            captured.take_late_request(&countries, &mut orders, true, 11, 0x1000, 9, false);
        assert_eq!(std::ptr::from_ref(&*request.packs), late_box);
        assert_eq!(
            captured.allocated_bytes()
                + orders.allocated_bytes()
                + request.packs.allocated_bytes()
                + request.plans.allocated_bytes(),
            bytes + order_bytes
        );
        removed[2] =
            request.packs.allocated_bytes() - box_bytes[2] - request.packs.orders.allocated_bytes();
        assert!(request.packs.air.stats_raw_mode);
        assert!(request.packs.naval_history.raw_mode);
        assert_eq!(request.packs.orders.geometry.as_ptr(), order_storage);
        assert_eq!(captured.divisions.first.as_ptr(), division_storage);
        assert!(!captured.air.stats_raw_mode);
        assert!(!captured.naval_history.raw_mode);
        captured.restore_late_request(request, completed, &mut orders);
        assert_eq!(
            std::ptr::from_ref(&**captured.late_spare.as_ref().unwrap()),
            late_box
        );
        assert!(captured.air.stats_raw_mode);
        assert!(captured.naval_history.raw_mode);
        assert_eq!(orders.geometry.as_ptr(), order_storage);
        assert_eq!(orders.geometry, [11, 13, 17]);
        assert_eq!(orders.root_outcome, Outcome::Rows { len: 3 });
        assert_eq!(captured.divisions.first.as_ptr(), division_storage);
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_vacant(&captured);
    }
    let total: usize = removed.iter().sum();
    println!(
        "Removed default spare payload bytes (Ordinary/Slow/Late/Gated): {removed:?}; \
         per frame {total}; four frames {}; retained box bytes per frame {}",
        total * 4,
        box_bytes.iter().sum::<usize>()
    );
}

#[test]
fn late_handoff_retains_only_the_current_input_lease() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let mut orders = Orders::vacant();
    for pointer in [0x1234, 0x5678] {
        let inputs = engine_allocation::shared(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer,
                ..Default::default()
            }],
            Outcome::Rows { len: 1 },
        ))
        .unwrap();
        captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&inputs)));
        let request =
            captured.take_late_request(&countries, &mut orders, true, 11, 0x1000, 9, false);
        assert!(engine_allocation::SharedBox::ptr_eq(
            request.inputs.as_ref().unwrap(),
            &inputs
        ));
        assert_eq!(request.inputs.as_ref().unwrap().seeds()[0].pointer, pointer);
        assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 3);
        captured.restore_late_request(request, true, &mut orders);
        assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 2);
        captured.release_unit_inputs();
        assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 1);
    }
}

#[test]
fn presentation_handoff_restores_storage_and_exact_source_leases() {
    let mut capacity = Capacities::default();
    capacity
        .division_presentation
        .raise_for_census(carmy::PresentationDemand {
            rows: 1,
            ..Default::default()
        });
    let mut captured = Captured::with_capacities(&capacity);
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let inputs = engine_allocation::shared(UnitInputs::fixture(
        vec![carmy::DivisionSeed {
            pointer: 0x1234,
            handle: Some(7),
            ..Default::default()
        }],
        Outcome::Rows { len: 1 },
    ))
    .unwrap();
    let bytes = captured.allocated_bytes();
    let presentation_bytes = captured.division_presentation.allocated_bytes();
    assert!(presentation_bytes > 0);
    let report = division_presentation::CaptureReport {
        ticks: [89, 97, 5, 103, 0, 0, 0, 7, 11, 13, 17, 19],
    };
    captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&inputs)));
    captured.presentation_report = Some(report);
    let request = captured.take_presentation_request(&countries, 11, 0x1000, 0, None);
    assert!(captured.presentation_report.is_none());
    assert_eq!(captured.division_presentation.allocated_bytes(), 0);
    assert_eq!(captured.allocated_bytes() + presentation_bytes, bytes);
    assert_eq!(request.captured.allocated_bytes(), presentation_bytes);
    assert_eq!(
        (request.context.base, request.context.game_state),
        (11, 0x1000)
    );
    assert!(engine_allocation::SharedBox::ptr_eq(
        &request.countries,
        &countries
    ));
    assert!(engine_allocation::SharedBox::ptr_eq(
        request.inputs.as_ref().unwrap(),
        &inputs
    ));
    assert_eq!(request.inputs.as_ref().unwrap().seeds(), inputs.seeds());
    assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 2);
    assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 3);
    captured.restore_presentation_request(request, Ok(report));
    assert_eq!(captured.allocated_bytes(), bytes);
    assert_eq!(
        captured.division_presentation.allocated_bytes(),
        presentation_bytes
    );
    assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
    assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 2);
    assert!(engine_allocation::SharedBox::ptr_eq(
        captured.unit_inputs.as_ref().unwrap(),
        &inputs
    ));
    let pending = captured.presentation_report.as_ref().unwrap();
    assert_eq!(pending.ticks, report.ticks);
    captured.drop_and_reset();
    assert!(captured.presentation_report.is_none());
    assert_eq!(captured.allocated_bytes(), bytes);
    assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 1);
}

#[test]
fn presentation_terminal_reports_scatter_once_without_live_retry() {
    struct NoPresentationRead {
        independent: std::cell::Cell<usize>,
        presentation: std::cell::Cell<usize>,
        forbid_presentation: std::cell::Cell<bool>,
    }
    impl Memory for NoPresentationRead {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            if address == 0x1000 + engine_native::cgame_state::PROVINCES {
                assert!(
                    !self.forbid_presentation.get(),
                    "terminal Presentation report must not retry live acquisition"
                );
                self.presentation.set(self.presentation.get() + 1);
            } else {
                self.independent.set(self.independent.get() + 1);
            }
            false
        }
    }
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let inputs = engine_allocation::shared(UnitInputs::empty_rows()).unwrap();
    let presentation_bytes = captured.division_presentation.allocated_bytes();
    let report = division_presentation::CaptureReport {
        ticks: [113, 227, 5, 449, 0, 0, 0, 3, 23, 29, 31, 37],
    };
    for failure in [
        None,
        Some(collector_workers::acquisition::Failure::Panicked),
        Some(collector_workers::acquisition::Failure::Stopped),
    ] {
        captured.drop_and_reset();
        captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&inputs)));
        let request = captured.take_presentation_request(&countries, 11, 0x1000, 0, None);
        let context = request.context;
        captured.restore_presentation_request(request, failure.map_or(Ok(report), Err));
        let expected_outcomes = captured.division_presentation.outcomes();
        if failure.is_some() {
            assert!(
                expected_outcomes
                    .iter()
                    .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
            );
        }
        let memory = NoPresentationRead {
            independent: std::cell::Cell::new(0),
            presentation: std::cell::Cell::new(0),
            forbid_presentation: std::cell::Cell::new(true),
        };
        let completed = captured.acquire_presentation();
        assert_eq!(memory.presentation.get(), 0);
        assert_eq!(memory.independent.get(), 0);
        assert!(captured.presentation_report.is_none());
        captured.presentation_report = Some(completed);
        memory.forbid_presentation.set(false);
        let ticks = captured.capture_tables_all(&memory, 11, 0x1000, 0, false);
        assert!(memory.independent.get() > 0);
        assert!(
            captured.presentation_report.is_none(),
            "the report is consumed once"
        );
        assert_eq!(captured.division_presentation.outcomes(), expected_outcomes);
        for (slot, table) in division_presentation::TABLES.into_iter().enumerate() {
            assert_eq!(
                ticks[table.index()],
                if failure.is_none() {
                    report.ticks[slot]
                } else {
                    0
                }
            );
        }
        assert_eq!(
            captured.division_presentation.allocated_bytes(),
            presentation_bytes
        );
        assert_eq!(captured.decode_division_presentation_inputs(), [0; 2]);
        assert_eq!(captured.division_presentation.outcomes(), expected_outcomes);
        let before = memory.presentation.get();
        memory.forbid_presentation.set(true);
        captured.acquire_presentation();
        assert_eq!(memory.presentation.get(), before);
        assert_eq!(
            captured.division_presentation.outcomes(),
            [Outcome::Unavailable {
                reason: "division presentation acquisition not supplied",
            }; division_presentation::N]
        );
        memory.forbid_presentation.set(false);
        captured.division_presentation.capture_owned(
            &memory,
            context,
            unit_sources(&captured.unit_inputs),
        );
        assert!(memory.presentation.get() > before);
    }
}

#[test]
fn failed_pack_jobs_are_terminal_without_live_retry_or_partial_publication() {
    use engine_native_memory::memory::{CountedMemory, SliceMemory};
    use replay_frame::frame::TableView;

    let slow = [
        peace_bids::TABLES.as_slice(),
        map_names::TABLES.as_slice(),
        visibility::TABLES.as_slice(),
        faction_theatres::TABLES.as_slice(),
    ]
    .concat();
    let gated = [
        fog::TABLES.as_slice(),
        &construction::TABLES[2..],
        projects::TABLES.as_slice(),
    ]
    .concat()
    .into_iter()
    .filter(|id| *id != TableId::StateResistance)
    .collect::<Vec<_>>();
    let late = [
        leader_portraits::TABLES.as_slice(),
        leader_paths::TABLES.as_slice(),
        air::TABLES.as_slice(),
        naval_history::TABLES.as_slice(),
        army_icon_inputs::TABLES.as_slice(),
    ]
    .concat();
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let memory = CountedMemory::new(SliceMemory {
        base: 0,
        bytes: &[],
    });
    for force_full in [false, true] {
        let hour = 1;
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.prepare_hour(hour, force_full);
        let bytes = captured.allocated_bytes();
        let mut orders = Orders::vacant();
        let mut request = captured.take_slow_request(
            &countries,
            11,
            0x1000,
            hour,
            force_full,
            SlowStores::default(),
        );
        request.packs.ticks.fill(17);
        captured.restore_slow_request(request, false);
        let mut request =
            captured.take_stability_request(&countries, 11, 0x1000, hour, force_full, None);
        request.packs.ticks.fill(18);
        captured.restore_stability_request(request, false);
        let mut request =
            captured.take_war_overview_request(&countries, 11, 0x1000, hour, force_full);
        request.packs.ticks.fill(20);
        captured.restore_war_overview_request(request, false);
        let state = RaidStateStores::new();
        let mut request =
            captured.take_raid_state_request(&countries, 11, 0x1000, hour, force_full, state);
        request.packs.ticks.fill(19);
        captured.restore_raid_state_request(request, false);
        let launch = Box::new(collector_certificates::slicing::SliceFingerprints::new());
        let mut request =
            captured.take_raid_launch_request(&countries, 11, 0x1000, hour, force_full, launch);
        request.packs.ticks.fill(21);
        captured.restore_raid_launch_request(request, false);
        let mut request =
            captured.take_raid_targets_request(&countries, 11, 0x1000, hour, force_full);
        request.packs.ticks.fill(22);
        captured.restore_raid_targets_request(request, false);
        let mut request = captured.take_gated_request(
            11,
            0x1000,
            hour,
            force_full,
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            None,
        );
        request.packs.ticks.fill(23);
        captured.restore_gated_request(request, false);
        let request = captured.take_character_inputs_request(
            11,
            0x1000,
            hour,
            force_full,
            Default::default(),
        );
        captured.restore_character_inputs_request(request, None);
        let mut request = captured.take_late_request(
            &countries,
            &mut orders,
            false,
            11,
            0x1000,
            hour,
            force_full,
        );
        request.packs.ticks.fill(31);
        captured.restore_late_request(request, false, &mut orders);
        assert_eq!(captured.allocated_bytes(), bytes);

        for _ in 0..2 {
            let counted = collector_memory::read_accounting::Capture::new();
            let mut ticks = captured.capture_tables(&memory, 11, 0x1000, hour, force_full);
            captured.capture_tables_late(
                &memory,
                11,
                0x1000,
                hour,
                force_full,
                &mut orders,
                &mut ticks,
            );
            let reads = counted.finish();
            let outcomes = captured.outcomes();
            let mut tables = std::array::from_fn(|_| {
                TableInput::Sampled(collector_projection::table_conversion::unavailable(
                    "fixture",
                ))
            });
            let mut costs = crate::pack_conversion::HourCosts::EMPTY;
            // SAFETY: The spool owns the frame and skips any packs on loan to its joined helpers.
            unsafe {
                captured.insert_tables(
                    hour,
                    &mut tables,
                    &mut crate::pack_conversion::SpoolConversion {
                        variables_baseline: None,
                        variables_scratch: &mut Default::default(),
                        characters: None,
                        lexical: None,
                        intel: None,
                        projection: None,
                        helper: None,
                    },
                    false,
                    &mut costs,
                )
            };
            for (ids, reason) in [
                (&slow, "slow acquisition failed"),
                (
                    &country_stability::TABLES.to_vec(),
                    "stability acquisition failed",
                ),
                (
                    &war_overview::TABLES.to_vec(),
                    "war overview acquisition failed",
                ),
                (&gated, "gated acquisition failed"),
                (
                    &character_inputs::TABLES.to_vec(),
                    "character inputs acquisition failed",
                ),
                (&late, "late acquisition failed"),
                (
                    &raid_target_statuses::TABLES.to_vec(),
                    "raid target acquisition failed",
                ),
            ] {
                for &id in ids {
                    let expected = failed_capture(id, hour, force_full, reason);
                    assert_eq!(outcomes[id.index()], expected, "{id:?}");
                    assert_eq!(
                        reads[id.index()],
                        engine_native_memory::memory::ReadCounts::ZERO,
                        "retried {id:?}"
                    );
                    assert_eq!(ticks[id.index()], 0, "partial cost for {id:?}");
                    let Outcome::Unavailable { reason } = expected else {
                        unreachable!()
                    };
                    assert_eq!(
                        tables[id.index()],
                        TableInput::Sampled(TableView::Unavailable {
                            reason: reason.into()
                        }),
                        "{id:?}"
                    );
                }
            }
        }
        captured.drop_and_reset();
        let counted = collector_memory::read_accounting::Capture::new();
        stage_fixture_acquisitions(&mut captured, &memory, 11, 0x1000, hour, true);
        let mut ticks = captured.capture_tables(&memory, 11, 0x1000, hour, true);
        stage_fixture_late(&mut captured, &memory, 11, 0x1000, hour, true, &mut orders);
        captured.capture_tables_late(&memory, 11, 0x1000, hour, true, &mut orders, &mut ticks);
        let reads = counted.finish();
        for id in [
            TableId::ProvinceNames,
            TableId::VolunteerHosts,
            TableId::RailCooldowns,
        ] {
            assert!(
                reads[id.index()].copy_calls > 0,
                "control did not read {id:?}"
            );
        }
    }
}

#[test]
fn failed_pack_jobs_preserve_separately_acquired_tables_and_raw_gates() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    captured.prepare_hour(1, true);
    let mut orders = Orders::vacant();
    let raw_failure = "independent raw result";

    captured.fog.fail_radar_capture(raw_failure);
    captured.fog.fail_intel_capture(raw_failure);
    captured.fog.fail_registrations_capture(raw_failure);
    captured.fog.radar_raw_mode = true;
    captured.fog.intel_raw_mode = true;
    captured.fog.registrations_raw_mode = true;
    captured.projects.fail_projects_capture(true, raw_failure);
    captured.projects.raw_mode = true;
    captured
        .construction
        .fail_construction_rest_capture(raw_failure);
    captured.construction.rest_raw_mode = true;
    let buildings = captured.construction.take_buildings();
    captured
        .construction
        .restore_buildings(buildings, Err("independent building result"));
    let before = captured.outcomes();
    let request = captured.take_gated_request(
        11,
        0x1000,
        1,
        true,
        Box::new(collector_certificates::slicing::SliceFingerprints::new()),
        None,
    );
    captured.restore_gated_request(request, false);
    let after = captured.outcomes();
    for id in [
        TableId::RadarCoverage,
        TableId::UnitIntel,
        TableId::AirBaseRegistrations,
    ]
    .into_iter()
    .chain(projects::RAW)
    .chain(construction::TABLES)
    {
        assert_eq!(after[id.index()], before[id.index()], "{id:?}");
    }
    for id in [TableId::Projects, TableId::ProjectHistory] {
        assert_eq!(
            after[id.index()],
            Outcome::Unavailable {
                reason: "gated acquisition failed"
            },
            "{id:?}"
        );
    }
    assert!(
        captured.fog.radar_raw_mode
            && captured.fog.intel_raw_mode
            && captured.fog.registrations_raw_mode
    );
    assert!(captured.projects.raw_mode && captured.construction.rest_raw_mode);

    captured.air.fail_air_bases_capture(raw_failure);
    captured.air.fail_air_stats_capture(raw_failure);
    captured.air.bases_raw_mode = true;
    captured.air.stats_raw_mode = true;
    captured.naval_history.fail_naval_capture(raw_failure);
    captured.naval_history.fail_supremacy_capture(raw_failure);
    captured.naval_history.fail_sunk_capture(raw_failure);
    captured.naval_history.raw_mode = true;
    captured.naval_history.supremacy_raw_mode = true;
    captured.naval_history.sunk_raw_mode = true;
    let before = captured.outcomes();
    let request = captured.take_late_request(&countries, &mut orders, true, 11, 0x1000, 1, true);
    captured.restore_late_request(request, false, &mut orders);
    let after = captured.outcomes();
    for id in [
        TableId::AirBases,
        TableId::AirAces,
        TableId::AirStatistics,
        TableId::NavalSupremacy,
        TableId::SunkShips,
    ]
    .into_iter()
    .chain(naval_history::TABLES[4..10].iter().copied())
    .chain(naval_history::TABLES[14..].iter().copied())
    {
        assert_eq!(after[id.index()], before[id.index()], "{id:?}");
    }
    assert!(captured.air.bases_raw_mode && captured.air.stats_raw_mode);
    assert!(
        captured.naval_history.raw_mode
            && captured.naval_history.supremacy_raw_mode
            && captured.naval_history.sunk_raw_mode
    );
}

#[test]
fn a_refused_air_missions_job_fails_its_tables_without_allocating() {
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.prepare_hour(0, true);
    let before = captured.allocated_bytes();
    let mut request = captured.take_air_missions_request(&countries, 11, 0x1000, None);
    request.refuse();
    captured.restore_air_missions_request(request, true);
    assert_eq!(captured.allocated_bytes(), before);
    let reason = "air mission worker admission failed";
    for table in [TableId::AirRegionalMissions, TableId::AirNavalPatrolCombats] {
        assert_eq!(
            captured.outcomes()[table.index()],
            Outcome::Unavailable { reason },
            "{table:?}"
        );
    }
}

#[test]
fn ordinary_terminal_failures_forget_the_counter_censuses() {
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    for (family, task_forces, other) in [
        (None, None, None),
        (None, Some("acquisition worker panicked"), None),
        (
            None,
            None,
            Some("acquisition worker stopped before completion"),
        ),
        (Some("ordinary worker admission failed"), None, None),
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let units =
            engine_allocation::shared(UnitInputs::fixture(Vec::new(), Outcome::Rows { len: 0 }))
                .unwrap();
        captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&units)));
        captured
            .naval_counters
            .hold_census(engine_native::ctask_force::HeldCensus::default());
        let parts = captured.take_ordinary_parts(
            &countries,
            11,
            0x1000,
            1,
            Ok(()),
            crate::extended::NavalMissionLoan::default(),
        );
        let request = captured.take_ordinary_request(&countries, 11, 0x1000, 1);
        captured.restore_ordinary_request(
            request,
            family,
            parts.map(|part| {
                let failure = match part.part {
                    OrdinaryPart::TaskForces(_) => task_forces,
                    OrdinaryPart::Combat(_) => other,
                    _ => None,
                };
                (part, failure)
            }),
        );
        let failed = family.or(task_forces).or(other);
        assert_eq!(captured.ordinary_failure, failed);
        assert_eq!(
            captured.naval_counters.held_censuses(),
            usize::from(failed.is_none()),
            "{failed:?}"
        );
        captured.drop_and_reset();
        assert_eq!(
            captured.naval_counters.held_censuses(),
            usize::from(failed.is_none()),
            "the per-hour reset keeps what the family held: {failed:?}"
        );
    }
}

#[derive(Default)]
struct TerminalReadTrace(std::cell::RefCell<Vec<(usize, usize)>>);

impl Memory for TerminalReadTrace {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.0.borrow_mut().push((address, output.len()));
        false
    }
}

impl TerminalReadTrace {
    fn assert_only_inline_reads(self, base: usize, state: usize, hour: i32, late: bool) {
        let mut inline = Captured::with_capacities(&Capacities::default());
        let allowed = Self::default();
        inline.state_flags.capture(&allowed, base, state);
        inline.event_feed.capture(&allowed, base, state);
        if late {
            inline.lexical.capture(&allowed, base, state, hour, true);
        }
        let mut allowed = allowed.0.into_inner();
        let mut actual = self.0.into_inner();
        allowed.sort_unstable();
        actual.sort_unstable();
        assert_eq!(actual, allowed, "missing acquisitions must not read memory");
    }
}

#[test]
fn terminal_capture_keeps_missing_acquisition_reports_unavailable() {
    let memory = TerminalReadTrace::default();
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.weather.raw_mode = true;
    captured.capture_tables(&memory, 11, 0x1000, 1, true);
    memory.assert_only_inline_reads(11, 0x1000, 1, false);
    let outcomes = captured.outcomes();
    for (table, reason) in [
        (supply_trains::TABLES[0], "supply_trains acquisition not supplied"),
        (
            character_portraits::TABLES[0],
            "character_portraits acquisition not supplied",
        ),
        (
            province_unit_order::TABLES[0],
            "province_unit_order acquisition not supplied",
        ),
        (intel::TABLES[1], "residual Intel acquisition not supplied"),
    ] {
        assert_eq!(
            outcomes[table.index()],
            Outcome::Unavailable { reason },
            "{table:?}"
        );
    }
}

#[test]
fn terminal_capture_keeps_missing_worker_results_unavailable() {
    let memory = TerminalReadTrace::default();
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.supply_trains_report = Some(supply_trains::CaptureReport::default());
    captured.character_portraits_report = Some(character_portraits::CaptureReport::default());
    captured.province_unit_order_report = Some(province_unit_order::CaptureReport::default());
    captured.intel_residual_report = Some(intel::CaptureReport::default());
    let mut ticks = captured.capture_tables(&memory, 11, 0x1000, 1, true);
    captured.capture_tables_late(
        &memory,
        11,
        0x1000,
        1,
        true,
        &mut Orders::vacant(),
        &mut ticks,
    );
    memory.assert_only_inline_reads(11, 0x1000, 1, true);
    let outcomes = captured.outcomes();
    for (table, reason) in [
        (weather::TABLES[0], "weather acquisition not supplied"),
        (peace_bids::TABLES[0], "slow acquisition not supplied"),
        (country_stability::TABLES[0], "stability acquisition not supplied"),
        (war_overview::TABLES[0], "war overview acquisition not supplied"),
        (division_templates::TABLES[0], "template acquisition not supplied"),
        (raid_state_inputs::TABLES[0], "raid_state acquisition not supplied"),
        (raid_target_statuses::TABLES[0], "raid target acquisition not supplied"),
        (raid_launch_inputs::TABLES[0], "raid launch acquisition not supplied"),
        (fog::TABLES[0], "gated acquisition not supplied"),
        (character_inputs::TABLES[0], "character inputs acquisition not supplied"),
        (army_icon_inputs::TABLES[0], "late acquisition not supplied"),
    ] {
        assert_eq!(
            outcomes[table.index()],
            Outcome::Unavailable { reason },
            "{table:?}"
        );
    }
}

#[test]
fn ordinary_terminal_missing_marker_forgets_the_counter_censuses() {
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let mut captured = Captured::with_capacities(&Capacities::default());
    let units =
        engine_allocation::shared(UnitInputs::fixture(Vec::new(), Outcome::Rows { len: 0 }))
            .unwrap();
    captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&units)));
    captured
        .naval_counters
        .hold_census(engine_native::ctask_force::HeldCensus::default());
    let parts = captured.take_ordinary_parts(
        &countries,
        11,
        0x1000,
        1,
        Ok(()),
        crate::extended::NavalMissionLoan::default(),
    );
    let request = captured.take_ordinary_request(&countries, 11, 0x1000, 1);
    captured.restore_ordinary_request(request, None, parts.map(|part| (part, None)));
    assert_eq!(captured.ordinary_failure, None);
    assert_eq!(captured.naval_counters.held_censuses(), 1);
    captured.capture_tables(&NoRead, 11, 0x1000, 1, false);
    assert_eq!(captured.ordinary_failure, None);
    assert_eq!(
        captured.naval_counters.held_censuses(),
        1,
        "reused next hour"
    );
    captured.capture_tables(&NoRead, 11, 0x1000, 1, false);
    assert_eq!(
        captured.ordinary_failure,
        Some("ordinary acquisition not supplied")
    );
    assert_eq!(captured.naval_counters.held_censuses(), 0);
    captured.drop_and_reset();
    assert_eq!(captured.naval_counters.held_censuses(), 0);
    captured
        .naval_counters
        .hold_census(engine_native::ctask_force::HeldCensus::default());
    captured.capture_tables(&NoRead, 11, 0x1000, 2, false);
    assert_eq!(
        captured.ordinary_failure,
        Some("ordinary acquisition not supplied")
    );
    assert_eq!(captured.naval_counters.held_censuses(), 0);
    captured.drop_and_reset();
    assert_eq!(captured.naval_counters.held_censuses(), 0);
    captured
        .naval_counters
        .hold_census(engine_native::ctask_force::HeldCensus::default());
    captured.set_ordinary_failure(Some("ordinary worker admission failed"));
    assert_eq!(captured.naval_counters.held_censuses(), 0);
}

#[test]
fn ordinary_terminal_results_cannot_retry_or_publish_partial_tables() {
    #[derive(Default)]
    struct Reads {
        combat: std::cell::Cell<usize>,
    }
    impl Memory for Reads {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            if address == 0x1000 + engine_native::cgame_state::COMBATS + 8 {
                self.combat.set(self.combat.get() + 1);
            }
            false
        }
    }
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    for reason in [
        "ordinary worker admission failed",
        "acquisition worker panicked",
        "acquisition worker stopped before completion",
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let units =
            engine_allocation::shared(UnitInputs::fixture(Vec::new(), Outcome::Rows { len: 0 }))
                .unwrap();
        captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&units)));
        let before = captured.allocated_bytes();
        let mut parts = captured.take_ordinary_parts(
            &countries,
            11,
            0x1000,
            1,
            Err("fixture layout refusal"),
            crate::extended::NavalMissionLoan::default(),
        );
        let request = captured.take_ordinary_request(&countries, 11, 0x1000, 1);
        assert_eq!(request.hour, 1);
        assert!(
            parts
                .iter()
                .all(|part| part.fresh_modifier_guard == Err("fixture layout refusal"))
        );
        let owned = request.allocated_bytes()
            + parts
                .iter()
                .map(|part| part.allocated_bytes())
                .sum::<usize>();
        if reason == "ordinary worker admission failed" {
            parts[0].fail(reason);
        }
        let failure = (reason != "ordinary worker admission failed").then_some(reason);
        captured.restore_ordinary_request(
            request,
            None,
            parts.map(|part| {
                let failed = matches!(part.part, OrdinaryPart::Combat(_));
                (part, failure.filter(|_| failed))
            }),
        );
        assert_eq!(captured.allocated_bytes(), before);
        let memory = Reads::default();
        let ticks = captured.capture_tables(&memory, 11, 0x1000, 1, false);
        assert_eq!(memory.combat.get(), 0, "terminal work must not retry");
        for &id in super::ORDINARY_TABLES
            .iter()
            .flat_map(|tables| tables.iter())
        {
            assert_eq!(
                captured.outcomes()[id.index()],
                Outcome::Unavailable { reason }
            );
            assert_eq!(ticks[id.index()], 0);
        }
        captured.capture_tables(&memory, 11, 0x1000, 1, false);
        assert_eq!(memory.combat.get(), 0);
        assert_eq!(
            captured.ordinary_failure,
            Some("ordinary acquisition not supplied")
        );
        captured.drop_and_reset();
        assert!(captured.ordinary_failure.is_none());
        assert_eq!(engine_allocation::SharedBox::strong_count(&units), 1);
        let mut parts = captured.take_ordinary_parts(
            &countries,
            11,
            0x1000,
            2,
            Ok(()),
            crate::extended::NavalMissionLoan::default(),
        );
        let mut request = captured.take_ordinary_request(&countries, 11, 0x1000, 2);
        assert_eq!(
            request.allocated_bytes()
                + parts
                    .iter()
                    .map(|part| part.allocated_bytes())
                    .sum::<usize>(),
            owned
        );
        for part in &mut parts {
            part.part
                .capture(&memory, 11, 0x1000, part.hour, part.fresh_modifier_guard);
        }
        request.packs.capture_at(&memory, 11, 0x1000, request.hour);
        assert!(
            memory.combat.get() > 0,
            "control must exercise the native reader"
        );
        captured.restore_ordinary_request(request, None, parts.map(|part| (part, None)));
        assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
    }
}

#[test]
fn unpublished_raid_slices_are_read_again_at_the_next_hour() {
    use collector_certificates::slicing::{SliceFingerprints, SliceSelection};
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    let (hour, force_full) = (7, false);
    let seen = |store: &mut SliceFingerprints| {
        let mut selection = SliceSelection::for_test(0, true, &[]);
        selection.scan(store, 7, Some(1));
        store.finish_scan();
    };
    for completed in [true, false] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.prepare_hour(hour, force_full);
        let mut launch = Box::new(collector_certificates::slicing::SliceFingerprints::new());
        let mut state = RaidStateStores::new();
        for store in [&mut launch, &mut state.states, &mut state.buildings] {
            seen(store);
        }
        let mut state_request =
            captured.take_raid_state_request(&countries, 11, 0x1000, hour, force_full, state);
        let mut launch_request =
            captured.take_raid_launch_request(&countries, 11, 0x1000, hour, force_full, launch);
        if completed {
            let mut stores = RaidStateStores {
                states: state_request.states_store.take().unwrap(),
                buildings: state_request.buildings_store.take().unwrap(),
                routes: std::mem::replace(
                    &mut state_request.routes_store,
                    Box::new(collector_certificates::slicing::SliceFingerprints::new()),
                ),
            };
            state_request
                .packs
                .capture(&memory, 11, 0x1000, hour, force_full, &mut stores);
            state_request.states_store = Some(stores.states);
            state_request.buildings_store = Some(stores.buildings);
            state_request.routes_store = stores.routes;
            launch_request.packs.capture(
                &memory,
                11,
                0x1000,
                hour,
                force_full,
                &mut launch_request.store,
            );
        }
        let mut state = captured.restore_raid_state_request(state_request, completed);
        let mut launch = captured.restore_raid_launch_request(launch_request, completed);
        for (family, store) in [
            (collector_certificates::slicing::RAID_LAUNCH, &mut launch),
            (
                collector_certificates::slicing::RAID_STATE,
                &mut state.states,
            ),
            (
                collector_certificates::slicing::RAID_BUILDINGS,
                &mut state.buildings,
            ),
        ] {
            assert_eq!(captured.family_selection(family).hour, hour);
            let mut next = SliceSelection::for_test(hour + 1, false, &[]);
            next.scan(store, 7, Some(1));
            assert!(next.contains(7), "family {family}, completed {completed}");
        }
    }
}

#[test]
fn research_handoff_maps_failures_scatters_ticks_and_survives_a_governance_failure() {
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let (hour, force_full) = (7, false);
    let capacity = Capacities {
        governance: governance::Capacities::for_test_research(1),
        ..Default::default()
    };
    for failure in [
        None,
        Some(collector_workers::acquisition::Failure::Panicked),
        Some(collector_workers::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&capacity);
        captured.prepare_hour(hour, force_full);
        let bytes = captured.allocated_bytes();
        let mut store = Box::new(collector_certificates::slicing::SliceFingerprints::new());
        let mut seen = collector_certificates::slicing::SliceSelection::for_test(0, true, &[]);
        seen.scan(&mut store, 7, Some(1));
        store.finish_scan();
        let research =
            captured.take_research_request(&countries, 11, 0x1000, hour, force_full, store);
        assert!(captured.allocated_bytes() < bytes);
        let governance = captured.take_governance_request(
            &countries,
            11,
            0x1000,
            hour,
            force_full,
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            None,
            None,
            None,
        );
        let _technologies = captured.restore_governance_request(
            governance,
            Err(collector_workers::acquisition::Failure::Panicked),
        );
        let report = match failure {
            None => Ok(crate::acquisition_jobs::ResearchReport { ticks: [5, 6, 7] }),
            Some(failure) => Err(failure),
        };
        let mut store = captured.restore_research_request(research, report);
        let mut next =
            collector_certificates::slicing::SliceSelection::for_test(hour + 1, false, &[]);
        next.scan(&mut store, 7, Some(1));
        assert!(next.contains(7), "{failure:?}");
        assert_eq!(captured.allocated_bytes(), bytes);
        let outcomes = captured.governance.outcomes();
        let family = captured
            .governance_report
            .expect("governance report restored");
        match failure {
            None => {
                for (index, cost) in governance::RESEARCH.into_iter().zip([5, 6, 7]) {
                    assert_eq!(family.ticks[index], cost);
                    assert!(!matches!(
                        outcomes[index],
                        Outcome::Unavailable { reason } if reason == "governance acquisition panicked"
                    ));
                }
            }
            Some(failure) => {
                let expected = match failure {
                    collector_workers::acquisition::Failure::Panicked => {
                        "research acquisition panicked"
                    }
                    collector_workers::acquisition::Failure::Stopped => {
                        "research acquisition stopped before reading"
                    }
                };
                for index in governance::RESEARCH {
                    assert_eq!(family.ticks[index], 0);
                    assert_eq!(outcomes[index], Outcome::Unavailable { reason: expected });
                }
            }
        }
        assert_eq!(
            outcomes[5],
            Outcome::Unavailable {
                reason: "governance acquisition panicked"
            }
        );
    }
}

#[test]
fn variant_handoff_skips_off_slot_without_reads_and_never_retries_a_failed_worker() {
    struct Reads(std::cell::Cell<usize>);
    impl Memory for Reads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            self.0.set(self.0.get() + 1);
            false
        }
    }
    let slot = replay_data::cadence::refresh_slot(TableId::VariantDefinitions).unwrap() as i32;
    let mut capacity = Capacities::default();
    capacity.production.rows[3..7].fill(1);
    let mut captured = Captured::with_capacities(&capacity);
    let memory = engine_native_memory::memory::CountedMemory::new(Reads(std::cell::Cell::new(0)));
    let bytes = captured.allocated_bytes();
    let mut request = collector_test_support::with_allocations_denied(|| {
        captured.take_variants_request(11, 0x1000, slot + 1, false)
    });
    assert_eq!(
        captured.allocated_bytes() + request.allocated_bytes(),
        bytes
    );
    request.registry.mark_skipped();
    request
        .variants
        .fail(replay_data::cadence::SKIPPED_REASON, slot + 1, false);
    request.changes_ticks = 79;
    collector_test_support::with_allocations_denied(|| {
        captured.restore_variants_request(request, Ok([0; production::N]));
    });
    assert_eq!(captured.allocated_bytes(), bytes);
    assert_eq!(captured.overhead[TableId::VariantChanges.index()], 79);
    let ticks = captured.capture_tables_all(&memory, 11, 0x1000, slot + 1, false);
    for (index, table) in production::TABLES.into_iter().enumerate().skip(3).take(4) {
        assert_eq!(ticks[table.index()], 0);
        assert_eq!(
            captured.production.outcomes()[index],
            Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON
            }
        );
    }
    for failure in [
        collector_workers::acquisition::Failure::Panicked,
        collector_workers::acquisition::Failure::Stopped,
    ] {
        let bytes = captured.allocated_bytes();
        let request = captured.take_variants_request(11, 0x1000, slot, false);
        assert_eq!(
            captured.allocated_bytes() + request.allocated_bytes(),
            bytes
        );
        collector_test_support::with_allocations_denied(|| {
            captured.restore_variants_request(request, Err(failure));
        });
        assert_eq!(captured.allocated_bytes(), bytes);
        let reads = collector_memory::read_accounting::Capture::new();
        captured.capture_tables_all(&memory, 11, 0x1000, slot, false);
        let counts = reads.finish();
        for table in production::TABLES[3..7]
            .iter()
            .chain([&TableId::VariantChanges])
        {
            assert_eq!(
                counts[table.index()],
                engine_native_memory::memory::ReadCounts::ZERO
            );
        }
        let expected = match failure {
            collector_workers::acquisition::Failure::Panicked => "variant acquisition panicked",
            collector_workers::acquisition::Failure::Stopped => {
                "variant acquisition stopped before reading"
            }
        };
        for table in production::TABLES[3..7]
            .iter()
            .chain([&TableId::VariantChanges])
        {
            let index = production::TABLES
                .iter()
                .position(|id| id == table)
                .unwrap();
            assert_eq!(
                captured.production.outcomes()[index],
                Outcome::Unavailable { reason: expected }
            );
        }
    }
}

#[test]
fn variant_handoff_preserves_a_finished_change_capture_after_a_later_failure() {
    let changes_index = production::TABLES
        .iter()
        .position(|&table| table == TableId::VariantChanges)
        .unwrap();
    for failure in [
        collector_workers::acquisition::Failure::Panicked,
        collector_workers::acquisition::Failure::Stopped,
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut request = captured.take_variants_request(11, 0x1000, 1, false);
        request.changes.fail("finished design-change verdict");
        request.changes_finished = true;
        request.changes_ticks = 31;
        collector_test_support::with_allocations_denied(|| {
            captured.restore_variants_request(request, Err(failure));
        });
        assert_eq!(
            captured.production.outcomes()[changes_index],
            Outcome::Unavailable {
                reason: "finished design-change verdict"
            }
        );
        assert_eq!(captured.overhead[TableId::VariantChanges.index()], 31);
    }
}

#[test]
fn governance_handoff_restores_owned_storage_and_never_retries_terminal_results() {
    struct Reads {
        total: std::cell::Cell<usize>,
        governance: std::cell::Cell<usize>,
    }
    impl Memory for Reads {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            self.total.set(self.total.get() + 1);
            if address == 0x1000 + engine_native::cgame_state::TECH_SHARING_GROUPS {
                self.governance.set(self.governance.get() + 1);
            }
            false
        }
    }
    let mut capacity = Capacities {
        construction: construction::Capacities::for_test_buildings(1),
        ..Default::default()
    };
    capacity.governance.raise_for_countries(1);
    for failure in [
        None,
        Some(collector_workers::acquisition::Failure::Panicked),
        Some(collector_workers::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&capacity);
        captured.governance_report = Some(governance::CaptureReport::default());
        let bytes = captured.allocated_bytes();
        let governance_bytes = captured.governance.allocated_bytes();
        let countries = engine_allocation::shared(
            engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
        )
        .unwrap();
        let request = captured.take_governance_request(
            &countries,
            11,
            0x1000,
            1,
            failure.is_none(),
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            None,
            None,
            None,
        );
        assert!(captured.governance_report.is_none());
        assert_eq!(captured.governance.allocated_bytes(), 0);
        assert_eq!(
            captured.allocated_bytes() + request.allocated_bytes(),
            bytes
        );
        assert_eq!(request.captured.allocated_bytes(), governance_bytes);
        assert!(request.allocated_bytes() > governance_bytes);
        assert!(engine_allocation::SharedBox::ptr_eq(
            &request.countries,
            &countries
        ));
        assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 2);
        assert_eq!(
            (
                request.context.base,
                request.context.game_state,
                request.context.hour
            ),
            (11, 0x1000, 1)
        );
        assert_eq!(request.context.force_full, failure.is_none());
        let context = request.context;
        let report = crate::acquisition_jobs::GovernanceReport {
            governance: governance::CaptureReport {
                ticks: std::array::from_fn(|slot| slot as u64 + 101),
            },
            buildings: 307,
        };
        let variant_ticks: [u64; production::N] = std::array::from_fn(|slot| {
            if (3..=6).contains(&slot) {
                slot as u64 + 211
            } else {
                0
            }
        });
        let gated = captured.take_gated_request(
            11,
            0x1000,
            1,
            context.force_full,
            Box::new(collector_certificates::slicing::SliceFingerprints::new()),
            None,
        );
        captured.restore_gated_request(gated, false);
        let _technologies =
            captured.restore_governance_request(request, failure.map_or(Ok(report), Err));
        let variants = captured.take_variants_request(11, 0x1000, 1, context.force_full);
        captured.restore_variants_request(
            variants,
            Ok(if context.force_full {
                variant_ticks
            } else {
                [0; production::N]
            }),
        );
        assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
        assert_eq!(captured.allocated_bytes(), bytes);
        let expected_outcomes = captured.governance.outcomes();
        let memory = Reads {
            total: std::cell::Cell::new(0),
            governance: std::cell::Cell::new(0),
        };
        let ticks = captured.capture_tables_all(&memory, 11, 0x1000, 1, context.force_full);
        assert!(memory.total.get() > 0);
        assert_eq!(
            memory.governance.get(),
            0,
            "completed Governance must not run again during independent capture"
        );
        assert!(captured.governance_report.is_none());
        assert_eq!(captured.governance.outcomes(), expected_outcomes);
        for (slot, table) in governance::TABLES.into_iter().enumerate() {
            assert_eq!(
                ticks[table.index()],
                if failure.is_none() {
                    report.governance.ticks[slot]
                } else {
                    0
                }
            );
            if failure.is_some() && slot == 3 {
                assert_eq!(
                    expected_outcomes[slot],
                    Outcome::Unavailable {
                        reason: replay_data::cadence::SKIPPED_REASON
                    }
                );
            }
            if failure.is_some() && governance::TECHNOLOGIES.contains(&slot) {
                assert!(matches!(
                    expected_outcomes[slot],
                    Outcome::Unavailable { reason } if reason != replay_data::cadence::SKIPPED_REASON
                ));
            }
        }
        for (slot, table) in production::TABLES.into_iter().enumerate().skip(3).take(4) {
            assert_eq!(
                ticks[table.index()],
                if failure.is_none() {
                    variant_ticks[slot]
                } else {
                    0
                }
            );
        }
        assert_eq!(
            ticks[TableId::StateBuildings.index()],
            if failure.is_none() {
                report.buildings
            } else {
                0
            }
        );
        assert!(captured.variants_ticks.is_none());
        captured.capture_tables_all(&memory, 11, 0x1000, 1, true);
        assert_eq!(memory.governance.get(), 0);
        assert_eq!(
            captured.governance.outcomes()[2],
            Outcome::Unavailable {
                reason: "governance acquisition not supplied"
            }
        );
        captured.governance.capture_owned(
            &memory,
            governance::CaptureContext {
                force_full: true,
                ..context
            },
            &mut collector_certificates::slicing::SliceFingerprints::new(),
            &mut collector_certificates::slicing::SliceFingerprints::new(),
            None,
            None,
        );
        assert!(memory.governance.get() > 0);
        captured.drop_and_reset();
        assert!(captured.governance_report.is_none());
    }
}

#[test]
fn intel_residual_handoff_preserves_terminal_results_without_repeating_live_reads() {
    let mut capacity = Capacities::default();
    capacity.intel.raise_for_countries(1);
    for failure in [
        None,
        Some(collector_workers::acquisition::Failure::Panicked),
        Some(collector_workers::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&capacity);
        captured.intel.capitulation_raw_mode = true;
        captured.intel.relations_raw_mode = true;
        let bytes = captured.allocated_bytes();
        let family_bytes = captured.intel.allocated_bytes();
        let countries = engine_allocation::shared(
            engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
        )
        .unwrap();
        let context = intel::CaptureContext {
            base: 11,
            game_state: 0x1000,
            hour: 1,
            force_full: true,
        };
        let (request, variables) =
            captured.take_intel_residual_request(&countries, context, None, None);
        assert_eq!(captured.intel.allocated_bytes(), 0);
        assert_eq!(captured.allocated_bytes() + family_bytes, bytes);
        assert_eq!(
            request.captured.allocated_bytes() + variables.part.allocated_bytes(),
            family_bytes
        );
        assert!(engine_allocation::SharedBox::ptr_eq(
            &request.countries,
            &countries
        ));
        assert!(engine_allocation::SharedBox::ptr_eq(
            &variables.countries,
            &countries
        ));
        let original = intel::CaptureReport {
            ticks: std::array::from_fn(|index| {
                if [0, 13, 14].contains(&index) {
                    0
                } else {
                    index as u64 + 101
                }
            }),
        };
        let flags =
            captured.restore_intel_residual_request(request, failure.map_or(Ok(original), Err));
        let store = captured.restore_script_variables(variables, failure.map_or(Ok(0), Err));
        assert!(flags.is_none() && store.is_none());
        assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
        assert_eq!(captured.allocated_bytes(), bytes);
        let outcomes = captured.intel.outcomes();
        let report = captured.finish_intel_residual(context);
        assert!(captured.intel_residual_report.is_none());
        for index in (1..=12).chain(15..intel::N) {
            assert_eq!(captured.intel.outcomes()[index], outcomes[index]);
            assert_eq!(
                report.ticks[index],
                if failure.is_none() {
                    original.ticks[index]
                } else {
                    0
                }
            );
        }
        for index in [0, 13, 14] {
            assert_eq!(report.ticks[index], 0);
        }
        captured.drop_and_reset();
        assert!(captured.intel_residual_report.is_none());
    }
}

#[test]
fn ship_model_parts_restore_part_zero_first_sum_ticks_and_match_the_unsplit_walk() {
    struct Faulty;
    impl Memory for Faulty {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let walked = |ships: u64| {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.ship_models.begin_sources();
        for id in 1..=ships {
            captured.ship_models.visit_source(
                0x1000 * id as usize,
                &engine_native::cship::Row {
                    id,
                    ..Default::default()
                },
            );
        }
        captured.ship_models.finish_sources(Outcome::Rows {
            len: ships as usize,
        });
        let request =
            captured.take_ship_models_request(&countries, 11, 0x1000, ShipCensus::vacant());
        let _census = captured.restore_ship_models_request(request, Ok(()));
        captured
    };
    let mut reference = walked(3);
    reference
        .ship_models
        .capture_owned(&Faulty, ship_models::CaptureContext::new(11));
    let expected = reference.ship_models.outcomes();
    assert!(matches!(expected[0], Outcome::Unavailable { .. }));
    for (ships, parts, stop) in [
        (3, 1, None),
        (1200, 3, None),
        (2049, 4, None),
        (1200, 3, Some(2)),
    ] {
        let mut captured = walked(ships);
        let requests = captured.take_ship_model_parts(&countries, 11);
        assert_eq!(requests.iter().flatten().count(), parts, "{ships} ships");
        assert_eq!(captured.ship_models.allocated_bytes(), 0);
        for (index, request) in requests.into_iter().flatten().enumerate() {
            assert_eq!(request.part, index);
            assert_eq!(
                request.context.base, 11,
                "each detail part retains the original census request base"
            );
            let mut request = request;
            let report = if stop == Some(index) {
                Err(collector_workers::acquisition::Failure::Stopped)
            } else {
                request.captured.capture_owned(&Faulty, request.context);
                Ok(ship_models::CaptureReport {
                    ticks: [index as u64 + 1],
                })
            };
            captured.restore_ship_model_part(request, report);
        }
        captured.finish_ship_model_parts();
        if stop.is_some() {
            assert!(captured.ship_models.has_terminal_failure());
            continue;
        }
        let ticks = (1..=parts as u64).sum::<u64>();
        assert_eq!(captured.ship_models_report.unwrap().ticks, [ticks]);
        assert_eq!(captured.ship_models.outcomes(), expected, "{ships} ships");
    }
    let mut captured = walked(3);
    let request = captured.take_ship_models_request(&countries, 11, 0x1000, ShipCensus::vacant());
    let _census = captured.restore_ship_models_request(
        request,
        Err(collector_workers::acquisition::Failure::Panicked),
    );
    assert!(
        captured
            .take_ship_model_parts(&countries, 11)
            .iter()
            .all(Option::is_none)
    );
}

#[test]
fn ship_models_late_terminal_handoff_is_unavailable_until_new_sources() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("terminal ShipModels routing must not retry owned detail reads");
        }
    }
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    captured.ship_models.begin_sources();
    captured
        .ship_models
        .finish_sources(Outcome::Rows { len: 0 });
    let request = captured.take_ship_models_request(&countries, 11, 0x1000, ShipCensus::vacant());
    let family_bytes = request.captured.allocated_bytes();
    assert_eq!(captured.ship_models.allocated_bytes(), 0);
    let census = captured.restore_ship_models_request(
        request,
        Err(collector_workers::acquisition::Failure::Panicked),
    );
    assert!(matches!(census.ships.outcome, Outcome::Unavailable { .. }));
    assert_eq!(captured.ship_models.allocated_bytes(), family_bytes);
    assert_eq!(
        captured.ship_models_report.unwrap().ticks,
        [0],
        "interrupted worker detail cost remains unmeasured"
    );
    let ticks = captured.capture_ship_models(&NoReads, 11);
    assert_eq!(ticks, 0, "failed whole-table service cost is unmeasured");
    assert!(
        captured.ship_models_report.is_none(),
        "terminal report is consumed once"
    );
    assert!(captured.ship_models.has_terminal_failure());
    assert!(matches!(
        captured.ship_models.outcomes()[0],
        Outcome::Unavailable {
            reason: "ship models acquisition panicked"
        }
    ));
    let again_ticks = captured.capture_ship_models(&NoReads, 11);
    assert_eq!(again_ticks, 0);
    assert!(captured.ship_models_report.is_none());
    assert!(matches!(
        captured.ship_models.outcomes()[0],
        Outcome::Unavailable {
            reason: "ship models acquisition panicked"
        }
    ));

    captured.drop_and_reset();
    captured.ship_models.begin_sources();
    captured
        .ship_models
        .finish_sources(Outcome::Rows { len: 0 });
    captured.capture_ship_models(&NoReads, 11);
    assert!(
        !captured.ship_models.has_terminal_failure(),
        "a new complete source epoch clears the stale terminal failure"
    );
    assert_eq!(captured.ship_models.outcomes(), [Outcome::Rows { len: 0 }]);
    assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
}

#[test]
fn hourly_family_handoffs_restore_independently_and_consume_terminal_reports_once() {
    #[derive(Default)]
    struct Reads(std::cell::Cell<usize>);
    impl Memory for Reads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            self.0.set(self.0.get() + 1);
            false
        }
    }
    use collector_workers::acquisition::Failure::{Panicked, Stopped};
    let mut capacity = Capacities::default();
    capacity.supply_trains.raise_for_countries(1);
    let mut captured = Captured::with_capacities(&capacity);
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let supply_bytes = captured.supply_trains.allocated_bytes();
    let portrait_bytes = captured.character_portraits.allocated_bytes();
    let province_order_bytes = captured.province_unit_order.allocated_bytes();
    for (supply_failure, portrait_failure, province_order_failure) in [
        (None, None, None),
        (Some(Panicked), None, None),
        (None, Some(Panicked), None),
        (None, None, Some(Panicked)),
        (Some(Stopped), None, None),
        (None, Some(Stopped), None),
        (None, None, Some(Stopped)),
    ] {
        captured.drop_and_reset();
        let bytes = captured.allocated_bytes();
        let before = captured.outcomes();
        captured.supply_trains_report = Some(supply_trains::CaptureReport::default());
        captured.character_portraits_report = Some(character_portraits::CaptureReport::default());
        captured.province_unit_order_report = Some(province_unit_order::CaptureReport::default());
        let supply: Vec<_> = (0..supply_trains::PARTS)
            .map(|part| {
                captured.take_supply_trains_request(&countries, 11, 0x1000, 0, false, part, None)
            })
            .collect();
        assert!(captured.supply_trains_report.is_none());
        assert_eq!(captured.supply_trains.allocated_bytes(), 0);
        assert_eq!(
            captured.character_portraits.allocated_bytes(),
            portrait_bytes
        );
        assert_eq!(
            supply
                .iter()
                .map(|r| r.nodes.allocated_bytes()
                    + r.trains.as_ref().map_or(0, |t| t.allocated_bytes()))
                .sum::<usize>(),
            supply_bytes
        );
        assert_eq!(captured.allocated_bytes() + supply_bytes, bytes);
        let portraits: Vec<_> = (0..character_portraits::PARTS)
            .map(|part| captured.take_character_portraits_request(11, 0x1000, part))
            .collect();
        assert!(captured.character_portraits_report.is_none());
        assert_eq!(captured.character_portraits.allocated_bytes(), 0);
        assert_eq!(
            portraits
                .iter()
                .map(|r| r.captured.allocated_bytes())
                .sum::<usize>(),
            portrait_bytes
        );
        assert_eq!(
            captured.allocated_bytes() + supply_bytes + portrait_bytes,
            bytes
        );
        let census = engine_allocation::shared(
            collector_divisions::division_census::UnitInputs::empty_rows(),
        )
        .unwrap();
        let province_order = captured.take_province_unit_order_request(
            11,
            0x1000,
            Some(engine_allocation::SharedBox::clone(&census)),
            Some(province_unit_order::owned_tests::held_store()),
        );
        assert_eq!(engine_allocation::SharedBox::strong_count(&census), 2);
        assert!(captured.province_unit_order_report.is_none());
        assert_eq!(captured.province_unit_order.allocated_bytes(), 0);
        assert_eq!(
            province_order.captured.allocated_bytes(),
            province_order_bytes
        );
        assert_eq!(
            captured.allocated_bytes() + supply_bytes + portrait_bytes + province_order_bytes,
            bytes
        );
        assert!(
            supply
                .iter()
                .all(|r| engine_allocation::SharedBox::ptr_eq(&r.countries, &countries))
        );
        assert_eq!(
            engine_allocation::SharedBox::strong_count(&countries),
            1 + supply_trains::PARTS
        );
        let supply_context = supply[0].context;
        let portrait_context = portraits[0].context;
        let province_order_context = province_order.context;
        assert_eq!(
            (supply_context.base, supply_context.game_state),
            (11, 0x1000)
        );
        assert_eq!(
            (portrait_context.base, portrait_context.game_state),
            (11, 0x1000)
        );
        assert_eq!(
            (
                province_order_context.base,
                province_order_context.game_state
            ),
            (11, 0x1000)
        );
        let supply_report = supply_trains::CaptureReport {
            ticks: [101 * supply_trains::PARTS as u64, 103],
        };
        let portrait_report = character_portraits::CaptureReport {
            ticks: [107 * character_portraits::PARTS as u64],
        };
        let province_order_report = province_unit_order::CaptureReport { ticks: [109] };
        for request in supply {
            let part = supply_trains::CaptureReport {
                ticks: [101, if request.part == 0 { 103 } else { 0 }],
            };
            captured.restore_supply_trains_request(request, supply_failure.map_or(Ok(part), Err));
        }
        assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
        assert!(captured.supply_trains_report.is_some());
        assert!(captured.character_portraits_report.is_none());
        for request in portraits {
            captured.restore_character_portraits_request(
                request,
                portrait_failure
                    .map_or(Ok(character_portraits::CaptureReport { ticks: [107] }), Err),
            );
        }
        assert_eq!(engine_allocation::SharedBox::strong_count(&countries), 1);
        let store = captured
            .restore_province_unit_order_request(
                province_order,
                province_order_failure.map_or(Ok(province_order_report), Err),
            )
            .unwrap();
        assert_eq!(store.holds(), province_order_failure.is_none());
        assert_eq!(engine_allocation::SharedBox::strong_count(&census), 1);
        assert_eq!(captured.allocated_bytes(), bytes);
        let outcomes = captured.outcomes();
        for (index, outcome) in outcomes.into_iter().enumerate() {
            if ![
                TableId::SupplyNodes.index(),
                TableId::TrainInventory.index(),
                TableId::CharacterPortraits.index(),
                TableId::ProvinceUnitOrder.index(),
            ]
            .contains(&index)
            {
                assert_eq!(outcome, before[index], "unowned table {index}");
            }
        }
        for (tables, failure, panicked, stopped) in [
            (
                supply_trains::TABLES.as_slice(),
                supply_failure,
                "supply trains acquisition panicked",
                "supply trains acquisition stopped before reading",
            ),
            (
                character_portraits::TABLES.as_slice(),
                portrait_failure,
                "character portraits acquisition panicked",
                "character portraits acquisition stopped before reading",
            ),
            (
                province_unit_order::TABLES.as_slice(),
                province_order_failure,
                "province unit order acquisition panicked",
                "province unit order acquisition stopped before reading",
            ),
        ] {
            if let Some(failure) = failure {
                let reason = match failure {
                    Panicked => panicked,
                    Stopped => stopped,
                };
                for table in tables {
                    assert_eq!(outcomes[table.index()], Outcome::Unavailable { reason });
                }
            }
        }
        let supply = captured.finish_supply_trains();
        let portraits = captured.finish_character_portraits();
        let province_order = captured.finish_province_unit_order();
        assert_eq!(
            supply.ticks,
            if supply_failure.is_none() {
                supply_report.ticks
            } else {
                [0; 2]
            }
        );
        assert_eq!(
            portraits.ticks,
            if portrait_failure.is_none() {
                portrait_report.ticks
            } else {
                [0]
            }
        );
        assert_eq!(
            province_order.ticks,
            if province_order_failure.is_none() {
                province_order_report.ticks
            } else {
                [0]
            }
        );
        assert!(captured.supply_trains_report.is_none());
        assert!(captured.character_portraits_report.is_none());
        assert!(captured.province_unit_order_report.is_none());
        captured.supply_trains_report = Some(supply);
        captured.character_portraits_report = Some(portraits);
        captured.province_unit_order_report = Some(province_order);
        let memory = Reads::default();
        let ticks = captured.capture_tables_all(&memory, 11, 0x1000, 1, false);
        let routed_bytes = captured.allocated_bytes();
        assert_eq!(captured.supply_trains.allocated_bytes(), supply_bytes);
        assert_eq!(
            captured.character_portraits.allocated_bytes(),
            portrait_bytes
        );
        assert_eq!(
            captured.province_unit_order.allocated_bytes(),
            province_order_bytes
        );
        assert!(memory.0.get() > 0);
        assert_eq!(
            supply_trains::TABLES.map(|table| ticks[table.index()]),
            supply.ticks
        );
        assert_eq!(
            character_portraits::TABLES.map(|table| ticks[table.index()]),
            portraits.ticks
        );
        assert_eq!(
            province_unit_order::TABLES.map(|table| ticks[table.index()]),
            province_order.ticks
        );
        let routed_outcomes = captured.outcomes();
        assert_eq!(
            supply_trains::TABLES.map(|table| routed_outcomes[table.index()]),
            supply_trains::TABLES.map(|table| outcomes[table.index()])
        );
        assert_eq!(
            character_portraits::TABLES.map(|table| routed_outcomes[table.index()]),
            character_portraits::TABLES.map(|table| outcomes[table.index()])
        );
        assert_eq!(
            province_unit_order::TABLES.map(|table| routed_outcomes[table.index()]),
            province_unit_order::TABLES.map(|table| outcomes[table.index()])
        );
        assert!(captured.supply_trains_report.is_none());
        assert!(captured.character_portraits_report.is_none());
        assert!(captured.province_unit_order_report.is_none());
        let direct = Reads::default();
        captured.supply_trains.capture_owned(&direct, supply_context);
        captured.character_portraits.capture_owned(&direct, portrait_context);
        captured
            .province_unit_order
            .capture_owned(&direct, province_order_context, None, None);
        assert_eq!(
            direct.0.get(),
            3 * supply_trains::PARTS + 1 + character_portraits::PARTS + 1
        );
        captured.supply_trains_report = Some(supply);
        captured.character_portraits_report = Some(portraits);
        captured.province_unit_order_report = Some(province_order);
        captured.drop_and_reset();
        assert!(captured.supply_trains_report.is_none());
        assert!(captured.character_portraits_report.is_none());
        assert!(captured.province_unit_order_report.is_none());
        assert_eq!(captured.allocated_bytes(), routed_bytes);
    }
}

#[test]
fn shared_unit_sources_keep_census_faults_and_release_on_reset() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(unit_sources(&captured.unit_inputs).outcome, MISSING);
    assert!(unit_sources(&captured.unit_inputs).rows.is_empty());
    let inputs = engine_allocation::shared(UnitInputs::fixture(
        vec![carmy::DivisionSeed {
            pointer: 0x1234,
            ..Default::default()
        }],
        Outcome::Truncated {
            len: 1,
            unread_rows: 7,
        },
    ))
    .unwrap();
    captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&inputs)));
    let sources = unit_sources(&captured.unit_inputs);
    assert_eq!(sources.rows, inputs.seeds());
    assert_eq!(sources.rows[0].pointer, 0x1234);
    assert_eq!(
        sources.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 7
        }
    );
    let failed = engine_allocation::shared(UnitInputs::fixture(
        Vec::new(),
        Outcome::Unavailable {
            reason: "current census failed",
        },
    ))
    .unwrap();
    captured.install_unit_inputs(Some(engine_allocation::SharedBox::clone(&failed)));
    assert_eq!(engine_allocation::SharedBox::strong_count(&inputs), 1);
    assert!(unit_sources(&captured.unit_inputs).rows.is_empty());
    assert_eq!(
        unit_sources(&captured.unit_inputs).outcome,
        Outcome::Unavailable {
            reason: "current census failed"
        },
    );
    captured.drop_and_reset();
    assert_eq!(engine_allocation::SharedBox::strong_count(&failed), 1);
    assert_eq!(unit_sources(&captured.unit_inputs).outcome, MISSING);
    assert!(unit_sources(&captured.unit_inputs).rows.is_empty());
}

#[test]
fn peace_settlement_packs_are_registered_without_inventing_empty_data() {
    for table in [TableId::PeaceSettlements, TableId::PeaceBids] {
        assert_eq!(TABLES.iter().filter(|&&id| id == table).count(), 1);
    }
    for view in peace_settlements::Captured::vacant()
        .tables()
        .into_iter()
        .chain(peace_bids::Captured::vacant().tables())
    {
        assert!(matches!(
            view,
            replay_frame::frame::TableView::Unavailable { .. }
        ));
    }
}

#[test]
fn the_slow_job_stores_travel_with_it_and_always_return() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    for completed in [true, false] {
        let mut available = Box::new(collector_certificates::slicing::SliceFingerprints::new());
        let mut seen = collector_certificates::slicing::SliceSelection::for_test(0, true, &[]);
        seen.scan(&mut available, 7, Some(0));
        seen.scan(&mut available, 9, Some(0));
        available.finish_scan();
        let stores = SlowStores {
            doctrine_bytes: None,
            doctrines: None,
            available: Some(available),
            advisors: None,
            local_subunits: None,
        };
        captured.prepare_hour(9, false);
        let request = captured.take_slow_request(&countries, 11, 0x1000, 9, false, stores);
        assert!(request.stores.available.is_some());
        let mut stores = captured.restore_slow_request(request, completed);
        assert_eq!(captured.hired_advisors.available_selection().hour, 9);
        let available = stores.available.as_mut().expect("the store returns");
        let mut next = collector_certificates::slicing::SliceSelection::for_test(10, false, &[]);
        next.scan(available, 9, Some(0));
        next.scan(available, 7, Some(0));
        available.finish_scan();
        assert!(next.contains(9), "completed {completed}");
        assert_eq!(next.contains(7), !completed, "completed {completed}");
    }
    let request =
        captured.take_slow_request(&countries, 11, 0x1000, 9, false, SlowStores::default());
    let stores = captured.restore_slow_request(request, true);
    assert!(stores.available.is_none());
}

#[test]
fn the_stability_tables_are_their_own_job_and_its_store_always_returns() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let (hour, force_full) = (9, false);
    for completed in [true, false] {
        let mut store = Box::new(collector_certificates::slicing::SliceFingerprints::new());
        let mut seen = collector_certificates::slicing::SliceSelection::for_test(0, true, &[]);
        seen.scan(&mut store, 7, Some(0));
        seen.scan(&mut store, 9, Some(0));
        store.finish_scan();
        captured.prepare_hour(hour, force_full);
        let slow = captured.take_slow_request(
            &countries,
            11,
            0x1000,
            hour,
            force_full,
            SlowStores::default(),
        );
        let stability =
            captured.take_stability_request(&countries, 11, 0x1000, hour, force_full, Some(store));
        assert!(stability.store.is_some());
        captured.restore_slow_request(slow, false);
        let mut store = captured
            .restore_stability_request(stability, completed)
            .expect("the store returns");
        assert_eq!(captured.country_stability.selection().hour, hour);
        let outcomes = captured.outcomes();
        for &table in &country_stability::TABLES {
            let failed = |reason| failed_capture(table, hour, force_full, reason);
            assert!(!matches!(outcomes[table.index()], Outcome::Rows { .. }));
            assert_ne!(outcomes[table.index()], failed("slow acquisition failed"));
            assert_eq!(
                outcomes[table.index()] == failed("stability acquisition failed"),
                !completed,
                "{table:?}"
            );
        }
        for &table in &peace_bids::TABLES {
            assert_eq!(
                outcomes[table.index()],
                failed_capture(table, hour, force_full, "slow acquisition failed")
            );
        }
        let mut next =
            collector_certificates::slicing::SliceSelection::for_test(hour + 1, false, &[]);
        next.scan(&mut store, 9, Some(0));
        next.scan(&mut store, 7, Some(0));
        store.finish_scan();
        assert!(next.contains(9), "completed {completed}");
        assert_eq!(next.contains(7), !completed, "completed {completed}");
    }
    let request = captured.take_stability_request(&countries, 11, 0x1000, 9, false, None);
    assert!(captured.restore_stability_request(request, true).is_none());
}

#[test]
fn the_war_overview_is_its_own_job_and_fails_apart_from_slow() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let (hour, force_full) = (9, false);
    for completed in [true, false] {
        captured.prepare_hour(hour, force_full);
        let slow = captured.take_slow_request(
            &countries,
            11,
            0x1000,
            hour,
            force_full,
            SlowStores::default(),
        );
        let overview = captured.take_war_overview_request(&countries, 11, 0x1000, hour, force_full);
        captured.restore_slow_request(slow, false);
        captured.restore_war_overview_request(overview, completed);
        let outcomes = captured.outcomes();
        for &table in &war_overview::TABLES {
            let failed = |reason| failed_capture(table, hour, force_full, reason);
            assert_ne!(outcomes[table.index()], failed("slow acquisition failed"));
            assert_eq!(
                outcomes[table.index()] == failed("war overview acquisition failed"),
                !completed,
                "{table:?}"
            );
        }
        for &table in &peace_bids::TABLES {
            assert_eq!(
                outcomes[table.index()],
                failed_capture(table, hour, force_full, "slow acquisition failed")
            );
        }
    }
}

#[test]
fn modifier_header_verdict_travels_with_the_slow_pack_and_returns() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    captured.set_modifier_definitions_moved(true);
    let request =
        captured.take_slow_request(&countries, 11, 0x1000, 9, false, SlowStores::default());
    assert!(request.packs.modifier_definitions.header_changed);
    assert!(!captured.modifier_definitions.header_changed);
    captured.restore_slow_request(request, true);
    assert!(captured.modifier_definitions.header_changed);
    captured.set_modifier_definitions_moved(false);
    assert!(!captured.modifier_definitions.header_changed);
}

#[test]
fn production_handoffs_preserve_detail_failures_without_live_retry() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("accepted division detail failure must not retry live reads");
        }
    }
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.production_rest_pilot().rest_raw_mode = true;
    assert!(captured.production.rest_raw_mode);
    captured.production_rest_pilot().rest_raw_mode = false;
    assert!(!captured.production.rest_raw_mode);
    captured.test_resize_division_buffers([1; 6]);
    let tables = [
        TableId::DivisionEquipment,
        TableId::ArmyUpgradeGroups,
        TableId::ArmyUpgradeRequests,
        TableId::ArmyUpgradeDeliveries,
        TableId::ArmyUpgradeProduced,
    ];
    for reason in [
        "accepted division acquisition panicked",
        "owned division payload unavailable",
    ] {
        captured.drop_and_reset();
        captured.set_divisions_raw_mode(true);
        captured.production_rest_pilot().rest_raw_mode = true;
        for outcome in [
            &mut captured.production.division_equipment.outcome,
            &mut captured.production.groups.outcome,
            &mut captured.production.requests.outcome,
            &mut captured.production.deliveries.outcome,
            &mut captured.production.produced.outcome,
        ] {
            *outcome = Outcome::Rows { len: 1 };
        }
        captured.production.division_equipment.rows[0].division = 17;
        let memory = SourceReads::default();
        captured.capture_tables_all(&memory, 11, 0x1000, 2, false);
        assert!(!memory.reads.borrow().is_empty());
        let before = captured.outcomes();
        assert!(
            tables
                .iter()
                .all(|table| before[table.index()] == Outcome::Rows { len: 1 })
        );
        let bytes = captured.allocated_bytes();
        let capacities = captured.division_capture_capacities();

        captured.fail_division_details(reason);
        let outcomes = captured.outcomes();
        for (index, outcome) in outcomes.into_iter().enumerate() {
            if tables.iter().any(|table| table.index() == index) {
                assert_eq!(outcome, Outcome::Unavailable { reason });
            } else {
                assert_eq!(outcome, before[index], "unowned table {index}");
            }
        }
        for (table, view) in production::TABLES
            .into_iter()
            .zip(captured.production.tables())
        {
            if tables.contains(&table) {
                assert_eq!(
                    view,
                    collector_projection::table_conversion::unavailable(reason)
                );
            }
        }
        assert!(captured.production.divisions_raw_mode);
        let ticks = captured.production.acquire(
            &NoReads,
            11,
            0x1000,
            captured.divisions.sources(),
            2,
            false,
        );
        assert_eq!(ticks, [0; production::N]);
        assert_eq!(captured.outcomes(), outcomes);
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_eq!(captured.division_capture_capacities(), capacities);
        assert_eq!(captured.production.division_equipment.rows[0].division, 17);
    }
}

#[derive(Default)]
struct SourceReads {
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    bytes: Option<Vec<u8>>,
}
impl Memory for SourceReads {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, destination.len()));
        self.bytes.as_ref().is_some_and(|bytes| {
            engine_native_memory::memory::SliceMemory {
                base: air::wing_test_kit::BASE,
                bytes,
            }
            .copy(address, destination)
        })
    }
}

#[test]
fn country_naming_terminal_loan_restores_original_buffers_and_never_retries_native_sources() {
    struct RefusedReads(std::cell::Cell<usize>);
    impl Memory for RefusedReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            self.0.set(self.0.get() + 1);
            false
        }
    }
    let (base, state, hour) = (11, 0x1000, 9);
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    for rejected in [true, false] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut rows = Vec::<engine_native::ccountry::Row>::with_capacity(1);
        let mut links = Vec::<engine_native::ccountry::LinkRow>::with_capacity(1);
        let row_storage = rows.as_ptr();
        let link_storage = links.as_ptr();
        captured
            .country_appearance
            .raw
            .reserve(engine_native_memory::raw_country_observations::Capacity {
                links: 8,
                ..Default::default()
            })
            .unwrap();
        captured.country_appearance.raw.begin(hour);
        captured
            .country_appearance
            .raw
            .link(7, Err("original link refusal"));
        let retained_bytes = captured.allocated_bytes();
        let mut request = captured.take_countries_request(
            &countries,
            base,
            state,
            &mut rows,
            &mut links,
            (hour, true),
            None,
        );
        let mut templates = captured.take_templates_request(&countries, base, state, (hour, true));
        assert!(request.has_naming());
        let reason = if rejected {
            "country naming worker admission failed"
        } else {
            "acquisition worker stopped before completion"
        };
        let failure = if rejected {
            let report = request.fail_naming(reason);
            assert_eq!(report.countries, Outcome::Unavailable { reason });
            assert_eq!(report.links.result, Outcome::Unavailable { reason });
            let _ = templates.fail(reason);
            None
        } else {
            Some(reason)
        };
        captured.restore_countries_request(request, &mut rows, &mut links, failure);
        captured.restore_templates_request(templates, failure);
        assert_eq!(rows.as_ptr(), row_storage);
        assert_eq!(links.as_ptr(), link_storage);
        assert_eq!(captured.allocated_bytes(), retained_bytes);
        assert_eq!(
            captured.country_appearance.raw.map_loan().links.word(7),
            Some(Err("original link refusal")),
        );
        assert!(!captured.country_appearance.raw.complete());
        let memory = RefusedReads(std::cell::Cell::new(0));
        for _ in 0..2 {
            assert_eq!(
                captured
                    .country_appearance
                    .capture_at(&memory, base, state, hour),
                [0; country_appearance::N]
            );
            assert_eq!(
                // SAFETY: Isolated test exercises preacquired refusal on a local frame.
                unsafe {
                    captured
                        .division_templates
                        .capture(&memory, base, state, hour, true)
                },
                [0; division_templates::N]
            );
            assert!(
                captured
                    .country_appearance
                    .outcomes()
                    .into_iter()
                    .all(|outcome| outcome == Outcome::Unavailable { reason })
            );
            assert!(
                captured
                    .division_templates
                    .outcomes()
                    .into_iter()
                    .all(|outcome| outcome == Outcome::Unavailable { reason })
            );
        }
        assert_eq!(
            memory.0.get(),
            0,
            "completed refusal markers must suppress every retry"
        );
        captured
            .country_appearance
            .capture_at(&memory, base, state, hour + 1);
        // SAFETY: Isolated test exercises preacquired context mismatch on a local frame.
        unsafe {
            captured
                .division_templates
                .capture(&memory, base, state, hour + 1, true);
        }
        assert_eq!(memory.0.get(), 0, "context mismatch remains a refusal");
        captured.reset();
        captured
            .country_appearance
            .capture_at(&memory, base, state, hour + 1);
        assert!(
            memory.0.get() > 0,
            "reset must admit the next independent source pass"
        );
    }
}

#[test]
fn ordinary_terminal_outcomes_return_original_naval_store_and_forget_unpublished_fingerprint() {
    use collector_certificates::slicing::{SliceFingerprints, SliceSelection};
    let hour = 9;
    let owner = (1..=32)
        .find(|&owner| !replay_data::cadence::slice_active(owner, hour, false))
        .unwrap();
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    for refused_before_completion in [false, true] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut fingerprints = Box::new(SliceFingerprints::new());
        let mut prior = SliceSelection::new();
        prior.reset(hour, false);
        prior.scan(&mut fingerprints, owner, Some(71));
        assert!(prior.contains(owner));
        prior.reset(hour, false);
        prior.scan(&mut fingerprints, owner, Some(71));
        assert!(
            !prior.contains(owner),
            "control must establish unchanged outside-slice owner"
        );
        let original_store = std::ptr::from_ref(&*fingerprints);
        let parts = captured.take_ordinary_parts(
            &countries,
            11,
            0x1000,
            hour,
            Ok(()),
            crate::extended::NavalMissionLoan {
                force_full: false,
                fingerprints: Some(fingerprints),
            },
        );
        let mut request = captured.take_ordinary_request(&countries, 11, 0x1000, hour);
        let reason = if refused_before_completion {
            request.fail("ordinary worker admission failed");
            None
        } else {
            Some("ordinary worker stopped before completion")
        };
        let mut returned =
            captured.restore_ordinary_request(request, reason, parts.map(|part| (part, None)));
        assert_eq!(
            std::ptr::from_ref(&**returned.fingerprints.as_ref().unwrap()),
            original_store
        );
        assert!(!returned.force_full);
        let mut next = SliceSelection::new();
        next.reset(hour, false);
        next.scan(returned.fingerprints.as_mut().unwrap(), owner, Some(71));
        assert!(
            next.contains(owner),
            "terminal failure must invalidate unchanged-owner freshness"
        );
        for &id in super::ORDINARY_TABLES
            .iter()
            .flat_map(|tables| tables.iter())
        {
            assert!(matches!(
                captured.outcomes()[id.index()],
                Outcome::Unavailable { .. }
            ));
        }
    }
}

#[test]
fn scores_and_rails_keep_their_own_job_outcome_through_a_late_failure() {
    use engine_native_memory::memory::SliceMemory;
    let tables = [
        TableId::RaidAirScores,
        TableId::RailCooldowns,
        TableId::RailEdges,
    ];
    for completed in [true, false] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = engine_allocation::shared(
            engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
        )
        .unwrap();
        captured.prepare_hour(1, true);
        let mut orders = Orders::vacant();
        let mut request = captured.take_scores_rails_request(
            &countries,
            0,
            0x1000,
            1,
            true,
            Box::default(),
            None,
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &[],
        };
        let parts = &mut *request.parts;
        parts.scores.capture(
            &memory,
            0,
            0x1000,
            1,
            true,
            request.score_capacity,
            &mut request.air_scores,
        );
        parts.rails.capture(&memory, 0, 0x1000, 1, None);
        let (_scores, store) = captured.restore_scores_rails_request(request, completed);
        assert!(store.is_none());
        let installed = captured.outcomes();
        for id in tables {
            let outcome = installed[id.index()];
            assert!(matches!(outcome, Outcome::Unavailable { .. }), "{id:?}");
            assert_eq!(
                outcome
                    == Outcome::Unavailable {
                        reason: "scores and rails acquisition failed"
                    },
                !completed,
                "{id:?}"
            );
        }
        let request =
            captured.take_late_request(&countries, &mut orders, true, 11, 0x1000, 1, true);
        captured.restore_late_request(request, false, &mut orders);
        let after = captured.outcomes();
        for id in tables {
            assert_eq!(after[id.index()], installed[id.index()], "{id:?}");
        }
        assert_eq!(
            after[TableId::AirReferenceWitnesses.index()],
            Outcome::Unavailable {
                reason: "late acquisition failed"
            }
        );
    }
}

#[test]
fn army_icons_keep_their_own_job_outcome_through_a_late_failure() {
    use engine_native_memory::memory::SliceMemory;
    for completed in [true, false] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries = engine_allocation::shared(
            engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
        )
        .unwrap();
        captured.prepare_hour(1, true);
        let mut orders = Orders::vacant();
        let mut request = captured.take_army_icons_request(0);
        request.ticks = request.icons.capture(
            &SliceMemory {
                base: 0,
                bytes: &[],
            },
            0,
            request.divisions.sources(),
            None,
        );
        let captured_outcome = request.icons.outcomes()[0];
        captured.restore_army_icons_request(request, completed);
        let installed = captured.outcomes()[TableId::ArmyIconInputs.index()];
        let failed = Outcome::Unavailable {
            reason: "army icon acquisition failed",
        };
        assert_eq!(installed, if completed { captured_outcome } else { failed });
        let request =
            captured.take_late_request(&countries, &mut orders, true, 11, 0x1000, 1, true);
        captured.restore_late_request(request, false, &mut orders);
        assert_eq!(
            captured.outcomes()[TableId::ArmyIconInputs.index()],
            installed
        );
    }
}

#[test]
fn the_daily_production_tables_keep_slows_refusals_and_skip_off_their_slot() {
    use engine_native_memory::memory::CountedMemory;
    use replay_data::cadence::SKIPPED_REASON;
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let outcomes = |captured: &Captured| -> Vec<Outcome> {
        captured
            .industrial_organisations
            .outcomes()
            .into_iter()
            .chain(captured.production_modifiers.outcomes())
            .collect()
    };
    let refused = |captured: &Captured, reason: &'static str| {
        for outcome in outcomes(captured) {
            assert_eq!(outcome, Outcome::Unavailable { reason }, "{reason}");
        }
    };
    const SLOT: i32 = 8;
    assert!(ProductionDailyPacks::due(SLOT, false));
    let mut captured = Captured::with_capacities(&Capacities::default());
    let request = captured.take_production_daily_request(&countries, 11, 0x1000, SLOT, false);
    captured.restore_production_daily_request(request, false);
    captured.slow_ticks = Some([0; TABLE_COUNT]);
    captured.capture_tables(&NoRead, 11, 0x1000, SLOT, false);
    refused(&captured, "slow acquisition failed");
    captured.capture_tables(&NoRead, 11, 0x1000, SLOT, false);
    refused(&captured, "slow acquisition failed");
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.slow_ticks = Some([0; TABLE_COUNT]);
    let memory = CountedMemory::new(NoRead);
    for refusal in [None, Some("slow acquisition failed")] {
        captured.slow_refusal = refusal;
        let before = (
            captured.industrial_organisations.allocated_bytes(),
            captured.production_modifiers.allocated_bytes(),
        );
        for _ in 0..2 {
            let reads = collector_memory::read_accounting::Capture::new();
            captured.capture_tables(&memory, 11, 0x1000, SLOT + 1, false);
            let reads = reads.finish();
            for table in industrial_organisations::TABLES
                .into_iter()
                .chain(production_modifiers::TABLES)
            {
                assert_eq!(
                    reads[table.index()],
                    engine_native_memory::memory::ReadCounts::ZERO,
                    "{table:?}"
                );
            }
            refused(&captured, SKIPPED_REASON);
            assert_eq!(
                (
                    captured.industrial_organisations.allocated_bytes(),
                    captured.production_modifiers.allocated_bytes(),
                ),
                before
            );
        }
    }
}

#[test]
fn the_daily_production_tables_follow_slow_and_their_own_job_through_assembly() {
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Job {
        Completed,
        Failed,
        Missing,
    }
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let outcomes = |captured: &Captured| -> Vec<Outcome> {
        captured
            .industrial_organisations
            .outcomes()
            .into_iter()
            .chain(captured.production_modifiers.outcomes())
            .collect()
    };
    let hour_of =
        |captured: &mut Captured, slow: Job, mio: Job, daily: Job, hour: i32, full: bool| {
            match slow {
                Job::Missing => captured.fail_missing_slow(hour, full),
                done => {
                    let request = captured.take_slow_request(
                        &countries,
                        11,
                        0x1000,
                        hour,
                        full,
                        SlowStores::default(),
                    );
                    let _ = captured.restore_slow_request(request, done == Job::Completed);
                }
            }
            let mio_request = (mio != Job::Missing).then(|| {
                let mut request = captured
                    .take_industrial_organisations_request(&countries, 11, 0x1000, hour, full);
                let ticks = request.captured.capture(&NoRead, 11, 0x1000, hour, full);
                (request, ticks)
            });
            if daily != Job::Missing {
                let mut request =
                    captured.take_production_daily_request(&countries, 11, 0x1000, hour, full);
                request.packs.capture(&NoRead, 11, 0x1000, hour, full);
                if let Some((mio_request, ticks)) = mio_request {
                    captured.restore_industrial_organisations_request(
                        mio_request,
                        mio == Job::Completed,
                        ticks,
                        &mut request.packs,
                    );
                }
                captured.restore_production_daily_request(request, daily == Job::Completed);
            } else if let Some((mio_request, ticks)) = mio_request {
                let mut packs = captured.production_daily_spare.take().unwrap();
                captured.restore_industrial_organisations_request(
                    mio_request,
                    mio == Job::Completed,
                    ticks,
                    &mut packs,
                );
                std::mem::swap(
                    &mut captured.industrial_organisations,
                    &mut packs.industrial_organisations,
                );
                captured.production_daily_spare = Some(packs);
            }
            let mut ticks = [0; TABLE_COUNT];
            captured.assemble_production_daily(&NoRead, 11, 0x1000, hour, full, &mut ticks, false);
            let first = outcomes(captured);
            captured.assemble_production_daily(&NoRead, 11, 0x1000, hour, full, &mut ticks, false);
            assert_eq!(outcomes(captured), first, "a second assembly changed them");
            first
        };
    let refused = |reason: &'static str| {
        vec![Outcome::Unavailable { reason }; industrial_organisations::N + production_modifiers::N]
    };
    for (hour, full) in [(8, false), (3, true)] {
        assert!(ProductionDailyPacks::due(hour, full));
        let healthy = hour_of(
            &mut Captured::with_capacities(&Capacities::default()),
            Job::Completed,
            Job::Completed,
            Job::Completed,
            hour,
            full,
        );
        let mut serial = Captured::with_capacities(&Capacities::default());
        serial.assemble_production_daily(
            &NoRead,
            11,
            0x1000,
            hour,
            full,
            &mut [0; TABLE_COUNT],
            true,
        );
        assert_eq!(
            healthy,
            outcomes(&serial),
            "independent native faults changed order"
        );
        for reason in ["slow acquisition failed", "slow acquisition not supplied"] {
            assert_ne!(healthy, refused(reason), "{hour} {full}");
        }
        for slow in [Job::Completed, Job::Failed, Job::Missing] {
            for mio in [Job::Completed, Job::Failed, Job::Missing] {
                for daily in [Job::Completed, Job::Failed, Job::Missing] {
                    let expected = match (slow, mio, daily) {
                        (Job::Failed, _, _)
                        | (Job::Completed, _, Job::Failed)
                        | (Job::Completed, Job::Failed, Job::Completed) => {
                            refused("slow acquisition failed")
                        }
                        (Job::Missing, _, _)
                        | (Job::Completed, _, Job::Missing)
                        | (Job::Completed, Job::Missing, Job::Completed) => {
                            refused("slow acquisition not supplied")
                        }
                        (Job::Completed, Job::Completed, Job::Completed) => healthy.clone(),
                    };
                    let mut captured = Captured::with_capacities(&Capacities::default());
                    assert_eq!(
                        hour_of(&mut captured, slow, mio, daily, hour, full),
                        expected,
                        "{slow:?} {mio:?} {daily:?} {hour} {full}"
                    );
                    captured.reset();
                    assert_eq!(
                        hour_of(
                            &mut captured,
                            Job::Completed,
                            Job::Completed,
                            Job::Completed,
                            hour,
                            full,
                        ),
                        healthy,
                        "after {slow:?} {mio:?} {daily:?} {hour} {full}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_split_production_modifier_capture_follows_its_loans_through_assembly() {
    use production_modifiers::{Captured as Modifiers, test_support};
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Returned {
        Completed,
        Failed,
        Missing,
    }
    let countries = engine_allocation::shared(
        engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
    )
    .unwrap();
    let context = engine_production::production_modifiers::Context {
        start: 17,
        cap: 93,
        gain: -5,
        ..Default::default()
    };
    let rows: Vec<(u32, usize, usize)> = (1..=5_000_u32)
        .filter(|id| !(2_000..2_600).contains(id))
        .map(|id| {
            (
                id,
                0x10_0000 - id as usize * 16,
                if id % 7 == 0 { 3 } else { 1 },
            )
        })
        .collect();
    let range = |first: u32, end: u32| -> Vec<(u32, usize, usize)> {
        rows.iter()
            .copied()
            .filter(|&(id, _, _)| first <= id && id < end)
            .collect()
    };
    let starts = [1_200, 2_000, 2_600];
    let bounds = [1, 1_200, 2_000, 2_600, u32::MAX];
    let tables = |modifiers: &Modifiers| (modifiers.outcomes(), modifiers.tables());
    let whole = tables(&Modifiers::whole(test_support::part(&rows, context)));
    let hour = |captured: &mut Captured,
                parts: [Option<production_modifiers::Part>; 4],
                returned: [Returned; 4],
                daily: bool| {
        captured
            .production_modifiers
            .grow(&mut production_modifiers::Capacities::default());
        assert_eq!(
            captured.prepare_production_modifier_parts(&NoRead, 11, &starts),
            4
        );
        assert_eq!(
            captured.production_modifiers.prepared_slots(),
            bounds
                .windows(2)
                .map(|pair| pair[0]..pair[1])
                .collect::<Vec<_>>()
        );
        let [first, rest @ ..] = parts;
        let mut loans: Vec<_> = (1..4)
            .map(|part| {
                captured.take_production_modifier_part(part, &countries, 11, 0x1000, 8, false)
            })
            .collect();
        for (loan, part) in loans.iter_mut().zip(rest) {
            *loan.captured = part.unwrap();
        }
        let mio = captured.take_industrial_organisations_request(&countries, 11, 0x1000, 8, false);
        let mut request = captured.take_production_daily_request(&countries, 11, 0x1000, 8, false);
        request
            .packs
            .production_modifiers
            .captured_as(first.unwrap());
        captured.restore_industrial_organisations_request(
            mio,
            true,
            [0; industrial_organisations::N],
            &mut request.packs,
        );
        captured.restore_production_daily_request(request, daily);
        for (loan, returned) in loans.into_iter().zip(&returned[1..]) {
            if *returned != Returned::Missing {
                captured.restore_production_modifier_part(
                    loan,
                    *returned == Returned::Completed,
                    0,
                );
            }
        }
        let mut ticks = [0; TABLE_COUNT];
        captured.assemble_production_daily(&NoRead, 11, 0x1000, 8, false, &mut ticks, false);
        let first = tables(&captured.production_modifiers);
        captured.assemble_production_daily(&NoRead, 11, 0x1000, 8, false, &mut ticks, false);
        assert_eq!(tables(&captured.production_modifiers), first);
        first
    };
    let split = |bounds: &[u32]| -> [Option<production_modifiers::Part>; 4] {
        std::array::from_fn(|part| {
            Some(test_support::part(
                &range(bounds[part], bounds[part + 1]),
                context,
            ))
        })
    };
    let all = [Returned::Completed; 4];
    let refused = |reason: &'static str| {
        (
            [Outcome::Unavailable { reason }; production_modifiers::N],
            [
                collector_projection::table_conversion::unavailable(reason),
                collector_projection::table_conversion::unavailable(reason),
            ],
        )
    };
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(hour(&mut captured, split(&bounds), all, true), whole);
    for returned in [Returned::Failed, Returned::Missing] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut parts = all;
        parts[2] = returned;
        assert_eq!(
            hour(&mut captured, split(&bounds), parts, true),
            refused("slow acquisition failed")
        );
    }
    let mut parts = split(&bounds);
    parts[1] = Some(test_support::failed("second range's failure"));
    parts[3] = Some(test_support::failed("fourth range's failure"));
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(
        hour(&mut captured, parts, all, true),
        refused("second range's failure")
    );
    let mut parts = split(&bounds);
    parts[0] = Some(test_support::part(&[], context));
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(
        hour(&mut captured, parts, all, true),
        tables(&Modifiers::whole(test_support::part(
            &range(1_200, u32::MAX),
            context
        )))
    );
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(
        hour(&mut captured, split(&bounds), all, false),
        refused("slow acquisition failed")
    );
    captured.reset();
    assert_eq!(
        captured.prepare_production_modifier_parts(&NoRead, 11, &[]),
        1
    );
    captured.reset();
    let mut ticks = [0; TABLE_COUNT];
    captured.assemble_production_daily(&NoRead, 11, 0x1000, 9, false, &mut ticks, false);
    assert_eq!(
        captured.production_modifiers.outcomes(),
        [Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON
        }; production_modifiers::N]
    );
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured
        .production_modifiers
        .grow(&mut production_modifiers::Capacities::default());
    assert_eq!(
        captured.prepare_production_modifier_parts(&NoRead, 11, &starts),
        4
    );
    captured.reset();
    let mio = captured.take_industrial_organisations_request(&countries, 11, 0x1000, 8, false);
    let mut request = captured.take_production_daily_request(&countries, 11, 0x1000, 8, false);
    request
        .packs
        .production_modifiers
        .captured_as(test_support::part(&rows, context));
    captured.restore_industrial_organisations_request(
        mio,
        true,
        [0; industrial_organisations::N],
        &mut request.packs,
    );
    captured.restore_production_daily_request(request, true);
    let mut ticks = [0; TABLE_COUNT];
    captured.assemble_production_daily(&NoRead, 11, 0x1000, 8, false, &mut ticks, false);
    assert_eq!(tables(&captured.production_modifiers), whole);
    let hour_at = |captured: &mut Captured, starts: &[u32], captured_first: bool, daily: bool| {
        captured
            .production_modifiers
            .grow(&mut production_modifiers::Capacities::default());
        let parts = captured.prepare_production_modifier_parts(&NoRead, 11, starts);
        assert_eq!(parts, starts.len() + 1);
        let bounds: Vec<u32> = std::iter::once(1)
            .chain(starts.iter().copied())
            .chain(std::iter::once(u32::MAX))
            .collect();
        let slots: Vec<_> = bounds.windows(2).map(|pair| pair[0]..pair[1]).collect();
        assert_eq!(captured.production_modifiers.prepared_slots(), slots);
        let loans: Vec<_> = (1..parts)
            .map(|part| {
                let mut loan =
                    captured.take_production_modifier_part(part, &countries, 11, 0x1000, 8, false);
                assert_eq!(loan.slots, slots[part]);
                *loan.captured =
                    test_support::part(&range(slots[part].start, slots[part].end), context);
                loan
            })
            .collect();
        let mio = captured.take_industrial_organisations_request(&countries, 11, 0x1000, 8, false);
        let mut request = captured.take_production_daily_request(&countries, 11, 0x1000, 8, false);
        if captured_first {
            request
                .packs
                .production_modifiers
                .captured_as(test_support::part(&range(1, slots[0].end), context));
        }
        captured.restore_industrial_organisations_request(
            mio,
            true,
            [0; industrial_organisations::N],
            &mut request.packs,
        );
        captured.restore_production_daily_request(request, daily);
        for loan in loans {
            captured.restore_production_modifier_part(loan, true, 0);
        }
        let mut ticks = [0; TABLE_COUNT];
        captured.assemble_production_daily(&NoRead, 11, 0x1000, 8, false, &mut ticks, false);
        tables(&captured.production_modifiers)
    };
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(
        hour_at(&mut captured, &starts, false, false),
        refused("slow acquisition failed")
    );
    assert_eq!(
        captured.production_modifiers.prepared_slots(),
        std::iter::once(1..u32::MAX).collect::<Vec<_>>()
    );
    let mio = captured.take_industrial_organisations_request(&countries, 11, 0x1000, 8, false);
    let mut request = captured.take_production_daily_request(&countries, 11, 0x1000, 8, false);
    request
        .packs
        .production_modifiers
        .captured_as(test_support::part(&rows, context));
    captured.restore_industrial_organisations_request(
        mio,
        true,
        [0; industrial_organisations::N],
        &mut request.packs,
    );
    captured.restore_production_daily_request(request, true);
    let mut ticks = [0; TABLE_COUNT];
    captured.assemble_production_daily(&NoRead, 11, 0x1000, 8, false, &mut ticks, false);
    assert_eq!(tables(&captured.production_modifiers), whole);
    captured.reset();
    captured.assemble_production_daily(&NoRead, 11, 0x1000, 9, false, &mut ticks, false);
    assert_eq!(
        captured.production_modifiers.outcomes(),
        [Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON
        }; production_modifiers::N]
    );
    assert_eq!(
        captured.production_modifiers.prepared_slots(),
        std::iter::once(1..u32::MAX).collect::<Vec<_>>()
    );
    let mut captured = Captured::with_capacities(&Capacities::default());
    for starts in [&starts[..], &[2_600][..], &[][..]] {
        assert_eq!(
            hour_at(&mut captured, starts, true, true),
            whole,
            "{starts:?}"
        );
        captured.reset();
    }
}

#[test]
fn map_assembly_refuses_the_packs_it_leaves_out_and_keeps_the_rest() {
    use replay_data::scope::{self, CaptureMode};
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let gated: Vec<TableId> = [
        &country_stability::TABLES[..],
        &war_overview::TABLES,
        &raid_target_statuses::TABLES,
        &supply_trains::TABLES,
        &industrial_organisations::TABLES,
        &production_modifiers::TABLES,
    ]
    .concat();
    let hour = 8;
    let outcomes = [CaptureMode::Full, CaptureMode::Map].map(|mode| {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.set_capture_mode(mode);
        assert_eq!(captured.capture_mode(), mode);
        captured.slow_ticks = Some([0; TABLE_COUNT]);
        captured.capture_tables(&NoRead, 11, 0x1000, hour, true);
        captured.outcomes()
    });
    let [full, map] = outcomes;
    let not_recorded = Outcome::Unavailable {
        reason: NOT_RECORDED,
    };
    for table in TableId::ALL {
        let index = table.index();
        if gated.contains(&table) {
            assert_eq!(map[index], not_recorded, "{table:?}");
            assert_ne!(full[index], not_recorded, "{table:?}");
        } else if scope::records(CaptureMode::Map, table) {
            assert_ne!(map[index], not_recorded, "{table:?}");
        }
    }
}

#[test]
fn independent_variant_family_survives_every_final_input_return_order_without_allocating() {
    for variant_order in 0..3 {
        for failure in [None, Some("final input continuation failed")] {
            let mut capacity = Capacities::default();
            capacity.production.rows[3..7].fill(67);
            capacity.production.definition_bytes = 4096;
            capacity.production.rows[0] = 1500;
            let mut captured = Captured::with_capacities(&capacity);
            let mut variant = Some(captured.take_variants_request(0, 0, 60759371, true));
            variant
                .as_mut()
                .unwrap()
                .variants
                .fail("original variant verdict", 60759371, true);
            variant
                .as_mut()
                .unwrap()
                .changes
                .fail("original changes verdict");
            variant.as_mut().unwrap().changes_finished = true;
            let bytes = captured.allocated_bytes() + variant.as_ref().unwrap().allocated_bytes();
            let countries = engine_allocation::shared(
                engine_native_intelligence::country_memory::Cache::try_with_capacity(0).unwrap(),
            )
            .unwrap();
            collector_test_support::with_allocations_denied(|| {
                if variant_order == 0 {
                    captured
                        .restore_variants_request(variant.take().unwrap(), Ok([0; production::N]));
                }
                let request = captured.take_final_inputs_request(&countries, 0, 0, 60759371, true);
                if variant_order == 1 {
                    captured
                        .restore_variants_request(variant.take().unwrap(), Ok([0; production::N]));
                }
                captured.restore_final_inputs_request(
                    request,
                    FinalInputsReport::default(),
                    failure,
                );
                if variant_order == 2 {
                    captured
                        .restore_variants_request(variant.take().unwrap(), Ok([0; production::N]));
                }
            });
            assert_eq!(captured.allocated_bytes(), bytes);
            let outcomes = captured.production.outcomes();
            for outcome in &outcomes[3..7] {
                assert_eq!(
                    *outcome,
                    Outcome::Unavailable {
                        reason: "original variant verdict"
                    }
                );
            }
            assert_eq!(
                outcomes[17],
                Outcome::Unavailable {
                    reason: "original changes verdict"
                }
            );
            assert_eq!(captured.production.lines.rows.capacity(), 1500);
        }
    }
}

#[test]
fn variant_lanes_are_claimed_behind_the_division_status_chain() {
    use crate::acquisition_jobs::{Batch, Job, Pool, Report};
    use collector_workers::roles::Role;
    let pool = Pool::with_test_runner(2, |job| match job {
        Job::Variants(_) => Report::Variants([0; production::N]),
        _ => panic!("only Variants was submitted"),
    });
    let mut captured = Captured::with_capacities(&Capacities::default());
    let mut request = Some(Job::Variants(
        captured.take_variants_request(0, 0x1000, 1, true),
    ));
    let mut batch = Batch::new(&pool);
    let ticket = unsafe { batch.submit(&mut request) }.unwrap();
    let completed = batch.finish(ticket).unwrap();
    let Job::Variants(request) = completed.request else {
        panic!("the variants request must be returned");
    };
    let lanes = request.workers.priority();
    for role in [
        Role::Variants,
        Role::PresentationPrepare,
        Role::PresentationPreparation,
        Role::PresentationSettle,
        Role::PresentationStatus,
        Role::Presentation,
    ] {
        assert!(role.priority() < lanes);
    }
    captured.restore_variants_request(request, Ok([0; production::N]));
}
