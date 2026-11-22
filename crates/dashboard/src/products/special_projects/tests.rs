use super::fold::{COMPLETION_MARKER, Entry, Fold, Look, Observed, Person, Row};
use super::*;

const START: i32 = 1_000;
const HOLDER: (u32, u32) = (5, 9);

fn running(local: u32, progress: i32) -> Row {
    Row { owner: 11, local, definition: 40, program: (7_u64 << 32) | u64::from(PROGRAM_REFERENCE), started: true, offset: 0x18, not_completed: 1, prototype_progress: progress }
}

fn done(local: u32) -> Row {
    Row { offset: 0xe8, not_completed: 0, program: 0, ..running(local, 100) }
}

fn entries(count: u32, completion: Option<i32>) -> Vec<Entry> {
    let mut out: Vec<Entry> = (0..count).map(|ordinal| Entry { ordinal, completion: false, date: START }).collect();
    if let Some(date) = completion {
        out.push(Entry { ordinal: count, completion: true, date });
    }
    out
}

fn observe(fold: &mut Fold, first: i32, last: i32, rows: &[(Row, Option<Vec<Entry>>, Person)]) {
    let shown: Vec<Observed<'_>> = rows.iter().map(|(row, history, person)| Observed { row, read: first, history: history.as_deref().map(|entries| (first, entries)), person: person.clone() }).collect();
    fold.observe(HOLDER, first, last, &shown);
}

#[test]
fn a_completed_project_keeps_its_last_scientist_and_its_history_date() {
    let mut fold = Fold::new(START);
    let scientist = Person::Character(31);
    observe(&mut fold, 1_000, 1_023, &[(running(1, 40), Some(entries(1, None)), scientist.clone())]);
    observe(&mut fold, 1_024, 1_047, &[(running(1, 40), Some(entries(1, None)), scientist.clone())]);
    observe(&mut fold, 1_048, 1_071, &[(running(1, 70), Some(entries(2, None)), scientist.clone())]);
    observe(&mut fold, 1_072, 1_200, &[(done(1), Some(entries(2, Some(1_060))), Person::Unassigned)]);
    assert_eq!(fold.episodes.len(), 1);
    let episode = &fold.episodes[0];
    assert_eq!(episode.start, ProjectDate { hour: 1_000, exact: false });
    let runs: Vec<_> = episode.runs.iter().map(|run| (run.first_hour, run.last_hour, run.progress, run.state, run.iterations)).collect();
    assert_eq!(runs, [(1_000, 1_047, 40, ProjectState::Running, Some(1)), (1_048, 1_071, 70, ProjectState::Running, Some(2)), (1_072, 1_200, 100, ProjectState::Completed, Some(2))]);
    let last = episode.runs.last().unwrap();
    assert_eq!(last.completion, Some(ProjectDate { hour: 1_060, exact: true }));
    assert_eq!(fold.looks[last.presentation as usize], Look { definition: 40, person: scientist, as_of: Some(1_071) });
}

#[test]
fn projects_of_two_raw_owners_sharing_a_local_id_are_their_own_episodes() {
    let mut fold = Fold::new(START);
    let other = Row { owner: 12, ..running(1, 25) };
    observe(&mut fold, 1_000, 1_010, &[(running(1, 40), None, Person::Unassigned), (other.clone(), None, Person::Unassigned)]);
    observe(&mut fold, 1_011, 1_020, &[(running(1, 41), None, Person::Unassigned), (Row { prototype_progress: 26, ..other }, None, Person::Unassigned)]);
    let episodes: Vec<_> = fold.episodes.iter().map(|episode| (episode.owner, episode.local, episode.runs.iter().map(|run| run.progress).collect::<Vec<_>>())).collect();
    assert_eq!(episodes, [(11, 1, vec![40, 41]), (12, 1, vec![25, 26])]);
}

#[test]
fn a_completion_without_a_campaign_date_is_bounded_by_its_first_observation() {
    let mut fold = Fold::new(START);
    observe(&mut fold, 1_000, 1_010, &[(done(1), Some(entries(3, Some(900))), Person::Unassigned), (done(2), None, Person::Unassigned)]);
    observe(&mut fold, 1_020, 1_030, &[(done(1), Some(entries(3, Some(900))), Person::Unassigned), (done(2), None, Person::Unassigned)]);
    for episode in &fold.episodes {
        assert_eq!(episode.runs.len(), 2, "the unobserved hours end the run");
        for run in &episode.runs {
            assert_eq!(run.completion, Some(ProjectDate { hour: 1_000, exact: false }));
        }
    }
    assert_eq!(fold.episodes[0].runs[0].iterations, Some(3));
    assert_eq!(fold.episodes[1].runs[0].iterations, None, "no history is not zero iterations");
}

#[test]
fn episodes_end_at_a_restart_a_fall_in_iterations_and_another_holder() {
    let mut fold = Fold::new(START);
    let cancelled = Row { offset: 0x1e8, started: false, program: 0, ..running(2, 30) };
    observe(&mut fold, 1_000, 1_009, &[(running(1, 10), Some(entries(3, None)), Person::Unassigned), (running(2, 30), None, Person::Unassigned)]);
    observe(&mut fold, 1_010, 1_019, &[(running(1, 20), Some(entries(1, None)), Person::Unassigned), (cancelled.clone(), None, Person::Unassigned)]);
    observe(&mut fold, 1_020, 1_029, &[(running(1, 20), Some(entries(1, None)), Person::Unassigned), (Row { prototype_progress: 31, ..cancelled.clone() }, None, Person::Unassigned)]);
    observe(&mut fold, 1_030, 1_039, &[(running(1, 20), Some(entries(1, None)), Person::Unassigned), (running(2, 0), None, Person::Unassigned)]);
    let starts: Vec<_> = fold.episodes.iter().map(|episode| (episode.id.as_str(), episode.start.hour)).collect();
    assert_eq!(starts, [("1:1000:0", 1_000), ("2:1000:1", 1_000), ("1:1010:2", 1_010), ("2:1030:3", 1_030)]);
    let cancellations: Vec<_> = fold.episodes[1].runs.iter().map(|run| run.cancellation.clone()).collect();
    assert_eq!(cancellations, [None, Some(ProjectDate { hour: 1_010, exact: false }), Some(ProjectDate { hour: 1_010, exact: false })]);
    let shown: Vec<Observed<'_>> = Vec::new();
    fold.observe((6, 9), 1_040, 1_049, &shown);
    let row = running(1, 20);
    let history = entries(1, None);
    fold.observe((6, 9), 1_050, 1_059, &[Observed { row: &row, read: 1_050, history: Some((1_050, &history)), person: Person::Unassigned }]);
    assert_eq!(fold.episodes.len(), 5, "another holder's project is a new episode");
}

#[test]
fn a_replacement_project_counts_no_choices_read_before_it() {
    let mut fold = Fold::new(START);
    let earlier = entries(3, Some(1_000));
    let (finished, replacement) = (done(1), Row { definition: 41, ..running(1, 5) });
    let at = |row, read, history| Observed { row, read, history, person: Person::Unassigned };
    fold.observe(HOLDER, 1_000, 1_009, &[at(&finished, 1_000, Some((1_000, &earlier[..])))]);
    fold.observe(HOLDER, 1_010, 1_033, &[at(&replacement, 1_010, Some((1_000, &earlier[..])))]);
    fold.observe(HOLDER, 1_034, 1_040, &[at(&replacement, 1_034, Some((1_034, &[][..])))]);
    let shown: Vec<_> = fold.episodes.iter().map(|episode| (episode.start.hour, episode.runs.iter().map(|run| (run.first_hour, run.last_hour, run.iterations)).collect::<Vec<_>>())).collect();
    assert_eq!(
        shown,
        [(1_000, vec![(1_000, 1_009, Some(3))]), (1_010, vec![(1_010, 1_033, None), (1_034, 1_040, Some(0))])],
        "choices read before the replacement first appeared are its predecessor's, and its own empty history continues it"
    );
}

#[test]
fn unstarted_projects_are_hidden_and_unknown_states_are_named() {
    let mut fold = Fold::new(START);
    let waiting = Row { started: false, program: 0, ..running(1, 0) };
    let odd = Row { offset: 0x99, ..running(2, 5) };
    observe(&mut fold, 1_000, 1_000, &[(waiting, None, Person::Unassigned), (odd, None, Person::Unassigned)]);
    assert_eq!(fold.episodes.len(), 1);
    assert_eq!(fold.episodes[0].runs[0].state, ProjectState::Unknown);
    assert_eq!(fold.warnings.values().sum::<usize>(), 1);
}

#[test]
fn a_programs_scientist_is_a_character_no_one_or_unknown_by_name() {
    let program = (7_u64 << 32) | u64::from(PROGRAM_REFERENCE);
    let character = [Cell::Number(((31_u64 << 32) | u64::from(CHARACTER_REFERENCE)) as i64)];
    assert_eq!(person(0, None), Person::Unassigned);
    assert_eq!(person(program, Some(Some(&character[..]))), Person::Character(31));
    assert_eq!(person(program, Some(Some(&[Cell::Number(0)][..]))), Person::Unassigned);
    assert!(matches!(person(program, Some(None)), Person::Unknown(_)));
    assert!(matches!(person(program, Some(Some(&[Cell::Unrecorded][..]))), Person::Unknown(_)));
    assert!(matches!(person(7, None), Person::Unknown(_)));
}

#[test]
fn the_latest_completion_entry_dated_by_the_observation_dates_it() {
    let marker = i64::from(COMPLETION_MARKER);
    let entry = |ordinal, reward, date| history_entry(ordinal, &[Cell::Number(reward), Cell::Number(marker), Cell::Number(date)]).unwrap();
    assert!(entry(0, marker, 5).completion);
    assert!(!entry(0, 3, 5).completion);
    let history = [entry(0, marker, 1_010), entry(1, marker, 1_030), entry(2, marker, 1_090), entry(3, 3, 1_040)];
    assert_eq!(fold::completion_date(&history, 1_050, START), Some(ProjectDate { hour: 1_030, exact: true }));
    assert_eq!(fold::completion_date(&history, 1_005, START), None);
}

#[test]
fn owners_are_every_raw_id_linking_to_the_slot_and_edges_stay_in_the_piece() {
    let links = RunLinks { slots: HashMap::from([(10, vec![(0, 99, Some(5)), (100, 199, Some(6))]), (11, vec![(50, 199, Some(5))]), (12, vec![(0, 199, Some(6))])]) };
    let owners = owner_ids(&links, 5, 20, 150);
    assert_eq!(owners, BTreeMap::from([(10, vec![(20, 99)]), (11, vec![(50, 150)])]));
    let spans: [&[(i32, i32)]; 2] = [&[(20, 99)], &[(30, 40), (60, 300)]];
    assert_eq!(boundaries(20, 150, &[], &spans, &History::default()), [20, 30, 41, 60, 100]);
    let stamps = History { keys: vec![(0, vec![Step { hour: 25, cells: Some(Box::new([Cell::Number(25)])) }])], ..History::default() };
    assert_eq!(boundaries(20, 150, &[], &spans, &stamps), [20, 25, 30, 41, 25 + STAGGER_PERIOD, 60, 100], "a read's expiry ends what it supplied, though no row changes");
    assert_eq!(uncovered(&[(30, 40), (60, 300)], 20, 150), [(20, 29), (41, 59)]);
}

#[test]
fn a_dated_project_request_accepts_the_displayed_hour() {
    let params: SpecialProjectsParams = serde_json::from_value(serde_json::json!({"branchId":"1","country":"GER","hour":23})).unwrap();
    assert_eq!(params.hour, Some(23));
}

#[test]
fn a_reply_spells_only_the_projects_and_scientists_that_first_appeared_since_the_last() {
    let at = |pairs: &[(u32, i32)]| pairs.iter().copied().collect::<BTreeMap<u32, i32>>();
    let (defs, chars) = (at(&[(40, START), (41, START + 24)]), at(&[(7, START)]));
    assert!(matches!(respell(None, &defs, &chars), Respell::All), "nothing spelled yet");
    assert!(matches!(respell(Some((&defs, &chars)), &defs, &chars), Respell::Kept), "the same projects and scientists");
    let Respell::Added(new_defs, new_chars) = respell(Some((&defs, &chars)), &at(&[(40, START), (41, START + 24), (42, START + 48)]), &at(&[(7, START), (8, START + 48)])) else {
        panic!("a project and a scientist appeared");
    };
    assert_eq!((new_defs, new_chars), (at(&[(42, START + 48)]), at(&[(8, START + 48)])), "spelled at the hour each first appeared, the rest kept");
    assert!(matches!(respell(Some((&defs, &chars)), &at(&[(40, START - 24), (41, START + 24)]), &chars), Respell::All), "a project first seen earlier is spelled again where it now starts");
    assert!(matches!(respell(Some((&defs, &chars)), &at(&[(40, START)]), &chars), Respell::All), "a project gone leaves its refusals behind unless all are spelled again");
}

#[test]
fn a_row_read_while_its_owner_was_linked_to_another_country_is_never_this_countrys() {
    let steps = |reads: &[(i32, [i64; 6])]| reads.iter().map(|(hour, cells)| Step { hour: *hour, cells: Some(cells.iter().map(|&value| Cell::Number(value)).collect()) }).collect::<Vec<_>>();
    let samples = |hours: &[i32]| hours.iter().map(|&hour| Step { hour, cells: Some(Box::new([Cell::Number(i64::from(hour))])) }).collect::<Vec<_>>();
    let rows = |keys: Vec<(u128, Vec<Step>)>| History { keys, ..History::default() };
    let read = PieceRows {
        projects: vec![
            rows(vec![((11_u128 << 32) | 3, steps(&[(47, [40, 0, 1, 0x18, 1, 17]), (50, [40, 0, 1, 0x18, 1, 20])]))]),
            rows(vec![((12_u128 << 32) | 4, steps(&[(45, [41, 0, 1, 0x18, 1, 5])]))]),
        ],
        histories: vec![History::default(), History::default()],
        programs: History::default(),
        reads: rows(vec![(read_key(TableId::Projects, 11), samples(&[47, 50])), (read_key(TableId::Projects, 12), samples(&[45]))]),
        owners: BTreeMap::from([(11, vec![(49, 60)]), (12, vec![(40, 55)])]),
        projects_available: vec![(40, 60)],
        projects_partial: Vec::new(),
        history_available: Vec::new(),
    };
    let mut fold = Fold::new(0);
    let mut observed = Vec::new();
    fold_piece(&mut fold, &Piece { run: 0, first: 40, last: 60, slot: 6, id: 1 }, &read, &mut observed, &mut Vec::new(), &AtomicBool::new(false)).unwrap();
    let shown: Vec<_> = fold.episodes.iter().map(|episode| (episode.owner, episode.start.hour, episode.runs.iter().map(|run| (run.first_hour, run.last_hour)).collect::<Vec<_>>())).collect();
    assert_eq!(
        shown,
        [(12, 45, vec![(45, 48), (50, 55)]), (11, 50, vec![(50, 60)])],
        "the read at 47 was of another country's project, so the first of its own is at 50; a raw owner that left keeps no project after it left"
    );
    assert_eq!(observed, [(45, 48), (50, 60)], "an hour whose linked owner has no proven read is not recorded, never an empty list");
    assert_eq!(supplied(&read.owners, 11, None, 50), None, "a row whose read is not recorded is not admitted");
}

fn folded(id: &str, first: i32, last: i32, frame: impl Fn(i32) -> replay_frame::frame::DecodedFrame) -> [(Fold, Vec<(i32, i32)>, Vec<(i32, i32)>); 2] {
    use crate::test_fixtures::{CAMPAIGN, records};
    let replay = replay::campaign::Replay::from_records(CAMPAIGN.into(), &[records(1, (first..=last).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(crate::SourceKey { source_id: id.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = crate::answer::query(&source, 1).unwrap();
    let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
    let mut links = HashMap::new();
    let mut direct = Fold::new(first);
    let (mut observed, mut partial) = (Vec::new(), Vec::new());
    for piece in pieces(&query, &country, last).0 {
        let read = read_piece(&query, links_of(&query, &mut links, piece.run).unwrap(), &piece).unwrap();
        fold_piece(&mut direct, &piece, &read, &mut observed, &mut partial, &AtomicBool::new(false)).unwrap();
    }
    [(direct, observed, partial), incremental::followed(&query, &country, first, last).unwrap()]
}

fn runs(fold: &Fold) -> Vec<(u32, Vec<(i32, i32, Option<i32>)>)> {
    fold.episodes.iter().map(|episode| (episode.local, episode.runs.iter().map(|run| (run.first_hour, run.last_hour, run.iterations)).collect())).collect()
}

#[test]
fn a_read_older_than_a_day_supplies_no_list_and_no_iterations_though_no_row_changes() {
    use crate::test_fixtures::source;
    use replay_data::rows::{ProjectHistoryRow, ProjectRow, SliceReadRow, TableRows};
    use replay_frame::frame::TableView;
    const FIRST: i32 = 20;
    const LAST: i32 = 60;
    const LAST_PROJECTS_READ: i32 = 30;
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        let mut project = ProjectRow { owner: 9001, local_id: 1, definition_id: 40, program_reference: 1, started: 1, ..Default::default() };
        project.state.current_offset = 0x18;
        project.state.not_completed = 1;
        let choice = |ordinal| ProjectHistoryRow { owner: 9001, project: 1, ordinal, prototype_reward_handle: 3, option_handle: 4, date: FIRST, ..Default::default() };
        *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(vec![project]));
        *frame.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::ProjectHistory(vec![choice(0), choice(1)]));
        *frame.tables[TableId::ProjectPrograms.index()] = TableView::Complete(TableRows::empty(TableId::ProjectPrograms));
        let stamp = |table: TableId, sample_hour| [11, 9001].map(|owner| SliceReadRow { table: table.index() as u32, owner, sample_hour });
        let mut stamps = [stamp(TableId::Projects, hour.min(LAST_PROJECTS_READ)), stamp(TableId::ProjectHistory, FIRST)].concat();
        stamps.sort_by_key(|row| (row.table, row.owner));
        *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(stamps));
        frame
    };
    for (fold, observed, _) in folded("special-projects-expired", FIRST, LAST, frame) {
        assert_eq!(
            runs(&fold),
            [(1, vec![(FIRST, FIRST + STAGGER_PERIOD - 1, Some(2)), (FIRST + STAGGER_PERIOD, LAST_PROJECTS_READ + STAGGER_PERIOD - 1, None)])],
            "a history read a day old counts no iterations"
        );
        assert_eq!(observed, [(FIRST, LAST_PROJECTS_READ + STAGGER_PERIOD - 1)], "a projects read a day old is not recorded, never the last list");
    }
}

#[test]
fn a_read_of_projects_or_choices_in_part_ends_no_project_and_counts_no_iterations() {
    use crate::test_fixtures::source;
    use replay_data::rows::{ProjectHistoryRow, ProjectRow, SliceReadRow, TableRows};
    use replay_frame::frame::TableView;
    const FIRST: i32 = 20;
    const LAST: i32 = 40;
    const PROJECTS_IN_PART: i32 = 30;
    const CHOICES_IN_PART: i32 = 33;
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        let project = |local_id| {
            let mut row = ProjectRow { owner: 9001, local_id, definition_id: 40, program_reference: 1, started: 1, ..Default::default() };
            row.state.current_offset = 0x18;
            row.state.not_completed = 1;
            row
        };
        let choice = |ordinal| ProjectHistoryRow { owner: 9001, project: 1, ordinal, prototype_reward_handle: 3, option_handle: 4, date: FIRST, ..Default::default() };
        *frame.tables[TableId::Projects.index()] = if hour == PROJECTS_IN_PART {
            TableView::Truncated { rows: TableRows::Projects(vec![project(1)]), missing: 1 }
        } else {
            TableView::Complete(TableRows::Projects(vec![project(1), project(2)]))
        };
        *frame.tables[TableId::ProjectHistory.index()] = if hour == CHOICES_IN_PART {
            TableView::Truncated { rows: TableRows::ProjectHistory(vec![choice(0)]), missing: 1 }
        } else {
            TableView::Complete(TableRows::ProjectHistory(vec![choice(0), choice(1)]))
        };
        *frame.tables[TableId::ProjectPrograms.index()] = TableView::Complete(TableRows::empty(TableId::ProjectPrograms));
        let mut stamps: Vec<SliceReadRow> =
            [TableId::Projects, TableId::ProjectHistory].into_iter().flat_map(|table| [11, 9001].map(|owner| SliceReadRow { table: table.index() as u32, owner, sample_hour: hour })).collect();
        stamps.sort_by_key(|row| (row.table, row.owner));
        *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(stamps));
        frame
    };
    for (fold, observed, partial) in folded("special-projects-in-part", FIRST, LAST, frame) {
        let shown = runs(&fold);
        assert_eq!(
            shown,
            [
                (1, vec![(FIRST, PROJECTS_IN_PART - 1, Some(2)), (PROJECTS_IN_PART + 1, CHOICES_IN_PART - 1, Some(2)), (CHOICES_IN_PART, CHOICES_IN_PART, None), (CHOICES_IN_PART + 1, LAST, Some(2))]),
                (2, vec![(FIRST, PROJECTS_IN_PART - 1, Some(0)), (PROJECTS_IN_PART + 1, CHOICES_IN_PART - 1, Some(0)), (CHOICES_IN_PART, CHOICES_IN_PART, None), (CHOICES_IN_PART + 1, LAST, Some(0))]),
            ],
            "a project missing from a read in part is not ended, and a history read in part is no count"
        );
        assert_eq!(observed, [(FIRST, PROJECTS_IN_PART - 1), (PROJECTS_IN_PART + 1, LAST)]);
        assert_eq!(partial, [(PROJECTS_IN_PART, PROJECTS_IN_PART)], "the hour read in part is named apart from an unrecorded one");
    }
}
