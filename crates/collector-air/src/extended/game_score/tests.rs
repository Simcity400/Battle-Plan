use super::*;
use engine_native_air::cair_wing;
use engine_native::memory::SliceMemory;
use replay_data::rows::TableRows;

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put64(bytes, at, data as u64);
    put32(bytes, at + 8, count);
    put32(bytes, at + 12, count);
}

#[test]
fn game_score_projection_keeps_countries_independent_values_and_ordered_witnesses() {
    const COUNT: usize = 3;
    let mut capacity = Capacities::default();
    capacity.raise_for_countries(COUNT + 1);
    capacity.fleets = 3;
    capacity.sites = 2;
    let mut capture = Captured::with_capacities(&capacity);
    for (index, row) in capture.countries.rows.iter_mut().take(COUNT).enumerate() {
        *row = native::CountryRow {
            country: 0x4000_0000 + index as u32,
            canonical_slot: index as u32 + 1,
            army_count: Ok(-(index as i32)),
            navy_count: Ok(0),
            land_count: Ok(index as i32),
            fleet_start: 0,
            fleet_len: 0,
        };
        capture.air.countries[index + 1].read = native::AirRead {
            state: native::AirState::Absent,
            error: "",
            len: 0,
            required: 0,
        };
    }
    capture.countries.rows[0].army_count = Err("game score army roster count unreadable");
    capture.countries.rows[0].fleet_len = 3;
    capture.fleets.first.copy_from_slice(&[
        native::FleetCount {
            fleet: 61,
            task_forces: Ok(i32::MAX),
        },
        native::FleetCount {
            fleet: 62,
            task_forces: Ok(2),
        },
        native::FleetCount {
            fleet: 61,
            task_forces: Ok(i32::MAX),
        },
    ]);
    capture.fleets.len = 3;
    capture.air.countries[1] = AirCountry {
        start: 0,
        read: native::AirRead {
            state: native::AirState::Present,
            error: "",
            len: 2,
            required: 2,
        },
    };
    capture.air.sites.first.copy_from_slice(&[
        native::AirSite {
            base: 65,
            access: Ok(true),
        },
        native::AirSite {
            base: 65,
            access: Err("game score air-site access unreadable"),
        },
    ]);
    capture.air.sites.len = 2;
    capture.countries.outcome = Outcome::Rows { len: COUNT };
    capture.country_len = COUNT;
    let [
        TableView::Complete(TableRows::CountryGameScoreInputs(rows)),
        _,
        _,
    ] = capture.tables()
    else {
        panic!("all recorded countries must be available");
    };
    assert_eq!(rows.len(), COUNT);
    assert_eq!(
        (rows[0].army_count, rows[0].navy_count, rows[0].land_count),
        (None, Some(0), Some(0))
    );
    assert_eq!(
        rows[0].army_error,
        b"game score army roster count unreadable"
    );
    assert_eq!(
        rows[0]
            .fleets
            .iter()
            .map(|row| row.fleet)
            .collect::<Vec<_>>(),
        [61, 62, 61]
    );
    assert_eq!(
        rows[0]
            .first_sites
            .iter()
            .map(|row| row.base)
            .collect::<Vec<_>>(),
        [65, 65]
    );
    assert_eq!(
        rows[0].first_sites[1].access,
        rows::GameScoreAirSite::ACCESS_UNKNOWN
    );
    assert_eq!(rows[COUNT - 1].country, 0x4000_0000 + COUNT as u32 - 1);
    assert_eq!(rows[COUNT - 1].army_count, Some(1 - COUNT as i32));
    capture.countries.outcome = Outcome::Truncated {
        len: COUNT,
        unread_rows: 3,
    };
}

#[test]
fn game_score_air_census_is_unknown_until_finished_and_short_storage_is_named() {
    let mut captured = Captured::with_capacities(&Capacities {
        countries: 1,
        ..Default::default()
    });
    captured.reset();
    assert!(
        matches!(captured.tables()[1], TableView::Unavailable { ref reason }
        if reason == "game score air census not run")
    );
    captured.air.finish(Outcome::Rows { len: 0 });
    assert!(
        matches!(captured.tables()[1], TableView::Complete(TableRows::AirWingGameScoreStats(ref rows))
        if rows.is_empty())
    );
    captured.air.begin(captured.enabled);
    assert!(
        matches!(captured.tables()[1], TableView::Unavailable { ref reason }
        if reason == "game score air census not run")
    );
    captured.countries.rows[0] = native::CountryRow {
        country: 5,
        canonical_slot: 2,
        army_count: Ok(0),
        navy_count: Ok(0),
        land_count: Ok(0),
        ..Default::default()
    };
    captured.countries.outcome = Outcome::Rows { len: 1 };
    captured.country_len = 1;
    let [
        TableView::Complete(TableRows::CountryGameScoreInputs(rows)),
        _,
        _,
    ] = captured.tables()
    else {
        panic!("independent country operands remain available");
    };
    assert_eq!(
        rows[0].air_error,
        b"game score air-country storage exhausted"
    );
    assert_eq!(
        rows[0].air_state,
        rows::CountryGameScoreInputRow::AIR_UNKNOWN
    );
    assert_eq!(
        [
            native::AirState::Unknown,
            native::AirState::Absent,
            native::AirState::Present
        ]
        .map(replay_air_state),
        [
            rows::CountryGameScoreInputRow::AIR_UNKNOWN,
            rows::CountryGameScoreInputRow::AIR_ABSENT,
            rows::CountryGameScoreInputRow::AIR_PRESENT
        ]
    );
}

#[test]
fn game_score_test_hints_are_scoped_without_changing_process_hints() {
    let before = [
        SITES_HINT.load(Ordering::Relaxed),
        WINGS_HINT.load(Ordering::Relaxed),
    ];
    {
        let _hints = TestHints::new();
        assert!(note_arena_hints(3, 7));
        assert!(!note_arena_hints(2, 6));
        assert_eq!(arena_hints(), [3, 7]);
        {
            let _nested = TestHints::new();
            note_arena_hints(11, 13);
            assert_eq!(arena_hints(), [11, 13]);
        }
        assert_eq!(arena_hints(), [3, 7]);
        let joined = std::thread::spawn(arena_hints).join().unwrap();
        assert_eq!(joined, before);
    }
    assert_eq!(
        [
            SITES_HINT.load(Ordering::Relaxed),
            WINGS_HINT.load(Ordering::Relaxed)
        ],
        before
    );
}

#[test]
fn game_score_live_demand_presizes_all_frame_and_wing_job_buffers() {
    use crate::extended::air::{self, AirWingCopyContext, AirWingJob, wing_test_kit as kit};
    let _hint = crate::sizing::hint_guard();
    let _arenas = TestHints::new();
    let mut bytes = kit::fixture();
    const COUNTRY: usize = 0x20000;
    const ENTRY: usize = 0x31000;
    const FLEET: usize = 0x3e100;
    const SITE: usize = 0x3f100;
    array(
        &mut bytes,
        COUNTRY + engine_native::ccountry::FLEETS,
        0x3e000,
        3,
    );
    for index in 0..3 {
        put64(&mut bytes, 0x3e000 + index * 8, FLEET as u64);
    }
    put64(
        &mut bytes,
        FLEET,
        engine_native::cfleet::VTABLE_RVA.0 as u64,
    );
    put64(&mut bytes, FLEET + engine_native::cfleet::HANDLE, 61);
    put32(
        &mut bytes,
        FLEET + engine_native::cfleet::TASK_FORCE_COUNT,
        7,
    );
    array(
        &mut bytes,
        ENTRY + engine_native_air::cstrategic_air::FIRST_AIR_SITES,
        0x3f000,
        1,
    );
    put64(&mut bytes, 0x3f000, SITE as u64);
    put64(
        &mut bytes,
        SITE,
        engine_native_air::cair_base::VTABLE_RVA.0 as u64,
    );
    put64(&mut bytes, SITE + 8, 65);
    put32(
        &mut bytes,
        SITE + engine_native_air::cair_base::COUNTRY_CONTAINERS_COUNT,
        1,
    );
    for wing in [kit::WING, 0x34000] {
        bytes[wing + cair_wing::MISSION_STATS + cair_wing::MISSION_STAT_INDEXES] = 255;
    }
    let memory = kit::memory(&bytes);
    let observed = native::arena_demand(&memory, 0, kit::GAME_STATE).unwrap();
    assert!(observed.failures.is_empty());
    let demand = observed.demand;
    assert_eq!(
        demand,
        native::ArenaDemand {
            countries: 3,
            fleets: 3,
            sites: 1,
            wings: 2
        }
    );
    let mut capacity = Capacities::default();
    capacity.raise(demand);
    for _ in 0..collector_extended_support::hourly::BUFFERS {
        let mut capture = Captured::with_capacities(&capacity);
        collector_test_support::with_allocations_denied(|| {
            capture.capture(&memory, 0, kit::GAME_STATE);
            capture.air.begin(capture.enabled);
            capture.air.owner(&memory, 0, 1, Some(ENTRY));
            capture.air.finish(Outcome::Rows { len: 0 });
        });
        assert_eq!(capture.countries.rows[0].navy_count, Ok(21));
        assert_eq!(
            (
                capture.air.countries[1].read.state,
                capture.air.countries[1].read.len
            ),
            (native::AirState::Present, 1)
        );
    }
    let context = AirWingCopyContext {
        base: 0,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
        limits: [2, 3, 2],
        game_score: true,
    };
    for _ in 0..3 {
        let mut job = AirWingJob::fixture();
        job.game_score = AirWingJob::with_game_score(true).game_score;
        let report = collector_test_support::with_allocations_denied(|| {
            air::copy_air_refresh(&memory, context, &mut job)
        });
        assert!(report.raw_ok);
        assert_eq!(job.game_score.wings.outcome, Outcome::Rows { len: 2 });
        assert_eq!(
            job.game_score.countries[1].read.state,
            native::AirState::Present
        );
        assert_eq!(job.game_score.sites.len, 1);
    }
}

#[test]
fn game_score_disable_performs_no_operand_reads_and_survives_country_loan() {
    struct NoRead;
    impl Memory for NoRead {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("disabled score read");
        }
    }
    let mut capacity = Capacities {
        enabled: false,
        ..Default::default()
    };
    capacity.raise_for_countries(5001);
    let mut capture = Captured::with_capacities(&capacity);
    assert_eq!(capture.allocated_bytes(), 0);
    capture.grow(&mut capacity);
    assert_eq!(capture.allocated_bytes(), 0);
    assert_eq!(capture.capture(&NoRead, 0, 0), [0; N]);
    capture.air.begin(capture.enabled);
    capture.air.owner(&NoRead, 0, 1, Some(0x1000));
    capture.air.wing(
        &NoRead,
        0x2000,
        &cair_wing::ScoreLeaf {
            id: 0,
            planes: Ok(0),
        },
    );
    capture.air.finish(Outcome::Rows { len: 1 });
    assert!(capture.tables().iter().all(|view| matches!(view, TableView::Unavailable { reason } if reason == GAME_SCORE_DISABLED_REASON)));
    let mut enabled = Captured::with_capacities(&Capacities {
        countries: 2,
        ..Default::default()
    });
    let mut spare = Captured::vacant();
    enabled.swap_country(&mut spare);
    enabled.air.begin(enabled.enabled);
    enabled.air.owner(&NoRead, 0, 1, None);
    assert!(spare.enabled);
    enabled.swap_country(&mut spare);
    assert_eq!(
        enabled.air.countries[1].read.state,
        native::AirState::Absent
    );
}

#[test]
fn game_score_disabled_wing_buffers_ignore_live_hints() {
    use crate::extended::air::AirWingJob;
    let _hint = crate::sizing::hint_guard();
    let _arenas = TestHints::new();
    collector_primitives::country_slots::note_slots(5001);
    note_arena_hints(7, 11);
    let mut job = AirWingJob::new();
    assert_eq!(job.game_score.allocated_bytes(), 0);
    job.presize_game_score();
    job.grow();
    assert_eq!(job.game_score.allocated_bytes(), 0);
    let mut enabled = AirWingJob::with_game_score(true);
    let before = enabled.game_score.allocated_bytes();
    assert!(before > 0);
    note_arena_hints(17, 19);
    enabled.presize_game_score();
    assert!(enabled.game_score.allocated_bytes() > before);
}

#[test]
fn game_score_owned_air_shares_census_and_preserves_independent_failures() {
    use crate::extended::air::{self, AirWingCopyContext, AirWingJob, wing_test_kit as kit};
    let mut bytes = kit::fixture();
    const ENTRY: usize = 0x31000;
    const BASE: usize = 0x3a000;
    array(
        &mut bytes,
        ENTRY + engine_native_air::cstrategic_air::FIRST_AIR_SITES,
        0x3b000,
        2,
    );
    put64(&mut bytes, 0x3b000, BASE as u64);
    put64(&mut bytes, 0x3b008, BASE as u64);
    put64(
        &mut bytes,
        BASE,
        engine_native_air::cair_base::VTABLE_RVA.0 as u64,
    );
    put64(&mut bytes, BASE + 8, 65);
    put32(
        &mut bytes,
        BASE + engine_native_air::cair_base::COUNTRY_CONTAINERS_COUNT,
        2,
    );
    put64(
        &mut bytes,
        BASE + engine_native_air::cair_base::COUNTRY_CONTAINERS,
        0x3c000,
    );
    put64(&mut bytes, 0x3c008, 0x3d000);
    for wing in [kit::WING, 0x34000] {
        bytes[wing + cair_wing::MISSION_STATS + cair_wing::MISSION_STAT_INDEXES] = 255;
        put64(
            &mut bytes,
            wing + cair_wing::DEFAULT_STATS + 0x33 * 8,
            (-73_i64) as u64,
        );
    }
    let context = AirWingCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
        limits: [1024, 8192, 4096],
        game_score: true,
    };
    let mut job = AirWingJob::fixture();
    job.game_score = AirInputs::new(false, 3, 2, 2);
    let report = collector_test_support::with_allocations_denied(|| {
        air::copy_air_refresh(&kit::memory(&bytes), context, &mut job)
    });
    assert!(report.raw_ok);
    assert_eq!(job.game_score.wings.outcome, Outcome::Rows { len: 2 });
    assert_eq!(job.game_score.wings.rows[0].stat_raw, Ok(-73));
    assert_eq!(job.game_score.wings.rows[0].planes, Ok(10));
    assert_eq!(
        job.game_score.countries[1].read.state,
        native::AirState::Present
    );
    assert_eq!(
        job.game_score.sites.first[..2],
        [native::AirSite {
            base: 65,
            access: Ok(true)
        }; 2]
    );
    struct ScoreReads<'a> {
        bytes: &'a [u8],
        reads: std::cell::Cell<usize>,
    }
    impl Memory for ScoreReads<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            let watched = [
                ENTRY + engine_native_air::cstrategic_air::FIRST_AIR_SITES,
                kit::WING + cair_wing::DEFAULT_STATS + 0x33 * 8,
                0x34000 + cair_wing::DEFAULT_STATS + 0x33 * 8,
            ];
            if watched
                .into_iter()
                .any(|value| at <= value && value < at.saturating_add(out.len()))
            {
                self.reads.set(self.reads.get() + 1);
            }
            kit::memory(self.bytes).copy(at, out)
        }
    }
    let disabled = ScoreReads {
        bytes: &bytes,
        reads: Default::default(),
    };
    assert!(
        air::copy_air_refresh(
            &disabled,
            AirWingCopyContext {
                game_score: false,
                ..context
            },
            &mut job
        )
        .raw_ok
    );
    assert_eq!(
        disabled.reads.get(),
        0,
        "disabled extensions add no native operand reads"
    );
    struct Fault<'a> {
        bytes: &'a [u8],
        at: usize,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            !(at <= self.at && self.at < at.saturating_add(out.len()))
                && kit::memory(self.bytes).copy(at, out)
        }
    }
    let memory = Fault {
        bytes: &bytes,
        at: kit::WING + cair_wing::DEFAULT_STATS + 0x33 * 8,
    };
    assert!(air::copy_air_refresh(&memory, context, &mut job).raw_ok);
    assert!(matches!(job.source_outcome, Outcome::Rows { len: 2 }));
    assert_eq!(
        job.game_score.wings.rows[0].stat_raw,
        Err("game score air superiority stat unreadable")
    );
    assert_eq!(job.game_score.wings.rows[1].stat_raw, Ok(-73));
    let mut unrelated = bytes.clone();
    put64(
        &mut unrelated,
        kit::WING + cair_wing::MISSION + engine_native_air::air_mission::REGION,
        0xdead0000,
    );
    assert!(air::copy_air_refresh(&kit::memory(&unrelated), context, &mut job).raw_ok);
    assert!(matches!(job.source_outcome, Outcome::Unavailable { .. }));
    assert_eq!(job.game_score.wings.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        (
            job.game_score.wings.rows[0].planes,
            job.game_score.wings.rows[0].stat_raw
        ),
        (Ok(10), Ok(-73))
    );
    let mut capture = Captured::with_capacities(&Capacities::default());
    let original_wings = job.game_score.wings.rows.as_ptr();
    capture.install_air(&mut job.game_score);
    assert_eq!(
        capture.air.wings.rows.as_ptr(),
        original_wings,
        "owned transfer must not clone rows"
    );
    assert!(job.game_score.grow_for_next());
    job.game_score.grow_owned();
    assert!(
        job.game_score.countries.len() >= 2
            && job.game_score.sites.first.len() >= 2
            && job.game_score.wings.rows.len() >= 2
    );
}

#[test]
fn game_score_fleet_header_grows_first_witness_and_the_air_sites_grow_with_it() {
    use crate::extended::air::wing_test_kit as kit;
    let mut bytes = kit::fixture();
    const COUNTRY: usize = 0x20000;
    const ENTRY: usize = 0x31000;
    const FLEET: usize = 0x3e100;
    put32(
        &mut bytes,
        COUNTRY + engine_native::ccountry::DIVISIONS_COUNT,
        (-2_i32) as u32,
    );
    put32(
        &mut bytes,
        COUNTRY + engine_native::ccountry::CONTROLLED_PROVINCES_COUNT,
        9,
    );
    array(
        &mut bytes,
        COUNTRY + engine_native::ccountry::FLEETS,
        0x3e000,
        1,
    );
    put64(&mut bytes, 0x3e000, FLEET as u64);
    put64(
        &mut bytes,
        FLEET,
        engine_native::cfleet::VTABLE_RVA.0 as u64,
    );
    put64(&mut bytes, FLEET + engine_native::cfleet::HANDLE, 61);
    put32(
        &mut bytes,
        FLEET + engine_native::cfleet::TASK_FORCE_COUNT,
        7,
    );
    array(
        &mut bytes,
        ENTRY + engine_native_air::cstrategic_air::FIRST_AIR_SITES,
        0x3f000,
        1,
    );
    put64(&mut bytes, 0x3f000, 0x3f100);
    put64(
        &mut bytes,
        0x3f100,
        engine_native_air::cair_base::VTABLE_RVA.0 as u64,
    );
    put64(&mut bytes, 0x3f108, 65);
    put32(
        &mut bytes,
        0x3f100 + engine_native_air::cair_base::COUNTRY_CONTAINERS_COUNT,
        1,
    );
    struct Globals<'a>(&'a [u8]);
    impl Memory for Globals<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if (0x033e_1f00..=0x033e_21f8).contains(&at) && out.len() == 8 {
                out.copy_from_slice(&100_000_i64.to_le_bytes());
                true
            } else {
                kit::memory(self.0).copy(at, out)
            }
        }
    }
    let memory = Globals(&bytes);
    let capacity = Capacities {
        countries: 3,
        ..Default::default()
    };
    let mut capture = Captured::with_capacities(&capacity);
    for _ in 0..2 {
        capture.reset();
        capture.capture(&memory, 0, kit::GAME_STATE);
        capture.air.begin(capture.enabled);
        capture.air.owner(&memory, 0, 1, Some(ENTRY));
        capture.air.finish(Outcome::Rows { len: 0 });
        let [
            TableView::Complete(TableRows::CountryGameScoreInputs(countries)),
            _,
            TableView::Complete(TableRows::GameScoreMultipliers(multipliers)),
        ] = capture.tables()
        else {
            panic!("independent fields remain published");
        };
        assert_eq!(
            (countries[0].army_count, countries[0].land_count),
            (Some(-2), Some(9))
        );
        assert_eq!(
            (multipliers[0].total_raw, multipliers[0].land_raw),
            (Some(100_000), Some(100_000))
        );
        assert_eq!(countries[0].navy_count, Some(7));
        assert_eq!(countries[0].fleets[0].fleet, 61);
        assert_eq!(
            countries[0].air_state,
            rows::CountryGameScoreInputRow::AIR_PRESENT
        );
        assert_eq!(
            countries[0].first_sites[0].access,
            rows::GameScoreAirSite::ACCESS_FALSE
        );
    }
}

#[test]
fn game_score_air_short_storage_grows_from_full_demand_and_absence_is_not_failure() {
    let mut bytes = vec![0; 0x500];
    array(
        &mut bytes,
        0x100 + engine_native_air::cstrategic_air::FIRST_AIR_SITES,
        0x300,
        7,
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut air = AirInputs::new(true, 1, 0, 0);
    air.begin(true);
    air.owner(&memory, 0, 2, Some(0x100));
    air.owner(&memory, 0, 3, None);
    air.finish(Outcome::Truncated {
        len: 0,
        unread_rows: 9,
    });
    assert_eq!(
        (
            air.required_countries,
            air.sites.required,
            air.required_wings
        ),
        (4, 7, 9)
    );
    air.grow_owned();
    assert!(air.countries.len() >= 4 && air.sites.first.len() >= 7 && air.wings.rows.len() >= 9);
    air.begin(true);
    air.owner(&memory, 0, 3, None);
    array(
        &mut bytes,
        0x100 + engine_native_air::cstrategic_air::FIRST_AIR_SITES,
        0,
        0,
    );
    air.owner(
        &SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        0,
        2,
        Some(0x100),
    );
    air.finish(Outcome::Unavailable {
        reason: "later country wing census failed",
    });
    assert_eq!(air.countries[2].read.state, native::AirState::Present);
    assert_eq!(air.countries[2].read.len, 0);
    assert_eq!(air.countries[3].read.state, native::AirState::Absent);
    assert_eq!(
        air.countries[1].read.error,
        "later country wing census failed"
    );
}

#[test]
fn worker_score_pass_grows_storage_a_frame_lent_back_instead_of_truncating() {
    use crate::extended::air::{self, AirWingCopyContext, AirWingJob, wing_test_kit as kit};
    let mut bytes = kit::fixture();
    for wing in [kit::WING, 0x34000] {
        bytes[wing + cair_wing::MISSION_STATS + cair_wing::MISSION_STAT_INDEXES] = 255;
    }
    let memory = kit::memory(&bytes);
    let context = AirWingCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
        limits: [1024, 8192, 4096],
        game_score: true,
    };
    let mut job = AirWingJob::default();
    for hour in 0..3 {
        assert!(air::copy_air_refresh(&memory, context, &mut job).raw_ok);
        assert_eq!(
            job.game_score.wings.outcome,
            Outcome::Rows { len: 2 },
            "hour {hour}: ordinary growth truncated the shared census"
        );
        assert!(job.game_score.countries.len() > 1);
        assert_ne!(
            job.game_score.countries[1].read.error,
            AirCountry::short_storage().read.error
        );
        let mut frame = Captured::with_capacities(&Capacities::default());
        frame.install_air(&mut job.game_score);
        assert_eq!(frame.air.wings.outcome, Outcome::Rows { len: 2 });
        assert!(job.game_score.wings.rows.is_empty());
    }
}
