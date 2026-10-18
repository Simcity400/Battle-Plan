use super::*;
use crate::products::special_projects::history_tests::frame;
use dashboard_base::test_fixtures::{CAMPAIGN, records};
use replay::{container::BlockChange, spool::Record};
use replay_data::rows::TableRows;
use replay_frame::{frame::TableUpdate, frame::TableView};

/// Hours 100 to 105 whose rows are equal; at 103 the dictionary is recorded
/// again whole, with the same rows.
fn reread_dictionary() -> Replay {
    let frames: Vec<_> = (100..=105).map(|hour| frame(hour, true, 2)).collect();
    let rows = frames[0]
        .complete::<tables::LexicalTokens>()
        .unwrap()
        .to_vec();
    let mut entries = records(1, frames);
    let mut recorded = 0;
    for entry in &mut entries {
        if let Record::Frame(frame) = entry
            && frame.hour == 103
        {
            frame.tables[TableId::LexicalTokens.index()] =
                TableUpdate::Snapshot(TableRows::LexicalTokens(rows.clone()));
            recorded += 1;
        }
    }
    assert_eq!(recorded, 1);
    Replay::from_records(CAMPAIGN.into(), &[entries]).unwrap()
}

/// The dictionary recorded again with equal rows is a new allocation, which
/// the published dictionary revision counts: the block reader reports it
/// rebuilt at that hour under the family's watch, where the dictionary is
/// watched whole (so the pre-pass observes that hour).
#[test]
fn an_equal_dictionary_recorded_again_is_reported_rebuilt_by_the_block_reader() {
    let replay = reread_dictionary();
    let index = &replay.index[0];
    let lexical = TableId::LexicalTokens.index();
    let mut alone = Vec::new();
    replay
        .read_block_alone(
            index,
            tables(false),
            |_, frame| {
                alone.push(frame.clone());
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    let at = alone.iter().position(|frame| frame.hour == 103).unwrap();
    assert_eq!(alone[at].tables[lexical], alone[at - 1].tables[lexical]);
    assert!(!SharedView::ptr_eq(
        &alone[at].tables[lexical],
        &alone[at - 1].tables[lexical]
    ));
    let watched = fields(true).unwrap();
    assert!(watched.contains(&TableFields::all(TableId::LexicalTokens)));
    let mut rebuilt = Vec::new();
    replay
        .read_block_changes(
            index,
            &watched,
            |change| {
                match change {
                    BlockChange::Changed {
                        frame,
                        rebuilt: tables,
                        ..
                    } => {
                        if tables.contains(&TableId::LexicalTokens) {
                            rebuilt.push(frame.hour);
                        }
                    }
                    BlockChange::Quiet(quiet) => {
                        for (position, &hour) in quiet.hours().iter().enumerate() {
                            if quiet.rebuilt(position, TableId::LexicalTokens) {
                                rebuilt.push(hour);
                            }
                        }
                    }
                }
                Ok(())
            },
            || Ok(()),
        )
        .unwrap();
    // The block's first hour, and the hour read again.
    assert_eq!(rebuilt, [100, 103]);
}

/// The family's watch list names fields every table holds.
#[test]
fn the_watch_list_names_existing_fields() {
    let watched = fields(true).unwrap();
    assert_eq!(
        watched.len(),
        tables(true).len() - 1,
        "every table but the campaign"
    );
    for table in WATCHED_WHOLE {
        assert!(watched.contains(&TableFields::all(table)), "{table:?}");
    }
}

fn extract() -> BlockExtract {
    BlockExtract {
        format: replay_data::format::FormatVersion::CURRENT,
        world: true,
        hours: Vec::new(),
        last: None,
        built: Vec::new(),
        deadline: None,
        frames: 0,
        history_scans: 0,
    }
}

#[test]
fn unscoped_project_reads_leave_out_country_presentation_tables() {
    let selection = bound(false).unwrap();
    for table in installed_content_country_names::country_names::TABLES {
        assert_eq!(
            selection.tables.contains(&table),
            matches!(table, TableId::Countries | TableId::CountryLinks)
        );
    }
    assert!(
        bound(true).is_none(),
        "scoped content retains its whole trigger world"
    );
}

#[test]
fn unchanged_history_keeps_its_completion_deadline_without_scanning_again() {
    let mut extract = extract();
    let mut first = frame(100, true, 2);
    first.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::ProjectHistory(vec![
            replay_data::rows::ProjectHistoryRow {
                owner: 1,
                ordinal: 1,
                date: 105,
                prototype_reward_handle: crate::products::special_projects::COMPLETION_MARKER,
                option_handle: crate::products::special_projects::COMPLETION_MARKER,
                ..Default::default()
            },
        ]))
        .into();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])]).unwrap();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    for hour in 100..110 {
        let mut next = first.clone();
        next.hour = hour;
        let changes = tracker.observe(&next, replay.format);
        extract.observe(&replay, &next, &changes).unwrap();
        assert_eq!(extract.deadline(), (hour < 105).then_some(105));
    }
    assert_eq!(extract.history_scans, 2);
}

/// How `extract` holds `frame`, which it then keeps as its last, as
/// [`Extract::observe`] does.
fn held(extract: &mut BlockExtract, frame: &DecodedFrame) -> Held {
    let (held, built) = extract.held(frame);
    extract.last = Some(frame.clone());
    extract.built = built;
    held
}

/// A frame is the hour's whole frame when a watched table holds other rows
/// than the last frame and no table holds rows built earlier: the block's
/// first, a changed one; a frame built on the last one (a table rebuilt
/// with that one's rows, another's rows changed only where nothing watches)
/// is not.
#[test]
fn an_extract_tells_whole_frames_from_frames_built_on_the_last() {
    let mut extract = extract();
    let first = frame(100, true, 2);
    assert_eq!(extract.held(&first).0, Held::Whole);
    extract.last = Some(first.clone());
    // Built: every table the last one's rows, the countries at a new
    // allocation.
    let mut built = first.clone();
    built.hour = 101;
    let countries = TableId::Countries.index();
    built.tables[countries] = SharedView::from((*first.tables[countries]).clone());
    assert_eq!(extract.held(&built).0, Held::Built);
    // Whole: the projects' rows changed, every other table the last
    // frame's own.
    let mut changed = first.clone();
    changed.hour = 101;
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *changed.tables[TableId::Projects.index()]
    {
        rows[0].state.prototype_progress += 1;
    } else {
        panic!("the fixture records projects");
    }
    assert_eq!(extract.held(&changed).0, Held::Whole);
}

/// The scenario hourly preparation and a block read by its changes once
/// disagreed on. At 101 a completion deadline falls in a quiet hour whose
/// update rebuilt the states (their manpower moved, which nothing watches),
/// so the pre-pass builds 101's frame on 100's: its states hold 100's rows.
/// At 102 a script variable changes and the states do not, so the frame read
/// there carries the states at that built allocation, 100's manpower. A
/// watched table holds other rows, but the frame is not 102's whole frame:
/// it is a read one carrying a built table. At 103 the states are decoded
/// again, the read's own, and the frame is whole.
#[test]
fn a_read_frame_carrying_a_table_built_earlier_is_not_whole() {
    use replay_data::{rows::ScriptVariableEntry, rows::ScriptVariablesRow, rows::StateRow};
    let states = TableId::States.index();
    let variables = TableId::ScriptVariables.index();
    let with = |hour: i32, manpower: i32, flag: i64| {
        let mut frame = frame(hour, true, 2);
        frame.tables[states] = TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner: 1,
            controller: 1,
            manpower_available: manpower,
            ..Default::default()
        }]))
        .into();
        frame.tables[variables] =
            TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
                scope_kind: 1,
                scope_id: 1,
                seed: replay_data::rows::Recorded::Value(None),
                entries: vec![ScriptVariableEntry {
                    name: b"project_flag".to_vec(),
                    value_raw: flag,
                }],
            }]))
            .into();
        frame
    };
    let mut extract = extract();
    let first = with(100, 1_000, 0);
    assert_eq!(held(&mut extract, &first), Held::Whole);
    // 101, built on 100: the states rebuilt at a new allocation of 100's
    // rows (101's manpower is 2,000).
    let mut built = first.clone();
    built.hour = 101;
    built.tables[states] = SharedView::from((*first.tables[states]).clone());
    assert_eq!(held(&mut extract, &built), Held::Built);
    assert_eq!(extract.built, [TableId::States]);
    // 102, read: the variable changed, the states carried at 101's built
    // allocation, holding 100's manpower where 102's is 2,000.
    let read = with(102, 2_000, 100_000);
    let mut carried = read.clone();
    carried.tables[states] = built.tables[states].clone();
    assert!(SharedView::ptr_eq(
        &carried.tables[states],
        &built.tables[states]
    ));
    assert_ne!(carried.tables[states], read.tables[states]);
    assert_eq!(held(&mut extract, &carried), Held::Read);
    assert_eq!(extract.built, [TableId::States]);
    // 103, read: the states decoded again, the read's own.
    let whole = with(103, 3_000, 0);
    let mut own = whole.clone();
    own.tables[TableId::Projects.index()] = carried.tables[TableId::Projects.index()].clone();
    assert_eq!(held(&mut extract, &own), Held::Whole);
    assert!(extract.built.is_empty());
}

/// The owners whose project rows changed between two kept frames are those
/// a recorded delta names: a changed row's, an inserted row's and a deleted
/// key's, and no owner whose rows are equal.
#[test]
fn changed_project_owners_are_those_a_delta_names() {
    use replay_data::rows::ProjectRow;
    let with = |rows: Vec<(u32, u32, i32)>| {
        let mut frame = frame(100, true, 2);
        frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(
            rows.into_iter()
                .map(|(owner, local_id, progress)| {
                    let mut row = ProjectRow {
                        owner,
                        local_id,
                        ..Default::default()
                    };
                    row.state.prototype_progress = progress;
                    row
                })
                .collect(),
        ))
        .into();
        frame
    };
    let before = with(vec![(1, 1, 0), (2, 1, 0), (3, 1, 0), (5, 1, 0)]);
    let after = with(vec![(1, 1, 0), (2, 1, 4), (4, 1, 0), (5, 1, 0), (5, 2, 0)]);
    assert_eq!(
        changed_project_owners(&before, &after),
        Some([2, 3, 4, 5].into())
    );
    assert_eq!(
        changed_project_owners(&before, &before),
        Some(BTreeSet::new())
    );
    let mut unread = after.clone();
    unread.tables[TableId::Projects.index()] = TableView::Unavailable {
        reason: "not read".into(),
    }
    .into();
    assert_eq!(changed_project_owners(&before, &unread), None);
}
