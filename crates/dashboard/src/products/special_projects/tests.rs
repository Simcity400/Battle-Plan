use super::fold::{COMPLETION_MARKER, Entry, Fold, Look, Observed, Person, Row};
use super::*;

const START: i32 = 1_000;
const HOLDER: (u32, u32) = (5, 9);

fn running(local: u32, progress: i32) -> Row {
    Row { local, definition: 40, program: (7_u64 << 32) | u64::from(PROGRAM_REFERENCE), started: true, offset: 0x18, not_completed: 1, prototype_progress: progress }
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
    let shown: Vec<Observed<'_>> = rows.iter().map(|(row, history, person)| Observed { row, history: history.as_deref(), person: person.clone() }).collect();
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
    fold.observe((6, 9), 1_050, 1_059, &[Observed { row: &row, history: Some(&history), person: Person::Unassigned }]);
    assert_eq!(fold.episodes.len(), 5, "another holder's project is a new episode");
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
    assert_eq!(boundaries(20, 150, &[], &spans), [20, 30, 41, 60, 100]);
    assert_eq!(uncovered(&[(30, 40), (60, 300)], 20, 150), [(20, 29), (41, 59)]);
}

#[test]
fn a_dated_project_request_accepts_the_displayed_hour() {
    let params: SpecialProjectsParams = serde_json::from_value(serde_json::json!({"branchId":"1","tag":"GER","hour":23})).unwrap();
    assert_eq!(params.hour, Some(23));
}
