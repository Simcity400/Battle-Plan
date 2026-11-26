use super::facts::{COMPLETION_MARKER, Facts, Look, Person};
use super::incremental::Follow;
use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay_data::rows::{CountryLinkRow, CountryRow, LexicalTokenRow, ProjectHistoryRow, ProjectProgramRow, ProjectRow, SliceReadRow, TableRows};
use replay_frame::frame::TableView;
use replay_recording::cells::count_reads;

const START: i32 = 1_000;
const RUNNING: (u32, u8, u8, u64) = (0x18, 1, 1, 7 << 32 | PROGRAM_REFERENCE as u64);
const COMPLETED: (u32, u8, u8, u64) = (0xe8, 0, 1, 0);
const CANCELLED: (u32, u8, u8, u64) = (0x1e8, 1, 0, 0);
const UNKNOWN: (u32, u8, u8, u64) = (0x99, 1, 1, 0);
const POOL: (u32, u8, u8, u64) = (0x18, 1, 0, 0);

#[test]
fn custody_holds_only_when_the_owner_stayed_linked_from_the_read_through_the_hour() {
    let links = RunLinks { slots: HashMap::from([(10, vec![(0, 49, Some(5)), (50, 99, Some(5)), (100, 199, Some(6))]), (11, vec![(50, 199, Some(5))]), (12, vec![(0, 199, Some(6))])]) };
    assert_eq!(owner_ids(&links, 5, 20, 150), BTreeMap::from([(10, vec![(20, 99)]), (11, vec![(50, 150)])]));
    let piece = |held_from, opened| Piece { run: 0, first: 100, last: 150, slot: 5, id: 1, held_from, opened };
    assert_eq!(piece(0, 0).reach(), 100 - (STAGGER_PERIOD - 1), "an owner reaches back as far as a read that still supplies the first hour");
    assert_eq!(piece(90, 0).reach(), 90, "never past the start of the holder's continuous possession");
    assert_eq!(piece(0, 95).reach(), 95, "R2-S1: nor past the first hour of the recording run it reads");
    let returned = RunLinks { slots: HashMap::from([(10, vec![(20, 30, Some(5)), (31, 31, Some(6)), (32, 60, Some(5))])]) };
    let custody = Custody::new(&returned, 5, 20, 60);
    assert_eq!(custody.usable(10, Some(25), &[(20, 60)], 28), Some(25));
    assert_eq!(custody.usable(10, Some(25), &[(20, 60)], 40), None, "a read from before the owner left and came back does not supply an hour after");
    assert_eq!(custody.usable(10, Some(25), &[(26, 60)], 28), None, "a read in part supplies nothing");
    assert!(!Custody::new(&returned, 5, 26, 60).held(10, 25, 28), "nor a read from before the country held the tag");
    assert_eq!(uncovered(&[(30, 40), (60, 300)], 20, 150), [(20, 29), (41, 59)]);
}

#[test]
fn a_program_names_the_scientist_only_of_the_project_it_was_read_for() {
    let program = 9_u64 << 32 | u64::from(PROGRAM_REFERENCE);
    let cells = |owner, active| Some((28, Some(Assignment { owner: Some(owner), active, scientist: Some(5 << 32 | u64::from(CHARACTER_REFERENCE)) })));
    assert_eq!(assignment(9001, 1, program, cells(9001, Some(1))).scientist(), Ok(Some(5)));
    assert!(assignment(9001, 1, program, cells(9001, Some(2))).scientist().is_err(), "a program read for another project names no scientist of this one");
    assert!(assignment(9001, 1, program, cells(9002, Some(1))).scientist().is_err(), "nor one read for another owner's");
    assert!(assignment(9001, 1, program, None).scientist().is_err(), "an unread program is not recorded");
    assert_eq!(assignment(9001, 1, 0, None).scientist(), Ok(None), "no program, no scientist");
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
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (first..=last).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
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
    programs_whole: bool,
}

fn hour(at: i32, state: (u32, u8, u8, u64)) -> Hour {
    Hour {
        state: Some(state),
        history: Vec::new(),
        projects_read: Some(at),
        history_read: Some(at),
        other_read: true,
        projects_table: true,
        linked: true,
        links_table: true,
        holder: Some(true),
        program: None,
        programs_whole: true,
    }
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
    let programs = TableRows::ProjectPrograms(spec.program.map(|(scientist, _)| ProjectProgramRow { id: 7, owner: 9001, active_project: Some(1), scientist, ..Default::default() }).into_iter().collect());
    *frame.tables[TableId::ProjectPrograms.index()] = if spec.programs_whole { TableView::Complete(programs) } else { TableView::Truncated { rows: programs, missing: 1 } };
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
    recorded_in(id, synthetic(id, first, last, |at| frame(at, &spec(at))), first, last)
}

fn recorded_in(id: &str, source: Source, first: i32, last: i32) -> Facts {
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
    facts.evidence.values().flat_map(|evidence| &evidence.entries).filter(|kept| kept.entry.completion()).map(|kept| (kept.entry.date, kept.read, kept.counted)).collect()
}

fn attempts(facts: &Facts) -> Vec<(u32, i32, Option<i32>)> {
    facts.evidence.values().flat_map(|evidence| &evidence.attempts).map(|attempt| (attempt.entry.ordinal, attempt.read, attempt.since)).collect()
}

fn run_at(facts: &Facts, at: i32) -> &super::facts::Run {
    facts.projects[0].runs.iter().find(|run| run.first <= at && at <= run.last).unwrap()
}

const M: u32 = COMPLETION_MARKER;

#[test]
fn t1_a_fresh_empty_history_beside_an_older_completed_state_invents_no_replacement() {
    let facts = recorded("t1", 20, 26, |at| Hour { history: if at == 20 { vec![(0, 3, 20), (1, 3, 20), (2, M, 20)] } else { Vec::new() }, projects_read: Some(20), ..hour(at, COMPLETED) });
    assert_eq!(runs(&facts), [(20, 20, ProjectState::Completed, Some(0)), (21, 26, ProjectState::Completed, None)], "a history listing fewer entries than before makes the count not recorded");
    assert_eq!(markers(&facts), [(20, 20, true)], "one completion");
    assert_eq!(facts.projects.len(), 1);
    assert!(attempts(&facts).is_empty(), "missing entries prove no new attempt");
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
        history: match at { 20..=21 => vec![(0, M, 20)], _ => vec![(0, M, 20), (1, M, 22)] },
        projects_table: at != 22,
        ..hour(at, if at == 20 { COMPLETED } else { UNKNOWN })
    });
    assert_eq!(markers(&facts), [(20, 20, true), (22, 22, true)], "each marker once, from the read that first listed it, however often it is read again, though projects were unavailable then");
    assert_eq!(facts.observed, [(20, 21), (23, 26)], "an hour with projects unavailable is not published");
    assert_eq!(attempts(&facts).len(), 1, "a marker followed by a later entry in one read proves another attempt");
}

#[test]
fn t4_a_changed_entry_at_an_ordinal_proves_another_attempt_between_the_two_reads() {
    let facts = recorded("t4", 20, 23, |at| Hour { history: if at < 22 { vec![(0, 41, 20)] } else { vec![(0, 43, 21)] }, ..hour(at, RUNNING) });
    assert_eq!(attempts(&facts), [(0, 22, Some(21))]);
    assert_eq!(facts.evidence.values().map(|evidence| evidence.entries.len()).sum::<usize>(), 2, "both entries are kept");
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
    assert!(attempts(&facts).is_empty());
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
    let look = |at: i32| facts.looks[run_at(&facts, at).look as usize].clone();
    assert_eq!(look(29), Look { definition: 40, person: Person::Character(5), last: None });
    assert_eq!(run_at(&facts, 29).scientist_read, Some(28), "read 28, never the project read's hour");
    assert!(matches!(look(27).person, Person::Unknown(_)) && look(27).last.is_none(), "not read yet");
    assert!(matches!(look(52).person, Person::Unknown(_)) && look(52).last == Some(5), "not read now, last observed 5");
    assert_eq!(run_at(&facts, 52).last_scientist_read, Some(28), "read 28");
    let facts = recorded("t10c", 20, 32, |at| Hour { program: Some((0, at == 31)), ..hour(at, RUNNING) });
    let run = facts.projects[0].runs.last().unwrap();
    assert_eq!(facts.looks[run.look as usize].person, Person::Unassigned, "observed unassigned at its program read");
    assert_eq!(run.scientist_read, Some(31));
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
    assert_eq!(facts.evidence.values().map(|evidence| evidence.entries.len()).sum::<usize>(), 2, "the entries stay kept");
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

#[test]
fn h1_the_facts_keep_every_reading_history_makes() {
    let facts = recorded("h1-unlisted", 20, 26, |at| Hour {
        history: if at == 22 { vec![(0, M, 20), (1, M, 22)] } else { vec![(0, M, 20)] },
        history_read: (at <= 22).then_some(at),
        projects_table: at != 22,
        ..hour(at, COMPLETED)
    });
    assert_eq!(markers(&facts), [(20, 20, true), (22, 22, true)], "P-S1: a marker listed only by a history read taken while projects were unavailable is kept");
    let facts = recorded("h1-gap", 20, 24, |at| Hour { history: match at { 20..=21 => vec![(0, 41, 20)], 22 => Vec::new(), _ => vec![(0, 43, 23)] }, ..hour(at, RUNNING) });
    assert_eq!(attempts(&facts), [(0, 23, Some(21))], "P-S5: an empty read between keeps what each ordinal last held, so the changed entry still proves another attempt");
    let scientist = 5_u64 << 32 | u64::from(CHARACTER_REFERENCE);
    let facts = recorded("h1-reads", 20, 53, |at| Hour { program: Some((scientist, at == 28 || at == 52)), ..hour(at, RUNNING) });
    assert_eq!(run_at(&facts, 29).look, run_at(&facts, 53).look, "P-S12: a daily program read naming the same scientist keeps the presentation, so its icon is not copied again");
    assert_eq!((run_at(&facts, 29).scientist_read, run_at(&facts, 53).scientist_read), (Some(28), Some(52)), "the read hours sit on the status");
}

#[test]
fn h2_a_read_supplies_an_hour_only_while_its_owner_stayed_with_the_country_from_the_read() {
    let facts = recorded("h2-transfer", 20, 32, |at| Hour { linked: at >= 30, history: vec![(0, M, 25)], ..hour(at, COMPLETED) });
    assert_eq!(markers(&facts), [(25, 30, false)], "P-S6: a marker dated before the owner joined the country is not this country's completion");
    let (first, second) = (5_u64 << 32 | u64::from(CHARACTER_REFERENCE), 6_u64 << 32 | u64::from(CHARACTER_REFERENCE));
    let facts = recorded("h2-truncated", 20, 32, |at| Hour { program: Some((if at < 30 { first } else { second }, at == 28 || at == 30)), programs_whole: at != 30, ..hour(at, RUNNING) });
    assert_eq!((facts.looks[run_at(&facts, 31).look as usize].person.clone(), run_at(&facts, 31).scientist_read), (Person::Character(5), Some(28)), "P-S4: the scientist is the one its complete read named, never cells a later read in part left");
}

#[test]
fn h3_a_completion_is_named_only_by_the_completed_row_read_with_it_when_nothing_follows_it() {
    let definitions = |id: &str, state: (u32, u8, u8, u64), history: Vec<(u32, u32, i32)>| {
        let facts = recorded(id, 20, 22, move |at| Hour { history: history.clone(), ..hour(at, state) });
        facts.evidence.values().flat_map(|evidence| &evidence.entries).filter(|kept| kept.entry.completion()).map(|kept| kept.definition).collect::<Vec<_>>()
    };
    assert_eq!(definitions("h3-last", COMPLETED, vec![(0, 41, 18), (1, M, 20)]), [Some(40)], "the completed row read with the marker names it");
    assert_eq!(definitions("h3-restart", COMPLETED, vec![(0, 41, 18), (1, M, 20), (2, 41, 20)]), [None], "R2-S5: a later ordinal in the same hour is the next attempt, whose row it is");
    assert_eq!(definitions("h3-started", RUNNING, vec![(0, 41, 18), (1, M, 20)]), [None], "a running row is already the next attempt");
}

#[test]
fn feed_completions_are_the_ledgers_counted_markers() {
    let completions = |id: &str, spec: fn(i32) -> Hour| {
        let source = synthetic(id, 20, 34, move |at| frame(at, &spec(at)));
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        let (found, _) = counted(&query, 20, 34, &AtomicBool::new(false)).unwrap();
        found.into_iter().map(|counted| (counted.tag, counted.entry.date, counted.read, counted.definition)).collect::<Vec<_>>()
    };
    let listed = completions("feed-listed", |at| Hour { history: if at >= 21 { vec![(0, M, 21)] } else { Vec::new() }, ..hour(at, COMPLETED) });
    assert_eq!(listed, [("GER".to_owned(), 21, 21, Some(40))], "one event per counted marker, named by the row read with it");
    let early = completions("feed-early", |at| Hour { history: if at >= 25 { vec![(0, M, 33)] } else { Vec::new() }, ..hour(at, COMPLETED) });
    assert_eq!(early, [], "a marker dated after the read that first listed it is no completion, however often it is listed again");
    let renamed = completions("feed-renamed", |at| Hour { holder: Some(at >= 30), history: if at >= 30 { vec![(0, M, 25)] } else { Vec::new() }, ..hour(at, COMPLETED) });
    assert_eq!(renamed, [], "P-S7: a completion dated before this tag's holder took it is not credited to it, though the slot never changed");
}

fn adjacent(id: &str, first: i32, last: i32, load: i32, spec: impl Fn(i32) -> Hour) -> Source {
    let frames = |hours: std::ops::RangeInclusive<i32>, branch: u64| {
        hours
            .map(|at| {
                let mut frame = frame(at, &spec(at));
                frame.branch = branch;
                frame
            })
            .collect()
    };
    let segments = [records(1, frames(first..=load - 1, 1)), records(2, frames(load..=last, 2))];
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &segments).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: id.into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

#[test]
fn t15_a_load_between_two_runs_reads_each_run_from_its_own_first_hour() {
    let facts = recorded_in("t15", adjacent("t15", 20, 30, 25, |at| hour(at, RUNNING)), 20, 30);
    assert_eq!(facts.observed, [(20, 30)], "R2-S1: the run after the load is read from its first hour, never asked for hours before it");
    assert_eq!(runs(&facts).iter().map(|run| run.2).collect::<Vec<_>>(), [ProjectState::Running]);
}

#[test]
fn h3b_a_completed_read_before_a_marker_does_not_name_it() {
    let facts = recorded("h3-early", 20, 31, |at| Hour {
        history: if at >= 30 { vec![(0, M, 20), (1, 41, 25), (2, M, 29)] } else { vec![(0, M, 20)] },
        projects_read: Some(at.min(25)),
        ..hour(at, COMPLETED)
    });
    let named: Vec<_> = facts.evidence.values().flat_map(|evidence| &evidence.entries).filter(|kept| kept.entry.completion()).map(|kept| (kept.entry.date, kept.definition)).collect();
    assert_eq!(named, [(20, Some(40)), (29, None)], "P-S9, R3-S4: a completed row read at 25 names no completion dated 29; the marker and its date stay");
}

fn reference_last_read(reads: &[(i32, i32)], hour: i32) -> Option<i32> {
    let at = reads.partition_point(|read| read.0 <= hour);
    at.checked_sub(1).map(|at| reads[at].1.min(hour)).filter(|&read| current(read, hour))
}

fn reference_see_piece(facts: &mut Facts, piece: &Piece, read: &PieceRows, cancelled: &AtomicBool) -> io::Result<()> {
    let mut projects: Vec<(u32, u32, Cursor<'_>)> = Vec::new();
    for (history, &raw) in read.projects.iter().zip(&read.owners) {
        projects.extend(history.keys.iter().map(|(key, steps)| (raw, *key as u32, Cursor::new(steps))));
    }
    let mut entries: Vec<(u32, u32, u32, Cursor<'_>)> = Vec::new();
    for history in &read.histories {
        for (key, steps) in &history.keys {
            entries.push(((key >> 64) as u32, (key >> 32) as u32, *key as u32, Cursor::new(steps)));
        }
    }
    let mut programs: HashMap<u32, Cursor<'_>> = read.programs.keys.iter().map(|(key, steps)| (*key as u32, Cursor::new(steps))).collect();
    let mut stamps: HashMap<u128, Cursor<'_>> = read.reads.keys.iter().map(|(key, steps)| (*key, Cursor::new(steps))).collect();
    let custody = &read.custody;
    for hour in piece.first..=piece.last {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let linked: Vec<u32> = custody.linked(hour).collect();
        let mut usable = |table: TableId, complete: &[(i32, i32)], raw: u32| custody.usable(raw, sample(stamps.get_mut(&read_key(table, raw)).and_then(|cursor| cursor.at(hour))), complete, hour);
        let projects_read: BTreeMap<u32, i32> = linked.iter().filter_map(|&raw| Some((raw, usable(TableId::Projects, &read.projects_complete, raw)?))).collect();
        let histories_read: BTreeMap<u32, i32> = linked.iter().filter_map(|&raw| Some((raw, usable(TableId::ProjectHistory, &read.history_complete, raw)?))).collect();
        let mut listed: BTreeMap<(u32, u32), Vec<Entry>> = BTreeMap::new();
        for (owner, project, ordinal, cursor) in &mut entries {
            if let Some(&stamp) = histories_read.get(owner)
                && let Some(entry) = cursor.at(stamp).and_then(|cells| history_entry(*ordinal, cells))
            {
                listed.entry((*owner, *project)).or_default().push(entry);
            }
        }
        let program_read = reference_last_read(&read.program_reads, hour);
        let mut rows: Vec<(Row, Program, i32)> = Vec::new();
        for (raw, local, cursor) in &mut projects {
            let (raw, local) = (*raw, *local);
            if let Some(&stamp) = projects_read.get(&raw)
                && let Some(row) = cursor.at(stamp).and_then(|cells| project_row(raw, local, cells))
            {
                let cells = program_read.filter(|&at| custody.held(raw, at, hour)).map(|at| (at, programs.get_mut(&((row.program >> 32) as u32)).and_then(|cursor| cursor.at(at)).map(program_cells)));
                let program = assignment(raw, local, row.program, cells);
                rows.push((row, program, stamp));
            }
        }
        let shown: Vec<facts::Read<'_>> = rows.iter().map(|(row, program, stamp)| facts::Read { row, read: *stamp, program: program.clone() }).collect();
        let published = within(&read.roster, hour) && projects_read.len() == linked.len();
        if !published && within(&read.projects_partial, hour) {
            facts::extend(&mut facts.partial, hour, hour);
        }
        facts.hour_span(
            &facts::Hour {
                country: (piece.slot, piece.id),
                at: hour,
                held: &|raw, from, through| custody.held(raw, from, through),
                projects: &projects_read,
                histories: &histories_read,
                listed: &listed,
                reads: &shown,
                published,
            },
            hour,
        );
    }
    Ok(())
}

fn reference_counted(query: &Query<'_>, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<(Vec<Counted>, Vec<HourSpan>)> {
    let recording = query.replay.recording();
    let mut unrecorded: Vec<HourSpan> = Vec::new();
    let mut links: HashMap<u32, RunLinks> = HashMap::new();
    let mut tags: BTreeSet<String> = BTreeSet::new();
    for stretch in query.timeline.within(from, to) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let held = available(query, stretch.run, TableId::ProjectHistory, stretch.first, stretch.last)?;
        unrecorded.extend(uncovered(&held, stretch.first, stretch.last).into_iter().map(|(first, last)| HourSpan { first_hour: first, last_hour: last }));
        let history = dashboard_query::cost::storage(|| lanes::history(recording, stretch.run, TableId::ProjectHistory, &HISTORY_COLUMNS, Keys::All, stretch.first, stretch.last))?;
        let run_links = links_of(query, &mut links, stretch.run)?;
        for (key, steps) in &history.keys {
            for step in steps {
                let Some(entry) = step.cells.as_deref().and_then(|cells| history_entry(*key as u32, cells)) else { continue };
                if !entry.completion() || !(from..=to).contains(&entry.date) {
                    continue;
                }
                let Some(slot) = run_links.slot_at((key >> 64) as u32, step.hour) else { continue };
                let holds = |runs: &[dashboard_query::identity::HolderRun]| {
                    runs.iter().any(|run| run.first <= step.hour && step.hour <= run.last && matches!(run.holder, Holder::Found { slot: held, .. } if held == slot))
                };
                tags.extend(query.holders.tags.iter().filter(|(_, runs)| holds(runs)).map(|(tag, _)| tag.clone()));
            }
        }
    }
    let mut found: BTreeMap<(u32, u32, u32, i32), Counted> = BTreeMap::new();
    for tag in tags {
        let mut facts = Facts::new(query.timeline.first);
        let (pieces, _) = pieces(query, &Country::Tag(tag.clone()), query.timeline.first, to);
        for piece in pieces {
            let read = read_piece(query, links_of(query, &mut links, piece.run)?, &piece)?;
            reference_see_piece(&mut facts, &piece, &read, cancelled)?;
        }
        for (&(slot, _, owner, local), evidence) in &facts.evidence {
            for kept in evidence.entries.iter().filter(|kept| kept.counted && (from..=to).contains(&kept.entry.date)) {
                found
                    .entry((owner, local, kept.entry.ordinal, kept.entry.date))
                    .or_insert_with(|| Counted { tag: tag.clone(), slot, owner, local, entry: kept.entry, read: kept.read, definition: kept.definition });
            }
        }
    }
    Ok((found.into_values().collect(), unrecorded))
}

fn reference_spell_completions(query: &Query<'_>, found: &[Counted]) -> io::Result<HashMap<(i32, u32), String>> {
    let mut frames = at_hour::Frames::new(query)?;
    let mut out = HashMap::new();
    for item in found {
        if let Some(definition) = item.definition {
            let lexical = frames.frame(item.read, &[Read::these(TableId::LexicalTokens, [u128::from(definition)])], "Special project names")?;
            let tokens = spellings(&lexical, [definition]);
            if let Some(text) = tokens.get(&definition) {
                out.insert((item.read, definition), text.clone());
            }
        }
    }
    Ok(out)
}

#[test]
fn f1_slice_reads_unavailable_at_probe_hours_still_finds_valid_stamp_between() {
    let source = synthetic("f1-unavailable-probes", 20, 50, |at| {
        let mut frame = frame(at, &Hour { history: vec![(0, M, 20)], projects_read: Some(20), history_read: Some(20), ..hour(at, COMPLETED) });
        if at != 21 {
            *frame.tables[TableId::SliceReads.index()] = TableView::Unavailable { reason: "Synthetic slice-reads refusal".into() };
        }
        frame
    });
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let cancelled = AtomicBool::new(false);
    let actual = counted(&query, 20, 50, &cancelled).unwrap();
    let expected = reference_counted(&query, 20, 50, &cancelled).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.0.len(), 1);
    assert_eq!((actual.0[0].entry.date, actual.0[0].read, actual.0[0].definition), (20, 20, Some(40)));
}

#[test]
fn f2_replacement_marker_keeps_earlier_marker_read_from_delayed_stamp() {
    let source = synthetic("f2-replacement-marker", 20, 35, |at| {
        let history = match at {
            24..=27 => vec![(0, M, 24)],
            28..=35 => vec![(0, M, 28)],
            _ => Vec::new(),
        };
        let stamp = match at {
            28 => Some(27),
            29..=35 => Some(28),
            _ => None,
        };
        frame(at, &Hour { history, projects_read: stamp, history_read: stamp, ..hour(at, if at >= 24 { COMPLETED } else { RUNNING }) })
    });
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let cancelled = AtomicBool::new(false);
    let actual = counted(&query, 20, 35, &cancelled).unwrap();
    let expected = reference_counted(&query, 20, 35, &cancelled).unwrap();
    assert_eq!(actual, expected);
    let summary: Vec<_> = actual.0.iter().map(|c| (c.entry.date, c.read, c.definition)).collect();
    assert_eq!(summary, [(24, 27, Some(40)), (28, 28, Some(40))]);
}

#[test]
fn f3_lexical_token_appearing_later_never_names_an_earlier_completion() {
    let source = synthetic("f3-late-lexical-token", 20, 34, |at| {
        let history = match at {
            24..=29 => vec![(0, M, 24)],
            30..=34 => vec![(0, M, 24), (1, M, 30)],
            _ => Vec::new(),
        };
        let mut frame = frame(at, &Hour { history, ..hour(at, COMPLETED) });
        let tokens = if at >= 28 { vec![LexicalTokenRow { token: 40, literal: b"sp_late_token".to_vec() }] } else { Vec::new() };
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(tokens));
        frame
    });
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let cancelled = AtomicBool::new(false);
    let (found, _) = counted(&query, 20, 34, &cancelled).unwrap();
    assert_eq!(found.len(), 2);
    let actual = spell_completions(&query, &found, &cancelled).unwrap();
    let expected = reference_spell_completions(&query, &found).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.get(&(24, 40)), None);
    assert_eq!(actual.get(&(30, 40)).map(String::as_str), Some("sp_late_token"));
}

#[test]
fn f4_damaged_lexical_block_keeps_dated_completions_and_other_runs() {
    let make = |hours: std::ops::RangeInclusive<i32>, branch: u64| {
        hours
            .map(|at| {
                let history = if at < 24 {
                    Vec::new()
                } else if at < 34 {
                    vec![(0, M, 24)]
                } else {
                    vec![(0, M, 24), (1, M, 34)]
                };
                let mut frame = frame(at, &Hour { history, ..hour(at, COMPLETED) });
                frame.branch = branch;
                if let TableView::Complete(TableRows::Projects(rows)) = &mut *frame.tables[TableId::Projects.index()]
                    && at >= 30
                {
                    rows[0].definition_id = 41;
                }
                let tokens = if at < 30 {
                    vec![LexicalTokenRow { token: 40, literal: b"sp_run_zero".to_vec() }]
                } else {
                    vec![
                        LexicalTokenRow { token: 40, literal: b"sp_run_zero".to_vec() },
                        LexicalTokenRow { token: 41, literal: b"sp_run_one".to_vec() },
                    ]
                };
                *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(tokens));
                frame
            })
            .collect()
    };
    let segments = [records(1, make(20..=29, 1)), records(2, make(30..=39, 2))];
    let clean = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &segments).unwrap();
    let mut bytes = clean.recording().memory().unwrap().to_vec();
    let entry = clean.recording().table_entry(0, 0, TableId::LexicalTokens.index()).unwrap();
    for (_, chunks) in &entry.unit.first.as_ref().unwrap().streams {
        for chunk in chunks {
            let f = &chunk.frame;
            bytes[(f.offset + u64::from(f.stored) / 2) as usize] ^= 0x40;
        }
    }
    let damaged = replay::campaign::Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "f4-damaged-lexical".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(damaged), &AtomicBool::new(false)).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let cancelled = AtomicBool::new(false);
    let (found, _) = counted(&query, 20, 39, &cancelled).unwrap();
    assert_eq!(found.len(), 2);
    let actual = spell_completions(&query, &found, &cancelled).unwrap();
    let expected = reference_spell_completions(&query, &found).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.get(&(24, 40)), None);
    assert_eq!(actual.get(&(34, 41)).map(String::as_str), Some("sp_run_one"));
}

#[test]
fn g1_decreasing_history_stamp_preserves_main_cursor_and_completion_name() {
    let source = synthetic("g1-decreasing-history-stamp", 20, 32, |at| {
        let history = match at {
            24..=26 | 29..=32 => vec![(0, M, 24)],
            _ => Vec::new(),
        };
        let history_read = match at {
            27 => Some(27),
            28 => Some(26),
            29..=32 => Some(29),
            _ => None,
        };
        frame(at, &Hour { history, projects_read: Some(at), history_read, ..hour(at, if at >= 24 { COMPLETED } else { RUNNING }) })
    });
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let cancelled = AtomicBool::new(false);
    let actual = counted(&query, 20, 32, &cancelled).unwrap();
    let expected = reference_counted(&query, 20, 32, &cancelled).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(actual.0.len(), 1);
    assert_eq!((actual.0[0].entry.date, actual.0[0].read, actual.0[0].definition), (24, 29, Some(40)));
}

fn multi_actor_source(id: &str, actors: &[(u32, u32, &[u8], i32, u32)], first: i32, last: i32, overlap_owner: Option<(u32, u32)>) -> Source {
    synthetic(id, first, last, |at| {
        let mut frame = source(1, at, None);
        if let TableView::Complete(TableRows::Countries(countries)) = &mut *frame.tables[TableId::Countries.index()] {
            let template = countries[0].clone();
            countries.clear();
            for &(owner, slot, tag, ..) in actors {
                countries.push(CountryRow { id: owner, canonical_slot: slot, tag: tag.to_vec(), ..template.clone() });
            }
            countries.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(links)) = &mut *frame.tables[TableId::CountryLinks.index()] {
            links.clear();
            for &(owner, slot, ..) in actors {
                links.push(CountryLinkRow { id: owner, canonical_slot: slot });
            }
            if let Some((extra_owner, slot)) = overlap_owner {
                links.push(CountryLinkRow { id: extra_owner, canonical_slot: slot });
            }
            links.sort_by_key(|row| row.id);
        }
        let mut projects = Vec::new();
        let mut histories = Vec::new();
        let mut stamps = Vec::new();
        let mut tokens = Vec::new();
        for &(owner, _, _, done, def) in actors {
            let mut row = ProjectRow { owner, local_id: 1, definition_id: def, started: 1, ..Default::default() };
            let (offset, not_completed) = if at >= done { (0xe8, 0) } else { (0x18, 1) };
            row.state.current_offset = offset;
            row.state.not_completed = not_completed;
            projects.push(row);
            let mut unfinished = ProjectRow { owner, local_id: 2, definition_id: def + 10, started: 1, ..Default::default() };
            unfinished.state.current_offset = 0x18;
            unfinished.state.not_completed = 1;
            projects.push(unfinished);
            if at >= done + 2 {
                histories.push(ProjectHistoryRow { owner, project: 1, ordinal: 0, prototype_reward_handle: M, option_handle: M, date: done, ..Default::default() });
            }
            if owner == actors[0].0 && at >= done + 60 {
                histories.push(ProjectHistoryRow { owner, project: 1, ordinal: 1, prototype_reward_handle: M, option_handle: M, date: done + 58, ..Default::default() });
            }
            histories.push(ProjectHistoryRow { owner, project: 2, ordinal: 0, prototype_reward_handle: 3, option_handle: 4, date: first, ..Default::default() });
            let sample_hour = first + ((at - first) / STAGGER_PERIOD) * STAGGER_PERIOD;
            stamps.push(SliceReadRow { table: TableId::Projects.index() as u32, owner, sample_hour });
            stamps.push(SliceReadRow { table: TableId::ProjectHistory.index() as u32, owner, sample_hour });
            tokens.push(LexicalTokenRow { token: def, literal: format!("sp_def_{def}").into_bytes() });
        }
        if let Some((extra_owner, _)) = overlap_owner {
            let mut row = ProjectRow { owner: extra_owner, local_id: 1, definition_id: 49, started: 1, ..Default::default() };
            row.state.current_offset = 0xe8;
            row.state.not_completed = 0;
            projects.push(row);
            if at >= first + 40 {
                histories.push(ProjectHistoryRow { owner: extra_owner, project: 1, ordinal: 0, prototype_reward_handle: M, option_handle: M, date: first + 38, ..Default::default() });
            }
            let sample_hour = first + ((at - first) / STAGGER_PERIOD) * STAGGER_PERIOD;
            stamps.push(SliceReadRow { table: TableId::Projects.index() as u32, owner: extra_owner, sample_hour });
            stamps.push(SliceReadRow { table: TableId::ProjectHistory.index() as u32, owner: extra_owner, sample_hour });
            tokens.push(LexicalTokenRow { token: 49, literal: b"sp_def_49".to_vec() });
        }
        projects.sort_by_key(|row| (row.owner, row.local_id));
        histories.sort_by_key(|row| (row.owner, row.project, row.ordinal));
        stamps.sort_by_key(|row| (row.table, row.owner));
        tokens.sort_by_key(|row| row.token);
        tokens.dedup_by_key(|row| row.token);
        *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(projects));
        *frame.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::ProjectHistory(histories));
        *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(stamps));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(tokens));
        frame
    })
}

#[test]
fn f10_reader_boundary_counts_stay_bounded_across_countries_mid_run_and_overlaps() {
    let three: [(u32, u32, &[u8], i32, u32); 3] = [(9001, 1, b"GER", 24, 40), (9002, 2, b"FRA", 52, 41), (9003, 3, b"ENG", 76, 42)];
    let six: [(u32, u32, &[u8], i32, u32); 6] = [
        (9001, 1, b"GER", 24, 40),
        (9002, 2, b"FRA", 40, 41),
        (9003, 3, b"ENG", 56, 42),
        (9004, 4, b"ITA", 72, 43),
        (9005, 5, b"SOV", 88, 44),
        (9006, 6, b"USA", 104, 45),
    ];
    let (first, last) = (20, 150);
    let cancelled = AtomicBool::new(false);

    let src3 = multi_actor_source("f10-three-overlap", &three, first, last, Some((9010, 1)));
    let q3 = dashboard_base::queries::query(&src3, 1).unwrap();
    let ((full3, full3_unrec), full3_reads) = count_reads(|| counted(&q3, first, last, &cancelled).unwrap());
    let ((ref_full3, ref_full3_unrec), ref_full3_reads) = count_reads(|| reference_counted(&q3, first, last, &cancelled).unwrap());
    assert!(full3_unrec.is_empty());
    assert_eq!((&full3, &full3_unrec), (&ref_full3, &ref_full3_unrec));
    assert_eq!(full3.len(), 5);
    assert_eq!(full3_reads[&TableId::ProjectHistory].stretches, 1);
    assert_eq!(full3_reads[&TableId::ProjectHistory].backward_seeks, 0);
    assert_eq!(full3_reads[&TableId::SliceReads].opened, 1);
    assert_eq!(full3_reads[&TableId::SliceReads].stretches, 0);
    assert_eq!(full3_reads[&TableId::SliceReads].backward_seeks, 0);
    assert_eq!(full3_reads[&TableId::Projects].opened, 1);
    assert_eq!(full3_reads[&TableId::Projects].stretches, 0);
    assert_eq!(full3_reads[&TableId::Projects].backward_seeks, 0);
    assert!(!full3_reads.contains_key(&TableId::ProjectPrograms));
    assert!(full3_reads[&TableId::ProjectHistory].stretches < ref_full3_reads[&TableId::ProjectHistory].stretches);
    assert!(full3_reads[&TableId::SliceReads].stretches < ref_full3_reads[&TableId::SliceReads].stretches);

    let ((mid3, mid3_unrec), mid3_reads) = count_reads(|| counted(&q3, 50, last, &cancelled).unwrap());
    let (ref_mid3, ref_mid3_unrec) = reference_counted(&q3, 50, last, &cancelled).unwrap();
    assert_eq!((&mid3, &mid3_unrec), (&ref_mid3, &ref_mid3_unrec));
    assert_eq!(mid3_reads[&TableId::ProjectHistory].stretches, 2);
    assert_eq!(mid3_reads[&TableId::SliceReads].opened, 1);
    assert_eq!(mid3_reads[&TableId::SliceReads].stretches, 0);
    assert_eq!(mid3_reads[&TableId::SliceReads].backward_seeks, 0);
    assert_eq!(mid3_reads[&TableId::Projects].opened, 1);
    assert_eq!(mid3_reads[&TableId::Projects].stretches, 0);
    assert_eq!(mid3_reads[&TableId::Projects].backward_seeks, 0);
    assert!(!mid3_reads.contains_key(&TableId::ProjectPrograms));

    let (spelled3, spell3_reads) = count_reads(|| spell_completions(&q3, &full3, &cancelled).unwrap());
    assert_eq!(spelled3, reference_spell_completions(&q3, &full3).unwrap());
    assert_eq!(spell3_reads[&TableId::LexicalTokens].opened, 1);
    assert_eq!(spell3_reads[&TableId::LexicalTokens].stretches, 0);
    assert_eq!(spell3_reads[&TableId::LexicalTokens].backward_seeks, 0);

    let src6 = multi_actor_source("f10-six", &six, first, last, None);
    let q6 = dashboard_base::queries::query(&src6, 1).unwrap();
    let ((full6, _), full6_reads) = count_reads(|| counted(&q6, first, last, &cancelled).unwrap());
    let ((ref_full6, _), ref_full6_reads) = count_reads(|| reference_counted(&q6, first, last, &cancelled).unwrap());
    assert_eq!(full6, ref_full6);
    assert_eq!(full6.len(), 7);
    assert_eq!(full6_reads[&TableId::ProjectHistory].opened, full3_reads[&TableId::ProjectHistory].opened);
    assert_eq!(full6_reads[&TableId::SliceReads].opened, full3_reads[&TableId::SliceReads].opened);
    assert_eq!(full6_reads[&TableId::Projects].opened, full3_reads[&TableId::Projects].opened);
    assert_eq!(full6_reads[&TableId::ProjectHistory].stretches, full3_reads[&TableId::ProjectHistory].stretches);
    assert_eq!(full6_reads[&TableId::SliceReads].stretches, full3_reads[&TableId::SliceReads].stretches);
    assert!(ref_full6_reads[&TableId::SliceReads].stretches > ref_full3_reads[&TableId::SliceReads].stretches);

    let gap_actors: [(u32, u32, &[u8], i32, u32); 2] = [(9001, 1, b"GER", 24, 40), (9002, 2, b"FRA", 450, 41)];
    let gap_src = multi_actor_source("f10-long-gap", &gap_actors, 20, 500, None);
    let q_gap = dashboard_base::queries::query(&gap_src, 1).unwrap();
    let ((gap_found, _), gap_reads) = count_reads(|| counted(&q_gap, 20, 500, &cancelled).unwrap());
    let (ref_gap_found, _) = reference_counted(&q_gap, 20, 500, &cancelled).unwrap();
    assert_eq!(gap_found, ref_gap_found);
    assert_eq!(gap_found.len(), 3);
    assert_eq!(gap_reads[&TableId::SliceReads].opened, 1);
    assert_eq!(gap_reads[&TableId::SliceReads].stretches, 0);
    assert_eq!(gap_reads[&TableId::SliceReads].backward_seeks, 0);
    assert_eq!(gap_reads[&TableId::SliceReads].visited_ordinals, 29);
}

#[test]
fn differential_batched_completions_matches_reference_across_300_seeds() {
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            if hi <= lo { return lo; }
            lo + (self.next() % ((hi - lo + 1) as u64)) as i32
        }
        fn chance(&mut self, num: u64, den: u64) -> bool {
            self.next() % den < num
        }
    }
    let tags_pool: [&[u8]; 4] = [b"GER", b"FRA", b"ENG", b"ITA"];
    let cancelled = AtomicBool::new(false);
    let mut total_completions = 0usize;
    let mut total_spelled = 0usize;
    for seed in 0..300u64 {
        let mut rng = Rng((seed + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let first = 20;
        let len = rng.range(24, 42);
        let last = first + len;
        let two_runs = rng.chance(1, 3);
        let split = first + len / 2;
        let actor_count = rng.range(2, 4) as usize;
        let mut actor_owners: Vec<(u32, u32, &[u8])> = Vec::with_capacity(actor_count);
        for i in 0..actor_count {
            actor_owners.push((9001 + i as u32, (i + 1) as u32, tags_pool[i]));
        }
        let swap_hour = if rng.chance(1, 4) { Some(rng.range(first + 4, last - 4)) } else { None };
        let unlink_owner = if rng.chance(1, 4) { Some((actor_owners[0].0, rng.range(first + 2, last - 4))) } else { None };

        #[derive(Clone)]
        struct MarkerPlan {
            project: u32,
            ordinal: u32,
            appear: i32,
            disappear: Option<i32>,
            reappear: Option<i32>,
            date: i32,
            replaced_at: Option<(i32, i32)>,
            is_completion: bool,
        }
        let mut plans: Vec<(u32, Vec<MarkerPlan>, u32, i32)> = Vec::new();
        for &(owner, _, _) in &actor_owners {
            let def = 40 + (rng.next() % 5) as u32;
            let complete_at = rng.range(first, last);
            let mut markers = Vec::new();
            let count = rng.range(1, 3);
            for idx in 0..count {
                let project = if idx == 0 { 1 } else { rng.range(1, 2) as u32 };
                let ordinal = if idx == 0 { 0 } else { rng.range(0, 2) as u32 };
                let appear = rng.range(first, last - 2);
                let disappear = if rng.chance(1, 4) { Some(rng.range(appear + 1, last)) } else { None };
                let reappear = disappear.and_then(|d| if d + 1 <= last && rng.chance(1, 2) { Some(rng.range(d + 1, last)) } else { None });
                let date = match rng.next() % 5 {
                    0 => first - 2,
                    1 => (appear + rng.range(1, 5)).min(last + 3),
                    _ => rng.range(first, appear),
                };
                let replaced_at = if rng.chance(1, 3) && appear + 2 <= last {
                    let at = rng.range(appear + 1, last);
                    Some((at, rng.range(first, at)))
                } else {
                    None
                };
                let is_completion = idx == 0 || rng.chance(3, 4);
                markers.push(MarkerPlan { project, ordinal, appear, disappear, reappear, date, replaced_at, is_completion });
            }
            plans.push((owner, markers, def, complete_at));
        }

        let lex_appear: [i32; 5] = std::array::from_fn(|_| if rng.chance(1, 3) { rng.range(first + 3, last + 2) } else { first });
        let mut slice_unavail: BTreeSet<i32> = BTreeSet::new();
        let mut slice_trunc: BTreeSet<i32> = BTreeSet::new();
        let mut proj_unavail: BTreeSet<i32> = BTreeSet::new();
        let mut hist_unavail: BTreeSet<i32> = BTreeSet::new();
        let mut lex_unavail: BTreeSet<i32> = BTreeSet::new();
        for at in first..=last {
            if rng.chance(1, 7) { slice_unavail.insert(at); }
            else if rng.chance(1, 9) { slice_trunc.insert(at); }
            if rng.chance(1, 10) { proj_unavail.insert(at); }
            if rng.chance(1, 12) { hist_unavail.insert(at); }
            if rng.chance(1, 10) { lex_unavail.insert(at); }
        }
        let mut stamps_h: HashMap<(u32, i32), Option<i32>> = HashMap::new();
        let mut stamps_p: HashMap<(u32, i32), Option<i32>> = HashMap::new();
        for &(owner, ..) in &actor_owners {
            let mut cur_h: Option<i32> = None;
            let mut cur_p: Option<i32> = None;
            for at in first..=last {
                if cur_h.is_none() || rng.chance(1, 3) {
                    let delay = rng.range(0, 4);
                    cur_h = if rng.chance(1, 8) { None } else { Some((at - delay).max(first)) };
                }
                if cur_p.is_none() || rng.chance(1, 3) {
                    let delay = rng.range(0, 4);
                    cur_p = if rng.chance(1, 8) { None } else { Some((at - delay).max(first)) };
                }
                stamps_h.insert((owner, at), cur_h);
                stamps_p.insert((owner, at), cur_p);
            }
        }
        let drop_trunc_owner = actor_owners[0].0;

        let build_frame = |at: i32, branch: u64| {
            let mut frame = source(branch, at, None);
            if let TableView::Complete(TableRows::Countries(countries)) = &mut *frame.tables[TableId::Countries.index()] {
                let template = countries[0].clone();
                countries.clear();
                for &(owner, slot, tag) in &actor_owners {
                    let shown_tag = if swap_hour.is_some_and(|sh| at >= sh) && slot == 1 { b"ALT".as_slice() } else { tag };
                    countries.push(CountryRow { id: owner, canonical_slot: slot, tag: shown_tag.to_vec(), ..template.clone() });
                }
                countries.sort_by_key(|row| row.id);
            }
            if let TableView::Complete(TableRows::CountryLinks(links)) = &mut *frame.tables[TableId::CountryLinks.index()] {
                links.clear();
                for &(owner, slot, _) in &actor_owners {
                    if unlink_owner.is_some_and(|(u, h)| owner == u && (h..=h + 2).contains(&at)) {
                        continue;
                    }
                    links.push(CountryLinkRow { id: owner, canonical_slot: slot });
                }
                links.sort_by_key(|row| row.id);
            }
            if proj_unavail.contains(&at) {
                *frame.tables[TableId::Projects.index()] = TableView::Unavailable { reason: "Synthetic projects unavailable".into() };
            } else {
                let mut rows = Vec::new();
                for &(owner, _, def, complete_at) in &plans {
                    let mut p1 = ProjectRow { owner, local_id: 1, definition_id: def, started: 1, ..Default::default() };
                    if at >= complete_at {
                        p1.state.current_offset = 0xe8;
                        p1.state.not_completed = 0;
                    } else {
                        p1.state.current_offset = 0x18;
                        p1.state.not_completed = 1;
                    }
                    rows.push(p1);
                    let mut p2 = ProjectRow { owner, local_id: 2, definition_id: def, started: 1, ..Default::default() };
                    p2.state.current_offset = 0xe8;
                    p2.state.not_completed = 0;
                    rows.push(p2);
                }
                rows.sort_by_key(|row| (row.owner, row.local_id));
                *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(rows));
            }
            if hist_unavail.contains(&at) {
                *frame.tables[TableId::ProjectHistory.index()] = TableView::Unavailable { reason: "Synthetic history unavailable".into() };
            } else {
                let mut by_key: BTreeMap<(u32, u32, u32), ProjectHistoryRow> = BTreeMap::new();
                for (owner, markers, _, _) in &plans {
                    for m in markers {
                        let visible = if at < m.appear {
                            false
                        } else if let Some(d) = m.disappear {
                            at < d || m.reappear.is_some_and(|r| at >= r)
                        } else {
                            true
                        };
                        if !visible {
                            continue;
                        }
                        let (date, is_comp) = match m.replaced_at {
                            Some((rep_at, rep_date)) if at >= rep_at => (rep_date, true),
                            _ => (m.date, m.is_completion),
                        };
                        let (reward, option) = if is_comp { (M, M) } else { (3, 4) };
                        by_key.insert(
                            (*owner, m.project, m.ordinal),
                            ProjectHistoryRow { owner: *owner, project: m.project, ordinal: m.ordinal, prototype_reward_handle: reward, option_handle: option, date, ..Default::default() },
                        );
                    }
                }
                *frame.tables[TableId::ProjectHistory.index()] = TableView::Complete(TableRows::ProjectHistory(by_key.into_values().collect()));
            }
            if slice_unavail.contains(&at) {
                *frame.tables[TableId::SliceReads.index()] = TableView::Unavailable { reason: "Synthetic slice reads unavailable".into() };
            } else {
                let mut rows = Vec::new();
                for &(owner, ..) in &actor_owners {
                    if slice_trunc.contains(&at) && owner == drop_trunc_owner {
                        continue;
                    }
                    if let Some(sh) = stamps_p.get(&(owner, at)).copied().flatten() {
                        rows.push(SliceReadRow { table: TableId::Projects.index() as u32, owner, sample_hour: sh });
                    }
                    if let Some(sh) = stamps_h.get(&(owner, at)).copied().flatten() {
                        rows.push(SliceReadRow { table: TableId::ProjectHistory.index() as u32, owner, sample_hour: sh });
                    }
                }
                rows.sort_by_key(|row| (row.table, row.owner));
                *frame.tables[TableId::SliceReads.index()] = if slice_trunc.contains(&at) {
                    TableView::Truncated { rows: TableRows::SliceReads(rows), missing: 1 }
                } else {
                    TableView::Complete(TableRows::SliceReads(rows))
                };
            }
            if lex_unavail.contains(&at) {
                *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable { reason: "Synthetic lexical unavailable".into() };
            } else {
                let mut rows = Vec::new();
                for (i, &start_at) in lex_appear.iter().enumerate() {
                    if at >= start_at {
                        let tok = 40 + i as u32;
                        rows.push(LexicalTokenRow { token: tok, literal: format!("sp_tok_{tok}").into_bytes() });
                    }
                }
                *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(rows));
            }
            frame
        };

        let segments = if two_runs {
            vec![
                records(1, (first..=split - 1).map(|at| build_frame(at, 1)).collect()),
                records(2, (split..=last).map(|at| build_frame(at, 2)).collect()),
            ]
        } else {
            vec![records(1, (first..=last).map(|at| build_frame(at, 1)).collect())]
        };
        let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &segments).unwrap().into_timelines(&Default::default()).unwrap();
        let src = Source::from_replay(dashboard_base::SourceKey { source_id: format!("diff-{seed}"), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let win_from = if rng.chance(1, 2) { first } else { rng.range(first + 1, last - 2) };
        let win_to = if rng.chance(1, 2) { last } else { rng.range(win_from, last) };
        let actual = counted(&query, win_from, win_to, &cancelled).unwrap();
        let expected = reference_counted(&query, win_from, win_to, &cancelled).unwrap();
        assert_eq!(actual, expected, "seed {seed} window {win_from}..={win_to} two_runs={two_runs}");
        let actual_spelled = spell_completions(&query, &actual.0, &cancelled).unwrap();
        let expected_spelled = reference_spell_completions(&query, &expected.0).unwrap();
        assert_eq!(actual_spelled, expected_spelled, "seed {seed} spellings window {win_from}..={win_to}");
        total_completions += actual.0.len();
        total_spelled += actual_spelled.len();
    }
    assert!(total_completions > 100, "expected differential suite to exercise completions, got {total_completions}");
    assert!(total_spelled > 50, "expected differential suite to exercise spellings, got {total_spelled}");
}

#[test]
fn sparse_stretches_skip_re_reading_already_seen_completions() {
    let segments: Vec<_> = (0..25u32)
        .map(|i| {
            let at = 20 + i as i32 * 10;
            records(
                i as u64 + 1,
                vec![frame(at, &Hour { history: vec![(0, M, 20)], projects_read: Some(at), history_read: Some(at), ..hour(at, COMPLETED) })],
            )
        })
        .collect();
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &segments)
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
    let src = Source::from_replay(
        dashboard_base::SourceKey { source_id: "sparse-completions".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let query = dashboard_base::queries::query(&src, 1).unwrap();
    let cancelled = AtomicBool::new(false);
    let ((actual, _), actual_reads) = count_reads(|| counted_with(Some(&src), &query, 20, 260, &cancelled).unwrap());
    let ((expected, _), ref_reads) = count_reads(|| reference_counted(&query, 20, 260, &cancelled).unwrap());
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 1);
    assert_eq!(actual_reads[&TableId::SliceReads].opened, 1, "SliceReads should only be opened for the stretch introducing the completion: {actual_reads:?}");
    assert!(actual_reads[&TableId::Projects].opened <= 1, "Projects should only be opened once across 25 stretches: {actual_reads:?}");
    assert!(ref_reads[&TableId::SliceReads].opened >= 25, "reference path reads SliceReads on every stretch: {ref_reads:?}");
}

#[test]
fn the_card_at_an_hour_orders_running_by_progress_and_dates_a_completion_only_by_a_marker_of_that_attempt() {
    let info = |name: &str| ProjectPresentation { name: name.into(), specialization: None, specialization_name: None, icon: None, icon_missing: None, scientist: 0, last_scientist: None };
    let status = |first_hour: i32, last_hour: i32, presentation: u32, state: ProjectState, progress: i32| ProjectStatus { first_hour, last_hour, read_hour: first_hour, presentation, progress, state, iterations: None, scientist_read: None, last_scientist_read: None };
    let completion = |date: i32, read_hour: i32| ProjectEntry { ordinal: 0, date, completion: true, read_hour, counted: true, presentation: None };
    let project = |id: &str, first_state: ProjectState, statuses: Vec<ProjectStatus>, entries: Vec<ProjectEntry>| ProjectRecord { id: id.into(), owner: 1, local: 0, first_read: 10, first_state, statuses, entries, attempts: Vec::new() };
    let presentations = (0..8).map(|n| info(&format!("Invented project {n}"))).collect::<Vec<_>>();
    use ProjectState::{Completed, Running, Unknown};
    let projects = vec![
        project("a", Running, vec![status(10, 19, 0, Running, 80), status(20, 50, 0, Completed, 100)], vec![completion(20, 20)]),
        project("b", Running, vec![status(10, 50, 1, Running, 24)], Vec::new()),
        project("c", Running, vec![status(10, 29, 2, Running, 90), status(30, 50, 2, Completed, 100)], vec![completion(30, 30)]),
        project("d", Running, vec![status(10, 50, 3, Running, 60)], Vec::new()),
        project("e", Running, vec![status(10, 24, 4, Running, 90), status(25, 50, 4, Completed, 100)], vec![completion(25, 45)]),
        project("f", Completed, vec![status(10, 50, 5, Completed, 100)], vec![completion(10, 10)]),
        project("g", Completed, vec![status(10, 19, 6, Completed, 100), status(20, 29, 6, Running, 50), status(30, 50, 6, Completed, 100)], vec![completion(12, 12)]),
        project("u", Running, vec![status(10, 50, 7, Unknown, 40)], Vec::new()),
    ];
    let available = vec![HourSpan { first_hour: 10, last_hour: 50 }];
    let shown = shown_at(&projects, &presentations, &available, 40);
    assert_eq!(shown.running, [3, 1], "more progress first, and an unknown state is not in progress");
    assert_eq!(shown.unknown, [7]);
    assert_eq!(shown.completed[..2], [ShownCompleted { project: 2, date: Some(30) }, ShownCompleted { project: 0, date: Some(20) }]);
    assert_eq!(shown.undated, 3, "a marker read after the hour, a project first seen complete and an earlier attempt's marker date nothing");
    assert!(shown.completed[2..].iter().all(|item| item.date.is_none()));
    assert_eq!(shown_at(&projects, &presentations, &available, 60), ProjectsShown { through: Some(60), ..ProjectsShown::default() }, "an hour not recorded shows nothing");
    let split = vec![project("s", Running, vec![status(10, 19, 0, Running, 80), status(20, 29, 0, Completed, 100), status(30, 50, 1, Completed, 100)], vec![completion(20, 20)])];
    assert_eq!(shown_at(&split, &presentations, &available, 40).completed, [ShownCompleted { project: 0, date: Some(20) }], "a presentation change inside the completed status keeps the attempt's completion date");
    let gapped = vec![project("g", Running, vec![status(10, 15, 0, Running, 80), status(20, 50, 0, Completed, 100)], vec![completion(18, 20)])];
    assert_eq!(shown_at(&gapped, &presentations, &available, 40).completed, [ShownCompleted { project: 0, date: None }], "a marker across an unread gap does not link the attempt to its completion");
}

#[test]
fn the_current_view_carries_only_the_status_at_the_hour_and_the_one_before_it_whatever_the_history() {
    use super::{facts::Run, shown_runs};
    let run = |first: i32, last: i32, state: ProjectState| Run { first, last, read: first, look: 0, progress: 0, state, iterations: None, scientist_read: None, last_scientist_read: None };
    let mut history: Vec<Run> = (0..1_000).map(|day| run(day * 24, day * 24 + 23, ProjectState::Running)).collect();
    history.push(run(24_000, 24_100, ProjectState::Completed));
    assert_eq!(shown_runs(&history, 24_050), Some(999..=1000), "a completed project keeps the status that dates its attempt and nothing older");
    assert_eq!(shown_runs(&history, 12), Some(0..=0));
    assert_eq!(shown_runs(&history, 30_000), None, "not held at the hour is not shown");
    let cancelled = [run(0, 9, ProjectState::Running), run(10, 20, ProjectState::Cancelled)];
    assert_eq!(shown_runs(&cancelled, 15), None, "a cancelled project leaves Current");
    let relooked = [run(0, 9, ProjectState::Running), run(10, 19, ProjectState::Completed), run(20, 40, ProjectState::Completed)];
    assert_eq!(shown_runs(&relooked, 30), Some(0..=2), "completed statuses that follow each other keep the attempt before them");
}
