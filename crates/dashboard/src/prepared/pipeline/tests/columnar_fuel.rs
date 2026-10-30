use crate::{Source, SourceKey, prepared::pipeline::FuelReference};
use dashboard_base::{
    prepared::series::tests::{Fixture, assert_payloads_equal},
    test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame, source},
};
use dashboard_country_fuel::products::country_fuel::{columnar, prepared::Prepared};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    container::Replay,
    segments::{self, Operator, Start},
};
use replay_data::rows::{
    CountryFuelRow, CountryLinkRow, CountryRow, FuelAwardDay, FuelAwardDayRecipient,
    FuelDemandRead, FuelDemandUnit, Recorded, StateRow, TableId, TableRows, TableSchema, tables,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::AtomicBool},
};

const DAY_HOURS: i32 = replay_data::cadence::STAGGER_PERIOD;

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame, read: bool) {
    let read = read && T::ID == TableId::CountryFuel;
    let sample = if read {
        Sample::FromReadBit
    } else {
        Sample::Carried(frame.hour - 1)
    };
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame, read: bool) {
    write::<tables::Campaign>(walk, frame, read);
    write::<tables::Countries>(walk, frame, read);
    write::<tables::CountryLinks>(walk, frame, read);
    write::<tables::States>(walk, frame, read);
    write::<tables::CountryFuel>(walk, frame, read);
    write::<tables::FuelDemandParts>(walk, frame, read);
    if frame.metrics.table_ns[TableId::CountryFuelConsumers.index()] != 0 {
        walk.table::<replay_data::rows::CountryFuelConsumerRow>(
            TableId::CountryFuelConsumers,
            Input::Unavailable {
                reason: "synthetic timing anchor",
                read: true,
                sample: Sample::FromReadBit,
            },
        )
        .unwrap();
    }
}

fn fuel_slot(day: i32) -> i32 {
    let replay_data::cadence::Cadence::Daily(slot) =
        replay_data::cadence::cadence(TableId::CountryFuel)
    else {
        panic!("daily fuel")
    };
    day * replay_data::cadence::STAGGER_PERIOD + slot as i32
}

fn fuel_frame(hour: i32, germany: u32, fuel: (i64, i64, i64, i64)) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 21,
            canonical_slot: 8,
            tag: b"ITA".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: germany,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 21,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: germany,
                canonical_slot: 7,
            },
        ]));
    *frame.tables[TableId::CountryFuel.index()] =
        TableView::Complete(TableRows::CountryFuel(vec![
            CountryFuelRow {
                country: 11,
                current_raw: fuel.0,
                maximum_raw: fuel.1,
                ring_gain_raw: fuel.2,
                ring_consumed_raw: fuel.3,
                ring_other_raw: 3,
                lend_lease_gain_raw: 7,
                ..Default::default()
            },
            CountryFuelRow {
                country: 21,
                current_raw: 1,
                maximum_raw: 2,
                ring_gain_raw: 3,
                ring_consumed_raw: 4,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            owner: 21,
            controller: 21,
            ..Default::default()
        },
        StateRow {
            id: 2,
            owner: 11,
            controller: 11,
            ..Default::default()
        },
    ]));
    frame
}

fn award_row(hour: i32) -> CountryFuelRow {
    CountryFuelRow {
        country: 11,
        country_instance: Recorded::Value(Some(9)),
        ring_gain_raw: 999,
        award_day: Recorded::Value(FuelAwardDay {
            country_instance: 9,
            continuity_start_hour: Some(977),
            last_hour: hour,
            gains_raw: [24, -48, 72, 0],
            own_gain_raw: 48,
            own_gain_quantity_raw: 17,
            transfers_raw: [24, -72, 96, 24],
            consumed_raw: -24,
            received_raw: [-24, 0, 0],
            recipients: vec![FuelAwardDayRecipient {
                kind: 0,
                reference: 7 << 32 | 29,
                owner: 37,
                name: Some(b"synthetic recipient".to_vec()),
                first_hour: hour - 23,
                last_hour: hour,
                amount_raw: -24,
                ..Default::default()
            }],
            reason: Vec::new(),
        }),
        ..Default::default()
    }
}

fn emit(hours: &mut Vec<columnar::Hour>, prepared: &mut Prepared) -> Vec<usize> {
    std::mem::take(hours)
        .into_iter()
        .map(|hour| {
            let handed = hour.changed_countries();
            prepared.observe_projected(hour).unwrap();
            handed
        })
        .collect()
}

fn parity(label: &str, inputs: &[(DecodedFrame, bool)], restarts: &[usize]) -> Vec<usize> {
    let mut walk = Synthetic::new(columnar::TABLES, 1, inputs[0].0.hour).unwrap();
    let mut normalized = Vec::new();
    let mut cuts = BTreeSet::from([0, inputs.len()]);
    cuts.extend(restarts.iter().copied());
    for (ordinal, (input, read)) in inputs.iter().enumerate() {
        if ordinal > 0 && inputs[ordinal - 1].0.hour.checked_add(1) != Some(input.hour) {
            walk = Synthetic::new(columnar::TABLES, 1, input.hour).unwrap();
            cuts.insert(ordinal);
        }
        write_hour(&mut walk, input, *read);
        walk.end(|_, _, frame, _| normalized.push(frame.clone()))
            .unwrap();
    }
    let cuts: Vec<_> = cuts.into_iter().collect();
    let mut chunks = cuts.windows(2);
    let first = chunks.next().unwrap();
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, normalized[first[0]..first[1]].to_vec())],
    )
    .unwrap();
    for chunk in chunks {
        let next = continuation_records(&replay, 1, normalized[chunk[0]..chunk[1]].to_vec());
        replay = replay.append(&[next]).unwrap();
    }
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey {
            source_id: format!("fuel-columnar-{label}"),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let replay = source.replay_ref();
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut row = FuelReference::new(replay.format);
    let mut projected = Prepared::default();
    let mut projector = projected.projector(replay).unwrap();
    let mut hours: Vec<columnar::Hour> = Vec::new();
    assert!(projector.starts_at(Start::Run { continues: false }));
    assert!(projector.starts_at(Start::Run { continues: true }));
    projector
        .start(Start::Run { continues: false }, 1, inputs[0].0.hour)
        .unwrap();
    let mut walk = Synthetic::new(columnar::TABLES, 1, inputs[0].0.hour).unwrap();
    let mut handed = Vec::new();
    for (ordinal, (input, read)) in inputs.iter().enumerate() {
        let run = ordinal > 0 && inputs[ordinal - 1].0.hour.checked_add(1) != Some(input.hour);
        let block_start = ordinal > 0
            && replay
                .index
                .iter()
                .any(|block| block.first_hour == input.hour);
        if run {
            walk = Synthetic::new(columnar::TABLES, 1, input.hour).unwrap();
        }
        if block_start {
            handed.extend(emit(&mut hours, &mut projected));
            projector = projected.projector(replay).unwrap();
            let start = if run {
                Start::Run { continues: true }
            } else {
                Start::Restart(walk.state())
            };
            assert!(projector.starts_at(start));
            projector.start(start, 1, input.hour).unwrap();
        }
        write_hour(&mut walk, input, *read);
        walk.end(|state, manifest, frame, starts| {
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: u32::from(run),
                        ordinal: ordinal as u32,
                        starts: starts || block_start,
                        opens: starts || block_start,
                        state,
                        manifest,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
        if let Some(block) = replay
            .index
            .iter()
            .find(|block| block.last_hour == input.hour)
        {
            handed.extend(emit(&mut hours, &mut projected));
            row.block(&source, replay, block, &flag).unwrap();
            let mut writer = fixtures[0].writer();
            row.flush(&mut writer, replay, 1, &flag).unwrap();
            writer.commit().unwrap();
            let mut writer = fixtures[1].writer();
            projected
                .flush(&mut writer, 1, replay.format, &flag)
                .unwrap();
            writer.commit().unwrap();
        }
    }
    handed.extend(emit(&mut hours, &mut projected));
    assert_payloads_equal(&fixtures[1].payloads(), &fixtures[0].payloads(), label);
    handed
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = fuel_frame(hour, 9001, (100, 1000, 37, 23));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 31,
            canonical_slot: 9,
            tag: b"empty".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 31,
            canonical_slot: 9,
        });
    }
    let (pack, parts) = FuelDemandRead {
        country_instance: Some(9),
        divisions: vec![FuelDemandUnit {
            reference: 17,
            rate_raw: Some(-77),
            ..Default::default()
        }],
        ..Default::default()
    }
    .split(11)
    .unwrap();
    if let TableView::Complete(TableRows::CountryFuel(rows)) =
        &mut *frame.tables[TableId::CountryFuel.index()]
    {
        rows[0].country_instance = Recorded::Value(Some(9));
        rows[0].demand = Recorded::Value(pack);
    }
    *frame.tables[TableId::FuelDemandParts.index()] =
        TableView::Complete(TableRows::FuelDemandParts(parts));
    frame
}

#[test]
fn days_crossing_restarts_keep_ownership_overlap_and_progressive_pages() {
    let slot = fuel_slot(100);
    let inputs: Vec<_> = (slot - 30..=slot + 75)
        .map(|hour| {
            let mut input = frame(hour);
            if hour >= slot + 30 && hour < slot + 35 {
                if let TableView::Complete(TableRows::States(rows)) =
                    &mut *input.tables[TableId::States.index()]
                {
                    rows.retain(|row| row.owner != 11);
                }
            }
            (
                input,
                (hour - slot).rem_euclid(DAY_HOURS) == 0
                    || [slot - 2, slot + 2, slot + 25].contains(&hour),
            )
        })
        .collect();
    for restart in [vec![], vec![29], vec![31, 43, 54, 79]] {
        parity("boundary days", &inputs, &restart);
    }
}

#[test]
fn every_input_refusal_is_unknown_and_can_restart() {
    let start = fuel_slot(100);
    for table in columnar::TABLES
        .iter()
        .copied()
        .filter(|table| *table != TableId::Campaign)
    {
        for partial in [false, true] {
            let mut refused = frame(start + 2);
            let TableView::Complete(rows) = &*refused.tables[table.index()] else {
                unreachable!()
            };
            refused.tables[table.index()] = if partial {
                TableView::Truncated {
                    rows: rows.clone(),
                    missing: 1,
                }
            } else {
                TableView::Unavailable {
                    reason: "synthetic refusal".into(),
                }
            }
            .into();
            let mut inputs: Vec<_> = (start..=start + 28)
                .map(|hour| (frame(hour), hour == start || hour == start + 24))
                .collect();
            inputs[2] = (refused, true);
            for restart in [vec![], vec![2], vec![3, 9]] {
                parity("partial/unavailable", &inputs, &restart);
            }
        }
    }
}

#[test]
fn aliases_identity_changes_unresolved_owners_and_country_removal() {
    let start = fuel_slot(100);
    let inputs: Vec<_> = (start..=start + 58)
        .map(|hour| {
            let mut input = frame(hour);
            if hour >= start + 4 && hour < start + 9 {
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *input.tables[TableId::CountryLinks.index()]
                {
                    rows.retain(|row| row.id != 11);
                }
            }
            if hour >= start + 10 && hour < start + 15 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                {
                    rows.retain(|row| row.tag != b"GER");
                }
            }
            if hour >= start + 16 && hour < start + 20 {
                if let TableView::Complete(TableRows::States(rows)) =
                    &mut *input.tables[TableId::States.index()]
                {
                    rows.push(StateRow {
                        id: 4,
                        owner: 123456,
                        ..Default::default()
                    });
                }
            }
            if hour >= start + 21 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                {
                    rows.iter_mut().find(|row| row.tag == b"GER").unwrap().id = 9002;
                    rows.push(CountryRow {
                        id: 44,
                        canonical_slot: 7,
                        tag: b"alias".to_vec(),
                        ..Default::default()
                    });
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *input.tables[TableId::CountryLinks.index()]
                {
                    rows.push(CountryLinkRow {
                        id: 9002,
                        canonical_slot: 7,
                    });
                    rows.push(CountryLinkRow {
                        id: 44,
                        canonical_slot: 7,
                    });
                }
            }
            if hour >= start + 30 {
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *input.tables[TableId::CountryLinks.index()]
                {
                    rows.push(CountryLinkRow {
                        id: 12,
                        canonical_slot: 7,
                    });
                }
                if let TableView::Complete(TableRows::CountryFuel(rows)) =
                    &mut *input.tables[TableId::CountryFuel.index()]
                {
                    rows.push(CountryFuelRow {
                        country: 12,
                        current_raw: 888,
                        maximum_raw: 999,
                        ..Default::default()
                    });
                    if hour >= start + 35 {
                        rows.retain(|row| row.country != 11);
                    }
                }
            }
            (input, (hour - start).rem_euclid(6) == 0)
        })
        .collect();
    for restart in [vec![], vec![4, 9, 13, 15, 19, 23, 37]] {
        parity("identities and owners", &inputs, &restart);
    }
}

#[test]
fn awarded_days_and_history_carry_across_runs() {
    let start = fuel_slot(100);
    let inputs: Vec<_> = (start..=start + 76)
        .filter(|hour| !(start + 27..start + 32).contains(hour))
        .map(|hour| {
            let mut input = frame(hour);
            if let TableView::Complete(TableRows::CountryFuel(rows)) =
                &mut *input.tables[TableId::CountryFuel.index()]
            {
                let demand = rows[0].demand.clone();
                rows[0] = award_row(hour);
                rows[0].demand = demand;
                if hour >= start + 40
                    && let Recorded::Value(day) = &mut rows[0].award_day
                {
                    day.continuity_start_hour = Some(start + 17);
                }
            }
            (
                input,
                (hour - start).rem_euclid(12) == 0 || hour == start + 32,
            )
        })
        .collect();
    for restart in [vec![], vec![7, 23, 35, 48]] {
        parity("awards and runs", &inputs, &restart);
    }
}

#[test]
fn quiet_and_one_country_changes_hand_only_reached_countries_at_1500() {
    let start = fuel_slot(100);
    let mut first = scale_frame(1, start, 1500);
    *first.tables[TableId::FuelDemandParts.index()] =
        TableView::Complete(TableRows::FuelDemandParts(Vec::new()));
    *first.tables[TableId::CountryFuel.index()] = TableView::Complete(TableRows::CountryFuel(
        (0..1500)
            .map(|index| CountryFuelRow {
                country: 100_000 + index,
                current_raw: i64::from(index),
                maximum_raw: 9000,
                ..Default::default()
            })
            .collect(),
    ));
    let inputs: Vec<_> = (0..5)
        .map(|ordinal| {
            let mut input = first.clone();
            input.hour += ordinal;
            if ordinal >= 1 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                {
                    rows[10].political_power_raw = Some(i64::from(ordinal));
                }
                if let TableView::Complete(TableRows::States(rows)) =
                    &mut *input.tables[TableId::States.index()]
                {
                    rows[10].movement_infrastructure = i64::from(ordinal);
                }
            }
            if ordinal >= 3
                && let TableView::Complete(TableRows::CountryFuel(rows)) =
                    &mut *input.tables[TableId::CountryFuel.index()]
            {
                rows[777].current_raw += 1;
            }
            (input, ordinal == 0)
        })
        .collect();
    assert_eq!(parity("1500 countries", &inputs, &[]), [1500, 0, 0, 1, 0]);
}

#[test]
fn ownership_changes_hand_only_the_affected_country() {
    let start = fuel_slot(100);
    let mut first = scale_frame(1, start, 1500);
    *first.tables[TableId::CountryFuel.index()] =
        TableView::Complete(TableRows::CountryFuel(Vec::new()));
    *first.tables[TableId::FuelDemandParts.index()] =
        TableView::Complete(TableRows::FuelDemandParts(Vec::new()));
    let inputs: Vec<_> = (0..5)
        .map(|ordinal| {
            let mut input = first.clone();
            input.hour += ordinal;
            if ordinal >= 3
                && let TableView::Complete(TableRows::States(rows)) =
                    &mut *input.tables[TableId::States.index()]
            {
                rows[777].owner = 100_778;
            }
            (input, false)
        })
        .collect();
    assert_eq!(parity("1500 owners", &inputs, &[]), [1500, 0, 0, 1, 0]);
}

#[test]
fn changed_parts_equal_fresh_reads_and_full_status_changes_keep_their_dates() {
    let start = fuel_slot(100);
    let inputs: Vec<_> = (start..=start + 55)
        .map(|hour| {
            let mut input = frame(hour);
            if hour >= start + 5 {
                if let TableView::Complete(TableRows::FuelDemandParts(parts)) =
                    &mut *input.tables[TableId::FuelDemandParts.index()]
                {
                    if let replay_data::rows::FuelDemandPart::Division(unit) = &mut parts[0].part {
                        unit.rate_raw = Some(1234);
                    }
                }
            }
            if (start + 7..start + 11).contains(&hour) {
                let TableView::Complete(rows) = &*input.tables[TableId::FuelDemandParts.index()]
                else {
                    unreachable!()
                };
                input.tables[TableId::FuelDemandParts.index()] = TableView::Truncated {
                    rows: rows.clone(),
                    missing: if hour < start + 9 { 1 } else { 2 },
                }
                .into();
            }
            if hour >= start + 12 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                {
                    rows.push(CountryRow {
                        id: 99,
                        canonical_slot: 10,
                        tag: vec![0xff],
                        ..Default::default()
                    });
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *input.tables[TableId::CountryLinks.index()]
                {
                    rows.push(CountryLinkRow {
                        id: 99,
                        canonical_slot: 10,
                    });
                }
            }
            (
                input,
                [
                    start,
                    start + 6,
                    start + 8,
                    start + 10,
                    start + 11,
                    start + 24,
                    start + 48,
                ]
                .contains(&hour),
            )
        })
        .collect();
    for restart in [vec![], vec![6, 8, 10, 12, 25]] {
        parity("parts and equal reads", &inputs, &restart);
    }
}

#[test]
fn later_runs_opening_with_refused_joins_start_cold_and_recover_current_fuel() {
    let start = fuel_slot(100);
    for refused in [TableId::Countries, TableId::CountryLinks] {
        let inputs: Vec<_> = [
            start,
            start + 1,
            start + 8,
            start + 9,
            start + 10,
            start + 35,
        ]
        .into_iter()
        .map(|hour| {
            let mut input = frame(hour);
            if hour >= start + 8 {
                if let TableView::Complete(TableRows::CountryFuel(rows)) =
                    &mut *input.tables[TableId::CountryFuel.index()]
                {
                    rows[0].current_raw = 12345;
                }
            }
            if hour == start + 8 {
                *input.tables[refused.index()] = TableView::Unavailable {
                    reason: "synthetic run opening refusal".into(),
                };
            }
            (input, true)
        })
        .collect();
        for restart in [vec![], vec![3], vec![2, 3, 4]] {
            parity("later run refused join", &inputs, &restart);
        }
    }
}

#[test]
fn off_clock_slot_reads_in_quiet_spans_use_the_productions_hourly_fallback() {
    let start = fuel_slot(100);
    let inputs: Vec<_> = (start..=start + 53)
        .map(|hour| {
            let mut input = frame(hour);
            if [start + 5, start + 29].contains(&hour) {
                input.metrics.table_ns[TableId::CountryFuelConsumers.index()] = 1;
            }
            (input, false)
        })
        .collect();
    for restart in [vec![], vec![4, 6, 28, 30]] {
        parity("quiet off-clock slot read", &inputs, &restart);
    }
}

#[test]
fn branch_returns_reload_the_reference_and_keep_ordered_history() {
    fn normalized(branch: u64, inputs: &[(DecodedFrame, bool)]) -> Vec<DecodedFrame> {
        let mut walk = Synthetic::new(columnar::TABLES, branch, inputs[0].0.hour).unwrap();
        let mut frames = Vec::new();
        for (input, read) in inputs {
            write_hour(&mut walk, input, *read);
            walk.end(|_, _, frame, _| frames.push(frame.clone()))
                .unwrap();
        }
        frames
    }
    let start = fuel_slot(100);
    let mut missing = frame(start + 1);
    if let TableView::Complete(TableRows::CountryFuel(rows)) =
        &mut *missing.tables[TableId::CountryFuel.index()]
    {
        rows.retain(|row| row.country != 11);
    }
    let first = normalized(1, &[(frame(start), true), (missing, false)]);
    let second = normalized(2, &[(frame(start + 4), true), (frame(start + 5), false)]);
    let returned = normalized(1, &[(frame(start + 2), false), (frame(start + 3), true)]);
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, first)]).unwrap();
    replay = replay.append(&[records(2, second)]).unwrap();
    let continuation = continuation_records(&replay, 1, returned);
    replay = replay.append(&[continuation]).unwrap();
    assert_eq!(
        replay
            .index
            .iter()
            .map(|block| block.branch)
            .collect::<Vec<_>>(),
        [1, 2, 1]
    );
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        SourceKey {
            source_id: "fuel-branch-return".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let replay = source.replay_ref();
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut rows: std::collections::BTreeMap<u64, FuelReference> =
        std::collections::BTreeMap::new();
    let mut projected: std::collections::BTreeMap<u64, Prepared> =
        std::collections::BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (ordinal, block) in replay.index.iter().enumerate() {
        let prepared = projected.entry(block.branch).or_default();
        let mut projector = prepared.projector(replay).unwrap();
        let mut hours: Vec<columnar::Hour> = Vec::new();
        let start = Start::Run {
            continues: !seen.insert(block.branch),
        };
        assert!(projector.starts_at(start));
        projector
            .start(start, block.branch, block.first_hour)
            .unwrap();
        let mut walk = Synthetic::new(columnar::TABLES, block.branch, block.first_hour).unwrap();
        let mut at = 0;
        replay
            .read_block_alone(
                block,
                columnar::TABLES,
                |_, input| {
                    write_hour(
                        &mut walk,
                        input,
                        input.metrics.table_ns[TableId::CountryFuel.index()] != 0,
                    );
                    walk.end(|state, manifest, frame, starts| {
                        projector
                            .hour(
                                &segments::Hour {
                                    branch: block.branch,
                                    hour: frame.hour,
                                    run: ordinal as u32,
                                    ordinal: at,
                                    starts,
                                    opens: starts,
                                    state,
                                    manifest,
                                },
                                &mut hours,
                            )
                            .unwrap();
                    })
                    .unwrap();
                    at += 1;
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        emit(&mut hours, prepared);
        let row = rows
            .entry(block.branch)
            .or_insert_with(|| FuelReference::new(replay.format));
        row.block(&source, replay, block, &flag).unwrap();
        let mut writer = fixtures[0].writer();
        row.flush(&mut writer, replay, block.branch, &flag).unwrap();
        writer.commit().unwrap();
        let mut writer = fixtures[1].writer();
        prepared
            .flush(&mut writer, block.branch, replay.format, &flag)
            .unwrap();
        writer.commit().unwrap();
        if replay
            .index
            .get(ordinal + 1)
            .is_none_or(|next| next.branch != block.branch)
        {
            row.reload().unwrap();
        }
    }
    assert_payloads_equal(
        &fixtures[1].payloads(),
        &fixtures[0].payloads(),
        "branch return",
    );
}
