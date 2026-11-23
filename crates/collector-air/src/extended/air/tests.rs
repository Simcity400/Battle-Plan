#[cfg(test)]
use engine_air_history::air_kamikaze::TaskForceCombat;

#[cfg(test)]
fn combat(province: u32, region: u32) -> TaskForceCombat {
    TaskForceCombat { province, region }
}

#[cfg(test)]
fn patrols(scratch: &mut [u64]) -> usize {
    scratch[..3].copy_from_slice(&[2 << 32 | 5, 1 << 32 | 9, 1 << 32 | 5]);
    3
}

#[cfg(test)]
fn walk(
    slot: u32,
    emit: &mut dyn FnMut(TaskForceCombat) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let fleet: &[TaskForceCombat] = match slot {
        1 => &[combat(10, 7), combat(11, 9), combat(12, 5)],
        2 => &[combat(20, 5)],
        _ => &[],
    };
    fleet.iter().try_for_each(|&combat| emit(combat))
}

#[test]
fn patrol_combats_keep_order_filter_regions_and_list_the_regions() {
    let mut scratch = vec![0; 6];
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

use super::*;

impl Capacities {
    pub fn fixture() -> Self {
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
    pub fn fixture() -> Self {
        Self {
            log: RawLog::new(1 << 16, 4 << 20),
            ..Self::default()
        }
    }
}

impl AirBasesJob {
    pub fn fixture() -> Self {
        let mut job = Self {
            log: RawLog::new(1 << 17, 8 << 20),
            census: Vec::with_capacity(1024),
            roster: engine_native::ccountry::RosterSnapshot::try_with_capacity(1024).unwrap(),
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
    pub fn fixture() -> Self {
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
        let report = copy_air_stats_refresh(
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
        let report = copy_air_bases_fixture(
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
fn worker_wing_extensions_and_decode_follow_first_hour_live_demand() {
    use crate::unit_fixture as fixture;
    const WINGS: usize = 120;
    let world = fixture::build(fixture::Shape {
        countries: 40,
        divisions: 0,
        wings: WINGS,
        planes: WINGS,
        ships: 0,
    });
    let mut job = AirWingJob::new();
    job.equipment.truncate(1);
    job.cumulative.truncate(1);
    let report = copy_air_refresh(
        &world.live(),
        AirWingCopyContext {
            base: world.base,
            game_state: world.state,
            hour: 0,
            force_full: true,
            limits: [1; 3],
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
fn zero_hint_wing_growth_preserves_fixed_original_read_order_and_first_faults() {
    use super::wing_test_kit as kit;
    struct Observed<'a> {
        inner: engine_native_memory::memory::SliceMemory<'a>,
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
        let growing_report = copy_air_refresh(
            &growing_memory,
            AirWingCopyContext {
                limits: [0; 3],
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
        let fixed_calls = fixed_memory.calls.borrow();
        let original_requests: std::collections::BTreeSet<_> =
            fixed_calls.iter().copied().collect();
        let growing_original_calls: Vec<_> = growing_memory
            .calls
            .borrow()
            .iter()
            .copied()
            .filter(|request| original_requests.contains(request))
            .collect();
        assert_eq!(growing_original_calls, *fixed_calls, "fault {fault:?}");
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
            copy_air_stats_refresh(
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
            cloop_history::walk(&memory, kit::BASE, kit::GAME_STATE, &mut direct),
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
    let _hint = crate::sizing::hint_guard();
    use collector_primitives::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
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
            + job.roster.allocated_bytes() as u64
            + vector_bytes(&job.seen_links)
            + vector_bytes(&job.seen_link_index)
            + job.carrier.bytes()
    };
    let initial = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(initial, expected(&job));
    job.grow();
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, initial);
    job.required_census = job.census.capacity() + 1;
    job.required_links = job.seen_links.capacity() + 1;
    job.carrier.required_index = 3;
    job.carrier.demand = engine_native_air::cair_base::CarrierDemand {
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

#[cfg(test)]
use replay_data::rows::TableSchema;

#[cfg(test)]
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

#[cfg(test)]
fn complete<S: TableSchema>(table: &TableView) -> &[S::Row] {
    let TableView::Complete(rows) = table else {
        panic!("expected complete table");
    };
    rows.rows::<S>().unwrap()
}

#[test]
fn equipment_capture_preserves_full_handles_order_and_exact_capacity() {
    use engine_native::cequipment_variant::{self, pool as cequipment_variant_pool};
    use engine_native_memory::memory::SliceMemory;
    const WING: usize = 0x1000;
    const ENTRIES: usize = 0x1400;
    const VARIANT: usize = 0x1480;
    const HANDLE: u64 = 0xfedc_ba98_0000_0046;
    let mut bytes = vec![0; 0x500];
    let pool = cair_wing::EQUIPMENT_POOL;
    bytes[pool..pool + 8]
        .copy_from_slice(&(cequipment_variant_pool::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[pool + cequipment_variant_pool::ITEMS..pool + cequipment_variant_pool::ITEMS + 8]
        .copy_from_slice(&(ENTRIES as u64).to_le_bytes());
    bytes[pool + cequipment_variant_pool::CAPACITY..pool + cequipment_variant_pool::CAPACITY + 4]
        .copy_from_slice(&2_i32.to_le_bytes());
    bytes[pool + cequipment_variant_pool::COUNT..pool + cequipment_variant_pool::COUNT + 4]
        .copy_from_slice(&2_i32.to_le_bytes());
    bytes[VARIANT - WING..VARIANT - WING + 8]
        .copy_from_slice(&(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes());
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
        row: &row,
    };
    let mut output = Vec::new();
    assert_eq!(
        read_equipment_buffer(
            &memory,
            0,
            &source,
            engine_native_memory::walk::Buffer::Growing(&mut output),
            0
        )
        .0,
        Outcome::Rows { len: 2 }
    );
    assert!(
        output
            .iter()
            .all(|row| row.wing == u64::MAX && row.variant == HANDLE)
    );
    assert_eq!((output[0].ordinal, output[1].ordinal), (0, 1));
    assert_eq!((output[0].amount_raw, output[1].amount_raw), (0, i64::MIN));
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
        inner: engine_native_memory::memory::SliceMemory<'a>,
        cumulative: bool,
        theatres: bool,
    }
    impl Memory for Guard<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            for (field, active) in [
                (cair_wing::AIR_ACCIDENTS, self.cumulative),
                (cair_wing::EXPERIENCE, self.cumulative),
                (engine_native_parts::cair_wing_layout::GROUP_HANDLE, self.theatres),
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
            assert_eq!(report.copy_ticks, [0]);
            captured.fail_air_stats_capture("worker failed");
            let mut ticks = [0];
            decode_air_stats_payload(&mut captured, 0, &stats, &mut ticks);
            assert_eq!((captured.statistics.outcome, ticks), (SKIPPED, [0]));
        }
    }
}

#[test]
fn wing_truncation_matrix_matches_direct_with_same_caps() {
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
    use engine_native_air::air_mission;
    use engine_native_parts::cstrategic_region;
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
        (kit::BASE + cstrategic_region::VTABLE_RVA.0) as u64,
    );
    put(
        &mut bytes,
        region + cstrategic_region::SECONDARY,
        (kit::BASE + cstrategic_region::SECONDARY_VTABLE_RVA.0) as u64,
    );
    bytes[region + cstrategic_region::ID..region + cstrategic_region::ID + 4]
        .copy_from_slice(&17u32.to_le_bytes());
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
    use engine_native_memory::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native_memory::Memory for Fault<'_> {
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
        kit::GAME_STATE + engine_native::cgame_state::COUNTRIES,
        kit::WING,
        kit::WING + engine_native_air::cair_wing::EXPERIENCE,
        kit::WING + engine_native_air::cair_wing::EQUIPMENT_POOL,
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
fn shared_wing_map_grants_bound_no_census() {
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
            game_score: false,
        };
        let report = super::copy_air_refresh(&memory, context, &mut first);
        assert!(report.raw_ok);
        assert_eq!(first.source_outcome, Outcome::Rows { len: 2 });
        assert_eq!(first.equipment_outcome, Outcome::Rows { len: 3 });
        assert_eq!(first.cumulative_outcome, Outcome::Rows { len: 2 });
        assert_eq!(first.memberships().1.len(), 2);
        assert_eq!(first.map_outcome, Outcome::Rows { len: 2 }, "map grant {map_limit}");
    }
}

#[test]
fn stats_split_copy_matches_direct_walk() {
    use super::stats_test_kit as kit;
    use engine_native_memory::memory::{CountedMemory, ReadCounts};
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
        if replay_data::cadence::is_refresh(anchor, hour) {
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
        assert_eq!(skipped.copy_ticks, [0]);
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
fn air_bases_split_copy_matches_direct_walk_with_a_reserved_country_alias() {
    use super::air_bases_test_kit as kit;
    let mut bytes = kit::fixture();
    kit::add_reserved_alias(&mut bytes);
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    assert_eq!(direct.aces_outcome, Outcome::Rows { len: 2 });
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
}

#[test]
fn stats_split_copy_matches_direct_walk_past_a_reserved_alias_and_a_null_slot() {
    use super::stats_test_kit as kit;
    let mut bytes = kit::fixture();
    kit::add_reserved_alias_and_null_slot(&mut bytes);
    let memory = kit::memory(&bytes);
    let capacities = Capacities::fixture();
    let direct = kit::direct(&memory, &capacities);
    assert_eq!(direct.outcome, Outcome::Rows { len: 2 });
    let (split, report, _) = kit::split(&memory, &capacities);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
}

#[test]
fn air_bases_split_copy_matches_direct_walk_with_alias_and_rocket_vector() {
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
fn air_bases_split_copy_matches_direct_walk() {
    use super::air_bases_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let (split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
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
    let direct = kit::direct_with(&memory, 3, 2, 2, 64);
    assert_eq!(direct.bases_outcome, Outcome::Rows { len: 3 });
    assert_eq!(direct.aces_outcome, Outcome::Rows { len: 2 });
    assert_eq!(direct.regs.outcome, Outcome::Rows { len: 2 });
}

#[test]
fn air_bases_split_copy_matches_direct_walk_on_fault_matrix() {
    use super::air_bases_test_kit as kit;
    use engine_native_memory::memory::SliceMemory;
    struct Fault<'a> {
        memory: SliceMemory<'a>,
        fault: std::ops::Range<usize>,
    }
    impl engine_native_memory::Memory for Fault<'_> {
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
        kit::GAME_STATE + engine_native::cgame_state::COUNTRIES,
        kit::C0,
        kit::LTAB + 4,
        kit::MANAGER,
        kit::SVEC,
        kit::B0,
        kit::B0 + engine_native_parts::cair_base_layout::HOST_REFERENCE,
        kit::S0,
        kit::CONT1,
        kit::CONT1 + engine_native_air::ccountry_air_container::COUNTRY,
        kit::POOL0,
        kit::W0,
        kit::W0 + engine_native_parts::cair_wing_layout::POOL,
        kit::W1 + engine_native_parts::cair_wing_layout::POOL + 7,
        kit::C0 + engine_native::ccountry::ACES,
        kit::A0,
        kit::A0 + engine_native_air::cace::ALIVE,
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
        assert_eq!(
            job.registration_demand.filter(|_| !job.log.fault[1]),
            matches!(direct.regs.outcome, Outcome::Rows { .. }).then_some([2, 2]),
            "fault={fault:x}"
        );
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
    for (address, value) in [
        (kit::W0 + engine_native_parts::cair_wing_layout::REFERENCE_TYPE, 0),
        (kit::W0 + engine_native_parts::cair_wing_layout::POOL, 0),
        (kit::W0 + engine_native_parts::cair_wing_layout::POOL, kit::POOL0 as u64 + 0x100),
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
    use engine_native_memory::Memory;
    let mut bytes = kit::fixture();
    let header = kit::B0 + engine_native_air::cair_base::COUNTRY_CONTAINERS;
    let array = u64::from_le_bytes(bytes[header..header + 8].try_into().unwrap()) as usize;
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
            let mut registrations = [engine_native_air::cair_base::RegistrationRow::default(); 4];
            let mut wings = [0; 8];
            let prefix = engine_native_air::cair_base::walk_registrations(
                &memory,
                kit::BASE,
                kit::GAME_STATE,
                engine_native_air::cair_base::RegistrationBuffers {
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

#[cfg(test)]
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
    seen.push((raw, slot));
    Some(slot)
}

#[derive(Default)]
#[cfg(test)]
struct AirLinkMemory {
    value: std::cell::Cell<Option<u32>>,
    calls: std::cell::RefCell<Vec<(usize, usize)>>,
}

#[cfg(test)]
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
fn air_bases_link_index_matches_linear_collisions_faults_and_growth() {
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
                assert_eq!(index.capacity(), index_capacity);
            }
            assert!(!actual.overflowed);
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

#[cfg(test)]
fn kit_memory(bytes: &[u8]) -> engine_native_memory::memory::SliceMemory<'_> {
    engine_native_memory::memory::SliceMemory { base: 0, bytes }
}

#[cfg(test)]
struct SlotReads<'a> {
    inner: engine_native_memory::memory::SliceMemory<'a>,
    reads: std::cell::RefCell<Vec<usize>>,
}

#[cfg(test)]
impl engine_native_memory::Memory for SlotReads<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        self.inner.copy(address, output)
    }
}

#[cfg(test)]
const SPARSE_GAME: usize = 0x10000;
#[cfg(test)]
const SPARSE_SLOTS: usize = 0x70000;
#[cfg(test)]
const SPARSE_COUNTRIES: u32 = 5_001;
#[cfg(test)]
const SPARSE_HANDLE: u64 = (0x43_u64 << 32) | engine_native_parts::cair_base_layout::TYPE as u64;

#[cfg(test)]
fn sparse_slot_world(stray: Option<u32>) -> Vec<u8> {
    use engine_native::{ccountry, cgame_state};
    use engine_native_air::{ccountry_air_container, cstrategic_air_manager};
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
        cstrategic_air_manager::VTABLE_RVA.0 as u64,
    );
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::STATELESS_AIR_BASES,
        HOSTS,
        1,
    );
    put64(&mut bytes, HOSTS, HOST as u64);
    put64(
        &mut bytes,
        HOST,
        engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64,
    );
    put64(
        &mut bytes,
        HOST + engine_native_parts::cair_base_layout::HANDLE,
        SPARSE_HANDLE,
    );
    put64(
        &mut bytes,
        HOST + engine_native_parts::cair_base_layout::HOST_REFERENCE,
        (3 << 32) | 51,
    );
    header(
        &mut bytes,
        HOST + engine_native_air::cair_base::COUNTRY_CONTAINERS,
        SPARSE_SLOTS,
        SPARSE_COUNTRIES,
    );
    header(
        &mut bytes,
        HOST + engine_native_air::cair_base::REGISTERED_CONTAINERS,
        DENSE,
        3,
    );
    for slot in 1..SPARSE_COUNTRIES {
        let object = OBJECTS + slot as usize * 16;
        put64(&mut bytes, TABLE + slot as usize * 8, object as u64);
        put32(&mut bytes, LINKS + slot as usize * 4, slot);
        put64(&mut bytes, object, ccountry::VTABLE_RVA.0 as u64);
        put32(&mut bytes, object + ccountry::ID, slot);
    }
    let occupied = [1_u32, 2600, 5000];
    for (index, slot) in occupied.into_iter().chain(stray).enumerate() {
        let container = CONTAINERS + index * 0x100;
        put64(
            &mut bytes,
            container,
            ccountry_air_container::VTABLE_RVA.0 as u64,
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

#[cfg(test)]
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
    if !copy_air_bases_fixture(&memory, ctx, &mut job).raw_ok {
        assert!(job.grow_for_next());
        job.grow();
        job.clear();
    }
    memory.reads.borrow_mut().clear();
    assert!(copy_air_bases_fixture(&memory, ctx, &mut job).raw_ok);
    (job, memory.reads.into_inner())
}

#[cfg(test)]
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

#[test]
fn a_container_the_dense_list_lacks_is_missed_by_the_plan_alone() {
    let _hint = crate::sizing::hint_guard();
    use crate::air_base_changes::SlotRun;
    for stray in [2, 4000] {
        let bytes = sparse_slot_world(Some(stray));
        let (job, _) = sparse_slot_copy(&bytes, SlotRun { dense: true });
        let (missed, direct) = sparse_slot_decode(&bytes, &job);
        assert_eq!(direct.outcome, Outcome::Rows { len: 4 });
        assert_eq!(missed.outcome, Outcome::Rows { len: 3 });
        let (job, _) = sparse_slot_copy(&bytes, SlotRun { dense: false });
        assert_eq!(job.slot_report.full, 1);
        let (decoded, direct) = sparse_slot_decode(&bytes, &job);
        assert_eq!(
            (decoded.outcome, &decoded.rows, &decoded.wings),
            (direct.outcome, &direct.rows, &direct.wings)
        );
    }
}

#[test]
fn supply_merges_into_sorted_ground_crews_and_adds_flagged_rows() {
    use engine_map_air::map_air_supply::Summary;
    let crew = |country, region, enabled| rows::AirGroundCrewRow {
        supply: rows::Recorded::Value(None),
        country,
        region,
        enabled,
    };
    let sample = |country, region, needed| Summary {
        country_pointer: 0x100 * country as usize,
        region_pointer: if region == u32::MAX { 0 } else { 0x10 * region as usize },
        country,
        region,
        severity: 1,
        needed,
        supplied: 2,
        deficit: 3,
    };
    let mut ground = vec![crew(1, 4, Some(1)), crew(1, 9, Some(1)), crew(3, 2, None)];
    let supply = [
        sample(1, 4, 10),
        sample(1, 6, 11),
        sample(1, u32::MAX, 0),
        sample(2, u32::MAX, 0),
    ];
    let mut merged = Vec::new();
    let mut asked = Vec::new();
    super::merge_supply(&mut ground, &mut merged, &supply, |country, region| {
        asked.push((country, region));
        Some(1)
    })
    .unwrap();
    let keys: Vec<_> = ground.iter().map(|row| (row.country, row.region, row.enabled)).collect();
    assert_eq!(
        keys,
        [
            (1, 4, Some(1)),
            (1, 6, Some(1)),
            (1, 9, Some(1)),
            (1, u32::MAX, None),
            (2, u32::MAX, None),
            (3, 2, None),
        ]
    );
    assert_eq!(asked, [(0x100, 0x60)]);
    let supply = |at: usize| match &ground[at].supply {
        rows::Recorded::Value(supply) => supply.as_ref().map(|supply| supply.needed),
        rows::Recorded::NotRecorded => panic!("a captured row records its supply"),
    };
    assert_eq!(supply(0), Some(10));
    assert_eq!(supply(2), None);
    assert_eq!(supply(5), None);
}
