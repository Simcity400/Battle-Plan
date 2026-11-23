use super::facts::{COMPLETION_MARKER, Facts, Look, Person};
use super::incremental::Follow;
use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay_data::rows::{CountryLinkRow, CountryRow, ProjectHistoryRow, ProjectProgramRow, ProjectRow, SliceReadRow, TableRows};
use replay_frame::frame::TableView;

const START: i32 = 1_000;
const RUNNING: (u32, u8, u8, u64) = (0x18, 1, 1, 7 << 32 | PROGRAM_REFERENCE as u64);
const COMPLETED: (u32, u8, u8, u64) = (0xe8, 0, 1, 0);
const CANCELLED: (u32, u8, u8, u64) = (0x1e8, 1, 0, 0);
const UNKNOWN: (u32, u8, u8, u64) = (0x99, 1, 1, 0);
const POOL: (u32, u8, u8, u64) = (0x18, 1, 0, 0);

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
fn owners_are_every_raw_id_linking_to_the_slot_continuous_spans_merged() {
    let links = RunLinks { slots: HashMap::from([(10, vec![(0, 49, Some(5)), (50, 99, Some(5)), (100, 199, Some(6))]), (11, vec![(50, 199, Some(5))]), (12, vec![(0, 199, Some(6))])]) };
    assert_eq!(owner_ids(&links, 5, 20, 150), BTreeMap::from([(10, vec![(20, 99)]), (11, vec![(50, 150)])]));
    let piece = |held_from| Piece { run: 0, first: 100, last: 150, slot: 5, id: 1, held_from };
    assert_eq!(piece(0).reach(), 100 - (STAGGER_PERIOD - 1), "an owner reaches back as far as a read that still supplies the first hour");
    assert_eq!(piece(90).reach(), 90, "never past the start of the holder's continuous possession");
    let owners = BTreeMap::from([(10, vec![(20, 30), (32, 60)])]);
    assert_eq!(supplied(&owners, 10, Some(25), 28), Some(25));
    assert_eq!(supplied(&owners, 10, Some(25), 40), None, "a read from before the owner left and came back does not supply an hour after");
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
fn hours_whose_holder_is_unknown_are_named_apart_from_hours_no_one_held() {
    const UNKNOWN_HOUR: i32 = 25;
    let source = synthetic("special-projects-unknown-holder", 20, 30, |hour| {
        let mut frame = source(1, hour, None);
        if hour == UNKNOWN_HOUR {
            *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
        }
        frame
    });
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
    let (_, holding) = pieces(&query, &country, query.timeline.first, 30);
    assert_eq!(holding.unresolved, [(UNKNOWN_HOUR, UNKNOWN_HOUR)]);
    assert!(holding.identities.iter().all(|run| run.last_hour < UNKNOWN_HOUR || run.first_hour > UNKNOWN_HOUR));
}

fn synthetic(id: &str, first: i32, last: i32, frame: impl Fn(i32) -> replay_frame::frame::DecodedFrame) -> Source {
    let replay = replay::campaign::Replay::from_records(CAMPAIGN.into(), &[records(1, (first..=last).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: id.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

struct Hour {
    state: Option<(u32, u8, u8, u64)>,
    history: Vec<(u32, u32, i32)>,
    projects_read: Option<i32>,
    history_read: Option<i32>,
    other_read: bool,
    projects_table: bool,
    linked: bool,
    links_table: bool,
    holder: Option<bool>,
    program: Option<(u64, bool)>,
}

fn hour(at: i32, state: (u32, u8, u8, u64)) -> Hour {
    Hour { state: Some(state), history: Vec::new(), projects_read: Some(at), history_read: Some(at), other_read: true, projects_table: true, linked: true, links_table: true, holder: Some(true), program: None }
}

fn frame(at: i32, spec: &Hour) -> replay_frame::frame::DecodedFrame {
    let mut frame = source(1, at, None);
    let row = spec.state.map(|(offset, not_completed, started, program_reference)| {
        let mut row = ProjectRow { owner: 9001, local_id: 1, definition_id: 40, program_reference, started, ..Default::default() };
        row.state.current_offset = offset;
        row.state.not_completed = not_completed;
        row
    });
    *frame.tables[TableId::Projects.index()] =
        if spec.projects_table { TableView::Complete(TableRows::Projects(row.into_iter().collect())) } else { TableView::Unavailable { reason: "Synthetic projects refusal".into() } };
    let history = spec.history.iter().map(|&(ordinal, reward, date)| ProjectHistoryRow {
        owner: 9001,
        project: 1,
        ordinal,
        prototype_reward_handle: reward,
        option_handle: if reward == COMPLETION_MARKER { COMPLETION_MARKER } else { 4 },
        date,
        ..Default::default()
    });
    *frame.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::ProjectHistory(history.collect()));
    let programs = spec.program.map(|(scientist, _)| ProjectProgramRow { id: 7, owner: 9001, active_project: Some(1), scientist, ..Default::default() });
    *frame.tables[TableId::ProjectPrograms.index()] = TableView::Complete(TableRows::ProjectPrograms(programs.into_iter().collect()));
    frame.metrics.table_ns[TableId::ProjectPrograms.index()] = u64::from(spec.program.is_some_and(|(_, read)| read));
    let stamp = |table: TableId, owner: u32, read: Option<i32>| read.map(|sample_hour| SliceReadRow { table: table.index() as u32, owner, sample_hour });
    let other = spec.other_read.then_some(at);
    let mut stamps: Vec<SliceReadRow> = [
        stamp(TableId::Projects, 11, other),
        stamp(TableId::ProjectHistory, 11, other),
        stamp(TableId::Projects, 9001, spec.projects_read),
        stamp(TableId::ProjectHistory, 9001, spec.history_read),
    ]
    .into_iter()
    .flatten()
    .collect();
    stamps.sort_by_key(|row| (row.table, row.owner));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(stamps));
    if let TableView::Complete(TableRows::CountryLinks(links)) = &mut *frame.tables[TableId::CountryLinks.index()] {
        if !spec.linked {
            links.retain(|link: &CountryLinkRow| link.id != 9001);
        }
    }
    if !spec.links_table {
        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "Synthetic links refusal".into() };
    }
    if let TableView::Complete(TableRows::Countries(countries)) = &mut *frame.tables[TableId::Countries.index()] {
        match spec.holder {
            Some(true) => {}
            Some(false) => countries[0].tag = b"POL".to_vec(),
            None => {
                let other = CountryRow { id: 9100, canonical_slot: 8, ..countries[0].clone() };
                countries[0].tag = b"POL".to_vec();
                countries.push(other);
            }
        }
    }
    frame
}

fn recorded(id: &str, first: i32, last: i32, spec: impl Fn(i32) -> Hour) -> Facts {
    let source = synthetic(id, first, last, |at| frame(at, &spec(at)));
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let country: Country = serde_json::from_value(serde_json::json!("GER")).unwrap();
    let cancelled = AtomicBool::new(false);
    let batch = |at: i32| {
        let mut follow = Follow::new(query.timeline.first);
        follow.to(&query, &country, at, &cancelled).unwrap();
        follow
    };
    let mut stepped = Follow::new(query.timeline.first);
    for at in first..=last {
        stepped.to(&query, &country, at, &cancelled).unwrap();
        let direct = batch(at);
        assert_eq!((&stepped.facts, &stepped.holding), (&direct.facts, &direct.holding), "{id}: the follow equals the batch at {at}");
    }
    let middle = (first + last) / 2;
    stepped.to(&query, &country, middle, &cancelled).unwrap();
    assert_eq!(stepped.facts, batch(middle).facts, "{id}: a rewind equals the batch at {middle}");
    batch(last).facts
}

fn runs(facts: &Facts) -> Vec<(i32, i32, ProjectState, Option<i32>)> {
    facts.projects.iter().flat_map(|project| &project.runs).map(|run| (run.first, run.last, run.state, run.iterations)).collect()
}

fn markers(facts: &Facts) -> Vec<(i32, i32, bool)> {
    facts.projects.iter().flat_map(|project| &project.entries).filter(|kept| kept.entry.completion()).map(|kept| (kept.entry.date, kept.read, kept.counted)).collect()
}

const M: u32 = COMPLETION_MARKER;

#[test]
fn t1_a_fresh_empty_history_beside_an_older_completed_state_invents_no_replacement() {
    let facts = recorded("t1", 20, 26, |at| Hour { history: if at == 20 { vec![(0, 3, 20), (1, 3, 20), (2, M, 20)] } else { Vec::new() }, projects_read: Some(20), ..hour(at, COMPLETED) });
    assert_eq!(runs(&facts), [(20, 20, ProjectState::Completed, Some(0)), (21, 26, ProjectState::Completed, None)], "a history listing fewer entries than before makes the count not recorded");
    assert_eq!(markers(&facts), [(20, 20, true)], "one completion");
    assert_eq!(facts.projects.len(), 1);
    assert!(facts.projects[0].attempts.is_empty(), "missing entries prove no new attempt");
}

#[test]
fn t2_a_marker_read_late_is_its_own_dated_completion_and_never_dates_the_status() {
    let facts = recorded("t2", 20, 28, |at| Hour { history: if at >= 26 { vec![(0, M, 21)] } else { Vec::new() }, ..hour(at, if at < 24 { RUNNING } else { COMPLETED }) });
    assert_eq!(runs(&facts), [(20, 23, ProjectState::Running, Some(0)), (24, 28, ProjectState::Completed, Some(0))]);
    assert_eq!(markers(&facts), [(21, 26, true)]);
}

#[test]
fn t3_markers_are_kept_once_each_across_an_unknown_state_and_unavailable_projects() {
    let facts = recorded("t3", 20, 26, |at| Hour {
        history: match at { 20..=22 => vec![(0, M, 20)], _ => vec![(0, M, 20), (1, M, 22)] },
        projects_table: at != 22,
        ..hour(at, if at == 20 { COMPLETED } else { UNKNOWN })
    });
    assert_eq!(markers(&facts), [(20, 20, true), (22, 23, true)], "each marker once, from the read that first listed it, however often it is read again");
    assert_eq!(facts.observed, [(20, 21), (23, 26)], "an hour with projects unavailable is not published");
    assert_eq!(facts.projects[0].attempts.len(), 1, "a marker followed by a later entry in one read proves another attempt");
}

#[test]
fn t4_a_changed_entry_at_an_ordinal_proves_another_attempt_between_the_two_reads() {
    let facts = recorded("t4", 20, 23, |at| Hour { history: if at < 22 { vec![(0, 41, 20)] } else { vec![(0, 43, 21)] }, ..hour(at, RUNNING) });
    let attempts: Vec<_> = facts.projects[0].attempts.iter().map(|attempt| (attempt.entry.ordinal, attempt.read, attempt.since)).collect();
    assert_eq!(attempts, [(0, 22, Some(21))]);
    assert_eq!(facts.projects[0].entries.len(), 2, "both entries are kept");
}

#[test]
fn t5_an_owner_back_after_leaving_is_unread_until_read_again_whatever_the_other_owner() {
    let run = |other: bool| {
        recorded(if other { "t5-other-read" } else { "t5-other-unread" }, 20, 25, |at| {
            let read = if at < 24 { Some(20) } else { Some(at) };
            Hour { linked: at != 21, projects_read: read, history_read: read, other_read: other || !(21..=22).contains(&at), ..hour(at, RUNNING) }
        })
    };
    let (read, unread) = (run(true), run(false));
    assert_eq!(read.projects, unread.projects, "another owner's stamps decide nothing about this owner's project");
    let hours: Vec<_> = read.projects[0].runs.iter().map(|run| (run.first, run.last)).collect();
    assert_eq!(hours, [(20, 20), (24, 25)], "a pre-departure read never makes the project current after the owner returns");
    assert!(read.observed.iter().all(|&(first, last)| last < 22 || first > 23), "the hours the returned owner is unread are not published: {:?}", read.observed);
}

#[test]
fn t6_statuses_are_shown_as_read_and_a_cancellation_is_never_dated_or_joined() {
    let facts = recorded("t6", 20, 22, |at| hour(at, [CANCELLED, UNKNOWN, RUNNING][(at - 20) as usize]));
    let states: Vec<_> = runs(&facts).into_iter().map(|run| run.2).collect();
    assert_eq!(states, [ProjectState::Cancelled, ProjectState::Unknown, ProjectState::Running]);
    assert!(facts.projects[0].attempts.is_empty());
    let facts = recorded("t6b", 20, 24, |at| Hour { history: if at >= 23 { vec![(0, M, 21)] } else { Vec::new() }, ..hour(at, if at < 22 { CANCELLED } else { COMPLETED }) });
    let states: Vec<_> = runs(&facts).into_iter().map(|run| run.2).collect();
    assert_eq!(states, [ProjectState::Cancelled, ProjectState::Completed]);
    assert_eq!(markers(&facts), [(21, 23, true)]);
}

#[test]
fn t7_an_unstarted_pool_row_stays_hidden() {
    let facts = recorded("t7", 20, 22, |at| hour(at, POOL));
    assert!(facts.projects.is_empty());
    assert_eq!(facts.observed, [(20, 22)], "the list is published, empty");
}

#[test]
fn t8_another_country_holding_the_tag_between_leaves_the_project_unobserved_and_one_completion_once() {
    let facts = recorded("t8", 20, 34, |at| Hour { holder: if (25..30).contains(&at) { None } else { Some(true) }, ..hour(at, RUNNING) });
    assert_eq!(facts.projects.len(), 1);
    let hours: Vec<_> = facts.projects[0].runs.iter().map(|run| (run.first, run.last)).collect();
    assert_eq!(hours, [(20, 24), (30, 34)], "unobserved while another country held the tag");
    let facts = recorded("t8b", 20, 34, |at| Hour { holder: if (25..30).contains(&at) { None } else { Some(true) }, history: vec![(0, M, 20)], ..hour(at, COMPLETED) });
    assert_eq!(markers(&facts), [(20, 20, true)], "the same completion read on both sides is one");
    let facts = recorded("t8c", 20, 34, |at| Hour { holder: Some(at >= 30), history: vec![(0, M, 25)], ..hour(at, COMPLETED) });
    assert_eq!(markers(&facts), [(25, 30, false)], "a marker dated before the country held the project is not its completion");
}

#[test]
fn t9_the_first_state_is_the_first_usable_read_even_in_an_unpublished_hour() {
    let facts = recorded("t9", 20, 31, |at| Hour { state: (at >= 25).then_some(if at < 30 { RUNNING } else { COMPLETED }), other_read: !(25..30).contains(&at), ..hour(at, RUNNING) });
    assert_eq!(facts.projects[0].first, (25, ProjectState::Running), "not already complete when first recorded");
    assert_eq!(runs(&facts), [(30, 31, ProjectState::Completed, Some(0))]);
}

#[test]
fn t10_the_scientist_comes_from_the_program_read_at_its_own_hour() {
    let scientist = 5_u64 << 32 | u64::from(CHARACTER_REFERENCE);
    let facts = recorded("t10", 20, 53, |at| Hour { program: Some((scientist, at == 28)), ..hour(at, RUNNING) });
    let look = |at: i32| {
        let run = facts.projects[0].runs.iter().find(|run| run.first <= at && at <= run.last).unwrap();
        facts.looks[run.look as usize].clone()
    };
    assert_eq!(look(29), Look { definition: 40, person: Person::Character(5), read: Some(28), last: None }, "read 28, never the project read's hour");
    assert!(matches!(look(27).person, Person::Unknown(_)) && look(27).last.is_none(), "not read yet");
    assert!(matches!(look(52).person, Person::Unknown(_)) && look(52).last == Some((5, 28)), "not read now, last observed 5 read 28");
    let facts = recorded("t10c", 20, 32, |at| Hour { program: Some((0, at == 31)), ..hour(at, RUNNING) });
    let run = facts.projects[0].runs.last().unwrap();
    assert_eq!(facts.looks[run.look as usize].person, Person::Unassigned, "observed unassigned at its program read");
    assert_eq!(facts.looks[run.look as usize].read, Some(31));
}

#[test]
fn t11_an_unread_owner_roster_withholds_the_list() {
    let facts = recorded("t11", 38, 42, |at| Hour { links_table: at != 40, ..hour(at, RUNNING) });
    assert_eq!(facts.observed, [(38, 39), (41, 42)]);
}

#[test]
fn t12_a_count_becomes_not_recorded_when_its_history_read_expires() {
    let facts = recorded("t12", 20, 45, |at| Hour { history: vec![(0, 3, 20), (1, 3, 20)], history_read: Some(20), ..hour(at, RUNNING) });
    assert_eq!(runs(&facts), [(20, 20 + STAGGER_PERIOD - 1, ProjectState::Running, Some(2)), (20 + STAGGER_PERIOD, 45, ProjectState::Running, None)]);
    assert_eq!(facts.projects[0].entries.len(), 2, "the entries stay kept");
}

#[test]
fn t13_markers_outside_the_campaign_or_after_their_read_are_not_counted() {
    let facts = recorded("t13", 20, 22, |at| Hour { history: vec![(0, M, 19), (1, M, 30)], ..hour(at, COMPLETED) });
    assert_eq!(markers(&facts), [(19, 20, false), (30, 20, false)]);
}

#[test]
fn t14_an_unknown_state_read_once_warns_once_however_often_it_is_revisited() {
    let facts = recorded("t14", 20, 46, |at| Hour { projects_read: Some(20), ..hour(at, UNKNOWN) });
    assert_eq!(facts.warnings.values().sum::<usize>(), 1);
}
