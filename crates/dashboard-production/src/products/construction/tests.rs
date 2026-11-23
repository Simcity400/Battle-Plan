use super::{
    col,
    log::{Assembled, EndKind, LevelsBuilt, ProjectRun, ProjectSource, Rises, UnknownEnd, assemble, rise_hours},
    spending::{deltas, identity_keys, leaf_keys, leaf_of, line_gap, totals},
    *,
};
use replay_lanes::lanes::Step;
use replay_data::rows::{ConstructionRise, ConstructionSpendRow, TagHolderRow};

fn row(set: &[(usize, i64)]) -> Box<[Cell]> {
    let mut cells = vec![Cell::Missing; col::ALL.len()];
    for &(column, value) in set {
        cells[column] = Cell::Number(value);
    }
    cells.into()
}

fn step(hour: i32, set: &[(usize, i64)]) -> Step {
    Step { hour, cells: Some(row(set)) }
}

fn step_with_unrecorded(hour: i32, set: &[(usize, i64)], unrecorded: &[usize]) -> Step {
    let mut cells = row(set);
    for &column in unrecorded {
        cells[column] = Cell::Unrecorded;
    }
    Step { hour, cells: Some(cells) }
}

fn census(factories: i64, remaining: i64, built: i64, repaired: i64, more: &[(usize, i64)]) -> Vec<(usize, i64)> {
    let mut set = vec![
        (col::KIND, i64::from(P::BUILDING)),
        (col::OUTPUT_ALONE, 0),
        (col::COUNTRY, 7),
        (col::BUILDING_TYPE, 300),
        (col::TARGET_STATE, 12),
        (col::COST, 1_080_000_000),
        (col::CREATED_HOUR, 80),
        (col::FIRST_READ, 90),
        (col::CENSUS_READ, 90),
        (col::QUEUE_ORDINAL, 2),
        (col::EARLIER_UNKNOWN, 0),
        (col::FACTORIES, factories),
        (col::REMAINING, remaining),
        (col::REPAIR_LEVELS, 0),
        (col::BUILT, built),
        (col::REPAIRED, repaired),
        (col::END, i64::from(P::OPEN)),
        (col::OUTPUT_REASON, i64::from(P::OUTPUT_NOT_RECORDED)),
    ];
    set.extend_from_slice(more);
    set
}

fn tag(country: u32, _: i32) -> Option<String> {
    (country == 7).then(|| "AAA".to_owned())
}

#[test]
fn the_column_numbers_are_the_schemas() {
    let names = [
        (col::KIND, "kind"),
        (col::OUTPUT_ALONE, "output_alone"),
        (col::COUNTRY, "country"),
        (col::BUILDING_TYPE, "building_type"),
        (col::FIRST_READ, "first_read"),
        (col::FACTORIES, "factories"),
        (col::BUILT_RISES, "built_rises"),
        (col::REPAIRED_RISES, "repaired_rises"),
        (col::UNACCOUNTED_REPAIR, "unaccounted_repair"),
        (col::END_KNOWN_AT, "end_known_at"),
        (col::OUTPUT_REASON, "output_reason"),
        (col::FIRST_REPAIR_TICK, "first_repair_tick"),
    ];
    for (column, name) in names {
        assert_eq!(dashboard_declare::field(TableId::ConstructionProjects, name).unwrap(), column, "{name}");
    }
    assert_eq!(TableId::ConstructionProjects.field_count(), col::ALL.len());
}

#[test]
fn a_window_carries_counts_reached_before_it_and_dates_rises_by_the_row() {
    let steps = vec![
        step(100, &census(2, 2, 1, 0, &[])),
        step(120, &census(2, 1, 2, 0, &[(col::OUTPUT_SINCE, 60), (col::OBSERVED_SINCE_CREATION, 0), (col::OUTPUT_REASON, 0), (col::CONSTRUCTION_RAW, 500), (col::REPAIR_RAW, 0)])),
        step(144, &census(3, 1, 2, 0, &[(col::OUTPUT_SINCE, 60), (col::OBSERVED_SINCE_CREATION, 0), (col::OUTPUT_REASON, 0), (col::CONSTRUCTION_RAW, 500), (col::REPAIR_RAW, 0)])),
        step(168, &census(3, 1, 2, 1, &[(col::OUTPUT_SINCE, 60), (col::OBSERVED_SINCE_CREATION, 0), (col::OUTPUT_REASON, i64::from(P::OUTPUT_LINKED))])),
        Step { hour: 170, cells: None },
        step(180, &census(3, 1, 2, 1, &[(col::OUTPUT_SINCE, 60), (col::OBSERVED_SINCE_CREATION, 0), (col::OUTPUT_REASON, i64::from(P::OUTPUT_LINKED))])),
        step(
            192,
            &census(
                3,
                1,
                3,
                1,
                &[(col::END, i64::from(P::FINISHED)), (col::END_HOUR, 189), (col::END_KNOWN_AT, 192), (col::OUTPUT_SINCE, 60), (col::OBSERVED_SINCE_CREATION, 0), (col::OUTPUT_REASON, i64::from(P::OUTPUT_LINKED))],
            ),
        ),
    ];
    assert_eq!(rise_hours(&steps), [120, 168, 192]);
    let dated = Rises { built: vec![ConstructionRise { hour: 117, count: 1 }], repaired: Vec::new() };
    let rises = |hour: i32| (hour == 120).then_some(&dated);
    let Assembled::Project(project) = assemble(5, &steps, 199, &[], &rises, &tag) else { panic!("the line is shown and named") };
    assert_eq!((project.id.as_str(), project.tag.as_str(), project.source, project.building, project.state), ("5", "AAA", ProjectSource::Census, Some(300), Some(12)));
    assert_eq!(project.first_read, 90);
    assert_eq!(project.repair, Some(false));
    assert_eq!(
        project.runs,
        [
            ProjectRun { first_hour: 100, last_hour: 119, factories: 2, remaining: 2, repair_levels: 0 },
            ProjectRun { first_hour: 120, last_hour: 143, factories: 2, remaining: 1, repair_levels: 0 },
            ProjectRun { first_hour: 144, last_hour: 169, factories: 3, remaining: 1, repair_levels: 0 },
            ProjectRun { first_hour: 180, last_hour: 191, factories: 3, remaining: 1, repair_levels: 0 },
        ]
    );
    let levels = |hour, known_at, count, repair, carried| LevelsBuilt { hour, known_at, count, repair, carried };
    assert_eq!(
        project.built,
        [levels(None, 100, 1, false, true), levels(Some(117), 120, 1, false, false), levels(None, 168, 1, true, false), levels(None, 192, 1, false, false)]
    );
    let end = project.end.as_ref().unwrap();
    assert_eq!((end.kind, end.hour, end.known_at, end.unknown), (EndKind::Finished, Some(189), 192, None));
    assert_eq!(project.outputs.len(), 3);
    assert_eq!((project.outputs[0].from, project.outputs[0].to, project.outputs[0].construction_raw.as_deref()), (120, 167, Some("500")));
    assert!(project.outputs[0].before_observation);
    assert_eq!((project.outputs[1].from, project.outputs[1].to, project.outputs[1].reason, project.outputs[1].construction_raw.clone()), (168, 169, Some(SpendingGap::LinkedOutput), None));
    assert_eq!((project.outputs[2].from, project.outputs[2].to), (180, 199));
}

#[test]
fn an_output_alone_line_keeps_its_origin_and_an_unresolved_owner_is_left_out() {
    let alone = |country: i64| {
        vec![(col::KIND, i64::from(P::BUILDING)), (col::OUTPUT_ALONE, 1), (col::COUNTRY, country), (col::BUILDING_TYPE, 301), (col::TARGET_STATE, 4), (col::FIRST_READ, 50), (col::END, 0), (col::OUTPUT_REASON, 0), (col::OUTPUT_SINCE, 50), (col::CONSTRUCTION_RAW, 10), (col::REPAIR_RAW, 0)]
    };
    let mut taken = census(1, 1, 0, 0, &[(col::OUTPUT_REASON, 0), (col::OUTPUT_SINCE, 50), (col::CONSTRUCTION_RAW, 10), (col::REPAIR_RAW, 0)]);
    taken.retain(|(column, _)| *column != col::FIRST_READ);
    taken.push((col::FIRST_READ, 50));
    let steps = vec![step(50, &alone(7)), step(90, &taken)];
    let none = |_: i32| -> Option<&Rises> { None };
    let Assembled::Project(project) = assemble(6, &steps, 99, &[], &none, &tag) else { panic!("named") };
    assert_eq!(project.source, ProjectSource::Census);
    let origin = project.output_origin.as_ref().unwrap();
    assert_eq!((origin.tag.as_str(), origin.building, origin.state), ("AAA", Some(301), Some(4)));
    assert_eq!(project.runs.len(), 1);
    assert_eq!(project.runs[0].first_hour, 90);
    assert!(matches!(assemble(8, &[step(50, &alone(9))], 99, &[], &none, &tag), Assembled::Unnamed));
    assert!(matches!(assemble(8, &[Step { hour: 50, cells: None }], 99, &[], &none, &tag), Assembled::Hidden));
    let unknown = census(0, 0, 0, 0, &[(col::END, i64::from(P::UNKNOWN_END)), (col::END_REASON, i64::from(P::SHARED_TARGET)), (col::END_KNOWN_AT, 96)]);
    let Assembled::Project(gone) = assemble(9, &[step(96, &unknown)], 99, &[], &none, &tag) else { panic!("named") };
    let end = gone.end.clone().unwrap();
    assert_eq!((end.kind, end.hour, end.known_at, end.unknown), (EndKind::Unknown, None, 96, Some(UnknownEnd::SharedTarget)));
    assert!(gone.runs.is_empty(), "a gone line holds no run");
}

#[test]
fn unread_project_counters_and_classification_fields_stay_unknown_never_defaulted() {
    let none = |_: i32| -> Option<&Rises> { None };
    let levels = |hour, known_at, count, repair, carried| LevelsBuilt { hour, known_at, count, repair, carried };
    let base = census(2, 2, 0, 0, &[]);
    let Assembled::Project(partial) = assemble(
        10,
        &[step(100, &base), step_with_unrecorded(110, &base, &[col::BUILT]), step(120, &base)],
        129,
        &[],
        &none,
        &tag,
    ) else { panic!("named") };
    assert!(partial.built.is_empty());
    assert_eq!(partial.built_unread, Some(vec![(110, 119)]), "an unread built counter records only the unread hour spans");
    assert_eq!(partial.repaired_unread, None, "a recorded zero repaired counter is not marked unread");

    let Assembled::Project(unknown) = assemble(
        11,
        &[step_with_unrecorded(100, &base, &[col::KIND, col::OUTPUT_ALONE, col::REPAIR_LEVELS, col::END])],
        105,
        &[],
        &none,
        &tag,
    ) else { panic!("named") };
    assert_eq!(unknown.kind, ProjectKind::Unknown, "unread kind is unknown, not building");
    assert_eq!(unknown.source, ProjectSource::Unknown, "unread output_alone is unknown, not census");
    assert_eq!(unknown.repair, None, "unread repair_levels stays absent, never false");
    assert!(unknown.runs.is_empty(), "unread repair_levels never fabricates an open run");

    let Assembled::Project(no_repair) = assemble(12, &[step_with_unrecorded(100, &base, &[col::REPAIR_LEVELS])], 105, &[], &none, &tag) else { panic!("named") };
    assert_eq!(no_repair.source, ProjectSource::Census);
    assert_eq!(no_repair.repair, None, "a census line with unread repair_levels leaves repair absent");

    let Assembled::Project(no_end) = assemble(13, &[step_with_unrecorded(100, &census(5, 2, 0, 0, &[]), &[col::END])], 105, &[], &none, &tag) else { panic!("named") };
    assert_eq!(no_end.end, None);
    assert_eq!(
        no_end.runs,
        [ProjectRun { first_hour: 100, last_hour: 105, factories: 5, remaining: 2, repair_levels: 0 }],
        "a census row with only END unread retains its witnessed factories and remaining levels"
    );

    let gone = Step { hour: 106, cells: None };
    let started_one = census(2, 1, 1, 0, &[]);
    let Assembled::Project(disappeared) = assemble(
        14,
        &[step(100, &started_one), step_with_unrecorded(104, &started_one, &[col::BUILT]), gone.clone()],
        120,
        &[],
        &none,
        &tag,
    ) else { panic!("named") };
    assert_eq!(disappeared.built, [levels(None, 100, 1, false, true)]);
    assert_eq!(disappeared.built_unread, Some(vec![(104, 120)]), "unread final counter stays uncertain through window end after the row disappears");

    let resolved_steps = vec![step(100, &started_one), step_with_unrecorded(104, &started_one, &[col::BUILT]), gone.clone(), step(108, &census(2, 0, 2, 0, &[]))];
    assert_eq!(rise_hours(&resolved_steps), [108]);
    let rise_108 = Rises { built: vec![ConstructionRise { hour: 107, count: 1 }], repaired: Vec::new() };
    let rises_108 = |hour: i32| (hour == 108).then_some(&rise_108);
    let Assembled::Project(resolved) = assemble(15, &resolved_steps, 120, &[], &rises_108, &tag) else { panic!("named") };
    assert_eq!(resolved.built, [levels(None, 100, 1, false, true), levels(Some(107), 108, 1, false, false)]);
    assert_eq!(resolved.built_unread, Some(vec![(104, 107)]), "a later recorded counter resolves the earlier unread span");

    let recovered_steps = vec![step_with_unrecorded(100, &census(5, 2, 0, 0, &[]), &[col::BUILT]), step(110, &census(5, 1, 1, 0, &[]))];
    assert_eq!(rise_hours(&recovered_steps), [110], "a counter becoming readable after an unread step looks up its recorded rises");
    let rise_110 = Rises { built: vec![ConstructionRise { hour: 108, count: 1 }], repaired: Vec::new() };
    let rises_110 = |hour: i32| (hour == 110).then_some(&rise_110);
    let Assembled::Project(recovered) = assemble(16, &recovered_steps, 120, &[], &rises_110, &tag) else { panic!("named") };
    assert_eq!(recovered.built, [levels(Some(108), 110, 1, false, false)], "recorded rise list dates the build to 108 instead of marking it carried");
    assert_eq!(recovered.built_unread, Some(vec![(100, 109)]));

    let Assembled::Project(kept_source) = assemble(
        17,
        &[step(100, &census(5, 2, 0, 0, &[])), step_with_unrecorded(110, &census(5, 2, 0, 0, &[]), &[col::OUTPUT_ALONE])],
        120,
        &[],
        &none,
        &tag,
    ) else { panic!("named") };
    assert_eq!(kept_source.source, ProjectSource::Census, "an unread output_alone later in the window keeps earlier witnessed census source");
    assert_eq!(kept_source.runs, [ProjectRun { first_hour: 100, last_hour: 120, factories: 5, remaining: 2, repair_levels: 0 }]);

    let Assembled::Project(truncated_gap) = assemble(
        18,
        &[step(100, &census(5, 2, 0, 0, &[])), step_with_unrecorded(104, &census(5, 2, 0, 0, &[]), &[col::BUILT]), gone],
        120,
        &[(106, 106)],
        &none,
        &tag,
    ) else { panic!("named") };
    assert_eq!(truncated_gap.built_unread, Some(vec![(104, 105)]), "a truncated omission closes the unread span before the truncated hour");
    assert_eq!(truncated_gap.runs, [ProjectRun { first_hour: 100, last_hour: 105, factories: 5, remaining: 2, repair_levels: 0 }]);
}

#[test]
fn a_holding_counts_only_output_after_its_start_and_an_unrecorded_total_stays_unknown() {
    let first = deltas(&[(49, Some(Cell::Number(400))), (60, Some(Cell::Number(450))), (70, None), (80, Some(Cell::Number(500)))], 50);
    assert_eq!(first, [(60, Some(50)), (80, Some(50))]);
    assert_eq!(deltas(&[(55, Some(Cell::Number(30)))], 50), [(55, Some(30))]);
    let second = deltas(&[(90, Some(Cell::Number(20))), (95, Some(Cell::Unrecorded)), (99, Some(Cell::Number(25)))], 90);
    assert_eq!(second, [(90, Some(20)), (95, None)]);
    let steps = totals([first, second].concat());
    let shown: Vec<(i32, Option<&str>)> = steps.iter().map(|step| (step.known_at, step.total_raw.as_deref())).collect();
    assert_eq!(shown, [(60, Some("50")), (80, Some("100")), (90, Some("120")), (95, None)]);
}

#[test]
fn spend_keys_name_one_identity_and_their_leaf() {
    let (low, high) = identity_keys(3, 41);
    let wide = (1u32 << 24) + 12;
    let railway = ConstructionSpendRow { slot: 3, country: 41, repair: true, railway: true, building_type: TagHolderRow::NONE, state: wide, to_state: 300, ..Default::default() }.key();
    let building = ConstructionSpendRow { slot: 3, country: 41, building_type: 300, state: u32::MAX - 1, to_state: TagHolderRow::NONE, ..Default::default() }.key();
    assert!(low <= railway && railway <= high && low <= building && building <= high);
    let other = ConstructionSpendRow { slot: 4, country: 41, ..Default::default() }.key();
    assert!(!(low..=high).contains(&other));
    assert_eq!(leaf_of(railway), (true, true, TagHolderRow::NONE, wide, 300));
    assert_eq!(leaf_of(building), (false, false, 300, u32::MAX - 1, TagHolderRow::NONE));
}

#[test]
fn a_line_works_on_its_kinds_leaves_and_lacks_only_output_it_had() {
    let values: HashMap<usize, i64> = HashMap::from([(col::KIND, 0), (col::COUNTRY, 7), (col::BUILDING_TYPE, 300), (col::TARGET_STATE, 12), (col::FIRST_READ, 90), (col::REPAIRED, 1), (col::OUTPUT_REASON, i64::from(P::OUTPUT_LINKED)), (col::OUTPUT_SINCE, 90), (col::OBSERVED_SINCE_CREATION, 0)]);
    let line = line_of(5, 90, 140, &|column| values.get(&column).copied());
    assert!(line.repair_work && !line.new_work && line.before_observation && line.had_output);
    let none = TagHolderRow::NONE;
    assert_eq!(leaf_keys(&line), [(true, false, 300, 12, none)]);
    let gap = line_gap(&line, Some(141)).unwrap();
    assert_eq!((gap.known_at, gap.reason, gap.until), (90, SpendingGap::LinkedOutput, Some(141)));
    let quiet = Line { output_reason: P::OUTPUT_NOT_RECORDED, had_output: false, ..line.clone() };
    assert_eq!(line_gap(&quiet, None), None, "a line the output table never listed lacks nothing");
    let idle = Line { repair_work: false, ..line.clone() };
    assert_eq!(leaf_keys(&idle), [(false, false, 300, 12, none)], "a line with no work yet is a new building");
    let railway = Line { railway: true, to_state: Some(14), ..idle };
    let spent = ConstructionSpendRow { slot: 3, country: 41, railway: true, building_type: TagHolderRow::NONE, state: 12, to_state: 14, ..Default::default() }.key();
    assert_eq!(leaf_keys(&railway), [leaf_of(spent)], "a railway's project and its spending are one leaf");
}

#[test]
fn a_logged_end_is_the_line_its_final_row_was() {
    use replay_data::rows::{ConstructionLineEndRow, Recorded};
    let rows = [
        P { line: 5, country: 7, building_type: Some(300), target_state: Some(12), first_read: 90, repaired: 1, output_reason: P::OUTPUT_LINKED, output_since: Some(90), ..Default::default() },
        P { line: 6, kind: P::RAILWAY, country: 8, target_state: Some(3), to_state: Some(4), first_read: 70, remaining: Recorded::Value(2), output_since: Some(60), observed_since_creation: true, ..Default::default() },
        P { line: 7, output_alone: true, first_read: 80, construction_raw: Recorded::Value(5), first_repair_tick: Some(81), ..Default::default() },
        P { line: 8, first_read: 50, remaining: Recorded::NotRecorded, construction_raw: Recorded::Value(0), ..Default::default() },
    ];
    for row in &rows {
        let final_row: HashMap<usize, i64> = HashMap::from_iter(
            [
                (col::KIND, Some(i64::from(row.kind))),
                (col::OUTPUT_ALONE, Some(i64::from(row.output_alone))),
                (col::COUNTRY, Some(i64::from(row.country))),
                (col::BUILDING_TYPE, row.building_type.map(i64::from)),
                (col::TARGET_STATE, row.target_state.map(i64::from)),
                (col::TO_STATE, row.to_state.map(i64::from)),
                (col::FIRST_READ, Some(i64::from(row.first_read))),
                (col::REMAINING, match row.remaining { Recorded::Value(count) => Some(i64::from(count)), Recorded::NotRecorded => None }),
                (col::REPAIRED, Some(row.repaired)),
                (col::CONSTRUCTION_RAW, match row.construction_raw { Recorded::Value(amount) => Some(amount), Recorded::NotRecorded => None }),
                (col::OUTPUT_SINCE, row.output_since.map(i64::from)),
                (col::OBSERVED_SINCE_CREATION, Some(i64::from(row.observed_since_creation))),
                (col::OUTPUT_REASON, Some(i64::from(row.output_reason))),
                (col::FIRST_REPAIR_TICK, row.first_repair_tick.map(i64::from)),
            ]
            .into_iter()
            .filter_map(|(column, value)| Some((column, value?))),
        );
        let before = line_of(row.line, 40, 140, &|column| final_row.get(&column).copied());
        let end = ConstructionLineEndRow::of(140, row);
        let logged: HashMap<usize, i64> = HashMap::from_iter(
            [
                (column::<ConstructionLineEndRow>("kind"), Some(i64::from(end.kind))),
                (column::<ConstructionLineEndRow>("output_alone"), Some(i64::from(end.output_alone))),
                (column::<ConstructionLineEndRow>("country"), Some(i64::from(end.country))),
                (column::<ConstructionLineEndRow>("building_type"), end.building_type.map(i64::from)),
                (column::<ConstructionLineEndRow>("target_state"), end.target_state.map(i64::from)),
                (column::<ConstructionLineEndRow>("to_state"), end.to_state.map(i64::from)),
                (column::<ConstructionLineEndRow>("first_read"), Some(i64::from(end.first_read))),
                (column::<ConstructionLineEndRow>("new_work"), Some(i64::from(end.new_work))),
                (column::<ConstructionLineEndRow>("repair_work"), Some(i64::from(end.repair_work))),
                (column::<ConstructionLineEndRow>("before_observation"), Some(i64::from(end.before_observation))),
                (column::<ConstructionLineEndRow>("had_output"), Some(i64::from(end.had_output))),
                (column::<ConstructionLineEndRow>("output_reason"), Some(i64::from(end.output_reason))),
            ]
            .into_iter()
            .filter_map(|(column, value)| Some((column, value?))),
        );
        assert_eq!(line_of_end(row.line, 40, 140, &|column| logged.get(&column).copied()), before, "line {}", row.line);
    }
}

mod recorded {
    use dashboard_base::{Source, test_fixtures::{CAMPAIGN, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{BuildingConstructionRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};

    fn recording(available: bool) -> Source {
        let frames = (100..=110)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                if available {
                    frame.tables[TableId::BuildingConstruction.index()] = TableView::Complete(TableRows::BuildingConstruction(vec![BuildingConstructionRow { id: 1, country: 9001, cost: 500_000, created_hour: 90, ..Default::default() }])).into();
                    frame.tables[TableId::RailwayConstruction.index()] = TableView::Complete(TableRows::RailwayConstruction(Vec::new())).into();
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: "construction".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn queues_held_available_are_recorded_though_no_hour_stamps_them_read() {
        let held = recording(true);
        assert!(super::super::recorded(&dashboard_base::queries::query(&held, 1).unwrap()).unwrap(), "available queues with no read stamp are recorded");
        let unheld = recording(false);
        assert!(!super::super::recorded(&dashboard_base::queries::query(&unheld, 1).unwrap()).unwrap(), "queues never available are not recorded");
    }
}
