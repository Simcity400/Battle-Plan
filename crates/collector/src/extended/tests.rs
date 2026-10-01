use super::*;
use std::collections::BTreeSet;

#[test]
fn vacant_spares_restore_storage_and_verdicts_after_success_and_failure() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let mut orders = Orders::vacant();
    orders.geometry = vec![11, 13, 17];
    orders.geometry_len = 3;
    orders.root_outcome = Outcome::Rows { len: 3 };
    let order_storage = orders.geometry.as_ptr();
    let division_storage = captured.divisions.first.as_ptr();
    let box_bytes = [
        size_of::<OrdinaryPacks>(),
        size_of::<SlowPacks>(),
        size_of::<LatePacks>(),
        size_of::<GatedPacks>(),
        size_of::<air::Captured>(),
        size_of::<RaidStatePacks>(),
        size_of::<RaidLaunchPacks>(),
        size_of::<RaidTargetPacks>(),
    ];
    let assert_vacant = |captured: &Captured| {
        // The boxes remain reusable; none of their parked packs owns a
        // second row, arena, hash or source allocation.
        assert_eq!(
            [
                captured.ordinary_spare.as_ref().unwrap().allocated_bytes(),
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
            ],
            box_bytes
        );
        assert_eq!(captured.spare_bytes(), box_bytes.iter().sum::<usize>());
        assert_eq!(
            captured
                .allocated_by_pack()
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
        let request = captured.take_ordinary_request(&countries, 11, 0x1000);
        assert_eq!(std::ptr::from_ref(&*request.packs), ordinary_box);
        assert_eq!(
            captured.allocated_bytes() + request.allocated_bytes(),
            bytes
        );
        removed[0] = request.allocated_bytes() - box_bytes[0];
        captured.restore_ordinary_request(request, (!completed).then_some("fixture failure"));
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

        captured.fog.radar_raw_mode = true;
        captured.fog.intel_raw_mode = true;
        captured.fog.registrations_raw_mode = true;
        captured.projects.raw_mode = true;
        captured.construction.rest_raw_mode = true;
        let gated_box = std::ptr::from_ref(&**captured.gated_spare.as_ref().unwrap());
        let request = captured.take_gated_request(11, 0x1000, 9, false);
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
        let request = captured.take_air_missions_request(&countries, 11, 0x1000);
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
            captured.take_late_request(&mut orders, true, 11, 0x1000, 9, false, Box::default());
        assert_eq!(std::ptr::from_ref(&*request.packs), late_box);
        assert_eq!(
            captured.allocated_bytes() + orders.allocated_bytes() + request.packs.allocated_bytes(),
            bytes + order_bytes
        );
        // These two auxiliary placeholders were already vacant before
        // this change; only the four late table packs lose duplicates.
        removed[2] = request.packs.allocated_bytes()
            - box_bytes[2]
            - request.packs.divisions.allocated_bytes()
            - request.packs.orders.allocated_bytes();
        assert!(request.packs.air.stats_raw_mode);
        assert!(request.packs.naval_history.raw_mode);
        assert_eq!(request.packs.air_root, Some(Outcome::Rows { len: 7 }));
        assert_eq!(request.packs.orders.geometry.as_ptr(), order_storage);
        assert_eq!(request.packs.divisions.first.as_ptr(), division_storage);
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
            captured.take_late_request(&mut orders, true, 11, 0x1000, 9, false, Box::default());
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
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
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
    let report = division_presentation::CaptureReport { ticks: [89, 97, 5] };
    captured.install_unit_inputs(Some(Arc::clone(&inputs)));
    captured.presentation_report = Some(report);
    let request = captured.take_presentation_request(&countries, 11, 0x1000, 0, true, None);
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

#[test]
fn eta_input_handoff_restores_storage_leases_and_a_terminal_failure() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let inputs = Arc::new(UnitInputs::fixture(
        vec![carmy::DivisionSeed {
            pointer: 0x1234,
            handle: Some(7),
            ..Default::default()
        }],
        Outcome::Rows { len: 1 },
    ));
    let bytes = captured.allocated_bytes();
    let owned = captured.division_eta_inputs.allocated_bytes();
    // The placeholder keeps the packet's fixed route and transport arenas.
    let vacant = division_eta_inputs::RangeCaptured::vacant().allocated_bytes();
    assert!(owned > vacant);
    for failure in [
        crate::acquisition::Failure::Panicked,
        crate::acquisition::Failure::Stopped,
    ] {
        captured.install_unit_inputs(Some(Arc::clone(&inputs)));
        // Stable worker ownership does not depend on census size.
        assert!(captured.plan_eta_input_ranges() >= 1);
        let request =
            captured.take_eta_inputs_request(0, &countries, 11, 0x1000, 7, false, Box::default());
        let lent = captured.division_eta_inputs.allocated_bytes();
        assert!(lent < owned);
        assert_eq!(captured.allocated_bytes() + owned, bytes + lent);
        assert_eq!(request.context.span, (0, 1));
        assert_eq!(
            (request.context.base, request.context.game_state),
            (11, 0x1000)
        );
        assert!(Arc::ptr_eq(&request.countries, &countries));
        assert!(Arc::ptr_eq(request.inputs.as_ref().unwrap(), &inputs));
        assert_eq!(Arc::strong_count(&countries), 2);
        assert_eq!(Arc::strong_count(&inputs), 3);
        captured.restore_eta_inputs_request(request, Err(failure));
        assert_eq!(captured.allocated_bytes(), bytes);
        assert_eq!(captured.division_eta_inputs.allocated_bytes(), owned);
        assert_eq!(Arc::strong_count(&countries), 1);
        assert_eq!(Arc::strong_count(&inputs), 2);
        // The terminal failure is consumed once, without a live retry.
        let report = captured.acquire_eta_inputs();
        assert_eq!(report.ticks, [0]);
        assert!(captured.eta_inputs_report.is_none());
        assert_eq!(
            captured.division_eta_inputs.outcomes(),
            [Outcome::Unavailable {
                reason: match failure {
                    crate::acquisition::Failure::Panicked =>
                        "division ETA input acquisition panicked",
                    crate::acquisition::Failure::Stopped =>
                        "division ETA input acquisition stopped before reading",
                },
            }]
        );
        captured.drop_and_reset();
        assert!(captured.eta_inputs_report.is_none());
        assert_eq!(Arc::strong_count(&inputs), 1);
    }
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
            if address == 0x1000 + engine::cgame_state::PROVINCES {
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
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let inputs = Arc::new(UnitInputs::empty_rows());
    let bytes = captured.allocated_bytes();
    let report = division_presentation::CaptureReport {
        ticks: [113, 227, 5],
    };
    for failure in [
        None,
        Some(crate::acquisition::Failure::Panicked),
        Some(crate::acquisition::Failure::Stopped),
    ] {
        captured.drop_and_reset();
        captured.install_unit_inputs(Some(Arc::clone(&inputs)));
        let request = captured.take_presentation_request(&countries, 11, 0x1000, 0, true, None);
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
        assert_eq!(captured.allocated_bytes(), bytes);
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
            None,
        );
        assert!(memory.presentation.get() > before);
    }
}

#[test]
fn failed_pack_jobs_are_terminal_without_live_retry_or_partial_publication() {
    use engine::memory::{CountedMemory, SliceMemory};
    use replay::frame::TableView;

    let slow = [
        map_names::TABLES.as_slice(),
        division_templates::TABLES.as_slice(),
        visibility::TABLES.as_slice(),
        faction_theatres::TABLES.as_slice(),
    ]
    .concat();
    let gated = [
        fog::TABLES.as_slice(),
        &construction::TABLES[2..],
        projects::TABLES.as_slice(),
    ]
    .concat();
    let late = [
        leader_portraits::TABLES.as_slice(),
        leader_paths::TABLES.as_slice(),
        air::TABLES.as_slice(),
        naval_history::TABLES.as_slice(),
        army_icon_inputs::TABLES.as_slice(),
    ]
    .concat();
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
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
        let mut request = captured.take_gated_request(11, 0x1000, hour, force_full);
        request.packs.ticks.fill(23);
        captured.restore_gated_request(request, false);
        let mut request = captured.take_late_request(
            &mut orders,
            false,
            11,
            0x1000,
            hour,
            force_full,
            Box::default(),
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
                &mut tables,
                None,
                &mut Default::default(),
                None,
                None,
                None,
                None,
            );
            for (ids, reason) in [
                (&slow, "slow acquisition failed"),
                (&gated, "gated acquisition failed"),
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
                        engine::memory::ReadCounts::ZERO,
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
    let request = captured.take_gated_request(11, 0x1000, 1, true);
    captured.restore_gated_request(request, false);
    let after = captured.outcomes();
    for id in [
        TableId::RadarCoverage,
        TableId::UnitIntel,
        TableId::AirBaseRegistrations,
    ]
    .into_iter()
    .chain(projects::TABLES[..4].iter().copied())
    .chain(construction::TABLES)
    {
        assert_eq!(after[id.index()], before[id.index()], "{id:?}");
    }
    assert!(
        captured.fog.radar_raw_mode
            && captured.fog.intel_raw_mode
            && captured.fog.registrations_raw_mode
    );
    assert!(captured.projects.raw_mode && captured.construction.rest_raw_mode);

    captured.air.fail_air_bases_capture(raw_failure);
    captured.air.fail_air_history_capture(raw_failure);
    captured.air.fail_air_stats_capture(raw_failure);
    captured.air.bases_raw_mode = true;
    captured.air.history_raw_mode = true;
    captured.air.stats_raw_mode = true;
    captured.naval_history.fail_naval_capture(raw_failure);
    captured.naval_history.fail_supremacy_capture(raw_failure);
    captured.naval_history.fail_sunk_capture(raw_failure);
    captured.naval_history.raw_mode = true;
    captured.naval_history.supremacy_raw_mode = true;
    captured.naval_history.sunk_raw_mode = true;
    let before = captured.outcomes();
    let request =
        captured.take_late_request(&mut orders, true, 11, 0x1000, 1, true, Box::default());
    captured.restore_late_request(request, false, &mut orders);
    let after = captured.outcomes();
    for id in [
        TableId::AirBases,
        TableId::AirRegionHistory,
        TableId::AirRegionDamage,
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
    assert!(
        captured.air.bases_raw_mode && captured.air.history_raw_mode && captured.air.stats_raw_mode
    );
    assert!(
        captured.naval_history.raw_mode
            && captured.naval_history.supremacy_raw_mode
            && captured.naval_history.sunk_raw_mode
    );
}

#[test]
fn a_refused_air_missions_job_fails_its_tables_without_allocating() {
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.prepare_hour(0, true);
    let before = captured.allocated_bytes();
    let mut request = captured.take_air_missions_request(&countries, 11, 0x1000);
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

#[test]
fn ordinary_terminal_results_cannot_retry_or_publish_partial_tables() {
    #[derive(Default)]
    struct Reads {
        combat: std::cell::Cell<usize>,
    }
    impl Memory for Reads {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            if address == 0x1000 + engine::cgame_state::COMBATS + 8 {
                self.combat.set(self.combat.get() + 1);
            }
            false
        }
    }
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    for reason in [
        "ordinary worker admission failed",
        "acquisition worker panicked",
        "acquisition worker stopped before completion",
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let before = captured.allocated_bytes();
        let mut request = captured.take_ordinary_request(&countries, 11, 0x1000);
        request.packs.ticks[TableId::LandCombatDetails.index()] = 71;
        let owned = request.allocated_bytes();
        if reason == "ordinary worker admission failed" {
            request.fail(reason);
            captured.restore_ordinary_request(request, None);
        } else {
            captured.restore_ordinary_request(request, Some(reason));
        }
        assert_eq!(captured.allocated_bytes(), before);
        let memory = Reads::default();
        let ticks = captured.capture_tables(&memory, 11, 0x1000, 1, false);
        assert_eq!(memory.combat.get(), 0, "terminal work must not retry");
        for &id in OrdinaryPacks::TABLES
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
        let mut request = captured.take_ordinary_request(&countries, 11, 0x1000);
        assert_eq!(request.allocated_bytes(), owned);
        request.packs.capture(&memory, 11, 0x1000);
        assert!(
            memory.combat.get() > 0,
            "control must exercise the native reader"
        );
        captured.restore_ordinary_request(request, None);
        assert_eq!(Arc::strong_count(&countries), 1);
    }
}

#[test]
fn unpublished_raid_slices_are_read_again_at_the_next_hour() {
    use crate::slicing::{SliceFingerprints, SliceSelection};
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let memory = engine::memory::SliceMemory {
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
fn unpublished_retreat_ban_reads_are_read_again_at_the_next_hour() {
    use crate::slicing::{SliceFingerprints, SliceSelection};
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let memory = engine::memory::SliceMemory {
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
    let unseen_next_hour = |store: &mut SliceFingerprints| {
        let mut next = SliceSelection::for_test(hour + 1, false, &[]);
        next.scan(store, 7, Some(1));
        next.contains(7)
    };
    let inputs = Arc::new(UnitInputs::empty_rows());
    let truncated = Outcome::Truncated {
        len: 0,
        unread_rows: 1,
    };
    // A completed job whose bans read was unavailable (an empty image),
    // truncated or published; then a failed job.
    for (bans, completed, again) in [
        (None, true, true),
        (Some(truncated), true, true),
        (Some(Outcome::Rows { len: 0 }), true, false),
        (None, false, true),
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.prepare_hour(hour, force_full);
        captured.install_unit_inputs(Some(Arc::clone(&inputs)));
        let mut store = Box::new(SliceFingerprints::new());
        seen(&mut store);
        let mut request = captured.take_presentation_request(
            &countries,
            11,
            0x1000,
            hour,
            force_full,
            Some(store),
        );
        let report = if completed {
            let context = request.context;
            let report = request.captured.capture_owned(
                &memory,
                context,
                unit_sources(&request.inputs),
                request.store.as_deref_mut(),
            );
            if let Some(outcome) = bans {
                request.captured.test_set_bans_outcome(outcome);
            }
            Ok(report)
        } else {
            Err(crate::acquisition::Failure::Panicked)
        };
        let mut store = captured
            .restore_presentation_request(request, report)
            .unwrap();
        assert_eq!(
            unseen_next_hour(&mut store),
            again,
            "bans {bans:?}, completed {completed}"
        );
    }
}

#[test]
fn research_handoff_maps_failures_scatters_ticks_and_survives_a_governance_failure() {
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let (hour, force_full) = (7, false);
    for failure in [
        None,
        Some(crate::acquisition::Failure::Panicked),
        Some(crate::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
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
    let slot = replay::cadence::refresh_slot(TableId::VariantDefinitions).unwrap() as i32;
    let mut captured = Captured::with_capacities(&Capacities::default());
    let bytes = captured.allocated_bytes();
    let memory = engine::memory::CountedMemory::new(Reads(std::cell::Cell::new(0)));
    assert!(
        captured
            .take_variants_request(11, 0x1000, slot + 1, false)
            .is_none()
    );
    let ticks = captured.capture_tables_all(&memory, 11, 0x1000, slot + 1, false);
    for (index, table) in production::TABLES.into_iter().enumerate().skip(3).take(4) {
        assert_eq!(ticks[table.index()], 0);
        assert_eq!(
            captured.production.outcomes()[index],
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON
            }
        );
    }
    for failure in [
        crate::acquisition::Failure::Panicked,
        crate::acquisition::Failure::Stopped,
    ] {
        let request = captured
            .take_variants_request(11, 0x1000, slot, false)
            .expect("the variant slot loans the family");
        assert_eq!(
            captured.allocated_bytes() + request.allocated_bytes(),
            bytes
        );
        captured.restore_variants_request(request, Err(failure));
        assert_eq!(captured.allocated_bytes(), bytes);
        let reads = crate::read_accounting::Capture::new();
        captured.capture_tables_all(&memory, 11, 0x1000, slot, false);
        let counts = reads.finish();
        for table in &production::TABLES[3..7] {
            assert_eq!(counts[table.index()], engine::memory::ReadCounts::ZERO);
        }
        let expected = match failure {
            crate::acquisition::Failure::Panicked => "variant acquisition panicked",
            crate::acquisition::Failure::Stopped => "variant acquisition stopped before reading",
        };
        for index in 3..7 {
            assert_eq!(
                captured.production.outcomes()[index],
                Outcome::Unavailable { reason: expected }
            );
        }
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
            if address == 0x1000 + engine::cgame_state::TECH_SHARING_GROUPS {
                self.governance.set(self.governance.get() + 1);
            }
            false
        }
    }
    for failure in [
        None,
        Some(crate::acquisition::Failure::Panicked),
        Some(crate::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.governance.focus_raw_mode = true;
        captured.governance_report = Some(governance::CaptureReport::default());
        let bytes = captured.allocated_bytes();
        let governance_bytes = captured.governance.allocated_bytes();
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let request = captured.take_governance_request(
            &countries,
            11,
            0x1000,
            1,
            failure.is_none(),
            Box::new(crate::slicing::SliceFingerprints::new()),
        );
        assert!(captured.governance_report.is_none());
        assert_eq!(captured.governance.allocated_bytes(), 0);
        assert_eq!(
            captured.allocated_bytes() + request.allocated_bytes(),
            bytes
        );
        assert_eq!(request.captured.allocated_bytes(), governance_bytes);
        assert!(request.allocated_bytes() > governance_bytes);
        assert!(request.captured.focus_raw_mode);
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
        let gated = captured.take_gated_request(11, 0x1000, 1, context.force_full);
        captured.restore_gated_request(gated, false);
        let _technologies =
            captured.restore_governance_request(request, failure.map_or(Ok(report), Err));
        // Variants leave only on their slot or a full hour.
        let variants = captured.take_variants_request(11, 0x1000, 1, context.force_full);
        assert_eq!(variants.is_some(), context.force_full);
        if let Some(variants) = variants {
            captured.restore_variants_request(variants, Ok(variant_ticks));
        }
        assert_eq!(Arc::strong_count(&countries), 1);
        assert_eq!(captured.allocated_bytes(), bytes);
        assert!(captured.governance.focus_raw_mode);
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
                        reason: replay::cadence::SKIPPED_REASON
                    }
                );
            }
            if failure.is_some() && governance::TECHNOLOGIES.contains(&slot) {
                assert!(matches!(
                    expected_outcomes[slot],
                    Outcome::Unavailable { reason } if reason != replay::cadence::SKIPPED_REASON
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
    for failure in [
        None,
        Some(crate::acquisition::Failure::Panicked),
        Some(crate::acquisition::Failure::Stopped),
    ] {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.intel.capitulation_raw_mode = true;
        captured.intel.relations_raw_mode = true;
        let bytes = captured.allocated_bytes();
        let family_bytes = captured.intel.allocated_bytes();
        let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
        let context = intel::CaptureContext {
            base: 11,
            game_state: 0x1000,
            hour: 1,
            force_full: true,
        };
        let request = captured.take_intel_residual_request(&countries, context);
        assert_eq!(captured.intel.allocated_bytes(), 0);
        assert_eq!(captured.allocated_bytes() + family_bytes, bytes);
        assert_eq!(request.captured.allocated_bytes(), family_bytes);
        assert!(Arc::ptr_eq(&request.countries, &countries));
        let original = intel::CaptureReport {
            ticks: std::array::from_fn(|index| index as u64 + 101),
        };
        captured.restore_intel_residual_request(request, failure.map_or(Ok(original), Err));
        assert_eq!(Arc::strong_count(&countries), 1);
        assert_eq!(captured.allocated_bytes(), bytes);
        let outcomes = captured.intel.outcomes();
        let report = captured.acquire_intel(&NoReads, context);
        assert!(captured.intel_residual_report.is_none());
        for index in (1..=12).chain(std::iter::once(15)) {
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
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let walked = |ships: u64| {
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.ship_models.begin_sources();
        for id in 1..=ships {
            captured.ship_models.visit_source(
                0x1000 * id as usize,
                &engine::cship::Row {
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
        .capture_owned(&Faulty, ship_models::CaptureContext { base: 11 });
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
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
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
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    let bytes = captured.allocated_bytes();
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
        let before = captured.outcomes();
        captured.supply_trains_report = Some(supply_trains::CaptureReport::default());
        captured.character_portraits_report = Some(character_portraits::CaptureReport::default());
        captured.province_unit_order_report = Some(province_unit_order::CaptureReport::default());
        let supply: Vec<_> = (0..supply_trains::PARTS)
            .map(|part| captured.take_supply_trains_request(&countries, 11, 0x1000, part))
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
        let province_order = captured.take_province_unit_order_request(11, 0x1000);
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
        captured.restore_province_unit_order_request(
            province_order,
            province_order_failure.map_or(Ok(province_order_report), Err),
        );
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
        assert_eq!(captured.allocated_bytes(), bytes);
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
fn metadata_partitions_every_extended_table_once_and_preserves_cost_membership() {
    // BorderConflicts is the Provinces walk's second output, set by the core spool.
    assert_eq!(N, replay::frame::TABLE_COUNT - 21);
    let actual: BTreeSet<_> = TABLES.into_iter().collect();
    assert_eq!(actual.len(), N);
    let expected: BTreeSet<_> = TableId::ALL
        .into_iter()
        .filter(|table| table.index() >= 20 && *table != TableId::BorderConflicts)
        .collect();
    assert_eq!(actual, expected);
    for (name, anchor, members) in read_groups() {
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
        } else if anchor == TableId::Ships {
            assert_eq!(members, vec![TableId::Ships, TableId::ShipModels]);
        } else if anchor == TableId::States {
            assert_eq!(members, vec![TableId::States, TableId::StateFlags]);
        } else {
            assert!(members.iter().all(|table| actual.contains(table)));
        }
    }
    let groups = read_groups();
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
fn registry_false_counts_are_rejected_and_incomplete_censuses_remain_incomplete() {
    assert!(matches!(
        bounded_registry(Outcome::Rows { len: 3 }, 2),
        Outcome::Unavailable { .. }
    ));
    assert!(matches!(
        bounded_registry(
            Outcome::Truncated {
                len: 0,
                unread_rows: 0
            },
            2
        ),
        Outcome::Unavailable { .. }
    ));
    let empty = bounded_registry(Outcome::Rows { len: 0 }, 0);
    assert_eq!(empty, Outcome::Rows { len: 0 });
    let truncated = Outcome::Truncated {
        len: 1,
        unread_rows: 9,
    };
    assert_eq!(bounded_registry(truncated, 1), truncated);
}

#[test]
fn the_slow_job_stores_travel_with_it_and_always_return() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
    for completed in [true, false] {
        let stores = SlowStores {
            stability: Some(Box::new(crate::slicing::SliceFingerprints::new())),
            naval_missions: Some(Box::new(crate::slicing::SliceFingerprints::new())),
        };
        let request = captured.take_slow_request(&countries, 11, 0x1000, 9, false, stores);
        assert!(request.stores.stability.is_some() && request.stores.naval_missions.is_some());
        // Nothing was read: the unpublished reads are invalidated or the
        // failed job's stores forgotten, and the stores come back either way.
        let stores = captured.restore_slow_request(request, completed);
        assert!(stores.stability.is_some() && stores.naval_missions.is_some());
        assert!(!matches!(
            captured.country_stability.outcomes()[0],
            Outcome::Rows { .. }
        ));
    }
    let request =
        captured.take_slow_request(&countries, 11, 0x1000, 9, false, SlowStores::default());
    let stores = captured.restore_slow_request(request, true);
    assert!(stores.stability.is_none() && stores.naval_missions.is_none());
}

#[test]
fn modifier_header_verdict_travels_with_the_slow_pack_and_returns() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    let countries = Arc::new(engine::country_memory::Cache::with_capacity(0));
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
fn focus_pilot_handoff_reaches_the_governance_pack() {
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.focus_pilot().focus_raw_mode = true;
    assert!(captured.governance.focus_raw_mode);
    captured.focus_pilot().focus_raw_mode = false;
    assert!(!captured.governance.focus_raw_mode);
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
    captured.characters_pilot().raw_active = [true; 6];
    assert!(captured.characters.raw_active.iter().any(|&active| active));
    captured.characters_pilot().raw_active = [false; 6];
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
            engine::memory::SliceMemory {
                base: air::wing_test_kit::BASE,
                bytes,
            }
            .copy(address, destination)
        })
    }
}

#[test]
fn source_adapter_routes_each_staged_consumer_without_replacing_faults() {
    let capacity = Capacities::default();
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
