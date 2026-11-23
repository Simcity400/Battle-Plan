use super::{breakdown::Histories, *};
use dashboard_base::{products::at_hour::Owner, test_fixtures::source};
use replay_data::rows::{
    CountryLinkRow, LexicalTokenRow, Recorded, ResearchAssignmentRow as A, ResearchCoverageRow, ResearchModifierRow, SliceReadRow, TableRows,
};
use dashboard_follow::live::Cells;
use replay_frame::frame::TableView;

fn step(hour: i32, cells: Option<Vec<Cell>>) -> Step {
    Step { hour, cells: cells.map(Vec::into_boxed_slice) }
}

fn assignment(first: i32, after: Option<i32>, last: Option<i32>, ended: Option<i32>, outcome: u8, observed: Option<i32>, awaiting: bool) -> Vec<Cell> {
    let mut cells = vec![Cell::Missing; TableId::ResearchAssignments.field_count()];
    let mut set = |name: &str, cell: Cell| cells[dashboard_declare::field(TableId::ResearchAssignments, name).unwrap()] = cell;
    let optional = |value: Option<i32>| value.map_or(Cell::Missing, |value| Cell::Number(i64::from(value)));
    set("technology_handle", Cell::Number(31));
    set("database_index", Cell::Number(9));
    set("token", Cell::Text(0));
    set("level", Cell::Number(0));
    set("continuity_start_hour", Cell::Number(100));
    set("first_observed_hour", Cell::Number(i64::from(first)));
    set("started_after_hour", optional(after));
    set("last_observed_hour", optional(last));
    set("ended_by_hour", optional(ended));
    set("outcome", Cell::Number(i64::from(outcome)));
    set("outcome_observed_hour", optional(observed));
    set("awaiting", Cell::Number(i64::from(awaiting)));
    cells
}

fn assignments(keys: Vec<(u128, Vec<Step>)>) -> History {
    History { columns: all_columns(TableId::ResearchAssignments), to: 400, texts: vec![b"radio".to_vec().into_boxed_slice()], keys }
}

#[test]
fn coverage_is_each_stretch_a_row_lives_and_adjacent_stretches_join() {
    let (holder, _) = coverage_keys(5, 7);
    let since = |hour: i32| Some(vec![Cell::Number(i64::from(hour))]);
    let history = History {
        columns: vec![0],
        to: 300,
        texts: Vec::new(),
        keys: vec![
            (holder, vec![step(100, since(100)), step(130, None), step(150, since(150)), step(170, since(170))]),
            (holder | 1, vec![step(160, since(160)), step(200, None)]),
            (holder | u128::from(ResearchCoverageRow::HOLDER), vec![step(100, since(100)), step(124, since(124)), step(140, None), step(150, since(150))]),
        ],
    };
    let mut covered = Vec::new();
    let mut slots = BTreeMap::new();
    coverage(&history, &mut covered, &mut slots);
    let span = |first_hour, last_hour| ResearchCoverage { first_hour, last_hour };
    assert_eq!(covered, [span(100, 139), span(150, 300)]);
    assert_eq!(slots[&0], [span(100, 129), span(150, 300)]);
    assert_eq!(slots[&1], [span(160, 199)]);
}

#[test]
fn an_assignment_is_its_rows_last_cells_across_lanes_and_one_still_active_is_observed_through_the_last_read() {
    let (base, _) = assignment_keys(5, 7);
    let lane = |index: u128, lane: u128| base | index << 16 | lane;
    let history = assignments(vec![
        (lane(0, 0), vec![step(100, Some(assignment(100, None, None, None, A::ACTIVE, None, false))), step(200, Some(assignment(200, Some(176), None, None, A::ACTIVE, None, false)))]),
        (lane(0, 1), vec![
            step(200, Some(assignment(100, None, Some(176), Some(200), A::SWITCHED, None, true))),
            step(220, Some(assignment(100, None, Some(176), Some(200), A::COMPLETED, Some(220), false))),
            step(221, None),
        ]),
        (lane(1, 0), vec![step(150, Some(vec![Cell::Missing; TableId::ResearchAssignments.field_count()]))]),
    ]);
    let mut unreadable = 0;
    let slots = entries(&history, Some(392), None, &mut unreadable).unwrap();
    assert_eq!(unreadable, 1);
    let shown: Vec<_> = slots[&0].iter().map(|entry| (entry.first_observed_hour, entry.outcome, entry.ended_by_hour, entry.outcome_observed_hour, entry.last_observed_hour)).collect();
    assert_eq!(shown, [(100, ResearchOutcome::Completed, Some(200), Some(220), 176), (200, ResearchOutcome::Active, None, None, 392)]);
    assert_eq!(slots[&0][0].token, "radio");
    assert_eq!(slots[&0][0].database_index, 9);
    let slots = entries(&history, Some(392), Some(401), &mut unreadable).unwrap();
    assert_eq!((slots[&0][1].outcome, slots[&0][1].ended_by_hour), (ResearchOutcome::Unknown, Some(401)));
}

fn input(index: u32, handle: u32, occurrence: i32, token: &[u8], progress: i64) -> ResearchInputRow {
    ResearchInputRow {
        country: 11,
        index,
        technology_handle: handle,
        technology_present: u8::from(handle != 0),
        token: token.to_vec(),
        progress_raw: progress,
        cost_raw: 100 * 100_000,
        cost_scale_raw: 100_000,
        research_speed_raw: 100_000,
        database_index: if handle != 0 { Recorded::Value(occurrence) } else { Recorded::NotRecorded },
        ..Default::default()
    }
}

fn frame(hour: i32, read: i32, inputs: Vec<ResearchInputRow>, modifiers: Vec<ResearchModifierRow>) -> replay_frame::frame::DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    let mut set = |table: TableId, rows: TableRows| *frame.tables[table.index()] = TableView::Complete(rows);
    set(TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: 11, canonical_slot: 5 }, CountryLinkRow { id: 12, canonical_slot: 6 }]));
    set(TableId::ResearchCoverage, TableRows::ResearchCoverage(vec![ResearchCoverageRow { holder_slot: 5, holder_id: 7, member: ResearchCoverageRow::HOLDER, since: read }]));
    set(
        TableId::ResearchAssignments,
        TableRows::ResearchAssignments(vec![A {
            holder_slot: 5,
            holder_id: 7,
            index: 0,
            lane: 0,
            technology_handle: 31,
            database_index: 9,
            token: b"radio".to_vec(),
            first_observed_hour: read - 48,
            started_after_hour: Some(read - 72),
            outcome: A::ACTIVE,
            ..Default::default()
        }]),
    );
    set(
        TableId::SliceReads,
        TableRows::SliceReads([INPUTS, BONUSES, MODIFIERS].map(|table| SliceReadRow { table, owner: 11, sample_hour: read }).into_iter().chain([SliceReadRow { table: INPUTS, owner: 12, sample_hour: read - 5 }]).collect()),
    );
    set(TableId::ResearchInputs, TableRows::ResearchInputs(inputs));
    set(TableId::ResearchAppliedBonuses, TableRows::ResearchAppliedBonuses(Vec::new()));
    set(TableId::ResearchModifiers, TableRows::ResearchModifiers(modifiers));
    set(TableId::LexicalTokens, TableRows::LexicalTokens(vec![LexicalTokenRow { token: 41, literal: b"radar".to_vec() }]));
    frame
}

fn inventory(owner: u32) -> Vec<ResearchModifierRow> {
    let row = |ordinal: u32, kind: u8, source_id: u32, raw: i64| ResearchModifierRow { country: owner, index: grants::INVENTORY_INDEX, ordinal, kind, source_id, raw, name: if kind == ResearchModifierRow::SPEED_GRANT { b"grant_a".to_vec() } else { Vec::new() }, ..Default::default() };
    vec![row(0, ResearchModifierRow::GRANT_COUNTER, 0, 3), row(1, ResearchModifierRow::SPEED_GRANT, 2, 10_000), row(2, ResearchModifierRow::GRANT_USES, 2, 1), row(3, ResearchModifierRow::GRANT_TECHNOLOGY, 2, 41)]
}

#[test]
fn slots_at_an_hour_join_the_active_assignment_and_say_why_they_are_unknown() {
    let owner = Owner { slot: 5, id: 7 };
    let none = |_: &str| -> Option<String> { None };
    let rows = vec![input(0, 31, 9, b"radio", 40 * 100_000), input(1, 0, 0, b"", 0)];
    let live = Cells::default();
    let (read, slots, grants) = slots::slots(&frame(130, 120, rows.clone(), inventory(11)), owner, 130, &none, &live).unwrap();
    assert_eq!(read, 120);
    assert_eq!(slots.len(), 2);
    let researching = &slots[0];
    assert_eq!((researching.state.first_hour, researching.state.last_hour), (120, 120), "as read, whatever the hour asked");
    let live = live.finish();
    assert_eq!(live.groups.len(), 1);
    assert_eq!((live.groups[0].table, live.groups[0].keys.clone()), (TableId::ResearchInputs, vec![11u128 << 32]));
    let json = live.json(&[dashboard_follow::live::GroupState::Follow(1)]);
    assert_eq!(json["rows"], serde_json::json!(["0"]));
    assert_eq!(json["kinds"][0]["id"], "progress");
    assert_eq!(json["kinds"][0]["op"], "ratio");
    assert_eq!(researching.state.entry_first_observed_hour, Some(72));
    assert_eq!((researching.state.token.as_deref(), researching.state.database_index), (Some("radio"), Some(9)));
    let progress = researching.progress.as_ref().unwrap();
    assert_eq!((progress.progress_raw.as_str(), progress.base_days_raw.as_deref()), ("4000000", Some("10000000")));
    assert_eq!(researching.state.details.as_ref().unwrap().remaining_days, None);
    assert_eq!((slots[1].state.token.as_ref(), slots[1].state.database_index, slots[1].progress.as_ref()), (None, None, None));
    assert_eq!(grants.grants.len(), 1);
    assert_eq!(grants.grants[0].technologies[0].token, "radar");
    let (_, mut slots, _) = slots::slots(&frame(130, 120, vec![input(0, 31, 9, b"radio", 0), input(1, 32, 10, b"radar", 0)], Vec::new()), owner, 130, &none, &Cells::default()).unwrap();
    assert_eq!((slots[1].state.entry_first_observed_hour, slots[1].state.database_index, slots[1].progress.as_ref()), (None, Some(10), None), "an unjoined read keeps the definition it read");
    slots::defined(&mut slots, &|token, index| (Some(format!("{token} #{index}")), None));
    assert_eq!(slots[1].definition.as_ref().map(|text| (text.database_index, text.name.as_deref())), Some((10, Some("radar #10"))), "its text is looked up by its own template");
    assert_eq!(slots[0].definition, None, "a joined read takes its text from its assignment");
    assert!(slots::slots(&frame(130, 120, rows.clone(), Vec::new()), owner, 130, &none, &Cells::default()).unwrap().2.unavailable.is_some());
    let mut uncovered = frame(130, 120, rows.clone(), Vec::new());
    *uncovered.tables[TableId::ResearchCoverage.index()] = TableView::Complete(TableRows::ResearchCoverage(Vec::new()));
    assert_eq!(slots::slots(&uncovered, owner, 130, &none, &Cells::default()).unwrap_err(), "No research read covers this hour");
    assert!(slots::slots(&frame(130, 120, vec![input(1, 31, 9, b"radio", 0)], Vec::new()), owner, 130, &none, &Cells::default()).is_err());
}

#[test]
fn a_breakdown_follows_one_assignment_read_by_read_and_stops_where_its_slot_holds_another() {
    let key = 11u128 << 32;
    let sample = |hour: i32| Some(vec![Cell::Number(i64::from(hour))]);
    let samples = History {
        columns: vec![0],
        to: 400,
        texts: Vec::new(),
        keys: vec![(slice_key(INPUTS, 11), vec![step(100, sample(100)), step(124, sample(124)), step(148, sample(148)), step(172, sample(172))])],
    };
    let cells = |row: &ResearchInputRow| {
        let mut cells = vec![Cell::Missing; TableId::ResearchInputs.field_count()];
        let mut set = |name: &str, cell: Cell| cells[dashboard_declare::field(TableId::ResearchInputs, name).unwrap()] = cell;
        set("technology_handle", Cell::Number(i64::from(row.technology_handle)));
        set("technology_present", Cell::Number(i64::from(row.technology_present)));
        set("token", Cell::Text(u32::from(row.technology_handle != 31)));
        for (name, value) in [("classifier", 0), ("level", 0), ("progress_raw", row.progress_raw), ("cost_raw", row.cost_raw), ("cost_scale_raw", row.cost_scale_raw), ("cost_debug", 0)] {
            set(name, Cell::Number(value));
        }
        for (name, value) in [("research_speed_raw", row.research_speed_raw), ("banked_days_raw", 0), ("applied_days_raw", 0), ("ahead_reduction_raw", 0), ("speed_bonus_raw", 0), ("bonus_count", 0)] {
            set(name, Cell::Number(value));
        }
        if let Recorded::Value(index) = row.database_index {
            set("database_index", Cell::Number(i64::from(index)));
        }
        Some(cells)
    };
    let inputs = History {
        columns: all_columns(TableId::ResearchInputs),
        to: 400,
        texts: vec![b"radio".to_vec().into_boxed_slice(), b"radar".to_vec().into_boxed_slice()],
        keys: vec![(key, vec![
            step(100, cells(&input(0, 31, 9, b"radio", 100_000))),
            step(124, cells(&input(0, 31, 9, b"radio", 300_000))),
            step(148, cells(&input(0, 31, 9, b"radio", 500_000))),
            step(172, cells(&input(0, 32, 10, b"radar", 0))),
        ])],
    };
    let empty = || History { columns: Vec::new(), to: 400, texts: Vec::new(), keys: Vec::new() };
    let histories = Histories { samples, inputs, slot_modifiers: empty(), shared_modifiers: empty(), bonuses: empty() };
    let (states, progress) = breakdown::assignment(&histories, 11, 0, 100, 400, true, &|_| None).unwrap();
    let days = |state: &ResearchSlotState| state.details.as_ref().unwrap().remaining_days;
    assert_eq!(states.iter().map(|state| (state.first_hour, state.last_hour, days(state))).collect::<Vec<_>>(), [(100, 123, Some(99)), (124, 171, Some(49))]);
    assert_eq!(progress.iter().map(|sample| (sample.first_hour, sample.progress_raw.as_str())).collect::<Vec<_>>(), [(100, "100000"), (124, "300000"), (148, "500000")]);
    let (states, _) = breakdown::assignment(&histories, 11, 0, 100, 400, false, &|_| None).unwrap();
    assert_eq!(days(&states[0]), None);
    assert!(breakdown::assignment(&histories, 11, 0, 172, 400, true, &|_| None).is_ok());
    assert!(breakdown::assignment(&histories, 11, 1, 100, 400, true, &|_| None).is_err());
}

#[test]
fn a_state_read_again_continues_while_only_its_days_left_count_down() {
    let read = |first_hour: i32, last_hour: i32, remaining_days: Option<u32>| ResearchSlotState {
        first_hour,
        last_hour,
        token: Some("radio".into()),
        database_index: Some(9),
        entry_first_observed_hour: Some(100),
        speed_multiplier_raw: "100000".into(),
        banked_days_raw: "0".into(),
        applied_days_raw: "0".into(),
        ahead_reduction_years_raw: Some("0".into()),
        details: Some(ResearchDetails { remaining_days, estimate_raw: None, bonuses: Some(Vec::new()), modifiers: None }),
    };
    assert!(continues(&read(100, 123, Some(10)), &read(124, 147, Some(9))));
    assert!(continues(&read(100, 123, Some(10)), &read(124, 147, Some(10))));
    assert!(!continues(&read(100, 123, Some(10)), &read(124, 147, Some(7))));
    assert!(!continues(&read(100, 120, Some(10)), &read(124, 147, Some(9))));
}

#[test]
fn the_days_left_estimate_keeps_the_fraction_the_game_rounds_up() {
    let row = |left: i64| ResearchInputRow { progress_raw: 100 * 100_000 - left, ..input(0, 31, 9, b"radio", 0) };
    assert_eq!(super::remaining(&row(1_050_000), 150_000), Some((7, 700_000)), "10.5 base days at 1.5x is exactly 7 days");
    assert_eq!(super::remaining(&row(1_000_000), 300_000), Some((4, 333_334)), "10 base days at 3x is 3 1/3 days, which the game shows as 4");
    assert_eq!(super::remaining(&row(300_001), 300_000), Some((2, 100_001)), "3.00001 base days at 3x is just over a day, which the game shows as 2");
    assert_eq!(super::remaining(&row(0), 300_000), Some((0, 0)));
    assert_eq!(super::remaining(&row(1_000_000), 0), None);
}
