//! `read_block_watched` against each watch's own read, hour by hour.

use super::super::tests::{CAMPAIGN, interleaved_copy, records, source};
use super::*;
use crate::{
    frame::FrameEncoder,
    rows::{CountryRow, DivisionRow, SunkShipsRow},
};

/// Two blocks of one branch, the first with a recorded gap (hours 10 and 11
/// missing): countries whose command power moves at some hours and whose
/// keys change once, ships sunk truncated for two hours, and world tension
/// unavailable for two hours and then recorded again whole.
fn fixture() -> Replay {
    let mut captured = records(1, &[1]);
    captured.pop();
    let mut encoder = FrameEncoder::new();
    let hours = (1..=24)
        .filter(|hour| !matches!(hour, 10 | 11))
        .chain(721..=736);
    for hour in hours {
        let t = (hour - 1) % 720 + 1;
        let mut frame = source(1, hour);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            [1_u32, 2, 3]
                .into_iter()
                .filter(|&id| !(id == 3 && t >= 15))
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    command_power_raw: if id == 1 { i64::from(t / 4) } else { 0 },
                    stability_raw: if id == 2 { i64::from(t) } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::SunkShips.index()] = if matches!(t, 5 | 6) {
            TableView::Truncated {
                rows: TableRows::SunkShips(vec![SunkShipsRow::default()]),
                missing: 1,
            }
        } else {
            TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow::default()]))
        };
        if matches!(t, 8 | 9) {
            *frame.tables[TableId::WorldTension.index()] = TableView::Unavailable {
                reason: "synthetic tension failure".into(),
            };
        }
        frame.metrics.table_ns[TableId::Clock.index()] = 1;
        frame.metrics.table_ns[TableId::Countries.index()] = u64::from(t != 7);
        frame.metrics.table_ns[TableId::WorldTension.index()] = u64::from(t % 3 == 0);
        captured.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(CAMPAIGN.into(), &[captured]).unwrap()
}

fn watchings() -> Vec<Watching<'static>> {
    static COMMAND: std::sync::LazyLock<Vec<TableFields>> = std::sync::LazyLock::new(|| {
        vec![
            TableFields::named(TableId::Countries, &["command_power_raw"]).unwrap(),
            TableFields::all(TableId::SunkShips),
        ]
    });
    static KEYS: std::sync::LazyLock<Vec<TableFields>> =
        std::sync::LazyLock::new(|| vec![TableFields::keys(TableId::Countries)]);
    static TENSION: std::sync::LazyLock<Vec<TableFields>> =
        std::sync::LazyLock::new(|| vec![TableFields::all(TableId::WorldTension).when_read()]);
    vec![
        Watching::Changes(&COMMAND),
        Watching::Every(&[TableId::Clock, TableId::WorldTension, TableId::SunkShips]),
        Watching::Changes(&KEYS),
        Watching::Every(&[TableId::Countries]),
        Watching::Changes(&TENSION),
        Watching::Every(&[]),
    ]
}

fn checkpoint(frame: &DecodedFrame) -> Vec<u8> {
    let mut bytes = Vec::new();
    frame.encode_checkpoint(&mut bytes).unwrap();
    bytes
}

/// One step a watch was handed, as compared: a frame with what its read
/// says of it, or a quiet span with everything it reports per table.
#[derive(Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Keep complete watch results inline so equality tests use the existing owned frame and update representations."
)]
enum Step {
    Changed {
        frame: Vec<u8>,
        decoded: DecodedFrame,
        sampled: Vec<Option<Hour>>,
        tables: Vec<TableId>,
        rebuilt: Vec<TableId>,
    },
    Quiet {
        hours: Vec<Hour>,
        metrics: Vec<Metrics>,
        sampled: Vec<Option<Hour>>,
        rebuilt: Vec<bool>,
    },
    Hour {
        frame: Vec<u8>,
        decoded: DecodedFrame,
        sampled: Vec<Option<Hour>>,
        /// The hour, keyframe, metrics and the watch's own tables' updates.
        updates: (Hour, bool, Metrics, Vec<TableUpdate>),
    },
}

/// Each step, and each visited frame for the allocation relation.
#[derive(Default)]
struct Log {
    steps: Vec<Step>,
    frames: Vec<DecodedFrame>,
}

impl Log {
    fn change(&mut self, change: BlockChange<'_>) {
        match change {
            BlockChange::Changed {
                frame,
                tables,
                rebuilt,
            } => {
                self.frames.push(frame.clone());
                self.steps.push(Step::Changed {
                    frame: checkpoint(frame),
                    decoded: frame.clone(),
                    sampled: sampled(frame),
                    tables: tables.to_vec(),
                    rebuilt: rebuilt.to_vec(),
                });
            }
            BlockChange::Quiet(quiet) => {
                let positions = 0..quiet.hours().len();
                self.steps.push(Step::Quiet {
                    hours: quiet.hours().to_vec(),
                    metrics: quiet.metrics().to_vec(),
                    sampled: positions
                        .clone()
                        .flat_map(|at| TableId::ALL.map(|table| quiet.sampled_at(at, table)))
                        .collect(),
                    rebuilt: positions
                        .flat_map(|at| TableId::ALL.map(|table| quiet.rebuilt(at, table)))
                        .collect(),
                });
            }
        }
    }

    fn hour(
        &mut self,
        own: &[TableId],
        updates: &(impl crate::frame::FrameSource + ?Sized),
        frame: &DecodedFrame,
    ) {
        self.frames.push(frame.clone());
        self.steps.push(Step::Hour {
            frame: checkpoint(frame),
            decoded: frame.clone(),
            sampled: sampled(frame),
            updates: (
                updates.hour(),
                updates.keyframe(),
                updates.metrics().clone(),
                own.iter()
                    .map(|table| updates.update(table.index()).clone())
                    .collect(),
            ),
        });
    }

    /// Whether each visited frame shares each table's allocation with the
    /// one visited before it.
    fn shared(&self) -> Vec<Vec<bool>> {
        self.frames
            .windows(2)
            .map(|pair| {
                TableId::ALL
                    .iter()
                    .map(|table| {
                        SharedView::ptr_eq(
                            &pair[0].tables[table.index()],
                            &pair[1].tables[table.index()],
                        )
                    })
                    .collect()
            })
            .collect()
    }
}

fn sampled(frame: &DecodedFrame) -> Vec<Option<Hour>> {
    frame.tables.iter().map(SharedView::sampled_at).collect()
}

/// A watch's tables and Campaign, as its own read reconstructs them.
fn own(watching: Watching<'_>) -> Vec<TableId> {
    let named: Vec<TableId> = match watching {
        Watching::Changes(fields) => fields.iter().map(|entry| entry.table).collect(),
        Watching::Every(tables) => tables.to_vec(),
    };
    TableId::ALL
        .into_iter()
        .filter(|table| *table == TableId::Campaign || named.contains(table))
        .collect()
}

/// A watch's own read of a block alone.
fn alone(replay: &Replay, index: &BlockIndex, watching: Watching<'_>) -> Log {
    let mut log = Log::default();
    match watching {
        Watching::Changes(fields) => replay
            .read_block_changes(
                index,
                fields,
                |change| {
                    log.change(change);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap(),
        Watching::Every(tables) => {
            let own = own(watching);
            replay
                .read_block_alone(
                    index,
                    tables,
                    |updates, frame| {
                        log.hour(&own, &updates, frame);
                        Ok(())
                    },
                    || Ok(()),
                )
                .unwrap()
        }
    }
    log
}

/// Every watch of one read of a block.
fn together(replay: &Replay, index: &BlockIndex, watchings: &[Watching<'_>]) -> Vec<Log> {
    together_releasing(replay, index, watchings, false)
}

fn together_releasing(
    replay: &Replay,
    index: &BlockIndex,
    watchings: &[Watching<'_>],
    releasing: bool,
) -> Vec<Log> {
    let mut logs: Vec<Log> = watchings.iter().map(|_| Log::default()).collect();
    let owns: Vec<Vec<TableId>> = watchings.iter().map(|&watching| own(watching)).collect();
    {
        let mut watches: Vec<BlockWatch<'_>> = watchings
            .iter()
            .zip(logs.iter_mut())
            .zip(&owns)
            .map(|((&watching, log), own)| {
                let watch = BlockWatch::new(watching, move |step| {
                    match step {
                        Watched::Change(change) => log.change(change),
                        Watched::Hour { updates, frame } => log.hour(own, updates, frame),
                        Watched::Replacing(_) => {}
                    }
                    Ok(())
                });
                if releasing { watch.releasing() } else { watch }
            })
            .collect();
        replay
            .read_block_watched(index, &mut watches, || Ok(()))
            .unwrap();
        for watch in watches {
            watch.finish().unwrap();
        }
    }
    logs
}

fn replays() -> (Replay, Replay, Replay, std::path::PathBuf) {
    let memory = fixture();
    let interleaved = interleaved_copy(&memory);
    let path = std::env::temp_dir().join(format!(
        "replay-watched-{}-{:?}.battleplan",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(&path, memory.bytes()).unwrap();
    let held = Replay::open_path(&path).unwrap();
    (memory, interleaved, held, path)
}

/// Every watch in one read is handed exactly what its own read hands it:
/// frames, change lists, rebuilt tables, quiet spans with their metrics and
/// sample hours, the recorded updates of its tables, and which visited hours
/// share each table's allocation; in grouped, interleaved and file-backed
/// blocks, with a gap, a truncated table, an unavailable table and the
/// snapshot after it.
#[test]
fn watches_in_one_read_are_handed_what_their_own_reads_hand_them() {
    let (memory, interleaved, held, path) = replays();
    let watchings = watchings();
    for (label, replay) in [
        ("memory", &memory),
        ("interleaved", &interleaved),
        ("held", &held),
    ] {
        assert_eq!(replay.index.len(), 2, "{label}");
        for (block, index) in replay.index.iter().enumerate() {
            // Every watch together, and each pair (so a watch shares the
            // read with fewer and with other tables).
            let mut sets: Vec<Vec<usize>> = vec![(0..watchings.len()).collect()];
            for first in 0..watchings.len() {
                for second in first + 1..watchings.len() {
                    sets.push(vec![first, second]);
                }
            }
            for set in sets {
                let chosen: Vec<Watching<'_>> = set.iter().map(|&at| watchings[at]).collect();
                let logs = together(replay, index, &chosen);
                let releasing = together_releasing(replay, index, &chosen, true);
                for (before, after) in logs.iter().zip(&releasing) {
                    assert_eq!(before.steps, after.steps, "{label}: releasing visits");
                    assert_eq!(
                        before.shared(),
                        after.shared(),
                        "{label}: releasing identities"
                    );
                }
                for (&at, log) in set.iter().zip(&logs) {
                    let expected = alone(replay, index, watchings[at]);
                    assert!(!expected.steps.is_empty());
                    assert_eq!(
                        log.steps, expected.steps,
                        "{label} block {block}: watch {at} with {set:?}"
                    );
                    assert_eq!(
                        log.shared(),
                        expected.shared(),
                        "{label} block {block}: watch {at} with {set:?} shares allocations as alone"
                    );
                }
            }
        }
    }
    drop(held);
    std::fs::remove_file(path).unwrap();
}

/// One read of a grouped block decodes each table section once however many
/// watches read it; their own reads decode it once each.
#[test]
fn watches_in_one_read_decode_each_section_once() {
    let replay = fixture();
    let watchings = watchings();
    let mut union: Vec<TableId> = watchings
        .iter()
        .flat_map(|&watching| own(watching))
        .collect();
    union.sort_unstable_by_key(|table| table.index());
    union.dedup();
    for index in &replay.index {
        let before = decoded();
        together(&replay, index, &watchings);
        assert_eq!(
            decoded() - before,
            union.len(),
            "each of the union's sections decoded once"
        );
        let before = decoded();
        for &watching in &watchings {
            alone(&replay, index, watching);
        }
        let separately: usize = watchings.iter().map(|&watching| own(watching).len()).sum();
        assert_eq!(decoded() - before, separately);
        assert!(separately > union.len());
    }
}

/// A watch whose visit fails stops with its error at that hour; the others
/// are handed every hour as alone.
#[test]
fn a_failed_visit_stops_only_its_watch() {
    let replay = fixture();
    let index = &replay.index[0];
    let mut hours = Vec::new();
    let mut log = Log::default();
    let tables = [TableId::Clock];
    let every = Watching::Every(&tables);
    {
        let mut watches = [
            BlockWatch::new(every, |step| {
                let Watched::Hour { frame, .. } = step else {
                    unreachable!()
                };
                hours.push(frame.hour);
                if frame.hour == 3 {
                    return Err(io::Error::other("synthetic visit failure"));
                }
                Ok(())
            }),
            BlockWatch::new(every, |step| {
                let Watched::Hour { updates, frame } = step else {
                    unreachable!()
                };
                log.hour(&own(every), updates, frame);
                Ok(())
            }),
        ];
        replay
            .read_block_watched(index, &mut watches, || Ok(()))
            .unwrap();
        let [failed, other] = watches;
        assert_eq!(
            failed.finish().unwrap_err().to_string(),
            "synthetic visit failure"
        );
        other.finish().unwrap();
    }
    assert_eq!(hours, [1, 2, 3]);
    assert_eq!(log.steps, alone(&replay, index, every).steps);
}

/// The keyframe check over every table refuses exactly the watches whose own
/// read refuses the keyframe: one holding a dependent update of a table it
/// reads, and in an interleaved block one holding a delta of a table it does
/// not read; a grouped read refuses every watch when a table it decodes for
/// none of them does.
#[test]
fn a_dependent_keyframe_refuses_the_watches_whose_own_reads_refuse_it() {
    let mut chosen = [false; TABLE_COUNT];
    for table in [TableId::Campaign, TableId::Clock, TableId::SunkShips] {
        chosen[table.index()] = true;
    }
    let tables = BlockTables::new(CAMPAIGN, FormatVersion::CURRENT, &chosen).unwrap();
    let position = |table: TableId| tables.slots[table.index()].unwrap();
    let clock = [position(TableId::Campaign), position(TableId::Clock)];
    let ships = [position(TableId::Campaign), position(TableId::SunkShips)];
    let replay = fixture();
    let mut frame = replay
        .read_block_selected(&replay.index[0], &TableId::ALL)
        .unwrap()
        .remove(0);
    assert!(frame.keyframe);
    for keyframe in [Keyframe::Grouped(false), Keyframe::Interleaved] {
        assert!(!tables.refuses_keyframe(
            &clock,
            &crate::frame::RecordedFrame::from(frame.clone()),
            keyframe
        ));
        assert!(!tables.refuses_keyframe(
            &ships,
            &crate::frame::RecordedFrame::from(frame.clone()),
            keyframe
        ));
    }
    assert!(tables.refuses_keyframe(
        &clock,
        &crate::frame::RecordedFrame::from(frame.clone()),
        Keyframe::Grouped(true)
    ));
    frame.tables[TableId::SunkShips.index()] = TableUpdate::Unchanged { baseline: 0 };
    for keyframe in [Keyframe::Grouped(false), Keyframe::Interleaved] {
        assert!(!tables.refuses_keyframe(
            &clock,
            &crate::frame::RecordedFrame::from(frame.clone()),
            keyframe
        ));
        assert!(tables.refuses_keyframe(
            &ships,
            &crate::frame::RecordedFrame::from(frame.clone()),
            keyframe
        ));
    }
    frame.tables[TableId::SunkShips.index()] = TableUpdate::Delta {
        baseline: 0,
        changed: Vec::new(),
        deleted: Vec::new(),
    };
    assert!(!tables.refuses_keyframe(
        &clock,
        &crate::frame::RecordedFrame::from(frame.clone()),
        Keyframe::Grouped(false)
    ));
    assert!(tables.refuses_keyframe(
        &clock,
        &crate::frame::RecordedFrame::from(frame.clone()),
        Keyframe::Interleaved
    ));
    assert!(tables.refuses_keyframe(
        &ships,
        &crate::frame::RecordedFrame::from(frame.clone()),
        Keyframe::Grouped(false)
    ));
}

/// A read keeping a grouped block's frames for another reader hands it,
/// before any hour is reconstructed, every hour's recorded updates of the
/// tables kept and the watches' tables as a selected read of them returns
/// them, and its watches are still handed what their own reads hand them.
/// An interleaved block's frames are not kept.
#[test]
fn a_kept_block_holds_the_selected_reads_updates_and_its_watches_their_own_reads() {
    let (memory, interleaved, held, path) = replays();
    let watchings = watchings();
    let extra = [TableId::Countries, TableId::Clock, TableId::CountryLinks];
    for (label, replay) in [
        ("memory", &memory),
        ("interleaved", &interleaved),
        ("held", &held),
    ] {
        for index in &replay.index {
            let mut logs: Vec<Log> = watchings.iter().map(|_| Log::default()).collect();
            let owns: Vec<Vec<TableId>> = watchings.iter().map(|&watching| own(watching)).collect();
            let mut kept: Option<Vec<Arc<RecordedFrame>>> = None;
            let mut visited_before_kept = false;
            {
                let visited = std::cell::Cell::new(false);
                let mut watches: Vec<BlockWatch<'_>> = watchings
                    .iter()
                    .zip(logs.iter_mut())
                    .zip(&owns)
                    .map(|((&watching, log), own)| {
                        let visited = &visited;
                        BlockWatch::new(watching, move |step| {
                            visited.set(true);
                            match step {
                                Watched::Change(change) => log.change(change),
                                Watched::Hour { updates, frame } => log.hour(own, updates, frame),
                                Watched::Replacing(_) => {
                                    unreachable!("a watch retaining its views")
                                }
                            }
                            Ok(())
                        })
                    })
                    .collect();
                let mut keep = |frames: &[Arc<RecordedFrame>]| {
                    visited_before_kept = visited.get();
                    kept = Some(frames.to_vec());
                };
                let before = decoded();
                replay
                    .read_block_watched_keeping(
                        index,
                        &mut watches,
                        Some(Keep {
                            tables: &extra,
                            frames: &mut keep,
                        }),
                        || Ok(()),
                    )
                    .unwrap();
                if !std::ptr::eq(replay, &interleaved) {
                    let mut union: Vec<TableId> =
                        owns.iter().flatten().copied().chain(extra).collect();
                    union.sort_unstable();
                    union.dedup();
                    assert_eq!(decoded() - before, union.len(), "each section decoded once");
                }
                for watch in watches {
                    watch.finish().unwrap();
                }
            }
            for (at, log) in logs.iter().enumerate() {
                let expected = alone(replay, index, watchings[at]);
                assert_eq!(log.steps, expected.steps, "{label}: watch {at}");
                assert_eq!(log.shared(), expected.shared(), "{label}: watch {at}");
            }
            if std::ptr::eq(replay, &interleaved) {
                assert!(kept.is_none(), "{label}: an interleaved block is not kept");
                continue;
            }
            assert!(
                !visited_before_kept,
                "{label}: kept before any hour is visited"
            );
            let kept = kept.expect("a grouped block is kept");
            let mut read: Vec<TableId> = owns.iter().flatten().copied().chain(extra).collect();
            read.sort_unstable_by_key(|table| table.index());
            read.dedup();
            let selected = replay.read_block_selected(index, &read).unwrap();
            assert_eq!(kept.len(), selected.len(), "{label}");
            for (kept, selected) in kept.iter().zip(&selected) {
                assert_eq!(
                    (kept.branch, kept.hour, kept.keyframe, &kept.metrics),
                    (
                        selected.branch,
                        selected.hour,
                        selected.keyframe,
                        &selected.metrics
                    ),
                    "{label}"
                );
                for table in &read {
                    assert_eq!(
                        kept.tables[table.index()],
                        selected.tables[table.index()],
                        "{label}: {table:?} at hour {}",
                        kept.hour
                    );
                }
            }
        }
    }
    drop(held);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn kept_only_tables_have_no_reconstructed_views() {
    let replay = fixture();
    let own = own(Watching::Every(&[TableId::Campaign, TableId::Clock]));
    let extra = [
        TableId::Countries,
        TableId::SunkShips,
        TableId::WorldTension,
    ];
    let selected: Vec<TableId> = own.iter().copied().chain(extra).collect();
    for index in &replay.index {
        let mut log = Log::default();
        {
            let mut watches = [BlockWatch::new(Watching::Every(&own), |step| {
                let Watched::Hour { updates, frame } = step else {
                    unreachable!("a watch of every hour")
                };
                log.hour(&own, updates, frame);
                Ok(())
            })];
            let mut reading =
                Reading::new(CAMPAIGN, index.branch, replay.format, &mut watches, &extra).unwrap();
            for frame in replay.read_block_selected(index, &selected).unwrap() {
                assert!(
                    reading
                        .hour(Arc::new(frame), Keyframe::Grouped(false))
                        .unwrap()
                );
                for table in extra {
                    let position = reading.tables.slots[table.index()].unwrap();
                    assert!(reading.tables.views[position].is_none(), "{table:?}");
                }
                assert!(
                    reading.tables.views[reading.tables.slots[TableId::Clock.index()].unwrap()]
                        .is_some()
                );
            }
            reading.flush();
            drop(reading);
            for watch in watches {
                watch.finish().unwrap();
            }
        }
        let expected = alone(&replay, index, Watching::Every(&own));
        assert_eq!(log.steps, expected.steps);
        assert_eq!(log.shared(), expected.shared());
    }
}

#[test]
fn clock_hours_keep_no_change_only_division_updates_in_either_layout() {
    let mut captured = records(1, &[1]);
    captured.pop();
    let mut encoder = FrameEncoder::new();
    for hour in 1..=3 {
        let mut frame = source(1, hour);
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                id: 1,
                roster_owner: 1,
                logical_owner: 1,
                province: hour as u32,
                ..Default::default()
            }]));
        frame.metrics.table_ns[TableId::Clock.index()] = 1;
        captured.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    let grouped = Replay::from_records(CAMPAIGN.into(), &[captured]).unwrap();
    let interleaved = interleaved_copy(&grouped);
    let clock_tables = own(Watching::Every(&[TableId::Clock]));
    let fields = [TableFields::named(TableId::Divisions, &["province"]).unwrap()];
    for replay in [&grouped, &interleaved] {
        for index in &replay.index {
            let raw = replay
                .read_block_selected(index, &[TableId::Clock, TableId::Divisions])
                .unwrap();
            assert!(matches!(
                raw[0].tables[TableId::Divisions.index()],
                TableUpdate::Snapshot(_)
            ));
            assert!(raw[1..].iter().any(|frame| matches!(
                frame.tables[TableId::Divisions.index()],
                TableUpdate::Delta { .. }
            )));
            let mut clock = Log::default();
            let mut divisions = Log::default();
            let mut saved = Vec::new();
            {
                let mut watches = [
                    BlockWatch::new(Watching::Every(&[TableId::Clock]), |step| {
                        let Watched::Hour { updates, frame } = step else {
                            unreachable!("a watch of every hour")
                        };
                        clock.hour(&clock_tables, updates, frame);
                        saved.push(Arc::clone(updates));
                        Ok(())
                    }),
                    BlockWatch::new(Watching::Changes(&fields), |step| {
                        let Watched::Change(change) = step else {
                            unreachable!("a watch of changes")
                        };
                        divisions.change(change);
                        Ok(())
                    }),
                ];
                replay
                    .read_block_watched(index, &mut watches, || Ok(()))
                    .unwrap();
                for watch in watches {
                    watch.finish().unwrap();
                }
            }
            assert_eq!(
                clock.steps,
                alone(replay, index, Watching::Every(&[TableId::Clock])).steps
            );
            assert_eq!(
                divisions.steps,
                alone(replay, index, Watching::Changes(&fields)).steps
            );
            let expected = replay.read_block_selected(index, &clock_tables).unwrap();
            assert_eq!(saved.len(), expected.len());
            for (saved, expected) in saved.iter().zip(&expected) {
                for table in TableId::ALL {
                    if clock_tables.contains(&table) {
                        assert_eq!(saved.tables[table.index()], expected.tables[table.index()]);
                    } else {
                        assert!(
                            matches!(&saved.tables[table.index()],
                                TableUpdate::Unavailable { reason } if reason.is_empty()
                            ),
                            "{table:?}: hourly facts retain a table used only by the change watch"
                        );
                    }
                }
            }
        }
    }
}
