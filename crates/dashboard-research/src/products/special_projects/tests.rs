use super::fold::{COMPLETION_MARKER, Entry, Fold, Look, Observed, Person, Row};
use super::*;

const START: i32 = 1_000;
const HOLDER: fold::Possession = (5, 9, START);

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
fn a_completion_read_before_its_history_marker_counts_the_same_choices_in_one_episode() {
    let mut fold = Fold::new(START);
    let scientist = Person::Character(31);
    observe(&mut fold, 1_000, 1_000, &[(running(1, 90), Some(entries(3, None)), scientist.clone()), (running(2, 90), Some(entries(0, None)), scientist.clone())]);
    observe(&mut fold, 1_001, 1_001, &[(done(1), Some(entries(3, None)), Person::Unassigned), (done(2), Some(entries(0, None)), Person::Unassigned)]);
    observe(&mut fold, 1_002, 1_002, &[(done(1), Some(entries(3, Some(1_001))), Person::Unassigned), (done(2), Some(entries(0, Some(1_001))), Person::Unassigned)]);
    assert_eq!(fold.episodes.len(), 2, "a completion whose marker is not read yet is no replacement");
    for (episode, count) in fold.episodes.iter().zip([3, 0]) {
        assert!(episode.runs.iter().all(|run| run.iterations == Some(count)), "the choices are counted, never less one for a marker not read: {:?}", episode.runs);
        let done = &episode.runs[1];
        assert_eq!(fold.looks[done.presentation as usize].person, scientist, "the completed project keeps its scientist");
        let reads: Vec<_> = episode.runs.iter().map(|run| run.iterations_read).collect();
        assert_eq!(reads, [Some(1_000), Some(1_001), None], "a count is final only once its read lists the completion");
    }
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
    fold.observe((6, 9, START), 1_040, 1_049, &shown);
    let row = running(1, 20);
    let history = entries(1, None);
    fold.observe((6, 9, START), 1_050, 1_059, &[Observed { row: &row, read: 1_050, history: Some((1_050, &history)), person: Person::Unassigned }]);
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
fn two_completions_sharing_an_owner_project_and_marker_ordinal_on_different_dates_are_two_events() {
    let marker = i64::from(COMPLETION_MARKER);
    let step = |hour, date: i64| Step { hour, cells: Some(Box::new([Cell::Number(marker), Cell::Number(marker), Cell::Number(date)])) };
    let key = (11_u128 << 64) | (1_u128 << 32) | 4;
    let history = History { keys: vec![(key, vec![step(1_010, 1_008), step(1_011, 1_008), step(1_050, 1_049)])], ..History::default() };
    let mut found = BTreeMap::new();
    completed_in(&mut found, 2, &history, START, 1_100);
    assert_eq!(found, BTreeMap::from([((key, 1_008), (2, 1_010)), ((key, 1_049), (2, 1_050))]), "a replacement's completion reusing the marker ordinal is its own event, each first observed once");
}

#[test]
fn owners_are_every_raw_id_linking_to_the_slot_and_edges_stay_in_the_piece() {
    let links = RunLinks { slots: HashMap::from([(10, vec![(0, 99, Some(5)), (100, 199, Some(6))]), (11, vec![(50, 199, Some(5))]), (12, vec![(0, 199, Some(6))])]) };
    let owners = owner_ids(&links, 5, 20, 150);
    assert_eq!(owners, BTreeMap::from([(10, vec![(20, 99)]), (11, vec![(50, 150)])]));
    let day = STAGGER_PERIOD - 1;
    let piece = |held_from| Piece { run: 0, first: 100, last: 150, slot: 5, id: 1, held_from };
    assert_eq!(owner_ids(&links, 5, piece(0).reach(), 150), BTreeMap::from([(10, vec![(100 - day, 99)]), (11, vec![(100 - day, 150)])]), "an owner reaches back as far as a read that still supplies the first hour");
    assert_eq!(piece(90).reach(), 90, "never past the start of the holder's continuous possession");
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
    fold_piece(&mut fold, &Piece { run: 0, first: 40, last: 60, slot: 6, id: 1, held_from: 0 }, &read, &mut observed, &mut Vec::new(), &AtomicBool::new(false)).unwrap();
    let shown: Vec<_> = fold.episodes.iter().map(|episode| (episode.owner, episode.start.hour, episode.runs.iter().map(|run| (run.first_hour, run.last_hour)).collect::<Vec<_>>())).collect();
    assert_eq!(
        shown,
        [(12, 45, vec![(45, 48), (50, 55)]), (11, 50, vec![(50, 60)])],
        "the read at 47 was of another country's project, so the first of its own is at 50; a raw owner that left keeps no project after it left"
    );
    assert_eq!(observed, [(45, 48), (50, 60)], "an hour whose linked owner has no proven read is not recorded, never an empty list");
    assert_eq!(supplied(&read.owners, 11, None, 50), None, "a row whose read is not recorded is not admitted");
}

fn synthetic(id: &str, first: i32, last: i32, frame: impl Fn(i32) -> replay_frame::frame::DecodedFrame) -> Source {
    use dashboard_base::test_fixtures::{CAMPAIGN, records};
    let replay = replay::campaign::Replay::from_records(CAMPAIGN.into(), &[records(1, (first..=last).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: id.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn folded(id: &str, first: i32, last: i32, frame: impl Fn(i32) -> replay_frame::frame::DecodedFrame) -> [(Fold, Vec<(i32, i32)>, Vec<(i32, i32)>); 2] {
    let source = synthetic(id, first, last, frame);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
    let mut links = HashMap::new();
    let mut direct = Fold::new(first);
    let (mut observed, mut partial) = (Vec::new(), Vec::new());
    for piece in pieces(&query, &country, query.timeline.first, last).0 {
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
    use dashboard_base::test_fixtures::source;
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
    use dashboard_base::test_fixtures::source;
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

#[test]
fn a_completion_marker_after_the_known_completion_is_a_new_episode_and_an_earlier_one_dates_it() {
    let mut fold = Fold::new(START);
    observe(&mut fold, 1_100, 1_110, &[(done(1), Some(entries(3, None)), Person::Unassigned)]);
    observe(&mut fold, 1_111, 1_120, &[(done(1), Some(entries(3, Some(1_099))), Person::Unassigned)]);
    observe(&mut fold, 1_125, 1_130, &[(done(1), Some(entries(3, Some(1_120))), Person::Unassigned)]);
    let completions: Vec<Vec<_>> = fold.episodes.iter().map(|episode| episode.runs.iter().map(|run| (run.first_hour, run.completion.clone(), run.iterations_read)).collect()).collect();
    let date = |hour, exact| Some(ProjectDate { hour, exact });
    assert_eq!(
        completions,
        [vec![(1_100, date(1_100, false), Some(1_100)), (1_111, date(1_099, true), None)], vec![(1_125, date(1_120, true), None)]],
        "a marker before the bound dates that completion; one after it is another completion, never the first one moved"
    );
    let mut fold = Fold::new(START);
    observe(&mut fold, 1_100, 1_123, &[(done(1), Some(entries(3, None)), Person::Unassigned)]);
    observe(&mut fold, 1_124, 1_130, &[(done(1), Some(entries(3, None)), Person::Unassigned)]);
    let reads: Vec<_> = fold.episodes[0].runs.iter().map(|run| (run.first_hour, run.last_hour, run.iterations_read)).collect();
    assert_eq!(reads, [(1_100, 1_130, Some(1_100))], "a count read again unchanged keeps the read every hour of its run can cite, never a later one");
}

#[test]
fn hours_whose_holder_is_unknown_are_named_apart_from_hours_no_one_held() {
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay_frame::frame::TableView;
    const UNKNOWN: i32 = 25;
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        if hour == UNKNOWN {
            *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
        }
        frame
    };
    let replay = replay::campaign::Replay::from_records(CAMPAIGN.into(), &[records(1, (20..=30).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "special-projects-unknown-holder".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
    let (_, holding) = pieces(&query, &country, query.timeline.first, 30);
    assert_eq!(holding.unresolved, [(UNKNOWN, UNKNOWN)]);
    assert!(holding.identities.iter().all(|run| run.last_hour < UNKNOWN || run.first_hour > UNKNOWN));
}

#[test]
fn a_project_gone_from_a_complete_read_and_back_is_another_episode_whatever_its_history_and_the_other_owners_reads() {
    use dashboard_base::test_fixtures::source;
    use replay_data::rows::{ProjectHistoryRow, ProjectRow, SliceReadRow, TableRows};
    use replay_frame::frame::TableView;
    const FIRST: i32 = 20;
    const GONE: std::ops::Range<i32> = 30..35;
    const LAST: i32 = 45;
    const CHOICE: i32 = 25;
    let variants: [(&str, fn(i32) -> Option<i32>); 3] = [
        ("special-projects-one-episode", Some),
        ("special-projects-one-episode-lagging", |hour| Some(hour - 1)),
        ("special-projects-one-episode-unstamped", |hour| (!GONE.contains(&hour)).then_some(hour)),
    ];
    for (id, other) in variants {
        let frame = |hour: i32| {
            let mut frame = source(1, hour, None);
            let mut project = ProjectRow { owner: 9001, local_id: 1, definition_id: 40, program_reference: 1, started: 1, ..Default::default() };
            project.state.current_offset = 0x18;
            project.state.not_completed = 1;
            let choice = ProjectHistoryRow { owner: 9001, project: 1, ordinal: 0, prototype_reward_handle: 3, option_handle: 4, date: CHOICE, ..Default::default() };
            let (projects, history) = match hour {
                _ if GONE.contains(&hour) => (Vec::new(), Vec::new()),
                _ if hour >= GONE.end => (vec![project], vec![choice]),
                _ => (vec![project], Vec::new()),
            };
            *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(projects));
            *frame.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::ProjectHistory(history));
            *frame.tables[TableId::ProjectPrograms.index()] = TableView::Complete(TableRows::empty(TableId::ProjectPrograms));
            let mut stamps: Vec<SliceReadRow> = [TableId::Projects, TableId::ProjectHistory]
                .into_iter()
                .flat_map(|table| [(11, other(hour)), (9001, Some(hour))].into_iter().filter_map(move |(owner, read)| Some(SliceReadRow { table: table.index() as u32, owner, sample_hour: read? })))
                .collect();
            stamps.sort_by_key(|row| (row.table, row.owner));
            *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(stamps));
            frame
        };
        let source = synthetic(id, FIRST, LAST, frame);
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
        assert!(!one_episode(&query, &country, (9001, 1, 40), CHOICE, LAST).unwrap(), "{id}: a choice dated before a complete read showed the project gone belongs to its predecessor, though that one's history was empty and another owner was unread");
        assert!(one_episode(&query, &country, (9001, 1, 40), GONE.end + 1, LAST).unwrap(), "{id}: an hour of the listing project's own episode proves it, with every owner's read reaching back to its stamp");
        assert!(!one_episode(&query, &country, (9001, 1, 41), GONE.end + 1, LAST).unwrap(), "{id}: another definition is another project");
        assert!(!one_episode(&query, &country, (9001, 1, 40), GONE.start, LAST).unwrap(), "{id}: an hour no read shows the project proves nothing");
        for (fold, ..) in folded(&format!("{id}-followed"), FIRST, LAST, frame) {
            let starts: Vec<i32> = fold.episodes.iter().map(|episode| episode.start.hour).collect();
            assert_eq!(starts.last(), Some(&GONE.end), "{id}: the project back after its owner's proven absence is a new episode: {starts:?}");
        }
    }
}

fn project_frame(hour: i32, state: impl Fn(i32) -> (u32, u8), stamped: impl Fn(u32, i32) -> Option<i32>) -> replay_frame::frame::DecodedFrame {
    use dashboard_base::test_fixtures::source;
    use replay_data::rows::{ProjectRow, SliceReadRow, TableRows};
    use replay_frame::frame::TableView;
    let mut frame = source(1, hour, None);
    let (offset, not_completed) = state(hour);
    let mut project = ProjectRow { owner: 9001, local_id: 1, definition_id: 40, program_reference: u64::from(offset == 0x18), started: u8::from(offset == 0x18), ..Default::default() };
    project.state.current_offset = offset;
    project.state.not_completed = not_completed;
    *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(vec![project]));
    *frame.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::empty(TableId::ProjectHistory));
    *frame.tables[TableId::ProjectPrograms.index()] = TableView::Complete(TableRows::empty(TableId::ProjectPrograms));
    let stamped = &stamped;
    let mut stamps: Vec<SliceReadRow> = [TableId::Projects, TableId::ProjectHistory]
        .into_iter()
        .flat_map(|table| [11, 9001].into_iter().filter_map(move |owner| Some(SliceReadRow { table: table.index() as u32, owner, sample_hour: stamped(owner, hour)? })))
        .collect();
    stamps.sort_by_key(|row| (row.table, row.owner));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(stamps));
    frame
}

#[test]
fn a_completion_in_an_owners_complete_read_ends_its_episode_though_another_owner_is_unstamped() {
    const FIRST: i32 = 20;
    const DONE: std::ops::Range<i32> = 30..35;
    const LAST: i32 = 45;
    let state = |hour: i32| if DONE.contains(&hour) { (0xe8, 0) } else { (0x18, 1) };
    let stamped = |owner: u32, hour: i32| (owner != 11 || !DONE.contains(&hour)).then_some(hour);
    for (fold, ..) in folded("special-projects-ended-unstamped", FIRST, LAST, |hour| project_frame(hour, state, stamped)) {
        let starts: Vec<i32> = fold.episodes.iter().map(|episode| episode.start.hour).collect();
        assert_eq!(starts, [FIRST, DONE.end], "the completion read while another owner was unstamped ends the episode, and the project running again is a new one");
    }
}

#[test]
fn a_read_before_the_holder_took_the_tag_supplies_no_list() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    const FIRST: i32 = 20;
    const TAKEN: i32 = 25;
    const STALE: i32 = TAKEN - 1;
    const READ: i32 = 31;
    const LAST: i32 = 40;
    let frame = |hour: i32| {
        let mut frame = project_frame(hour, |_| (0x18, 1), |_, hour| Some(if (STALE..READ).contains(&hour) { STALE } else { hour }));
        if hour < TAKEN {
            let TableView::Complete(TableRows::Countries(countries)) = &mut *frame.tables[TableId::Countries.index()] else { unreachable!() };
            countries[0].tag = b"POL".to_vec();
        }
        frame
    };
    for (_, observed, _) in folded("special-projects-taken", FIRST, LAST, frame) {
        assert_eq!(observed, [(READ, LAST)], "a read from before the country held the tag supplies none of its hours, as the details hold");
    }
}

#[test]
fn a_restart_read_while_another_owner_is_unstamped_ends_the_completed_episode() {
    let mut fold = Fold::new(START);
    observe(&mut fold, 1_020, 1_020, &[(done(1), None, Person::Unassigned)]);
    let restarted = running(1, 5);
    fold.absent(HOLDER, 1_021, 1_021, |raw| raw == restarted.owner, &[Observed { row: &restarted, read: 1_021, history: None, person: Person::Unassigned }]);
    observe(&mut fold, 1_022, 1_030, &[(done(1), None, Person::Unassigned)]);
    let completions: Vec<_> = fold.episodes.iter().map(|episode| episode.runs.last().and_then(|run| run.completion.clone())).collect();
    assert_eq!(
        completions,
        [Some(ProjectDate { hour: 1_020, exact: false }), Some(ProjectDate { hour: 1_022, exact: false })],
        "the project read running between two completed reads is a replacement, never given the earlier completion"
    );
}

#[test]
fn a_tag_held_again_after_another_country_is_a_new_possession_though_no_projects_were_read_between() {
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    const FIRST: i32 = 20;
    const OTHER: std::ops::Range<i32> = 25..30;
    const LAST: i32 = 40;
    let frame = |hour: i32| {
        let mut frame = project_frame(hour, |_| (0x18, 1), |_, hour| Some(hour));
        if OTHER.contains(&hour) {
            let TableView::Complete(TableRows::Countries(countries)) = &mut *frame.tables[TableId::Countries.index()] else { unreachable!() };
            let mut other = countries[0].clone();
            (other.id, other.canonical_slot) = (9100, 8);
            countries[0].tag = b"POL".to_vec();
            countries.push(other);
            *frame.tables[TableId::Projects.index()] = TableView::Unavailable { reason: "Synthetic projects refusal".into() };
        }
        frame
    };
    for (fold, ..) in folded("special-projects-tag-reused", FIRST, LAST, frame) {
        let starts: Vec<i32> = fold.episodes.iter().map(|episode| episode.start.hour).collect();
        assert_eq!(starts, [FIRST, OTHER.end], "another country held the tag between, so the same key read again is another episode");
    }
}

#[test]
fn an_iterations_episode_is_proven_by_its_own_owners_reads_whatever_other_owners_stamps() {
    const FIRST: i32 = 20;
    const CHOICE: i32 = 25;
    const LAST: i32 = 30;
    let source = synthetic("special-projects-own-owner", FIRST, LAST, |hour| project_frame(hour, |_| (0x18, 1), |owner, hour| (owner != 11 || hour != CHOICE).then_some(hour)));
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
    assert!(one_episode(&query, &country, (9001, 1, 40), CHOICE, LAST).unwrap(), "another owner lacking a stamp at the choice hour proves nothing about this owner's project");
}

#[test]
fn an_unknown_state_between_a_completion_and_a_running_read_keeps_the_completion_apart() {
    let mut fold = Fold::new(START);
    observe(&mut fold, 1_020, 1_020, &[(done(1), None, Person::Unassigned)]);
    observe(&mut fold, 1_021, 1_021, &[(Row { offset: 0x99, ..done(1) }, None, Person::Unassigned)]);
    observe(&mut fold, 1_022, 1_030, &[(running(1, 5), None, Person::Unassigned)]);
    let starts: Vec<i32> = fold.episodes.iter().map(|episode| episode.start.hour).collect();
    assert_eq!(starts, [1_020, 1_022], "a state the replay does not know is no evidence the completion was undone");
}

fn counted_frame(hour: i32, choices: impl Fn(i32) -> u32, stamped: impl Fn(u32, i32) -> Option<i32>) -> replay_frame::frame::DecodedFrame {
    use replay_data::rows::{ProjectHistoryRow, TableRows};
    use replay_frame::frame::TableView;
    let mut frame = project_frame(hour, |_| (0x18, 1), stamped);
    let choice = |ordinal| ProjectHistoryRow { owner: 9001, project: 1, ordinal, prototype_reward_handle: 3, option_handle: 4, date: 20, ..Default::default() };
    *frame.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::ProjectHistory((0..choices(hour)).map(choice).collect()));
    frame
}

#[test]
fn a_fall_in_choices_read_while_another_owner_is_unstamped_ends_the_episode_in_the_list_as_in_the_iteration_lookup() {
    const FIRST: i32 = 20;
    const RESET: std::ops::Range<i32> = 25..30;
    const LAST: i32 = 35;
    let choices = |hour: i32| if RESET.contains(&hour) { 0 } else { 2 };
    let stamped = |owner: u32, hour: i32| (owner != 11 || !RESET.contains(&hour)).then_some(hour);
    let frame = |hour| counted_frame(hour, choices, stamped);
    for (fold, ..) in folded("special-projects-reset-unstamped", FIRST, LAST, frame) {
        let starts: Vec<i32> = fold.episodes.iter().map(|episode| episode.start.hour).collect();
        assert_eq!(starts, [FIRST, RESET.start], "the owner's complete history read with fewer choices starts a replacement there, whatever another owner's stamps");
    }
    let source = synthetic("special-projects-reset-lookup", FIRST, LAST, frame);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
    assert!(!one_episode(&query, &country, (9001, 1, 40), FIRST, LAST).unwrap(), "the iteration lookup splits there too");
}

#[test]
fn a_completion_seen_again_after_an_unknown_holder_hour_is_the_same_completion() {
    use replay_frame::frame::TableView;
    const FIRST: i32 = 20;
    const UNKNOWN: i32 = 25;
    const LAST: i32 = 30;
    let frame = |hour: i32| {
        let mut frame = project_frame(hour, |_| (0xe8, 0), |_, hour| Some(hour));
        if hour == UNKNOWN {
            *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
        }
        frame
    };
    for (fold, ..) in folded("special-projects-unknown-gap", FIRST, LAST, frame) {
        assert_eq!(fold.episodes.len(), 1, "one completion, seen on both sides of an hour whose holder is unknown, is counted once: {:?}", fold.episodes);
    }
    let running = |hour: i32| {
        let mut frame = project_frame(hour, |_| (0x18, 1), |_, hour| Some(hour));
        if hour == UNKNOWN {
            *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
        }
        frame
    };
    for (fold, ..) in folded("special-projects-unknown-gap-running", FIRST, LAST, running) {
        let starts: Vec<i32> = fold.episodes.iter().map(|episode| episode.start.hour).collect();
        assert_eq!(starts, [FIRST, UNKNOWN + 1], "a running project is never joined across an hour whose holder is unknown");
    }
}

struct FoldStep {
    hour: i32,
    row: Row,
    history: Option<Vec<Entry>>,
}

fn fold_step(hour: i32, row: Row, history: Option<Vec<Entry>>) -> FoldStep {
    FoldStep { hour, row, history }
}

fn unknown(local: u32) -> Row {
    Row { offset: 0x99, ..done(local) }
}

fn marked(choices: u32, date: i32) -> Option<Vec<Entry>> {
    let mut out: Vec<Entry> = (0..choices).map(|ordinal| Entry { ordinal, completion: false, date: START }).collect();
    out.push(Entry { ordinal: choices, completion: true, date });
    Some(out)
}

fn assignments(steps: &[FoldStep], partial: bool) -> Vec<usize> {
    let mut fold = Fold::new(START);
    fold.watch((11, 1, 40));
    for (at, step) in steps.iter().enumerate() {
        let shown = [Observed { row: &step.row, read: step.hour, history: step.history.as_deref().map(|entries| (step.hour, entries)), person: Person::Unassigned }];
        if partial && at != 0 && at + 1 != steps.len() {
            fold.absent(HOLDER, step.hour, step.hour, |owner| owner == 11, &shown);
        } else {
            fold.observe(HOLDER, step.hour, step.hour, &shown);
        }
    }
    steps.iter().map(|step| fold.assigned(step.hour).expect("every read is assigned")).collect()
}

#[test]
fn one_rule_assigns_every_read_to_an_episode_from_everything_it_established_whether_or_not_every_owner_was_read() {
    let completed_at = |hour, history| fold_step(hour, done(1), history);
    let cases: [(&str, Vec<FoldStep>, bool); 9] = [
        ("a marker after a completion seen before an unknown state is a second completion", vec![completed_at(1_100, None), fold_step(1_101, unknown(1), None), completed_at(1_102, marked(0, 1_102))], true),
        ("a marker after a completion seen while another owner was unread is a second completion", vec![fold_step(1_100, running(1, 50), None), completed_at(1_101, None), completed_at(1_103, marked(0, 1_102))], true),
        ("a marker at or before the known completion dates it", vec![completed_at(1_100, None), completed_at(1_101, None), completed_at(1_102, marked(0, 1_100))], false),
        ("a marker after an exact completion is a second completion", vec![completed_at(1_100, marked(0, 1_099)), fold_step(1_101, unknown(1), None), completed_at(1_102, marked(0, 1_101))], true),
        ("a known end then a restart", vec![completed_at(1_100, None), fold_step(1_101, running(1, 5), None), fold_step(1_102, running(1, 6), None)], true),
        ("an unknown state is no evidence the end was undone", vec![completed_at(1_100, None), fold_step(1_101, unknown(1), None), fold_step(1_102, running(1, 5), None)], true),
        ("a fall in choices", vec![fold_step(1_100, running(1, 5), Some(entries(2, None))), fold_step(1_101, running(1, 6), Some(entries(0, None))), fold_step(1_102, running(1, 7), Some(entries(2, None)))], true),
        ("more choices continue", vec![fold_step(1_100, running(1, 5), Some(entries(1, None))), fold_step(1_101, running(1, 6), Some(entries(2, None))), fold_step(1_102, running(1, 7), Some(entries(3, None)))], false),
        ("a completion read and confirmed continues", vec![fold_step(1_100, running(1, 90), Some(entries(3, None))), completed_at(1_101, Some(entries(3, None))), completed_at(1_102, marked(3, 1_101))], false),
    ];
    for (name, steps, splits) in cases {
        let full = assignments(&steps, false);
        assert_eq!(full.first() != full.last(), splits, "{name}: {full:?}");
        assert_eq!(assignments(&steps, true), full, "{name}: a read while another owner was unread is the same evidence");
    }
}

#[test]
fn the_iteration_lookup_assigns_a_choice_to_the_episode_the_list_does() {
    use replay_data::rows::{ProjectHistoryRow, TableRows};
    use replay_frame::frame::TableView;
    const FIRST: i32 = 20;
    const CHOICE: i32 = 21;
    const LISTED: i32 = 22;
    const LAST: i32 = 25;
    let frame = |hour: i32| {
        let mut frame = project_frame(hour, |_| (0xe8, 0), |_, hour| Some(hour));
        let entry = |ordinal, completion: bool| ProjectHistoryRow {
            owner: 9001,
            project: 1,
            ordinal,
            prototype_reward_handle: if completion { COMPLETION_MARKER } else { 3 },
            option_handle: if completion { COMPLETION_MARKER } else { 4 },
            date: CHOICE,
            ..Default::default()
        };
        let history = if hour >= LISTED { vec![entry(0, false), entry(1, true)] } else { Vec::new() };
        *frame.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::ProjectHistory(history));
        frame
    };
    for (fold, ..) in folded("special-projects-lookup-agrees", FIRST, LAST, frame) {
        let starts: Vec<i32> = fold.episodes.iter().map(|episode| episode.start.hour).collect();
        assert_eq!(starts, [FIRST, LISTED], "a completion dated after the one known since the first hour is a second completion");
    }
    let source = synthetic("special-projects-lookup-agrees-lookup", FIRST, LAST, frame);
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
    assert!(!one_episode(&query, &country, (9001, 1, 40), CHOICE, LISTED).unwrap(), "the choice hour belongs to the predecessor the list ends, so its values are never the replacement's");
    assert!(one_episode(&query, &country, (9001, 1, 40), LISTED, LAST).unwrap());
}
