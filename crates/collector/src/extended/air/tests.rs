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
    let (outcome, (first, len)) =
        patrol_combats(&mut scratch, count, &mut rows, walk).unwrap();
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    let row = |country, ordinal, province, region| rows::AirNavalPatrolCombatRow {
        country,
        ordinal,
        province,
        region,
    };
    assert_eq!(rows[..3], [row(1, 0, 11, 9), row(1, 1, 12, 5), row(2, 0, 20, 5)]);
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

#[test]
fn registration_failures_name_the_country_and_region_and_do_not_survive_reset() {
    let mut captured = Captured::with_capacities(&Capacities::default());
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
    };
    let initial = LOCAL.snapshot().current.auxiliary_vectors;
    assert_eq!(initial, expected(&job));
    job.grow();
    let grown = LOCAL.snapshot().current.auxiliary_vectors;
    assert!(grown > initial);
    assert_eq!(grown, expected(&job));
    assert_eq!(job.seen_link_index.len(), job.seen_links.capacity());
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
    bytes[pool + cequipment_variant_pool::CAPACITY
        ..pool + cequipment_variant_pool::CAPACITY + 4]
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
fn grouped_children_preserve_duplicate_coordinates_order_and_full_damage_amounts() {
    let key = sair_wing_combat_data::Key {
        country: u32::MAX,
        region: 0,
        side: 1,
        date: i32::MIN,
        wing_id: 0,
        classification: u8::MAX,
    };
    let history = [
        sair_wing_combat_data::Row {
            key,
            planes: i32::MAX,
            selection: 1,
            ..Default::default()
        },
        sair_wing_combat_data::Row {
            key,
            planes: i32::MIN,
            selection: 2,
            ..Default::default()
        },
    ];
    let view = history_table(Outcome::Rows { len: 2 }, &history);
    let groups = complete::<tables::AirRegionHistory>(&view);
    assert_eq!(groups.len(), 1);
    assert_eq!((groups[0].country, groups[0].region), (u32::MAX, 0));
    assert_eq!(
        groups[0]
            .entries
            .iter()
            .map(|entry| (entry.planes, entry.selection))
            .collect::<Vec<_>>(),
        [(i32::MAX, 1), (i32::MIN, 2)]
    );
    let damage = [
        sdamage_entry::Row {
            record: key,
            channel: 1,
            cause: u32::MAX,
            amount: i64::MIN,
        },
        sdamage_entry::Row {
            record: key,
            channel: 1,
            cause: u32::MAX,
            amount: i64::MAX,
        },
    ];
    let view = damage_table(Outcome::Rows { len: 2 }, &damage);
    let groups = complete::<tables::AirRegionDamage>(&view);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].entries[0].amount, i64::MIN);
    assert_eq!(groups[0].entries[1].amount, i64::MAX);
    assert_eq!(groups[0].entries[0].cause, u32::MAX);
}

#[test]
fn incomplete_group_and_repeated_outer_parent_are_never_complete_rows() {
    let first = sair_wing_combat_data::Row::default();
    let mut other = first;
    other.key.region = 3;
    assert!(matches!(
        history_table(Outcome::Rows { len: 3 }, &[first, other, first]),
        TableView::Unavailable { .. }
    ));
    assert!(matches!(
        history_table(
            Outcome::Truncated {
                len: 1,
                unread_rows: 4
            },
            &[first]
        ),
        TableView::Unavailable { .. }
    ));
    assert!(matches!(
        history_table(Outcome::Rows { len: 2 }, &[first]),
        TableView::Unavailable { .. }
    ));
    assert!(matches!(
        damage_table(
            Outcome::Unavailable {
                reason: "late root unreadable"
            },
            &[]
        ),
        TableView::Unavailable { .. }
    ));
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
    let ace = &complete::<tables::AirAces>(&views[5])[0];
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
fn growth_counts_native_entries_and_retains_primary_capacity() {
    let (mut capacity, mut captured) = small();
    captured.history.outcome = Outcome::Truncated {
        len: 3,
        unread_rows: 60,
    };
    captured.names.required = 100;
    captured.grow(&mut capacity);
    assert!(capacity.rows[3] >= 63);
    assert!(capacity.ace_names >= 100);
    assert_eq!(captured.history.rows.len(), capacity.rows[3]);
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
    let mut job = AirWingJob::new();
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
            rows: [3, 0, if cumulative { 2 } else { 0 }, 0, 0, 0, 0, 4096, 1024, 64, 0],
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
fn daily_statistics_and_history_skip_reads_and_capacity_preflight() {
    struct NoReads;
    impl Memory for NoReads {
        fn copy(&self, address: usize, _: &mut [u8]) -> bool {
            panic!("inactive air read at {address:#x}");
        }
    }
    let mut stats = AirStatsJob::default();
    let mut history = AirHistoryJob::default();
    let mut captured = Captured::with_capacities(&Capacities::default());
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
        if hour != 7 {
            let report = copy_air_history_refresh(
                &NoReads,
                AirHistoryCopyContext {
                    base: 0,
                    game_state: 1,
                    hour,
                    force_full: false,
                    history_limit: usize::MAX,
                    damage_limit: usize::MAX,
                },
                &mut history,
            );
            assert!(!report.raw_ok && !history.grow_for_next());
            assert_eq!((report.copy_ticks, report.raw_bytes), ([0; 2], [0; 2]));
            captured.fail_air_history_capture("worker failed");
            let mut ticks = [0; 2];
            captured.decode_air_history_payload(&history, &mut ticks);
            assert_eq!(
                (captured.history.outcome, captured.damage.outcome, ticks),
                (SKIPPED, SKIPPED, [0; 2])
            );
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
            rows: [equipment, 2048, cumulative, 8192, 16384, 1024, 2048, 4096, 1024, 64, 0],
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
        rows: [3, 2048, 2, 8192, 16384, 1024, 2048, 4096, 1024, 64, 0],
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
        let mut first = super::AirWingJob::new();
        let context = super::AirWingCopyContext {
            hour: 0,
            force_full: true,
            base: kit::BASE,
            game_state: kit::GAME_STATE,

            limits: [map_limit, 3, 2],
            naval_limit: 4096,
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
    let mut first = super::AirWingJob::new();
    let context = super::AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,

        limits: [2, 3, 2],
        naval_limit: 1,
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
    let capacities = Capacities::default();
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
            rows: [8192, 2048, 4096, 8192, 16384, 1024, rows, 4096, 1024, 64, 0],
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
        rows: [8192, 2048, 4096, 8192, 16384, 1024, 2, 4096, 1024, 64, 0],
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
    let mut job = super::AirStatsJob::new();
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
        job.clear();
        assert_eq!(job.registration_demand, None);
    }
    // An invalid repeated wing still refuses the table even though the
    // original acquisition already saw the container and pool counts.
    kit::put64(&mut bytes, kit::W0 + cair_wing::REFERENCE_TYPE, 0);
    let memory = kit::memory(&bytes);
    let direct = kit::collect_typed(&memory, [8; SIZED], 256, 0, 0);
    let (split, _, job) = kit::collect_split(&memory, [8; SIZED], 256, 0, 0);
    assert!(matches!(split.regs.outcome, Outcome::Unavailable { .. }));
    assert_eq!(job.registration_demand, None);
    kit::assert_matches_direct(&split, &direct);
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
        source: engine::memory::SliceMemory<'a>,
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
                    linear_air_link(
                        &mut expected,
                        &reference,
                        &links,
                        &mut linear_seen,
                        4,
                        raw
                    ),
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
                    linear_air_link(
                        &mut expected,
                        &reference,
                        &links,
                        &mut linear_seen,
                        3,
                        raw
                    )
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
    for (value, expected, calls) in [(None, None, 3), (Some(8), None, 4), (Some(3), Some(3), 5)]
    {
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
        inner: engine::memory::SliceMemory<'a>,
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
    let mut indexed = AirBasesJob::new();
    let mut linear = AirBasesJob::new();
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
        game_state: 0,
        registration_demand: None,
        aces_active: false,
        census: Vec::new(),
        seen_links: Vec::new(),
        seen_link_index: Vec::new(),
        required_census: 0,
        required_links: 0,
        capacity_charge: crate::raw_memory::Charge::default(),
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
    header(&mut bytes, GAME + cgame_state::COUNTRY_LINKS, LINKS, COUNTRIES);
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
    let mut job = AirBasesJob::new();
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
    assert!(!job.grow_for_next());

    let rows = COUNTRIES as usize;
    let mut air = Captured::with_capacities(&Capacities::default());
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

#[test]
fn air_bases_pools_cover_appendix_worst_case() {
    // Worst-case caps proof: the steady-state pools exceed a
    // conservative span/byte estimate built from the appendix census
    // (396 bases, 5,148 registrations, 57 aces, 439 countries), so live
    // refreshes never take the overflow fallback in steady state.
    use super::AirBasesJob;
    let job = AirBasesJob::new();
    // Census: 2 headers + 439 slots x (pointer, vtable, id, 3 links,
    // canonical pointer). Vectors: manager + vtable + 4 headers + ~500
    // element pointers + identities. Bases detail: ~500 x 6 field spans.
    // Registrations: 5,148 rows x (container pointer, vtable, country,
    // link, pools header) + pools/blocks/headers + ~10k wing takes.
    // Aces: 439 roster headers + 57 x ~13 object/name spans.
    let spans =
        2 + 439 * 7 + 6 + 500 * 2 + 500 * 6 + 5_148 * 5 + 5_148 * 3 + 10_000 + 439 + 57 * 13;
    assert!(
        job.log.spans.capacity() >= spans,
        "span cap {} < worst case {spans}",
        job.log.spans.capacity()
    );
    // Bytes: census words + identities + host blocks + container/pool
    // headers + wing identities + ace objects and names, with headroom.
    let bytes = 439 * 64 + 500 * 48 + 500 * 64 + 5_148 * 64 + 10_000 * 40 + 57 * 512;
    assert!(
        job.log.bytes.capacity() >= bytes,
        "byte cap {} < worst case {bytes}",
        job.log.bytes.capacity()
    );
    assert!(job.census.capacity() >= 439);
    assert!(job.seen_links.capacity() >= 439);
    assert_eq!(job.seen_link_index.len(), job.seen_links.capacity());
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
fn air_history_and_damage_share_slot_7() {
    use replay::rows::TableId;
    for table in [TableId::AirRegionHistory, TableId::AirRegionDamage] {
        assert!(replay::cadence::is_staggered(table), "{table:?}");
        for hour in -24_i32..48 {
            assert_eq!(
                replay::cadence::is_refresh(table, hour),
                hour.rem_euclid(24) == 7,
                "{table:?} at {hour}"
            );
        }
    }
}

#[test]
fn air_history_split_copy_matches_direct_walk() {
    use super::air_history_test_kit as kit;
    use engine::memory::{CountedMemory, ReadCounts};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let direct = kit::direct(&memory);
    let memory = CountedMemory::new(memory);
    let capture = read_accounting::Capture::new();
    let (split, report, mut job) = kit::split(&memory);
    let counts = capture.finish();
    assert!(report.raw_ok);
    kit::assert_matches_direct(&split, &direct);
    // Three records across two sides plus three damage entries.
    assert_eq!(split.history_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.damage_outcome, Outcome::Rows { len: 3 });
    assert_eq!(split.history[0].key.wing_id, 0x1111);
    assert_eq!(split.history[0].region_owner, 7);
    assert_eq!(split.damage[0].cause, 5);
    assert_eq!(split.damage[0].amount, 1000);
    assert_eq!(split.damage[2].channel, 1);
    let anchor = TableId::AirRegionHistory;
    let mut expected = read_accounting::ZERO;
    // 220 scalar bytes, 153 bytes in six leaf gathers and 88 bytes in
    // four root batches. AirRegionDamage shares this actual native anchor.
    expected[anchor.index()] = ReadCounts {
        copy_calls: 20,
        scattered_calls: 6,
        record_calls: 4,
        plan_calls: 0,
        spans: 45,
        bytes: 461,
    };
    assert_eq!(counts, expected);
    let mut refreshes = 0;
    for hour in 0..24 {
        if replay::cadence::is_refresh(anchor, hour) {
            refreshes += 1;
            continue;
        }
        let capture = read_accounting::Capture::new();
        let skipped = copy_air_history_refresh(
            &memory,
            AirHistoryCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
                history_limit: usize::MAX,
                damage_limit: usize::MAX,
            },
            &mut job,
        );
        assert_eq!(capture.finish(), read_accounting::ZERO);
        assert!(!skipped.raw_ok && !job.grow_for_next());
        assert_eq!((skipped.copy_ticks, skipped.raw_bytes), ([0; 2], [0; 2]));
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
        "daily_fixture cohort=air_history_split_copy_matches_direct_walk anchor={anchor:?} counters=[copy,scattered,records,plans,spans,bytes] per_refresh={per_refresh:?} before_24h={:?} after_24h={per_refresh:?} basis=unchanged_refresh_x_24_legacy_hourly_vs_1_daily",
        per_refresh.map(|value| value * 24)
    );
}

#[test]
fn air_history_truncation_matrix_matches_direct_with_same_caps() {
    // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
    // copy→decode against the SAME memory with the SAME pooled caps,
    // asserting identical outcome (incl. len/unread) and identical rows.
    use super::air_history_test_kit as kit;
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    for (history, damage) in [(0, 0), (1, 1), (2, 1), (3, 3), (64, 64)] {
        let direct = kit::direct_with(&memory, history, damage);
        let (split, report, _) = kit::split_with(&memory, history, damage);
        assert!(report.raw_ok, "history={history} damage={damage}");
        kit::assert_matches_direct(&split, &direct);
    }
    let direct = kit::direct_with(&memory, 3, 3);
    assert_eq!(direct.history_outcome, Outcome::Rows { len: 3 });
    assert_eq!(direct.damage_outcome, Outcome::Rows { len: 3 });
}

#[test]
fn air_history_split_copy_matches_direct_walk_on_fault_matrix() {
    // Every fault arm asserts identical admitted rows/outcomes plus per-slot
    // attribution: record faults block both slots (the typed walk fails
    // both tables), damage-only faults block damage while history stays
    // Rows (the typed walk keeps records).
    use super::air_history_test_kit as kit;
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
        kit::MANAGER,
        kit::R0,
        kit::R0 + engine::cair_region_combat_data::OWNER,
        kit::REC0,
        // Damage-scoped faults: history still decodes identical Rows.
        kit::REC0 + engine::sair_wing_combat_data::FIRST_DAMAGE,
        kit::E0,
        kit::E0 + engine::sdamage_entry::AMOUNT,
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
fn air_history_staging_overflow_falls_back_to_typed_walk() {
    use super::air_history_test_kit as kit;
    use super::{AirHistoryCopyContext, AirHistoryJob};
    let bytes = kit::fixture();
    let memory = kit::memory(&bytes);
    let ctx = AirHistoryCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,

        history_limit: 8192,
        damage_limit: 16_384,
    };
    let mut tiny = AirHistoryJob::default();
    let report = super::copy_air_history_refresh(&memory, ctx, &mut tiny);
    assert!(!report.raw_ok);
}

#[test]
fn air_history_pools_cover_appendix_worst_case() {
    let job = AirHistoryJob::new();
    assert!(job.history.len() >= 1_341);
    assert!(job.damage.len() >= 500);
}

#[test]
fn air_history_seed_covers_full_country_by_region_null_pointer_census() {
    use super::air_history_test_kit as kit;
    use engine::cstrategic_air;
    fn put32(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn put64(bytes: &mut [u8], at: usize, value: usize) {
        bytes[at..at + 8].copy_from_slice(&(value as u64).to_le_bytes());
    }
    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put64(bytes, at, data);
        put32(bytes, at + 8, count);
        put32(bytes, at + 12, count);
    }
    let mut bytes = kit::fixture();
    bytes.resize(0xa0000, 0);
    header(
        &mut bytes,
        kit::GAME_STATE + cgame_state::COUNTRIES,
        0x40000,
        440,
    );
    header(
        &mut bytes,
        kit::GAME_STATE + cgame_state::COUNTRY_LINKS,
        0x41000,
        440,
    );
    header(
        &mut bytes,
        kit::MANAGER + cstrategic_air_manager::COUNTRY_AIR,
        0x42000,
        440,
    );
    for slot in 1..440_usize {
        let country = 0x50000 + slot * 16;
        let air = 0x60000 + slot * 512;
        put64(&mut bytes, 0x40000 + slot * 8, country);
        put32(&mut bytes, 0x41000 + slot * 4, slot as u32);
        put64(&mut bytes, country, ccountry::VTABLE_RVA);
        put32(&mut bytes, country + ccountry::ID, slot as u32);
        put64(&mut bytes, 0x42000 + slot * 8, air);
        put64(&mut bytes, air, cstrategic_air::VTABLE_RVA);
        put32(&mut bytes, air + cstrategic_air::COUNTRY, slot as u32);
        // Shared zero bytes deliberately still require every observation.
        header(
            &mut bytes,
            air + cstrategic_air::REGION_COMBAT,
            0x43000,
            512,
        );
    }
    let mut job = AirHistoryJob::new();
    acquire_air_history_pass(
        &mut job,
        &kit::memory(&bytes),
        kit::BASE,
        kit::GAME_STATE,
        0,
        0,
    );
    assert_eq!(job.history_result, Outcome::Rows { len: 0 });
    assert_eq!(job.damage_result, Outcome::Rows { len: 0 });
    assert!(job.raw_bytes[0] >= (439 * 512 * 8) as u64);
    assert!(job.acquired);
    assert!(!job.grow_for_next());
}

#[test]
fn air_history_copy_sizes_cover_documented_read_set() {
    // S3: every Memory::copy size the shared copy pass issues, recorded
    // through the real pass. Census words (4/8), array headers (16),
    // root identity (8/4/16), record head (26) with attribution (4) and
    // copied byte (1), damage headers (16) with entries (12/8).
    use super::air_history_test_kit as kit;
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
    let (_split, report, _) = kit::split(&memory);
    assert!(report.raw_ok);
    let sizes = memory.sizes.borrow().clone();
    for required in [1, 4, 8, 12, 16, 26] {
        assert!(
            sizes.contains(&required),
            "missing size {required} in {sizes:?}"
        );
    }
}

#[test]
fn air_history_preflights_both_buffers_before_source_reads() {
    struct Unread;
    impl Memory for Unread {
        fn copy(&self, _: usize, _: &mut [u8]) -> bool {
            panic!("preflight must not read");
        }
    }
    for short_history in [false, true] {
        let mut job = AirHistoryJob::new();
        if short_history {
            job.history.clear();
        } else {
            job.damage.clear();
        }
        let report = copy_air_history_refresh(
            &Unread,
            AirHistoryCopyContext {
                hour: 0,
                force_full: true,
                base: 0,
                game_state: 0x1000,
                history_limit: 8192,
                damage_limit: 16_384,
            },
            &mut job,
        );
        assert!(!report.raw_ok && !job.acquired);
        assert!(job.grow_for_next());
        job.grow();
        assert!(job.has_storage());
        assert!(!job.grow_for_next());
    }
}

#[test]
fn air_history_owned_walk_preserves_exact_read_order_and_fault_prefix_without_retry() {
    use super::air_history_test_kit as kit;
    struct Observed<'a> {
        inner: engine::memory::SliceMemory<'a>,
        fault: usize,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Observed<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, output.len()));
            if address <= self.fault && self.fault < address.saturating_add(output.len()) {
                return false;
            }
            self.inner.copy(address, output)
        }
    }
    let bytes = kit::fixture();
    for fault in [
        usize::MAX,
        kit::R0 + cair_region_combat_data::OWNER,
        kit::REC0 + sair_wing_combat_data::ATTRIBUTED_COUNTRY,
        kit::E0 + sdamage_entry::AMOUNT,
    ] {
        let memory = Observed {
            inner: kit::memory(&bytes),
            fault,
            calls: Default::default(),
        };
        let expected = kit::direct(&memory);
        let calls = std::mem::take(&mut *memory.calls.borrow_mut());
        let mut job = AirHistoryJob::new();
        acquire_air_history_pass(&mut job, &memory, kit::BASE, kit::GAME_STATE, 8192, 16_384);
        assert!(job.acquired);
        assert_eq!(job.history_result, expected.history_outcome);
        assert_eq!(job.damage_result, expected.damage_outcome);
        assert_eq!(*memory.calls.borrow(), calls, "fault {fault:x}");
    }
}

#[test]
fn air_history_worker_installs_only_admitted_owned_prefix_before_recycle() {
    use super::air_history_test_kit as kit;
    let bytes = kit::fixture();
    let (_, _, mut job) = kit::split(&kit::memory(&bytes));
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.history.rows[4].planes = 123;
    job.history[4].planes = 456;
    captured.decode_air_history_payload(&job, &mut [0; 2]);
    assert_eq!(
        captured.history.rows[4].planes, 123,
        "unadmitted tail is not copied"
    );
    let first = captured.history.rows[0];
    job.history[0].planes ^= 7;
    job.clear();
    assert_eq!(
        captured.history.rows[0], first,
        "recycled storage does not alias frame"
    );
}

#[test]
fn air_history_capacity_charge_follows_owned_payload_growth() {
    use crate::raw_memory::{Charge, Tracker, vector_bytes};
    static LOCAL: Tracker = Tracker::new();
    let mut job = AirHistoryJob {
        capacity_charge: Charge::empty_on(&LOCAL),
        required_history: 3,
        required_damage: 5,
        ..Default::default()
    };
    job.grow();
    let capacity = LOCAL.snapshot().current;
    assert_eq!(
        capacity.payload_vectors,
        vector_bytes(&job.history) + vector_bytes(&job.damage)
    );
    assert_eq!(capacity.auxiliary_vectors, 0);
    assert_eq!(capacity.index_vectors, 0);
    job.clear();
    assert_eq!(LOCAL.snapshot().current, capacity);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Default::default());
}
