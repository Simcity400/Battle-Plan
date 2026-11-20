use super::fixtures::{self, every, recording};
use super::*;
use crate::rows::TableRows;
use replay_columnar::{
    container::Root,
    envelope::{self, Kind},
};
use std::sync::atomic::{AtomicU64, Ordering};

fn opened(bytes: Vec<u8>) -> Recording {
    Recording::open(bytes).unwrap()
}

/// A recording's bytes in a file of their own, removed afterwards.
struct Saved(std::path::PathBuf);

impl Saved {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("columnar-frames-{}-{}.battleplan", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn open(&self) -> Recording {
        Recording::open_path(&self.0).unwrap()
    }

    fn append(&self, bytes: &[u8]) {
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(&self.0).unwrap().write_all(bytes).unwrap();
    }
}

impl Drop for Saved {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn append_revision(bytes: &mut Vec<u8>, revision: u64) {
    let (extent, _) = envelope::last_root(bytes).unwrap();
    let stored = extent.stored_in(bytes).unwrap();
    let raw = zstd::bulk::decompress(stored, 1 << 20).unwrap();
    let mut root = Root::read(&raw).unwrap();
    root.revision = revision;
    let root = fixtures::append(bytes, Kind::Root, (0, 0, 0, 0), &root.write().unwrap());
    bytes.extend_from_slice(&envelope::footer(root.offset, root.stored, &root.digest));
}

/// The frames of every hour of every run, read on hour after hour.
fn read_on(file: &Recording, tables: &[TableId]) -> Vec<fixtures::Seen> {
    let mut frames = Frames::new(file, tables).unwrap();
    let mut out = Vec::new();
    for run in 0..file.run_count() {
        let held = file.run(run).unwrap();
        for ordinal in 0..held.ordinals {
            frames.go_to(run, ordinal).unwrap();
            let frame = frames.frame(held.branch).unwrap();
            let sampled = frame.tables.iter().map(SharedView::sampled_at).collect();
            out.push((frame, sampled));
        }
    }
    out
}

/// Each table's view and sample hour, the metrics aside: the writer's frames carry none.
fn same(read: &fixtures::Seen, written: &fixtures::Seen) -> bool {
    read.0.branch == written.0.branch && read.0.hour == written.0.hour && read.0.tables == written.0.tables && read.1 == written.1
}

/// Five runs, two of one branch, one after the other.
const CHURNED: [(u64, i32, u32); 5] = [
    (1, 100, 5),
    (1, 100 + 5 * envelope::UNIT_ORDINALS as i32, 4),
    (2, 50_000, 3),
    (3, 90_000, 6),
    (2, 200_000, 2),
];

const WATCHED: [TableId; 2] = [TableId::Countries, TableId::WorldTension];

/// Frames read on hour after hour, from a file and from memory, are the writer's own tables at
/// every hour of every run: rows, strings, statuses and sample hours.
#[test]
fn frames_read_on_are_the_writer_s_tables_at_every_hour_of_every_run() {
    let (bytes, expected) = fixtures::churned(&CHURNED);
    let disk = Saved::new(&bytes);
    for file in [opened(bytes.clone()), disk.open()] {
        assert_eq!(file.run_count(), 5);
        assert_eq!(file.segments().len(), 5);
        assert_eq!(file.hours(), expected.len() as u64);
        let read = read_on(&file, &WATCHED);
        assert_eq!(read.len(), expected.len());
        for (at, (read, written)) in read.iter().zip(&expected).enumerate() {
            assert!(same(read, written), "hour {at} ({}) differs", written.0.hour);
        }
    }
}

/// A derived table is its writer's, run each hour as the recorder runs it: a balance that falls
/// reads back as that hour's drop, and what the hours themselves say of the table is left out.
/// A cell its writer leaves unrecorded reads back not recorded: a ceiling without its command
/// power row.
#[test]
fn a_derived_table_reads_back_as_its_writer_wrote_it_whatever_the_hours_say() {
    use super::synthetic::{Input, Sample};
    use crate::rows::{BalanceDropRow, CountryRow, CountryStabilityInputRow, CountryStabilityRow, Recorded};
    let derived = TableId::BalanceDrops;
    assert!(recorder_derive::writers().tables().contains(&derived.index()));
    const FIRST: i32 = 100;
    // Rises four hours and falls on the fifth.
    let power = |ordinal: u32| i64::from(ordinal % 5) * 1_000;
    let inputs = CountryStabilityInputRow {
        country: 9,
        stability_raw: 50_000,
        war_support_raw: 20_000,
        max_stability: 100_000,
        max_war_support: 100_000,
        base_max_command_power: 30_000_000,
        stability_factor: Some(0),
        party_popularity_stability_factor: Some(0),
        war_support_factor: Some(0),
        // A key per total, none for a total not read; the command power keys are not read.
        keys: vec![1, 2, u32::MAX, u32::MAX, u32::MAX, 6, u32::MAX, u32::MAX, u32::MAX, u32::MAX],
        ..Default::default()
    };
    let streams = [TableId::Countries, TableId::CountryStabilityInputs, derived, TableId::CountryStability];
    let (bytes, _) = fixtures::written(b"derived", &[(1, FIRST, 1)], &streams, |synthetic, _, ordinal| {
        let rows = [CountryRow { id: 9, tag: b"GER".to_vec(), political_power_raw: Some(power(ordinal)), ..Default::default() }];
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        let read = std::slice::from_ref(&inputs);
        synthetic.table(TableId::CountryStabilityInputs, Input::Snapshot { rows: read, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table::<BalanceDropRow>(derived, Input::Unavailable { reason: "the hours' own word", read: false, sample: Sample::FromReadBit }).unwrap();
    });
    let recorded = |value: Option<i64>| value.map_or(Recorded::NotRecorded, Recorded::Value);
    let stability = CountryStabilityRow {
        country: 9,
        stability: recorded(replay_data::country_stability::stability(&inputs).value),
        war_support: recorded(replay_data::country_stability::war_support(&inputs).value),
        max_command_power: Recorded::NotRecorded,
    };
    assert!(matches!((stability.stability, stability.war_support), (Recorded::Value(_), Recorded::Value(_))), "the fixture's stability and war support are recorded");
    let read = read_on(&opened(bytes), &[derived, TableId::CountryStability]);
    assert_eq!(read.len(), envelope::UNIT_ORDINALS as usize);
    let mut drops = 0;
    for (ordinal, (frame, _)) in (0..).zip(&read) {
        let fell = ordinal > 0 && power(ordinal) < power(ordinal - 1);
        let rows: Vec<BalanceDropRow> = fell
            .then(|| BalanceDropRow { kind: 0, country: 9, hour: FIRST + ordinal as i32, before: power(ordinal - 1), after: power(ordinal) })
            .into_iter()
            .collect();
        drops += rows.len();
        assert_eq!(*frame.tables[derived.index()], TableView::Complete(TableRows::BalanceDrops(rows)), "hour {ordinal}");
        assert_eq!(*frame.tables[TableId::CountryStability.index()], TableView::Complete(TableRows::CountryStability(vec![stability.clone()])), "hour {ordinal}");
    }
    assert_eq!(drops, (envelope::UNIT_ORDINALS as usize - 1) / 5);
}

/// A table holding a cell not recorded in a field its row cannot leave unrecorded is withheld by
/// name at that hour, and every other table of the hour is built.
#[test]
fn a_table_whose_row_cannot_say_not_recorded_is_withheld_by_name_not_the_hour() {
    use super::synthetic::{Input, Sample};
    use crate::rows::{CountryExperienceUnsayableRow, CountryRow, Recorded};
    let withheld = TableId::CountryExperience;
    let countries = [CountryRow { id: 9, tag: b"GER".to_vec(), ..Default::default() }];
    let (bytes, _) = fixtures::written(b"withheld", &[(1, 100, 1)], &[TableId::Countries], |synthetic, _, ordinal| {
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        let owned = if ordinal % 2 == 0 { Recorded::NotRecorded } else { Recorded::Value(5) };
        let rows = [CountryExperienceUnsayableRow { country: 9, army_raw: owned, ..Default::default() }];
        synthetic.table(withheld, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
    });
    let read = read_on(&opened(bytes), &[TableId::Countries, withheld]);
    assert_eq!(read.len(), envelope::UNIT_ORDINALS as usize);
    for (ordinal, (frame, _)) in (0..).zip(&read) {
        assert_eq!(*frame.tables[TableId::Countries.index()], TableView::Complete(TableRows::Countries(countries.to_vec())), "hour {ordinal}");
        let view = &*frame.tables[withheld.index()];
        if ordinal % 2 == 0 {
            let reason = "CountryExperience.army_raw is not recorded and its row cannot say so";
            assert_eq!(*view, TableView::Unavailable { reason: reason.into() }, "hour {ordinal}");
        } else {
            let TableView::Complete(TableRows::CountryExperience(rows)) = view else { panic!("hour {ordinal}: {view:?}") };
            assert_eq!((rows.len(), rows[0].country, rows[0].army_raw), (1, 9, 5), "hour {ordinal}");
        }
    }
}

/// A frame sought at any hour, each table restored at its last restore before the hour and
/// advanced to it, is the frame reading on reaches there.
#[test]
fn a_frame_sought_at_any_hour_is_the_one_reading_on_reaches() {
    let (bytes, expected) = fixtures::churned(&CHURNED);
    let file = opened(bytes);
    let mut starts = Vec::new();
    for run in 0..file.run_count() {
        let held = file.run(run).unwrap();
        for ordinal in 0..held.ordinals {
            starts.push((run, ordinal, held.branch));
        }
    }
    let mut frames = Frames::new(&file, &WATCHED).unwrap();
    // Every 37th hour, backwards, so no seek reads on from the one before.
    for (at, &(run, ordinal, branch)) in starts.iter().enumerate().rev().step_by(37) {
        frames.seek(run, ordinal).unwrap();
        let frame = frames.frame(branch).unwrap();
        let sampled = frame.tables.iter().map(SharedView::sampled_at).collect();
        assert!(same(&(frame, sampled), &expected[at]), "run {run} ordinal {ordinal} differs");
    }
}

/// Frames moved forward a day at a time through a run, across its block ends, are at every stop
/// the frame a fresh seek reads; each table's reader goes on from where it stands and restores
/// only across a block end the table was restored at.
#[test]
fn frames_moved_forward_by_days_go_on_and_are_the_ones_sought() {
    // Seven blocks: the writer restores each table at block 5's end, six blocks after its first.
    let (bytes, _) = fixtures::churned(&[(1, 0, 7)]);
    let file = opened(bytes);
    let ordinals = file.run(0).unwrap().ordinals;
    let block = envelope::UNIT_ORDINALS;
    let restored = |table: TableId, at: u32| file.table_entry(0, at, table.index()).unwrap().unit.restore.is_some();
    let restores = |frames: &Frames<&Recording>| WATCHED.map(|table| frames.reader(table).map_or(0, TableReader::restores));
    let sampled = |frame: &DecodedFrame| frame.tables.iter().map(SharedView::sampled_at).collect::<Vec<_>>();
    let mut frames = Frames::new(&file, &WATCHED).unwrap();
    let mut crossed = [false; 2];
    let mut previous: Option<u32> = None;
    for ordinal in (5..ordinals).step_by(24) {
        let before = restores(&frames);
        frames.go_to(0, ordinal).unwrap();
        let frame = frames.frame(1).unwrap();
        let mut fresh = Frames::new(&file, &WATCHED).unwrap();
        fresh.seek(0, ordinal).unwrap();
        let sought = fresh.frame(1).unwrap();
        assert_eq!(frame.tables, sought.tables, "ordinal {ordinal}");
        assert_eq!(sampled(&frame), sampled(&sought), "ordinal {ordinal}");
        let Some(at) = previous else {
            previous = Some(ordinal);
            continue;
        };
        let after = restores(&frames);
        for (index, table) in WATCHED.into_iter().enumerate() {
            let across = (at / block..ordinal / block).any(|end| restored(table, end));
            if at / block != ordinal / block {
                crossed[usize::from(across)] = true;
            }
            assert_eq!(after[index] - before[index], u32::from(across), "{table:?} from ordinal {at} to {ordinal}");
        }
        previous = Some(ordinal);
    }
    assert_eq!(crossed, [true, true], "the walk crosses a block end with no restore and one with");
}

/// Every table's [`Spent`] of `frames`, summed.
fn total(frames: &Frames<&Recording>) -> Spent {
    frames.spent().fold(Spent::default(), |sum, (_, spent)| Spent {
        patched: sum.patched + spent.patched,
        in_place: sum.in_place + spent.in_place,
        rebuilt: sum.rebuilt + spent.rebuilt,
        snapshots_patched: sum.snapshots_patched + spent.snapshots_patched,
        move_ns: sum.move_ns + spent.move_ns,
        view_ns: sum.view_ns + spent.view_ns,
        update_ns: sum.update_ns + spent.update_ns,
    })
}

/// Every hour of every run of `file`, stepped to as a walk of blocks steps (each block's first
/// hour a keyframe), its frame and recorded facts made three ways at once: every view built from
/// every cell, the reference; views patched while the frame and facts of the hour before are
/// still held, so each patch copies; and patched with them dropped, so each patch is in the rows'
/// own storage. Each patched hour is the reference's: every table's view, sample hour and update,
/// and whether it is a keyframe. `seen` sees the dropped walk's frames before each move (false)
/// and after it (true). The [`Spent`] totals of the reference, the held walk and the dropped one.
fn three_ways(file: &Recording, tables: &[TableId], mut seen: impl FnMut(&Frames<&Recording>, bool)) -> [Spent; 3] {
    let mut rebuilt = Frames::new(file, tables).unwrap().rebuilding();
    let mut held = Frames::new(file, tables).unwrap();
    let mut dropped = Frames::new(file, tables).unwrap();
    let mut kept = None;
    for run in 0..file.run_count() {
        let at = file.run(run).unwrap();
        for ordinal in 0..at.ordinals {
            seen(&dropped, false);
            for frames in [&mut rebuilt, &mut held, &mut dropped] {
                frames.go_to(run, ordinal).unwrap();
                if ordinal % envelope::UNIT_ORDINALS == 0 {
                    frames.keyframe();
                }
            }
            seen(&dropped, true);
            let (want, want_facts) = rebuilt.frame_recorded(at.branch).unwrap();
            let (shared, shared_facts) = held.frame_recorded(at.branch).unwrap();
            let (alone, alone_facts) = dropped.frame_recorded(at.branch).unwrap();
            for (how, frame, facts) in [("held", &shared, &shared_facts), ("dropped", &alone, &alone_facts)] {
                assert_eq!(facts.keyframe, want_facts.keyframe, "run {run} ordinal {ordinal}, the hour before {how}: keyframe");
                for &table in tables {
                    let index = table.index();
                    assert_eq!(frame.tables[index], want.tables[index], "{table:?} at run {run} ordinal {ordinal}, the hour before {how}: the patched view is not the view built from every cell");
                    assert_eq!(frame.tables[index].sampled_at(), want.tables[index].sampled_at(), "{table:?} at run {run} ordinal {ordinal}, the hour before {how}: sample hour");
                    assert_eq!(facts.tables[index], want_facts.tables[index], "{table:?} at run {run} ordinal {ordinal}, the hour before {how}: update");
                }
            }
            kept = Some((shared, shared_facts));
        }
    }
    drop(kept);
    [total(&rebuilt), total(&held), total(&dropped)]
}

/// The three walks took the paths they name: the reference patched nothing; a patch beside a
/// held frame copied its rows and its Snapshot; one with the hour before dropped patched both in
/// their own storage.
fn took_each_path([rebuilt, held, dropped]: [Spent; 3]) {
    assert_eq!(rebuilt.patched, 0, "the reference patched: {rebuilt:?}");
    assert!(held.patched > 0 && held.in_place == 0 && held.snapshots_patched == 0, "a frame held copies: {held:?}");
    assert!(dropped.patched > 0 && dropped.in_place == dropped.patched && dropped.snapshots_patched > 0, "a frame dropped is patched in place: {dropped:?}");
}

/// At every hour of every run of the churned recording (strings coming back blocks later, a row
/// leaving and returning, a table unavailable for stretches) a patched view is the view built
/// from every cell, and its update the update built with it, whether the frame before is held
/// or dropped. A patch that missed a delete or put rows out of key order fails the view's
/// assertion in [`three_ways`].
#[test]
fn a_view_patched_at_every_hour_is_the_view_built_from_every_cell_held_or_dropped() {
    let (bytes, _) = fixtures::churned(&CHURNED);
    took_each_path(three_ways(&opened(bytes), &WATCHED, |_, _| {}));
}

/// The same through every edit a step patches and every move it does not: rows written, inserted,
/// deleted, deleted one hour and back the next, a deleted row's slot taken by a row inserted the
/// same hour; a block's end packing the table; hours read in part and not at all.
#[test]
fn a_view_patched_through_inserts_deletes_packs_and_status_changes_is_the_view_built_from_every_cell() {
    let (bytes, _) = fixtures::edited(3);
    let countries = TableId::Countries;
    let (mut packs, mut taken, mut returned, mut statuses) = (0_u32, 0_u32, 0_u32, 0_u32);
    let (mut gone, mut status) = (Vec::new(), None);
    let spent = three_ways(&opened(bytes), &[countries], |frames, moved| {
        let Some(reader) = frames.reader(countries) else { return };
        if !moved {
            packs += u32::from(reader.packs_on_step());
            return;
        }
        let changes = reader.changes();
        taken += u32::from(changes.deleted.iter().any(|(slot, _)| changes.inserted.contains(slot)));
        let inserted: Vec<u128> = changes.inserted.iter().filter_map(|&slot| reader.cells().key(slot)).collect();
        returned += u32::from(inserted.iter().any(|key| gone.contains(key)));
        gone = changes.deleted.iter().map(|&(_, key)| key).collect();
        statuses += u32::from(status.is_some_and(|before| before != reader.status()));
        status = Some(reader.status());
    });
    assert!(packs > 0, "a block's end packs the table");
    assert!(taken > 0, "a row inserted takes the slot of a row deleted that hour");
    assert!(returned > 0, "a row deleted comes back the next hour");
    assert!(statuses > 0, "the table's status changes");
    took_each_path(spent);
}

/// The keys `now` holds that `before` does not hold equal, and the keys of `before` it lacks.
fn keyed_edits<T: replay_data::rows::RowCodec>(now: &[T], before: &[T]) -> (Vec<u128>, Vec<u128>) {
    let at = |rows: &[T], key: u128| rows.binary_search_by_key(&key, T::key).ok();
    let changed = now.iter().filter(|row| at(before, row.key()).is_none_or(|old| before[old] != **row)).map(T::key).collect();
    let deleted = before.iter().map(T::key).filter(|&key| at(now, key).is_none()).collect();
    (changed, deleted)
}

/// [`keyed_edits`] of a watched table's complete rows.
fn watched_edits(now: &TableRows, before: &TableRows) -> (Vec<u128>, Vec<u128>) {
    match (now, before) {
        (TableRows::Countries(now), TableRows::Countries(before)) => keyed_edits(now, before),
        (TableRows::WorldTension(now), TableRows::WorldTension(before)) => keyed_edits(now, before),
        _ => panic!("not a watched table's rows"),
    }
}

/// With change facts, every hour stepped to from the hour built before names, against that
/// hour's view, exactly the keys the writer changed, inserted and deleted that hour: a consumer
/// asking since that view gets those facts, or Unchanged when the hour wrote no row (a view
/// rebuilt for its sample hour alone keeps its stamp's fields), and the field stamps tell a
/// field no hour writes unchanged; an hour reached by any other move (a run's
/// first hour, a seek back, a move of two hours, two steps with no frame between) names none, and
/// frames without change facts never do.
#[test]
fn an_hour_stepped_to_names_what_it_changed_and_no_other_move_does() {
    use crate::frame::{ChangeDiscontinuity, HourInput, TableChanges};
    let runs = [(1, 0, 3), (2, 50_000, 1)];
    let (bytes, seen) = fixtures::churned(&runs);
    let file = opened(bytes);
    let countries = TableId::Countries;
    let unwritten = 1_u64 << (0..).find(|&field| countries.field_name(field) == Some("political_power_raw")).unwrap();
    let mut frames = Frames::new(&file, &WATCHED).unwrap().with_change_facts();
    let mut previous: Option<DecodedFrame> = None;
    let (mut at, mut named, mut rebuilt, mut membership, mut quiet) = (0, [0_u32; 2], [0_u32; 2], 0, 0);
    for run in 0..file.run_count() {
        let held = file.run(run).unwrap();
        for ordinal in 0..held.ordinals {
            frames.go_to(run, ordinal).unwrap();
            let frame = frames.frame(held.branch).unwrap();
            for (index, table) in WATCHED.into_iter().enumerate() {
                let Some(before) = &previous else { continue };
                let (old, new) = (&before.tables[table.index()], &frame.tables[table.index()]);
                let identity = old.identity();
                let since = HourInput::new(&frame).changes().since(table, Some(&identity), u64::MAX);
                if SharedView::ptr_eq(old, new) {
                    assert!(matches!(since, TableChanges::Unchanged), "{table:?} at run {run} ordinal {ordinal}: a view kept is unchanged");
                    continue;
                }
                let complete = matches!((&**old, &**new), (TableView::Complete(_), TableView::Complete(_)));
                if ordinal == 0 || !complete {
                    assert!(new.changes_since(&identity).is_none(), "{table:?} at run {run} ordinal {ordinal} names changes");
                    assert!(matches!(since, TableChanges::Discontinuity(_)), "{table:?} at run {run} ordinal {ordinal}: {since:?}");
                    continue;
                }
                let facts = new.changes_since(&identity).unwrap_or_else(|| panic!("{table:?} at run {run} ordinal {ordinal} names no changes"));
                let (TableView::Complete(written), TableView::Complete(written_before)) = (&*seen[at].0.tables[table.index()], &*seen[at - 1].0.tables[table.index()]) else {
                    panic!("the writer's {table:?} at hour {at} is not complete where the read's is");
                };
                let (changed, deleted) = watched_edits(written, written_before);
                assert_eq!(facts.changed().iter().map(|row| row.key).collect::<Vec<_>>(), changed, "{table:?} at run {run} ordinal {ordinal}: changed keys");
                assert_eq!(facts.deleted(), deleted.as_slice(), "{table:?} at run {run} ordinal {ordinal}: deleted keys");
                if changed.is_empty() && deleted.is_empty() {
                    assert!(matches!(since, TableChanges::Unchanged), "{table:?} at run {run} ordinal {ordinal}: an hour that wrote no row is unchanged: {since:?}");
                    rebuilt[index] += 1;
                    continue;
                }
                assert!(matches!(since, TableChanges::Rows(rows) if std::ptr::eq(rows, facts)), "{table:?} at run {run} ordinal {ordinal}: {since:?}");
                for row in facts.changed() {
                    assert_eq!(row.inserted, !written_before.contains_key(row.key), "{table:?} at run {run} ordinal {ordinal}: key {}", row.key);
                    assert_ne!(row.fields, 0, "{table:?} at run {run} ordinal {ordinal}: key {} names no field", row.key);
                }
                named[index] += 1;
                if table == countries {
                    let moved = facts.changed().iter().any(|row| row.inserted) || !facts.deleted().is_empty();
                    let unwritten_since = HourInput::new(&frame).changes().since(countries, Some(&identity), unwritten);
                    if moved {
                        membership += 1;
                        assert!(matches!(unwritten_since, TableChanges::Rows(_)), "ordinal {ordinal}: rows came or went");
                    } else {
                        quiet += 1;
                        assert!(matches!(unwritten_since, TableChanges::Unchanged), "ordinal {ordinal}: no hour writes political power");
                    }
                }
            }
            previous = Some(frame);
            at += 1;
        }
    }
    assert_eq!(at, seen.len());
    let stepped = (seen.len() - runs.len()) as u32;
    assert_eq!(named[0], stepped, "every hour stepped to changes a country");
    assert!(rebuilt[1] > 0, "world tension rebuilt for its sample hour alone, its row unwritten, is unchanged");
    assert!(membership > 0 && quiet > 0, "hours with and without rows coming and going");

    fn since<'a>(frame: &'a DecodedFrame, before: &DecodedFrame) -> TableChanges<'a> {
        let countries = TableId::Countries;
        HourInput::new(frame).changes().since(countries, Some(&before.tables[countries.index()].identity()), u64::MAX)
    }
    let discontinuous = |change: TableChanges<'_>| matches!(change, TableChanges::Discontinuity(ChangeDiscontinuity::SnapshotSeekOrSkippedEdit));
    let mut frames = Frames::new(&file, &WATCHED).unwrap().with_change_facts();
    frames.go_to(0, 40).unwrap();
    let built = frames.frame(1).unwrap();
    frames.seek(0, 39).unwrap();
    assert!(discontinuous(since(&frames.frame(1).unwrap(), &built)), "a seek back");
    frames.go_to(0, 50).unwrap();
    let built = frames.frame(1).unwrap();
    frames.go_to(0, 52).unwrap();
    assert!(discontinuous(since(&frames.frame(1).unwrap(), &built)), "a move of two hours");
    frames.go_to(0, 60).unwrap();
    let built = frames.frame(1).unwrap();
    assert!(frames.step().unwrap() && frames.step().unwrap());
    assert!(discontinuous(since(&frames.frame(1).unwrap(), &built)), "two steps, no frame between");
    frames.go_to(0, 3 * envelope::UNIT_ORDINALS - 1).unwrap();
    let built = frames.frame(1).unwrap();
    frames.go_to(1, 0).unwrap();
    assert!(discontinuous(since(&frames.frame(2).unwrap(), &built)), "a run's first hour");
    frames.go_to(0, 70).unwrap();
    let built = frames.frame(1).unwrap();
    frames.go_to(0, 71).unwrap();
    assert!(matches!(since(&frames.frame(1).unwrap(), &built), TableChanges::Rows(_)), "the next hour after a seek");

    let mut plain = Frames::new(&file, &WATCHED).unwrap();
    plain.go_to(0, 10).unwrap();
    let built = plain.frame(1).unwrap();
    plain.go_to(0, 11).unwrap();
    let next = plain.frame(1).unwrap();
    assert!(next.tables[countries.index()].changes_since(&built.tables[countries.index()].identity()).is_none(), "frames without change facts name none");
    assert!(discontinuous(since(&next, &built)));
}

/// Strings come back blocks after they were last held, as cells and as reasons, after a seek.
#[test]
fn strings_that_return_on_later_blocks_are_read_after_a_seek() {
    let (bytes, _) = fixtures::churned(&[(1, 0, 6)]);
    let file = opened(bytes);
    let mut frames = Frames::new(&file, &WATCHED).unwrap();
    let mut tags = |hour: u32| {
        frames.seek(0, hour).unwrap();
        let frame = frames.frame(1).unwrap();
        let TableView::Complete(TableRows::Countries(rows)) = &*frame.tables[TableId::Countries.index()] else {
            panic!("hour {hour}'s countries are not complete");
        };
        rows.iter().map(|row| (row.id, String::from_utf8(row.tag.clone()).unwrap())).collect::<Vec<_>>()
    };
    let named = |rows: &[(u32, &str)]| rows.iter().map(|&(id, tag)| (id, tag.to_owned())).collect::<Vec<_>>();
    assert_eq!(tags(800), named(&[(1, "old string"), (2, "new string 800"), (3, "cycle 16"), (4, "returning")]));
    assert_eq!(tags(2000), named(&[(1, "old string"), (2, "new string 2000"), (3, "cycle 0"), (4, "returning")]));
    assert_eq!(tags(4319), named(&[(1, "old string"), (2, "new string 4319"), (3, "cycle 6")]));
    let mut frames = Frames::new(&file, &WATCHED).unwrap();
    frames.seek(0, 2100).unwrap();
    assert_eq!(*frames.frame(1).unwrap().tables[TableId::WorldTension.index()], TableView::Unavailable { reason: "refused 0".into() });
}

/// A table whose part in a block cannot be read is unavailable by name from there, and the
/// other tables read on; a seek before that block reads it, and one past it, replaying from the
/// table's last restore before the damage, names it too.
#[test]
fn a_damaged_part_refuses_its_table_by_name_and_no_other() {
    let (bytes, expected) = fixtures::churned(&[(1, 0, 4)]);
    let clean = opened(bytes.clone());
    let entry = clean.table_entry(0, 1, TableId::Countries.index()).unwrap();
    let frame = &entry.unit.parts[0].frames[0];
    let mut damaged = bytes;
    damaged[(frame.offset + u64::from(frame.stored) / 2) as usize] ^= 0x40;
    let file = opened(damaged);
    let read = read_on(&file, &WATCHED);
    let block = envelope::UNIT_ORDINALS as usize;
    for (at, (read, written)) in read.iter().zip(&expected).enumerate() {
        let tension = TableId::WorldTension.index();
        assert_eq!(read.0.tables[tension], written.0.tables[tension], "hour {at}'s world tension differs");
        let countries = &*read.0.tables[TableId::Countries.index()];
        if at < block {
            assert_eq!(countries, &*written.0.tables[TableId::Countries.index()], "hour {at}");
        } else {
            let TableView::Unavailable { reason } = countries else { panic!("hour {at}'s countries read past their damage") };
            assert!(reason.contains("Countries of run 0 in block 1"), "{reason}");
        }
    }
    let mut frames = Frames::new(&file, &WATCHED).unwrap();
    frames.seek(0, 300).unwrap();
    assert_eq!(frames.frame(1).unwrap().tables[TableId::Countries.index()], expected[300].0.tables[TableId::Countries.index()]);
    frames.seek(0, 2 * block as u32 + 5).unwrap();
    let frame = frames.frame(1).unwrap();
    assert!(clean.table_entry(0, 1, TableId::Countries.index()).unwrap().unit.restore.is_none(), "the fixture restores no table at block 1's end");
    let TableView::Unavailable { reason } = &*frame.tables[TableId::Countries.index()] else { panic!("a seek past the damage read it") };
    assert!(reason.contains("Countries of run 0 in block 1"), "{reason}");
    assert_eq!(frame.tables[TableId::WorldTension.index()], expected[2 * block + 5].0.tables[TableId::WorldTension.index()]);
    // Lost to the damage, the same frames sought back before it read the clean hour again.
    frames.seek(0, 300).unwrap();
    assert_eq!(frames.frame(1).unwrap().tables[TableId::Countries.index()], expected[300].0.tables[TableId::Countries.index()], "a seek back before the damage reads it again");
}

/// A recording opened ends where the last footer whose root verifies does, from a file and from
/// memory.
#[test]
fn torn_tails_choose_the_last_committed_root_for_both_inputs() {
    let runs = [(1, 0, 10)];
    let (mut bytes, _) = recording(&runs, every(&runs), |_| {});
    let original = bytes.len();
    append_revision(&mut bytes, 2);
    let committed = bytes.len();
    append_revision(&mut bytes, 3);
    bytes.truncate(bytes.len() - 13);
    let disk = Saved::new(&bytes);
    for reader in [opened(bytes.clone()), disk.open()] {
        assert_eq!(reader.len(), committed as u64);
        assert_eq!(reader.hours(), 10);
    }
    let (root, _) = envelope::last_root(&bytes).unwrap();
    bytes[root.offset as usize] ^= 1;
    assert_eq!(opened(bytes).len(), original as u64);
}

/// A recording opened from its file keeps the revision it opened while a writer appends the
/// next, and reads its hours.
#[test]
fn an_open_recording_keeps_its_revision_while_its_file_grows() {
    let runs = [(1, 0, 10)];
    let (mut bytes, _) = recording(&runs, every(&runs), |_| {});
    let disk = Saved::new(&bytes);
    let pinned = disk.open();
    let before = bytes.len();
    append_revision(&mut bytes, 2);
    disk.append(&bytes[before..]);
    assert_eq!(pinned.len(), before as u64);
    assert_eq!(read_on(&pinned, &[TableId::Countries]).len(), 10);
    assert_eq!(disk.open().len(), bytes.len() as u64);
}

#[test]
fn root_and_run_lists_hold_many_short_runs() {
    for count in [4, 256] {
        let runs: Vec<_> = (0..count).map(|run| (run as u64, 0, 1)).collect();
        let file = opened(recording(&runs, every(&runs), |_| {}).0);
        assert_eq!(file.hours(), count as u64);
        assert_eq!(file.run_count(), count as u32);
        assert_eq!(file.segments().len(), count);
    }
}

/// The columns a format-136 or 134 recording framed (each format's field counts are
/// replay-data's `earlier_native_formats_frame_their_tables_as_written`), the tables whose
/// meaning changed since withheld by name, and since format 140 framed the division tables
/// narrower, no prefix of this build's schema.
#[test]
fn format_136_columns_are_this_builds_but_the_appended() {
    use replay_data::rows::compatibility::AIR_HISTORY_SLOTS_FORMAT;
    let schema = schema().unwrap();
    let columns = columns_at(AIR_HISTORY_SLOTS_FORMAT).unwrap();
    // The division tables whose meaning changed at format 140 are withheld by name; no other.
    let mut changed = [TableId::DivisionMovement, TableId::DivisionPanelStats, TableId::DivisionStatus, TableId::LandCombatDivisions, TableId::LandCombatDivisionModifiers].map(TableId::index).to_vec();
    changed.sort_unstable();
    let named = |format: u16| {
        withheld_at(format)
            .unwrap()
            .into_iter()
            .map(|(table, reason)| {
                assert!(reason.contains(&format!("{:?}", TableId::ALL[table])), "{reason}");
                table
            })
            .collect::<Vec<usize>>()
    };
    assert_eq!(named(AIR_HISTORY_SLOTS_FORMAT), changed);
    // A format-134 capture frames its tables as written and withholds by name those whose
    // layout changed since; every other table reads.
    use replay_data::rows::compatibility::EXACT_NAVAL_CONDITION_FORMAT;
    let mut expected = changed.clone();
    expected.extend([TableId::AirNavalPatrolHistories, TableId::AirMissionInputs].map(TableId::index));
    expected.sort_unstable();
    assert_eq!(named(EXACT_NAVAL_CONDITION_FORMAT), expected);
    let earlier = columns_at(EXACT_NAVAL_CONDITION_FORMAT).unwrap();
    assert_eq!(earlier[TableId::AirNavalPatrolHistories.index()], 3);
    assert_eq!(earlier[TableId::Cryptology.index()], 4);
    assert_eq!(earlier[TableId::RaidAirScores.index()], 6);
    // Each format held this build's tables but those added at format 142, the division tables
    // framed wider then than now: a recording of either format is refused by name as of another
    // schema, not read.
    for held in [&earlier, &columns] {
        assert_eq!(held.len(), TABLE_COUNT - 3);
        assert_eq!(replay_columnar::envelope::schema_digest_prefix(&schema[..held.len()], held), None);
    }
    let current = columns_at(replay_data::FORMAT_VERSION).unwrap();
    assert_eq!(
        replay_columnar::envelope::schema_digest_prefix(&schema, &current),
        Some(replay_columnar::envelope::schema_digest(&schema))
    );
    assert_ne!(
        replay_columnar::envelope::schema_digest_prefix(&schema, &columns),
        Some(replay_columnar::envelope::schema_digest(&schema))
    );
}

/// `bytes` with one more root, naming schema `theirs`.
fn naming_schema(mut bytes: Vec<u8>, theirs: [u8; 32]) -> Vec<u8> {
    let (extent, _) = envelope::last_root(&bytes).unwrap();
    let raw = zstd::bulk::decompress(extent.stored_in(&bytes).unwrap(), 1 << 20).unwrap();
    let mut root = Root::read(&raw).unwrap();
    root.identity.schema = theirs;
    let root = fixtures::append(&mut bytes, Kind::Root, (0, 0, 0, 0), &root.write().unwrap());
    bytes.extend_from_slice(&envelope::footer(root.offset, root.stored, &root.digest));
    bytes
}

/// Opening `bytes` is refused as of schema `theirs`, naming its digest and this build's.
fn refused_naming_both(bytes: Vec<u8>, theirs: &[u8; 32]) {
    let hex = |digest: &[u8; 32]| digest[..8].iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let error = Recording::open(bytes).unwrap_err().to_string();
    let ours = hex(&envelope::schema_digest(&schema().unwrap()));
    assert!(error.contains(&format!("This recording was written with another schema (its digest {}, this build's {ours})", hex(theirs))), "{error}");
}

/// A recording written with another schema is refused naming both digests, the words a person
/// sees: the schema before the log tables (LogForgets a table of its own after
/// DivisionModelInputs, the event tables state) among them.
#[test]
fn another_schema_is_refused_naming_both_digests() {
    let mut before_logs = schema().unwrap();
    before_logs.insert(TableId::DivisionModelInputs.index() + 1, vec![Column::I32]);
    for theirs in [[0xab; 32], envelope::schema_digest(&before_logs)] {
        let runs = [(1, 0, 3)];
        let (bytes, _) = recording(&runs, every(&runs), |_| {});
        refused_naming_both(naming_schema(bytes, theirs), &theirs);
    }
}

/// A recording of format 141, written as that build wrote it (its identity naming that format's
/// schema; its journal, store, pages and restores holding the tables it held, each framed as
/// then), opens and plays: its hours read, each table added at format 142 is not recorded at
/// every hour, its reason named, and refused by every reader of its pages, each field appended at 142 reads as not
/// recorded (an opaque one among them, which a cell decoded at this build's columns gave as an
/// empty payload), and the rest reads as written, across a closed span and an open block. A
/// table keeping its last complete read restores it at every block's end as that format framed
/// it, so a seek past any restore reads what reading on reads. A format-141 capture whose
/// identity names a schema that is no prefix of this build's framed at that format is still
/// refused naming both digests.
#[test]
fn a_recording_of_an_earlier_format_opens_and_reads_what_it_held() {
    use super::synthetic::{Input, Sample};
    use crate::rows::{CountryRow, DivisionRow, DivisionRowAt141, Recorded, StateRow, StateRowAt141};
    use replay_data::rows::compatibility::CAPTURE_ADDITIONS_FORMAT;
    let format = CAPTURE_ADDITIONS_FORMAT - 1;
    let added: Vec<TableId> = TableId::ALL.into_iter().filter(|table| table.last_change().introduced == CAPTURE_ADDITIONS_FORMAT).collect();
    assert_eq!(added, [TableId::LandUnitOrder, TableId::StrategicRegionWeights, TableId::StateBuildingInputs]);
    assert!(LAST_COMPLETE.contains(&TableId::Divisions));
    let states = [StateRowAt141 { id: 4, owner: 7, controller: 9, custom_name: b"Bohemia".to_vec(), continent: Some(2).into(), ..Default::default() }];
    let countries = [CountryRow { id: 7, tag: b"GER".to_vec(), stability_raw: 5, ..Default::default() }];
    let divisions = [
        DivisionRowAt141 { id: (3_u64 << 32) | 1, roster_owner: 7, province: 11, hit_points: 500_000, organisation: 250_000, ..Default::default() },
        DivisionRowAt141 { id: (4_u64 << 32) | 1, roster_owner: 7, province: 12, ..Default::default() },
    ];
    // The block's last hours read the divisions in part, so every block end restores Divisions
    // with its last complete read kept apart.
    let partial = |ordinal: u32| ordinal % envelope::UNIT_ORDINALS >= envelope::UNIT_ORDINALS - 10;
    let runs = [(1, 100, envelope::SPAN_BLOCKS + 1)];
    let (bytes, _) = fixtures::written_at(format, b"format 141", &runs, &[TableId::States, TableId::Countries], |synthetic, _, ordinal| {
        synthetic.table(TableId::States, Input::Snapshot { rows: &states, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        synthetic.table(TableId::Countries, Input::Snapshot { rows: &countries, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
        let input = match partial(ordinal) {
            true => Input::Snapshot { rows: &divisions[..1], partial: Some(1), read: true, sample: Sample::FromReadBit },
            false => Input::Snapshot { rows: &divisions, partial: None, read: true, sample: Sample::FromReadBit },
        };
        synthetic.table(TableId::Divisions, input).unwrap();
    });
    let recording = opened(bytes.clone());
    assert_eq!(recording.tables(), TABLE_COUNT - added.len());
    assert_eq!(recording.columns().len(), TABLE_COUNT);
    let state_columns = TableId::States.last_change().fields(TableId::States, format);
    assert_eq!(recording.columns()[TableId::States.index()], state_columns);
    assert_eq!(TableId::States.field_name(state_columns), Some("marked_owners"));
    assert_eq!(state_columns, TableId::States.field_count() - 1);
    let mode = crate::scope::CaptureMode::from_byte(recording.root_identity().capture_mode).unwrap();
    let block = envelope::UNIT_ORDINALS;
    for table in &added {
        assert_eq!(recording.columns()[table.index()], 0);
        let gap = table.last_change().table_gap(*table, format).unwrap();
        assert_eq!(recording.predates(table.index()), Some(gap.as_str()));
        assert_eq!(recording.withheld_reason(table.index()), Some(gap.as_str()));
        // Its pages name no such table: a reader of them is refused by name, never given rows.
        for error in [recording.table_entry(0, 0, table.index()).map(|_| ()).unwrap_err(), recording.span_summaries(0, 0, table.index()).map(|_| ()).unwrap_err()] {
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert!(error.to_string().contains(&gap), "{error}");
        }
        // A table reader reads it not recorded at every hour, however it moves.
        let mut reader = crate::cells::TableReader::new(&recording, 0, *table, Some(&[0])).unwrap();
        reader.keep_tracks();
        let not_recorded = |reader: &crate::cells::TableReader, at: &str| {
            assert_eq!(reader.status(), Status::NotRecorded, "{table:?} {at}");
            assert!(!reader.was_read() && reader.sampled_at().is_none() && reader.cells().rows().next().is_none(), "{table:?} {at}");
        };
        reader.seek(&recording, block + 3).unwrap();
        not_recorded(&reader, "sought");
        assert!(reader.step(&recording).unwrap());
        not_recorded(&reader, "stepped");
        assert!(reader.changes().quiet() && !reader.changes().status_changed, "{table:?}: a step changes nothing");
        reader.go_to(&recording, 7 * block + 1).unwrap();
        not_recorded(&reader, "advanced");
        let stretch = reader.read_stretch(&recording, 7 * block + 9).unwrap();
        assert_eq!(stretch.statuses, vec![Status::NotRecorded; 8], "{table:?}");
        assert!(stretch.events.is_empty() && stretch.start.is_empty(), "{table:?}");
        assert_eq!(reader.hour(), Some(runs[0].1 + 7 * block as i32 + 9));
    }
    assert_eq!(recording.columns()[TableId::Divisions.index()], TableId::Divisions.field_count() - 1);
    let mut tables = vec![TableId::States, TableId::Countries, TableId::Divisions];
    tables.extend(&added);
    let read = read_on(&recording, &tables);
    assert_eq!(read.len(), runs[0].2 as usize * envelope::UNIT_ORDINALS as usize);
    let state = StateRow {
        id: 4,
        owner: 7,
        controller: 9,
        custom_name: b"Bohemia".to_vec(),
        continent: Some(2).into(),
        marked_owners: Recorded::NotRecorded,
        ..Default::default()
    };
    let division = |at: usize| {
        let held = &divisions[at];
        DivisionRow { id: held.id, roster_owner: held.roster_owner, province: held.province, hit_points: held.hit_points, organisation: held.organisation, theatre: Recorded::NotRecorded, ..Default::default() }
    };
    for (ordinal, (frame, _)) in (0..).zip(&read) {
        assert_eq!(*frame.tables[TableId::States.index()], TableView::Complete(TableRows::States(vec![state.clone()])), "hour {ordinal}");
        assert_eq!(*frame.tables[TableId::Countries.index()], TableView::Complete(TableRows::Countries(countries.to_vec())), "hour {ordinal}");
        let expected = match partial(ordinal) {
            true => TableView::Truncated { rows: TableRows::Divisions(vec![division(0)]), missing: 1 },
            false => TableView::Complete(TableRows::Divisions(vec![division(0), division(1)])),
        };
        assert_eq!(*frame.tables[TableId::Divisions.index()], expected, "hour {ordinal}");
        for table in &added {
            assert_eq!(*frame.tables[table.index()], TableView::NotRecorded { mode }, "hour {ordinal}: {table:?}");
        }
    }
    // A seek just past a block end restores every table there, Divisions with its last complete
    // read as format 141 framed it.
    for ordinal in [block, 6 * block + 3, 12 * block + 1, envelope::SPAN_BLOCKS * block + 2] {
        let mut frames = Frames::new(&recording, &tables).unwrap();
        frames.seek(0, ordinal).unwrap();
        let sought = frames.frame(runs[0].0).unwrap();
        assert_eq!(sought.tables, read[ordinal as usize].0.tables, "sought to {ordinal}");
    }
    // Its schema with a table's columns other than that format framed them, or with the tables
    // added since framed as empty, is no format's: refused naming both digests.
    let schema = schema().unwrap();
    let mut columns = columns_at(format).unwrap();
    let every_table: Vec<usize> = columns.iter().copied().chain(added.iter().map(|_| 0)).collect();
    columns[TableId::Countries.index()] -= 1;
    let narrower = envelope::schema_digest_prefix(&schema[..columns.len()], &columns).unwrap();
    let padded = envelope::schema_digest_prefix(&schema, &every_table).unwrap();
    for theirs in [narrower, padded] {
        refused_naming_both(naming_schema(bytes.clone(), theirs), &theirs);
    }
}

/// A log's cells at an hour are the rows it lists then, as the collector held them: impacts that
/// each live a stretch from their hour, read back by seek, by step (the step's appends as inserted
/// rows and its forgets as deleted ones), across a block's end and by an advance; a stretch of
/// track events is refused by name.
#[test]
fn a_log_reads_at_an_hour_as_the_rows_it_lists() {
    use super::synthetic::{Input, Sample};
    use crate::rows::NuclearStrikeRow;
    const PROVINCE: usize = 3;
    let listed = |ordinal: u32| -> Vec<(u128, i64)> {
        (0..480_u32).filter(|&id| 3 * id <= ordinal && ordinal < 3 * id + id % 40 + 1).map(|id| (u128::from(id), i64::from(id * 7))).collect()
    };
    let (bytes, _) = fixtures::written(b"log at an hour", &[(1, 0, 2)], &[], |synthetic, _, ordinal| {
        let rows: Vec<NuclearStrikeRow> = listed(ordinal)
            .into_iter()
            .map(|(id, province)| NuclearStrikeRow { id: id as u64, hour: id as i32 * 3, province: province as u32, nuke_type: id as u32 % 3, ..Default::default() })
            .collect();
        synthetic.table(TableId::NuclearStrikes, Input::Snapshot { rows: &rows, partial: None, read: true, sample: Sample::FromReadBit }).unwrap();
    });
    let recording = opened(bytes);
    let mut reader = crate::cells::TableReader::new(&recording, 0, TableId::NuclearStrikes, None).unwrap();
    let held = |reader: &crate::cells::TableReader| -> Vec<(u128, i64)> {
        let view = reader.view();
        let mut rows: Vec<(u128, i64)> = view.rows().map(|slot| (view.key(slot).unwrap(), view.number(PROVINCE, slot).unwrap())).collect();
        rows.sort_unstable();
        rows
    };
    for ordinal in [0, 5, 719, 720, 1000, 1439] {
        reader.seek(&recording, ordinal).unwrap();
        assert_eq!(held(&reader), listed(ordinal), "sought to {ordinal}");
        assert!(matches!(reader.status(), replay_columnar::manifest::Status::Complete));
    }
    reader.seek(&recording, 700).unwrap();
    for ordinal in 701..760 {
        let before: Vec<u128> = held(&reader).into_iter().map(|(key, _)| key).collect();
        assert!(reader.step(&recording).unwrap());
        let after: Vec<u128> = held(&reader).into_iter().map(|(key, _)| key).collect();
        assert_eq!(held(&reader), listed(ordinal), "stepped to {ordinal}");
        let view = reader.view();
        let mut inserted: Vec<u128> = reader.changes().inserted.iter().map(|&slot| view.key(slot).unwrap()).collect();
        inserted.sort_unstable();
        let mut deleted: Vec<u128> = reader.changes().deleted.iter().map(|&(_, key)| key).collect();
        deleted.sort_unstable();
        assert_eq!(inserted, after.iter().copied().filter(|key| !before.contains(key)).collect::<Vec<_>>(), "appended at {ordinal}");
        assert_eq!(deleted, before.iter().copied().filter(|key| !after.contains(key)).collect::<Vec<_>>(), "forgotten at {ordinal}");
    }
    reader.go_to(&recording, 1300).unwrap();
    assert_eq!(held(&reader), listed(1300), "advanced to 1300");
    let mut tracked = crate::cells::TableReader::new(&recording, 0, TableId::NuclearStrikes, None).unwrap();
    tracked.keep_tracks();
    tracked.seek(&recording, 10).unwrap();
    let refused = tracked.read_stretch(&recording, 20).unwrap_err().to_string();
    assert!(refused.contains("NuclearStrikes is a log"), "{refused}");
}

/// A log at an hour read in part, as the recorder writes it, lists its rows as at any other hour:
/// the hour's appends and the rows it held before, which no partial read forgets.
#[test]
fn a_log_read_in_part_lists_its_rows() {
    use crate::frame::{FrameInput, Metrics, TableInput, TableView};
    use crate::rows::{NuclearStrikeRow, TABLE_COUNT};
    const PROVINCE: usize = 3;
    let strike = |id: u64| NuclearStrikeRow { id, hour: id as i32, province: id as u32 * 7, ..Default::default() };
    let bytes = fixtures::recorded(b"log read in part", 3, |ordinal| {
        let mut tables: [TableInput; TABLE_COUNT] = std::array::from_fn(|_| TableInput::Sampled(TableView::Unavailable { reason: "outside the fixture".into() }));
        tables[TableId::NuclearStrikes.index()] = TableInput::Sampled(match ordinal {
            0 => TableView::Complete(TableRows::NuclearStrikes(vec![strike(1)])),
            1 => TableView::Truncated { rows: TableRows::NuclearStrikes(vec![strike(2)]), missing: 1 },
            _ => TableView::Complete(TableRows::NuclearStrikes(vec![strike(2)])),
        });
        FrameInput { branch: 1, hour: 10 + ordinal as i32, tables, metrics: Metrics::default() }
    });
    let recording = opened(bytes);
    let read = |reader: &crate::cells::TableReader| -> Vec<i64> {
        let view = reader.view();
        let mut provinces: Vec<i64> = reader.read_rows().unwrap().iter().map(|&slot| view.number(PROVINCE, slot).unwrap()).collect();
        provinces.sort_unstable();
        provinces
    };
    let mut sought = crate::cells::TableReader::new(&recording, 0, TableId::NuclearStrikes, None).unwrap();
    sought.seek(&recording, 1).unwrap();
    assert!(matches!(sought.status(), replay_columnar::manifest::Status::Truncated { missing: 1 }));
    assert_eq!(read(&sought), [7, 14]);
    let mut stepped = crate::cells::TableReader::new(&recording, 0, TableId::NuclearStrikes, None).unwrap();
    stepped.seek(&recording, 0).unwrap();
    assert!(stepped.read_rows().is_none());
    assert!(stepped.step(&recording).unwrap());
    assert_eq!(read(&stepped), [7, 14]);
    assert!(stepped.step(&recording).unwrap());
    assert!(stepped.read_rows().is_none());
}

/// A pool whose last owner lets go of it on one of its own threads ends without waiting on its
/// threads there, which would never finish.
#[test]
fn a_pool_let_go_last_on_its_own_thread_ends() {
    use std::sync::mpsc;
    let pool = Arc::new(Pool::new(2, "pool self drop", || ()).unwrap());
    let (release, released) = mpsc::channel::<()>();
    let (ended, end) = mpsc::channel();
    let held = Arc::clone(&pool);
    pool.install(move || {
        rayon::spawn(move || {
            released.recv().unwrap();
            drop(held);
            ended.send(()).unwrap();
        })
    });
    drop(pool);
    release.send(()).unwrap();
    end.recv().unwrap();
}
