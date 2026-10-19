//! `read_block_changes` against `read_block_alone`, hour by hour.

use super::super::tests::{CAMPAIGN, interleaved_copy, records, source};
use super::*;
use crate::{
    frame::{FrameEncoder, not_selected},
    rows::{CountryLinkRow, CountryRow, SunkShipsRow},
};

/// The hour within its block: 1 at each block's first hour.
fn local(hour: Hour) -> Hour {
    (hour - 1) % 720 + 1
}

/// Three blocks of one branch: 40, 40 and 10 hours.
fn hours() -> Vec<Hour> {
    (1..=40).chain(721..=760).chain(1441..=1450).collect()
}

/// Country 1's command power: constant, then a steady ramp the block
/// predicts (5..=15), a stop, a one-off change (18), and from 31 a ramp of
/// 1, 1, 2 an hour, which only the rate prediction follows.
fn command_power(t: Hour) -> i64 {
    let ramp = |steps: Hour| i64::from(steps + steps / 3);
    match t {
        ..=4 => 100,
        5..=15 => 100 + 5 * i64::from(t - 4),
        16..=17 => 155,
        18..=30 => 195,
        31..=39 => 195 + ramp(t - 30),
        _ => 195 + ramp(9),
    }
}

/// Countries: country 1's stability falls by 3 every hour (predicted, never
/// stored after its first hours), country 3's war support changes at 3 and
/// 21, country 4 is inserted at 20, country 2 deleted at 30, and the table is
/// unavailable at 24.
fn countries(hour: Hour) -> TableView {
    let t = local(hour);
    if t == 24 {
        return TableView::Unavailable {
            reason: "synthetic failed country read".into(),
        };
    }
    let block = i64::from((hour - 1) / 720);
    let war_support = match t {
        ..=2 => 0,
        3..=20 => 7,
        _ => 9,
    };
    let rows = [1_u32, 2, 3, 4]
        .into_iter()
        .filter(|&id| !(id == 2 && t >= 30 || id == 4 && t < 20))
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            command_power_raw: if id == 1 {
                command_power(t) + 1000 * block
            } else {
                i64::from(id)
            },
            stability_raw: if id == 1 {
                10_000 - 3 * i64::from(hour)
            } else {
                0
            },
            war_support_raw: if id == 3 { war_support } else { 0 },
            ..Default::default()
        })
        .collect();
    TableView::Complete(TableRows::Countries(rows))
}

fn fixture() -> Replay {
    let mut captured = records(1, &[1]);
    captured.pop();
    let mut encoder = FrameEncoder::new();
    // A static table large enough that later keyframes anchor to the first.
    let links = TableRows::CountryLinks(
        (0..3_000_u32)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id.wrapping_mul(2_654_435_761),
            })
            .collect(),
    );
    for hour in hours() {
        let t = local(hour);
        let mut frame = source(1, hour);
        *frame.tables[TableId::Countries.index()] = countries(hour);
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(links.clone());
        *frame.tables[TableId::SunkShips.index()] = match t {
            26 | 27 => TableView::Truncated {
                rows: TableRows::SunkShips(vec![SunkShipsRow::default()]),
                missing: 1,
            },
            28 => TableView::Truncated {
                rows: TableRows::SunkShips(vec![SunkShipsRow::default()]),
                missing: 2,
            },
            _ => TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow::default()])),
        };
        if matches!(t, 22 | 23) {
            *frame.tables[TableId::WorldTension.index()] = TableView::Unavailable {
                reason: "synthetic tension failure".into(),
            };
        }
        frame.metrics.table_ns[TableId::Clock.index()] = 1;
        // Countries changes at 12 and 21 without being read: untimed.
        frame.metrics.table_ns[TableId::Countries.index()] = u64::from(t != 12 && t != 21);
        frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
        frame.metrics.table_ns[TableId::WorldTension.index()] = u64::from(t % 2 == 1);
        captured.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(CAMPAIGN.into(), &[captured]).unwrap()
}

fn selections() -> Vec<(&'static str, Vec<TableFields>)> {
    vec![
        (
            "command power",
            vec![
                TableFields::named(TableId::Countries, &["command_power_raw"]).unwrap(),
                TableFields::all(TableId::SunkShips),
                TableFields::all(TableId::WorldTension),
                TableFields::all(TableId::CountryLinks),
            ],
        ),
        ("country keys", vec![TableFields::keys(TableId::Countries)]),
        (
            "stability",
            vec![TableFields::named(TableId::Countries, &["stability_raw"]).unwrap()],
        ),
        ("nothing", Vec::new()),
        (
            "tension when read",
            vec![TableFields::all(TableId::WorldTension).when_read()],
        ),
        (
            "everything",
            TableId::ALL.into_iter().map(TableFields::all).collect(),
        ),
    ]
}

/// The block hours (`t`, from 1) a selection must visit.
fn expected_visit(selection: &str, t: Hour) -> bool {
    t == 1
        || match selection {
            // The predicted ramp every hour it moves, the one-off change,
            // the insertion, availability and truncation changes, the
            // deletion and the rate ramp; not the stop, the unwatched
            // stability and war support changes, nor an equal truncation.
            "command power" => matches!(t, 5..=15 | 18 | 20 | 22 | 24..=26 | 28..=39),
            "country keys" => matches!(t, 20 | 24 | 25 | 30),
            "nothing" => false,
            // Every read (odd hours) of rows that never change, and the
            // unavailable hours' edges.
            "tension when read" => t % 2 == 1 || matches!(t, 22 | 24),
            // Stability moves every hour; the clock changes every hour.
            _ => true,
        }
}

fn checkpoint(frame: &DecodedFrame) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame.encode_checkpoint(&mut bytes).unwrap();
    bytes
}

fn assert_frame(actual: &DecodedFrame, expected: &DecodedFrame) {
    assert_eq!(actual, expected);
    assert_eq!(checkpoint(actual), checkpoint(expected));
    for table in TableId::ALL {
        assert_eq!(
            actual.tables[table.index()].sampled_at(),
            expected.tables[table.index()].sampled_at(),
            "{table:?} sample hour at hour {}",
            actual.hour
        );
    }
}

/// Country fields by schema position, independent of the reader's masks.
fn country_fields(row: &CountryRow, fields: u64) -> Vec<String> {
    let values: [&dyn std::fmt::Debug; 10] = [
        &row.canonical_slot,
        &row.tag,
        &row.political_power_raw,
        &row.command_power_raw,
        &row.stability_raw,
        &row.war_support_raw,
        &row.convoys_raw,
        &row.peace_conference_participant,
        &row.manpower_residual,
        &row.border_land_doctrine_level,
    ];
    values
        .iter()
        .enumerate()
        .filter(|(field, _)| fields & (1_u64 << field) != 0)
        .map(|(_, value)| format!("{value:?}"))
        .collect()
}

fn rows_equal(watched: &TableFields, before: &TableRows, after: &TableRows) -> bool {
    let keys = |rows: &TableRows| rows.iter().map(|row| row.key()).collect::<Vec<_>>();
    if keys(before) != keys(after) {
        return false;
    }
    if let (TableRows::Countries(before), TableRows::Countries(after)) = (before, after) {
        return before.iter().zip(after).all(|(before, after)| {
            country_fields(before, watched.fields) == country_fields(after, watched.fields)
        });
    }
    let all = full_mask(watched.table.schema().len());
    match watched.fields & all {
        0 => true,
        fields if fields == all => before == after,
        _ => panic!("the fixture watches {:?} whole or by keys", watched.table),
    }
}

/// Whether two hours agree in a watched table's availability, row keys and
/// watched fields.
fn selected_equal(watched: &TableFields, before: &DecodedFrame, after: &DecodedFrame) -> bool {
    let index = watched.table.index();
    match (&*before.tables[index], &*after.tables[index]) {
        (TableView::Complete(before), TableView::Complete(after)) => {
            rows_equal(watched, before, after)
        }
        (
            TableView::Truncated {
                rows: before,
                missing: before_missing,
            },
            TableView::Truncated {
                rows: after,
                missing: after_missing,
            },
        ) => before_missing == after_missing && rows_equal(watched, before, after),
        (TableView::Unavailable { reason: before }, TableView::Unavailable { reason: after }) => {
            before == after
        }
        _ => false,
    }
}

/// Read one block both ways and check every hour; the hours visited.
fn checked(replay: &Replay, index: &BlockIndex, fields: &[TableFields]) -> Vec<Hour> {
    let tables: Vec<TableId> = fields.iter().map(|watched| watched.table).collect();
    let mut alone = Vec::new();
    replay
        .read_block_alone(
            index,
            &tables,
            |_, frame| {
                alone.push(frame.clone());
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    let watched: Vec<TableId> = TableId::ALL
        .into_iter()
        .filter(|table| tables.contains(table))
        .collect();
    let mut visited = Vec::new();
    let mut at = 0;
    let mut last: Option<(usize, DecodedFrame)> = None;
    replay
        .read_block_changes(
            index,
            fields,
            |change| {
                match change {
                    BlockChange::Changed {
                        frame,
                        tables: changed,
                        rebuilt,
                    } => {
                        let expected = &alone[at];
                        assert_frame(frame, expected);
                        // Rebuilt exactly where the held frames of the block
                        // read alone change allocation; every reconstructed
                        // table at the first hour.
                        for table in TableId::ALL {
                            let index = table.index();
                            let new = match at {
                                0 => tables.contains(&table) || table == TableId::Campaign,
                                _ => !SharedView::ptr_eq(
                                    &alone[at - 1].tables[index],
                                    &expected.tables[index],
                                ),
                            };
                            assert_eq!(
                                rebuilt.contains(&table),
                                new,
                                "{table:?} rebuilt at hour {} in one reader only",
                                frame.hour
                            );
                        }
                        match &last {
                            None => {
                                assert_eq!(at, 0, "the first hour is visited first");
                                assert_eq!(changed, &watched[..]);
                            }
                            Some((previous, previous_frame)) => {
                                for table in TableId::ALL {
                                    let index = table.index();
                                    assert_eq!(
                                        SharedView::ptr_eq(
                                            &previous_frame.tables[index],
                                            &frame.tables[index]
                                        ),
                                        SharedView::ptr_eq(
                                            &alone[*previous].tables[index],
                                            &expected.tables[index]
                                        ),
                                        "{table:?} at hour {} shares hour {}'s allocation in one reader only",
                                        frame.hour,
                                        previous_frame.hour
                                    );
                                }
                                for watch in fields {
                                    if !selected_equal(watch, &alone[at - 1], expected) {
                                        assert!(
                                            changed.contains(&watch.table),
                                            "{:?} changed at hour {} unlisted",
                                            watch.table,
                                            frame.hour
                                        );
                                    }
                                }
                                assert!(changed.iter().all(|table| watched.contains(table)));
                                for watch in fields.iter().filter(|watch| watch.read) {
                                    if expected.metrics.table_ns[watch.table.index()] != 0 {
                                        assert!(
                                            changed.contains(&watch.table),
                                            "{:?} read at hour {} unlisted",
                                            watch.table,
                                            frame.hour
                                        );
                                    }
                                }
                            }
                        }
                        visited.push(frame.hour);
                        last = Some((at, frame.clone()));
                        at += 1;
                    }
                    BlockChange::Quiet(quiet) => {
                        let (previous, _) =
                            last.as_ref().expect("a quiet span follows a changed hour");
                        assert_eq!(quiet.first(), alone[at].hour);
                        assert_eq!(quiet.hours().len(), quiet.metrics().len());
                        for (position, &hour) in quiet.hours().iter().enumerate() {
                            let expected = &alone[at];
                            assert_eq!(hour, expected.hour);
                            assert_eq!(quiet.metrics()[position], expected.metrics);
                            for table in TableId::ALL {
                                assert_eq!(
                                    quiet.sampled_at(position, table),
                                    expected.tables[table.index()].sampled_at(),
                                    "{table:?} sample hour at quiet hour {hour}"
                                );
                                assert_eq!(
                                    quiet.read(position, table),
                                    expected.metrics.table_ns[table.index()] != 0
                                );
                                assert_eq!(
                                    quiet.rebuilt(position, table),
                                    !SharedView::ptr_eq(
                                        &alone[at - 1].tables[table.index()],
                                        &expected.tables[table.index()]
                                    ),
                                    "{table:?} rebuilt at quiet hour {hour} in one reader only"
                                );
                            }
                            for watch in fields {
                                assert!(
                                    selected_equal(watch, &alone[at - 1], expected)
                                        && selected_equal(watch, &alone[*previous], expected),
                                    "{:?} changed at quiet hour {hour}",
                                    watch.table
                                );
                                assert!(
                                    !watch.read || !quiet.read(position, watch.table),
                                    "{:?} read at quiet hour {hour}",
                                    watch.table
                                );
                            }
                            at += 1;
                        }
                        assert_eq!(quiet.last(), alone[at - 1].hour);
                    }
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(at, alone.len(), "every hour is visited or quiet, once");
    visited
}

#[test]
fn changed_hours_equal_blocks_read_alone_and_quiet_hours_keep_watched_fields() {
    let names = [
        "canonical_slot",
        "tag",
        "political_power_raw",
        "command_power_raw",
        "stability_raw",
        "war_support_raw",
        "convoys_raw",
        "peace_conference_participant",
        "manpower_residual",
        "border_land_doctrine_level",
    ];
    assert_eq!(TableId::Countries.schema().len(), names.len());
    for (field, name) in names.iter().enumerate() {
        assert_eq!(TableId::Countries.field_name(field), Some(*name));
    }
    let memory = fixture();
    assert_eq!(memory.index.len(), 3);
    for block in [1, 2] {
        assert!(
            super::super::grouped::keyframe_chain(&memory, block, TableId::CountryLinks)
                .unwrap()
                .len()
                > 1,
            "block {block}'s CountryLinks keyframe is anchored to an earlier block's"
        );
    }
    let interleaved = interleaved_copy(&memory);
    let path =
        std::env::temp_dir().join(format!("replay-changes-{}.battleplan", std::process::id()));
    std::fs::write(&path, memory.bytes()).unwrap();
    let held = Replay::open_path(&path).unwrap();
    for replay in [&memory, &interleaved, &held] {
        // Every block alone on its own thread, the last first.
        let mut visits: Vec<Vec<Vec<Hour>>> = std::thread::scope(|scope| {
            let reads: Vec<_> = replay
                .index
                .iter()
                .rev()
                .map(|index| {
                    scope.spawn(move || {
                        selections()
                            .iter()
                            .map(|(_, fields)| checked(replay, index, fields))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            reads.into_iter().map(|read| read.join().unwrap()).collect()
        });
        visits.reverse();
        for (block, visits) in visits.iter().enumerate() {
            let last = local(replay.index[block].last_hour);
            for ((name, _), visited) in selections().iter().zip(visits) {
                let visited: Vec<Hour> = visited.iter().map(|&hour| local(hour)).collect();
                let expected: Vec<Hour> = (1..=last).filter(|&t| expected_visit(name, t)).collect();
                assert_eq!(visited, expected, "{name} in block {block}");
            }
        }
    }
    drop(held);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn cancellation_visitor_errors_and_damage_stop_as_blocks_read_alone() {
    let replay = fixture();
    let fields = [TableFields::all(TableId::Countries)];
    let error = replay
        .read_block_changes(
            &replay.index[0],
            &fields,
            |_| panic!("a cancelled read visited an hour"),
            || Err(io::Error::from(io::ErrorKind::Interrupted)),
        )
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    let mut calls = 0;
    let error = replay
        .read_block_changes(
            &replay.index[0],
            &fields,
            |_| {
                calls += 1;
                Err(io::Error::other("visitor stop"))
            },
            || Ok(()),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "visitor stop");
    assert_eq!(calls, 1);
    assert!(TableFields::named(TableId::Countries, &["no_such_field"]).is_err());
    assert_eq!(
        TableFields::named(TableId::Countries, &["command_power_raw", "stability_raw"])
            .unwrap()
            .fields,
        0b11000
    );
    // A damaged block refuses with the error reading it alone gives.
    for mut damaged in [
        replay.bytes().to_vec(),
        interleaved_copy(&replay).bytes().to_vec(),
    ] {
        let tail = Replay::open(damaged.clone())
            .unwrap()
            .index
            .last()
            .unwrap()
            .clone();
        damaged[(tail.offset + tail.length - 1) as usize] ^= 1;
        let alone = Replay::open(damaged.clone())
            .unwrap()
            .read_block_alone(&tail, &[TableId::Countries], |_, _| Ok(()), || Ok(()))
            .unwrap_err();
        let changes = Replay::open(damaged)
            .unwrap()
            .read_block_changes(&tail, &fields, |_| Ok(()), || Ok(()))
            .unwrap_err();
        assert_eq!(changes.kind(), alone.kind());
        assert_eq!(changes.to_string(), alone.to_string());
    }
}

#[test]
fn a_replaced_view_with_other_keys_differs_under_a_keys_only_watch() {
    let country = |id: u32, stability_raw: i64| CountryRow {
        id,
        canonical_slot: id,
        stability_raw,
        ..Default::default()
    };
    let truncated = |rows: Vec<CountryRow>| -> SharedView {
        TableView::Truncated {
            rows: TableRows::Countries(rows),
            missing: 1,
        }
        .into()
    };
    let complete = |rows: Vec<CountryRow>| -> SharedView {
        TableView::Complete(TableRows::Countries(rows)).into()
    };
    let keys = Some(TableFields::keys(TableId::Countries).fields);
    // One row swapped for another of the same count: the keys differ.
    assert!(view_differs(
        TableId::Countries,
        keys,
        &truncated(vec![country(1, 0)]),
        &truncated(vec![country(2, 0)]),
    ));
    assert!(view_differs(
        TableId::Countries,
        keys,
        &complete(vec![country(1, 0), country(3, 0)]),
        &complete(vec![country(2, 0), country(3, 0)]),
    ));
    // The same keys with only an unwatched field changed stay quiet.
    assert!(!view_differs(
        TableId::Countries,
        keys,
        &truncated(vec![country(1, 0)]),
        &truncated(vec![country(1, 7)]),
    ));
    assert!(!view_differs(
        TableId::Countries,
        keys,
        &complete(vec![country(1, 0), country(3, 0)]),
        &complete(vec![country(1, 5), country(3, 0)]),
    ));
}

/// Each changed hour's frame holds the one unselected view for every table
/// it does not reconstruct, moved on from the frame before: a block shares
/// it anew only for its first visited frame, however many hours change.
/// Selected tables also use that view temporarily when releasing a replaced
/// baseline before reconstruction; these shares are counted separately.
#[test]
fn changed_hours_move_the_unselected_view_on() {
    let replay = fixture();
    let fields = [TableFields::all(TableId::Countries)];
    let unselected = not_selected();
    for index in &replay.index {
        let selected = [TableId::Campaign, TableId::Countries];
        let released = replay
            .read_block_selected(index, &selected)
            .unwrap()
            .iter()
            .filter(|frame| !frame.keyframe)
            .map(|frame| {
                selected
                    .iter()
                    .filter(|table| {
                        !matches!(frame.tables[table.index()], TableUpdate::Unchanged { .. })
                    })
                    .count()
            })
            .sum::<usize>();
        let mut changed = 0;
        let (read, shares) = crate::frame::count_unselected(|| {
            replay.read_block_changes(
                index,
                &fields,
                |change| {
                    if let BlockChange::Changed { frame, .. } = change {
                        changed += 1;
                        for table in TableId::ALL {
                            if table != TableId::Countries && table != TableId::Campaign {
                                assert!(SharedView::ptr_eq(
                                    &frame.tables[table.index()],
                                    &unselected
                                ));
                            }
                        }
                    }
                    Ok(())
                },
                || Ok(()),
            )
        });
        read.unwrap();
        assert!(changed > 2, "{changed} changed hours");
        assert_eq!(shares, TABLE_COUNT - selected.len() + released);
    }
}
