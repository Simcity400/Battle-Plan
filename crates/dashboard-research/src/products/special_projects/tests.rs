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
fn exact_project_reads_advance_history_shared_with_the_load_path() {
    use dashboard_base::test_fixtures::visual_mods::Machine;

    let (first, previous, requested) = (100, 850, 1020);
    let machine = Machine::new("project_exact_history");
    machine.write("common/defines/fixture.lua", b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
    machine.write("common/special_projects/projects/fixture.txt", b"fixture_project={}");
    let frames: Vec<_> = (first..=requested).map(|at| {
        let mut frame = frame(at, &hour(at, RUNNING));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 40, literal: b"fixture_project".to_vec() }]));
        frame
    }).collect();
    let owners = frames[0].complete::<tables::CountryLinks>().unwrap().len() as u64;
    let (source, options) = machine.open_frames(frames);
    let load = LoadContext { content: Some(options) };
    let cancelled = AtomicBool::new(false);
    let load_at = |country, hour| {
        crate::registrations::SPECIAL_PROJECTS_SPECIALPROJECTS.answer(
            &source, serde_json::json!({"branchId":"1", "country":country, "hour":hour}), &load, &cancelled,
        ).unwrap()
    };
    let exact_at_requested = || {
        let panel = dated_panel(&source, &load, &serde_json::json!({"panel":"special-projects", "branchId":"1", "country":"GER"})).unwrap();
        count_reads(|| (panel.exact().unwrap().cold)(requested, &cancelled).unwrap())
    };
    let (rebuilt, rebuilt_reads) = exact_at_requested();
    load_at(serde_json::json!("GER"), previous);
    let (reply, forward_reads) = exact_at_requested();
    assert!(reply["projects"].as_array().is_some_and(|projects| !projects.is_empty()));
    assert_eq!(reply, rebuilt);
    let reach = (previous + 1 - (STAGGER_PERIOD - 1)).max(first);
    let block = |hour| (hour - first) as u64 / u64::from(replay_columnar::unit::BLOCK);
    let forward_limit = owners * (block(requested) - block(reach) + 1);
    load_at(serde_json::json!(7), requested);
    let (repeated, repeated_reads) = count_reads(|| load_at(serde_json::json!("GER"), requested));
    assert_eq!(repeated, reply);
    for table in [TableId::Projects, TableId::ProjectHistory] {
        let forward = forward_reads[&table].stretches;
        let rebuilt = rebuilt_reads[&table].stretches;
        assert!((1..=forward_limit).contains(&forward) && forward < rebuilt,
            "{table:?} walked {forward} stretches advancing (limit {forward_limit}) and {rebuilt} rebuilding");
        assert!(repeated_reads.get(&table).is_none_or(|reads| reads.stretches == 0), "{table:?} walked history at the same hour");
    }
}

#[test]
fn kept_project_history_matches_rebuilds_across_rewinds_gaps_and_run_changes() {
    let machine = dashboard_base::test_fixtures::visual_mods::Machine::new("project_exact_rebuilds");
    machine.write("common/defines/fixture.lua", b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
    let (content_source, options) = machine.open();
    let content = content(&content_source, &LoadContext { content: Some(options) }).unwrap();
    let segments = [(1, 100..=120), (2, 130..=140), (3, 141..=150)].map(|(branch, hours)| {
        records(branch, hours.map(|at| {
            let history = if at >= 107 { vec![(0, M, 107)] } else { Vec::new() };
            let mut frame = frame(at, &Hour { history, ..hour(at, if at >= 107 { COMPLETED } else { RUNNING }) });
            frame.branch = branch;
            frame
        }).collect())
    });
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &segments).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "kept-project-history".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let mut params: SpecialProjectsParams = serde_json::from_value(serde_json::json!({"branchId":"1", "country":"GER"})).unwrap();
    let cancelled = AtomicBool::new(false);
    let kept = Mutex::default();
    let read = |params: &SpecialProjectsParams, at, kept: &Mutex<Kept>| {
        let query = dashboard_base::queries::query(&source, 1).unwrap();
        reply_through(query, params, &content, at, &cancelled, kept)
    };
    for (at, current) in [(105, false), (110, true), (107, false), (120, false), (125, false), (130, false), (140, true), (141, false), (150, false), (105, true)] {
        params.current = Some(current);
        let rebuilt = read(&params, at, &Mutex::default()).unwrap();
        let cached = read(&params, at, &kept).unwrap();
        assert!(!cached.projects.is_empty(), "hour {at}");
        assert_eq!(serde_json::to_vec(&cached).unwrap(), serde_json::to_vec(&rebuilt).unwrap(), "hour {at}");
    }
    params.country = Country::Slot(7);
    let rebuilt = read(&params, 109, &Mutex::default()).unwrap();
    let cached = read(&params, 109, &kept).unwrap();
    assert_eq!(cached, rebuilt);
}

#[test]
fn cached_project_icons_keep_overlay_notes_in_exact_and_followed_replies() {
    use dashboard_base::test_fixtures::visual_mods::{BLUE, Machine, RED, first_pixel, png, select};

    for refused in [false, true] {
        let machine = Machine::new(if refused { "project_icon_refused_notes" } else { "project_icon_notes" });
        machine.write("common/defines/fixture.lua", b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
        machine.write("common/special_projects/projects/fixture.txt", b"fixture_project={icon=GFX_fixture_project}");
        machine.write("localisation/fixture_l_english.yml", b"\xef\xbb\xbfl_english:\n fixture_project:0 \"Fixture project\"\n");
        machine.write("interface/fixture.gfx", b"spriteTypes={spriteType={name=GFX_fixture_project textureFile=gfx/fixture_project.png}}");
        machine.write("gfx/fixture_project.png", &png(RED));
        machine.write_mod("fixture_art", "gfx/fixture_project.png", &if refused { b"invalid fixture image".to_vec() } else { png(BLUE) });
        let frames = [100, 101].map(|at| {
            let mut frame = frame(at, &hour(at, RUNNING));
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 40, literal: b"fixture_project".to_vec() }]));
            frame
        });
        let (source, mut options) = machine.open_frames(frames.into());
        select(&source, &mut options, &["fixture_art"]);
        let load = LoadContext { content: Some(options) };
        let cancelled = AtomicBool::new(false);
        let read = || crate::registrations::SPECIAL_PROJECTS_SPECIALPROJECTS.answer(
            &source, serde_json::json!({"branchId":"1", "country":"GER", "hour":100, "current":true}), &load, &cancelled,
        ).unwrap();
        let notes = |reply: &serde_json::Value| reply["warnings"].as_array().unwrap().iter().filter_map(|line| line.as_str()).filter(|line| line.starts_with("Artwork from visual mods")).map(str::to_owned).collect::<Vec<_>>();
        let first = read();
        let expected = notes(&first);
        assert!(!expected.is_empty(), "the drawing must name its overlay read or refusal: {first}");
        if !refused { assert!(expected.iter().any(|line| line.contains("fixture_art"))); }
        let pixel = if refused { RED } else { BLUE };
        assert_eq!(first_pixel(first["presentations"][0]["icon"].as_str().unwrap()), pixel);
        let second = read();
        assert_eq!(first_pixel(second["presentations"][0]["icon"].as_str().unwrap()), pixel);
        assert_eq!(notes(&second), expected);
        let panel = dated_panel(&source, &load, &serde_json::json!({"panel":"special-projects", "branchId":"1", "country":"GER", "current":true})).unwrap();
        let exact = panel.exact().unwrap();
        for at in [100, 101] {
            let followed = (exact.advance)(at, at == 100, &cancelled, None);
            let reply = followed.value.as_ref().as_ref().unwrap();
            assert_eq!(notes(reply), expected, "followed at {at}");
            assert_eq!(notes(&(exact.cold)(at, &cancelled).unwrap()), expected, "exact at {at}");
        }
    }
}

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
    assert_eq!(uncovered(&[(29, 30)], 25, 25), [(25, 25)]);
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
fn current_carries_the_attempt_the_shown_status_and_only_the_marker_that_dates_it() {
    let facts = recorded("current-pick", 20, 30, |at| Hour { history: if at >= 26 { vec![(0, M, 24)] } else { Vec::new() }, ..hour(at, if at < 24 { RUNNING } else { COMPLETED }) });
    let project = &facts.projects[0];
    let evidence = facts.evidence_of(project).unwrap();
    let late = super::current_pick(project, evidence, 30).unwrap();
    assert_eq!((late.runs.iter().map(|run| (run.first, run.state)).collect::<Vec<_>>(), late.block_first), (vec![(20, ProjectState::Running), (24, ProjectState::Completed)], Some(24)));
    assert_eq!(late.marker.map(|kept| (kept.entry.date, kept.read)), Some((24, 26)), "the marker of its own attempt, once read");
    assert!(super::current_pick(project, evidence, 25).unwrap().marker.is_none(), "a marker read after the hour does not date it yet");
    assert!(late.runs.iter().all(|run| run.first >= 20), "statuses keep their recorded bounds");
    let gapped = recorded("current-gap", 20, 34, |at| Hour { projects_table: !(25..30).contains(&at), ..hour(at, if at < 22 { RUNNING } else { COMPLETED }) });
    let project = &gapped.projects[0];
    let none = super::facts::Evidence::default();
    let pick = super::current_pick(project, gapped.evidence_of(project).unwrap_or(&none), 32).unwrap();
    assert_eq!((pick.block_first, pick.runs.len(), pick.marker.is_none()), (Some(30), 1, true), "a completed block after an unobserved gap starts after it and takes no attempt across it");
    assert_eq!(facts.first_definitions.values().copied().min(), Some(20), "a definition is first seen at its first status, kept as it is read");
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
        found.into_iter().map(|counted| (counted.attribution.tag().map(str::to_owned), counted.entry.date, counted.read, counted.definition)).collect::<Vec<_>>()
    };
    let listed = completions("feed-listed", |at| Hour { history: if at >= 21 { vec![(0, M, 21)] } else { Vec::new() }, ..hour(at, COMPLETED) });
    assert_eq!(listed, [(Some("GER".to_owned()), 21, 21, Some(40))], "one event per counted marker, named by the row read with it");
    let early = completions("feed-early", |at| Hour { history: if at >= 25 { vec![(0, M, 33)] } else { Vec::new() }, ..hour(at, COMPLETED) });
    assert_eq!(early, [], "a marker dated after the read that first listed it is no completion, however often it is listed again");
    let renamed = completions("feed-renamed", |at| Hour { holder: Some(at >= 30), history: if at >= 30 { vec![(0, M, 25)] } else { Vec::new() }, ..hour(at, COMPLETED) });
    assert_eq!(renamed, [(Some("POL".to_owned()), 25, 30, Some(40))], "a tag change between completion and read attributes the completion to the tag held at its date, never to the later tag");
    let bounced = completions("feed-tag-bounced", |at| Hour { holder: if (25..30).contains(&at) { None } else { Some(true) }, history: vec![(0, M, 20)], ..hour(at, COMPLETED) });
    assert_eq!(bounced, [(Some("GER".to_owned()), 20, 20, Some(40))], "another country holding the tag in between leaves the completion emitted once");
}

#[test]
fn feed_completions_keep_untagged_and_renamed_owners_once() {
    let cancelled = AtomicBool::new(false);

    let renamed_src = synthetic("feed-mod-renamed", 20, 34, |at| {
        let mut f = frame(at, &Hour { history: if at >= 30 { vec![(0, M, 25)] } else { Vec::new() }, ..hour(at, COMPLETED) });
        if let TableView::Complete(TableRows::Countries(countries)) = &mut *f.tables[TableId::Countries.index()] {
            countries[0].tag = if at < 30 { b"AAA".to_vec() } else { b"BBB".to_vec() };
        }
        f
    });
    let q_renamed = dashboard_base::queries::query(&renamed_src, 1).unwrap();
    let (found_renamed, _) = counted(&q_renamed, 20, 34, &cancelled).unwrap();
    assert_eq!(found_renamed.len(), 1);
    assert_eq!(found_renamed[0].attribution, CompletionOwner::Resolved { slot: 7, tag: "AAA".to_owned() });
    assert_eq!((found_renamed[0].entry.date, found_renamed[0].read, found_renamed[0].definition), (25, 30, Some(40)));

    let unlinked_src = synthetic("feed-unlinked-owner", 20, 34, |at| {
        frame(at, &Hour { linked: false, history: if at >= 25 { vec![(0, M, 25)] } else { Vec::new() }, ..hour(at, COMPLETED) })
    });
    let q_unlinked = dashboard_base::queries::query(&unlinked_src, 1).unwrap();
    let (found_unlinked, _) = counted(&q_unlinked, 20, 34, &cancelled).unwrap();
    assert_eq!(found_unlinked.len(), 1);
    assert_eq!((found_unlinked[0].entry.date, found_unlinked[0].read, found_unlinked[0].definition), (25, 25, Some(40)));
    assert_eq!(found_unlinked[0].attribution.slot(), None);
    assert_eq!(found_unlinked[0].attribution.tag(), None);
    assert!(found_unlinked[0].attribution.missing_reason().is_some_and(|why| why.contains("Owner not recorded")));

    let late_link_src = synthetic("feed-late-link", 20, 34, |at| {
        let mut f = frame(at, &Hour { linked: at >= 30, history: if at >= 30 { vec![(0, M, 25)] } else { Vec::new() }, ..hour(at, COMPLETED) });
        if let TableView::Complete(TableRows::Countries(countries)) = &mut *f.tables[TableId::Countries.index()] {
            countries[0].tag = b"BBB".to_vec();
        }
        f
    });
    let q_late = dashboard_base::queries::query(&late_link_src, 1).unwrap();
    let (found_late, _) = counted(&q_late, 20, 34, &cancelled).unwrap();
    assert_eq!(found_late.len(), 1);
    assert_eq!(found_late[0].attribution.tag(), None, "an owner not linked at the completion date is never credited to the later tag");
    assert!(found_late[0].attribution.missing_reason().is_some_and(|why| why.contains("Owner not recorded")));

    let no_tag_src = synthetic("feed-no-tag-at-date", 20, 34, |at| {
        let mut f = frame(at, &Hour { history: if at >= 25 { vec![(0, M, 25)] } else { Vec::new() }, ..hour(at, COMPLETED) });
        if at == 25 {
            *f.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic countries refusal".into() };
        } else if let TableView::Complete(TableRows::Countries(countries)) = &mut *f.tables[TableId::Countries.index()] {
            countries[0].tag = b"CCC".to_vec();
        }
        f
    });
    let q_no_tag = dashboard_base::queries::query(&no_tag_src, 1).unwrap();
    let (found_no_tag, _) = counted(&q_no_tag, 20, 34, &cancelled).unwrap();
    assert_eq!(found_no_tag.len(), 1);
    assert_eq!(found_no_tag[0].attribution.slot(), Some(7));
    assert_eq!(found_no_tag[0].attribution.tag(), None);
    assert!(found_no_tag[0].attribution.missing_reason().is_some_and(|why| why.contains("Countries")));

    let mid_gap_src = synthetic("feed-mid-links-gap", 20, 34, |at| {
        let mut f = frame(at, &Hour { history: if at >= 30 { vec![(0, M, 25)] } else { Vec::new() }, ..hour(at, COMPLETED) });
        if let TableView::Complete(TableRows::Countries(countries)) = &mut *f.tables[TableId::Countries.index()] {
            countries[0].tag = if at < 30 { b"AAA".to_vec() } else { b"BBB".to_vec() };
        }
        if at == 27 {
            *f.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "Synthetic mid-gap links refusal".into() };
        }
        f
    });
    let q_mid_gap = dashboard_base::queries::query(&mid_gap_src, 1).unwrap();
    let (found_mid_gap, _) = counted(&q_mid_gap, 20, 34, &cancelled).unwrap();
    assert_eq!(found_mid_gap.len(), 1);
    assert_eq!(found_mid_gap[0].attribution, CompletionOwner::Resolved { slot: 7, tag: "AAA".to_owned() });
    assert_eq!((found_mid_gap[0].entry.date, found_mid_gap[0].read, found_mid_gap[0].definition), (25, 30, Some(40)));
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

fn reference_counted(query: &Query<'_>, from: i32, to: i32, cancelled: &AtomicBool) -> io::Result<(Vec<Counted>, Vec<HourSpan>)> {
    let recording = query.replay.recording();
    let mut unrecorded: Vec<HourSpan> = Vec::new();
    let mut cached_links: HashMap<u32, Result<Arc<RunLinks>, String>> = HashMap::new();
    let get_links = |cached: &mut HashMap<u32, Result<Arc<RunLinks>, String>>, run: u32| -> io::Result<Result<Arc<RunLinks>, String>> {
        if let Some(held) = cached.get(&run) {
            return Ok(held.clone());
        }
        let entry = match dashboard_query::cost::storage(|| country_ids::run_links(recording, run)) {
            Ok(links) => Ok(Arc::new(links)),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => Err(error.to_string()),
            Err(error) => return Err(error),
        };
        cached.insert(run, entry.clone());
        Ok(entry)
    };
    let mut owners: BTreeSet<u32> = BTreeSet::new();
    for stretch in query.timeline.within(from, to) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let held = available(query, stretch.run, TableId::ProjectHistory, stretch.first, stretch.last)?;
        unrecorded.extend(uncovered(&held, stretch.first, stretch.last).into_iter().map(|(first, last)| HourSpan { first_hour: first, last_hour: last }));
        let history = dashboard_query::cost::storage(|| lanes::history(recording, stretch.run, TableId::ProjectHistory, &HISTORY_COLUMNS, Keys::All, stretch.first, stretch.last))?;
        for (key, steps) in &history.keys {
            if steps.iter().any(|step| step.cells.as_deref().and_then(|cells| history_entry(*key as u32, cells)).is_some_and(|entry| entry.completion() && (from..=to).contains(&entry.date))) {
                owners.insert((key >> 64) as u32);
            }
        }
    }
    let sample_cols = read_columns()?;
    let mut found: BTreeMap<(u32, u32, u32, i32), Counted> = BTreeMap::new();
    for owner in owners {
        let mut facts = Facts::new(query.timeline.first);
        for stretch in query.timeline.within(query.timeline.first, to) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let opened = recording.run(stretch.run).map_or(stretch.first, |run| run.first_hour);
            let reach = stretch.first.saturating_sub(STAGGER_PERIOD - 1).max(opened);
            let raw = u128::from(owner);
            let projects = dashboard_query::cost::storage(|| {
                lanes::history(recording, stretch.run, TableId::Projects, &PROJECT_COLUMNS, Keys::Range(raw << 32, raw << 32 | u128::from(u32::MAX)), reach, stretch.last)
            })?;
            let histories = dashboard_query::cost::storage(|| {
                lanes::history(recording, stretch.run, TableId::ProjectHistory, &HISTORY_COLUMNS, Keys::Range(raw << 64, raw << 64 | u128::from(u64::MAX)), reach, stretch.last)
            })?;
            let read_keys = [read_key(TableId::Projects, owner), read_key(TableId::ProjectHistory, owner)];
            let reads = dashboard_query::cost::storage(|| lanes::history(recording, stretch.run, TableId::SliceReads, &sample_cols, Keys::These(&read_keys), stretch.first, stretch.last))?;
            let projects_complete = available(query, stretch.run, TableId::Projects, reach, stretch.last)?;
            let history_complete = available(query, stretch.run, TableId::ProjectHistory, reach, stretch.last)?;
            let mut project_cursors: Vec<(u32, Cursor<'_>)> = projects.keys.iter().map(|(key, steps)| (*key as u32, Cursor::new(steps))).collect();
            let mut entry_cursors: Vec<(u32, u32, Cursor<'_>)> = histories.keys.iter().map(|(key, steps)| ((key >> 32) as u32, *key as u32, Cursor::new(steps))).collect();
            let mut stamps: HashMap<u128, Cursor<'_>> = reads.keys.iter().map(|(key, steps)| (*key, Cursor::new(steps))).collect();
            for hour in stretch.first..=stretch.last {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let mut usable = |table: TableId, complete: &[(i32, i32)]| {
                    sample(stamps.get_mut(&read_key(table, owner)).and_then(|cursor| cursor.at(hour)))
                        .filter(|&s| current(s, hour) && s >= reach && within(complete, s))
                };
                let projects_read: BTreeMap<u32, i32> = usable(TableId::Projects, &projects_complete).map(|s| (owner, s)).into_iter().collect();
                let histories_read: BTreeMap<u32, i32> = usable(TableId::ProjectHistory, &history_complete).map(|s| (owner, s)).into_iter().collect();
                let mut listed: BTreeMap<(u32, u32), Vec<Entry>> = BTreeMap::new();
                if let Some(&stamp) = histories_read.get(&owner) {
                    for (project, ordinal, cursor) in &mut entry_cursors {
                        if let Some(entry) = cursor.at(stamp).and_then(|cells| history_entry(*ordinal, cells)) {
                            listed.entry((owner, *project)).or_default().push(entry);
                        }
                    }
                }
                let mut rows: Vec<(Row, i32)> = Vec::new();
                if let Some(&stamp) = projects_read.get(&owner) {
                    for (local, cursor) in &mut project_cursors {
                        if let Some(row) = cursor.at(stamp).and_then(|cells| project_row(owner, *local, cells)) {
                            rows.push((row, stamp));
                        }
                    }
                }
                let shown: Vec<facts::Read<'_>> = rows.iter().map(|(row, stamp)| facts::Read { row, read: *stamp, program: Program::Unassigned }).collect();
                facts.histories_only(&facts::Hour {
                    country: (owner, 0),
                    at: hour,
                    held: &|_, _, _| true,
                    projects: &projects_read,
                    histories: &histories_read,
                    listed: &listed,
                    reads: &shown,
                    published: false,
                });
            }
        }
        for (&(_, _, owner, local), evidence) in &facts.evidence {
            for kept in evidence.entries.iter().filter(|kept| kept.counted && (from..=to).contains(&kept.entry.date)) {
                let key = (owner, local, kept.entry.ordinal, kept.entry.date);
                if let std::collections::btree_map::Entry::Vacant(vacant) = found.entry(key) {
                    let attribution = resolve_owner(query, &mut cached_links, &get_links, owner, kept.entry.date)?;
                    vacant.insert(Counted { attribution, owner, local, entry: kept.entry, read: kept.read, definition: kept.definition });
                }
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
        let mut countries_unavail: BTreeSet<i32> = BTreeSet::new();
        for at in first..=last {
            if rng.chance(1, 7) { slice_unavail.insert(at); }
            else if rng.chance(1, 9) { slice_trunc.insert(at); }
            if rng.chance(1, 10) { proj_unavail.insert(at); }
            if rng.chance(1, 12) { hist_unavail.insert(at); }
            if rng.chance(1, 10) { lex_unavail.insert(at); }
            if rng.chance(1, 14) { countries_unavail.insert(at); }
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
            if countries_unavail.contains(&at) {
                *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic countries unavailable".into() };
            } else if let TableView::Complete(TableRows::Countries(countries)) = &mut *frame.tables[TableId::Countries.index()] {
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
    let shown = shown_at(&projects, &presentations, &available, 40, &[]);
    assert_eq!(shown.running, [3, 1], "more progress first, and an unknown state is not in progress");
    assert_eq!(shown.unknown, [7]);
    assert_eq!(shown.completed[..2], [ShownCompleted { project: 2, date: Some(30) }, ShownCompleted { project: 0, date: Some(20) }]);
    assert_eq!(shown.undated, 3, "a marker read after the hour, a project first seen complete and an earlier attempt's marker date nothing");
    assert!(shown.completed[2..].iter().all(|item| item.date.is_none()));
    assert_eq!(shown_at(&projects, &presentations, &available, 60, &[]), ProjectsShown { through: Some(60), ..ProjectsShown::default() }, "an hour not recorded shows nothing");
    let split = vec![project("s", Running, vec![status(10, 19, 0, Running, 80), status(20, 29, 0, Completed, 100), status(30, 50, 1, Completed, 100)], vec![completion(20, 20)])];
    assert_eq!(shown_at(&split, &presentations, &available, 40, &[]).completed, [ShownCompleted { project: 0, date: Some(20) }], "a presentation change inside the completed status keeps the attempt's completion date");
    let gapped = vec![project("g", Running, vec![status(10, 15, 0, Running, 80), status(20, 50, 0, Completed, 100)], vec![completion(18, 20)])];
    assert_eq!(shown_at(&gapped, &presentations, &available, 40, &[]).completed, [ShownCompleted { project: 0, date: None }], "a marker across an unread gap does not link the attempt to its completion");
    let compact = vec![project("c", Running, vec![status(10, 19, 0, Running, 80), status(30, 50, 1, Completed, 100)], vec![completion(20, 20)])];
    assert_eq!(shown_at(&compact, &presentations, &available, 40, &[Some(20)]).completed, [ShownCompleted { project: 0, date: Some(20) }], "a compact Current reply dates the completion from its block's start, keeping each status's own bounds");
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

fn materialized_at(mut reply: serde_json::Value, hour: i32) -> serde_json::Value {
    fn clip(spans: &mut serde_json::Value, hour: i32) {
        for span in spans.as_array_mut().unwrap() {
            assert!(span["firstHour"].as_i64().unwrap() <= i64::from(hour));
            if span["lastHour"].as_i64().unwrap() > i64::from(hour) { span["lastHour"] = hour.into(); }
        }
    }
    for key in ["identities", "unresolved", "available", "partial", "reads"] {
        clip(&mut reply[key], hour);
    }
    for project in reply["projects"].as_array_mut().unwrap() {
        clip(&mut project["statuses"], hour);
    }
    reply["shown"]["through"] = hour.into();
    reply
}

#[test]
fn current_special_projects_names_the_unknown_holder_reason_at_the_requested_hour() {
    dashboard_core::test_fixtures::game::enter();
    let root = std::env::temp_dir().join(format!("special-projects-current-holder-{}-{:?}", std::process::id(), std::thread::current().id()));
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
    let _cleanup = Cleanup(root.clone());
    for (path, text) in [
        ("common/defines/test.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=7}}"),
        ("common/country_tags/fixture.txt", "GER=countries/home.txt POL=countries/other.txt"),
        ("common/special_projects/specialization/test.txt", "mod_science={}"),
        ("common/special_projects/projects/test.txt", "project={specialization=mod_science}"),
        ("localisation/test_l_english.yml", "\u{feff}l_english:\n project:0 \"Project\"\n"),
        ("hoi4.exe", "test"),
        ("user/settings.txt", "language=l_english force_pow2_textures=no"),
        ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let (options, identity) = dashboard_base::test_fixtures::content_identity(&root);
    let build = |name: &str, frames: Vec<replay_frame::frame::DecodedFrame>| {
        let mut input = records(1, frames);
        let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] else { unreachable!() };
        session.identity = identity.clone();
        let path = root.join(format!("{name}.battleplan"));
        dashboard_base::test_fixtures::write_with_facts(&path, &options, &identity, input);
        let replay = replay::campaign::Replay::open(std::fs::read(&path).unwrap()).unwrap().into_timelines(&Default::default()).unwrap();
        Source::from_replay(dashboard_base::SourceKey { source_id: name.into(), revision: "1".into() }, path, Arc::new(replay), &AtomicBool::new(false)).unwrap()
    };
    let with_unknown = |at: i32| {
        let mut f = frame(at, &hour(at, RUNNING));
        *f.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow { token: 40, literal: b"project".to_vec() }]));
        if at == 25 {
            *f.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
        }
        if at == 27 {
            if let TableView::Complete(TableRows::CountryLinks(links)) = &mut *f.tables[TableId::CountryLinks.index()] {
                links.retain(|link: &CountryLinkRow| link.id != 9001);
            }
        }
        f
    };
    let source = build("gap-mid", (20..=30).map(with_unknown).collect());
    let load = LoadContext { content: Some(options.clone()) };
    let cancelled = AtomicBool::new(false);
    for (country, subject) in [(Country::Tag("GER".into()), "GER"), (Country::Slot(7), "country slot 7")] {
        let product = |at: i32| {
            let asked = serde_json::json!({ "branchId": "1", "country": country, "current": true, "hour": at });
            crate::registrations::SPECIAL_PROJECTS_SPECIALPROJECTS.answer(&source, asked, &load, &cancelled).unwrap()["warnings"].clone()
        };
        product(24);
        let warned: Vec<_> = [24, 25, 26, 27, 28].map(product).into();
        let asked = serde_json::json!({ "panel": "special-projects", "branchId": "1", "country": country, "current": true });
        let panel = dated_panel(&source, &load, &asked).unwrap();
        let exact = panel.exact().unwrap();
        for (at, warned) in [24, 25, 26, 27, 28].into_iter().zip(warned) {
            let followed = (exact.advance)(at, at == 24, &cancelled, None).value.as_ref().as_ref().unwrap().clone();
            let direct = (exact.cold)(at, &cancelled).unwrap();
            assert_eq!(followed["warnings"], warned, "the followed reply warns as the product read does at hour {at} for {country:?}");
            assert_eq!(materialized_at(followed, at), direct, "followed matches direct at hour {at} for {country:?}");
            if at == 25 {
                let why = direct["unavailable"].as_str().expect("unknown holder hour must explain why Current is unavailable");
                assert_eq!(why, format!("Who holds {subject} is not known: Countries was not complete at these hours"));
            } else if at == 27 {
                let why = direct["unavailable"].as_str().expect("unlinked tag hour must explain why Current is unavailable");
                assert_eq!(why, format!("Who holds {subject} is not known: {}", dashboard_query::identity::TAG_UNLINKED));
            } else {
                assert_eq!(direct["unavailable"], serde_json::Value::Null, "known hour {at} should be available: {direct}");
            }
        }
    }
    let fixed = build("played-project-reads", (20..=35).map(|at| {
        let stamp = if at < 25 { 20 } else if at < 32 { 25 } else { 32 };
        let state = if stamp == 20 { (0x168, 1, 1, 0) } else { COMPLETED };
        let mut frame = frame(at, &hour(stamp, state));
        if let TableView::Complete(TableRows::SliceReads(rows)) = &mut *frame.tables[TableId::SliceReads.index()] {
            for row in rows { row.sample_hour = stamp; }
        }
        if (29..=31).contains(&at) {
            *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "Damaged project custody".into() };
        }
        frame
    }).collect());
    for current in [true, false] {
        let asked = serde_json::json!({"branchId":"1", "country":"GER", "current":current});
        let panel = dated_panel(&fixed, &load, &asked).unwrap();
        let exact = panel.exact().unwrap();
        for step in dashboard_base::test_fixtures::played(fixed.replay_ref(), exact, 20, 35) {
            let at = step.hour;
            let data = step.value.as_ref().as_ref().unwrap().clone();
            assert_eq!(materialized_at(data, at), (exact.cold)(at, &cancelled).unwrap(), "current={current} at {at}");
            assert_eq!(step.product_calls, u64::from(matches!(at, 20 | 25 | 29 | 32)), "current={current} at {at}");
        }
        let sought = (exact.advance)(22, true, &cancelled, None);
        assert!(sought.fresh);
        assert_eq!(materialized_at(sought.value.as_ref().clone().unwrap(), 22), (exact.cold)(22, &cancelled).unwrap());
    }
    let history = SpecialProjects::load(
        &source,
        &SpecialProjectsParams { branch_id: "1".into(), country: Country::Tag("GER".into()), hour: Some(30), current: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!(history.unavailable, None, "History keeps earlier and later lanes across an unknown-holder hour");
    assert_eq!(history.unresolved, vec![HourSpan { first_hour: 25, last_hour: 25 }, HourSpan { first_hour: 27, last_hour: 27 }]);
    let slot_history = SpecialProjects::load(
        &source,
        &SpecialProjectsParams { branch_id: "1".into(), country: Country::Slot(7), hour: Some(30), current: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!(slot_history.unavailable, None);
    assert_eq!(slot_history.unresolved, vec![HourSpan { first_hour: 25, last_hour: 25 }, HourSpan { first_hour: 27, last_hour: 27 }]);

    let only_unlinked = build(
        "only-unlinked",
        (20..=23)
            .map(|at| {
                let mut f = frame(at, &hour(at, RUNNING));
                if let TableView::Complete(TableRows::CountryLinks(links)) = &mut *f.tables[TableId::CountryLinks.index()] {
                    links.retain(|link: &CountryLinkRow| link.id != 9001);
                }
                f
            })
            .collect(),
    );
    let slot_unlinked_history = SpecialProjects::load(
        &only_unlinked,
        &SpecialProjectsParams { branch_id: "1".into(), country: Country::Slot(7), hour: Some(23), current: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!(
        slot_unlinked_history.unavailable.as_deref(),
        Some(format!("Who holds country slot 7 is not known: {}", dashboard_query::identity::TAG_UNLINKED).as_str()),
    );
    assert_eq!(slot_unlinked_history.unresolved, vec![HourSpan { first_hour: 20, last_hour: 23 }]);

    let bounded_unlinked = build(
        "bounded-unlinked",
        (25..=30)
            .map(|at| {
                let mut f = frame(at, &Hour { holder: Some(at == 25 || at >= 29), ..hour(at, RUNNING) });
                if at == 25 {
                    if let TableView::Complete(TableRows::CountryLinks(links)) = &mut *f.tables[TableId::CountryLinks.index()] {
                        links.retain(|link: &CountryLinkRow| link.id != 9001);
                    }
                } else if (26..=28).contains(&at) {
                    if let TableView::Complete(TableRows::Countries(countries)) = &mut *f.tables[TableId::Countries.index()] {
                        countries[0].canonical_slot = 8;
                    }
                    if let TableView::Complete(TableRows::CountryLinks(links)) = &mut *f.tables[TableId::CountryLinks.index()] {
                        for link in links.iter_mut() {
                            if link.id == 9001 {
                                link.canonical_slot = 8;
                            }
                        }
                    }
                }
                f
            })
            .collect(),
    );
    let slot_bounded = SpecialProjects::load(
        &bounded_unlinked,
        &SpecialProjectsParams { branch_id: "1".into(), country: Country::Slot(7), hour: Some(30), current: None },
        &load,
        &cancelled,
    )
    .unwrap();
    assert_eq!(slot_bounded.unavailable, None);
    assert_eq!(slot_bounded.unresolved, vec![HourSpan { first_hour: 25, last_hour: 25 }]);

    let early_unknown = |at: i32| {
        let mut f = frame(at, &Hour { holder: Some(false), ..hour(at, RUNNING) });
        if at <= 22 {
            *f.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
        }
        f
    };
    let absent_after = build("early-unknown", (20..=25).map(early_unknown).collect());
    let during = SpecialProjects::load(
        &absent_after,
        &SpecialProjectsParams { branch_id: "1".into(), country: Country::Tag("GER".into()), hour: Some(21), current: Some(true) },
        &load,
        &cancelled,
    )
    .unwrap();
    let why = during.unavailable.as_deref().expect("unknown holder at 21 must report reason");
    assert!(why.contains("Who holds GER is not known:") && why.contains("Countries"), "got {why:?}");
    let after = SpecialProjects::load(
        &absent_after,
        &SpecialProjectsParams { branch_id: "1".into(), country: Country::Tag("GER".into()), hour: Some(25), current: Some(true) },
        &load,
        &cancelled,
    )
    .unwrap();
    let why_after = after.unavailable.as_deref().expect("absent country at 25 is unavailable");
    assert!(!why_after.contains("is not known:"), "known absent hour 25 must not reuse earlier unknown-holder reason: {why_after:?}");
}
