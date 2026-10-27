use super::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;

#[test]
fn economy_rest_names_follow_its_tables() {
    assert_eq!(
        ECONOMY_REST_NAMES.map(str::to_owned),
        ECONOMY_REST.map(|table| format!("{table:?}"))
    );
}

#[test]
fn terminal_failure_releases_a_decode_handoff_waiting_on_a_full_queue() {
    let ready = crossbeam_queue::ArrayQueue::new(1);
    ready.push(7).unwrap();
    let completed = std::sync::atomic::AtomicI64::new(99);
    let mut checks = 0;
    assert!(!publish_ready(&ready, 8, &completed, 123, || {
        checks += 1;
        checks == 2
    }));
    assert_eq!(completed.load(Ordering::Acquire), 0);
    assert_eq!(ready.pop(), Some(7));
    assert!(publish_ready(&ready, 8, &completed, 123, || false));
    assert_eq!(completed.load(Ordering::Acquire), 123);
    assert_eq!(ready.pop(), Some(8));
}

#[test]
fn required_project_freeze_failure_is_terminal_before_ready_publication() {
    const CHILD: &str = "BATTLE_PLAN_PROJECT_FREEZE_PUBLICATION_TEST";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "decode::tests::required_project_freeze_failure_is_terminal_before_ready_publication", "--nocapture"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    for ordinal in [1, 2] {
        let ready = crossbeam_queue::ArrayQueue::new(1);
        let completed = std::sync::atomic::AtomicI64::new(99);
        let mut captured = collector_projects::projects::deferred_variable_allocation_fixture();
        crate::allocation::reset();
        crate::allocation::install_hooks();
        let _phase = crate::allocation::enter(crate::allocation::Phase::Decode);
        let denied = crate::test_support::NullGuard::new(ordinal);
        captured.decode_owned();
        drop(denied);
        assert!(crate::test_support::DENIALS.load(Ordering::Relaxed) > 0);
        assert!(crate::allocation::failed());
        assert!(engine_allocation::capture_stopped());
        let failure = crate::allocation::failure().unwrap();
        assert_eq!(failure.phase, crate::allocation::Phase::Decode as u8);
        assert_eq!(failure.code, crate::allocation::Code::Reserve as u8);
        assert!(!publish_ready(
            &ready,
            0,
            &completed,
            123,
            crate::allocation::failed
        ));
        assert_eq!(ready.pop(), None);
        assert_eq!(completed.load(Ordering::Acquire), 0);
        engine_allocation::set_hooks(None, None);
        engine_allocation::set_stop_hook(None);
    }
}

// Tests retaining payloads exercise decoding directly and publish only after
// that successful walk; production also gates after payload recycling.
fn process_test_job(shared: &Shared, job: &mut DecodeJob, memo: &mut AirBasesMemo) {
    let mut service = service::Service::start(shared, job);
    process_job(shared, job, memo, &mut service);
    if !crate::allocation::failed() {
        shared.ready.push(job.index).unwrap();
        shared.completed[job.index].store(job.game_elapsed.max(1), Ordering::Release);
    }
}

fn copy_divisions_worker_refresh(
    memory: &impl engine_native::Memory,
    ctx: crate::sampling::DivisionsCopyContext,
    payload: &mut crate::sampling::DivisionsJob,
) -> crate::sampling::DivisionsCopyReport {
    crate::sampling::copy_divisions_inputs_refresh_growing(memory, ctx, payload);
    crate::sampling::copy_divisions_core_refresh(memory, ctx, payload)
}

/// Calibration globals are shared across tests in this binary; the tests
/// below that reset or advance them hold this lock so assertions stay
/// deterministic under parallel execution.
static TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn decode_panic_retains_its_lease_and_first_cause_across_later_drain() {
    let _guard = TEST_LOCK.lock().unwrap();
    failure::reset();
    let _hook = PanicHook::install();
    let shared = Shared::new(0, 0, 1);
    let interrupted = shared.free.pop().unwrap();
    let queued = shared.free.pop().unwrap();
    let frame = unsafe { &mut *shared.frames[interrupted].load(Ordering::Acquire) };
    frame.world_generation = 12;
    frame.clock = Some(engine_native::cgame_state::Clock {
        game_state: 0,
        hour: 900,
        play_time: 0,
        play_session_started_at: 321,
        seed: 0,
        campaign_id: engine_native::capture_text::Owned::default(),
        speed: 0,
        tick_busy: 0,
    });
    let mut job = bare_job(interrupted, 74, 900, 17);
    job.force_full = true;
    // Inject after the lease was taken, before any family can finish it.
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        failure::worker(|| {
            failure::fail_at("air copies");
            process_owned_job(&shared, job, &mut AirBasesMemo::default());
        });
    }))
    .unwrap_err();
    assert_eq!(shared.ready.pop(), None);
    assert_eq!(shared.completed[interrupted].load(Ordering::Acquire), 0);
    for _ in 0..shared.free.len() {
        let free = shared.free.pop().unwrap();
        assert_ne!(free, interrupted);
        shared.free.push(free).unwrap();
    }
    let first = failure::joined(panic, "worker failed");
    for expected in [
        "worker=decode worker",
        "stage=air copies",
        "seq=74",
        "hour=900",
        "force_full=true",
        "load_signal=Some(321)",
        "world_generation=Some(12)",
        "synthetic decode failure",
        "location=Some(Location",
        "line:",
        "column:",
    ] {
        assert!(first.contains(expected), "{expected}: {first}");
    }

    // A successful later handoff does not publish the interrupted frame or
    // overwrite its identity with another world/hour. A drain panic follows.
    let later = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _drain = failure::Scope::worker("decode detach drain");
        process_owned_job(
            &shared,
            bare_job(queued, 75, 100, 19),
            &mut AirBasesMemo::default(),
        );
        assert_eq!(shared.ready.pop(), Some(queued));
        failure::stage("drain progress", None);
        panic!("later drain failure");
    }))
    .unwrap_err();
    assert_eq!(failure::joined(later, "drain failed"), first);
    assert_eq!(panic_failure(), Some(first));
    assert_eq!(shared.completed[interrupted].load(Ordering::Acquire), 0);
    failure::reset();
}

#[test]
fn decode_failure_context_ends_at_handoff_and_ignores_contained_panics() {
    let _guard = TEST_LOCK.lock().unwrap();
    failure::reset();
    let _hook = PanicHook::install();
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().unwrap();
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        failure::worker(|| {
            let _ = std::panic::catch_unwind(|| panic!("contained inner panic"));
            assert_eq!(panic_failure(), None);
            process_owned_job(
                &shared,
                bare_job(index, 3, 4, 5),
                &mut AirBasesMemo::default(),
            );
            assert_eq!(shared.ready.pop(), Some(index));
            panic!("after handoff");
        });
    }))
    .unwrap_err();
    let first = failure::joined(panic, "worker failed");
    assert!(first.contains("after handoff"), "{first}");
    assert!(
        first.contains("stage=after ready handoff; frame=none"),
        "{first}"
    );
    assert!(!first.contains("contained inner panic"), "{first}");
    assert!(!first.contains("seq=3"), "{first}");
    failure::reset();
}

#[test]
fn decode_join_preserves_string_and_opaque_payloads_without_running_their_drop() {
    let _guard = TEST_LOCK.lock().unwrap();
    struct Payload;
    impl Drop for Payload {
        fn drop(&mut self) {
            panic!("panic payload destructor must not run");
        }
    }
    failure::reset();
    assert!(
        failure::joined(
            Box::new(String::from("owned panic reason")),
            "worker failed"
        )
        .contains("owned panic reason")
    );
    failure::reset();
    assert!(
        failure::joined(Box::new(Payload), "worker failed").contains("non-string panic payload")
    );
    failure::reset();
}

#[test]
fn decode_detach_keeps_worker_panic_after_drain_panic_and_a_later_empty_drain() {
    let _guard = TEST_LOCK.lock().unwrap();
    failure::reset();
    let _hook = PanicHook::install();
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().unwrap();
    shared
        .decode_queue
        .push(bare_job(index, 10, 20, 30))
        .unwrap();
    assert!(sampling::SHARED.load(Ordering::Acquire).is_null());
    assert!(WORKER.lock().unwrap().is_none());
    *WORKER.lock().unwrap() = Some(std::thread::spawn(|| {
        failure::worker(|| panic!("original worker failure"));
    }));
    sampling::SHARED.store((&shared as *const Shared).cast_mut(), Ordering::Release);
    let first = detach_drain(|_| panic!("later detach progress failure")).unwrap_err();
    let later = detach_drain(|_| unreachable!());
    sampling::SHARED.store(std::ptr::null_mut(), Ordering::Release);
    assert!(first.contains("original worker failure"), "{first}");
    assert!(!first.contains("later detach progress failure"), "{first}");
    assert_eq!(later, Err(first));
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 30);
    failure::reset();
}

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
        recycle_air_wing_job(AirWingJob::default());
        recycle_air_stats_job(AirStatsJob::default());
        recycle_naval_job(NavalJob::default());
        recycle_players_job(PlayersJob::default());
        recycle_session_job(SessionJob::default());
        recycle_production_rest_job(ProductionRestJob::default());
        recycle_construction_rest_job(ConstructionRestJob::default());
        recycle_economy_rest_job(EconomyRestJob::default());
        recycle_air_bases_job(AirBasesJob::default());
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
fn cold_growing_weather_jobs_survive_pool_preparation_and_recycling() {
    let _guard = TEST_LOCK.lock().unwrap();
    let saved = std::mem::take(&mut *WEATHER_JOB_POOL.lock().unwrap());
    release_weather_job(WeatherJob::new());
    for _ in 0..2 {
        assert_eq!(WEATHER_JOB_POOL.lock().unwrap().len(), 1);
        let job = acquire_weather_job();
        assert!(
            job.is_provisioned(),
            "cold worker job must retain growth admission"
        );
        assert_eq!(job.log.spans.capacity(), 0);
        assert_eq!(job.log.bytes.capacity(), 0);
        release_weather_job(job);
    }
    release_weather_job(WeatherJob::default());
    assert_eq!(
        WEATHER_JOB_POOL.lock().unwrap().len(),
        1,
        "empty fixed fallback must still be discarded"
    );
    *WEATHER_JOB_POOL.lock().unwrap() = saved;
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
    check(&AIR_WING_JOB_POOL, acquire_air_wing_job, |job| {
        assert!(!job.has_storage());
    });
    check(&AIR_STATS_JOB_POOL, acquire_air_stats_job, |job| {
        assert_eq!(job.log.spans.capacity(), 0);
        assert_eq!(job.log.bytes.capacity(), 0);
    });
}

#[test]
fn worker_recycles_captured_payload_and_drains_vacant_retirements_before_publication() {
    use crate::extended::air::{AirStatsCopyContext, stats_test_kit as kit};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(kit::BASE, 0, 1);
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
    AIR_WING_JOB_POOL.lock().unwrap().clear();
    AIR_STATS_JOB_POOL.lock().unwrap().clear();
    NAVAL_JOB_POOL.lock().unwrap().clear();
    PLAYERS_JOB_POOL.lock().unwrap().clear();
    SESSION_JOB_POOL.lock().unwrap().clear();
    PRODUCTION_REST_JOB_POOL.lock().unwrap().clear();
    CONSTRUCTION_REST_JOB_POOL.lock().unwrap().clear();
    ECONOMY_REST_JOB_POOL.lock().unwrap().clear();
    AIR_BASES_JOB_POOL.lock().unwrap().clear();
    SUPREMACY_JOB_POOL.lock().unwrap().clear();
    SUNK_JOB_POOL.lock().unwrap().clear();
    RADAR_JOB_POOL.lock().unwrap().clear();
    INTEL_JOB_POOL.lock().unwrap().clear();
    WEATHER_JOB_POOL.lock().unwrap().clear();
    CAPITULATION_JOB_POOL.lock().unwrap().clear();
    let bytes = kit::fixture();
    let mut air = AirStatsJob::new();
    assert!(
        crate::extended::air::copy_air_stats_refresh_growing(
            &kit::memory(&bytes),
            AirStatsCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour: 0,
                force_full: true,
            },
            &mut air,
        )
        .raw_ok
    );
    let air_allocation = air.log.bytes.as_ptr();
    let job = DecodeJob {
        queued_qpc: 0,
        index,
        game_elapsed: 17,
        relations_payload: Some(RelationsJob::default()),
        characters_payload: [Some(CharactersJob::default()), None, None, None],
        projects_payload: Some(ProjectsJob::default()),
        lexical_payload: Some(LexicalJob::default()),
        divisions_payload: Some(DivisionsJob::default()),
        air_wing_payload: Some(AirWingJob::default()),
        air_stats_payload: Some(air),
        naval_payload: Some(NavalJob::default()),
        players_payload: Some(PlayersJob::default()),
        session_payload: Some(SessionJob::default()),
        production_rest_payload: Some(ProductionRestJob::default()),
        construction_rest_payload: Some(ConstructionRestJob::default()),
        economy_rest_payload: Some(EconomyRestJob::default()),
        air_bases_payload: Some(AirBasesJob::default()),
        supremacy_payload: Some(SupremacyJob::default()),
        sunk_payload: Some(SunkJob::default()),
        radar_payload: Some(RadarJob::default()),
        intel_payload: Some(IntelJob::default()),
        weather_payload: Some(WeatherJob::default()),
        capitulation_payload: Some(CapitulationJob::default()),
        ..Default::default()
    };
    // A callback can retire a refusal after an idle drain and then enqueue
    // this frame. Its retirement must be consumed before the lease returns.
    recycle_lexical_job(LexicalJob::default());
    let retirement = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    assert_eq!(retirement.len(), 1);
    assert_eq!(shared.ready.pop(), None);
    process_owned_job(&shared, job, &mut AirBasesMemo::default());
    assert!(retirement.is_empty());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 17);
    for (name, len) in [
        ("relations", RELATIONS_JOB_POOL.lock().unwrap().len()),
        ("characters", CHARACTERS_JOB_POOL.lock().unwrap()[0].len()),
        ("projects", PROJECTS_JOB_POOL.lock().unwrap().len()),
        ("lexical", LEXICAL_JOB_POOL.lock().unwrap().len()),
        ("divisions", DIVISIONS_JOB_POOL.lock().unwrap().len()),
        ("air wings", AIR_WING_JOB_POOL.lock().unwrap().len()),
        ("naval", NAVAL_JOB_POOL.lock().unwrap().len()),
        ("players", PLAYERS_JOB_POOL.lock().unwrap().len()),
        ("session", SESSION_JOB_POOL.lock().unwrap().len()),
        ("production", PRODUCTION_REST_JOB_POOL.lock().unwrap().len()),
        (
            "construction",
            CONSTRUCTION_REST_JOB_POOL.lock().unwrap().len(),
        ),
        ("economy", ECONOMY_REST_JOB_POOL.lock().unwrap().len()),
        ("air bases", AIR_BASES_JOB_POOL.lock().unwrap().len()),
        ("supremacy", SUPREMACY_JOB_POOL.lock().unwrap().len()),
        ("sunk ships", SUNK_JOB_POOL.lock().unwrap().len()),
        ("radar", RADAR_JOB_POOL.lock().unwrap().len()),
        ("intel", INTEL_JOB_POOL.lock().unwrap().len()),
        ("weather", WEATHER_JOB_POOL.lock().unwrap().len()),
        ("capitulation", CAPITULATION_JOB_POOL.lock().unwrap().len()),
    ] {
        assert_eq!(len, 0, "empty fixed {name} payload must be discarded");
    }
    assert_eq!(AIR_STATS_JOB_POOL.lock().unwrap().len(), 1);
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
    let memory = engine_native::memory::SliceMemory {
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
            AIR_STATS_JOB_POOL.lock().unwrap().len(),
            SESSION_JOB_POOL.lock().unwrap().len(),
            PRODUCTION_REST_JOB_POOL.lock().unwrap().len(),
            CONSTRUCTION_REST_JOB_POOL.lock().unwrap().len(),
            SUPREMACY_JOB_POOL.lock().unwrap().len(),
            SUNK_JOB_POOL.lock().unwrap().len(),
            RADAR_JOB_POOL.lock().unwrap().len(),
        ]
    };
    let idle_pools = daily_pool_lengths();
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    frame
        .extended
        .air_stats_pilot()
        .fail_air_stats_capture(replay_data::cadence::SKIPPED_REASON);
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
            ECONOMY_REST_JOB_POOL.lock().unwrap().len(),
        ]
    };
    let idle = pool_lengths();
    let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(16).unwrap());
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    for (hour, force_full, parts, economy) in [
        // CountryResources, its census and the exchanges are hourly; the
        // other economy tables publish in their daily slot.
        (0, false, [false, false], [true, false]),
        (3, false, [false, true], [true, false]),
        (8, false, [true, false], [true, false]),
        (19, false, [false, false], [true, false]),
        (-5, false, [false, false], [true, false]),
        (27, false, [false, true], [true, false]),
        (32, false, [true, false], [true, false]),
        (10, false, [false, false], [true, false]),
        (17, false, [false, false], [true, true]),
        (0, true, [true, true], [true, true]),
    ] {
        assert!(retired.is_empty());
        let reads = read_accounting::Capture::new();
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let request = sampling::acquire_world_request(&countries, 0, 0, hour, force_full);
        assert_eq!(
            [request.production.is_some(), request.construction.is_some(),],
            parts,
            "hour {hour}, force_full {force_full}"
        );
        // The economy rest is its own job, every hour.
        let economy_request = sampling::acquire_economy_request(
            &countries,
            0,
            0,
            frame.extended.economy_rest_pilot(),
            hour,
            force_full,
        );
        assert_eq!(
            [
                economy_request.context.resources_active,
                economy_request.context.other_active,
            ],
            economy
        );
        let owned = [parts[0], parts[1], true];
        let after_acquire: [usize; 3] =
            std::array::from_fn(|slot| idle[slot].saturating_sub(usize::from(owned[slot])));
        assert_eq!(pool_lengths(), after_acquire);
        assert!(retired.is_empty(), "absent parts must not retire a payload");
        assert_eq!(Arc::strong_count(&countries), 3);
        if let Some(part) = request.production {
            recycle_production_rest_job(part.payload);
        }
        if let Some(part) = request.construction {
            recycle_construction_rest_job(part.payload);
        }
        recycle_economy_rest_job(economy_request.payload);
        drop(request.countries);
        drop(economy_request.countries);
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
fn the_economy_rest_is_its_own_job_and_each_failure_refuses_only_its_tables() {
    use crate::extended::{economy, production};
    use engine_native::walk::Outcome;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    let countries = Arc::new(engine_native::country_memory::Cache::try_with_capacity(16).unwrap());
    let reason = "acquisition worker panicked";
    let world_tables = || PRODUCTION_REST.into_iter().chain(CONSTRUCTION_REST);
    for (world_failed, economy_failed) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        assert!(retired.is_empty());
        let mut captured =
            crate::extended::Captured::with_capacities(&crate::extended::Capacities::default());
        // A full hour: both World parts and every economy table are scheduled.
        let world = sampling::acquire_world_request(&countries, 0, 0, 0, true);
        let economy_request = sampling::acquire_economy_request(
            &countries,
            0,
            0,
            captured.economy_rest_pilot(),
            0,
            true,
        );
        let before = captured.outcomes();
        // Every raw copy completed; only the jobs themselves fail.
        let mut world_report = crate::acquisition_jobs::WorldReport {
            production: production::ProductionRestCopyReport {
                raw_ok: true,
                copy_ticks: [7; 8],
                ..Default::default()
            },
            construction: crate::extended::construction::ConstructionRestCopyReport {
                raw_ok: true,
                copy_ticks: [7; 10],
                ..Default::default()
            },
            ticks: 0,
        };
        let mut economy_report = economy::EconomyRestCopyReport {
            raw_ok: true,
            copy_ticks: [9; economy::N],
            ..Default::default()
        };
        let world_failure = world_failed.then_some(reason);
        let economy_failure = economy_failed.then_some(reason);
        let (mut production_payload, mut construction_payload, mut economy_payload) =
            (None, None, None);
        // The hour sets the construction verdict when it builds the World
        // request, before the Gated packs leave holding the construction
        // pack, so the World join installs only production's.
        captured.construction_rest_pilot().rest_raw_mode = world.construction.is_some();
        let active = sampling::settle_world(
            &mut captured,
            world,
            &mut world_report,
            world_failure,
            &mut production_payload,
            &mut construction_payload,
        );
        sampling::settle_economy(
            captured.economy_rest_pilot(),
            economy_request,
            &mut economy_report,
            economy_failure,
            &mut economy_payload,
        );
        assert_eq!(active, (true, true));
        // Each job's payloads go to decode or are recycled, never both.
        assert_eq!(production_payload.is_some(), !world_failed);
        assert_eq!(construction_payload.is_some(), !world_failed);
        assert_eq!(economy_payload.is_some(), !economy_failed);
        assert_eq!(
            retired.len(),
            2 * usize::from(world_failed) + usize::from(economy_failed)
        );
        assert_eq!(world_report.production.raw_ok, !world_failed);
        assert_eq!(world_report.construction.raw_ok, !world_failed);
        assert_eq!(economy_report.raw_ok, !economy_failed);
        let mut ticks = [0; replay_data::rows::TABLE_COUNT];
        sampling::refuse_world_rest(
            &mut captured,
            active,
            &world_report,
            world_failure,
            &mut ticks,
        );
        sampling::refuse_economy_rest(
            captured.economy_rest_pilot(),
            &economy_report,
            economy_failure,
            &mut ticks,
        );
        let outcomes = captured.outcomes();
        let refused = Outcome::Unavailable { reason };
        for table in world_tables() {
            let index = table.index();
            if world_failed {
                assert_eq!(outcomes[index], refused, "{table:?}");
                assert_eq!(ticks[index], 7, "{table:?}");
            } else {
                assert_eq!(outcomes[index], before[index], "{table:?}");
                assert_eq!(ticks[index], 0, "{table:?}");
            }
        }
        for table in ECONOMY_REST {
            let index = table.index();
            if economy_failed {
                assert_eq!(outcomes[index], refused, "{table:?}");
                assert_eq!(ticks[index], 9, "{table:?}");
            } else {
                assert_eq!(outcomes[index], before[index], "{table:?}");
                assert_eq!(ticks[index], 0, "{table:?}");
            }
        }
        // A refused family returns to its typed path; the other stays raw.
        assert_eq!(
            captured.production_rest_pilot().rest_raw_mode,
            !world_failed
        );
        assert_eq!(
            captured.construction_rest_pilot().rest_raw_mode,
            !world_failed
        );
        assert_eq!(captured.economy_rest_pilot().rest_raw_mode, !economy_failed);
        // Return every payload the decode slot kept, then drain the recycling.
        if let Some(job) = production_payload {
            recycle_production_rest_job(job);
        }
        if let Some(job) = construction_payload {
            recycle_construction_rest_job(job);
        }
        if let Some(job) = economy_payload {
            recycle_economy_rest_job(job);
        }
        assert_eq!(retired.len(), 3);
        reclaim_payloads();
        assert_eq!(Arc::strong_count(&countries), 1);
    }
    assert!(retired.is_empty());
}

#[test]
fn missing_declared_payloads_fail_only_the_frozen_active_tables() {
    use engine_native::walk::Outcome;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let characters = [true, false, false, true, false, true];
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
    process_owned_job(&shared, job, &mut AirBasesMemo::default());
    assert_eq!(shared.ready.pop(), Some(index));
    assert_eq!(shared.ready.pop(), None);
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 37);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let missing = Outcome::Unavailable {
        reason: MISSING_PAYLOAD_REASON,
    };
    let skipped = Outcome::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON,
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
        [
            air[0],
            air[1],
            air[2],
            air[air_slot(replay_data::rows::TableId::AirAces)]
        ],
        [missing, missing, skipped, skipped]
    );
    assert_eq!(frame.extended.radar_pilot().outcomes()[5], missing);
    let naval = frame.extended.naval_pilot().outcomes();
    // LandTheatres headers are hourly; the theatre details are daily.
    assert_eq!(naval[4], missing);
    assert_eq!(naval[5..10], [skipped; 5]);
    assert_eq!(naval[14..17], [missing; 3]);
    assert_eq!(calibration().refresh_jobs, 0);
}

#[test]
fn missing_division_payload_publishes_hourly_census_reason_even_without_details() {
    use engine_native::walk::Outcome;
    use replay_data::rows::TableId;

    struct NoReads;
    impl engine_native::Memory for NoReads {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("missing division payload must not retry native reads");
        }
    }
    let mut frame = sampling::Captured::new();
    for details_active in [false, true] {
        frame.extended.reset();
        let hour = (0..24)
            .find(|&hour| {
                replay_data::cadence::should_walk(TableId::DivisionEquipment, hour, false)
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
                        reason: replay_data::cadence::SKIPPED_REASON,
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
            assert!(replay_data::cadence::is_sliced(table));
            assert!(replay_data::cadence::should_walk(table, hour, false));
        }
        assert!(relations_refresh_hour(hour, true), "forced hour {hour}");
    }
}

#[test]
fn characters_gate_walks_the_roster_roles_and_traits_every_hour() {
    for hour in -48..48i32 {
        assert!(characters_refresh_hour(hour, false), "hour {hour}");
        for table in CHARACTERS {
            // Owner-approved training progress is daily slot 12; the roster and traits stay hourly.
            let expected = table != TableId::UnitLeaderTraitExperience
                || hour.rem_euclid(24) == replay_data::cadence::COMMANDER_PROGRESS_SLOT as i32;
            assert_eq!(
                replay_data::cadence::should_walk(table, hour, false),
                expected,
                "{table:?} at hour {hour}"
            );
            assert!(replay_data::cadence::should_walk(table, hour, true));
        }
        assert!(characters_refresh_hour(hour, true), "forced hour {hour}");
    }
}

#[test]
fn projects_gate_matches_their_daily_slot_and_force_full() {
    let slot = replay_data::cadence::refresh_slot(PROJECTS[0]).unwrap() as i32;
    for hour in -48..48i32 {
        let refresh = hour.rem_euclid(24) == slot;
        assert_eq!(projects_refresh_hour(hour, false), refresh, "hour {hour}");
        assert!(projects_refresh_hour(hour, true), "forced hour {hour}");
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
            queued_qpc: 0,
            index: *index,
            seq: 100 + order as u64,
            hour: 200 + order as i32,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
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
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
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
            queued_qpc: 0,
            index: 0,
            seq: seq as u64,
            hour: 0,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
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
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
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
        queued_qpc: 0,
        index: 0,
        seq: u64::MAX,
        hour: 0,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    // The fixed reference holds the complete two-row fixture.
    let direct = relations_test_kit::direct(&memory, 2, 2);
    let (report, first) = relations_test_kit::split(&memory, frame.extended.relations_pilot());
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 1,
        relations_refresh: true,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    use engine_native::walk::Outcome;
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
        let mut direct = kit::direct(&memory, 2, 2);
        if !opinions_active {
            direct.opinion_rows.clear();
            direct.opinion_outcome = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            };
        }
        let mut payload = RelationsJob::new();
        let report = intel::copy_relations_refresh(
            &memory,
            RelationsCopyContext {
                base: kit::BASE,
                game_state: kit::GS,
                active: [opinions_active, true],
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
        rows: [4096; crate::extended::characters::N],
        bytes: [131_072; crate::extended::characters::N],
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
        active: [true; crate::extended::characters::N],
        trait_rows,
        trait_bytes,
    };
    let mut first = CharactersJob::new();
    let report = crate::extended::characters::copy_characters_growing(
        &memory,
        ctx,
        &mut first,
        crate::extended::characters::Roles::Independent,
    );
    assert!(report.raw_ok);
    frame.extended.characters_pilot().raw_active = [true; crate::extended::characters::N];
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 0,
        relations_refresh: false,
        characters_refresh: [true, false, false, false],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
        rows: [4096; crate::extended::characters::N],
        bytes: [131_072; crate::extended::characters::N],
    };
    for (missing, missing_mask) in CHARACTER_MASKS.into_iter().enumerate() {
        let bytes = kit::mapped_fixture();
        let memory = kit::memory(&bytes);
        characters::advisors::note_modifier_demand(
            engine_native::cadvisor::modifier_demand(&memory, kit::BASE, kit::GAME_STATE).unwrap(),
        );
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let captured = frame.extended.characters_pilot();
        *captured = characters::Captured::with_capacities(&capacities);
        captured.set_character_activity([true; crate::extended::characters::N]);
        captured.raw_active = [true; crate::extended::characters::N];
        captured.capture(&memory, kit::BASE, kit::GAME_STATE, 0, false);
        let (trait_rows, trait_bytes) = captured.trait_capture_capacities();
        let mut job = bare_job(index, next_seq(), 0, 37);
        job.characters_refresh = [true; CHARACTER_PARTS];
        for (part, payload) in job.characters_payload.iter_mut().enumerate() {
            if part == missing {
                continue;
            }
            let mut owned = CharactersJob::new_part(part);
            let report = characters::copy_characters_growing(
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
                crate::extended::characters::Roles::Independent,
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
        assert_eq!(
            frame.extended.characters_pilot().raw_active,
            [false; crate::extended::characters::N]
        );
    }
}

#[test]
fn worker_characters_mixed_payload_matches_native_with_frozen_activity() {
    use crate::extended::characters::{self, CharactersCopyContext, CharactersJob};
    use characters::characters_test_kit as kit;
    use engine_native::walk::Outcome;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::mapped_fixture();
    let memory = kit::memory(&bytes);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let capacities = characters::Capacities {
        rows: [4096; crate::extended::characters::N],
        bytes: [131_072; crate::extended::characters::N],
    };
    for (hour, force_full) in [(0, false), (21, false), (7, true)] {
        let active =
            CHARACTERS.map(|table| replay_data::cadence::should_walk(table, hour, force_full));
        let mut direct = characters::Captured::with_capacities(&capacities);
        direct.capture(&memory, kit::BASE, kit::GAME_STATE, hour, force_full);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        *frame.extended.characters_pilot() = characters::Captured::with_capacities(&capacities);
        let (trait_rows, trait_bytes) = direct.trait_capture_capacities();
        let mut payload = CharactersJob::new();
        let report = characters::copy_characters_growing(
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
            crate::extended::characters::Roles::Independent,
        );
        assert!(report.raw_ok);
        for (slot, is_active) in active.into_iter().enumerate() {
            if !is_active {
                assert_eq!(report.copy_ticks[slot], 0);
                assert_eq!(report.raw_bytes[slot], 0);
            }
        }
        frame.extended.characters_pilot().raw_active = [true; crate::extended::characters::N];
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
                        reason: replay_data::cadence::SKIPPED_REASON
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
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 3,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: true,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 3,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: true,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    let direct = wars_test_kit::direct(&memory, 2);
    {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        frame.wars_result = direct.owned_outcome;
        frame.wars.clone_from(&direct.owned);
        frame.table_ns[TableId::WarRelations.index()] = 601;
    }
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 41,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    use crate::sampling::{DivisionsCopyContext, DivisionsJob, divisions_test_kit};
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
    let report = copy_divisions_worker_refresh(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    frame.extended.test_division_detail().groups.rows = Vec::new();
    let before_bytes = frame.extended.allocated_bytes();
    let pool_before = shared.pool_bytes.load(Ordering::Relaxed);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 42,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: true,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
        &frame.divisions[..direct.divisions_outcome.len()],
        &direct.divisions[..direct.divisions_outcome.len()]
    );
    let detail = frame.extended.test_division_detail();
    assert_eq!(detail.division_equipment.outcome, direct.equipment_outcome);
    assert_eq!(
        &detail.division_equipment.rows[..direct.equipment_outcome.len()],
        &direct.equipment[..direct.equipment_outcome.len()]
    );
    assert_eq!(detail.groups.outcome, direct.groups_outcome);
    assert_eq!(
        &detail.groups.rows[..direct.groups_outcome.len()],
        &direct.groups[..direct.groups_outcome.len()]
    );
    assert_eq!(detail.requests.outcome, direct.requests_outcome);
    assert_eq!(
        &detail.requests.rows[..direct.requests_outcome.len()],
        &direct.requests[..direct.requests_outcome.len()]
    );
    assert_eq!(detail.deliveries.outcome, direct.deliveries_outcome);
    assert_eq!(
        &detail.deliveries.rows[..direct.deliveries_outcome.len()],
        &direct.deliveries[..direct.deliveries_outcome.len()]
    );
    assert_eq!(detail.produced.outcome, direct.produced_outcome);
    assert_eq!(
        &detail.produced.rows[..direct.produced_outcome.len()],
        &direct.produced[..direct.produced_outcome.len()]
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
    use crate::sampling::{DivisionsCopyContext, divisions_test_kit as kit};
    use engine_native::walk::Outcome;

    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    let skipped = Outcome::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON,
    };
    frame
        .extended
        .fail_division_details(replay_data::cadence::SKIPPED_REASON);
    frame.extended.set_divisions_raw_mode(true);
    let detail_pointer = frame
        .extended
        .test_division_detail()
        .division_equipment
        .rows
        .as_ptr();
    let pool_len = DIVISIONS_JOB_POOL.lock().unwrap().len().max(1);
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
    let report = copy_divisions_worker_refresh(&memory, context, &mut payload);
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
            DivisionsCopyContext, copy_divisions_core_refresh,
            copy_divisions_inputs_refresh_growing, divisions_test_kit as kit,
        },
    };
    use engine_native::{Memory, memory::CountedMemory, memory::ReadCounts, walk::Outcome};
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
            let fault = kit::GAME_STATE + engine_native::cgame_state::PROVINCES;
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
    let pool_len = DIVISIONS_JOB_POOL.lock().unwrap().len().max(1);
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
        assert!(copy_divisions_worker_refresh(&kit::memory(&bytes), context, &mut payload).raw_ok);
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
        assert!(copy_divisions_inputs_refresh_growing(
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
                    reason: replay_data::cadence::SKIPPED_REASON
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
fn cold_worker_payload_grows_from_native_headers_before_publication_and_reuses_storage() {
    use crate::extended::air::{AirStatsCopyContext, Capacities, stats_test_kit as kit};
    use engine_native::walk::Outcome;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let saved = std::mem::take(&mut *AIR_STATS_JOB_POOL.lock().unwrap());
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled frame");
    let context = AirStatsCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
    };
    let mut fixed = AirStatsJob::default();
    let bytes = kit::mod_fixture(5_000);
    assert!(
        !crate::extended::air::copy_air_stats_refresh(&kit::memory(&bytes), context, &mut fixed,)
            .raw_ok
    );
    assert!(fixed.log.overflowed);
    drop(bytes);

    let mut retained = None;
    for (step, count) in [5_000, 5_000, 11_003].into_iter().enumerate() {
        let mut payload = if step == 0 {
            AirStatsJob::new()
        } else {
            acquire_air_stats_job()
        };
        if let Some((bytes, span_capacity, byte_capacity)) = retained {
            assert_eq!(payload.log.bytes.as_ptr(), bytes);
            assert!(payload.log.spans.capacity() >= span_capacity);
            assert!(payload.log.bytes.capacity() >= byte_capacity);
        } else {
            assert_eq!(payload.log.spans.capacity(), 0);
            assert_eq!(payload.log.bytes.capacity(), 0);
        }
        let bytes = kit::mod_fixture(count);
        let memory = kit::memory(&bytes);
        let mut capacities = Capacities::default();
        capacities.rows[air_slot(replay_data::rows::TableId::AirStatistics)] = count;
        let direct = kit::direct(&memory, &capacities);
        assert_eq!(direct.outcome, Outcome::Rows { len: count });
        let report =
            crate::extended::air::copy_air_stats_refresh_growing(&memory, context, &mut payload);
        assert!(report.raw_ok);
        assert!(!payload.log.overflowed);
        retained = Some((
            payload.log.bytes.as_ptr(),
            payload.log.spans.capacity(),
            payload.log.bytes.capacity(),
        ));
        // Publication and decoding use only the owned original observations.
        drop(bytes);
        assert_eq!(shared.ready.pop(), None);
        let mut job = bare_job(index, next_seq(), 0, step as i64 + 1);
        job.force_full = true;
        job.air_stats_refresh = true;
        job.air_stats_payload = Some(payload);
        process_owned_job(&shared, job, &mut AirBasesMemo::default());
        assert_eq!(shared.ready.pop(), Some(index));
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        kit::assert_captured_matches_direct(frame.extended.air_stats_pilot(), &direct);
        assert_eq!(AIR_STATS_JOB_POOL.lock().unwrap().len(), 1);
    }
    *AIR_STATS_JOB_POOL.lock().unwrap() = saved;
}

#[test]
fn queue_high_water_tracks_the_deepest_observed_fifo_occupancy() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(0, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    for _ in 0..3 {
        let job = DecodeJob {
            queued_qpc: 0,
            index,
            seq: next_seq(),
            hour: 0,
            relations_refresh: false,
            characters_refresh: [false; CHARACTER_PARTS],
            projects_refresh: false,
            lexical_refresh: false,
            divisions_refresh: false,
            air_wing_refresh: false,
            air_stats_refresh: false,
            naval_refresh: false,
            players_refresh: false,
            session_refresh: false,
            production_rest_refresh: false,
            construction_rest_refresh: false,
            economy_rest_refresh: false,
            air_bases_refresh: false,
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
            air_wing_payload: None,
            air_stats_payload: None,
            naval_payload: None,
            players_payload: None,
            session_payload: None,
            production_rest_payload: None,
            construction_rest_payload: None,
            economy_rest_payload: None,
            air_bases_payload: None,
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
    let report = crate::extended::air::copy_air_refresh_growing(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 9,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    let mut capacities = crate::extended::air::Capacities::default();
    capacities.rows[air_slot(replay_data::rows::TableId::AirStatistics)] = 2;
    let direct = stats_test_kit::direct(&memory, &capacities);
    let ctx = AirStatsCopyContext {
        base: stats_test_kit::BASE,
        game_state: stats_test_kit::GAME_STATE,
        hour: 0,
        force_full: true,
    };
    let mut first = acquire_air_stats_job();
    let report = crate::extended::air::copy_air_stats_refresh_growing(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 9,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: true,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: Some(first),
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    let report =
        crate::extended::naval_history::copy_naval_refresh_growing(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 9,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: true,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: Some(first),
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    let air_report =
        crate::extended::air::copy_air_refresh_growing(&air_memory, air_ctx, &mut air_job);
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
    let naval_report = crate::extended::naval_history::copy_naval_refresh_growing(
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
fn worker_decodes_a_production_rest_payload_into_the_same_typed_rows() {
    use crate::extended::production::{ProductionRestCopyContext, production_rest_test_kit as kit};
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let memory = engine_native::memory::SliceMemory {
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
        crate::extended::production::copy_production_rest_refresh_growing(&memory, ctx, &mut first);
    assert!(report.raw_ok);
    frame.extended.production_rest_pilot().rest_raw_mode = true;
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 11,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: true,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: Some(first),
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    let memory = engine_native::memory::SliceMemory {
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
    let report = crate::extended::construction::copy_construction_rest_refresh_growing(
        &memory, ctx, &mut first,
    );
    assert!(report.raw_ok);
    frame.extended.construction_rest_pilot().rest_raw_mode = true;
    shared.completed[index].store(0, Ordering::Relaxed);
    let mut job = DecodeJob {
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 12,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: true,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: Some(first),
        economy_rest_payload: None,
        air_bases_payload: None,
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
}

#[test]
fn worker_decodes_two_payloads_in_one_job_with_copy_only_table_ns() {
    // Two families decode in one process_job on one Shared. Both
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
    let air_report =
        crate::extended::air::copy_air_refresh_growing(&air_memory, air_ctx, &mut air_job);
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
    let naval_report = crate::extended::naval_history::copy_naval_refresh_growing(
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
        queued_qpc: 0,
        index,
        seq: next_seq(),
        hour: 9,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: true,
        air_stats_refresh: false,
        naval_refresh: true,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: Some(air_job),
        air_stats_payload: None,
        naval_payload: Some(naval_job),
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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

/// Build a FIFO job carrying only the selected family payload (all other
/// families off). Every worker test below uses this shape with its own
/// kit-staged payload.
fn bare_job(index: usize, seq: u64, hour: i32, game_elapsed: i64) -> DecodeJob {
    DecodeJob {
        queued_qpc: 0,
        index,
        seq,
        hour,
        relations_refresh: false,
        characters_refresh: [false; CHARACTER_PARTS],
        projects_refresh: false,
        lexical_refresh: false,
        divisions_refresh: false,
        air_wing_refresh: false,
        air_stats_refresh: false,
        naval_refresh: false,
        players_refresh: false,
        session_refresh: false,
        production_rest_refresh: false,
        construction_rest_refresh: false,
        economy_rest_refresh: false,
        air_bases_refresh: false,
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
        air_wing_payload: None,
        air_stats_payload: None,
        naval_payload: None,
        players_payload: None,
        session_payload: None,
        production_rest_payload: None,
        construction_rest_payload: None,
        economy_rest_payload: None,
        air_bases_payload: None,
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
    use engine_native::walk::Outcome;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let mut bytes = kit::fixture();
    let mut capacities = Capacities::default();
    capacities.rows[air_slot(replay_data::rows::TableId::AirStatistics)] = 2;
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let mut bases_memo = AirBasesMemo::default();
    for (step, hour) in [5, 29, 30, 53, 77, 101, 125, 149].into_iter().enumerate() {
        let mut job = bare_job(index, step as u64 + 1, hour, 41);
        job.air_stats_refresh = replay_data::cadence::should_walk(AIR_STATS[0], hour, false);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let captured = frame.extended.air_stats_pilot();
        captured.prepare_hour(hour, false);
        if job.air_stats_refresh && step != 5 {
            // Every scheduled payload contains a different current value;
            // earlier rows must not survive a skip or missing payload.
            kit::set_first_value(&mut bytes, -(step as i64 + 1));
            let memory = kit::memory(&bytes);
            let mut payload = AirStatsJob::new();
            let report = crate::extended::air::copy_air_stats_refresh_growing(
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
            // The pack reset must preserve the staged-payload reservation.
            captured.capture(
                &memory,
                kit::BASE,
                kit::GAME_STATE,
                hour,
                false,
                Some(&mut Default::default()),
            );
            job.air_stats_payload = Some(payload);
        } else if !job.air_stats_refresh {
            captured.fail_air_stats_capture(replay_data::cadence::SKIPPED_REASON);
        }
        process_test_job(&shared, &mut job, &mut bases_memo);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 41);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let captured = frame.extended.air_stats_pilot();
        if step == 2 || step == 5 {
            assert_eq!(
                captured.outcomes()[air_slot(replay_data::rows::TableId::AirStatistics)],
                Outcome::Unavailable {
                    reason: if step == 2 {
                        replay_data::cadence::SKIPPED_REASON
                    } else {
                        MISSING_PAYLOAD_REASON
                    },
                }
            );
        } else {
            let direct = kit::direct(&kit::memory(&bytes), &capacities);
            kit::assert_captured_matches_direct(captured, &direct);
        }
        assert!(!captured.stats_raw_mode);
    }
}

#[test]
fn worker_air_bases_memo_invalidates_on_skip_missing_payload_and_ace_mask_changes() {
    use crate::extended::air::{
        AirBasesCopyContext, Capacities, Captured, air_bases_test_kit as kit,
    };
    use engine_native::walk::Outcome;
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let bytes = kit::fixture();
    let shared = Shared::new(kit::BASE, 0, 1);
    let index = shared.free.pop().expect("pooled buffer");
    let reference = kit::direct(&kit::memory(&bytes));
    let mut capacities = Capacities::default();
    capacities.rows[1] = reference.bases_outcome.len();
    capacities.rows[air_slot(replay_data::rows::TableId::AirAces)] = reference.aces_outcome.len();
    capacities.ace_names = reference.ace_names.len();
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    *frame.extended.air_bases_pilot() = Captured::with_capacities(&capacities);
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
            let report = crate::extended::air::copy_air_bases_refresh_growing(
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
                frame.extended.air_bases_pilot().outcomes()
                    [air_slot(replay_data::rows::TableId::AirAces)],
                Outcome::Unavailable {
                    reason: replay_data::cadence::SKIPPED_REASON,
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
    use replay_frame::frame::TableView;
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
    let report = crate::extended::air::copy_air_bases_refresh_growing(&memory, ctx, &mut first);
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
    assert!(matches!(
        air[air_slot(replay_data::rows::TableId::AirAces)],
        TableView::Complete(_)
    ));
    assert!(matches!(fog[5], TableView::Complete(_)));
    assert_eq!(
        frame.extended.air_bases_pilot().outcomes()[1],
        direct.bases_outcome
    );
    assert_eq!(
        frame.extended.air_bases_pilot().outcomes()[air_slot(replay_data::rows::TableId::AirAces)],
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
fn worker_decodes_a_supremacy_payload_into_the_same_typed_rows() {
    use crate::extended::naval_history::supremacy_test_kit as kit;
    use replay_frame::frame::TableView;
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
    let report =
        crate::extended::naval_history::copy_supremacy_refresh_growing(&memory, ctx, &mut first);
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
    use replay_frame::frame::TableView;
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
    let report =
        crate::extended::naval_history::copy_sunk_refresh_growing(&memory, ctx, &mut first);
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
    assert_eq!(
        direct.outcome,
        engine_native::walk::Outcome::Rows { len: 2 }
    );
    let naval = frame.extended.sunk_pilot().tables();
    let entries = match &naval[13] {
        TableView::Complete(replay_data::rows::TableRows::SunkShips(rows)) => {
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
    use replay_frame::frame::TableView;
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
    let report = crate::extended::fog::copy_radar_refresh_growing(&memory, ctx, &mut first);
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
    use replay_frame::frame::TableView;
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
    let report = crate::extended::fog::copy_intel_refresh_with_divisions_growing(
        &memory, ctx, &mut first, None,
    );
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
    let snapshot = calibration();
    assert_eq!(snapshot.air_bases_copy_ns, [11, 0, 7]);
    assert_eq!(snapshot.air_bases_raw_bytes, [0, 100, 0]);
    assert_eq!(snapshot.supremacy_copy_ns, [3]);
    assert_eq!(snapshot.sunk_copy_ns, [4]);
    assert_eq!(snapshot.radar_copy_ns, [6]);
    assert_eq!(snapshot.intel_copy_ns, [8]);
    let json = snapshot.json();
    assert_eq!(json["air_bases_copy_ns"][0], 11);
    assert_eq!(json["air_bases_tables"][1], "AirBaseRegistrations");
    assert_eq!(json["supremacy_tables"][0], "NavalSupremacy");
    assert_eq!(json["sunk_tables"][0], "SunkShips");
    assert_eq!(json["radar_tables"][0], "RadarCoverage");
    assert_eq!(json["intel_tables"][0], "UnitIntel");
}

pub(crate) fn retire_and_reacquire_two_carrier_jobs_for_test(
    jobs: [AirBasesJob; 2],
) -> [AirBasesJob; 2] {
    let _guard = TEST_LOCK.lock().unwrap();
    prepare_retirement();
    reclaim_payloads();

    struct RestorePool(Vec<AirBasesJob>);
    impl Drop for RestorePool {
        fn drop(&mut self) {
            // Reclaim our outstanding ownership even if an assertion unwinds.
            // Other pool tests cannot enter while TEST_LOCK is held.
            reclaim_payloads();
            *AIR_BASES_JOB_POOL
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = std::mem::take(&mut self.0);
        }
    }
    let _restore = RestorePool(std::mem::take(&mut AIR_BASES_JOB_POOL.lock().unwrap()));
    let identities = jobs.each_ref().map(|job| job.log.spans.as_ptr());
    for job in jobs {
        recycle_air_bases_job(job);
    }
    let retired = unsafe { &*RETIRED_PAYLOADS.load(Ordering::Acquire) };
    assert_eq!(
        retired.len(),
        2,
        "both queued payloads await worker retirement"
    );
    assert!(AIR_BASES_JOB_POOL.lock().unwrap().is_empty());

    // The actual worker release grows, clears and pools each retained job.
    reclaim_payloads();
    assert!(retired.is_empty());
    assert_eq!(AIR_BASES_JOB_POOL.lock().unwrap().len(), 2);
    let second = acquire_air_bases_job();
    let first = acquire_air_bases_job();
    assert!(AIR_BASES_JOB_POOL.lock().unwrap().is_empty());
    assert_eq!(first.log.spans.as_ptr(), identities[0]);
    assert_eq!(second.log.spans.as_ptr(), identities[1]);
    [first, second]
}

/// The hourly body fills its job where the worker reads it: a reservation
/// publishes in place, and an unpublished one returns its slot vacant.
#[test]
fn a_reserved_job_publishes_in_place_and_an_unpublished_one_returns_vacant() {
    let _guard = TEST_LOCK.lock().unwrap();
    reset_calibration_for_test();
    let shared = Shared::new(0, 0, 1);
    let queue = &shared.decode_queue;
    let mut reserved = queue.reserve().expect("free decode slot");
    reserved.job().players_payload = Some(PlayersJob::new());
    reserved.job().seq = 5;
    drop(reserved);
    let mut every: Vec<_> = (0..BUFFERS)
        .map(|_| queue.reserve().expect("free decode slot"))
        .collect();
    assert!(queue.reserve().is_none());
    assert!(
        every
            .iter_mut()
            .all(|reserved| reserved.job().players_payload.is_none())
    );
    drop(every);
    let jobs = JOBS.load(Ordering::Relaxed);
    let mut reserved = queue.reserve().expect("free decode slot");
    let job = reserved.job();
    job.index = 1;
    job.seq = 9;
    job.hour = 4;
    job.players_payload = Some(PlayersJob::new());
    assert!(enqueue_reserved(&shared, reserved));
    assert_eq!(JOBS.load(Ordering::Relaxed), jobs + 1);
    assert_eq!(queue.len(), 1);
    let job = queue.pop().expect("published job");
    assert_eq!((job.index, job.seq, job.hour), (1, 9, 4));
    assert!(job.players_payload.is_some());
    assert!(queue.is_empty());
    let every: Vec<_> = (0..BUFFERS).filter_map(|_| queue.reserve()).collect();
    assert_eq!(every.len(), BUFFERS);
}

/// The air pack's slot of `table`, so fixtures follow the pack's order.
fn air_slot(table: replay_data::rows::TableId) -> usize {
    crate::extended::air::TABLES
        .iter()
        .position(|&candidate| candidate == table)
        .expect("an air pack table")
}
#[test]
fn project_variables_survive_production_fifo_recycling_and_terminal_decode_denial() {
    const CHILD: &str = "BATTLE_PLAN_PROJECT_VARIABLE_FIFO_TEST";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "decode::tests::project_variables_survive_production_fifo_recycling_and_terminal_decode_denial", "--nocapture"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    use collector_observers::project_variable_changes as changes;
    use collector_projects::projects::variable_test_kit as kit;
    changes::enabled(true);
    changes::project_hooks_enabled(true);
    crate::allocation::reset();
    let shared = Shared::new(0, 0, 1);
    let memory = kit::Fixture::new(0x1000, 7);
    // Hold every free lease so recycling must return the exact frame below.
    let leases = (0..shared.free.len())
        .map(|_| shared.free.pop().unwrap())
        .collect::<Vec<_>>();
    let mut expected = Vec::new();
    for (&index, seed) in leases[..2].iter().zip([7, 9]) {
        memory.written(seed);
        expected.push(kit::reference(&memory));
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        kit::acquire(frame.extended.projects_pilot(), &memory, false);
        assert!(kit::variables(frame.extended.projects_pilot()).is_none());
        assert!(
            enqueue(
                &shared,
                bare_job(index, u64::from(seed), seed as i32, i64::from(seed))
            )
            .is_none()
        );
    }
    memory.replace(11);
    memory.refuse.set(true);
    let reads = memory.reads.get();
    let mut memo = AirBasesMemo::default();
    for (&index, bytes) in leases[..2].iter().zip(&expected) {
        let job = shared.decode_queue.pop().expect("production FIFO");
        assert_eq!(job.index, index);
        process_owned_job(&shared, job, &mut memo);
        assert_eq!(shared.ready.pop(), Some(index));
        assert!(shared.completed[index].load(Ordering::Acquire) > 0);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(kit::encoded(frame.extended.projects_pilot()), *bytes);
        assert_eq!(
            kit::variables(frame.extended.projects_pilot())
                .unwrap()
                .entries
                .len(),
            3
        );
        shared.recycle_frame_prepared(index);
    }
    assert!(shared.decode_queue.is_empty() && shared.ready.is_empty());
    assert_eq!(memory.reads.get(), reads);
    let index = shared.free.pop().unwrap();
    assert_eq!(index, leases[0]);
    shared.completed[index].store(0, Ordering::Relaxed);
    memory.refuse.set(false);
    memory.written(13);
    let bytes = kit::reference(&memory);
    {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        kit::acquire(frame.extended.projects_pilot(), &memory, false);
    }
    assert!(enqueue(&shared, bare_job(index, 13, 13, 13)).is_none());
    memory.replace(15);
    memory.refuse.set(true);
    let reads = memory.reads.get();
    process_owned_job(&shared, shared.decode_queue.pop().unwrap(), &mut memo);
    assert_eq!(shared.ready.pop(), Some(index));
    {
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        assert_eq!(kit::encoded(frame.extended.projects_pilot()), bytes);
    }
    assert_eq!(memory.reads.get(), reads);
    // Reacquiring the same frame must not expose its previous successful store.
    shared.recycle_frame_prepared(index);
    let recycled = shared.free.pop().unwrap();
    assert_eq!(recycled, leases[1]);
    shared.completed[recycled].store(0, Ordering::Relaxed);
    let frame = unsafe { &mut *shared.frames[recycled].load(Ordering::Acquire) };
    kit::acquire(frame.extended.projects_pilot(), &memory, true);
    assert!(enqueue(&shared, bare_job(recycled, 16, 16, 16)).is_none());
    process_owned_job(&shared, shared.decode_queue.pop().unwrap(), &mut memo);
    assert_eq!(shared.ready.pop(), Some(recycled));
    let frame = unsafe { &mut *shared.frames[recycled].load(Ordering::Acquire) };
    assert!(kit::variables(frame.extended.projects_pilot()).is_none());
    shared.recycle_frame_prepared(recycled);
    let index = shared.free.pop().unwrap();
    assert_eq!(index, leases[0]);
    shared.completed[index].store(0, Ordering::Relaxed);
    memory.refuse.set(false);
    memory.written(17);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    kit::acquire(frame.extended.projects_pilot(), &memory, false);
    assert!(enqueue(&shared, bare_job(index, 17, 17, 17)).is_none());
    memory.replace(19);
    memory.refuse.set(true);
    let job = shared.decode_queue.pop().unwrap();
    crate::allocation::install_hooks();
    let denied = crate::test_support::NullGuard::new(1);
    process_owned_job(&shared, job, &mut memo);
    drop(denied);
    assert!(crate::test_support::DENIALS.load(Ordering::Relaxed) > 0);
    assert!(crate::allocation::failed() && engine_allocation::capture_stopped());
    let failure = crate::allocation::failure().unwrap();
    assert_eq!(failure.phase, crate::allocation::Phase::Decode as u8);
    assert_eq!(failure.code, crate::allocation::Code::Reserve as u8);
    assert!(shared.ready.is_empty() && shared.decode_queue.is_empty());
    assert_eq!(shared.completed[index].load(Ordering::Acquire), 0);
    let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
    assert!(kit::variables(frame.extended.projects_pilot()).is_none());
    assert_eq!(
        kit::pending(frame.extended.projects_pilot()),
        0,
        "production decode reached freezing"
    );
    assert_ne!(shared.free.pop(), Some(index));
    engine_allocation::set_hooks(None, None);
    engine_allocation::set_stop_hook(None);
}
