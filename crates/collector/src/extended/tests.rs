use super::*;
use engine_native::carmy;
use std::collections::BTreeSet;

#[test]
fn periodic_live_input_sizing_keeps_land_events_prompt_and_skips_worker_hints() {
    struct Unreadable;
    impl Memory for Unreadable {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let failures = || {
        let status = serde_json::to_value(crate::country_scale::demand_diagnostics()).unwrap();
        [
            "advisor_modifiers",
            "production_modifiers",
            "land_event_words",
        ]
        .map(|name| status[name]["failed_observations"].as_u64().unwrap())
    };
    let mut capacities = Capacities::default();
    let before = failures();
    assert!(
        capacities
            .presize_live_inputs_periodic(&Unreadable, 0, false)
            .is_err()
    );
    assert_eq!(failures(), [before[0], before[1], before[2] + 1]);
    assert!(capacities.presize_live_inputs(&Unreadable, 0).is_err());
    assert_eq!(failures(), [before[0] + 1, before[1] + 1, before[2] + 2]);
}

#[test]
fn final_worker_loan_preserves_mod_sized_storage_without_callback_allocation() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.divisions = Divisions::new(5_000);
    let source_storage = captured.divisions.first.as_ptr();
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    crate::test_support::with_allocations_denied(|| {
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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
    ];
    let assert_vacant = |captured: &Captured| {
        // The boxes remain reusable; none of their parked packs owns a
        // second row, arena, hash or source allocation.
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
        let mio = crate::test_support::with_allocations_denied(|| {
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
        crate::test_support::with_allocations_denied(|| {
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
        let mio = crate::test_support::with_allocations_denied(|| {
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
            Box::new(crate::slicing::SliceFingerprints::new()),
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
        captured.air_root = Some(Outcome::Rows { len: 7 });
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
        // These two auxiliary placeholders were already vacant before
        // this change; only the four late table packs lose duplicates.
        removed[2] =
            request.packs.allocated_bytes() - box_bytes[2] - request.packs.orders.allocated_bytes();
        assert!(request.packs.air.stats_raw_mode);
        assert!(request.packs.naval_history.raw_mode);
        assert_eq!(request.packs.air_root, Some(Outcome::Rows { len: 7 }));
        assert_eq!(request.packs.orders.geometry.as_ptr(), order_storage);
        // The division sources stay in the frame for the final borrowers.
        assert_eq!(captured.divisions.first.as_ptr(), division_storage);
        assert!(!captured.air.stats_raw_mode);
        assert!(!captured.naval_history.raw_mode);
        assert_eq!(captured.air_root, None);
        captured.restore_late_request(request, completed, &mut orders);
        assert_eq!(
            std::ptr::from_ref(&**captured.late_spare.as_ref().unwrap()),
            late_box
        );
        assert!(captured.air.stats_raw_mode);
        assert!(captured.naval_history.raw_mode);
        assert_eq!(captured.air_root, Some(Outcome::Rows { len: 7 }));
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let mut orders = Orders::vacant();
    for pointer in [0x1234, 0x5678] {
        let inputs = Arc::new(UnitInputs::fixture(
            vec![carmy::DivisionSeed {
                pointer,
                ..Default::default()
            }],
            Outcome::Rows { len: 1 },
        ));
        captured.install_unit_inputs(Some(Arc::clone(&inputs)));
        let request =
            captured.take_late_request(&countries, &mut orders, true, 11, 0x1000, 9, false);
        assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
        assert_eq!(request.inputs.as_ref().unwrap().seeds()[0].pointer, pointer);
        assert_eq!(Arc::strong_count(&inputs), 3);
        captured.restore_late_request(request, true, &mut orders);
        assert_eq!(Arc::strong_count(&inputs), 2);
        captured.release_unit_inputs();
        assert_eq!(Arc::strong_count(&inputs), 1);
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let inputs = Arc::new(UnitInputs::fixture(
        vec![carmy::DivisionSeed {
            pointer: 0x1234,
            handle: Some(7),
            ..Default::default()
        }],
        Outcome::Rows { len: 1 },
    ));
    let bytes = captured.allocated_bytes();
    let presentation_bytes = captured.division_presentation.allocated_bytes();
    assert!(presentation_bytes > 0);
    let report = division_presentation::CaptureReport {
        ticks: [89, 97, 5, 103, 0, 0, 0, 7, 11, 13, 17, 19],
    };
    captured.install_unit_inputs(Some(Arc::clone(&inputs)));
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
    assert!(Arc::ptr_eq(&request.countries, &countries));
    assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
    assert_eq!(request.inputs.as_ref().unwrap().seeds(), inputs.seeds());
    assert_eq!(Arc::strong_count(&countries), 2);
    assert_eq!(Arc::strong_count(&inputs), 3);
    captured.restore_presentation_request(request, Ok(report));
    assert_eq!(captured.allocated_bytes(), bytes);
    assert_eq!(
        captured.division_presentation.allocated_bytes(),
        presentation_bytes
    );
    assert_eq!(Arc::strong_count(&countries), 1);
    assert_eq!(Arc::strong_count(&inputs), 2);
    assert!(Arc::ptr_eq(captured.unit_inputs.as_ref().unwrap(), &inputs));
    let pending = captured.presentation_report.as_ref().unwrap();
    assert_eq!(pending.ticks, report.ticks);
    // Reset discards a pending report and releases the frame's source lease.
    captured.drop_and_reset();
    assert!(captured.presentation_report.is_none());
    assert_eq!(captured.allocated_bytes(), bytes);
    assert_eq!(Arc::strong_count(&inputs), 1);
}

/// The retired division ETA inputs and attrition breakdowns are neither
/// scheduled, acquired, sized nor reported by any pack or the division group.
#[test]
fn retired_division_tables_are_never_scheduled() {
    for retired in [
        TableId::DivisionEtaInputs,
        TableId::AttritionModifierBreakdowns,
    ] {
        assert!(!TABLES.contains(&retired), "{retired:?}");
        assert!(!DIVISION_SOURCE_TABLES.contains(&retired), "{retired:?}");
        assert!(
            !read_groups()
                .any(|(_, anchor, members)| anchor == retired || members.contains(&retired)),
            "{retired:?}"
        );
    }
    assert!(!PACK_NAMES.contains(&"division_eta_inputs"));
    assert!(
        !division_presentation::PHASE_NAMES.contains(&"attack"),
        "the presentation pass has no selected-attack phase"
    );
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
            // Presentation begins with this geometry root even for an empty
            // census. New province tables legitimately read the same root,
            // so the strict probe applies to the family handoff itself.
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let inputs = Arc::new(UnitInputs::empty_rows());
    let presentation_bytes = captured.division_presentation.allocated_bytes();
    let report = division_presentation::CaptureReport {
        ticks: [113, 227, 5, 449, 0, 0, 0, 3, 23, 29, 31, 37],
    };
    for failure in [
        None,
        Some(crate::acquisition::Failure::Panicked),
        Some(crate::acquisition::Failure::Stopped),
    ] {
        captured.drop_and_reset();
        captured.install_unit_inputs(Some(Arc::clone(&inputs)));
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
        // Exercise the same completed result through global table routing,
        // where unrelated province consumers may read this native root.
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
        // The same probe must detect an actual acquisition, making absence
        // above meaningful even when every unrelated read fails immediately.
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
    use engine_native::{memory::CountedMemory, memory::SliceMemory};
    use replay_frame::frame::TableView;

    // The template tables belong to the Countries job, not the Slow pack.
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
    // StateResistance now belongs to the independent hourly States job.
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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
        let launch = Box::new(crate::slicing::SliceFingerprints::new());
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
            Box::new(crate::slicing::SliceFingerprints::new()),
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

        // A second assembly must not turn a consumed failure marker into
        // an unsubmitted job and retry its reads on the callback.
        for _ in 0..2 {
            let counted = crate::read_accounting::Capture::new();
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
                TableInput::Sampled(crate::spooling::unavailable("fixture"))
            });
            captured.insert_tables(
                hour,
                &mut tables,
                &mut crate::spooling::SpoolConversion {
                    variables_baseline: None,
                    variables_scratch: &mut Default::default(),
                    characters: None,
                    lexical: None,
                    intel: None,
                    projection: None,
                    helper: None,
                },
                false,
            );
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
                        engine_native::memory::ReadCounts::ZERO,
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
        // Reset starts a new observation. The normal path must still read.
        captured.drop_and_reset();
        let counted = crate::read_accounting::Capture::new();
        let mut ticks = captured.capture_tables(&memory, 11, 0x1000, hour, true);
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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
        Box::new(crate::slicing::SliceFingerprints::new()),
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
    // The sliced project tables are this job's own read.
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.prepare_hour(0, true);
    let before = captured.allocated_bytes();
    let mut request = captured.take_air_missions_request(&countries, 11, 0x1000, None);
    // The pool refused it: the callback fails it instead of running it.
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

/// A failed Ordinary family holds no counter-walk census, wherever it
/// failed: the task-force part panicked, another part stopped after it
/// completed, or the family was refused admission. A family that returns
/// without a failure keeps them.
#[test]
fn ordinary_terminal_failures_forget_the_counter_censuses() {
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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
        let units = Arc::new(UnitInputs::fixture(Vec::new(), Outcome::Rows { len: 0 }));
        captured.install_unit_inputs(Some(Arc::clone(&units)));
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

/// A completed Ordinary family keeps its counter censuses through the
/// assembly that consumes its completion marker and the per-hour reset; a
/// second assembly call, which finds no marker and fails the family, forgets
/// them, as does an hour whose family never supplied its marker.
#[test]
fn ordinary_terminal_missing_marker_forgets_the_counter_censuses() {
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let mut captured = Captured::with_capacities(&Capacities::default());
    let units = Arc::new(UnitInputs::fixture(Vec::new(), Outcome::Rows { len: 0 }));
    captured.install_unit_inputs(Some(Arc::clone(&units)));
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
    // The marker was consumed: a second assembly call fails the family.
    captured.capture_tables(&NoRead, 11, 0x1000, 1, false);
    assert_eq!(
        captured.ordinary_failure,
        Some("ordinary acquisition not supplied")
    );
    assert_eq!(captured.naval_counters.held_censuses(), 0);
    captured.drop_and_reset();
    assert_eq!(captured.naval_counters.held_censuses(), 0);
    // An hour whose family never returned its marker.
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
    // The test-only failure setter forgets them too.
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    for reason in [
        "ordinary worker admission failed",
        "acquisition worker panicked",
        "acquisition worker stopped before completion",
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let units = Arc::new(UnitInputs::fixture(Vec::new(), Outcome::Rows { len: 0 }));
        captured.install_unit_inputs(Some(Arc::clone(&units)));
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
        // One refused or failed part refuses the whole family, as the single
        // Ordinary job did; the others completed.
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
        // A missing completion marker also cannot reacquire live data.
        captured.capture_tables(&memory, 11, 0x1000, 1, false);
        assert_eq!(memory.combat.get(), 0);
        assert_eq!(
            captured.ordinary_failure,
            Some("ordinary acquisition not supplied")
        );
        captured.drop_and_reset();
        assert!(captured.ordinary_failure.is_none());
        assert_eq!(Arc::strong_count(&units), 1);
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
        assert_eq!(Arc::strong_count(&countries), 1);
    }
}

#[test]
fn unpublished_raid_slices_are_read_again_at_the_next_hour() {
    use crate::slicing::{SliceFingerprints, SliceSelection};
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let memory = engine_native::memory::SliceMemory {
        base: 0,
        bytes: &[],
    };
    let (hour, force_full) = (7, false);
    // Owner 7 (in this hour's slice) was seen, unchanged, before this hour.
    let seen = |store: &mut SliceFingerprints| {
        let mut selection = SliceSelection::for_test(0, true, &[]);
        selection.scan(store, 7, Some(1));
        store.finish_scan();
    };
    for completed in [true, false] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.prepare_hour(hour, force_full);
        let mut launch = Box::new(crate::slicing::SliceFingerprints::new());
        let mut state = RaidStateStores::new();
        for store in [&mut launch, &mut state.states, &mut state.buildings] {
            seen(store);
        }
        let mut state_request =
            captured.take_raid_state_request(&countries, 11, 0x1000, hour, force_full, state);
        let mut launch_request =
            captured.take_raid_launch_request(&countries, 11, 0x1000, hour, force_full, launch);
        if completed {
            // The walks fail on an empty image: nothing is published.
            state_request.packs.capture(
                &memory,
                11,
                0x1000,
                hour,
                force_full,
                &mut state_request.stores,
            );
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
            (crate::slicing::RAID_LAUNCH, &mut launch),
            (crate::slicing::RAID_STATE, &mut state.states),
            (crate::slicing::RAID_BUILDINGS, &mut state.buildings),
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let (hour, force_full) = (7, false);
    let capacity = Capacities {
        governance: governance::Capacities::for_test_research(1),
        ..Default::default()
    };
    for failure in [
        None,
        Some(crate::acquisition::Failure::Panicked),
        Some(crate::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&capacity);
        captured.prepare_hour(hour, force_full);
        let bytes = captured.allocated_bytes();
        // Owner 7 (in this hour's slice) was seen, unchanged, before this hour.
        let mut store = Box::new(crate::slicing::SliceFingerprints::new());
        let mut seen = crate::slicing::SliceSelection::for_test(0, true, &[]);
        seen.scan(&mut store, 7, Some(1));
        store.finish_scan();
        let research =
            captured.take_research_request(&countries, 11, 0x1000, hour, force_full, store);
        // The family keeps no research capacity while the part is loaned.
        assert!(captured.allocated_bytes() < bytes);
        let governance = captured.take_governance_request(
            &countries,
            11,
            0x1000,
            hour,
            force_full,
            Box::new(crate::slicing::SliceFingerprints::new()),
            Box::new(crate::slicing::SliceFingerprints::new()),
            None,
            None,
            None,
        );
        // Governance fails; its restore must not reach the loaned research rows.
        let _technologies = captured
            .restore_governance_request(governance, Err(crate::acquisition::Failure::Panicked));
        let report = match failure {
            None => Ok(crate::acquisition_jobs::ResearchReport { ticks: [5, 6, 7] }),
            Some(failure) => Err(failure),
        };
        let mut store = captured.restore_research_request(research, report);
        // This hour's research read was not published (a job failure
        // forgets the store; a complete job whose tables are not all
        // complete invalidates its selection), so owner 7 is read again
        // at the next hour although nothing changed.
        let mut next = crate::slicing::SliceSelection::for_test(hour + 1, false, &[]);
        next.scan(&mut store, 7, Some(1));
        assert!(next.contains(7), "{failure:?}");
        assert_eq!(captured.allocated_bytes(), bytes);
        let outcomes = captured.governance.outcomes();
        let family = captured
            .governance_report
            .expect("governance report restored");
        match failure {
            None => {
                // Costs land on the research tables; the family's failure
                // did not replace the research outcomes.
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
                    crate::acquisition::Failure::Panicked => "research acquisition panicked",
                    crate::acquisition::Failure::Stopped => {
                        "research acquisition stopped before reading"
                    }
                };
                for index in governance::RESEARCH {
                    assert_eq!(family.ticks[index], 0);
                    assert_eq!(outcomes[index], Outcome::Unavailable { reason: expected });
                }
            }
        }
        // The family's own tables carry the family failure either way.
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
    let memory = engine_native::memory::CountedMemory::new(Reads(std::cell::Cell::new(0)));
    let bytes = captured.allocated_bytes();
    let mut request = crate::test_support::with_allocations_denied(|| {
        captured.take_variants_request(11, 0x1000, slot + 1, false)
    });
    assert_eq!(
        captured.allocated_bytes() + request.allocated_bytes(),
        bytes
    );
    // The worker always owns the hourly changes. A quiet roster stays skipped.
    request.registry.mark_skipped();
    request
        .variants
        .fail(replay_data::cadence::SKIPPED_REASON, slot + 1, false);
    request.changes_ticks = 79;
    crate::test_support::with_allocations_denied(|| {
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
        crate::acquisition::Failure::Panicked,
        crate::acquisition::Failure::Stopped,
    ] {
        let bytes = captured.allocated_bytes();
        let request = captured.take_variants_request(11, 0x1000, slot, false);
        assert_eq!(
            captured.allocated_bytes() + request.allocated_bytes(),
            bytes
        );
        crate::test_support::with_allocations_denied(|| {
            captured.restore_variants_request(request, Err(failure));
        });
        assert_eq!(captured.allocated_bytes(), bytes);
        let reads = crate::read_accounting::Capture::new();
        captured.capture_tables_all(&memory, 11, 0x1000, slot, false);
        let counts = reads.finish();
        for table in production::TABLES[3..7]
            .iter()
            .chain([&TableId::VariantChanges])
        {
            assert_eq!(
                counts[table.index()],
                engine_native::memory::ReadCounts::ZERO
            );
        }
        let expected = match failure {
            crate::acquisition::Failure::Panicked => "variant acquisition panicked",
            crate::acquisition::Failure::Stopped => "variant acquisition stopped before reading",
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
        crate::acquisition::Failure::Panicked,
        crate::acquisition::Failure::Stopped,
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut request = captured.take_variants_request(11, 0x1000, 1, false);
        // An independently refused drain must keep its original verdict too.
        request.changes.fail("finished design-change verdict");
        request.changes_finished = true;
        request.changes_ticks = 31;
        crate::test_support::with_allocations_denied(|| {
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
        Some(crate::acquisition::Failure::Panicked),
        Some(crate::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&capacity);
        captured.governance_report = Some(governance::CaptureReport::default());
        let bytes = captured.allocated_bytes();
        let governance_bytes = captured.governance.allocated_bytes();
        let countries =
            Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
        let request = captured.take_governance_request(
            &countries,
            11,
            0x1000,
            1,
            failure.is_none(),
            Box::new(crate::slicing::SliceFingerprints::new()),
            Box::new(crate::slicing::SliceFingerprints::new()),
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
        assert!(Arc::ptr_eq(&request.countries, &countries));
        assert_eq!(Arc::strong_count(&countries), 2);
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
        // Gated borrows the rest of construction while the allocating
        // building buffers are still on their worker. Restore that pack
        // before restoring the worker-only families.
        let gated = captured.take_gated_request(
            11,
            0x1000,
            1,
            context.force_full,
            Box::new(crate::slicing::SliceFingerprints::new()),
            None,
        );
        captured.restore_gated_request(gated, false);
        let _technologies =
            captured.restore_governance_request(request, failure.map_or(Ok(report), Err));
        // The hourly drain leaves even off the registry's slot.
        let variants = captured.take_variants_request(11, 0x1000, 1, context.force_full);
        captured.restore_variants_request(
            variants,
            Ok(if context.force_full {
                variant_ticks
            } else {
                [0; production::N]
            }),
        );
        assert_eq!(Arc::strong_count(&countries), 1);
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
            // ResearchSlots is off its slot; the sliced technology tables
            // are read every hour, so the failure is theirs.
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
        // A missing worker result also cannot trigger an allocating walk
        // on the caller, even on a full-refresh hour.
        captured.capture_tables_all(&memory, 11, 0x1000, 1, true);
        assert_eq!(memory.governance.get(), 0);
        assert_eq!(
            captured.governance.outcomes()[2],
            Outcome::Unavailable {
                reason: "governance acquisition not supplied"
            }
        );
        // A direct control capture proves that the spy detects Governance work.
        captured.governance.capture_owned(
            &memory,
            governance::CaptureContext {
                force_full: true,
                ..context
            },
            &mut crate::slicing::SliceFingerprints::new(),
            &mut crate::slicing::SliceFingerprints::new(),
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
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("completed residual capture and delegated raw jobs must not read again");
        }
    }
    let mut capacity = Capacities::default();
    capacity.intel.raise_for_countries(1);
    for failure in [
        None,
        Some(crate::acquisition::Failure::Panicked),
        Some(crate::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&capacity);
        captured.intel.capitulation_raw_mode = true;
        captured.intel.relations_raw_mode = true;
        let bytes = captured.allocated_bytes();
        let family_bytes = captured.intel.allocated_bytes();
        let countries =
            Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
        let context = intel::CaptureContext {
            base: 11,
            game_state: 0x1000,
            hour: 1,
            force_full: true,
        };
        let request = captured.take_intel_residual_request(&countries, context, None, None);
        assert_eq!(captured.intel.allocated_bytes(), 0);
        assert_eq!(captured.allocated_bytes() + family_bytes, bytes);
        assert_eq!(request.captured.allocated_bytes(), family_bytes);
        assert!(Arc::ptr_eq(&request.countries, &countries));
        let original = intel::CaptureReport {
            ticks: std::array::from_fn(|index| {
                if [0, 13, 14].contains(&index) {
                    0
                } else {
                    index as u64 + 101
                }
            }),
        };
        let (flags, variables) =
            captured.restore_intel_residual_request(request, failure.map_or(Ok(original), Err));
        assert!(flags.is_none() && variables.is_none());
        assert_eq!(Arc::strong_count(&countries), 1);
        assert_eq!(captured.allocated_bytes(), bytes);
        let outcomes = captured.intel.outcomes();
        let report = captured.acquire_intel(&NoReads, context);
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let walked = |ships: u64| {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.ship_models.begin_sources();
        for id in 1..=ships {
            captured.ship_models.visit_source_growing(
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
    // The unsplit walk's verdict, straight from the family capture.
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
        // The family lends its own capture as part 0.
        assert_eq!(captured.ship_models.allocated_bytes(), 0);
        for (index, request) in requests.into_iter().flatten().enumerate() {
            assert_eq!(request.part, index);
            assert_eq!(
                request.context.base, 11,
                "each detail part retains the original census request base"
            );
            let mut request = request;
            let report = if stop == Some(index) {
                Err(crate::acquisition::Failure::Stopped)
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
        // Every part's cost adds up; the verdict is the unsplit walk's.
        let ticks = (1..=parts as u64).sum::<u64>();
        assert_eq!(captured.ship_models_report.unwrap().ticks, [ticks]);
        assert_eq!(captured.ship_models.outcomes(), expected, "{ships} ships");
    }
    // A failed census lends no part.
    let mut captured = walked(3);
    let request = captured.take_ship_models_request(&countries, 11, 0x1000, ShipCensus::vacant());
    let _census =
        captured.restore_ship_models_request(request, Err(crate::acquisition::Failure::Panicked));
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    captured.ship_models.begin_sources();
    captured
        .ship_models
        .finish_sources(Outcome::Rows { len: 0 });
    let request = captured.take_ship_models_request(&countries, 11, 0x1000, ShipCensus::vacant());
    let family_bytes = request.captured.allocated_bytes();
    assert_eq!(captured.ship_models.allocated_bytes(), 0);
    let census =
        captured.restore_ship_models_request(request, Err(crate::acquisition::Failure::Panicked));
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
    assert_eq!(Arc::strong_count(&countries), 1);
}

#[test]
fn hourly_family_handoffs_restore_independently_and_consume_terminal_reports_once() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("a terminal hourly family must not read again");
        }
    }
    #[derive(Default)]
    struct Reads(std::cell::Cell<usize>);
    impl Memory for Reads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            self.0.set(self.0.get() + 1);
            false
        }
    }
    use crate::acquisition::Failure::{Panicked, Stopped};
    let mut capacity = Capacities::default();
    capacity.supply_trains.raise_for_countries(1);
    let mut captured = Captured::with_capacities(&capacity);
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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
        let census = Arc::new(crate::division_census::UnitInputs::empty_rows());
        let province_order = captured.take_province_unit_order_request(
            11,
            0x1000,
            Some(Arc::clone(&census)),
            Some(province_unit_order::owned_tests::held_store()),
        );
        assert_eq!(Arc::strong_count(&census), 2);
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
        assert!(supply.iter().all(|r| Arc::ptr_eq(&r.countries, &countries)));
        assert_eq!(Arc::strong_count(&countries), 1 + supply_trains::PARTS);
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
        assert_eq!(Arc::strong_count(&countries), 1);
        assert!(captured.supply_trains_report.is_some());
        assert!(captured.character_portraits_report.is_none());
        for request in portraits {
            captured.restore_character_portraits_request(
                request,
                portrait_failure
                    .map_or(Ok(character_portraits::CaptureReport { ticks: [107] }), Err),
            );
        }
        assert_eq!(Arc::strong_count(&countries), 1);
        // A failed job forgets the held walk its take consumed marks for.
        let store = captured
            .restore_province_unit_order_request(
                province_order,
                province_order_failure.map_or(Ok(province_order_report), Err),
            )
            .unwrap();
        assert_eq!(store.holds(), province_order_failure.is_none());
        // The join releases the census lease, so the next hour can reuse it.
        assert_eq!(Arc::strong_count(&census), 1);
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
        let supply = captured.acquire_supply_trains(&NoReads, supply_context);
        let portraits = captured.acquire_character_portraits(&NoReads, portrait_context);
        let province_order = captured.acquire_province_unit_order(&NoReads, province_order_context);
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
        // Route the identical reports through the full ordinary stage.
        // Unrelated families may legitimately read the same native roots.
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
        // With no pending report, each direct table read fails once at
        // its root: every supply part, the trains, portraits and the order.
        let direct = Reads::default();
        captured.acquire_supply_trains(&direct, supply_context);
        captured.acquire_character_portraits(&direct, portrait_context);
        captured.acquire_province_unit_order(&direct, province_order_context);
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
    let inputs = Arc::new(UnitInputs::fixture(
        vec![carmy::DivisionSeed {
            pointer: 0x1234,
            ..Default::default()
        }],
        Outcome::Truncated {
            len: 1,
            unread_rows: 7,
        },
    ));
    captured.install_unit_inputs(Some(Arc::clone(&inputs)));
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
    let failed = Arc::new(UnitInputs::fixture(
        Vec::new(),
        Outcome::Unavailable {
            reason: "current census failed",
        },
    ));
    captured.install_unit_inputs(Some(Arc::clone(&failed)));
    assert_eq!(Arc::strong_count(&inputs), 1);
    assert!(unit_sources(&captured.unit_inputs).rows.is_empty());
    assert_eq!(
        unit_sources(&captured.unit_inputs).outcome,
        Outcome::Unavailable {
            reason: "current census failed"
        },
    );
    captured.drop_and_reset();
    assert_eq!(Arc::strong_count(&failed), 1);
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
fn metadata_partitions_every_extended_table_once_and_preserves_cost_membership() {
    // BorderConflicts is the Provinces walk's second output, set by the core
    // spool. Retired tables are not recorded, so no pack produces them.
    let current = replay_data::format::FormatVersion::CURRENT.version();
    let retired = TableId::ALL
        .into_iter()
        .filter(|table| !table.last_change().present(current))
        .count();
    // Four inputs, five hover-only, oracle-only and wargoal tables and the nine
    // selected-transport split tables, plus retired NavalMineLevels.
    assert_eq!(retired, 19);
    assert_eq!(N, replay_data::rows::TABLE_COUNT - 21 - retired);
    let actual: BTreeSet<_> = TABLES.into_iter().collect();
    assert_eq!(actual.len(), N);
    let expected: BTreeSet<_> = TableId::ALL
        .into_iter()
        .filter(|table| {
            table.index() >= 20
                && *table != TableId::BorderConflicts
                && table.last_change().present(current)
        })
        .collect();
    assert_eq!(actual, expected);
    for (name, anchor, members) in read_groups() {
        let members = members.to_vec();
        assert!(!name.is_empty());
        assert!(members.contains(&anchor));
        if anchor == TableId::Divisions {
            assert_eq!(
                members,
                std::iter::once(TableId::Divisions)
                    .chain(DIVISION_SOURCE_TABLES)
                    .collect::<Vec<_>>()
            );
        } else if anchor == TableId::ArmyOrders {
            assert_eq!(
                members,
                vec![
                    TableId::ArmyOrders,
                    TableId::ArmyOrderPlans,
                    TableId::OrdersGroupPlans,
                    TableId::Fronts,
                    TableId::InvasionPreparations
                ]
            );
        } else if anchor == TableId::Countries {
            assert_eq!(
                members,
                vec![
                    TableId::Countries,
                    TableId::CountryAppearance,
                    TableId::SupplyNodes,
                    TableId::TrainInventory
                ]
            );
        } else if anchor == TableId::Fleets {
            assert_eq!(
                members,
                vec![
                    TableId::Fleets,
                    TableId::NavalRepairCountries,
                    TableId::NavalRepairFleets
                ]
            );
        } else if anchor == TableId::TaskForces {
            assert_eq!(
                members,
                vec![
                    TableId::TaskForces,
                    TableId::NavalRepairTaskForces,
                    TableId::NavalTaskForceComposition,
                    TableId::NavalRepairChildren
                ]
            );
        } else if anchor == TableId::Ships {
            assert_eq!(
                members,
                vec![
                    TableId::Ships,
                    TableId::ShipModels,
                    TableId::NavalRefitShips,
                    TableId::NavalRefitProgress
                ]
            );
        } else if anchor == TableId::States {
            assert_eq!(members, vec![TableId::States, TableId::StateFlags]);
        } else {
            assert!(members.iter().all(|table| actual.contains(table)));
        }
    }
    let groups: Vec<_> = read_groups().collect();
    for table in DIVISION_SOURCE_TABLES {
        assert_eq!(
            groups
                .iter()
                .filter(|(_, _, members)| members.contains(&table))
                .count(),
            1,
            "shared division cost counted more than once for {table:?}"
        );
    }
}
#[test]
fn faction_provider_inventory_matches_spooling_and_costs_exactly_once() {
    for table in super::FACTION_PROVIDER_TABLES {
        assert!(
            !super::PACK_TABLES.contains(&table),
            "independent ticks must not be overwritten for {table:?}"
        );
        assert_eq!(
            TABLES.iter().filter(|&&id| id == table).count(),
            1,
            "{table:?}"
        );
        assert_eq!(
            read_groups()
                .filter(|(_, _, members)| members.contains(&table))
                .count(),
            1,
            "{table:?}"
        );
    }
}
#[test]
fn naval_management_outputs_have_one_inventory_and_the_existing_census_costs() {
    let groups: Vec<_> = read_groups().collect();
    for (anchor, outputs) in [
        (
            TableId::Fleets,
            &[TableId::NavalRepairCountries, TableId::NavalRepairFleets][..],
        ),
        (
            TableId::TaskForces,
            &[
                TableId::NavalRepairTaskForces,
                TableId::NavalTaskForceComposition,
                TableId::NavalRepairChildren,
            ][..],
        ),
        (
            TableId::Ships,
            &[TableId::NavalRefitShips, TableId::NavalRefitProgress][..],
        ),
        (
            TableId::NavalBases,
            &collector_naval_management::bases::TABLES[..],
        ),
    ] {
        for &table in outputs {
            assert_eq!(
                TABLES.iter().filter(|&&id| id == table).count(),
                1,
                "{table:?}"
            );
            let owners: Vec<_> = groups
                .iter()
                .filter(|(_, _, members)| members.contains(&table))
                .collect();
            assert_eq!(owners.len(), 1, "{table:?}");
            assert_eq!(owners[0].1, anchor, "{table:?}");
        }
    }
    assert!(
        !PACK_NAMES.contains(&"naval_management"),
        "census outputs must not create another capture pack"
    );
}
#[test]
fn division_sources_preserve_census_identity_and_reject_an_incomplete_parent() {
    let mut sources = Divisions::new(1);
    let source = DivisionSource {
        pointer: 0x1234,
        division: (1_u64 << 48) | 14,
        country: 17,
        template: u64::MAX,
    };
    sources.visit_source(source);
    sources.finish(Outcome::Rows { len: 1 });
    assert_eq!(sources.sources().outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        sources.sources().rows[0],
        DivisionSource {
            pointer: 0x1234,
            division: (1_u64 << 48) | 14,
            template: u64::MAX,
            country: 17
        }
    );
    sources.visit_source(DivisionSource {
        pointer: 0x5678,
        ..source
    });
    sources.finish(Outcome::Truncated {
        len: 2,
        unread_rows: 7,
    });
    assert_eq!(
        sources.sources().outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 8
        }
    );
    assert_eq!(sources.required, 9);
    let mut capacity = 1;
    sources.grow(&mut capacity);
    assert!(capacity >= 9);
    assert_eq!(sources.sources().rows[0], source);
    sources.reset();
    sources.finish(Outcome::Rows { len: 1 });
    assert!(matches!(
        sources.sources().outcome,
        Outcome::Unavailable { .. }
    ));
}
#[test]
fn the_slow_job_stores_travel_with_it_and_always_return() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    for completed in [true, false] {
        // Owner 9 (in hour 9's slice) and owner 7 were seen, unchanged,
        // before this hour.
        let mut available = Box::new(crate::slicing::SliceFingerprints::new());
        let mut seen = crate::slicing::SliceSelection::for_test(0, true, &[]);
        seen.scan(&mut available, 7, Some(0));
        seen.scan(&mut available, 9, Some(0));
        available.finish_scan();
        let stores = SlowStores {
            doctrine_bytes: 0,
            doctrines: None,
            available: Some(available),
            advisors: None,
            local_subunits: None,
        };
        // The callback starts each frame's selections at its hour.
        captured.prepare_hour(9, false);
        let request = captured.take_slow_request(&countries, 11, 0x1000, 9, false, stores);
        assert!(request.stores.available.is_some());
        // Nothing was read: the unpublished reads are invalidated or the
        // failed job's stores forgotten, and the stores come back either way.
        let mut stores = captured.restore_slow_request(request, completed);
        assert_eq!(captured.hired_advisors.available_selection().hour, 9);
        // The unpublished slice is read again at the next hour, outside its
        // own slice. After a completed job an owner of another slice still
        // waits for its slice; a failed job forgets the store (its scan may
        // have stopped part-way), so every country is read again at once.
        let available = stores.available.as_mut().expect("the store returns");
        let mut next = crate::slicing::SliceSelection::for_test(10, false, &[]);
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let (hour, force_full) = (9, false);
    for completed in [true, false] {
        // Owner 9 (in hour 9's slice) and owner 7 were seen, unchanged,
        // before this hour.
        let mut store = Box::new(crate::slicing::SliceFingerprints::new());
        let mut seen = crate::slicing::SliceSelection::for_test(0, true, &[]);
        seen.scan(&mut store, 7, Some(0));
        seen.scan(&mut store, 9, Some(0));
        store.finish_scan();
        captured.prepare_hour(hour, force_full);
        // The Slow job no longer carries the stability pack: its failure
        // leaves the stability tables to their own job, and theirs leaves
        // Slow's tables as Slow left them.
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
        // Nothing was read: the unpublished read is invalidated or the
        // failed job's store forgotten, and the store comes back either way.
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
        // The unpublished slice is read again at the next hour; a failed job
        // forgets the store, so every country is read again at once.
        let mut next = crate::slicing::SliceSelection::for_test(hour + 1, false, &[]);
        next.scan(&mut store, 9, Some(0));
        next.scan(&mut store, 7, Some(0));
        store.finish_scan();
        assert!(next.contains(9), "completed {completed}");
        assert_eq!(next.contains(7), !completed, "completed {completed}");
    }
    // Without a store the job reads only the hour's slice and returns none.
    let request = captured.take_stability_request(&countries, 11, 0x1000, 9, false, None);
    assert!(captured.restore_stability_request(request, true).is_none());
}

#[test]
fn the_war_overview_is_its_own_job_and_fails_apart_from_slow() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let (hour, force_full) = (9, false);
    for completed in [true, false] {
        captured.prepare_hour(hour, force_full);
        // The Slow job no longer carries the war overview: its failure
        // leaves the overview's tables to their own job, and theirs leaves
        // Slow's tables as Slow left them.
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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
fn relations_pilot_handoff_reaches_the_intel_pack() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.relations_pilot().relations_raw_mode = true;
    assert!(captured.intel.relations_raw_mode);
    captured.relations_pilot().relations_raw_mode = false;
    assert!(!captured.intel.relations_raw_mode);
}

#[test]
fn characters_pilot_handoff_reaches_the_characters_pack() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.characters_pilot().raw_active = [true; characters::N];
    assert!(captured.characters.raw_active.iter().any(|&active| active));
    captured.characters_pilot().raw_active = [false; characters::N];
    assert!(!captured.characters.raw_active.iter().any(|&active| active));
}

#[test]
fn projects_pilot_handoff_reaches_the_projects_pack() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.projects_pilot().raw_mode = true;
    assert!(captured.projects.raw_mode);
    captured.projects_pilot().raw_mode = false;
    assert!(!captured.projects.raw_mode);
}

#[test]
fn lexical_pilot_handoff_reaches_the_lexical_pack() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.lexical_pilot().raw_mode = true;
    assert!(captured.lexical.raw_mode);
    captured.lexical_pilot().raw_mode = false;
    assert!(!captured.lexical.raw_mode);
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
                assert_eq!(view, crate::spooling::unavailable(reason));
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

#[test]
fn construction_rest_pilot_handoff_reaches_the_construction_pack() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.construction_rest_pilot().rest_raw_mode = true;
    assert!(captured.construction.rest_raw_mode);
    captured.construction_rest_pilot().rest_raw_mode = false;
    assert!(!captured.construction.rest_raw_mode);
}

#[test]
fn economy_rest_pilot_handoff_reaches_the_economy_pack() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.economy_rest_pilot().rest_raw_mode = true;
    assert!(captured.economy.rest_raw_mode);
    captured.economy_rest_pilot().rest_raw_mode = false;
    assert!(!captured.economy.rest_raw_mode);
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
            engine_native::memory::SliceMemory {
                base: air::wing_test_kit::BASE,
                bytes,
            }
            .copy(address, destination)
        })
    }
}

#[test]
fn source_adapter_routes_each_staged_consumer_without_replacing_faults() {
    let mut capacity = Capacities::default();
    capacity.air.rows[0] = 2;
    capacity.air.rows[2] = 1;
    capacity.air.wings = 1;
    capacity.naval_history.rows[9] = 1;
    capacity.naval_history.wing_sources = 1;
    let mut actual = Captured::with_capacities(&capacity);
    let mut expected = Captured::with_capacities(&capacity);
    let row = cair_wing::Row {
        id: 0x1234_5678_0000_0009,
        ..Default::default()
    };
    // All four raw-admission combinations retain the direct consumers'
    // read order, faults and nonempty rows from the existing typed fixture.
    for readable in [false, true] {
        for air_raw in [false, true] {
            for naval_raw in [false, true] {
                actual.drop_and_reset();
                expected.drop_and_reset();
                let memory = SourceReads {
                    bytes: readable.then(air::wing_test_kit::fixture),
                    ..Default::default()
                };
                let reference_memory = SourceReads {
                    bytes: readable.then(air::wing_test_kit::fixture),
                    ..Default::default()
                };
                {
                    let mut sources = actual.sources(&memory, 0, air_raw, naval_raw);
                    sources.begin_air();
                    sources.air(air::wing_test_kit::WING, 0, &row);
                    sources.finish_air(Outcome::Rows { len: 1 });
                }

                // Independent direct consumers define the reference rows
                // and ordered native reads for this source admission.
                if !air_raw {
                    expected.air.begin_wings();
                    expected.air.visit_wing(
                        &reference_memory,
                        0,
                        air::WingSource {
                            pointer: air::wing_test_kit::WING,
                            pool: air::wing_test_kit::POOL,
                            row: &row,
                        },
                    );
                    expected.air_root = Some(Outcome::Rows { len: 1 });
                }
                if !naval_raw {
                    expected.naval_history.visit_wing(
                        &reference_memory,
                        air::wing_test_kit::WING,
                        &row,
                    );
                    expected
                        .naval_history
                        .finish_wings(Outcome::Rows { len: 1 });
                }
                assert_eq!(*memory.reads.borrow(), *reference_memory.reads.borrow());
                assert_eq!(actual.air_root, expected.air_root);
                assert_eq!(actual.air.tables(), expected.air.tables());
                assert_eq!(
                    actual.naval_history.tables(),
                    expected.naval_history.tables()
                );
                if readable && !air_raw {
                    assert_eq!(actual.air.outcomes()[0], Outcome::Rows { len: 2 });
                    assert_eq!(actual.air.outcomes()[2], Outcome::Rows { len: 1 });
                }
                if air_raw && naval_raw {
                    assert!(memory.reads.borrow().is_empty());
                    assert_eq!(actual.air_root, None);
                }
            }
        }
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    for rejected in [true, false] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut rows = Vec::<engine_native::ccountry::Row>::with_capacity(1);
        let mut links = Vec::<engine_native::ccountry::LinkRow>::with_capacity(1);
        let row_storage = rows.as_ptr();
        let link_storage = links.as_ptr();
        // A worker may already hold a failed original prefix when its
        // terminal authority returns the owners. That result must survive.
        captured
            .country_appearance
            .raw
            .reserve(engine_native::raw_country_observations::Capacity {
                links: 8,
                ..Default::default()
            })
            .unwrap();
        captured.country_appearance.raw.begin(base, state, hour);
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
                captured
                    .division_templates
                    .capture(&memory, base, state, hour, true),
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
        captured
            .division_templates
            .capture(&memory, base, state, hour + 1, true);
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
    use crate::slicing::{SliceFingerprints, SliceSelection};
    let hour = 9;
    let owner = (1..=32)
        .find(|&owner| !replay_data::cadence::slice_active(owner, hour, false))
        .unwrap();
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    for refused_before_completion in [false, true] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let mut fingerprints = Box::new(SliceFingerprints::new());
        let mut prior = SliceSelection::new(true);
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
        let mut next = SliceSelection::new(true);
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

/// Characters, intel and character portraits convert on the helpers, the
/// first two with the spool caches lent to their lane; the ETA pair shares a
/// lane, and packs needing other spool caches stay on the spool thread.
#[test]
fn cached_and_unsynchronised_packs_convert_on_their_helper_lanes() {
    let lane = |name: &str| {
        super::HELPER_LANES[super::PACK_NAMES
            .iter()
            .position(|pack| *pack == name)
            .unwrap()]
    };
    assert_eq!(lane("characters"), Some(1));
    assert_eq!(lane("character_portraits"), Some(1));
    assert_eq!(lane("intel"), Some(0));
    assert_eq!(lane("division_presentation"), Some(0));
    assert_eq!(lane("country_appearance"), Some(1));
    assert_eq!(lane("lexical"), None);
    assert_eq!(lane("division_templates"), None);
}

/// RaidAirScores and the shared rail tables come from their own first-burst
/// job: a Late failure keeps that job's outcomes, and only the job's own
/// failure fails them, rereading every score pair at the next hour.
#[test]
fn scores_and_rails_keep_their_own_job_outcome_through_a_late_failure() {
    use engine_native::memory::SliceMemory;
    let tables = [
        TableId::RaidAirScores,
        TableId::RailCooldowns,
        TableId::RailEdges,
    ];
    for completed in [true, false] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries =
            Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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
        // An empty world: each walk fails with its own reason.
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
        // Another TableId the Late job owns still fails with it.
        assert_eq!(
            after[TableId::AirReferenceWitnesses.index()],
            Outcome::Unavailable {
                reason: "late acquisition failed"
            }
        );
    }
}

/// ArmyIconInputs come from their own job right after the division sources
/// join: a Late failure keeps that job's outcome, and only the job's own
/// failure fails the rows.
#[test]
fn army_icons_keep_their_own_job_outcome_through_a_late_failure() {
    use engine_native::memory::SliceMemory;
    for completed in [true, false] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let countries =
            Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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

/// The daily production tables, split from Slow: a failed job on their slot
/// refuses them as a failed Slow did, a second assembly neither reruns nor
/// changes them, and off their slot they are skipped without a read or an
/// allocation, Slow's refusal keeping them skipped.
#[test]
fn the_daily_production_tables_keep_slows_refusals_and_skip_off_their_slot() {
    use engine_native::memory::CountedMemory;
    use replay_data::cadence::SKIPPED_REASON;
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            false
        }
    }
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
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
    // A failed job on its slot, then a second assembly.
    let mut captured = Captured::with_capacities(&Capacities::default());
    let request = captured.take_production_daily_request(&countries, 11, 0x1000, SLOT, false);
    captured.restore_production_daily_request(request, false);
    captured.capture_tables(&NoRead, 11, 0x1000, SLOT, false);
    refused(&captured, "slow acquisition failed");
    captured.capture_tables(&NoRead, 11, 0x1000, SLOT, false);
    refused(&captured, "slow acquisition failed");
    // Off the slot, twice, with and without Slow's refusal.
    let mut captured = Captured::with_capacities(&Capacities::default());
    let memory = CountedMemory::new(NoRead);
    for refusal in [None, Some("slow acquisition failed")] {
        captured.slow_refusal = refusal;
        let before = (
            captured.industrial_organisations.allocated_bytes(),
            captured.production_modifiers.allocated_bytes(),
        );
        for _ in 0..2 {
            let reads = crate::read_accounting::Capture::new();
            captured.capture_tables(&memory, 11, 0x1000, SLOT + 1, false);
            let reads = reads.finish();
            for table in industrial_organisations::TABLES
                .into_iter()
                .chain(production_modifiers::TABLES)
            {
                assert_eq!(
                    reads[table.index()],
                    engine_native::memory::ReadCounts::ZERO,
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

/// Every combination of completed, failed and missing Slow, MIO and
/// ProductionDaily jobs, on the slot and on a full hour, through the real
/// take, restore and non-test missing-job paths: Slow's failure or absence
/// refuses the daily production tables with Slow's reason though their own
/// job completed; their own failure or absence refuses them with the reason
/// a failed or missing Slow recorded; with all complete they publish their
/// own capture, as on a frame reused after a refused hour.
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let outcomes = |captured: &Captured| -> Vec<Outcome> {
        captured
            .industrial_organisations
            .outcomes()
            .into_iter()
            .chain(captured.production_modifiers.outcomes())
            .collect()
    };
    // One hour: Slow's and both daily jobs' joins, then the assembly twice.
    let hour_of = |captured: &mut Captured, slow: Job, mio: Job, daily: Job, hour: i32, full: bool| {
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
            let mut request =
                captured.take_industrial_organisations_request(&countries, 11, 0x1000, hour, full);
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
            // Return MIO's storage even when range zero was never supplied.
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
                    // The frame reused after the reset that starts every hour:
                    // a healthy hour publishes its own capture.
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

/// A split production modifier capture through its real lifecycle (the
/// split prepared, its parts loaned and the first loaned with the
/// ProductionDaily job, each returned, then the assembly): its tables are
/// the whole capture's at 5,000 countries with unequal lines and an idle
/// range; a failed or missing part refuses both tables with Slow's failure
/// reason, the first failure in slot order winning; a first range without a
/// country with lines takes the next range's defines; a second assembly
/// changes nothing; and a reset after a preparation or a failed hour leaves
/// no split behind.
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
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(0).unwrap());
    let context = engine_production::production_modifiers::Context {
        start: 17,
        cap: 93,
        gain: -5,
        ..Default::default()
    };
    // 5,000 countries, every seventh with three lines, 2,000..2,600 idle.
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
    // One slot hour: the split, its loans and returns, and two assemblies.
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
                crate::spooling::unavailable(reason),
                crate::spooling::unavailable(reason),
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
    // Two parts fail with their own reasons: the earlier one's is published.
    let mut parts = split(&bounds);
    parts[1] = Some(test_support::failed("second range's failure"));
    parts[3] = Some(test_support::failed("fourth range's failure"));
    let mut captured = Captured::with_capacities(&Capacities::default());
    assert_eq!(
        hour(&mut captured, parts, all, true),
        refused("second range's failure")
    );
    // A first range without a country with lines: the next range's defines.
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
    // A failed ProductionDaily job refuses them, and a reset leaves no split.
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
    // A reset after a preparation: the next slot hour's whole capture.
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
    // One slot hour split at `starts`, every part complete: the daily job
    // either captured and returned (`captured` true) or returned failed
    // without running, as a refused or stopped job is.
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
    // A daily job refused before its capture: the failure, not a reset,
    // clears the pending split, so the next slot hour on the same frame is
    // the whole capture.
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
    // And the off-slot hour after it skips them without a split.
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
    // Four parts, then two, then one, on the same frame: each split's own
    // ranges and the whole capture's tables.
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
