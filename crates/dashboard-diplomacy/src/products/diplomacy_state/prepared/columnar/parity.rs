use super::*;
use dashboard_base::{
    prepared::{
        frame_changes::FrameChangeTracker,
        series::tests::{Fixture, assert_payloads_equal},
    },
    test_fixtures,
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{rows::*, scope::CaptureMode};
use replay_frame::frame::TableView;

const A: u32 = 1000;
const B: u32 = 1001;
const EMPTY: u32 = 1002;

fn frame(hour: i32, read: i32) -> DecodedFrame {
    let mut frame = test_fixtures::source(1, hour, None);
    frame.metrics.table_ns.fill(0);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        [(A, 1, "first"), (B, 2, "second"), (EMPTY, 3, "empty")]
            .into_iter()
            .map(|(id, canonical_slot, tag)| CountryRow {
                id,
                canonical_slot,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(A, 1), (B, 2), (EMPTY, 3), (9000, 1)]
            .into_iter()
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .collect(),
    ));
    let mut reads: Vec<_> = [TableId::RelationOpinions, TableId::RelationTuples]
        .into_iter()
        .flat_map(|table| {
            [A, B, EMPTY, 9000, 99000]
                .into_iter()
                .map(move |owner| SliceReadRow {
                    table: table.index() as u32,
                    owner,
                    sample_hour: read,
                })
        })
        .collect();
    reads.sort_by_key(RowCodec::key);
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    *frame.tables[TableId::RelationOpinions.index()] =
        TableView::Complete(TableRows::RelationOpinions(vec![
            RelationOpinionRow {
                owner_country: A,
                target_country: B,
                opinion: -7,
            },
            RelationOpinionRow {
                owner_country: B,
                target_country: A,
                opinion: i32::MAX,
            },
        ]));
    *frame.tables[TableId::RelationTuples.index()] =
        TableView::Complete(TableRows::RelationTuples(vec![RelationTupleRow {
            owner_country: A,
            target_country: B,
            type_id: 7,
            ordinal: 0,
            first_country: A,
            second_country: B,
            start_hour: -40,
            end_hour: 777,
            cancel: 0,
            retired: 0,
        }]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 7,
            literal: b"synthetic relation".to_vec(),
        }]));
    *frame.tables[TableId::IncomingDiplomaticActions.index()] =
        TableView::Complete(TableRows::IncomingDiplomaticActions(vec![
            IncomingDiplomaticActionRow {
                country: A,
                ordinal: 0,
                handle: u64::MAX,
                actor: B,
                recipient: A,
                original_actor: A,
                original_recipient: B,
                hour: -10,
                last_command_hour: -3,
                ..Default::default()
            },
        ]));
    frame
}
fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame, carried: Option<TableId>) {
    let read = frame.metrics.table_ns[T::ID.index()] != 0;
    let sample = if carried == Some(T::ID) {
        Sample::Carried(frame.hour - 2)
    } else {
        Sample::FromReadBit
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
fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame, carried: Option<TableId>) {
    write::<tables::Campaign>(walk, frame, carried);
    write::<tables::Countries>(walk, frame, carried);
    write::<tables::CountryLinks>(walk, frame, carried);
    write::<tables::SliceReads>(walk, frame, carried);
    write::<tables::RelationOpinions>(walk, frame, carried);
    write::<tables::RelationTuples>(walk, frame, carried);
    write::<tables::IncomingDiplomaticActions>(walk, frame, carried);
    write::<tables::LexicalTokens>(walk, frame, carried);
}
fn flush(prepared: &mut Prepared, fixture: &Fixture) {
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    prepared.save(&mut writer, 1).unwrap();
    writer.commit().unwrap();
}
fn emit(hours: &mut Vec<Hour>, prepared: &mut Prepared, fixture: &Fixture) {
    for hour in std::mem::take(hours) {
        let at = hour.hour;
        prepared.observe_projected(hour).unwrap();
        if at.rem_euclid(3) == 0 {
            flush(prepared, fixture);
        }
    }
}
fn parity(
    inputs: &[DecodedFrame],
    restart: Option<usize>,
    carried: Option<TableId>,
    restore: bool,
) {
    compare(inputs, restart, carried, restore, false);
}
/// Both paths over `inputs`; with `cross`, a later run goes on with the run before's projector.
fn compare(
    inputs: &[DecodedFrame],
    restart: Option<usize>,
    carried: Option<TableId>,
    restore: bool,
    cross: bool,
) {
    let mut walk = Synthetic::new(super::super::super::block::TABLES, 1, inputs[0].hour).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    let fixtures = [Fixture::new(), Fixture::new()];
    let (mut row, mut projected) = (Prepared::default(), Prepared::default());
    let mut tracker = FrameChangeTracker::new();
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let mut hours: Vec<Hour> = Vec::new();
    assert!(projector.starts_at(Start::Run { continues: false }));
    projector
        .start(Start::Run { continues: false }, 1, inputs[0].hour)
        .unwrap();
    for (ordinal, input) in inputs.iter().enumerate() {
        let run = ordinal > 0 && inputs[ordinal - 1].hour.checked_add(1) != Some(input.hour);
        let opens = run || ordinal == 0 || restart == Some(ordinal);
        if ordinal > 0 && opens {
            emit(&mut hours, &mut projected, &fixtures[1]);
            if run {
                walk = Synthetic::new(super::super::super::block::TABLES, 1, input.hour).unwrap();
            }
            if run && cross {
                projector.cross(input.hour).unwrap();
            } else {
                projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
                if restore {
                    flush(&mut row, &fixtures[0]);
                    flush(&mut projected, &fixtures[1]);
                    row = Prepared::restore(
                        fixtures[0]
                            .store
                            .read(&fixtures[0].key, &state_section(1), 0)
                            .unwrap()
                            .unwrap(),
                    )
                    .unwrap();
                    projected = Prepared::restore(
                        fixtures[1]
                            .store
                            .read(&fixtures[1].key, &state_section(1), 0)
                            .unwrap()
                            .unwrap(),
                    )
                    .unwrap();
                }
                let start = if run {
                    Start::Run { continues: true }
                } else {
                    Start::Restart(walk.state())
                };
                assert!(projector.starts_at(start));
                projector.start(start, 1, input.hour).unwrap();
            }
            // Production retains Prepared at block end; only its frame tracker starts cold.
            tracker.invalidate();
        }
        write_hour(&mut walk, input, carried);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &super::needs().unwrap(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            row.observe(
                frame,
                FormatVersion::CURRENT,
                &tracker.observe(frame, FormatVersion::CURRENT),
                &AtomicBool::new(false),
            )
            .unwrap();
            if frame.hour.rem_euclid(3) == 0 {
                flush(&mut row, &fixtures[0]);
            }
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: u32::from(run),
                        ordinal: ordinal as u32,
                        starts: starts || opens,
                        opens,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap();
        })
        .unwrap();
    }
    emit(&mut hours, &mut projected, &fixtures[1]);
    flush(&mut row, &fixtures[0]);
    flush(&mut projected, &fixtures[1]);
    let expected = fixtures[0].payloads();
    for section in [
        "diplomacy-series-1",
        "diplomacy-state-1",
        "diplomacy-state-1-catalogue",
    ] {
        assert!(
            expected.keys().any(|(name, _)| name == section),
            "{section}"
        );
    }
    assert_payloads_equal(
        &fixtures[1].payloads(),
        &expected,
        &format!("restart={restart:?} carried={carried:?} restore={restore} cross={cross}"),
    );
}
/// One projector's hours over consecutive `inputs`.
fn one_projector(inputs: &[DecodedFrame]) -> Vec<Hour> {
    let mut walk = Synthetic::new(super::super::super::block::TABLES, 1, inputs[0].hour).unwrap();
    let mut world_held: Option<dashboard_world::World> = None;
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let mut hours: Vec<Hour> = Vec::new();
    projector
        .start(Start::Run { continues: false }, 1, inputs[0].hour)
        .unwrap();
    for (ordinal, input) in inputs.iter().enumerate() {
        write_hour(&mut walk, input, None);
        walk.end(|state, manifest, frame, starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                starts,
                &super::needs().unwrap(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal: ordinal as u32,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap()
        })
        .unwrap();
    }
    hours
}
fn script() -> Vec<DecodedFrame> {
    (0..34)
        .map(|hour| {
            let read = match hour {
                0..5 => 0,
                5..10 => 5,
                10..16 => 10,
                _ => 16,
            };
            let mut f = frame(hour, read);
            if hour >= 5 {
                if let TableView::Complete(TableRows::RelationTuples(rows)) =
                    &mut *f.tables[TableId::RelationTuples.index()]
                {
                    rows[0].cancel = 255;
                    rows[0].retired = 1;
                    rows[0].end_hour = -1;
                }
            }
            if hour == 7 {
                let rows = TableRows::RelationTuples(vec![]);
                *f.tables[TableId::RelationTuples.index()] =
                    TableView::Truncated { rows, missing: 1 };
            }
            if (8..10).contains(&hour) {
                *f.tables[TableId::SliceReads.index()] = TableView::Unavailable {
                    reason: format!("refused {hour}"),
                };
            }
            if hour == 11 {
                *f.tables[TableId::Countries.index()] = TableView::Unavailable {
                    reason: "identity refusal".into(),
                };
            }
            if hour >= 12 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *f.tables[TableId::Countries.index()]
                {
                    rows[1].id = 2001;
                    rows.sort_by_key(RowCodec::key);
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *f.tables[TableId::CountryLinks.index()]
                {
                    rows.push(CountryLinkRow {
                        id: 2001,
                        canonical_slot: 2,
                    });
                    rows.sort_by_key(RowCodec::key);
                }
            }
            if hour == 14 {
                *f.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
                    reason: "token refusal".into(),
                };
            }
            if hour >= 16 {
                if let TableView::Complete(TableRows::RelationOpinions(rows)) =
                    &mut *f.tables[TableId::RelationOpinions.index()]
                {
                    rows.push(RelationOpinionRow {
                        owner_country: 9000,
                        target_country: B,
                        opinion: 0,
                    });
                    rows.push(RelationOpinionRow {
                        owner_country: 99000,
                        target_country: B,
                        opinion: 2,
                    });
                    rows.sort_by_key(RowCodec::key);
                }
                if let TableView::Complete(TableRows::RelationTuples(rows)) =
                    &mut *f.tables[TableId::RelationTuples.index()]
                {
                    let mut orphan = rows[0].clone();
                    orphan.owner_country = 99000;
                    rows.push(orphan);
                }
            }
            f
        })
        .collect()
}
#[test]
fn whole_walk_restarts_and_restored_publication_match_every_section() {
    let inputs = script();
    for restart in [
        None,
        Some(1),
        Some(5),
        Some(8),
        Some(9),
        Some(10),
        Some(12),
        Some(16),
        Some(25),
    ] {
        parity(&inputs, restart, None, restart == Some(12));
    }
}
#[test]
fn rowless_scheduled_reads_and_deadlines_need_no_changed_rows() {
    let mut inputs: Vec<_> = (0..52).map(|hour| frame(hour, 0)).collect();
    for f in &mut inputs {
        *f.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![]));
        *f.tables[TableId::RelationOpinions.index()] =
            TableView::Complete(TableRows::RelationOpinions(vec![]));
        *f.tables[TableId::RelationTuples.index()] =
            TableView::Complete(TableRows::RelationTuples(vec![]));
    }
    for restart in [None, Some(7), Some(25)] {
        parity(&inputs, restart, None, false);
    }
    let stale: Vec<_> = (0..52).map(|hour| frame(hour, 0)).collect();
    parity(&stale, Some(23), None, false);
}
#[test]
fn proposals_preserve_ambiguous_gaps_duplicate_handles_and_last_commands() {
    let mut inputs: Vec<_> = [0, 1, 6, 7, 8, 9, 10, 11]
        .into_iter()
        .map(|hour| frame(hour, hour))
        .collect();
    for f in &mut inputs {
        f.metrics.table_ns[TableId::IncomingDiplomaticActions.index()] = 1;
        if f.hour >= 7 {
            if let TableView::Complete(TableRows::IncomingDiplomaticActions(rows)) =
                &mut *f.tables[TableId::IncomingDiplomaticActions.index()]
            {
                rows[0].ordinal = 3;
                rows[0].status = 255;
                rows[0].action_type = -12345;
                rows[0].last_command_hour = f.hour - 1;
            }
        }
        if f.hour == 8 {
            if let TableView::Complete(TableRows::IncomingDiplomaticActions(rows)) =
                &mut *f.tables[TableId::IncomingDiplomaticActions.index()]
            {
                let mut duplicate = rows[0].clone();
                duplicate.ordinal = 4;
                rows.push(duplicate);
            }
        }
        if f.hour >= 10 {
            *f.tables[TableId::IncomingDiplomaticActions.index()] =
                TableView::Complete(TableRows::IncomingDiplomaticActions(vec![]));
        }
    }
    for restart in [None, Some(3), Some(5)] {
        parity(&inputs, restart, None, true);
    }
}
#[test]
fn partial_counts_refusal_reasons_and_carried_samples_survive_restarts() {
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::SliceReads,
        TableId::RelationOpinions,
        TableId::RelationTuples,
        TableId::IncomingDiplomaticActions,
        TableId::LexicalTokens,
    ] {
        let mut inputs: Vec<_> = (0..7).map(|hour| frame(hour, hour)).collect();
        for (at, missing) in [(1, 1), (2, 2)] {
            let TableView::Complete(rows) = &*inputs[at].tables[table.index()] else {
                panic!("complete")
            };
            let rows = rows.clone();
            *inputs[at].tables[table.index()] = TableView::Truncated { rows, missing };
        }
        for at in [3, 4] {
            *inputs[at].tables[table.index()] = TableView::Unavailable {
                reason: format!("refused {at}"),
            };
        }
        parity(&inputs, Some(2), Some(table), false);
        parity(&inputs, Some(5), Some(table), false);
    }
}
#[test]
fn unchanged_rows_share_their_values_and_changed_values_share_their_targets() {
    let mut inputs: Vec<_> = (0..3).map(|hour| frame(hour, hour)).collect();
    if let TableView::Complete(TableRows::RelationOpinions(rows)) =
        &mut *inputs[2].tables[TableId::RelationOpinions.index()]
    {
        rows[0].opinion = -8;
    }
    if let TableView::Complete(TableRows::RelationTuples(rows)) =
        &mut *inputs[2].tables[TableId::RelationTuples.index()]
    {
        rows[0].end_hour = 778;
        rows[0].cancel = 1;
    }
    let hours = one_projector(&inputs);
    assert!(hours[1].ids.is_none() && hours[2].ids.is_none());
    let a = hours[0]
        .ids
        .as_ref()
        .unwrap()
        .position(&DiplomacyCountry {
            tag: "first".into(),
            slot: 1,
            country_id: A,
        })
        .unwrap();
    let values = |hour: usize| {
        let opinions = &hours[hour].opinions.as_ref().unwrap().current;
        Arc::clone(&opinions.iter().find(|opinion| opinion.0 == a).unwrap().2)
    };
    let rows = |hour: usize| {
        let owners = &hours[hour].tuples.as_ref().unwrap().owners;
        Arc::clone(&owners.iter().find(|owner| owner.0 == A).unwrap().2)
    };
    assert!(Arc::ptr_eq(&values(0), &values(1)));
    assert!(!Arc::ptr_eq(&values(1), &values(2)));
    assert!(Arc::ptr_eq(&values(1).targets, &values(2).targets));
    assert!(Arc::ptr_eq(&rows(0), &rows(1)));
    assert!(!Arc::ptr_eq(&rows(1), &rows(2)));
    assert!(Arc::ptr_eq(&rows(1).identity, &rows(2).identity));
    for restart in [None, Some(1), Some(2)] {
        parity(&inputs, restart, None, false);
    }
}

#[test]
fn one_changed_stamp_and_pair_only_select_their_owner_with_fifteen_hundred_countries() {
    let mut first = frame(0, 0);
    *first.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (A..A + 1500)
            .map(|id| CountryRow {
                id,
                canonical_slot: id - A + 1,
                tag: format!("country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *first.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (A..A + 1500)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id - A + 1,
            })
            .collect(),
    ));
    let mut reads: Vec<_> = [TableId::RelationOpinions, TableId::RelationTuples]
        .into_iter()
        .flat_map(|table| {
            (A..A + 1500).map(move |owner| SliceReadRow {
                table: table.index() as u32,
                owner,
                sample_hour: 0,
            })
        })
        .collect();
    reads.sort_by_key(RowCodec::key);
    *first.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    let mut second = first.clone();
    second.hour = 1;
    if let TableView::Complete(TableRows::SliceReads(rows)) =
        &mut *second.tables[TableId::SliceReads.index()]
    {
        rows.iter_mut()
            .find(|row| row.table == TableId::RelationOpinions.index() as u32 && row.owner == A)
            .unwrap()
            .sample_hour = 1;
    }
    if let TableView::Complete(TableRows::RelationOpinions(rows)) =
        &mut *second.tables[TableId::RelationOpinions.index()]
    {
        rows[0].opinion = 9;
    }
    let inputs = [first, second];
    let hours = one_projector(&inputs);
    assert_eq!(hours[1].opinions.as_ref().unwrap().current.len(), 1);
    assert!(hours[1].tuples.as_ref().unwrap().countries.is_empty());
    parity(&inputs, None, None, false);
}

#[test]
fn later_runs_keep_the_ordered_scans_baseline_rule() {
    let inputs: Vec<_> = [0, 1, 6, 7, 24, 25]
        .into_iter()
        .map(|hour| frame(hour, 0))
        .collect();
    parity(&inputs, None, None, false);
    parity(&inputs, Some(3), None, false);
}

#[test]
fn repeated_tuple_reads_keep_the_last_token_for_shared_recorded_spellings() {
    let mut inputs: Vec<_> = (0..4).map(|hour| frame(hour, hour)).collect();
    for f in &mut inputs {
        if let TableView::Complete(TableRows::RelationTuples(rows)) =
            &mut *f.tables[TableId::RelationTuples.index()]
        {
            let mut another = rows[0].clone();
            another.owner_country = B;
            another.type_id = 8;
            rows.push(another);
        }
        *f.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 7,
                    literal: b"same synthetic spelling".to_vec(),
                },
                LexicalTokenRow {
                    token: 8,
                    literal: b"same synthetic spelling".to_vec(),
                },
            ]));
        if f.hour % 2 == 1 {
            if let TableView::Complete(TableRows::SliceReads(rows)) =
                &mut *f.tables[TableId::SliceReads.index()]
            {
                for row in rows.iter_mut().filter(|row| {
                    row.table == TableId::RelationTuples.index() as u32 && row.owner == B
                }) {
                    row.sample_hour = f.hour - 1;
                }
            }
        }
    }
    for restart in [None, Some(2)] {
        parity(&inputs, restart, None, false);
    }
}

#[test]
fn a_walk_going_on_into_a_later_run_matches_fresh_projectors() {
    let later: Vec<_> = [0, 1, 6, 7, 24, 25]
        .into_iter()
        .map(|hour| frame(hour, 0))
        .collect();
    // The hour before the gap is refused, so the later run's first hour admits a baseline.
    let mut refused: Vec<_> = [0, 1, 2, 6, 7]
        .into_iter()
        .map(|hour| frame(hour, hour.min(1)))
        .collect();
    *refused[2].tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "refused before the gap".into(),
    };
    for inputs in [&later, &refused] {
        for cross in [false, true] {
            compare(inputs, None, None, false, cross);
        }
    }
}

#[test]
fn full_refresh_hours_read_every_owner_without_stamps() {
    let inputs: Vec<_> = (0..30)
        .map(|hour| {
            let mut f = frame(hour, 0);
            *f.tables[TableId::SliceReads.index()] =
                TableView::Complete(TableRows::SliceReads(vec![]));
            // Off its slot, the queue's timed read is a full refresh; at its slot it is not.
            if [5, 6, 20, 22].contains(&hour) {
                f.metrics.table_ns[TableId::IncomingDiplomaticActions.index()] = 1;
            }
            if hour == 6 {
                *f.tables[TableId::SliceReads.index()] = TableView::Unavailable {
                    reason: "refused at a full refresh".into(),
                };
            }
            if let TableView::Complete(TableRows::RelationOpinions(rows)) =
                &mut *f.tables[TableId::RelationOpinions.index()]
            {
                rows.extend([(9000, B, 3), (99000, B, 2)].map(
                    |(owner_country, target_country, opinion)| RelationOpinionRow {
                        owner_country,
                        target_country,
                        opinion,
                    },
                ));
                rows.sort_by_key(RowCodec::key);
            }
            if let TableView::Complete(TableRows::RelationTuples(rows)) =
                &mut *f.tables[TableId::RelationTuples.index()]
            {
                let mut orphan = rows[0].clone();
                orphan.owner_country = 99000;
                rows.push(orphan);
            }
            f
        })
        .collect();
    assert!(evidence::full_refresh(&inputs[5], FormatVersion::CURRENT));
    assert!(!evidence::full_refresh(&inputs[22], FormatVersion::CURRENT));
    for restart in [None, Some(5), Some(6), Some(7), Some(21)] {
        parity(&inputs, restart, None, false);
    }
}

#[test]
fn a_links_only_change_reads_the_identities_again() {
    let inputs: Vec<_> = (0..12)
        .map(|hour| {
            let mut f = frame(hour, hour);
            if (4..8).contains(&hour)
                && let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *f.tables[TableId::CountryLinks.index()]
            {
                rows.iter_mut()
                    .find(|row| row.id == 9000)
                    .unwrap()
                    .canonical_slot = 2;
            }
            if let TableView::Complete(TableRows::RelationOpinions(rows)) =
                &mut *f.tables[TableId::RelationOpinions.index()]
            {
                rows.push(RelationOpinionRow {
                    owner_country: 9000,
                    target_country: A,
                    opinion: 4,
                });
                rows.sort_by_key(RowCodec::key);
            }
            if let TableView::Complete(TableRows::RelationTuples(rows)) =
                &mut *f.tables[TableId::RelationTuples.index()]
            {
                let mut aliased = rows[0].clone();
                aliased.owner_country = 9000;
                aliased.target_country = A;
                rows.push(aliased);
            }
            f
        })
        .collect();
    let hours = one_projector(&inputs);
    for (hour, read) in [(3, false), (4, true), (5, false), (8, true)] {
        assert_eq!(hours[hour].ids.is_some(), read, "{hour}");
    }
    for restart in [None, Some(4), Some(6), Some(8)] {
        parity(&inputs, restart, None, false);
    }
}

#[test]
fn a_restart_builds_a_baseline_only_after_a_refused_hour() {
    for refused in [false, true] {
        let mut inputs: Vec<_> = (0..3).map(|hour| frame(hour, 0)).collect();
        if refused {
            *inputs[1].tables[TableId::Countries.index()] = TableView::Unavailable {
                reason: "refused before the restart".into(),
            };
        }
        let mut walk = Synthetic::new(super::super::super::block::TABLES, 1, 0).unwrap();
        let mut world_held: Option<dashboard_world::World> = None;
        for input in &inputs[..2] {
            write_hour(&mut walk, input, None);
            walk.end(|_, _, _, _| ()).unwrap();
        }
        let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        let mut hours: Vec<Hour> = Vec::new();
        projector.start(Start::Restart(walk.state()), 1, 2).unwrap();
        write_hour(&mut walk, &inputs[2], None);
        walk.end(|state, manifest, frame, walk_starts| {
            let world = dashboard_world::World::at(
                &mut world_held,
                walk_starts,
                &super::needs().unwrap(),
                (
                    replay_data::format::FormatVersion::CURRENT,
                    replay_data::scope::CaptureMode::Full,
                ),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: frame.hour,
                        run: 0,
                        ordinal: 2,
                        starts: true,
                        opens: true,
                        state,
                        manifest,
                        world,
                    },
                    &mut hours,
                )
                .unwrap()
        })
        .unwrap();
        assert_eq!(hours[0].baseline, refused);
        let opinions = hours[0].opinions.as_ref().unwrap();
        assert!(opinions.current.is_empty());
        assert_eq!(opinions.baseline.is_empty(), !refused);
        parity(&inputs, Some(2), None, false);
    }
}
