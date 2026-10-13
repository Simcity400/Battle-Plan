use engine::air_kamikaze::TaskForceCombat;

fn combat(province: u32, region: u32) -> TaskForceCombat {
    TaskForceCombat { province, region }
}

/// Country 1 patrols regions 5 and 9, country 2 region 5.
fn patrols(scratch: &mut [u64]) -> usize {
    scratch[..3].copy_from_slice(&[2 << 32 | 5, 1 << 32 | 9, 1 << 32 | 5]);
    3
}

fn walk(
    slot: u32,
    emit: &mut dyn FnMut(TaskForceCombat) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let fleet: &[TaskForceCombat] = match slot {
        // Region 7 is not patrolled: skipped without an ordinal.
        1 => &[combat(10, 7), combat(11, 9), combat(12, 5)],
        2 => &[combat(20, 5)],
        _ => &[],
    };
    fleet.iter().try_for_each(|&combat| emit(combat))
}

#[test]
fn patrol_combats_keep_order_filter_regions_and_list_the_regions() {
    let mut scratch = [0; 6];
    let count = patrols(&mut scratch);
    let mut rows = vec![rows::AirNavalPatrolCombatRow::default(); 4];
    let (outcome, (first, len)) = patrol_combats(&mut scratch, count, &mut rows, walk).unwrap();
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    let row = |country, ordinal, province, region| rows::AirNavalPatrolCombatRow {
        country,
        ordinal,
        province,
        region,
    };
    assert_eq!(
        rows[..3],
        [row(1, 0, 11, 9), row(1, 1, 12, 5), row(2, 0, 20, 5)]
    );
    assert_eq!(scratch[first..first + len], [5, 9]);
}

#[test]
fn patrol_combats_truncate_and_refuse_short_scratch() {
    let mut scratch = [0; 6];
    let count = patrols(&mut scratch);
    let mut rows = vec![rows::AirNavalPatrolCombatRow::default(); 1];
    let (outcome, _) = patrol_combats(&mut scratch, count, &mut rows, walk).unwrap();
    // The unread rows are what the growth pass sizes the next hour by.
    assert_eq!(
        outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    let mut short = [0; 5];
    let count = patrols(&mut short);
    assert_eq!(
        patrol_combats(&mut short, count, &mut rows, walk).map(|_| ()),
        Err("naval-patrol scratch smaller than the registrations")
    );
}

use super::*;

impl Capacities {
    pub(crate) fn fixture() -> Self {
        Self {
            rows: [8192, 2048, 4096, 1024, 2048, 4096, 1024, 64, 64],
            ace_names: 65_536,
            wings: 8192,
            sorties: air_sorties::Capacities {
                missions: 4096,
                reached: 65_536,
                rows: 2048,
                keys: 8192,
                hosts: 256,
            },
            scores: ScoreCapacities {
                rows: 4096,
                regions: 0,
                countries: 0,
            },
        }
    }
}

impl AirStatsJob {
    pub(crate) fn fixture() -> Self {
        Self {
            log: RawLog::new(1 << 16, 4 << 20),
            ..Self::default()
        }
    }
}

impl AirBasesJob {
    pub(crate) fn fixture() -> Self {
        let mut job = Self {
            log: RawLog::new(1 << 17, 8 << 20),
            census: Vec::with_capacity(1024),
            seen_links: Vec::with_capacity(1024),
            seen_link_index: vec![usize::MAX; 1024],
            ..Self::default()
        };
        job.carrier.presize(cair_base::CarrierDemand {
            bases: 2048,
            registrations: 2048,
            raw_spans: 4096,
            raw_bytes: 256 << 10,
            ..Default::default()
        });
        job.update_capacity_charge();
        job
    }
}

impl AirWingJob {
    pub(crate) fn fixture() -> Self {
        let mut job = Self {
            names: vec![0; 65_536],
            map_rows: vec![Default::default(); 8192],
            transport: cair_wing::transport_operands::Scratch::owned(8192),
            sources: vec![Default::default(); 8192],
            identities: vec![Default::default(); 8192],
            equipment: vec![Default::default(); 8192],
            cumulative: vec![Default::default(); 4096],
            memberships: vec![Default::default(); 8192],
            ..Self::default()
        };
        job.update_capacity_charge();
        job
    }
}

#[test]
fn growing_air_raw_jobs_and_decode_publish_complete_mod_populations_from_cold_storage() {
    let capacity = Capacities {
        rows: [0; SIZED],
        ace_names: 0,
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    let mut stats = AirStatsJob {
        log: RawLog::new(0, 0),
        ..Default::default()
    };
    let mut bases = AirBasesJob {
        log: RawLog::new(0, 0),
        ..Default::default()
    };
    for count in [5_000_usize, 12_000] {
        let bytes = stats_test_kit::mod_fixture(count);
        let report = copy_air_stats_refresh_growing(
            &stats_test_kit::memory(&bytes),
            AirStatsCopyContext {
                base: stats_test_kit::BASE,
                game_state: stats_test_kit::GAME_STATE,
                hour: 0,
                force_full: true,
            },
            &mut stats,
        );
        assert!(report.raw_ok);
        captured.statistics.rows.truncate(0);
        decode_air_stats_payload(&mut captured, stats_test_kit::BASE, &stats, &mut [0]);
        assert_eq!(captured.statistics.outcome, Outcome::Rows { len: count });

        let bytes = air_bases_test_kit::mod_fixture(count);
        let report = copy_air_bases_refresh_growing(
            &air_bases_test_kit::memory(&bytes),
            AirBasesCopyContext {
                base: air_bases_test_kit::BASE,
                game_state: air_bases_test_kit::GAME_STATE,
                hour: 0,
                force_full: true,
            },
            &mut bases,
        );
        assert!(report.raw_ok);
        captured.bases.rows.truncate(0);
        captured.aces.rows.truncate(0);
        captured.names.first.truncate(0);
        captured.decode_air_bases_payload(air_bases_test_kit::BASE, &bases, &mut [0; 3]);
        assert_eq!(captured.bases.outcome, Outcome::Rows { len: count + 2 });
        assert_eq!(captured.aces.outcome, Outcome::Rows { len: count });
        assert_eq!(captured.names.len, captured.names.required);
    }
}

#[test]
fn worker_air_growth_hands_off_complete_mod_rosters_at_5000_countries() {
    use crate::{map_capture::MapCaptured, unit_scale_tests::fixture};
    const WINGS: usize = 12_000;
    let world = fixture::build(fixture::Shape {
        countries: 5000,
        divisions: 0,
        wings: WINGS,
        planes: WINGS,
        ships: 0,
    });
    let memory = world.live();
    let mut job = AirWingJob::new();
    job.map_rows.resize(1, Default::default());
    job.sources.resize(1, Default::default());
    job.identities.resize(1, Default::default());
    job.names.resize(1, 0);
    let hour = (0..24)
        .find(|&hour| {
            !replay::cadence::should_walk(TableId::WingCumulative, hour, false)
                && !replay::cadence::should_walk(TableId::AirTheatreMembers, hour, false)
        })
        .unwrap();
    let report = copy_air_refresh_growing(
        &memory,
        AirWingCopyContext {
            base: world.base,
            game_state: world.state,
            hour,
            force_full: false,
            limits: [1, 0, 0],
            naval_limit: 0,
            game_score: false,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    assert_eq!(job.map_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(job.source_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(job.identity_outcome, Outcome::Rows { len: WINGS });
    assert_eq!((job.names_len, job.name_failures), (WINGS * 4, 0));

    let mut map = MapCaptured::new();
    map.air_wings.rows.truncate(1);
    map.air_wing_names.truncate(1);
    let capacity = Capacities {
        wings: 1,
        ..Default::default()
    };
    let mut captured = Captured::with_capacities(&capacity);
    let old_rows = map.air_wings.rows.as_ptr();
    let old_names = map.air_wing_names.as_ptr();
    let old_identities = captured.identities.first.as_ptr();
    let owned_rows = job.map_rows.as_ptr();
    let owned_names = job.names.as_ptr();
    let owned_identities = job.identities.as_ptr();
    map.install_owned_air(&mut job);
    captured.install_owned_mission_identities(&mut job);
    assert_eq!(map.air_wings.rows.as_ptr(), owned_rows);
    assert_eq!(map.air_wing_names.as_ptr(), owned_names);
    assert_eq!(captured.identities.first.as_ptr(), owned_identities);
    assert_eq!(job.map_rows.as_ptr(), old_rows);
    assert_eq!(job.names.as_ptr(), old_names);
    assert_eq!(job.identities.as_ptr(), old_identities);
    assert_eq!(map.air_wings.outcome, Outcome::Rows { len: WINGS });
    assert_eq!(captured.identity_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(map.air_wing_names_len, WINGS * 4);
    assert!(
        map.air_wings.rows[..WINGS]
            .iter()
            .enumerate()
            .all(|(ordinal, row)| row.name == Some((ordinal * 4, 4)))
    );
}

#[test]
fn worker_wing_extensions_and_decode_follow_first_hour_live_demand() {
    use crate::unit_scale_tests::fixture;
    const WINGS: usize = 12_000;
    let world = fixture::build(fixture::Shape {
        countries: 5000,
        divisions: 0,
        wings: WINGS,
        planes: WINGS,
        ships: 0,
    });
    let mut job = AirWingJob::new();
    job.equipment.truncate(1);
    job.cumulative.truncate(1);
    let report = copy_air_refresh_growing(
        &world.live(),
        AirWingCopyContext {
            base: world.base,
            game_state: world.state,
            hour: 0,
            force_full: true,
            limits: [1; 3],
            naval_limit: 0,
            game_score: false,
        },
        &mut job,
    );
    assert!(report.raw_ok);
    assert_eq!(job.equipment_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(job.cumulative_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(job.required[1..], [WINGS; 2]);
    assert_eq!(job.naval_outcome, Outcome::Rows { len: WINGS });
    assert_eq!(job.naval_membership_outcome, Outcome::Rows { len: WINGS });
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.equipment.rows.truncate(1);
    captured.cumulative.rows.truncate(1);
    decode_air_payload(&mut captured, &job, &mut [0; 2]);
    assert_eq!(captured.equipment.outcome, Outcome::Rows { len: WINGS });
    assert_eq!(captured.cumulative.outcome, Outcome::Rows { len: WINGS });
    assert_eq!(captured.equipment.rows[..WINGS], job.equipment[..WINGS]);
    assert_eq!(captured.cumulative.rows[..WINGS], job.cumulative[..WINGS]);
}

#[test]
fn zero_hint_wing_worker_publishes_first_and_later_populations_for_5000_countries() {
    use crate::unit_scale_tests::fixture;

    let mut job = AirWingJob::default();
    let mut captured = Captured::with_capacities(&Capacities::default());
    for (hour, count) in [5_000, 12_000].into_iter().enumerate() {
        let world = fixture::build(fixture::Shape {
            countries: 5_000,
            divisions: 0,
            wings: count,
            planes: count,
            ships: 0,
        });
        let report = copy_air_refresh_growing(
            &world.live(),
            AirWingCopyContext {
                base: world.base,
                game_state: world.state,
                hour: hour as i32,
                force_full: true,
                limits: [0; 3],
                naval_limit: 0,
                game_score: false,
            },
            &mut job,
        );
        assert!(report.raw_ok);
        let complete = Outcome::Rows { len: count };
        assert_eq!(job.map_outcome, complete);
        assert_eq!(job.source_outcome, complete);
        assert_eq!(job.identity_outcome, complete);
        assert_eq!(job.equipment_outcome, complete);
        assert_eq!(job.cumulative_outcome, complete);
        assert_eq!(job.naval_outcome, complete);
        assert_eq!(job.naval_membership_outcome, complete);
        assert_eq!((job.names_len, job.name_failures), (count * 4, 0));
        decode_air_payload(&mut captured, &job, &mut [0; 2]);
        assert_eq!(captured.equipment.outcome, complete);
        assert_eq!(captured.cumulative.outcome, complete);
        assert_eq!(captured.equipment.rows[..count], job.equipment[..count]);
        assert_eq!(captured.cumulative.rows[..count], job.cumulative[..count]);
        // The next loan has no frame grants even after its owned vectors were handed off.
        let mut map = crate::map_capture::MapCaptured::new();
        map.install_owned_air(&mut job);
        captured.install_owned_mission_identities(&mut job);
        assert_eq!(map.air_wings.outcome, complete);
        assert_eq!(captured.identity_outcome, complete);
    }
}

#[test]
fn zero_hint_wing_growth_preserves_fixed_original_read_order_and_first_faults() {
    use super::wing_test_kit as kit;
    struct Observed<'a> {
        inner: engine::memory::SliceMemory<'a>,
        fault: Option<(usize, usize)>,
        failed: std::cell::Cell<bool>,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let request = (address, output.len());
            self.calls.borrow_mut().push(request);
            if self.fault == Some(request) && !self.failed.replace(true) {
                return false;
            }
            self.inner.copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let context = AirWingCopyContext {
        base: kit::BASE,
        game_state: kit::GAME_STATE,
        hour: 0,
        force_full: true,
        limits: [8192, 8192, 4096],
        naval_limit: 8192,
        game_score: false,
    };
    let baseline = Observed {
        inner: kit::memory(&bytes),
        fault: None,
        failed: Default::default(),
        calls: Default::default(),
    };
    assert!(copy_air_refresh(&baseline, context, &mut AirWingJob::fixture()).raw_ok);
    let requests: std::collections::BTreeSet<_> = baseline.calls.into_inner().into_iter().collect();
    for fault in std::iter::once(None).chain(requests.into_iter().map(Some)) {
        let fixed_memory = Observed {
            inner: kit::memory(&bytes),
            fault,
            failed: Default::default(),
            calls: Default::default(),
        };
        let growing_memory = Observed {
            inner: kit::memory(&bytes),
            fault,
            failed: Default::default(),
            calls: Default::default(),
        };
        let mut fixed = AirWingJob::fixture();
        let mut growing = AirWingJob::default();
        let fixed_report = copy_air_refresh(&fixed_memory, context, &mut fixed);
        let growing_report = copy_air_refresh_growing(
            &growing_memory,
            AirWingCopyContext {
                limits: [0; 3],
                naval_limit: 0,
                ..context
            },
            &mut growing,
        );
        assert_eq!(
            growing_report.raw_ok, fixed_report.raw_ok,
            "fault {fault:?}"
        );
        assert_eq!(growing.map_outcome, fixed.map_outcome, "fault {fault:?}");
        assert_eq!(
            growing.source_outcome, fixed.source_outcome,
            "fault {fault:?}"
        );
        assert_eq!(
            growing.naval_outcome, fixed.naval_outcome,
            "fault {fault:?}"
        );
        assert_eq!(
            growing.equipment_outcome, fixed.equipment_outcome,
            "fault {fault:?}"
        );
        assert_eq!(
            growing.cumulative_outcome, fixed.cumulative_outcome,
            "fault {fault:?}"
        );
        assert_eq!(
            growing_memory.calls.borrow().as_slice(),
            fixed_memory.calls.borrow().as_slice(),
            "fault {fault:?}"
        );
        assert_eq!(growing_memory.failed.get(), fault.is_some());
    }
}

#[test]
fn yearly_statistics_original_capacities_survive_cold_and_later_owned_decode() {
    use super::stats_test_kit as kit;
    let mut job = AirStatsJob::default();
    let mut captured = Captured::with_capacities(&Capacities::default());
    for (count, capacity) in [(5_000, 7), (12_000, 97)] {
        let mut bytes = kit::mod_fixture(count);
        kit::set_yearly_capacity(&mut bytes, capacity);
        let memory = kit::memory(&bytes);
        assert!(
            copy_air_stats_refresh_growing(
                &memory,
                AirStatsCopyContext {
                    base: kit::BASE,
                    game_state: kit::GAME_STATE,
                    hour: 0,
                    force_full: true,
                },
                &mut job,
            )
            .raw_ok
        );
        decode_air_stats_payload(&mut captured, kit::BASE, &job, &mut [0]);
        assert_eq!(captured.statistics.outcome, Outcome::Rows { len: count });
        assert!(
            captured.statistics.rows[..count]
                .iter()
                .enumerate()
                .all(|(region, row)| {
                    row.region == region as u32
                        && row.country == 2
                        && row.year_slot == capacity - 1
                        && row.year_window_wrapped == 1
                        && row.channels
                            == cloop_history::air_stats::CHANNELS
                                .map(|channel| 1_000 + channel as i64)
                })
        );
        let mut direct = Vec::new();
        assert_eq!(
            cloop_history::walk_growing(&memory, kit::BASE, kit::GAME_STATE, &mut direct),
            captured.statistics.outcome
        );
        assert_eq!(captured.statistics.rows[..count], direct[..count]);
    }
}

#[test]
fn registration_failures_name_the_country_and_region_and_do_not_survive_reset() {
    let mut captured = Captured::with_capacities(&Capacities::fixture());
    let reason = "active mission region vector missing or invalid";
    captured.registrations.outcome = Outcome::Unavailable { reason };
    captured.registration_failure = cair_regional_mission::FailureContext {
        country_slot: Some(17),
        region: Some(9),
    };
    assert!(matches!(
        captured.registration_table(),
        TableView::Unavailable { reason }
            if reason == "active mission region vector missing or invalid (country slot 17, region 9)"
    ));
    captured.registration_failure.region = None;
    assert!(matches!(
        captured.registration_table(),
        TableView::Unavailable { reason }
            if reason == "active mission region vector missing or invalid (country slot 17)"
    ));
    captured.fail_missions_rows("air mission job failed");
    assert!(matches!(
        captured.registration_table(),
        TableView::Unavailable { reason } if reason == "air mission job failed"
    ));
    assert_eq!(captured.registration_failure, Default::default());
    captured.registration_failure.country_slot = Some(17);
    captured.reset_missions();
    assert_eq!(captured.registration_failure, Default::default());
}

#[test]
fn air_bases_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
    // Sizes from the process-wide country hints; hold them at a fresh attachment's values.
    let _hint = crate::country_scale::hint_guard();
    use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
    static LOCAL: Tracker = Tracker::new();
    let mut job = AirBasesJob::default();
    job.census.reserve_exact(3);
    job.seen_links.reserve_exact(3);
    job.seen_link_index
        .resize(job.seen_links.capacity(), usize::MAX);
    job.capacity_charge = Charge::empty_on(&LOCAL);
    job.update_capacity_charge();
    let expected = |job: &AirBasesJob| {
        vector_bytes(&job.census)
            + vector_bytes(&job.seen_links)
            + vector_bytes(&job.seen_link_index)
            + job.carrier.bytes()
    };
    let initial = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(initial, expected(&job));
    // Growth responds to observed pressure; an idle job keeps its storage.
    job.grow();
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, initial);
    job.required_census = job.census.capacity() + 1;
    job.required_links = job.seen_links.capacity() + 1;
    job.carrier.required_index = 3;
    job.carrier.demand = engine::cair_base::CarrierDemand {
        bases: 3,
        registrations: 3,
        pools: 3,
        countries: 3,
        name_bytes: 3,
        indexed: 3,
        resolved: 3,
        country_terms: 3,
        aces: 3,
        forecasts: 3,
        ..Default::default()
    };
    assert!(job.grow_for_next());
    job.grow();
    let grown = LOCAL.snapshot().current.auxiliary_vectors;
    assert!(grown > initial);
    assert_eq!(grown, expected(&job));
    assert_eq!(job.seen_link_index.len(), job.seen_links.capacity());
    assert!(!job.grow_for_next());
    job.seen_link_index[0] = 0;
    job.seen_links.push((1, 1));
    job.clear();
    assert!(job.seen_link_index.iter().all(|&cell| cell == usize::MAX));
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
    let mut moved = Some(job);
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
    let job = moved.take().unwrap();
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}

use replay::rows::TableSchema;

fn small() -> (Capacities, Captured) {
    let capacity = Capacities {
        rows: [3; SIZED],
        ace_names: 8,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
    };
    let captured = Captured::with_capacities(&capacity);
    (capacity, captured)
}

fn complete<S: TableSchema>(table: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = table else {
        panic!("expected complete table");
    };
    rows.rows::<S>().unwrap()
}

#[test]
fn equipment_capture_preserves_full_handles_order_and_exact_capacity() {
    use engine::{cequipment_variant, cequipment_variant_pool, memory::SliceMemory};
    const WING: usize = 0x1000;
    const ENTRIES: usize = 0x1400;
    const VARIANT: usize = 0x1480;
    const HANDLE: u64 = 0xfedc_ba98_0000_0046;
    let mut bytes = vec![0; 0x500];
    let pool = cair_wing::EQUIPMENT_POOL;
    bytes[pool..pool + 8]
        .copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes());
    bytes[pool + cequipment_variant_pool::ITEMS..pool + cequipment_variant_pool::ITEMS + 8]
        .copy_from_slice(&(ENTRIES as u64).to_le_bytes());
    bytes[pool + cequipment_variant_pool::CAPACITY..pool + cequipment_variant_pool::CAPACITY + 4]
        .copy_from_slice(&2_i32.to_le_bytes());
    bytes[pool + cequipment_variant_pool::COUNT..pool + cequipment_variant_pool::COUNT + 4]
        .copy_from_slice(&2_i32.to_le_bytes());
    bytes[VARIANT - WING..VARIANT - WING + 8]
        .copy_from_slice(&(cequipment_variant::VTABLE_RVA as u64).to_le_bytes());
    bytes[VARIANT - WING + 8..VARIANT - WING + 16].copy_from_slice(&HANDLE.to_le_bytes());
    for (ordinal, amount) in [0_i64, i64::MIN].into_iter().enumerate() {
        let at = ENTRIES - WING + ordinal * equipment_entry::STRIDE;
        bytes[at..at + 8].copy_from_slice(&(VARIANT as u64).to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&amount.to_le_bytes());
    }
    let memory = SliceMemory {
        base: WING,
        bytes: &bytes,
    };
    let row = cair_wing::Row {
        id: u64::MAX,
        ..Default::default()
    };
    let source = WingSource {
        pointer: WING,
        pool: 0,
        row: &row,
    };
    let mut output = std::array::from_fn::<_, 2, _>(|_| rows::WingEquipmentRow::default());
    assert_eq!(
        read_equipment(&memory, 0, &source, &mut output),
        Outcome::Rows { len: 2 }
    );
    assert!(
        output
            .iter()
            .all(|row| row.wing == u64::MAX && row.variant == HANDLE)
    );
    assert_eq!((output[0].ordinal, output[1].ordinal), (0, 1));
    assert_eq!((output[0].amount_raw, output[1].amount_raw), (0, i64::MIN));
    assert_eq!(
        read_equipment(&memory, 0, &source, &mut output[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
}

#[test]
fn ace_bytes_and_native_widths_survive_conversion_but_invalid_ranges_refuse() {
    let (_, mut captured) = small();
    captured.equipment.outcome = Outcome::Rows { len: 1 };
    captured.equipment.rows[0] = rows::WingEquipmentRow {
        wing: u64::MAX,
        ordinal: u32::MAX,
        variant: u64::MAX - 1,
        amount_raw: i64::MIN,
    };
    captured.cumulative.outcome = Outcome::Rows { len: 1 };
    captured.cumulative.rows[0] = cair_wing::CumulativeRow {
        id: u64::MAX,
        accidents: i32::MIN,
        experience: i64::MAX,
    };
    captured.aces.outcome = Outcome::Rows { len: 1 };
    captured.aces.rows[0] = cace::Row {
        id: u64::MAX,
        alive: 0,
        handled: 0,
        death: Some(cace::Death {
            killer_ace: u64::MAX - 2,
            killer_country: u32::MAX,
        }),
        name: cace::TextRange { start: 0, len: 3 },
        ..Default::default()
    };
    captured.names.first[..3].copy_from_slice(&[0xff, 0, b'a']);
    captured.names.len = 3;
    captured.names.required = 3;
    let views = captured.tables();
    let equipment = &complete::<tables::WingEquipment>(&views[0])[0];
    assert_eq!(
        (
            equipment.wing,
            equipment.ordinal,
            equipment.variant,
            equipment.amount_raw
        ),
        (u64::MAX, u32::MAX, u64::MAX - 1, i64::MIN)
    );
    assert_eq!(
        complete::<tables::WingCumulative>(&views[2])[0].experience,
        i64::MAX
    );
    let ace = &complete::<tables::AirAces>(&views[3])[0];
    assert_eq!(ace.name, [0xff, 0, b'a']);
    assert_eq!(ace.death.as_ref().unwrap().killer_ace, u64::MAX - 2);
    for range in [
        cace::TextRange {
            start: usize::MAX,
            len: 1,
        },
        cace::TextRange { start: 2, len: 2 },
    ] {
        captured.aces.rows[0].name = range;
        assert!(matches!(
            captured.ace_table(captured.aces.outcome),
            TableView::Unavailable { .. }
        ));
    }
}

#[test]
fn source_failure_never_certifies_a_prefix_and_capacity_counts_remain_exact() {
    let partial = Outcome::Truncated {
        len: 3,
        unread_rows: 4,
    };
    assert_eq!(
        append(
            partial,
            Outcome::Truncated {
                len: 0,
                unread_rows: 7
            }
        ),
        Outcome::Truncated {
            len: 3,
            unread_rows: 11
        }
    );
    assert!(matches!(
        append(
            Outcome::Truncated {
                len: 0,
                unread_rows: u32::MAX
            },
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        ),
        Outcome::Unavailable { .. }
    ));
    assert_eq!(require_parent(Outcome::Rows { len: 5 }, partial), partial);
    assert!(matches!(
        require_parent(partial, Outcome::Rows { len: 0 }),
        Outcome::Unavailable { .. }
    ));
    assert_eq!(
        require_parent(
            Outcome::Unavailable {
                reason: "late wing root"
            },
            Outcome::Rows { len: 2 }
        ),
        Outcome::Unavailable {
            reason: "late wing root"
        }
    );
    let (_, mut captured) = small();
    captured.equipment.outcome = Outcome::Unavailable {
        reason: "equipment source fault",
    };
    captured.cumulative.outcome = Outcome::Rows { len: 0 };
    assert!(matches!(
        captured.outcomes()[0],
        Outcome::Unavailable { .. }
    ));
    assert_eq!(captured.outcomes()[2], Outcome::Rows { len: 0 });
}

#[test]
fn growth_counts_unread_rows_and_retains_primary_capacity() {
    let (mut capacity, mut captured) = small();
    captured.aces.outcome = Outcome::Truncated {
        len: 3,
        unread_rows: 60,
    };
    captured.names.required = 100;
    captured.grow(&mut capacity);
    assert!(capacity.rows[3] >= 63);
    assert!(capacity.ace_names >= 100);
    assert_eq!(captured.aces.rows.len(), capacity.rows[3]);
    let before = captured.allocated_bytes();
    captured.reset();
    captured.grow(&mut capacity);
    assert_eq!(captured.allocated_bytes(), before);
}

#[test]
fn wing_daily_private_fields_and_capacities_do_not_gate_hourly_equipment() {
    use super::wing_test_kit as kit;
    struct Guard<'a> {
        inner: engine::memory::SliceMemory<'a>,
        cumulative: bool,
        theatres: bool,
    }
    impl Memory for Guard<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            for (field, active) in [
                (cair_wing::AIR_ACCIDENTS, self.cumulative),
                (cair_wing::EXPERIENCE, self.cumulative),
                (cair_wing::GROUP_HANDLE, self.theatres),
            ] {
                let target = kit::WING + field;
                assert!(
                    active || !(address <= target && target < address + bytes.len()),
                    "inactive wing field {field:#x}"
                );
            }
            self.inner.copy(address, bytes)
        }
    }
    let bytes = kit::fixture();
    let mut job = AirWingJob::fixture();
    for hour in 0..24 {
        let cumulative = hour == 16;
        let theatres = hour == 13;
        job.cumulative
            .resize(if cumulative { 2 } else { 0 }, Default::default());
        job.memberships
            .resize(if theatres { 2 } else { 0 }, Default::default());
        let memory = Guard {
            inner: kit::memory(&bytes),
            cumulative,
            theatres,
        };
        let report = copy_air_refresh(
            &memory,
            AirWingCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
                limits: [2, 3, if cumulative { 2 } else { usize::MAX }],
                naval_limit: if theatres { 2 } else { usize::MAX },
                game_score: false,
            },
            &mut job,
        );
        assert!(report.raw_ok && !job.grow_for_next());
        assert_eq!(job.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(
            job.cumulative_outcome,
            if cumulative {
                Outcome::Rows { len: 2 }
            } else {
                SKIPPED
            }
        );
        if !cumulative {
            assert_eq!(report.raw_bytes[1], 0);
        }
        if !theatres {
            assert_eq!((job.required_memberships, job.membership_ticks), (0, 0));
        }
        let mut captured = Captured::with_capacities(&Capacities {
            rows: [
                3,
                0,
                if cumulative { 2 } else { 0 },
                0,
                0,
                4096,
                1024,
                64,
                0,
            ],
            ace_names: 0,
            wings: 8192,
            sorties: Default::default(),
            scores: Default::default(),
        });
        decode_air_payload(&mut captured, &job, &mut [0; WING_PACK_N]);
        assert_eq!(captured.equipment.outcome, job.equipment_outcome);
        assert_eq!(captured.cumulative.outcome, job.cumulative_outcome);
        captured.fail_air_wing_capture("worker failed");
        assert_eq!(
            captured.cumulative.outcome,
            failed(cumulative, "worker failed")
        );
    }
}

#[test]
fn daily_statistics_skip_reads_and_capacity_preflight() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            panic!("inactive air read at {address:#x}");
        }
    }
    let mut stats = AirStatsJob::default();
    let mut captured = Captured::with_capacities(&Capacities::fixture());
    for hour in 0..24 {
        captured.prepare_hour(hour, false);
        if hour != 5 {
            let report = copy_air_stats_refresh(
                &NoReads,
                AirStatsCopyContext {
                    base: 0,
                    game_state: 1,
                    hour,
                    force_full: false,
                },
                &mut stats,
            );
            assert!(!report.raw_ok && !stats.grow_for_next());
            assert_eq!((report.copy_ticks, report.raw_bytes), ([0], [0]));
            captured.fail_air_stats_capture("worker failed");
            let mut ticks = [0];
            decode_air_stats_payload(&mut captured, 0, &stats, &mut ticks);
            assert_eq!((captured.statistics.outcome, ticks), (SKIPPED, [0]));
        }
    }
}

#[test]
fn wing_equipment_is_hourly_and_cumulative_uses_slot_16() {
    use replay::rows::TableId;
    for table in [TableId::WingEquipment, TableId::WingCumulative] {
        assert_eq!(
            replay::cadence::is_staggered(table),
            table == TableId::WingCumulative
        );
        for hour in -24_i32..48 {
            assert_eq!(
                replay::cadence::is_refresh(table, hour),
                table == TableId::WingEquipment || hour.rem_euclid(24) == 16,
                "{table:?} at {hour}"
            );
        }
    }
}

#[test]
fn wing_split_copy_matches_direct_walk() {
    use super::wing_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.cumulative_outcome, Outcome::Rows { len: 2 });
}

#[test]
fn wing_truncation_matrix_matches_direct_with_same_caps() {
    // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
    // copy→decode against the SAME memory with the SAME pooled caps,
    // asserting identical outcome (incl. len/unread) and identical rows.
    // Fixture holds 2 wings with 3 equipment rows and 2 cumulative rows;
    // equipment cap 2 is the overflow-by-one arm (total 3 == cap+1),
    // cumulative cap 1 the overflow arm (total 2 == cap+1), cap 0 the
    // zero/massive arm. u32 unread saturation is not reachable and is
    // covered by the walk's own saturating arithmetic.
    use super::Capacities;
    use super::wing_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for (equipment, cumulative) in [(0, 0), (1, 1), (2, 1), (3, 2), (4, 3), (8192, 4096)] {
        let capacity = Capacities {
            rows: [equipment, 2048, cumulative, 1024, 2048, 4096, 1024, 64, 0],
            ace_names: 65536,
            wings: 8192,
            sorties: Default::default(),
            scores: Default::default(),
        };
        let direct = kit::direct_with(&memory, &capacity);
        let (split, report, _) = kit::split_with(&memory, &capacity);
        assert!(
            report.raw_ok,
            "equipment={equipment} cumulative={cumulative}"
        );
        kit::assert_matches_direct(&split, &direct);
    }
    // Exact-fit arm pins Rows with all rows present.
    let capacity = Capacities {
        rows: [3, 2048, 2, 1024, 2048, 4096, 1024, 64, 0],
        ace_names: 65536,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
    };
    let (split, _, _) = kit::split_with(&memory, &capacity);
    assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.cumulative_outcome, Outcome::Rows { len: 2 });
}

#[test]
fn wing_copy_includes_assigned_mission_region_and_skips_null_country_slots() {
    use super::wing_test_kit as kit;
    use engine::{air_mission, cstrategic_region};
    let mut bytes = kit::fixture();
    let region = 0x3a000;
    let put = |bytes: &mut [u8], address: usize, value: u64| {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    };
    put(
        &mut bytes,
        kit::WING + cair_wing::MISSION + air_mission::REGION,
        region as u64,
    );
    put(
        &mut bytes,
        region,
        (kit::BASE + cstrategic_region::VTABLE_RVA) as u64,
    );
    put(
        &mut bytes,
        region + cstrategic_region::SECONDARY,
        (kit::BASE + cstrategic_region::SECONDARY_VTABLE_RVA) as u64,
    );
    bytes[region + cstrategic_region::ID..region + cstrategic_region::ID + 4]
        .copy_from_slice(&17u32.to_le_bytes());
    // The fixture's third slot is an unused alias; a null country slot
    // is not an absent wing census and must not stop discovery.
    put(&mut bytes, 0x11000 + 16, 0);
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.equipment_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.cumulative_outcome, Outcome::Rows { len: 2 });
}

#[test]
fn wing_split_copy_matches_direct_walk_under_truncation() {
    use super::wing_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct_truncated(&memory);
    let (split, report, _) = kit::split_truncated(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
}

#[test]
fn wing_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::wing_test_kit as kit;
    use engine::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        kit::GAME_STATE + engine::cgame_state::COUNTRIES,
        kit::WING,
        kit::WING + engine::cair_wing::EXPERIENCE,
        kit::WING + engine::cair_wing::EQUIPMENT_POOL,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, _) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn shared_wing_map_caps_do_not_bound_extension_or_membership_censuses() {
    use super::wing_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for map_limit in [0, 1, 2] {
        let mut first = super::AirWingJob::fixture();
        let context = super::AirWingCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,

            limits: [map_limit, 3, 2],
            naval_limit: 4096,
            game_score: false,
        };
        let report = super::copy_air_refresh(&memory, context, &mut first);
        assert!(report.raw_ok);
        assert_eq!(first.source_outcome, Outcome::Rows { len: 2 });
        assert_eq!(first.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(first.cumulative_outcome, Outcome::Rows { len: 2 });
        assert_eq!(first.memberships().1.len(), 2);
        assert_eq!(
            first.map_outcome,
            if map_limit == 2 {
                Outcome::Rows { len: 2 }
            } else {
                Outcome::Truncated {
                    len: map_limit,
                    unread_rows: (2 - map_limit) as u32,
                }
            }
        );
    }
}

#[test]
fn shared_wing_naval_cap_keeps_tail_leaf_failure_local() {
    use super::wing_test_kit as kit;
    let mut bytes = kit::fixture();
    // The later wing is beyond the naval prefix and remains inside the
    // independently bounded map and extension census.
    bytes[0x34000..0x34008].fill(0);
    let memory = kit::memory(&bytes);
    let mut first = super::AirWingJob::fixture();
    let context = super::AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,

        limits: [2, 3, 2],
        naval_limit: 1,
        game_score: false,
    };
    let report = super::copy_air_refresh(&memory, context, &mut first);
    assert!(report.raw_ok);
    assert!(matches!(first.source_outcome, Outcome::Unavailable { .. }));
    assert!(matches!(first.map_outcome, Outcome::Unavailable { .. }));
    assert_eq!(
        first.naval_outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!(first.memberships().1.len(), 1);
}

#[test]
fn stats_split_copy_matches_direct_walk() {
    use super::stats_test_kit as kit;
    use engine::memory::{CountedMemory, ReadCounts};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let capacities = Capacities::fixture();
    let direct = kit::direct(&memory, &capacities);
    let memory = CountedMemory::new(memory);
    let capture = read_accounting::Capture::new();
    let (split, report, mut job) = kit::split(&memory, &capacities);
    let counts = capture.finish();
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.outcome, Outcome::Rows { len: 2 });
    assert_eq!((split.rows[0].country, split.rows[0].region), (2, 0));
    assert_eq!((split.rows[1].country, split.rows[1].region), (2, 1));
    assert_eq!(split.rows[0].channels[0], 1000);
    assert_eq!(split.rows[1].channels[0], 2000);
    let anchor = TableId::AirStatistics;
    let mut expected = read_accounting::ZERO;
    // 21 census scalars, 14 header scalars per history, two nine-span
    // channel gathers and one two-pointer record batch: 790 requested bytes.
    expected[anchor.index()] = ReadCounts {
        copy_calls: 49,
        scattered_calls: 2,
        record_calls: 1,
        plan_calls: 0,
        spans: 69,
        bytes: 790,
    };
    assert_eq!(counts, expected);
    let mut refreshes = 0;
    for hour in 0..24 {
        if replay::cadence::is_refresh(anchor, hour) {
            refreshes += 1;
            continue;
        }
        let capture = read_accounting::Capture::new();
        let skipped = copy_air_stats_refresh(
            &memory,
            AirStatsCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
            },
            &mut job,
        );
        assert_eq!(capture.finish(), read_accounting::ZERO);
        assert!(!skipped.raw_ok && !job.grow_for_next());
        assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0], [0]));
    }
    assert_eq!(refreshes, 1);
    let measured = counts[anchor.index()];
    let per_refresh = [
        measured.copy_calls,
        measured.scattered_calls,
        measured.record_calls,
        measured.plan_calls,
        measured.spans,
        measured.bytes,
    ];
    println!(
        "daily_fixture cohort=stats_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
        per_refresh.map(|value| value * 24)
    );
}

#[test]
fn stats_truncation_matrix_matches_direct_with_same_caps() {
    // B3: zero/one/exact-fit(2)/overflow-by-one/massive run typed AND
    // copy→decode against the SAME memory with the SAME pooled caps,
    // asserting identical outcome (incl. len/unread) and identical rows.
    // Cap 1 is the overflow-by-one arm (total 2 == cap+1), cap 0 the
    // zero/massive arm. u32 unread saturation is not reachable with 2
    // rows and is covered by the walk's own saturating arithmetic.
    use super::stats_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for rows in [0, 1, 2, 64] {
        let capacities = Capacities {
            rows: [8192, 2048, 4096, 1024, rows, 4096, 1024, 64, 0],
            ace_names: 65536,
            wings: 8192,
            sorties: Default::default(),
            scores: Default::default(),
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok, "rows={rows}");
        kit::assert_matches_direct(&split, &direct);
    }
    let capacities = Capacities {
        rows: [8192, 2048, 4096, 1024, 2, 4096, 1024, 64, 0],
        ace_names: 65536,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
    };
    let (split, _, _) = kit::split(&memory, &capacities);
    assert_eq!(split.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn stats_staging_overflow_falls_back_to_typed_walk() {
    use super::stats_test_kit as kit;
    use super::{AirStatsCopyContext, AirStatsJob};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = AirStatsCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    let mut tiny = AirStatsJob {
        log: crate::raw::RawLog::new(1, 1),
        game_state: 0,
        ..Default::default()
    };
    let report = super::copy_air_stats_refresh(&memory, ctx, &mut tiny);
    assert!(!report.raw_ok);
}

#[test]
fn stats_copy_sizes_cover_documented_read_set() {
    // S3: every Memory::copy size the statistics copy pass issues,
    // recorded through the real pass. Census words (4/8), array headers
    // (16), window byte (1), channel spans (8/32/40/72); the walk's
    // batched gathers decompose into these exact copies at replay.
    use super::stats_test_kit as kit;
    use engine::Memory;
    use std::collections::BTreeSet;
    struct Recorder<'a> {
        inner: engine::memory::SliceMemory<'a>,
        sizes: std::cell::RefCell<BTreeSet<usize>>,
    }
    impl Memory for Recorder<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.sizes.borrow_mut().insert(output.len());
            self.inner.copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let memory = Recorder {
        inner: kit::memory(&bytes),
        sizes: std::cell::RefCell::new(BTreeSet::new()),
    };
    let ctx = super::AirStatsCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    let mut job = super::AirStatsJob::fixture();
    let report = super::copy_air_stats_refresh(&memory, ctx, &mut job);
    assert!(report.raw_ok);
    let sizes = memory.sizes.borrow().clone();
    for required in [1, 4, 8, 16, 32, 40, 72] {
        assert!(
            sizes.contains(&required),
            "missing size {required} in {sizes:?}"
        );
    }
}

#[test]
fn wing_staging_overflow_falls_back_to_typed_walk() {
    use super::wing_test_kit as kit;
    use super::{AirWingCopyContext, AirWingJob};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,

        limits: [1024, 8192, 4096],
        naval_limit: 4096,
        game_score: false,
    };
    let mut tiny = AirWingJob::default();
    let report = super::copy_air_refresh(&memory, ctx, &mut tiny);
    assert!(!report.raw_ok);
}

#[test]
fn air_bases_split_copy_matches_direct_walk_with_alias_and_rocket_vector() {
    // Census alias skip plus a non-state base vector: the alias slot
    // publishes no duplicate rows and the rocket base decodes from the
    // same shared spans.
    use super::air_bases_test_kit as kit;
    let mut bytes = kit::fixture();
    kit::add_alias_and_rocket(&mut bytes);
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.bases_outcome, Outcome::Rows { len: 4 });
    assert_eq!(split.regs.outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.aces_outcome, Outcome::Rows { len: 2 });
}

#[test]
fn bases_and_registrations_are_hourly_and_aces_use_slot_21() {
    use replay::rows::TableId;
    for table in [
        TableId::AirBases,
        TableId::AirBaseRegistrations,
        TableId::AirAces,
    ] {
        assert_eq!(
            replay::cadence::is_staggered(table),
            table == TableId::AirAces
        );
        for hour in -24_i32..48 {
            assert_eq!(
                replay::cadence::is_refresh(table, hour),
                table != TableId::AirAces || hour.rem_euclid(24) == 21,
                "{table:?} at {hour}"
            );
        }
    }
}

#[test]
fn air_bases_split_copy_matches_direct_walk() {
    use super::air_bases_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    // Two state bases plus one stateless host; two registrations (one
    // with two wings, one empty); two aces (one alive inline, one dead
    // heap-named).
    assert_eq!(split.bases_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.aces_outcome, Outcome::Rows { len: 2 });
    assert_eq!(split.regs.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        split.regs.wings,
        vec![kit::WING_HANDLE_0, kit::WING_HANDLE_1]
    );
    assert_eq!(split.bases[0].state, Some(1));
    assert_eq!(split.bases[0].province, Some(5));
    assert_eq!(split.bases[2].state, None);
    assert_eq!(split.aces[0].alive, 1);
    assert_eq!(split.aces[1].alive, 0);
    assert!(split.aces[1].death.is_some());
}

#[test]
fn air_bases_truncation_matrix_preserves_other_tables_and_grows_registrations() {
    // Bases and aces retain their bounded typed result. Registrations can
    // fill complete current rows from the already-owned payload even when
    // their initial wing arena is too small.
    use super::air_bases_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for (bases, wings, aces, names) in [
        (0, 0, 0, 0),
        (1, 1, 1, 8),
        (2, 1, 1, 16),
        (3, 2, 2, 64),
        (64, 64, 64, 4096),
    ] {
        let mut direct = kit::direct_with(&memory, bases, wings, aces, names);
        direct.regs = crate::extended::fog::Captured::collect_registrations_direct(
            2,
            2,
            &memory,
            kit::BASE,
            kit::GAME_STATE,
        );
        let (split, report, _) = kit::split_with(&memory, bases, wings, aces, names);
        assert!(
            report.raw_ok,
            "bases={bases} wings={wings} aces={aces} names={names}"
        );
        kit::assert_matches_direct(&split, &direct);
    }
    // Exact-fit arm pins Rows with all rows present.
    let direct = kit::direct_with(&memory, 3, 2, 2, 64);
    assert_eq!(direct.bases_outcome, Outcome::Rows { len: 3 });
    assert_eq!(direct.aces_outcome, Outcome::Rows { len: 2 });
    assert_eq!(direct.regs.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn air_bases_split_copy_matches_direct_walk_on_fault_matrix() {
    // Every fault arm asserts byte-identical decode (same outcome and
    // rows as the typed walk on the same torn memory) plus the expected
    // per-slot raw-compare attribution: shared census faults block all
    // three slots, manager/vector faults block bases and registrations,
    // private faults block only their own slot.
    use super::air_bases_test_kit as kit;
    use engine::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine::Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let Some(end) = address.checked_add(output.len()) else {
                return false;
            };
            if address < self.fault.end && end > self.fault.start {
                return false;
            }
            self.memory.copy(address, output)
        }
    }
    for fault in [
        // Shared country census: every slot blocks.
        kit::GAME_STATE + engine::cgame_state::COUNTRIES,
        kit::C0,
        kit::LTAB + 4,
        // Shared manager and vectors: bases and registrations block.
        kit::MANAGER,
        kit::SVEC,
        kit::B0,
        // Bases-private fields: only bases blocks.
        kit::B0 + engine::cair_base::HOST_REFERENCE,
        kit::S0,
        // Registrations-private spans: only registrations blocks.
        kit::CONT1,
        kit::CONT1 + engine::ccountry_air_container::COUNTRY,
        kit::POOL0,
        kit::W0,
        kit::W0 + engine::cair_wing::POOL,
        kit::W1 + engine::cair_wing::POOL + 7,
        // Aces-private spans: only aces blocks.
        kit::C0 + engine::ccountry::ACES,
        kit::A0,
        kit::A0 + engine::cace::ALIVE,
        kit::NSTR,
    ] {
        let bytes = kit::fixture();
        let memory = Fault {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            fault: fault..fault + 1,
        };
        let direct = kit::direct(&memory);
        let (split, report, job) = kit::split(&memory);
        assert!(report.raw_ok, "fault={fault:x}");
        kit::assert_matches_direct(&split, &direct);
        // The census keeps copying discoverable subtrees after a country
        // fault, so counts can be complete while the table is unavailable.
        // The worker's fault gate must still reject that storage hint.
        assert_eq!(
            job.registration_demand.filter(|_| !job.log.fault[1]),
            matches!(direct.regs.outcome, Outcome::Rows { .. }).then_some([2, 2]),
            "fault={fault:x}"
        );
        // Empty registration storage must preserve every unavailable
        // result, including census faults with complete subtree counts.
        // Keep unrelated bases/aces admitted: their zero-capacity walkers
        // skip validation reads and have a different acquisition contract.
        if matches!(direct.regs.outcome, Outcome::Unavailable { .. }) {
            let direct = kit::collect_typed(&memory, [8; SIZED], 256, 0, 0);
            let (split, _, job) = kit::collect_split(&memory, [8; SIZED], 256, 0, 0);
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(job.registration_demand.filter(|_| !job.log.fault[1]), None);
        }
    }
}

#[test]
fn air_bases_registration_demand_counts_duplicate_occurrences_and_empty_rows() {
    use super::air_bases_test_kit as kit;
    let mut bytes = kit::fixture();
    // Revisit the same bases in another vector, the same pool twice, and
    // the first wing twice per pool. Every occurrence remains a value.
    let vector = kit::MANAGER + cstrategic_air_manager::STATE_GUN_BASES;
    kit::put64(&mut bytes, vector, kit::SVEC as u64);
    kit::put32(&mut bytes, vector + 8, 3);
    kit::put32(&mut bytes, vector + 12, 3);
    for (header, pointer, count) in [
        (kit::CONT1 + ccountry_air_container::POOLS, kit::POOL0, 2),
        (kit::POOL0 + cair_wing_pool::WINGS, kit::W0, 3),
    ] {
        let array = u64::from_le_bytes(bytes[header..header + 8].try_into().unwrap()) as usize;
        kit::put64(&mut bytes, array + (count - 1) * 8, pointer as u64);
        kit::put32(&mut bytes, header + 8, count as u32);
        kit::put32(&mut bytes, header + 12, count as u32);
    }
    let memory = kit::memory(&bytes);
    let expected = kit::collect_typed(&memory, [8; SIZED], 256, 4, 12);
    for capacity in [0, 1] {
        let (split, report, mut job) =
            kit::collect_split(&memory, [8; SIZED], 256, capacity, capacity);
        assert!(report.raw_ok);
        assert_eq!(job.registration_demand, Some([4, 12]));
        kit::assert_matches_direct(&split, &expected);
        assert_eq!(split.regs.outcome, Outcome::Rows { len: 4 });
        assert_eq!(
            split
                .regs
                .rows
                .iter()
                .filter(|row| row.wings_len == 0)
                .count(),
            2
        );
        assert_eq!(split.regs.wings.len(), 12);
        assert_eq!(
            split.regs.wings,
            [
                kit::WING_HANDLE_0,
                kit::WING_HANDLE_1,
                kit::WING_HANDLE_0,
                kit::WING_HANDLE_0,
                kit::WING_HANDLE_1,
                kit::WING_HANDLE_0,
            ]
            .repeat(2)
        );
        job.clear();
        assert_eq!(job.registration_demand, None);
    }
    // An invalid repeated wing still refuses the table even though the
    // original acquisition already saw the container and pool counts.
    for (address, value) in [
        (kit::W0 + cair_wing::REFERENCE_TYPE, 0),
        (kit::W0 + cair_wing::POOL, 0),
        (kit::W0 + cair_wing::POOL, kit::POOL0 as u64 + 0x100),
    ] {
        let mut invalid = bytes.clone();
        kit::put64(&mut invalid, address, value);
        let memory = kit::memory(&invalid);
        let direct = kit::collect_typed(&memory, [8; SIZED], 256, 0, 0);
        let (split, _, job) = kit::collect_split(&memory, [8; SIZED], 256, 0, 0);
        assert!(matches!(split.regs.outcome, Outcome::Unavailable { .. }));
        assert_eq!(job.registration_demand, None);
        kit::assert_matches_direct(&split, &direct);
    }
}

#[test]
fn air_base_container_chunks_preserve_null_slots_and_boundary_faults() {
    use super::air_bases_test_kit as kit;
    use engine::Memory;
    let mut bytes = kit::fixture();
    let header = kit::B0 + engine::cair_base::COUNTRY_CONTAINERS;
    let array = u64::from_le_bytes(bytes[header..header + 8].try_into().unwrap()) as usize;
    // The fixture reserves space before the first container. Keep its one
    // nonnull slot, and extend the zero-filled pointer table across three
    // chunks without changing any registration row.
    assert!(array + 130 * 8 <= kit::CONT1);
    kit::put32(&mut bytes, header + 8, 130);
    kit::put32(&mut bytes, header + 12, 130);
    struct Fault<'a> {
        source: kit::SyntheticMemory<'a>,
        address: Option<usize>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if self
                .address
                .is_some_and(|fault| address <= fault && fault - address < output.len())
            {
                return false;
            }
            self.source.copy(address, output)
        }
    }
    for bad_slot in [
        None,
        Some(0),
        Some(1),
        Some(63),
        Some(64),
        Some(127),
        Some(129),
    ] {
        let memory = Fault {
            source: kit::memory(&bytes),
            address: bad_slot.map(|slot| array + slot * 8),
        };
        let direct = kit::direct(&memory);
        let (split, report, job) = kit::split(&memory);
        assert!(report.raw_ok, "{bad_slot:?}");
        kit::assert_matches_direct(&split, &direct);
        if bad_slot.is_some_and(|slot| slot > 1) {
            let mut registrations = [engine::cair_base::RegistrationRow::default(); 4];
            let mut wings = [0; 8];
            let prefix = engine::cair_base::walk_registrations(
                &memory,
                kit::BASE,
                kit::GAME_STATE,
                engine::cair_base::RegistrationBuffers {
                    registrations: &mut registrations,
                    wings: &mut wings,
                    resolved: &mut [],
                },
            );
            assert_eq!((prefix.wings_len, prefix.required_wings), (2, 2));
            assert_eq!(&wings[..2], &[kit::WING_HANDLE_0, kit::WING_HANDLE_1]);
        }
        if bad_slot.is_none() {
            let chunks = job
                .log
                .spans
                .iter()
                .filter(|span| array <= span.addr && span.addr < array + 130 * 8)
                .collect::<Vec<_>>();
            assert_eq!(chunks.len(), 3);
            assert!(chunks.iter().all(|span| span.ok && span.bulk));
        }
    }
    // A later pointer fault must not replace the first container's own
    // independent failure, in either the native reader or owned replay.
    kit::put64(&mut bytes, kit::CONT1, 0);
    let memory = Fault {
        source: kit::memory(&bytes),
        address: Some(array + 63 * 8),
    };
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    assert_eq!(
        direct.regs.outcome,
        Outcome::Unavailable {
            reason: "air base or linked object vtable mismatch",
        }
    );
    kit::assert_matches_direct(&split, &direct);
}

#[test]
fn air_bases_shared_resolve_link_taken_once_per_pass() {
    // Shared-copy proof: two containers name the same country, but the
    // resolve link read is taken exactly once per pass (mirroring the
    // walk's Resolver cache) while both registrations decode.
    use super::air_bases_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let (_split, report, first) = kit::split(&memory);
    assert!(report.raw_ok);
    let addr = kit::LTAB + 4;
    let takes = first
        .log
        .spans
        .iter()
        .filter(|span| {
            span.addr == addr && span.len == 4 && span.mask == super::AIR_REGISTRATIONS_MASK
        })
        .count();
    assert_eq!(takes, 1, "resolve link takes: {:?}", first.log.spans);
}

/// Frozen pre-index resolver for exact observation/failure comparisons.
fn linear_air_link(
    log: &mut RawLog<AIR_BASES_N>,
    memory: &impl Memory,
    links: &Array,
    seen: &mut Vec<(u32, u32)>,
    countries: u32,
    raw: u32,
) -> Option<u32> {
    if let Some((_, slot)) = seen.iter().find(|(key, _)| *key == raw) {
        return Some(*slot);
    }
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = bspan(
        log,
        memory,
        links.data.checked_add(offset),
        4,
        AIR_REGISTRATIONS_MASK,
    )?;
    let slot = bases_pool_u32(log, start);
    if slot >= countries {
        return None;
    }
    if seen.len() == seen.capacity() {
        log.overflowed = true;
        return None;
    }
    seen.push((raw, slot));
    Some(slot)
}

#[derive(Default)]
struct AirLinkMemory {
    value: std::cell::Cell<Option<u32>>,
    calls: std::cell::RefCell<Vec<(usize, usize)>>,
}

impl Memory for AirLinkMemory {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, output.len()));
        assert_eq!(output.len(), 4);
        output.fill(0xa5);
        let Some(value) = self.value.get() else {
            return false;
        };
        output.copy_from_slice(&value.to_le_bytes());
        true
    }
}

#[test]
fn air_bases_link_index_matches_linear_collisions_faults_and_full_capacity() {
    // One-cell indexes deliberately collide every raw. Successful
    // observations stay authoritative even if the source changes or
    // faults; failed and invalid resolutions must be observed again.
    let steps = [
        (0, Some(1)),
        (1, Some(2)),
        (3, Some(1)),
        (1, None),
        (3, Some(0)),
        (2, None),
        (2, Some(8)),
        (2, Some(0)),
        (2, None),
        (4, Some(3)),
        (4, Some(1)),
        (8, Some(1)),
        (u32::MAX, Some(1)),
    ];
    for index_len in [0, 1, 2, 8] {
        for capacity in [0, 1, 2, 8] {
            let links = Array {
                data: 0x1000,
                count: 8,
            };
            let mut actual = RawLog::new(32, 128);
            let mut expected = RawLog::new(32, 128);
            let mut seen = Vec::with_capacity(capacity);
            let mut linear_seen = Vec::with_capacity(capacity);
            let mut index = vec![usize::MAX; index_len];
            let index_capacity = index.capacity();
            let memory = AirLinkMemory::default();
            let reference = AirLinkMemory::default();
            let mut results = Vec::new();
            for (raw, value) in steps {
                memory.value.set(value);
                reference.value.set(value);
                let result =
                    resolve_link(&mut actual, &memory, &links, &mut seen, &mut index, 4, raw);
                assert_eq!(
                    result,
                    linear_air_link(&mut expected, &reference, &links, &mut linear_seen, 4, raw),
                    "index {index_len}, seen capacity {capacity}, raw {raw}"
                );
                results.push(result);
                assert!(actual.logs_equal(&expected));
                assert_eq!(actual.fault, expected.fault);
                assert_eq!(actual.raw_bytes, expected.raw_bytes);
                assert_eq!(actual.overflowed, expected.overflowed);
                assert_eq!(seen, linear_seen);
                assert_eq!(*memory.calls.borrow(), *reference.calls.borrow());
                assert_eq!(seen.capacity(), capacity);
                assert_eq!(index.capacity(), index_capacity);
            }
            if capacity == 8 {
                assert_eq!(
                    results,
                    [
                        None,
                        Some(2),
                        Some(1),
                        Some(2),
                        Some(1),
                        None,
                        None,
                        Some(0),
                        Some(0),
                        Some(3),
                        Some(3),
                        None,
                        None,
                    ]
                );
                assert_eq!(
                    *memory.calls.borrow(),
                    [
                        (0x1004, 4),
                        (0x100c, 4),
                        (0x1008, 4),
                        (0x1008, 4),
                        (0x1008, 4),
                        (0x1010, 4),
                    ]
                );
            } else {
                assert!(actual.overflowed);
                // The uncached successful read occurs before detecting
                // a full seen list; the index cannot preflight it away.
                let expected_calls = match capacity {
                    0 => 1,
                    1 => 2,
                    2 => 5,
                    _ => unreachable!(),
                };
                assert_eq!(memory.calls.borrow().len(), expected_calls);
            }
        }
    }
}

#[test]
fn air_bases_link_index_preserves_local_and_raw_pool_failures() {
    for data in [0x1000, usize::MAX - 1, 0x8000_0000_0000] {
        for (spans, bytes) in [(0, 8), (2, 0), (2, 8)] {
            let links = Array { data, count: 3 };
            let mut actual = RawLog::new(spans, bytes);
            let mut expected = RawLog::new(spans, bytes);
            let mut seen = Vec::with_capacity(2);
            let mut linear_seen = Vec::with_capacity(2);
            let mut index = [usize::MAX; 1];
            let memory = AirLinkMemory::default();
            let reference = AirLinkMemory::default();
            memory.value.set(Some(1));
            reference.value.set(Some(1));
            for raw in [0, 1, 1, 3, u32::MAX] {
                assert_eq!(
                    resolve_link(&mut actual, &memory, &links, &mut seen, &mut index, 3, raw),
                    linear_air_link(&mut expected, &reference, &links, &mut linear_seen, 3, raw)
                );
                assert!(actual.logs_equal(&expected));
                assert_eq!(actual.fault, expected.fault);
                assert_eq!(actual.raw_bytes, expected.raw_bytes);
                assert_eq!(actual.overflowed, expected.overflowed);
                assert_eq!(actual.grow_for_next(), expected.grow_for_next());
                assert_eq!(seen, linear_seen);
                assert_eq!(*memory.calls.borrow(), *reference.calls.borrow());
            }
        }
    }
}

#[test]
fn air_bases_link_index_validates_hints_and_resets_between_passes() {
    let mut job = AirBasesJob {
        log: RawLog::new(8, 32),
        seen_links: Vec::with_capacity(4),
        seen_link_index: vec![usize::MAX; 2],
        ..Default::default()
    };
    let links = Array {
        data: 0x1000,
        count: 4,
    };
    let memory = AirLinkMemory::default();
    for (raw, value, expected) in [(1, 2, 2), (3, 1, 1), (1, 0, 2)] {
        memory.value.set(Some(value));
        assert_eq!(
            resolve_link(
                &mut job.log,
                &memory,
                &links,
                &mut job.seen_links,
                &mut job.seen_link_index,
                4,
                raw,
            ),
            Some(expected)
        );
    }
    job.seen_link_index[1] = 99;
    memory.value.set(None);
    assert_eq!(
        resolve_link(
            &mut job.log,
            &memory,
            &links,
            &mut job.seen_links,
            &mut job.seen_link_index,
            4,
            1,
        ),
        Some(2)
    );
    assert_eq!(memory.calls.borrow().len(), 2);
    // The other bucket is cold despite the populated seen list. Faults
    // and invalid slots leave it empty; a successful observation fills it.
    assert_eq!(job.seen_link_index[0], usize::MAX);
    for (value, expected, calls) in [(None, None, 3), (Some(8), None, 4), (Some(3), Some(3), 5)] {
        memory.value.set(value);
        assert_eq!(
            resolve_link(
                &mut job.log,
                &memory,
                &links,
                &mut job.seen_links,
                &mut job.seen_link_index,
                4,
                2,
            ),
            expected
        );
        assert_eq!(job.seen_link_index[0] == usize::MAX, expected.is_none());
        assert_eq!(memory.calls.borrow().len(), calls);
    }
    assert!(job.seen_link_index.iter().all(|&cell| cell != usize::MAX));
    let capacities = (job.seen_links.capacity(), job.seen_link_index.capacity());
    job.clear();
    assert!(job.seen_links.is_empty());
    assert!(job.seen_link_index.iter().all(|&cell| cell == usize::MAX));
    memory.value.set(Some(3));
    assert_eq!(
        resolve_link(
            &mut job.log,
            &memory,
            &links,
            &mut job.seen_links,
            &mut job.seen_link_index,
            4,
            1,
        ),
        Some(3)
    );
    assert_eq!(memory.calls.borrow().len(), 6);
    assert_eq!(
        (job.seen_links.capacity(), job.seen_link_index.capacity()),
        capacities
    );
}

#[test]
fn air_bases_link_index_keeps_the_complete_capture_read_trace() {
    use super::air_bases_test_kit as kit;
    struct Trace<'a> {
        inner: kit::SyntheticMemory<'a>,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            self.inner.copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let memory = Trace {
        inner: kit::memory(&bytes),
        calls: Default::default(),
    };
    let reference = Trace {
        inner: kit::memory(&bytes),
        calls: Default::default(),
    };
    let mut indexed = AirBasesJob::fixture();
    let mut linear = AirBasesJob::fixture();
    linear.seen_link_index.clear();
    for _ in 0..2 {
        copy_air_bases_pass(
            &mut indexed,
            &memory,
            kit::BASE,
            kit::GAME_STATE,
            true,
            &mut [0; 3],
        );
        copy_air_bases_pass(
            &mut linear,
            &reference,
            kit::BASE,
            kit::GAME_STATE,
            true,
            &mut [0; 3],
        );
        assert!(!indexed.log.overflowed);
        assert!(indexed.log.logs_equal(&linear.log));
        assert_eq!(indexed.log.fault, linear.log.fault);
        assert_eq!(indexed.log.raw_bytes, linear.log.raw_bytes);
        assert_eq!(indexed.seen_links, linear.seen_links);
        assert_eq!(*memory.calls.borrow(), *reference.calls.borrow());
    }
}

#[test]
fn air_bases_registrations_decode_matches_direct_from_shared_log() {
    // Dual-decode proof: the registrations half decoded alone from the
    // shared log (fresh fog buffers, no bases state) equals the typed
    // walk, proving both families read identical rows from shared spans.
    use super::air_bases_test_kit as kit;
    use crate::extended::fog::Capacities as FogCapacities;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (_split, report, first) = kit::split(&memory);
    assert!(report.raw_ok);
    let fog_cap = FogCapacities::default();
    let mut fog = crate::extended::fog::Captured::with_capacities(&fog_cap);
    fog.registrations_raw_mode = true;
    let mut ticks = [0u64; 3];
    fog.decode_registrations_payload(kit::BASE, &first, &mut ticks);
    let alone = fog.registrations_snapshot();
    assert_eq!(alone.outcome, direct.regs.outcome, "registrations outcome");
    assert_eq!(alone.rows, direct.regs.rows, "registrations rows");
    assert_eq!(alone.wings, direct.regs.wings, "registrations wings");
}

#[test]
fn air_bases_staging_overflow_falls_back_to_typed_walk() {
    use super::air_bases_test_kit as kit;
    use super::{AirBasesCopyContext, AirBasesJob};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = AirBasesCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    let mut tiny = AirBasesJob {
        log: crate::raw::RawLog::new(1, 1),
        carrier: super::carrier::Job::default(),
        game_state: 0,
        registration_demand: None,
        aces_active: false,
        census: Vec::new(),
        seen_links: Vec::new(),
        seen_link_index: Vec::new(),
        required_census: 0,
        required_links: 0,
        capacity_charge: crate::raw_memory::Charge::default(),
        slot_run: Default::default(),
        slot_full: Vec::new(),
        slot_report: Default::default(),
        carry: Default::default(),
        registration_store: None,
        registration_work: Default::default(),
        reached: Default::default(),
        segments: Vec::new(),
    };
    let report = super::copy_air_bases_refresh(&memory, ctx, &mut tiny);
    assert!(!report.raw_ok);
    assert_eq!(tiny.registration_demand, None);
}

/// 5,000 countries, far above the initial 1,024-entry census and link
/// stores. One host base files a container at every country slot except
/// slot five, whose country object is unreadable: that census fault must
/// not stop growth, and it touches no published row. After one off-thread
/// growth the owned payload decodes exactly like the typed walks.
#[test]
fn air_bases_census_grows_once_to_five_thousand_countries() {
    // Sizes from the process-wide country hints; hold them at a fresh attachment's values.
    let _hint = crate::country_scale::hint_guard();
    use crate::extended::fog;
    use engine::{
        cair_base, ccountry, ccountry_air_container, cgame_state, cstrategic_air_manager,
    };
    const COUNTRIES: u32 = 5_001;
    const GAME: usize = 0x10000;
    const MANAGER: usize = 0x20000;
    const HOSTS: usize = 0x21000;
    const HOST: usize = 0x22000;
    const TABLE: usize = 0x30000;
    const LINKS: usize = 0x40000;
    const OBJECTS: usize = 0x50000;
    const SLOTS: usize = 0x70000;
    const CONTAINERS: usize = 0x100000;
    const UNREADABLE: u32 = 5;
    let put32 = |bytes: &mut Vec<u8>, at: usize, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };
    let put64 = |bytes: &mut Vec<u8>, at: usize, value: u64| {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    let header = |bytes: &mut Vec<u8>, at: usize, data: usize, count: u32| {
        put64(bytes, at, data as u64);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    };
    let mut bytes = vec![0; CONTAINERS + COUNTRIES as usize * 0x100];
    header(&mut bytes, GAME + cgame_state::COUNTRIES, TABLE, COUNTRIES);
    header(
        &mut bytes,
        GAME + cgame_state::COUNTRY_LINKS,
        LINKS,
        COUNTRIES,
    );
    put64(
        &mut bytes,
        GAME + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    put64(
        &mut bytes,
        MANAGER,
        cstrategic_air_manager::VTABLE_RVA as u64,
    );
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOSTS,
        1,
    );
    put64(&mut bytes, HOSTS, HOST as u64);
    put64(&mut bytes, HOST, cair_base::VTABLE_RVA as u64);
    put64(
        &mut bytes,
        HOST + cair_base::HANDLE,
        (0x43_u64 << 32) | u64::from(cair_base::TYPE),
    );
    put64(&mut bytes, HOST + cair_base::HOST_REFERENCE, (3 << 32) | 51);
    header(
        &mut bytes,
        HOST + cair_base::COUNTRY_CONTAINERS,
        SLOTS,
        COUNTRIES,
    );
    for slot in 1..COUNTRIES {
        let object = if slot == UNREADABLE {
            0x7fff_0000_0000
        } else {
            OBJECTS + slot as usize * 16
        };
        put64(&mut bytes, TABLE + slot as usize * 8, object as u64);
        put32(&mut bytes, LINKS + slot as usize * 4, slot);
        if slot == UNREADABLE {
            continue;
        }
        put64(&mut bytes, object, ccountry::VTABLE_RVA as u64);
        put32(&mut bytes, object + ccountry::ID, slot);
        let container = CONTAINERS + slot as usize * 0x100;
        put64(
            &mut bytes,
            container,
            ccountry_air_container::VTABLE_RVA as u64,
        );
        put32(
            &mut bytes,
            container + ccountry_air_container::COUNTRY,
            slot,
        );
        put64(
            &mut bytes,
            container + ccountry_air_container::CAPACITY_RAW,
            u64::from(slot),
        );
        put64(&mut bytes, SLOTS + slot as usize * 8, container as u64);
    }
    let memory = kit_memory(&bytes);
    let ctx = AirBasesCopyContext {
        hour: 0,
        force_full: false,
        base: 0,
        game_state: GAME,
    };
    let mut job = AirBasesJob::fixture();
    assert!(job.census.capacity() < COUNTRIES as usize - 1);
    let report = copy_air_bases_refresh(&memory, ctx, &mut job);
    assert!(!report.raw_ok, "the initial census store overflows");
    assert!(job.log.fault.iter().any(|fault| *fault));
    assert!(!job.log.grow_for_next(), "staging alone would never grow");
    assert!(job.grow_for_next());
    job.grow();
    job.clear();
    assert!(job.census.capacity() >= COUNTRIES as usize - 1);
    assert!(job.seen_links.capacity() >= COUNTRIES as usize);
    assert_eq!(job.seen_link_index.len(), job.seen_links.capacity());
    let report = copy_air_bases_refresh(&memory, ctx, &mut job);
    assert!(report.raw_ok);
    super::air_bases_test_kit::assert_no_mandatory_growth(&job);

    let rows = COUNTRIES as usize;
    let mut air = Captured::with_capacities(&Capacities::fixture());
    let mut ticks = [0; AIR_BASES_N];
    air.decode_air_bases_payload(0, &job, &mut ticks);
    let mut bases = vec![cair_base::Row::default(); 4];
    let direct_bases = cair_base::walk(&memory, 0, GAME, &mut bases);
    assert_eq!(direct_bases, Outcome::Rows { len: 1 });
    assert_eq!(air.bases.outcome, direct_bases);
    assert_eq!(air.bases.rows[0], bases[0]);
    let direct = fog::Captured::collect_registrations_direct(rows, 1, &memory, 0, GAME);
    assert_eq!(direct.outcome, Outcome::Rows { len: rows - 2 });
    assert!(
        direct
            .rows
            .iter()
            .zip((1..COUNTRIES).filter(|slot| *slot != UNREADABLE))
            .all(|(row, slot)| (row.slot, row.capacity_raw) == (slot, i64::from(slot)))
    );
    let fog_capacities = fog::Capacities {
        rows: [0, 0, 0, 0, 0, rows, 0],
        radar_provinces: 0,
        area_provinces: 0,
        wings: 1,
    };
    let mut owned = fog::Captured::with_capacities(&fog_capacities);
    // The first decode has no resolver yet; it records the live demand,
    // and growth after it serves every later decode from the dense cache.
    for expected_len in [0, rows] {
        owned.registrations_raw_mode = true;
        owned.decode_registrations_payload(0, &job, &mut ticks);
        assert_eq!(owned.resolver_sizes(), (expected_len, rows));
        let decoded = owned.registrations_snapshot();
        assert_eq!(decoded.outcome, direct.outcome);
        assert_eq!(decoded.rows, direct.rows);
        assert_eq!(decoded.wings, direct.wings);
        owned.grow(&mut fog::Capacities {
            rows: [0, 0, 0, 0, 0, rows, 0],
            radar_provinces: 0,
            area_provinces: 0,
            wings: 1,
        });
    }
}

fn kit_memory(bytes: &[u8]) -> engine::memory::SliceMemory<'_> {
    engine::memory::SliceMemory { base: 0, bytes }
}

/// Every guarded copy's first address over a flat fixture.
struct SlotReads<'a> {
    inner: engine::memory::SliceMemory<'a>,
    reads: std::cell::RefCell<Vec<usize>>,
}

impl engine::Memory for SlotReads<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        self.inner.copy(address, output)
    }
}

const SPARSE_GAME: usize = 0x10000;
const SPARSE_SLOTS: usize = 0x70000;
const SPARSE_COUNTRIES: u32 = 5_001;
const SPARSE_HANDLE: u64 = (0x43_u64 << 32) | engine::cair_base::TYPE as u64;

/// 5,001 countries; one host base whose country-container vector holds
/// containers at slots 1, 2600 and 5000, its dense list naming them in that
/// order, and optionally one more container at `stray`, which the list
/// lacks.
fn sparse_slot_world(stray: Option<u32>) -> Vec<u8> {
    use engine::{ccountry, ccountry_air_container, cgame_state, cstrategic_air_manager};
    const MANAGER: usize = 0x20000;
    const HOSTS: usize = 0x21000;
    const HOST: usize = 0x22000;
    const DENSE: usize = 0x23000;
    const TABLE: usize = 0x30000;
    const LINKS: usize = 0x40000;
    const OBJECTS: usize = 0x50000;
    const CONTAINERS: usize = 0x100000;
    let put32 = |bytes: &mut Vec<u8>, at: usize, value: u32| {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    };
    let put64 = |bytes: &mut Vec<u8>, at: usize, value: u64| {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    };
    let header = |bytes: &mut Vec<u8>, at: usize, data: usize, count: u32| {
        put64(bytes, at, data as u64);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    };
    let mut bytes = vec![0; CONTAINERS + 4 * 0x100];
    let game = SPARSE_GAME;
    header(
        &mut bytes,
        game + cgame_state::COUNTRIES,
        TABLE,
        SPARSE_COUNTRIES,
    );
    header(
        &mut bytes,
        game + cgame_state::COUNTRY_LINKS,
        LINKS,
        SPARSE_COUNTRIES,
    );
    put64(
        &mut bytes,
        game + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    put64(
        &mut bytes,
        MANAGER,
        cstrategic_air_manager::VTABLE_RVA as u64,
    );
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOSTS,
        1,
    );
    put64(&mut bytes, HOSTS, HOST as u64);
    put64(&mut bytes, HOST, engine::cair_base::VTABLE_RVA as u64);
    put64(&mut bytes, HOST + engine::cair_base::HANDLE, SPARSE_HANDLE);
    put64(
        &mut bytes,
        HOST + engine::cair_base::HOST_REFERENCE,
        (3 << 32) | 51,
    );
    header(
        &mut bytes,
        HOST + engine::cair_base::COUNTRY_CONTAINERS,
        SPARSE_SLOTS,
        SPARSE_COUNTRIES,
    );
    header(
        &mut bytes,
        HOST + engine::cair_base::REGISTERED_CONTAINERS,
        DENSE,
        3,
    );
    for slot in 1..SPARSE_COUNTRIES {
        let object = OBJECTS + slot as usize * 16;
        put64(&mut bytes, TABLE + slot as usize * 8, object as u64);
        put32(&mut bytes, LINKS + slot as usize * 4, slot);
        put64(&mut bytes, object, ccountry::VTABLE_RVA as u64);
        put32(&mut bytes, object + ccountry::ID, slot);
    }
    let occupied = [1_u32, 2600, 5000];
    for (index, slot) in occupied.into_iter().chain(stray).enumerate() {
        let container = CONTAINERS + index * 0x100;
        put64(
            &mut bytes,
            container,
            ccountry_air_container::VTABLE_RVA as u64,
        );
        put32(
            &mut bytes,
            container + ccountry_air_container::COUNTRY,
            slot,
        );
        put64(
            &mut bytes,
            container + ccountry_air_container::CAPACITY_RAW,
            u64::from(slot),
        );
        put64(
            &mut bytes,
            SPARSE_SLOTS + slot as usize * 8,
            container as u64,
        );
        if index < occupied.len() {
            put64(&mut bytes, DENSE + index * 8, container as u64);
        }
    }
    bytes
}

/// One accepted acquisition of `bytes` under `run`, after the census grows
/// to the fixture's countries: the job and the second pass's live reads.
fn sparse_slot_copy(
    bytes: &[u8],
    run: crate::air_base_changes::SlotRun,
) -> (AirBasesJob, Vec<usize>) {
    let memory = SlotReads {
        inner: kit_memory(bytes),
        reads: Default::default(),
    };
    let ctx = AirBasesCopyContext {
        hour: 0,
        force_full: false,
        base: 0,
        game_state: SPARSE_GAME,
    };
    let mut job = AirBasesJob::fixture();
    job.slot_run = run;
    if !copy_air_bases_refresh(&memory, ctx, &mut job).raw_ok {
        assert!(job.grow_for_next());
        job.grow();
        job.clear();
    }
    memory.reads.borrow_mut().clear();
    assert!(copy_air_bases_refresh(&memory, ctx, &mut job).raw_ok);
    (job, memory.reads.into_inner())
}

/// The owned replay of `job`'s registrations, and the direct full walk.
fn sparse_slot_decode(
    bytes: &[u8],
    job: &AirBasesJob,
) -> (
    crate::extended::fog::RegistrationsSnapshot,
    crate::extended::fog::RegistrationsSnapshot,
) {
    use crate::extended::fog;
    let rows = 8;
    let mut owned = fog::Captured::with_capacities(&fog::Capacities {
        rows: [0, 0, 0, 0, 0, rows, 0],
        radar_provinces: 0,
        area_provinces: 0,
        wings: 1,
    });
    let mut ticks = [0; AIR_BASES_N];
    owned.registrations_raw_mode = true;
    owned.decode_registrations_payload(0, job, &mut ticks);
    let direct =
        fog::Captured::collect_registrations_direct(rows, 1, &kit_memory(bytes), 0, SPARSE_GAME);
    (owned.registrations_snapshot(), direct)
}

/// The distinct addresses of live reads of the slot vector, in order.
fn slot_vector_reads(reads: &[usize]) -> Vec<usize> {
    let vector = SPARSE_SLOTS..SPARSE_SLOTS + SPARSE_COUNTRIES as usize * 8;
    let mut reads: Vec<usize> = reads
        .iter()
        .copied()
        .filter(|at| vector.contains(at))
        .collect();
    reads.sort_unstable();
    reads.dedup();
    reads
}

/// A mod-sized base with a handful of containers: its dense list proves
/// their slots, so the acquisition reads only their slot words, and the
/// replay, laying the slots out from the same plan, publishes the full
/// scan's rows.
#[test]
fn a_five_thousand_slot_base_copies_only_its_listed_slot_words_and_decodes_the_full_scan() {
    let _hint = crate::country_scale::hint_guard();
    use crate::{air_base_changes::SlotRun, sampled_verify::Check};
    let bytes = sparse_slot_world(None);
    let (full, full_reads) = sparse_slot_copy(&bytes, SlotRun::default());
    assert_eq!(full.slot_report.full, 1);
    assert!(slot_vector_reads(&full_reads).len() > 3);
    let (decoded, direct) = sparse_slot_decode(&bytes, &full);
    assert_eq!(direct.outcome, Outcome::Rows { len: 3 });
    assert_eq!(
        (decoded.outcome, &decoded.rows),
        (direct.outcome, &direct.rows)
    );

    let dense = SlotRun {
        dense: true,
        check: Check::Nothing,
    };
    let (job, reads) = sparse_slot_copy(&bytes, dense);
    assert_eq!((job.slot_report.dense, job.slot_report.full), (1, 0));
    assert_eq!(
        slot_vector_reads(&reads),
        [1, 2600, 5000].map(|slot: usize| SPARSE_SLOTS + slot * 8)
    );
    let (decoded, direct) = sparse_slot_decode(&bytes, &job);
    assert_eq!(
        (decoded.outcome, &decoded.rows, &decoded.wings),
        (direct.outcome, &direct.rows, &direct.wings)
    );
}

/// A container the dense list lacks, whether it shares a listed container's
/// chunk or not: the plan reads neither, so only the hour's comparison sees
/// it, which records the base from the full scan (the replay too) and counts
/// the difference.
#[test]
fn a_container_the_dense_list_lacks_is_seen_by_the_comparison_alone() {
    let _hint = crate::country_scale::hint_guard();
    use crate::{air_base_changes::SlotRun, sampled_verify::Check};
    let unchecked = SlotRun {
        dense: true,
        check: Check::Nothing,
    };
    let compared = SlotRun {
        dense: true,
        check: Check::Every,
    };
    // Slot 2 shares slot 1's chunk; slot 4000 lies in a chunk no listed
    // container names.
    for stray in [2, 4000] {
        let bytes = sparse_slot_world(Some(stray));
        let (job, _) = sparse_slot_copy(&bytes, unchecked);
        assert_eq!(job.slot_report.mismatches, 0);
        let (missed, direct) = sparse_slot_decode(&bytes, &job);
        assert_eq!(direct.outcome, Outcome::Rows { len: 4 });
        assert_eq!(missed.outcome, Outcome::Rows { len: 3 });
        let (job, _) = sparse_slot_copy(&bytes, compared);
        assert_eq!(
            (
                job.slot_report.verified,
                job.slot_report.mismatches,
                job.slot_report.full
            ),
            (1, 1, 1)
        );
        assert_eq!(job.slot_full, [SPARSE_HANDLE]);
        let (decoded, direct) = sparse_slot_decode(&bytes, &job);
        assert_eq!(
            (decoded.outcome, &decoded.rows, &decoded.wings),
            (direct.outcome, &direct.rows, &direct.wings)
        );
    }
}

#[test]
fn air_raw_job_constructors_wait_for_live_demand() {
    let bases = AirBasesJob::new();
    assert_eq!(bases.log.spans.capacity(), 0);
    assert_eq!(bases.log.bytes.capacity(), 0);
    let stats = AirStatsJob::new();
    assert_eq!(stats.log.spans.capacity(), 0);
    assert_eq!(stats.log.bytes.capacity(), 0);
    let wing = AirWingJob::new();
    assert!(wing.map_rows.is_empty());
    assert!(wing.sources.is_empty());
    assert!(wing.identities.is_empty());
    assert!(wing.equipment.is_empty());
    assert!(wing.cumulative.is_empty());
    assert!(wing.memberships.is_empty());
    assert!(wing.naval_memberships.is_empty());
}

#[test]
fn air_bases_copy_sizes_cover_documented_read_set() {
    // S3: every Memory::copy size the shared copy pass issues, recorded
    // through the real pass. Census words (4/8), array headers (16),
    // base identity (16), host block (32), state/province words (8/4),
    // pool/wing blocks (chunked 8s), wing identities (32), ace scattered
    // fields (16/4/1/12) and name bytes (1 + content).
    use super::air_bases_test_kit as kit;
    use engine::Memory;
    use std::collections::BTreeSet;
    struct Recorder<'a> {
        inner: kit::SyntheticMemory<'a>,
        sizes: std::cell::RefCell<BTreeSet<usize>>,
    }
    impl Memory for Recorder<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.sizes.borrow_mut().insert(output.len());
            self.inner.copy(address, output)
        }
    }
    let bytes = kit::fixture();
    let memory = Recorder {
        inner: kit::memory(&bytes),
        sizes: std::cell::RefCell::new(BTreeSet::new()),
    };
    let (_split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    let sizes = memory.sizes.borrow().clone();
    for required in [1, 4, 8, 12, 16, 32] {
        assert!(
            sizes.contains(&required),
            "missing size {required} in {sizes:?}"
        );
    }
}

#[test]
fn scenario_history_regions_do_not_depend_on_a_naval_combat_census() {
    let wings = [
        WingIdentity {
            wing: 1,
            region: Some(9),
            ..Default::default()
        },
        WingIdentity {
            wing: 2,
            region: Some(3),
            ..Default::default()
        },
    ];
    let registrations = [
        cair_regional_mission::Row {
            wing: 1,
            ordinal: 0,
        },
        cair_regional_mission::Row {
            wing: 2,
            ordinal: 0,
        },
    ];
    let mut handles = [0; 4];
    let count = scenario_history_regions(
        &wings,
        Some(&registrations),
        Err("naval census unavailable"),
        &mut handles,
    )
    .unwrap();
    assert_eq!(&handles[..count], &[3, 9]);
    handles[2..].copy_from_slice(&[5, 9]);
    let count = scenario_history_regions(
        &wings,
        Some(&registrations),
        Ok((Outcome::Rows { len: 0 }, (2, 2))),
        &mut handles,
    )
    .unwrap();
    assert_eq!(&handles[..count], &[3, 5, 9]);
    assert!(
        scenario_history_regions(
            &wings,
            None,
            Ok((Outcome::Rows { len: 0 }, (0, 0))),
            &mut handles
        )
        .is_err()
    );
    assert!(
        scenario_history_regions(
            &wings,
            Some(&registrations),
            Ok((Outcome::Rows { len: 0 }, (0, 2))),
            &mut handles[..2]
        )
        .is_err()
    );
}
