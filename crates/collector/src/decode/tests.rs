use super::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;

// Tests retaining payloads exercise decoding directly and publish only after
// that successful walk; production also gates after payload recycling.
fn process_test_job(shared: &Shared, job: &mut DecodeJob, memo: &mut AirBasesMemo) {
    process_job(shared, job, memo);
    if !crate::allocation::failed() {
        shared.ready.push(job.index).unwrap();
        shared.completed[job.index].store(job.game_elapsed.max(1), Ordering::Release);
    }
}

/// Calibration globals are shared across tests in this binary; the tests
/// below that reset or advance them hold this lock so assertions stay
/// deterministic under parallel execution.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn advisor_payload_storage_survives_pool_preparation_and_recycling() {
    let _guard = TEST_LOCK.lock().unwrap();
    let saved = std::mem::take(&mut CHARACTERS_JOB_POOL.lock().unwrap()[2]);
    release_characters_job(CharactersJob::new_part(2));
    for _ in 0..2 {
        let job = acquire_characters_job(2);
        assert_eq!(job.part, 2);
        assert!(
            job.has_storage(),
            "typed advisor storage must not be mistaken for an empty raw fallback"
        );
        release_characters_job(job);
    }
    CHARACTERS_JOB_POOL.lock().unwrap()[2] = saved;
}

#[test]
fn retirement_capacity_covers_every_character_part_and_owned_frame() {
    let _guard = TEST_LOCK.lock().unwrap();
    prepare_retirement();
    reclaim_payloads();
    let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    // Full refreshes can recur on loads or discontinuities. If staging
    // overflows or has no reusable storage, every owned frame can retire
    // one object for each handoff before the decode worker makes progress.
    // Empty payloads exercise the real retirement path without large arenas.
    for _ in 0..BUFFERS {
        recycle_relations_job(RelationsJob::default());
        for part in 0..CHARACTER_PARTS {
            let mut job = CharactersJob::default();
            job.part = part;
            recycle_characters_job(job);
        }
        recycle_projects_job(ProjectsJob::default());
        recycle_lexical_job(LexicalJob::default());
        recycle_divisions_job(DivisionsJob::default());
        recycle_focus_job(FocusJob::default());
        recycle_air_wing_job(AirWingJob::default());
        recycle_air_stats_job(AirStatsJob::default());
        recycle_naval_job(NavalJob::default());
        recycle_players_job(PlayersJob::default());
        recycle_session_job(SessionJob::default());
        recycle_production_rest_job(ProductionRestJob::default());
        recycle_construction_rest_job(ConstructionRestJob::default());
        recycle_economy_rest_job(EconomyRestJob::default());
        recycle_air_bases_job(AirBasesJob::default());
        recycle_air_history_job(AirHistoryJob::default());
        recycle_supremacy_job(SupremacyJob::default());
        recycle_sunk_job(SunkJob::default());
        recycle_radar_job(RadarJob::default());
        recycle_intel_job(IntelJob::default());
        recycle_weather_job(WeatherJob::default());
        recycle_capitulation_job(CapitulationJob::default());
    }
    assert_eq!(retired.len(), RETIREMENT_CAPACITY);
    assert!(retired.is_full());
    reclaim_payloads();
    assert!(retired.is_empty());
}
#[test]
fn exhausted_or_contended_payload_pools_decline_without_allocating() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    fn check<J>(pool: &Mutex<Vec<J>>, acquire: fn() -> J, vacant: impl Fn(J)) {
        let mut locked = pool.lock().unwrap();
        vacant(acquire());
        let owned = std::mem::take(&mut *locked);
        drop(locked);
        vacant(acquire());
        assert!(pool.lock().unwrap().is_empty());
        *pool.lock().unwrap() = owned;
    }
    check(&LEXICAL_JOB_POOL, acquire_lexical_job, |job| {
        assert!(!job.initialized());
        assert_eq!(job.capacities(), (0, 0));
    });
    check(&DIVISIONS_JOB_POOL, acquire_divisions_job, |job| {
        assert!(!job.initialized());
        assert_eq!(job.census.allocated_bytes(), 0);
    });
    check(&FOCUS_JOB_POOL, acquire_focus_job, |job| {
        assert!(!job.is_provisioned());
        assert_eq!(job.capacities(), ([0; 2], 0));
    });
    check(&AIR_WING_JOB_POOL, acquire_air_wing_job, |job| {
        assert!(!job.has_storage());
    });
    check(&AIR_STATS_JOB_POOL, acquire_air_stats_job, |job| {
        assert_eq!(job.log.spans.capacity(), 0);
        assert_eq!(job.log.bytes.capacity(), 0);
    });
}

#[test]
fn worker_returns_every_owned_payload_to_its_reuse_pool() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled frame");
    RELATIONS_JOB_POOL.lock().unwrap().clear();
    CHARACTERS_JOB_POOL
        .lock()
        .unwrap()
        .iter_mut()
        .for_each(Vec::clear);
    PROJECTS_JOB_POOL.lock().unwrap().clear();
    LEXICAL_JOB_POOL.lock().unwrap().clear();
    DIVISIONS_JOB_POOL.lock().unwrap().clear();
    FOCUS_JOB_POOL.lock().unwrap().clear();
    AIR_WING_JOB_POOL.lock().unwrap().clear();
    AIR_STATS_JOB_POOL.lock().unwrap().clear();
    NAVAL_JOB_POOL.lock().unwrap().clear();
    PLAYERS_JOB_POOL.lock().unwrap().clear();
    SESSION_JOB_POOL.lock().unwrap().clear();
    PRODUCTION_REST_JOB_POOL.lock().unwrap().clear();
    CONSTRUCTION_REST_JOB_POOL.lock().unwrap().clear();
    ECONOMY_REST_JOB_POOL.lock().unwrap().clear();
    AIR_BASES_JOB_POOL.lock().unwrap().clear();
    AIR_HISTORY_JOB_POOL.lock().unwrap().clear();
    SUPREMACY_JOB_POOL.lock().unwrap().clear();
    SUNK_JOB_POOL.lock().unwrap().clear();
    RADAR_JOB_POOL.lock().unwrap().clear();
    INTEL_JOB_POOL.lock().unwrap().clear();
    WEATHER_JOB_POOL.lock().unwrap().clear();
    CAPITULATION_JOB_POOL.lock().unwrap().clear();
    let air = AirStatsJob::new();
    let air_allocation = air.log.bytes.as_ptr();
    let job = DecodeJob {
        index,
        game_elapsed: 17,
        relations_payload: Some(RelationsJob::new()),
        characters_payload: [Some(CharactersJob::new()), None, None, None],
        projects_payload: Some(ProjectsJob::new()),
        lexical_payload: Some(LexicalJob::new()),
        divisions_payload: Some(DivisionsJob::new()),
        focus_payload: Some(FocusJob::new()),
        air_wing_payload: Some(AirWingJob::new()),
        air_stats_payload: Some(air),
        naval_payload: Some(NavalJob::new()),
        players_payload: Some(PlayersJob::new()),
        session_payload: Some(SessionJob::new()),
        production_rest_payload: Some(ProductionRestJob::new()),
        construction_rest_payload: Some(ConstructionRestJob::new()),
        economy_rest_payload: Some(EconomyRestJob::new()),
        air_bases_payload: Some(AirBasesJob::new()),
        air_history_payload: Some(AirHistoryJob::new()),
        supremacy_payload: Some(SupremacyJob::new()),
        sunk_payload: Some(SunkJob::new()),
        radar_payload: Some(RadarJob::new()),
        intel_payload: Some(IntelJob::new()),
        weather_payload: Some(WeatherJob::new()),
        capitulation_payload: Some(CapitulationJob::new()),
        ..Default::default()
    };
    // A callback can retire a refusal after an idle drain and then enqueue
    // this frame. Its retirement must be consumed before the lease returns.
    recycle_focus_job(FocusJob::default());
    let retirement = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    assert_eq!(retirement.len(), 1);
    process_owned_job(&shared, job, &mut AirBasesMemo::default());
    assert!(retirement.is_empty());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 17);
    assert_eq!(
        RELATIONS_JOB_POOL.lock().unwrap().len(),
        1,
        "RELATIONS_JOB_POOL"
    );
    assert_eq!(
        CHARACTERS_JOB_POOL.lock().unwrap()[0].len(),
        1,
        "CHARACTERS_JOB_POOL"
    );
    assert_eq!(
        PROJECTS_JOB_POOL.lock().unwrap().len(),
        1,
        "PROJECTS_JOB_POOL"
    );
    assert_eq!(
        LEXICAL_JOB_POOL.lock().unwrap().len(),
        1,
        "LEXICAL_JOB_POOL"
    );
    assert_eq!(
        DIVISIONS_JOB_POOL.lock().unwrap().len(),
        1,
        "DIVISIONS_JOB_POOL"
    );
    assert_eq!(FOCUS_JOB_POOL.lock().unwrap().len(), 1, "FOCUS_JOB_POOL");
    assert_eq!(
        AIR_WING_JOB_POOL.lock().unwrap().len(),
        1,
        "AIR_WING_JOB_POOL"
    );
    assert_eq!(
        AIR_STATS_JOB_POOL.lock().unwrap().len(),
        1,
        "AIR_STATS_JOB_POOL"
    );
    assert_eq!(NAVAL_JOB_POOL.lock().unwrap().len(), 1, "NAVAL_JOB_POOL");
    assert_eq!(
        PLAYERS_JOB_POOL.lock().unwrap().len(),
        1,
        "PLAYERS_JOB_POOL"
    );
    assert_eq!(
        SESSION_JOB_POOL.lock().unwrap().len(),
        1,
        "SESSION_JOB_POOL"
    );
    assert_eq!(
        PRODUCTION_REST_JOB_POOL.lock().unwrap().len(),
        1,
        "PRODUCTION_REST_JOB_POOL"
    );
    assert_eq!(
        CONSTRUCTION_REST_JOB_POOL.lock().unwrap().len(),
        1,
        "CONSTRUCTION_REST_JOB_POOL"
    );
    assert_eq!(
        ECONOMY_REST_JOB_POOL.lock().unwrap().len(),
        1,
        "ECONOMY_REST_JOB_POOL"
    );
    assert_eq!(
        AIR_BASES_JOB_POOL.lock().unwrap().len(),
        1,
        "AIR_BASES_JOB_POOL"
    );
    assert_eq!(
        AIR_HISTORY_JOB_POOL.lock().unwrap().len(),
        1,
        "AIR_HISTORY_JOB_POOL"
    );
    assert_eq!(
        SUPREMACY_JOB_POOL.lock().unwrap().len(),
        1,
        "SUPREMACY_JOB_POOL"
    );
    assert_eq!(SUNK_JOB_POOL.lock().unwrap().len(), 1, "SUNK_JOB_POOL");
    assert_eq!(RADAR_JOB_POOL.lock().unwrap().len(), 1, "RADAR_JOB_POOL");
    assert_eq!(INTEL_JOB_POOL.lock().unwrap().len(), 1, "INTEL_JOB_POOL");
    assert_eq!(
        WEATHER_JOB_POOL.lock().unwrap().len(),
        1,
        "WEATHER_JOB_POOL"
    );
    assert_eq!(
        CAPITULATION_JOB_POOL.lock().unwrap().len(),
        1,
        "CAPITULATION_JOB_POOL"
    );
    let air = acquire_air_stats_job();
    assert_eq!(air.log.bytes.as_ptr(), air_allocation);
    recycle_air_stats_job(air);
}

#[test]
fn overflow_growth_is_deferred_and_keeps_its_flag_until_reclaimed() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    AIR_STATS_JOB_POOL.lock().unwrap().clear();
    let mut job = AirStatsJob::new();
    job.log = crate::raw::RawLog::new(1, 1);
    let bytes = [0u8; 2];
    let memory = engine::memory::SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert!(job.log.take(&memory, 0x1000, 1, 1).is_some());
    assert!(job.log.take(&memory, 0x1001, 1, 1).is_none());
    recycle_air_stats_job(job);
    // The game-side retirement neither grows nor publishes a reusable job.
    assert!(AIR_STATS_JOB_POOL.lock().unwrap().is_empty());
    reclaim_payloads();
    let job = acquire_air_stats_job();
    assert!(job.log.spans.capacity() >= 2);
    assert!(job.log.bytes.capacity() >= 2);
    assert!(!job.log.overflowed);
    // Do not leave the intentionally tiny fixture in another test's pool.
    drop(job);
}

#[test]
fn fifo_absent_daily_payloads_preserve_skipped_outcomes_and_idle_pools() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let daily_pool_lengths = || {
        [
            PROJECTS_JOB_POOL.lock().unwrap().len(),
            FOCUS_JOB_POOL.lock().unwrap().len(),
            AIR_STATS_JOB_POOL.lock().unwrap().len(),
            SESSION_JOB_POOL.lock().unwrap().len(),
            PRODUCTION_REST_JOB_POOL.lock().unwrap().len(),
            CONSTRUCTION_REST_JOB_POOL.lock().unwrap().len(),
            AIR_HISTORY_JOB_POOL.lock().unwrap().len(),
            SUPREMACY_JOB_POOL.lock().unwrap().len(),
            SUNK_JOB_POOL.lock().unwrap().len(),
            RADAR_JOB_POOL.lock().unwrap().len(),
        ]
    };
    let idle_pools = daily_pool_lengths();
    assert!(idle_pools.iter().all(|len| *len != 0));
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    frame
        .extended
        .air_stats_pilot()
        .fail_air_stats_capture(replay::cadence::SKIPPED_REASON);
    frame
        .extended
        .focus_pilot()
        .fail_focus_capture(false, replay::cadence::SKIPPED_REASON);
    let focus_before = frame.extended.focus_pilot().outcomes();
    let air_before = frame.extended.air_stats_pilot().outcomes();
    let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    assert!(retired.is_empty());
    for hour in [4, 5, 19] {
        let mut job = bare_job(index, next_seq(), hour, 29);
        // Actual admission remains authoritative even when the envelope
        // alone would request a daily family or every table.
        job.force_full = true;
        assert!(enqueue(&shared, job).is_none());
        process_owned_job(
            &shared,
            shared.decode_queue.pop().expect("queued skipped frame"),
            &mut AirBasesMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.ready.pop(), None);
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 29);
        assert!(retired.is_empty());
        assert_eq!(daily_pool_lengths(), idle_pools);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(frame.extended.focus_pilot().outcomes(), focus_before);
        assert_eq!(frame.extended.air_stats_pilot().outcomes(), air_before);
    }
    assert_eq!(calibration().refresh_jobs, 0);
}

#[test]
fn lazy_world_request_acquires_only_scheduled_parts_and_retires_them_once() {
    use crate::read_accounting;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let pool_lengths = || {
        [
            PRODUCTION_REST_JOB_POOL.lock().unwrap().len(),
            CONSTRUCTION_REST_JOB_POOL.lock().unwrap().len(),
            FOCUS_JOB_POOL.lock().unwrap().len(),
            ECONOMY_REST_JOB_POOL.lock().unwrap().len(),
        ]
    };
    let idle = pool_lengths();
    assert!(idle.iter().all(|&len| len > 0));
    let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    let countries = Arc::new(engine::country_memory::Cache::try_with_capacity(16).unwrap());
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    for (hour, force_full, parts, economy) in [
        // CountryResources is hourly; its census and the exchanges
        // publish in their daily slot.
        (0, false, [false, false, false], [true, false, false]),
        (3, false, [false, true, false], [true, false, false]),
        (8, false, [true, false, false], [true, false, false]),
        (19, false, [false, false, true], [true, false, false]),
        (-5, false, [false, false, true], [true, false, false]),
        (27, false, [false, true, false], [true, false, false]),
        (32, false, [true, false, false], [true, false, false]),
        (10, false, [false, false, false], [true, true, false]),
        (17, false, [false, false, false], [true, false, true]),
        (0, true, [true, true, true], [true, true, true]),
    ] {
        assert!(retired.is_empty());
        let reads = read_accounting::Capture::new();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let focus = frame
            .extended
            .focus_pilot()
            .focus_copy_context(0, 0, parts[2]);
        let request = sampling::acquire_world_request(
            &countries,
            0,
            0,
            focus,
            frame.extended.economy_rest_pilot(),
            hour,
            force_full,
        );
        assert_eq!(
            [
                request.production.is_some(),
                request.construction.is_some(),
                request.focus.is_some(),
            ],
            parts,
            "hour {hour}, force_full {force_full}"
        );
        assert_eq!(
            [
                request.economy.context.resources_active,
                request.economy.context.definitions_slot,
                request.economy.context.other_active,
            ],
            economy
        );
        let owned = [parts[0], parts[1], parts[2], true];
        let after_acquire: [usize; 4] =
            std::array::from_fn(|slot| idle[slot] - usize::from(owned[slot]));
        assert_eq!(pool_lengths(), after_acquire);
        assert!(retired.is_empty(), "absent parts must not retire a payload");
        assert_eq!(Arc::strong_count(&countries), 2);
        if let Some(part) = request.production {
            recycle_production_rest_job(part.payload);
        }
        if let Some(part) = request.construction {
            recycle_construction_rest_job(part.payload);
        }
        if let Some(part) = request.focus {
            assert!(part.context.active);
            recycle_focus_job(part.payload);
        }
        recycle_economy_rest_job(request.economy.payload);
        drop(request.countries);
        assert_eq!(Arc::strong_count(&countries), 1);
        assert_eq!(pool_lengths(), after_acquire);
        assert_eq!(
            retired.len(),
            owned.into_iter().filter(|&active| active).count()
        );
        assert!(enqueue(&shared, bare_job(index, next_seq(), hour, 31)).is_none());
        process_owned_job(
            &shared,
            shared
                .decode_queue
                .pop()
                .expect("queued retirement boundary"),
            &mut AirBasesMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.ready.pop(), None);
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 31);
        assert!(shared.decode_queue.is_empty());
        assert!(retired.is_empty());
        assert_eq!(pool_lengths(), idle);
        assert_eq!(reads.finish(), read_accounting::ZERO);
    }
}

#[test]
fn missing_declared_payloads_fail_only_the_frozen_active_tables() {
    use engine::walk::Outcome;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let characters = [true, false, false, true, false, false];
    let relations = [false, true];
    frame
        .extended
        .characters_pilot()
        .set_character_activity(characters);
    frame
        .extended
        .relations_pilot()
        .set_relation_activity(relations);
    frame.extended.air_bases_pilot().prepare_hour(0, false);
    frame.extended.naval_pilot().prepare_hour(0, false);
    frame.extended.radar_pilot().prepare_hour(0, false);
    let mut job = bare_job(index, next_seq(), 21, 37);
    // Neither this hour nor force_full may replace callback-frozen activity.
    job.force_full = true;
    job.characters_refresh = [true, false, false, false];
    job.relations_refresh = true;
    job.air_wing_refresh = true;
    job.air_bases_refresh = true;
    job.naval_refresh = true;
    job.focus_refresh = true;
    process_owned_job(&shared, job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.ready.pop(), None);
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 37);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let missing = Outcome::Unavailable {
        reason: MISSING_PAYLOAD_REASON,
    };
    let skipped = Outcome::Unavailable {
        reason: replay::cadence::SKIPPED_REASON,
    };
    for (active, outcome) in characters
        .into_iter()
        .zip(frame.extended.characters_pilot().outcomes())
    {
        assert_eq!(outcome, if active { missing } else { skipped });
    }
    assert_eq!(
        frame.extended.relations_pilot().outcomes()[13..15],
        [skipped, missing]
    );
    let air = frame.extended.air_bases_pilot().outcomes();
    assert_eq!(
        [air[0], air[1], air[2], air[5]],
        [missing, missing, skipped, skipped]
    );
    assert_eq!(frame.extended.radar_pilot().outcomes()[5], missing);
    let naval = frame.extended.naval_pilot().outcomes();
    // LandTheatres headers are hourly; the theatre details are daily.
    assert_eq!(naval[4], missing);
    assert_eq!(naval[5..10], [skipped; 5]);
    assert_eq!(naval[14..17], [missing; 3]);
    assert_eq!(frame.extended.focus_pilot().outcomes()[..2], [missing; 2]);
    assert_eq!(calibration().refresh_jobs, 0);
}

#[test]
fn missing_division_payload_publishes_hourly_census_reason_even_without_details() {
    use engine::walk::Outcome;
    use replay::rows::TableId;

    struct NoReads;
    impl engine::Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("missing division payload must not retry native reads");
        }
    }
    let mut frame = sampling::Captured::new();
    for details_active in [false, true] {
        frame.extended.reset();
        let hour = (0..24)
            .find(|&hour| {
                replay::cadence::should_walk(TableId::DivisionEquipment, hour, false)
                    == details_active
            })
            .unwrap();
        let details = frame.extended.test_division_detail();
        details.divisions_raw_mode = true;
        details.rest_raw_mode = true;
        details.acquire(
            &NoReads,
            0,
            0,
            crate::extended::production::Sources {
                rows: &[],
                outcome: Outcome::Rows { len: 0 },
            },
            hour,
            false,
        );
        assert_eq!(details.division_details_active(), details_active);
        let mut job = bare_job(0, 1, hour, 0);
        job.divisions_refresh = true;
        fail_missing_payloads(&mut frame, &job);
        let missing = Outcome::Unavailable {
            reason: MISSING_PAYLOAD_REASON,
        };
        assert_eq!(frame.divisions_result, missing);
        let outcomes = frame.extended.outcomes();
        assert_eq!(outcomes[TableId::DivisionCensus.index()], missing);
        for table in [
            TableId::DivisionEquipment,
            TableId::ArmyUpgradeGroups,
            TableId::ArmyUpgradeRequests,
            TableId::ArmyUpgradeDeliveries,
            TableId::ArmyUpgradeProduced,
        ] {
            assert_eq!(
                outcomes[table.index()],
                if details_active {
                    missing
                } else {
                    Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON,
                    }
                }
            );
        }
        assert!(!frame.extended.test_division_detail().divisions_raw_mode);
    }
}

#[test]
fn relations_gate_admits_every_hour_for_its_owner_slice() {
    for hour in -48..48i32 {
        assert!(relations_refresh_hour(hour, false), "hour {hour}");
        for table in RELATIONS {
            assert!(replay::cadence::is_sliced(table));
            assert!(replay::cadence::should_walk(table, hour, false));
        }
        assert!(relations_refresh_hour(hour, true), "forced hour {hour}");
    }
}

#[test]
fn characters_gate_walks_the_roster_roles_and_traits_every_hour() {
    for hour in -48..48i32 {
        assert!(characters_refresh_hour(hour, false), "hour {hour}");
        for table in CHARACTERS {
            assert!(
                replay::cadence::should_walk(table, hour, false),
                "{table:?} at hour {hour}"
            );
            assert!(replay::cadence::should_walk(table, hour, true));
        }
        assert!(characters_refresh_hour(hour, true), "forced hour {hour}");
    }
}

#[test]
fn projects_gate_matches_their_daily_slot_and_force_full() {
    let slot = replay::cadence::refresh_slot(PROJECTS[0]).unwrap() as i32;
    for hour in -48..48i32 {
        let refresh = hour.rem_euclid(24) == slot;
        assert_eq!(projects_refresh_hour(hour, false), refresh, "hour {hour}");
        assert!(projects_refresh_hour(hour, true), "forced hour {hour}");
    }
}

#[test]
fn lexical_gate_is_static_and_force_full() {
    for hour in -48..48i32 {
        assert!(!lexical_refresh_hour(hour, false), "hour {hour}");
        assert!(lexical_refresh_hour(hour, true), "forced hour {hour}");
    }
}

#[test]
fn fifo_hands_off_every_frame_in_production_order_without_reorder() {
    let shared = Shared::new(0, 0, 1);
    let mut indices = Vec::new();
    for _ in 0..4 {
        indices.push(shared.free.pop().expect("pooled buffer"));
    }
    // Enqueue out-of-seq labels; handoff order must still be FIFO.
    for (order, index) in indices.iter().enumerate() {
        shared.completed[*index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index: *index,
            seq: 100 + order as u64,
            hour: 200 + order as i32,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 7,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        shared.decode_queue.push(job).unwrap();
    }
    let mut handed = Vec::new();
    while let Some(job) = shared.decode_queue.pop() {
        let index = job.index;
        process_owned_job(&shared, job, &mut AirBasesMemo::default());
        handed.push(index);
    }
    assert_eq!(handed, indices);
    for index in indices {
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 7);
    }
    assert!(shared.ready.pop().is_none());
}

#[test]
fn queue_full_returns_the_job_for_the_existing_gap_path() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(0, 0, 1);
    for seq in 0..BUFFERS {
        let job = DecodeJob {
            index: 0,
            seq: seq as u64,
            hour: 0,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 1,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        assert!(shared.decode_queue.push(job).is_ok());
    }
    let before = DROPS.load(Ordering::Relaxed);
    let job = DecodeJob {
        index: 0,
        seq: u64::MAX,
        hour: 0,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 1,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    assert!(enqueue(&shared, job).is_some());
    assert_eq!(DROPS.load(Ordering::Relaxed), before + 1);
}

#[test]
fn worker_decodes_a_relations_refresh_payload_into_the_same_typed_rows() {
    use crate::extended::intel::relations_test_kit;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = relations_test_kit::opinionated_fixture();
    let memory = relations_test_kit::memory(&bytes);
    let shared = Shared::new(relations_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    // Reference with the pooled caps so truncation behavior matches.
    let opinion_cap = frame.extended.relations_pilot().opinion_rows_cap();
    let tuple_cap = frame.extended.relations_pilot().tuple_rows_cap();
    let direct = relations_test_kit::direct(&memory, opinion_cap, tuple_cap);
    let (report, first) = relations_test_kit::split(&memory, frame.extended.relations_pilot());
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 1,
        relations_refresh: true,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 17,
        relations_payload: Some(first),
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 17);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    relations_test_kit::assert_matches_direct(frame.extended.relations_pilot(), &direct);
}

#[test]
fn worker_relations_mixed_payload_keeps_hourly_tuples_and_skips_opinions() {
    use crate::extended::intel::{self, RelationsCopyContext, RelationsJob};
    use engine::walk::Outcome;
    use intel::relations_test_kit as kit;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::opinionated_fixture();
    let memory = kit::memory(&bytes);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    for opinions_active in [false, true, false] {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let captured = frame.extended.relations_pilot();
        let opinion_cap = captured.opinion_rows_cap();
        let tuple_cap = captured.tuple_rows_cap();
        let mut direct = kit::direct(&memory, opinion_cap, tuple_cap);
        if !opinions_active {
            direct.opinion_rows.clear();
            direct.opinion_outcome = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
        }
        let mut payload = RelationsJob::new();
        let report = intel::copy_relations_refresh(
            &memory,
            RelationsCopyContext {
                base: kit::BASE,
                game_state: kit::GS,
                active: [opinions_active, true],
                opinion_cap,
                tuple_cap,
                epoch: 0,
            },
            &mut payload,
        );
        assert!(report.raw_ok);
        if !opinions_active {
            assert_eq!(report.copy_ticks[0], 0);
            assert_eq!(report.raw_bytes[0], 0);
        }
        captured.relations_raw_mode = true;
        let mut job = bare_job(index, next_seq(), 23, 35);
        job.force_full = true;
        job.relations_refresh = true;
        job.relations_payload = Some(payload);
        process_owned_job(&shared, job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 35);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        kit::assert_matches_direct(frame.extended.relations_pilot(), &direct);
        assert!(!frame.extended.relations_pilot().relations_raw_mode);
    }
}

#[test]
fn worker_decodes_a_characters_refresh_payload_into_the_same_typed_rows() {
    use crate::extended::characters::characters_test_kit;
    use crate::extended::characters::{CharactersCopyContext, CharactersJob};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = characters_test_kit::mapped_fixture();
    let memory = characters_test_kit::memory(&bytes);
    let shared = Shared::new(characters_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let capacities = crate::extended::characters::Capacities {
        rows: [4096; 6],
        bytes: [131_072; 6],
    };
    let direct = characters_test_kit::direct(&memory, &capacities);
    let mut initial = capacities;
    initial.rows[2] = 0;
    initial.bytes[2] = 0;
    *frame.extended.characters_pilot() =
        crate::extended::characters::Captured::with_capacities(&initial);
    let family_bytes_before = frame.extended.characters_pilot().allocated_bytes();
    let pool_bytes_before = shared.pool_bytes.load(Ordering::Relaxed);
    let (trait_rows, trait_bytes) = direct.trait_capture_capacities();
    let ctx = CharactersCopyContext {
        force_full: false,
        base: characters_test_kit::BASE,
        game_state: characters_test_kit::GAME_STATE,
        active: [true; 6],
        trait_rows,
        trait_bytes,
    };
    let mut first = CharactersJob::new();
    let report = crate::extended::characters::copy_characters_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.characters_pilot().raw_active = [true; 6];
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 0,
        relations_refresh: false,
        characters_refresh: [true, false, false, false],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 19,
        relations_payload: None,
        characters_payload: [Some(first), None, None, None],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 19);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    characters_test_kit::assert_matches_direct(frame.extended.characters_pilot(), &direct);
    let grown = frame.extended.characters_pilot().allocated_bytes() - family_bytes_before;
    assert!(grown > 0);
    assert_eq!(
        shared.pool_bytes.load(Ordering::Relaxed),
        pool_bytes_before + grown as u64
    );
    // The worker clears the raw-mode gate it consumed.
    assert!(
        !frame
            .extended
            .characters_pilot()
            .raw_active
            .iter()
            .any(|&active| active)
    );
}

#[test]
fn independent_character_handoffs_isolate_a_missing_part_before_publication() {
    use crate::extended::characters::{self, CharactersCopyContext};
    use characters::characters_test_kit as kit;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let capacities = characters::Capacities {
        rows: [4096; 6],
        bytes: [131_072; 6],
    };
    for (missing, missing_mask) in CHARACTER_MASKS.into_iter().enumerate() {
        let bytes = kit::mapped_fixture();
        let memory = kit::memory(&bytes);
        characters::advisors::note_modifier_demand(
            engine::cadvisor::modifier_demand(&memory, kit::BASE, kit::GAME_STATE).unwrap(),
        );
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let captured = frame.extended.characters_pilot();
        *captured = characters::Captured::with_capacities(&capacities);
        captured.set_character_activity([true; 6]);
        captured.raw_active = [true; 6];
        captured.capture(&memory, kit::BASE, kit::GAME_STATE, 0, false);
        let (trait_rows, trait_bytes) = captured.trait_capture_capacities();
        let mut job = bare_job(index, next_seq(), 0, 37);
        job.characters_refresh = [true; CHARACTER_PARTS];
        for (part, payload) in job.characters_payload.iter_mut().enumerate() {
            if part == missing {
                continue;
            }
            let mut owned = CharactersJob::new_part(part);
            let report = characters::copy_characters_refresh(
                &memory,
                CharactersCopyContext {
                    force_full: false,
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    active: CHARACTER_MASKS[part],
                    trait_rows,
                    trait_bytes,
                },
                &mut owned,
            );
            assert!(report.raw_ok);
            *payload = Some(owned);
        }
        let mut expected = kit::direct(&memory, &capacities);
        expected.fail_character_tables(missing_mask, MISSING_PAYLOAD_REASON);
        drop(bytes);
        process_owned_job(&shared, job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.ready.pop(), None);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        kit::assert_matches_direct(frame.extended.characters_pilot(), &expected);
        assert_eq!(frame.extended.characters_pilot().raw_active, [false; 6]);
    }
}

#[test]
fn worker_characters_mixed_payload_matches_native_with_frozen_activity() {
    use crate::extended::characters::{self, CharactersCopyContext, CharactersJob};
    use characters::characters_test_kit as kit;
    use engine::walk::Outcome;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::mapped_fixture();
    let memory = kit::memory(&bytes);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let capacities = characters::Capacities {
        rows: [4096; 6],
        bytes: [131_072; 6],
    };
    for (hour, force_full) in [(0, false), (21, false), (7, true)] {
        let active = CHARACTERS.map(|table| replay::cadence::should_walk(table, hour, force_full));
        let mut direct = characters::Captured::with_capacities(&capacities);
        direct.capture(&memory, kit::BASE, kit::GAME_STATE, hour, force_full);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        *frame.extended.characters_pilot() = characters::Captured::with_capacities(&capacities);
        let (trait_rows, trait_bytes) = direct.trait_capture_capacities();
        let mut payload = CharactersJob::new();
        let report = characters::copy_characters_refresh(
            &memory,
            CharactersCopyContext {
                force_full: false,
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                active,
                trait_rows,
                trait_bytes,
            },
            &mut payload,
        );
        assert!(report.raw_ok);
        for (slot, is_active) in active.into_iter().enumerate() {
            if !is_active {
                assert_eq!(report.copy_ticks[slot], 0);
                assert_eq!(report.raw_bytes[slot], 0);
            }
        }
        frame.extended.characters_pilot().raw_active = [true; 6];
        let mut job = bare_job(index, next_seq(), 23, 39);
        job.force_full = !force_full;
        job.characters_refresh = [true, false, false, false];
        job.characters_payload = [Some(payload), None, None, None];
        process_owned_job(&shared, job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 39);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let actual = frame.extended.characters_pilot();
        kit::assert_matches_direct(actual, &direct);
        for (is_active, outcome) in active.into_iter().zip(actual.outcomes()) {
            if !is_active {
                assert_eq!(
                    outcome,
                    Outcome::Unavailable {
                        reason: replay::cadence::SKIPPED_REASON
                    }
                );
            }
        }
        assert!(!actual.raw_active.iter().any(|&active| active));
    }
}

#[test]
fn worker_decodes_a_projects_refresh_payload_into_the_same_typed_rows() {
    use crate::extended::projects::projects_test_kit;
    use crate::extended::projects::{ProjectsCopyContext, ProjectsJob};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = projects_test_kit::fixture();
    let memory = projects_test_kit::memory(&bytes);
    let shared = Shared::new(projects_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let capacities = crate::extended::projects::Capacities {
        rows: [256; crate::extended::projects::N],
        program_names: 8192,
        program_candidates: 1024,
        raid_type_tokens: 256,
        raid_show_for: 256,
        raid_transports: 16,
        raid_equipment_items: 16,
        raid_equipment_archetypes: 16,
        counteroperatives: 16,
    };
    let direct = projects_test_kit::direct(&memory, &capacities);
    let ctx = ProjectsCopyContext {
        base: projects_test_kit::BASE,
        game_state: projects_test_kit::GAME_STATE,
        active: true,
    };
    let mut first = ProjectsJob::new();
    let report = crate::extended::projects::copy_projects_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.projects_pilot().raw_mode = true;
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 3,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: true,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 23,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: Some(first),
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 23);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    projects_test_kit::assert_matches_direct(frame.extended.projects_pilot(), &direct);
    // The worker clears the raw-mode gate it consumed.
    assert!(!frame.extended.projects_pilot().raw_mode);
}

#[test]
fn worker_decodes_a_lexical_refresh_payload_into_the_same_typed_rows() {
    use crate::extended::lexical::lexical_test_kit;
    use crate::extended::lexical::{LexicalCopyContext, LexicalJob};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let memory = lexical_test_kit::KitMemory::fixture();
    let shared = Shared::new(lexical_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let capacities = crate::extended::lexical::Capacities {
        rows: 256,
        text: 8192,
    };
    let direct = lexical_test_kit::direct(&memory, &capacities);
    let ctx = LexicalCopyContext {
        base: lexical_test_kit::BASE,
    };
    let mut first = LexicalJob::new();
    let report = crate::extended::lexical::copy_lexical_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.lexical_pilot().raw_mode = true;
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 3,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: true,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 29,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: Some(first),
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 29);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    lexical_test_kit::assert_matches_direct(frame.extended.lexical_pilot(), &direct);
    // The worker clears the raw-mode gate it consumed.
    assert!(!frame.extended.lexical_pilot().raw_mode);
}

#[test]
fn worker_preserves_already_typed_wars_rows() {
    use crate::sampling::wars_test_kit;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = wars_test_kit::fixture();
    let memory = wars_test_kit::memory(&bytes);
    let shared = Shared::new(wars_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let direct = wars_test_kit::direct(&memory, 64);
    {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.wars_result = direct.owned_outcome;
        frame.wars[..direct.owned.len()].copy_from_slice(&direct.owned);
        frame.table_ns[TableId::WarRelations.index()] = 601;
    }
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 41,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 43,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    assert_eq!(frame.wars_result, direct.owned_outcome);
    assert_eq!(&frame.wars[..direct.owned.len()], &direct.owned[..]);
    assert_eq!(frame.table_ns[TableId::WarRelations.index()], 601);
}

#[test]
fn worker_decodes_a_divisions_refresh_payload_into_the_same_typed_rows() {
    use crate::sampling::{
        DivisionsCopyContext, DivisionsJob, copy_divisions_refresh, divisions_test_kit,
    };
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = divisions_test_kit::fixture();
    let memory = divisions_test_kit::memory(&bytes);
    let shared = Shared::new(divisions_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let direct = divisions_test_kit::direct(&memory, [64; 6]);
    let ctx = DivisionsCopyContext {
        base: divisions_test_kit::BASE,
        game_state: divisions_test_kit::GAME_STATE,
        row_capacity: 64,
        source_capacity: 64,
        detail_capacities: [64, 0, 64, 64, 64],
        details_active: true,
    };
    let mut first = DivisionsJob::new();
    let report = copy_divisions_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    frame.extended.test_division_detail().groups.rows = Vec::new();
    let before_bytes = frame.extended.allocated_bytes();
    let pool_before = shared.pool_bytes.load(Ordering::Relaxed);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 42,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: true,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 47,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: Some(first),
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 47);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    assert_eq!(
        shared.pool_bytes.load(Ordering::Relaxed),
        pool_before + (frame.extended.allocated_bytes() - before_bytes) as u64
    );
    assert_eq!(frame.divisions_result, direct.divisions_outcome);
    assert_eq!(
        &frame.divisions[..direct.divisions.len()],
        &direct.divisions[..]
    );
    let detail = frame.extended.test_division_detail();
    assert_eq!(detail.division_equipment.outcome, direct.equipment_outcome);
    assert_eq!(
        &detail.division_equipment.rows[..direct.equipment.len()],
        &direct.equipment[..]
    );
    assert_eq!(detail.groups.outcome, direct.groups_outcome);
    assert_eq!(
        &detail.groups.rows[..direct.groups.len()],
        &direct.groups[..]
    );
    assert_eq!(detail.requests.outcome, direct.requests_outcome);
    assert_eq!(
        &detail.requests.rows[..direct.requests.len()],
        &direct.requests[..]
    );
    assert_eq!(detail.deliveries.outcome, direct.deliveries_outcome);
    assert_eq!(
        &detail.deliveries.rows[..direct.deliveries.len()],
        &direct.deliveries[..]
    );
    assert_eq!(detail.produced.outcome, direct.produced_outcome);
    assert_eq!(
        &detail.produced.rows[..direct.produced.len()],
        &direct.produced[..]
    );
    // The worker clears the raw-mode gate it consumed.
    assert!(!detail.divisions_raw_mode);
    // Core decode remains hourly; this fixture explicitly requests details.
    assert_eq!(
        DIVISIONS,
        [
            TableId::Divisions,
            TableId::DivisionEquipment,
            TableId::ArmyUpgradeGroups,
            TableId::ArmyUpgradeRequests,
            TableId::ArmyUpgradeDeliveries,
            TableId::ArmyUpgradeProduced,
        ]
    );
}

#[test]
fn fifo_uses_frozen_division_detail_activity_and_retires_the_owned_core_once() {
    use crate::sampling::{
        DivisionsCopyContext, copy_divisions_refresh, divisions_test_kit as kit,
    };
    use engine::walk::Outcome;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let skipped = Outcome::Unavailable {
        reason: replay::cadence::SKIPPED_REASON,
    };
    frame
        .extended
        .fail_division_details(replay::cadence::SKIPPED_REASON);
    frame.extended.set_divisions_raw_mode(true);
    let detail_pointer = frame
        .extended
        .test_division_detail()
        .division_equipment
        .rows
        .as_ptr();
    let pool_len = DIVISIONS_JOB_POOL.lock().unwrap().len();
    let mut payload = acquire_divisions_job();
    let context = DivisionsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        row_capacity: 64,
        source_capacity: 64,
        // Inactive detail capacities do not constrain the hourly core.
        detail_capacities: [usize::MAX; 5],
        details_active: false,
    };
    let report = copy_divisions_refresh(&memory, context, &mut payload);
    assert!(report.raw_ok);
    assert_eq!(report.copy_ticks[1..], [0; 5]);
    assert_eq!(report.raw_bytes[1..], [0; 5]);
    let mut job = bare_job(index, next_seq(), 2, 43);
    // The envelope cannot override the capture's frozen activity, even
    // when it names the detail slot and requests a force-full frame.
    job.force_full = true;
    job.divisions_refresh = true;
    job.divisions_payload = Some(payload);
    assert!(enqueue(&shared, job).is_none());
    process_owned_job(
        &shared,
        shared.decode_queue.pop().expect("queued core payload"),
        &mut AirBasesMemo::default(),
    );
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.ready.pop(), None);
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
    assert_eq!(DIVISIONS_JOB_POOL.lock().unwrap().len(), pool_len);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
    let detail = frame.extended.test_division_detail();
    assert_eq!(detail.division_equipment.rows.as_ptr(), detail_pointer);
    for outcome in [
        detail.division_equipment.outcome,
        detail.groups.outcome,
        detail.requests.outcome,
        detail.deliveries.outcome,
        detail.produced.outcome,
    ] {
        assert_eq!(outcome, skipped);
    }
    assert!(!detail.divisions_raw_mode);
    assert_eq!(calibration().divisions_decode_ns[1..], [0; 5]);
}

#[test]
fn prior_active_division_details_stay_skipped_through_core_failure_and_retirement() {
    use crate::{
        read_accounting,
        sampling::{
            DivisionsCopyContext, copy_divisions_core_refresh, copy_divisions_inputs_refresh,
            copy_divisions_refresh, divisions_test_kit as kit,
        },
    };
    use engine::{
        Memory,
        memory::{CountedMemory, ReadCounts},
        walk::Outcome,
    };
    use std::cell::{Cell, RefCell};

    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("inactive details or completed core must not read again");
        }
    }
    struct CoreFault<'a> {
        bytes: &'a [u8],
        calls: RefCell<Vec<(usize, usize)>>,
        fault_attempts: Cell<usize>,
    }
    impl Memory for CoreFault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            let fault = kit::GAME_STATE + engine::cgame_state::PROVINCES;
            if address <= fault && address.saturating_add(output.len()) > fault {
                let attempts = self.fault_attempts.get();
                self.fault_attempts.set(attempts + 1);
                if attempts == 0 {
                    return false;
                }
            }
            kit::memory(self.bytes).copy(address, output)
        }
    }

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let pool_len = DIVISIONS_JOB_POOL.lock().unwrap().len();
    let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    for (active_hour, reject_context) in [(2, false), (26, true)] {
        let context = DivisionsCopyContext {
            base: kit::BASE,
            game_state: kit::GAME_STATE,
            row_capacity: 64,
            source_capacity: 64,
            detail_capacities: [64; 5],
            details_active: true,
        };
        let mut payload = acquire_divisions_job();
        assert!(copy_divisions_refresh(&kit::memory(&bytes), context, &mut payload).raw_ok);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.extended.set_divisions_raw_mode(true);
        assert_eq!(
            frame.extended.production_rest_pilot().acquire(
                &NoRead,
                kit::BASE,
                kit::GAME_STATE,
                crate::extended::production::Sources {
                    rows: &[],
                    outcome: Outcome::Rows { len: 0 },
                },
                active_hour,
                false,
            ),
            [0; crate::extended::production::N]
        );
        let mut active = bare_job(index, next_seq(), active_hour, 41);
        active.divisions_refresh = true;
        active.divisions_payload = Some(payload);
        assert!(enqueue(&shared, active).is_none());
        process_owned_job(
            &shared,
            shared.decode_queue.pop().expect("queued active details"),
            &mut AirBasesMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(index));
        assert!(shared.ready.is_empty());
        assert_eq!(DIVISIONS_JOB_POOL.lock().unwrap().len(), pool_len);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(frame.divisions_result, Outcome::Rows { len: 3 });
        let detail = frame.extended.test_division_detail();
        assert!(detail.division_details_active());
        for outcome in [
            detail.division_equipment.outcome,
            detail.groups.outcome,
            detail.requests.outcome,
            detail.deliveries.outcome,
            detail.produced.outcome,
        ] {
            assert!(matches!(outcome, Outcome::Rows { len } if len > 0));
        }
        let detail_pointer = detail.division_equipment.rows.as_ptr();
        let first_equipment = detail.division_equipment.rows[0];
        let detail_decode_before = calibration().divisions_decode_ns;

        let hour = active_hour + 1;
        let context = DivisionsCopyContext {
            // Both readiness checks must ignore these impossible grants.
            detail_capacities: [usize::MAX; 5],
            details_active: false,
            ..context
        };
        let memory = CountedMemory::new(CoreFault {
            bytes: &bytes,
            calls: RefCell::new(Vec::new()),
            fault_attempts: Cell::new(0),
        });
        let reads = read_accounting::Capture::new();
        let mut payload = acquire_divisions_job();
        assert!(copy_divisions_inputs_refresh(
            &memory,
            context,
            &mut payload
        ));
        assert_eq!(memory.0.fault_attempts.get(), 0);
        let lease = Arc::clone(payload.census.unit_inputs().unwrap());
        let input_pointer = Arc::as_ptr(&lease);
        frame.extended.install_unit_inputs(Some(Arc::clone(&lease)));
        assert_eq!(Arc::strong_count(&lease), 3);
        let report = if reject_context {
            copy_divisions_core_refresh(
                &NoRead,
                DivisionsCopyContext {
                    row_capacity: 0,
                    ..context
                },
                &mut payload,
            )
        } else {
            copy_divisions_core_refresh(&memory, context, &mut payload)
        };
        assert_eq!(report.raw_ok, !reject_context);
        assert_eq!(report.copy_ticks[1..], [0; 5]);
        assert_eq!(report.raw_bytes[1..], [0; 5]);
        let before_repeat = read_accounting::snapshot();
        let repeated = copy_divisions_core_refresh(&NoRead, context, &mut payload);
        assert_eq!(repeated.raw_ok, report.raw_ok);
        assert_eq!(repeated.copy_ticks, report.copy_ticks);
        assert_eq!(repeated.raw_bytes, report.raw_bytes);
        assert_eq!(read_accounting::snapshot(), before_repeat);
        assert_eq!(memory.0.fault_attempts.get(), usize::from(!reject_context));
        assert!(Arc::ptr_eq(&lease, payload.census.unit_inputs().unwrap()));

        frame.extended.set_divisions_raw_mode(true);
        assert_eq!(
            frame.extended.production_rest_pilot().acquire(
                &NoRead,
                kit::BASE,
                kit::GAME_STATE,
                crate::extended::production::Sources {
                    rows: &[],
                    outcome: Outcome::Rows { len: 0 },
                },
                hour,
                false,
            ),
            [0; crate::extended::production::N]
        );
        assert!(
            !frame
                .extended
                .test_division_detail()
                .division_details_active()
        );
        let failure = if reject_context {
            "division owned acquisition could not complete"
        } else {
            "province array unreadable or invalid"
        };
        // A failed hourly core cannot mark the five inactive siblings failed.
        frame.extended.fail_division_details(failure);
        let counts = reads.finish();
        let trace = memory.0.calls.borrow();
        let core = counts[TableId::Divisions.index()];
        assert!(core.spans > 0);
        assert_eq!(
            core.spans,
            trace.iter().filter(|(_, len)| *len != 0).count() as u64
        );
        assert_eq!(
            core.bytes,
            trace.iter().map(|(_, len)| *len as u64).sum::<u64>()
        );
        for table in TableId::ALL {
            if table != TableId::Divisions {
                assert_eq!(counts[table.index()], ReadCounts::ZERO, "{table:?}");
            }
        }
        drop(trace);
        frame.table_reads = counts;
        frame.extended.release_unit_inputs();
        drop(lease);
        assert_eq!(Arc::strong_count(payload.census.unit_inputs().unwrap()), 1);
        assert!(retired.is_empty());

        let mut failed = bare_job(index, next_seq(), hour, 43);
        if reject_context {
            frame.divisions_result = Outcome::Unavailable { reason: failure };
            recycle_divisions_job(payload);
        } else {
            failed.divisions_refresh = true;
            failed.divisions_payload = Some(payload);
        }
        assert_eq!(retired.len(), usize::from(reject_context));
        assert_eq!(DIVISIONS_JOB_POOL.lock().unwrap().len(), pool_len - 1);
        assert!(enqueue(&shared, failed).is_none());
        let decoding = read_accounting::Capture::new();
        process_owned_job(
            &shared,
            shared.decode_queue.pop().expect("queued failed core"),
            &mut AirBasesMemo::default(),
        );
        assert_eq!(decoding.finish(), read_accounting::ZERO);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.ready.pop(), None);
        assert!(shared.decode_queue.is_empty());
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
        assert!(retired.is_empty());
        assert_eq!(DIVISIONS_JOB_POOL.lock().unwrap().len(), pool_len);
        reclaim_payloads();
        let pool = DIVISIONS_JOB_POOL.lock().unwrap();
        assert_eq!(pool.len(), pool_len);
        assert_eq!(
            Arc::as_ptr(pool.last().unwrap().census.unit_inputs().unwrap()),
            input_pointer
        );
        drop(pool);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(
            frame.divisions_result,
            Outcome::Unavailable { reason: failure }
        );
        assert_eq!(frame.table_reads, counts);
        let detail = frame.extended.test_division_detail();
        for outcome in [
            detail.division_equipment.outcome,
            detail.groups.outcome,
            detail.requests.outcome,
            detail.deliveries.outcome,
            detail.produced.outcome,
        ] {
            assert_eq!(
                outcome,
                Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON
                }
            );
        }
        assert_eq!(detail.division_equipment.rows.as_ptr(), detail_pointer);
        assert_eq!(detail.division_equipment.rows[0], first_equipment);
        assert!(!detail.divisions_raw_mode);
        assert_eq!(
            calibration().divisions_decode_ns[1..],
            detail_decode_before[1..]
        );
    }
}

#[test]
fn initial_payload_capacities_cover_documented_seed_shapes() {
    use crate::extended::{
        air::AirStatsJob, characters::CharactersJob, intel::RelationsJob, lexical::LexicalJob,
        projects::ProjectsJob,
    };
    // Analytic worst cases from the 2026-09-11 appendix (late-campaign
    // maxima): relations 8639 opinions / 1512 tuples; characters 13127 rows with
    // 19744 experience rows; projects 21511 rows with 1756 breakthroughs;
    // lexical 70851 tokens; divisions 2843 with 17364 equipment rows and
    // about 4000 upgrade rows. Every cap below must exceed its shape's
    // span/byte cost so staging overflow — which falls back to the typed
    // walk with identical rows (`raw_ok=false`, enqueues nothing, no gap)
    // — never triggers in steady state. This is distinct from a full
    // decode queue, which takes the existing whole-frame drop plus
    // `LossBatch` gap path. A
    // force-full hour stages every family at once. These are seed-shape
    // assertions, not an aggregate memory bound or a growth limit.
    let relations = RelationsJob::new();
    assert!(relations.opinions.len() >= 8639);
    assert!(relations.tuples.len() >= 1512);
    let characters = CharactersJob::new();
    assert!(characters.log.spans.capacity() >= 300_000);
    assert!(characters.log.bytes.capacity() >= 10 << 20);
    let projects = ProjectsJob::new();
    assert!(projects.log.spans.capacity() >= 21_511 * 32);
    assert!(projects.log.bytes.capacity() >= 21_511 * 1024);
    let lexical = LexicalJob::new();
    assert!(lexical.capacities().0 >= 70_851 + 16);
    assert!(lexical.capacities().1 >= 70_851 * 32 * 3);
    let divisions = crate::sampling::DivisionsJob::new();
    assert!(divisions.initialized());
    // Hourly focus worst case is 439 countries with about 3.9k completed
    // rows plus token bytes (the appendix census proves the shape).
    let focus = FocusJob::new();
    assert!(focus.capacities().0[0] >= 439);
    assert!(focus.capacities().0[1] >= 3_900);
    assert!(focus.capacities().1 >= 128 << 10);
    // Hourly air-statistics worst case is 330 rows (appendix census)
    // with per-history channel spans; both pools start above that so
    // steady state never overflows.
    let air_stats = AirStatsJob::new();
    assert!(air_stats.log.spans.capacity() >= 330 * 64);
    assert!(air_stats.log.bytes.capacity() >= 330 * 2048);
    // Queue capacity follows the typed-frame ownership bound. Raw spans,
    // metadata and all three owned jobs must also be counted in memory
    // reporting; byte arenas alone substantially undercount the live set.
    assert_eq!(BUFFERS, 4);
}

#[test]
fn queue_high_water_tracks_the_deepest_observed_fifo_occupancy() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    for _ in 0..3 {
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 0,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            focus_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
            air_history_refresh: false,
            supremacy_refresh: false,
            sunk_refresh: false,
            radar_refresh: false,
            intel_refresh: false,
            weather_refresh: false,
            capitulation_refresh: false,
            force_full: false,
            game_elapsed: 1,
            relations_payload: None,
            characters_payload: [const { None }; CHARACTER_PARTS],
            projects_payload: None,
            lexical_payload: None,
            divisions_payload: None,
            focus_payload: None,
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
            air_history_payload: None,
            supremacy_payload: None,
            sunk_payload: None,
            radar_payload: None,
            intel_payload: None,
            weather_payload: None,
            capitulation_payload: None,
        };
        assert!(enqueue(&shared, job).is_none());
    }
    let snapshot = calibration();
    assert_eq!(snapshot.queue_high_water, 3);
    assert_eq!(snapshot.json()["queue_high_water"], 3);
    assert_eq!(snapshot.json()["queue_capacity"], BUFFERS);
    while shared.decode_queue.pop().is_some() {}
}

#[test]
fn worker_decodes_a_focus_payload_into_the_same_typed_rows() {
    use crate::extended::governance::focus_test_kit;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let fixture = focus_test_kit::Fixture::world();
    let memory = fixture.memory();
    let shared = Shared::new(focus_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let mut typed = focus_test_kit::Fixture::captured();
    typed.capture(&memory, focus_test_kit::BASE, 0x1_0000, 0, true);
    frame.extended.focus_pilot().focus_raw_mode = true;
    let (report, first) =
        focus_test_kit::Fixture::copy_first(frame.extended.focus_pilot(), &memory);
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 7,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: true,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 23,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: Some(first),
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 23);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let pilot = frame.extended.focus_pilot();
    assert!(!pilot.focus_raw_mode);
    focus_test_kit::Fixture::assert_matches_typed(pilot, &typed);
    let snapshot = calibration();
    assert_eq!(snapshot.focus_copy_ns, [0, 0]);
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["focus_tables"][0], "Focus");
    assert_eq!(snapshot.json()["focus_tables"][1], "CompletedFocuses");
}

#[test]
fn worker_decodes_a_wing_pack_payload_into_the_same_typed_rows() {
    use crate::extended::air::{AirWingCopyContext, wing_test_kit};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = wing_test_kit::fixture();
    let memory = wing_test_kit::memory(&bytes);
    let shared = Shared::new(wing_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let direct = wing_test_kit::direct(&memory);
    let ctx = AirWingCopyContext {
        base: wing_test_kit::BASE,
        game_state: wing_test_kit::GAME_STATE,
        hour: 0,
        force_full: true,
        limits: [1024, 8192, 4096],
        naval_limit: 4096,
        game_score: false,
    };
    let mut first = acquire_air_wing_job();
    let report = crate::extended::air::copy_air_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 9,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        air_wing_refresh: true,
        air_stats_refresh: false,
        force_full: false,
        game_elapsed: 31,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
        air_wing_payload: Some(first),
        air_stats_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 31);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    wing_test_kit::assert_captured_matches_direct(frame.extended.air_wing_pilot(), &direct);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["air_wing_tables"][0], "WingEquipment");
    recycle_air_wing_job(job.air_wing_payload.unwrap());
}

#[test]
fn worker_decodes_an_air_stats_payload_into_the_same_typed_rows() {
    use crate::extended::air::{AirStatsCopyContext, stats_test_kit};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = stats_test_kit::fixture();
    let memory = stats_test_kit::memory(&bytes);
    let shared = Shared::new(stats_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let capacities = crate::extended::air::Capacities::default();
    let direct = stats_test_kit::direct(&memory, &capacities);
    let ctx = AirStatsCopyContext {
        base: stats_test_kit::BASE,
        game_state: stats_test_kit::GAME_STATE,
        hour: 0,
        force_full: true,
    };
    let mut first = acquire_air_stats_job();
    let report = crate::extended::air::copy_air_stats_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 9,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: true,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 59,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: Some(first),
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 59);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    stats_test_kit::assert_captured_matches_direct(frame.extended.air_stats_pilot(), &direct);
    assert!(!frame.extended.air_stats_pilot().stats_raw_mode);
    // table_ns stays copy-only for the moved table.
    assert_eq!(frame.table_ns[AIR_STATS[0].index()], 0);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["air_stats_tables"][0], "AirStatistics");
    recycle_air_stats_job(job.air_stats_payload.unwrap());
}

#[test]
fn air_stats_calibration_counters_are_monotonic_and_additive() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    add_air_stats_copy_ns(0, 11);
    add_air_stats_raw_bytes(0, 100);
    let snapshot = calibration();
    assert_eq!(snapshot.air_stats_copy_ns, [11]);
    assert_eq!(snapshot.air_stats_raw_bytes, [100]);
    assert_eq!(snapshot.json()["air_stats_copy_ns"][0], 11);
    assert_eq!(snapshot.json()["air_stats_tables"][0], "AirStatistics");
    let job = acquire_air_stats_job();
    assert!(job.log.spans.capacity() >= 1 << 16);
    recycle_air_stats_job(job);
    let again = acquire_air_stats_job();
    assert!(again.log.spans.capacity() >= 1 << 16);
    recycle_air_stats_job(again);
}

#[test]
fn worker_decodes_a_naval_pack_payload_into_the_same_typed_rows() {
    use crate::extended::naval_history::{NavalCopyContext, naval_test_kit};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = naval_test_kit::fixture();
    let memory = naval_test_kit::memory(&bytes);
    let shared = Shared::new(naval_test_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let direct = naval_test_kit::direct(&memory);
    let ctx = NavalCopyContext {
        base: naval_test_kit::BASE,
        game_state: naval_test_kit::GAME_STATE,
        hour: 0,
        force_full: true,
    };
    let mut first = acquire_naval_job();
    naval_test_kit::stage_wings(&memory, &mut first);
    let report = crate::extended::naval_history::copy_naval_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 9,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: true,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 37,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: Some(first),
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 37);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    naval_test_kit::assert_captured_matches_direct(frame.extended.naval_pilot(), &direct);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["naval_tables"][0], "LandTheatres");
    assert_eq!(snapshot.json()["naval_tables"][8], "ArmyGroupMembers");
    recycle_naval_job(job.naval_payload.unwrap());
}

#[test]
fn retained_air_naval_payload_caps_fit_the_force_full_hour_live_set() {
    // Independent fixture worlds exercise each retained consumer pool.
    use crate::extended::air::{AirWingCopyContext, wing_test_kit as air_kit};
    use crate::extended::naval_history::{NavalCopyContext, naval_test_kit as naval_kit};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let air_bytes = air_kit::fixture();
    let air_memory = air_kit::memory(&air_bytes);
    let mut air_job = acquire_air_wing_job();
    let air_ctx = AirWingCopyContext {
        base: air_kit::BASE,
        game_state: air_kit::GAME_STATE,
        hour: 0,
        force_full: true,
        limits: [1024, 8192, 4096],
        naval_limit: 4096,
        game_score: false,
    };
    let air_report = crate::extended::air::copy_air_refresh(&air_memory, air_ctx, &mut air_job);
    assert!(
        air_report.raw_ok,
        "wing force-full live set must fit owned storage"
    );
    assert!(!air_job.grow_for_next());
    let naval_bytes = naval_kit::fixture();
    let naval_memory = naval_kit::memory(&naval_bytes);
    let mut naval_job = acquire_naval_job();
    let naval_ctx = NavalCopyContext {
        base: naval_kit::BASE,
        game_state: naval_kit::GAME_STATE,
        hour: 0,
        force_full: true,
    };
    naval_kit::stage_wings(&naval_memory, &mut naval_job);
    let naval_report = crate::extended::naval_history::copy_naval_refresh(
        &naval_memory,
        naval_ctx,
        &mut naval_job,
    );
    assert!(
        naval_report.raw_ok,
        "naval force-full live set must fit spans/bytes"
    );
    assert!(!naval_job.grow_for_next());
    recycle_air_wing_job(air_job);
    recycle_naval_job(naval_job);
}

#[test]
fn focus_calibration_counters_are_monotonic_and_additive_per_table() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    add_focus_copy_ns(0, 11);
    add_focus_copy_ns(0, 4);
    add_focus_copy_ns(1, 0);
    add_focus_raw_bytes(1, 100);
    let snapshot = calibration();
    assert_eq!(snapshot.focus_copy_ns, [15, 0]);
    assert_eq!(snapshot.focus_raw_bytes, [0, 100]);
    assert_eq!(snapshot.json()["focus_copy_ns"][0], 15);
    assert_eq!(snapshot.json()["focus_tables"][0], "Focus");
    // Recycled jobs keep their capacities for the next hour.
    let job = acquire_focus_job();
    assert!(job.is_provisioned());
    assert!(job.capacities().0[0] >= 439);
    assert!(job.capacities().0[1] >= 3_900);
    assert!(job.capacities().1 >= 128 << 10);
    recycle_focus_job(job);
    let again = acquire_focus_job();
    assert!(again.is_provisioned());
    assert!(again.capacities().0[0] >= 439);
    assert!(again.capacities().0[1] >= 3_900);
    assert!(again.capacities().1 >= 128 << 10);
    recycle_focus_job(again);
}

#[test]
fn worker_decodes_a_production_rest_payload_into_the_same_typed_rows() {
    use crate::extended::production::{ProductionRestCopyContext, production_rest_test_kit as kit};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = engine::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let direct = kit::direct(&memory, &kit::capacities());
    let ctx = ProductionRestCopyContext {
        base: 0,
        game_state: kit::BASE,
    };
    let mut first = acquire_production_rest_job();
    let report =
        crate::extended::production::copy_production_rest_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.production_rest_pilot().rest_raw_mode = true;
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 11,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: true,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 31,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: Some(first),
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 31);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    kit::assert_matches_direct(frame.extended.production_rest_pilot(), &direct);
    assert!(!frame.extended.production_rest_pilot().rest_raw_mode);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(
        snapshot.json()["production_rest_tables"][0],
        "ProductionLines"
    );
    assert_eq!(
        snapshot.json()["production_rest_tables"][7],
        "ProductionLicenseRecords"
    );
}

#[test]
fn worker_decodes_a_construction_rest_payload_into_the_same_typed_rows() {
    use crate::extended::construction::{
        ConstructionRestCopyContext, construction_rest_test_kit as kit,
    };
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = engine::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    // Runtime presizes the occupied-country arena before admission. This
    // synthetic Shared has no live sizing observer; install the same fixture
    // capacities used by the independent typed capture.
    *frame.extended.construction_rest_pilot() =
        crate::extended::construction::Captured::with_capacities(&kit::capacities());
    let direct = kit::direct(&memory, &kit::capacities());
    let ctx = ConstructionRestCopyContext {
        base: 0,
        game_state: kit::BASE,
    };
    let mut first = acquire_construction_rest_job();
    let report =
        crate::extended::construction::copy_construction_rest_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.construction_rest_pilot().rest_raw_mode = true;
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 12,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: true,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 37,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: Some(first),
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 37);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    kit::assert_matches_direct(frame.extended.construction_rest_pilot(), &direct);
    assert!(!frame.extended.construction_rest_pilot().rest_raw_mode);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(
        snapshot.json()["construction_rest_tables"][0],
        "StateBaseResources"
    );
    assert_eq!(
        snapshot.json()["construction_rest_tables"][9],
        "CountryCollaboration"
    );
}

#[test]
fn production_rest_calibration_counters_are_monotonic_and_additive_per_table() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    add_production_rest_copy_ns(0, 11);
    add_production_rest_copy_ns(0, 4);
    add_production_rest_copy_ns(7, 9);
    add_production_rest_raw_bytes(7, 100);
    let snapshot = calibration();
    assert_eq!(snapshot.production_rest_copy_ns, [15, 0, 0, 0, 0, 0, 0, 9]);
    assert_eq!(
        snapshot.production_rest_raw_bytes,
        [0, 0, 0, 0, 0, 0, 0, 100]
    );
    assert_eq!(snapshot.json()["production_rest_copy_ns"][0], 15);
    assert_eq!(
        snapshot.json()["production_rest_tables"][0],
        "ProductionLines"
    );
    let job = acquire_production_rest_job();
    assert!(job.log.spans.capacity() >= 1 << 17);
    recycle_production_rest_job(job);
    let again = acquire_production_rest_job();
    assert!(again.log.spans.capacity() >= 1 << 17);
    recycle_production_rest_job(again);
}

#[test]
fn worker_decodes_two_payloads_in_one_job_with_copy_only_table_ns() {
    // S4: two families decode in one process_job on one Shared. Both
    // payloads' rows install identically, both family counters advance
    // (REFRESH_JOBS +2), and per-frame table_ns stays copy-only (the
    // worker never writes it; decode ns goes to the calibration
    // counters). Fixture BASE compatibility: both the wing and naval
    // kits pin BASE=0 with GAME_STATE=0x10000, so one Shared::new(0,..)
    // serves both payloads at full strength with no fixture adaptation
    // and no weakened assertion (each payload is still staged from its
    // own fixture memory and compared against its own typed reference).
    use crate::extended::air::{AirWingCopyContext, wing_test_kit as air_kit};
    use crate::extended::naval_history::{NavalCopyContext, naval_test_kit as naval_kit};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    // Stage each payload from its own fixture memory (separate address
    // spaces; the kits share BASE/GAME_STATE values but not objects).
    let air_bytes = air_kit::fixture();
    let air_memory = air_kit::memory(&air_bytes);
    let air_direct = air_kit::direct(&air_memory);
    let air_ctx = AirWingCopyContext {
        base: air_kit::BASE,
        game_state: air_kit::GAME_STATE,
        hour: 0,
        force_full: true,
        limits: [1024, 8192, 4096],
        naval_limit: 4096,
        game_score: false,
    };
    let mut air_job = acquire_air_wing_job();
    let air_report = crate::extended::air::copy_air_refresh(&air_memory, air_ctx, &mut air_job);
    assert!(air_report.raw_ok);
    let naval_bytes = naval_kit::fixture();
    let naval_memory = naval_kit::memory(&naval_bytes);
    let naval_direct = naval_kit::direct(&naval_memory);
    let naval_ctx = NavalCopyContext {
        base: naval_kit::BASE,
        game_state: naval_kit::GAME_STATE,
        hour: 0,
        force_full: true,
    };
    let mut naval_job = acquire_naval_job();
    naval_kit::stage_wings(&naval_memory, &mut naval_job);
    let naval_report = crate::extended::naval_history::copy_naval_refresh(
        &naval_memory,
        naval_ctx,
        &mut naval_job,
    );
    assert!(naval_report.raw_ok);
    // One Shared serves both: BASE coincides (0 == 0), so no mismatch.
    assert_eq!(air_kit::BASE, naval_kit::BASE);
    let shared = Shared::new(air_kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    shared.completed[index].store(0, Ordering::Relaxed);
    let refresh_before = REFRESH_JOBS.load(Ordering::Relaxed);
    let mut job = DecodeJob {
        index,
        seq: next_seq(),
        hour: 9,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: true,
        air_stats_refresh: false,
        naval_refresh: true,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed: 53,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: Some(air_job),
        air_stats_payload: None,
        naval_payload: Some(naval_job),
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    };
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 53);
    // Both families' rows install identically in the one frame.
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    air_kit::assert_captured_matches_direct(frame.extended.air_wing_pilot(), &air_direct);
    naval_kit::assert_captured_matches_direct(frame.extended.naval_pilot(), &naval_direct);
    // Both counters advance: one REFRESH_JOB per family payload.
    assert_eq!(REFRESH_JOBS.load(Ordering::Relaxed), refresh_before + 2);
    // table_ns stays copy-only: the worker wrote rows + calibration
    // decode ns, never the per-frame copy-only slots.
    for table in AIR_WING.iter().chain(NAVAL_ORDER.iter()) {
        assert_eq!(
            frame.table_ns[table.index()],
            0,
            "table_ns for {table:?} must stay copy-only"
        );
    }
    let snapshot = calibration();
    assert!(snapshot.air_wing_decode_ns.iter().any(|ns| *ns > 0) || snapshot.refresh_jobs >= 2);
    assert!(snapshot.naval_decode_ns.iter().any(|ns| *ns > 0) || snapshot.refresh_jobs >= 2);
    recycle_air_wing_job(job.air_wing_payload.unwrap());
    recycle_naval_job(job.naval_payload.unwrap());
}

#[test]
fn construction_rest_calibration_counters_are_monotonic_and_additive_per_table() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    add_construction_rest_copy_ns(0, 11);
    add_construction_rest_copy_ns(0, 4);
    add_construction_rest_copy_ns(9, 9);
    add_construction_rest_raw_bytes(9, 100);
    let snapshot = calibration();
    assert_eq!(
        snapshot.construction_rest_copy_ns,
        [15, 0, 0, 0, 0, 0, 0, 0, 0, 9]
    );
    assert_eq!(
        snapshot.construction_rest_raw_bytes,
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 100]
    );
    assert_eq!(snapshot.json()["construction_rest_copy_ns"][0], 15);
    assert_eq!(
        snapshot.json()["construction_rest_tables"][0],
        "StateBaseResources"
    );
    let job = acquire_construction_rest_job();
    assert!(job.log.spans.capacity() >= 1 << 17);
    recycle_construction_rest_job(job);
    let again = acquire_construction_rest_job();
    assert!(again.log.spans.capacity() >= 1 << 17);
    recycle_construction_rest_job(again);
}

/// Build a FIFO job carrying only the selected family payload (all other
/// families off). Every worker test below uses this shape with its own
/// kit-staged payload.
fn bare_job(index: usize, seq: u64, hour: i32, game_elapsed: i64) -> DecodeJob {
    DecodeJob {
        index,
        seq,
        hour,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        focus_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
        air_history_refresh: false,
        supremacy_refresh: false,
        sunk_refresh: false,
        radar_refresh: false,
        intel_refresh: false,
        weather_refresh: false,
        capitulation_refresh: false,
        force_full: false,
        game_elapsed,
        relations_payload: None,
        characters_payload: [const { None }; CHARACTER_PARTS],
        projects_payload: None,
        lexical_payload: None,
        divisions_payload: None,
        focus_payload: None,
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
        air_history_payload: None,
        supremacy_payload: None,
        sunk_payload: None,
        radar_payload: None,
        intel_payload: None,
        weather_payload: None,
        capitulation_payload: None,
    }
}

#[test]
fn worker_air_stats_keeps_current_daily_payloads_and_explicit_missing_results() {
    use crate::extended::air::{AirStatsCopyContext, Capacities, stats_test_kit as kit};
    use engine::walk::Outcome;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let mut bytes = kit::fixture();
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let mut bases_memo = AirBasesMemo::default();
    for (step, hour) in [5, 29, 30, 53, 77, 101, 125, 149].into_iter().enumerate() {
        let mut job = bare_job(index, step as u64 + 1, hour, 41);
        job.air_stats_refresh = replay::cadence::should_walk(AIR_STATS[0], hour, false);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let captured = frame.extended.air_stats_pilot();
        captured.prepare_hour(hour, false);
        if job.air_stats_refresh && step != 5 {
            // Every scheduled payload contains a different current value;
            // earlier rows must not survive a skip or missing payload.
            kit::set_first_value(&mut bytes, -(step as i64 + 1));
            let memory = kit::memory(&bytes);
            let mut payload = AirStatsJob::new();
            let report = crate::extended::air::copy_air_stats_refresh(
                &memory,
                AirStatsCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour,
                    force_full: false,
                },
                &mut payload,
            );
            assert!(report.raw_ok);
            captured.stats_raw_mode = true;
            captured.bases_raw_mode = true;
            captured.history_raw_mode = true;
            // The pack reset must preserve the staged-payload reservation.
            captured.capture(
                &memory,
                kit::BASE,
                kit::GAME_STATE,
                hour,
                false,
                &mut Default::default(),
            );
            job.air_stats_payload = Some(payload);
        } else if !job.air_stats_refresh {
            captured.fail_air_stats_capture(replay::cadence::SKIPPED_REASON);
        }
        process_test_job(&shared, &mut job, &mut bases_memo);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 41);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let captured = frame.extended.air_stats_pilot();
        if step == 2 || step == 5 {
            assert_eq!(
                captured.outcomes()[6],
                Outcome::Unavailable {
                    reason: if step == 2 {
                        replay::cadence::SKIPPED_REASON
                    } else {
                        MISSING_PAYLOAD_REASON
                    },
                }
            );
        } else {
            let direct = kit::direct(&kit::memory(&bytes), &Capacities::default());
            kit::assert_captured_matches_direct(captured, &direct);
        }
        assert!(!captured.stats_raw_mode);
    }
}

#[test]
fn worker_air_bases_memo_invalidates_on_skip_missing_payload_and_ace_mask_changes() {
    use crate::extended::air::{AirBasesCopyContext, air_bases_test_kit as kit};
    use engine::walk::Outcome;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let mut memo = AirBasesMemo::default();
    let expected_hits = [0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 4, 5];
    let expected_installs = [1, 1, 1, 2, 2, 2, 3, 3, 4, 4, 5, 5];
    for step in 1..=12 {
        let mut job = bare_job(index, step, step as i32 + 100, 41);
        job.air_bases_refresh = step != 3;
        if step != 3 && step != 6 {
            let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
            let mut payload = AirBasesJob::new();
            let ctx = AirBasesCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour: if (9..=10).contains(&step) { 21 } else { 0 },
                force_full: false,
            };
            let report = crate::extended::air::copy_air_bases_refresh(
                &kit::memory(&bytes),
                ctx,
                &mut payload,
            );
            assert!(report.raw_ok);
            if !(9..=10).contains(&step) {
                assert_eq!(report.copy_ticks[2], 0);
                assert_eq!(report.raw_bytes[2], 0);
            }
            frame.extended.air_bases_pilot().bases_raw_mode = true;
            frame.extended.radar_pilot().registrations_raw_mode = true;
            job.air_bases_payload = Some(payload);
        }
        process_test_job(&shared, &mut job, &mut memo);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 41);
        assert_eq!(
            AIR_BASES_MEMO[0].load(Ordering::Relaxed),
            expected_hits[step as usize - 1]
        );
        assert_eq!(
            AIR_BASES_MEMO[4].load(Ordering::Relaxed),
            expected_installs[step as usize - 1]
        );
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        if !(9..=10).contains(&step) {
            assert_eq!(
                frame.extended.air_bases_pilot().outcomes()[5],
                Outcome::Unavailable {
                    reason: replay::cadence::SKIPPED_REASON,
                }
            );
        }
        if step == 6 {
            let missing = Outcome::Unavailable {
                reason: MISSING_PAYLOAD_REASON,
            };
            assert_eq!(frame.extended.air_bases_pilot().outcomes()[1], missing);
            assert_eq!(frame.extended.radar_pilot().outcomes()[5], missing);
        }
    }
    let diagnostics = calibration().json();
    assert_eq!(diagnostics["air_bases_memo"]["hits"], 5);
    assert_eq!(diagnostics["air_bases_memo"]["misses"], 5);
    assert_eq!(diagnostics["air_bases_memo"]["installs"], 5);
}

#[test]
fn worker_decodes_a_shared_bases_payload_into_the_same_typed_rows() {
    use crate::extended::air::air_bases_test_kit as kit;
    use replay::frame::TableView;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    shared.completed[index].store(0, Ordering::Relaxed);
    // Stage through the real game-thread entry point (cross-pack copy).
    let ctx = crate::extended::air::AirBasesCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
    };
    let mut first = acquire_air_bases_job();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let report = crate::extended::air::copy_air_bases_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.air_bases_pilot().bases_raw_mode = true;
    frame.extended.radar_pilot().registrations_raw_mode = true;
    let mut job = bare_job(index, next_seq(), 7, 41);
    job.air_bases_refresh = true;
    job.air_bases_payload = Some(first);
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 41);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    // Both packs' views install identically: bases and aces in air,
    // registrations in fog, with the worker clearing both raw gates.
    let air = frame.extended.air_bases_pilot().tables();
    let fog = frame.extended.radar_pilot().tables();
    assert!(matches!(air[1], TableView::Complete(_)));
    assert!(matches!(air[5], TableView::Complete(_)));
    assert!(matches!(fog[5], TableView::Complete(_)));
    assert_eq!(
        frame.extended.air_bases_pilot().outcomes()[1],
        direct.bases_outcome
    );
    assert_eq!(
        frame.extended.air_bases_pilot().outcomes()[5],
        direct.aces_outcome
    );
    assert!(!frame.extended.air_bases_pilot().bases_raw_mode);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["air_bases_tables"][0], "AirBases");
    assert_eq!(
        snapshot.json()["air_bases_tables"][1],
        "AirBaseRegistrations"
    );
    assert_eq!(snapshot.json()["air_bases_tables"][2], "AirAces");
    recycle_air_bases_job(job.air_bases_payload.unwrap());
}

#[test]
fn worker_decodes_a_shared_history_payload_into_the_same_typed_rows() {
    use crate::extended::air::air_history_test_kit as kit;
    use replay::frame::TableView;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    shared.completed[index].store(0, Ordering::Relaxed);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let [history_limit, damage_limit] = frame.extended.air_history_pilot().air_history_limits();
    let ctx = crate::extended::air::AirHistoryCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
        history_limit,
        damage_limit,
    };
    let mut first = acquire_air_history_job();
    let report = crate::extended::air::copy_air_history_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.air_history_pilot().history_raw_mode = true;
    let mut job = bare_job(index, next_seq(), 7, 43);
    job.air_history_refresh = true;
    job.air_history_payload = Some(first);
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 43);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let air = frame.extended.air_history_pilot().tables();
    assert!(matches!(air[3], TableView::Complete(_)));
    assert!(matches!(air[4], TableView::Complete(_)));
    assert_eq!(
        frame.extended.air_history_pilot().outcomes()[3],
        direct.history_outcome
    );
    assert_eq!(
        frame.extended.air_history_pilot().outcomes()[4],
        direct.damage_outcome
    );
    assert!(!frame.extended.air_history_pilot().history_raw_mode);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["air_history_tables"][0], "AirRegionHistory");
    assert_eq!(snapshot.json()["air_history_tables"][1], "AirRegionDamage");
    recycle_air_history_job(job.air_history_payload.unwrap());
}

#[test]
fn worker_decodes_a_supremacy_payload_into_the_same_typed_rows() {
    use crate::extended::naval_history::supremacy_test_kit as kit;
    use replay::frame::TableView;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    shared.completed[index].store(0, Ordering::Relaxed);
    let ctx = crate::extended::naval_history::SupremacyCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
    };
    let mut first = acquire_supremacy_job();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let report = crate::extended::naval_history::copy_supremacy_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.supremacy_pilot().supremacy_raw_mode = true;
    let mut job = bare_job(index, next_seq(), 7, 53);
    job.supremacy_refresh = true;
    job.supremacy_payload = Some(first);
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 53);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let naval = frame.extended.supremacy_pilot().tables();
    assert!(matches!(naval[0], TableView::Complete(_)));
    assert_eq!(
        frame.extended.supremacy_pilot().outcomes()[0],
        direct.outcome
    );
    assert!(!frame.extended.supremacy_pilot().supremacy_raw_mode);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["supremacy_tables"][0], "NavalSupremacy");
    recycle_supremacy_job(job.supremacy_payload.unwrap());
}

#[test]
fn worker_decodes_a_sunk_payload_into_the_same_typed_rows() {
    use crate::extended::naval_history::sunk_test_kit as kit;
    use replay::frame::TableView;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    shared.completed[index].store(0, Ordering::Relaxed);
    let ctx = crate::extended::naval_history::SunkCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: kit::HOUR,
        force_full: true,
    };
    let mut first = acquire_sunk_job();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let report = crate::extended::naval_history::copy_sunk_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.sunk_pilot().sunk_raw_mode = true;
    let mut job = bare_job(index, next_seq(), 7, 59);
    job.sunk_refresh = true;
    job.sunk_payload = Some(first);
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 59);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    // The sunk table is pack slot 13; the singleton conversion owns the
    // single row, so compare the converted view (one row, two entries).
    // The raw walk holds two records; the singleton owns one row.
    assert_eq!(direct.outcome, engine::walk::Outcome::Rows { len: 2 });
    let naval = frame.extended.sunk_pilot().tables();
    let entries = match &naval[13] {
        TableView::Complete(replay::rows::TableRows::SunkShips(rows)) => {
            assert_eq!(rows.len(), 1);
            rows[0].entries.len()
        }
        other => panic!("expected complete sunk table, found {other:?}"),
    };
    assert_eq!(entries, 2);
    assert!(!frame.extended.sunk_pilot().sunk_raw_mode);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["sunk_tables"][0], "SunkShips");
    recycle_sunk_job(job.sunk_payload.unwrap());
}

#[test]
fn worker_decodes_a_radar_payload_into_the_same_typed_rows() {
    use crate::extended::fog::radar_test_kit as kit;
    use replay::frame::TableView;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut first = acquire_radar_job();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let ctx = frame
        .extended
        .radar_pilot()
        .radar_copy_context(kit::BASE, kit::GAME_STATE, 0, true);
    let report = crate::extended::fog::copy_radar_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.radar_pilot().radar_raw_mode = true;
    let mut job = bare_job(index, next_seq(), 7, 61);
    job.radar_refresh = true;
    job.radar_payload = Some(first);
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 61);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let fog = frame.extended.radar_pilot().tables();
    assert!(matches!(fog[0], TableView::Complete(_)));
    assert_eq!(frame.extended.radar_pilot().outcomes()[0], direct.outcome);
    assert!(!frame.extended.radar_pilot().radar_raw_mode);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["radar_tables"][0], "RadarCoverage");
    recycle_radar_job(job.radar_payload.unwrap());
}

#[test]
fn worker_decodes_an_intel_payload_into_the_same_typed_rows() {
    use crate::extended::fog::intel_test_kit as kit;
    use replay::frame::TableView;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut first = acquire_intel_job();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let ctx = frame
        .extended
        .intel_pilot()
        .intel_copy_context(kit::BASE, kit::GAME_STATE);
    let report =
        crate::extended::fog::copy_intel_refresh_with_divisions(&memory, ctx, &mut first, None);
    assert!(report.raw_ok);
    frame.extended.intel_pilot().intel_raw_mode = true;
    let mut job = bare_job(index, next_seq(), 7, 67);
    job.intel_refresh = true;
    job.intel_payload = Some(first);
    process_test_job(&shared, &mut job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 67);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    // UnitIntel is fog pack slot 4.
    let fog = frame.extended.intel_pilot().tables();
    assert!(matches!(fog[4], TableView::Complete(_)));
    assert_eq!(frame.extended.intel_pilot().outcomes()[4], direct.outcome);
    assert!(!frame.extended.intel_pilot().intel_raw_mode);
    let snapshot = calibration();
    assert!(snapshot.refresh_jobs >= 1);
    assert_eq!(snapshot.json()["intel_tables"][0], "UnitIntel");
    recycle_intel_job(job.intel_payload.unwrap());
}

#[test]
fn new_family_calibration_counters_are_monotonic_and_additive_per_table() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    add_air_bases_copy_ns(0, 11);
    add_air_bases_copy_ns(2, 7);
    add_air_bases_raw_bytes(1, 100);
    add_supremacy_copy_ns(0, 3);
    add_sunk_copy_ns(0, 4);
    add_radar_copy_ns(0, 6);
    add_intel_copy_ns(0, 8);
    add_air_history_copy_ns(1, 2);
    let snapshot = calibration();
    assert_eq!(snapshot.air_bases_copy_ns, [11, 0, 7]);
    assert_eq!(snapshot.air_bases_raw_bytes, [0, 100, 0]);
    assert_eq!(snapshot.supremacy_copy_ns, [3]);
    assert_eq!(snapshot.sunk_copy_ns, [4]);
    assert_eq!(snapshot.radar_copy_ns, [6]);
    assert_eq!(snapshot.intel_copy_ns, [8]);
    assert_eq!(snapshot.air_history_copy_ns, [0, 2]);
    let json = snapshot.json();
    assert_eq!(json["air_bases_copy_ns"][0], 11);
    assert_eq!(json["air_bases_tables"][1], "AirBaseRegistrations");
    assert_eq!(json["air_history_tables"][1], "AirRegionDamage");
    assert_eq!(json["supremacy_tables"][0], "NavalSupremacy");
    assert_eq!(json["sunk_tables"][0], "SunkShips");
    assert_eq!(json["radar_tables"][0], "RadarCoverage");
    assert_eq!(json["intel_tables"][0], "UnitIntel");
    for acquire in [
        acquire_air_bases_job().log.spans.capacity() >= 1 << 17,
        acquire_air_history_job().has_storage(),
        acquire_supremacy_job().log.spans.capacity() >= 1 << 12,
        acquire_sunk_job().log.spans.capacity() >= 1 << 13,
        acquire_radar_job().allocated_bytes() > 0,
        acquire_intel_job().allocated_bytes() > 0,
    ] {
        assert!(acquire);
    }
}
