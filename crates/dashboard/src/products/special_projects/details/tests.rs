use super::*;
use crate::{
    products::at_hour::entry,
    test_fixtures::{CAMPAIGN, records, source},
};
use replay::campaign::SeekResult;
use replay_data::rows::{
    CharacterRoleTraitRow, CountryLinkRow, LexicalTokenRow, ProjectBreakthroughRow, ProjectHistoryRow, ProjectProgramRow, ProjectScientistAssignment, ScientistRow,
    ScientistSkillRow, SliceReadRow, TableRows,
};
use replay_frame::frame::TableView;
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

const FIRST: i32 = 20;
const LAST: i32 = 55;
const DAILY_READS: [i32; 2] = [28, 52];
const PROJECT_READS: [i32; 3] = [30, 40, 55];
const UNHELD: i32 = 45;
const LATE_ITERATION: i32 = 48;
const UNHELD_NEXT_ITERATION: i32 = 42;
const MOVED_OWNER: u32 = 9002;
const MOVED_READS: [i32; 3] = [47, 50, 53];
const MOVED_HELD: std::ops::Range<i32> = 49..53;
const MOVED_ITERATION: i32 = 51;
const HISTORY_UNSTAMPED: [i32; 2] = [30, 35];
const HISTORY_READ_AT_FIRST: std::ops::Range<i32> = FIRST..45;
const ALIAS_UNSTAMPED: i32 = 38;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, None);
    if hour == UNHELD {
        *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
    }
    for table in [TableId::Characters, TableId::CharacterPortraits, TableId::ModifierDefinitions] {
        set(&mut frame, table, TableRows::empty(table));
    }
    set(&mut frame, TableId::Scientists, TableRows::Scientists(vec![
        ScientistRow { character: 7, country: 11, active: Recorded::Value(Some(1)), ..Default::default() },
        ScientistRow { character: 8, country: 12, active: Recorded::Value(Some(0)), ..Default::default() },
    ]));
    set(&mut frame, TableId::ScientistDaily, TableRows::ScientistDaily(vec![ScientistDailyRow { character: 7, country: 11, injury_days: Recorded::Value(Some(8)), template_visibility: Recorded::Value(Some(0)), ..Default::default() }]));
    set(&mut frame, TableId::ScientistSkills, TableRows::ScientistSkills(vec![ScientistSkillRow { character: 7, ordinal: 0, token: 41, level: -2, experience_raw: i64::MAX }]));
    set(&mut frame, TableId::CharacterRoleTraits, TableRows::CharacterRoleTraits(vec![CharacterRoleTraitRow { character: 7, kind: CharacterRoleTraitRow::SCIENTIST, tokens: vec![42, 42], ..Default::default() }]));
    let spelled_42: &[u8] = if hour == FIRST { b"mod_option" } else { b"mod_trait" };
    set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(vec![LexicalTokenRow { token: 41, literal: b"mod_skill".to_vec() }, LexicalTokenRow { token: 42, literal: spelled_42.to_vec() }]));
    set(&mut frame, TableId::ProjectPrograms, TableRows::ProjectPrograms(vec![ProjectProgramRow {
        id: 9,
        owner: 11,
        assignments: Recorded::Value(Some(vec![ProjectScientistAssignment::default(), ProjectScientistAssignment { scientist: 7 << 32 | 73, target: 6, current: 2, timed: 1 }])),
        ..Default::default()
    }]));
    set(&mut frame, TableId::ProjectBreakthroughs, TableRows::ProjectBreakthroughs(vec![ProjectBreakthroughRow { owner: 11, specialization: 41, points: 777 }]));
    if MOVED_HELD.contains(&hour) {
        let TableView::Complete(TableRows::CountryLinks(links)) = &mut *frame.tables[TableId::CountryLinks.index()] else { unreachable!() };
        links.push(CountryLinkRow { id: MOVED_OWNER, canonical_slot: 7 });
    }
    let moved = MOVED_READS.into_iter().filter(|&at| at <= hour).max();
    let read = PROJECT_READS.into_iter().filter(|&at| at <= hour).max();
    let history = |ordinal, date, first: i32| (hour >= first).then_some(ProjectHistoryRow { owner: 9001, project: 3, ordinal, prototype_reward_handle: 41, option_handle: 42, date, ..Default::default() });
    set(&mut frame, TableId::Projects, TableRows::Projects(
        [read.map(|_| ProjectRow { owner: 9001, local_id: 3, definition_id: 77, ..Default::default() }), moved.map(|_| ProjectRow { owner: MOVED_OWNER, local_id: 5, definition_id: 77, ..Default::default() })]
            .into_iter()
            .flatten()
            .collect(),
    ));
    set(&mut frame, TableId::ProjectHistory, TableRows::ProjectHistory(
        [
            history(0, FIRST, FIRST),
            history(1, FIRST, FIRST).map(|row| ProjectHistoryRow { prototype_reward_handle: COMPLETION_MARKER, option_handle: COMPLETION_MARKER, ..row }),
            history(2, FIRST, FIRST),
            history(3, FIRST, FIRST),
            history(4, FIRST, FIRST),
            history(5, LATE_ITERATION, PROJECT_READS[2]),
            history(6, UNHELD_NEXT_ITERATION, PROJECT_READS[2]),
            moved.filter(|&read| read >= MOVED_READS[2]).map(|_| ProjectHistoryRow { owner: MOVED_OWNER, project: 5, ordinal: 0, prototype_reward_handle: 41, option_handle: 42, date: MOVED_ITERATION, ..Default::default() }),
        ]
        .into_iter()
        .flatten()
        .collect(),
    ));
    let history_read = |read: Option<i32>| match hour {
        _ if HISTORY_UNSTAMPED.contains(&hour) => Some(FIRST - 1),
        _ if HISTORY_READ_AT_FIRST.contains(&hour) => Some(FIRST),
        _ => read,
    };
    let stamp = |table: TableId, owner, read: Option<i32>| read.map(|sample_hour| SliceReadRow { table: table.index() as u32, owner, sample_hour });
    let mut slices: Vec<SliceReadRow> = [(9001, read), (11, read.filter(|_| hour != ALIAS_UNSTAMPED))]
        .into_iter()
        .flat_map(|(owner, read)| [stamp(TableId::Projects, owner, read), stamp(TableId::ProjectHistory, owner, history_read(read))])
        .chain([stamp(TableId::Projects, MOVED_OWNER, moved), stamp(TableId::ProjectHistory, MOVED_OWNER, moved)])
        .flatten()
        .collect();
    slices.sort_by_key(|row| (row.table, row.owner));
    set(&mut frame, TableId::SliceReads, TableRows::SliceReads(slices));
    for table in [TableId::ProjectPrograms, TableId::ProjectBreakthroughs, TableId::ScientistDaily] {
        frame.metrics.table_ns[table.index()] = u64::from(DAILY_READS.contains(&hour));
    }
    frame
}

fn recording() -> Source {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(crate::SourceKey { source_id: "special-project-details".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn load() -> LoadContext {
    LoadContext { content: None }
}

fn asked<P: AtHour>(source: &Source, request: Value) -> Result<Value, dashboard_core::QueryError> {
    let text = (entry::<P>().registered.load)(source, "lease", request, &load(), &AtomicBool::new(false))?;
    let mut reply: Value = serde_json::from_str(&text).unwrap();
    Ok(reply["data"].take())
}

fn details(source: &Source, hour: i32) -> Value {
    asked::<SpecialProjectDetailsAt>(source, json!({ "branchId": "1", "country": "GER", "hour": hour })).unwrap()
}

fn has(reply: &Value, text: &str) -> bool {
    reply["unavailable"].as_array().unwrap().iter().any(|line| line.as_str().unwrap().contains(text))
}

#[test]
fn details_carry_each_daily_value_with_the_hour_of_its_read() {
    let source = recording();
    let reply = details(&source, 40);
    let scientists = reply["scientists"].as_array().unwrap();
    assert_eq!(scientists.len(), 1, "another country's scientist is not GER's: {scientists:?}");
    let scientist = &scientists[0];
    assert_eq!((scientist["active"].clone(), scientist["injuryDays"].clone(), scientist["injuryHour"].clone()), (json!(1), json!(8), json!(28)));
    assert_eq!((scientist["visible"].clone(), scientist["visibilityHour"].clone()), (json!(true), json!(28)), "a null retained template is visible");
    assert_eq!(scientist["skills"][0]["experienceRaw"], json!(i64::MAX.to_string()));
    assert_eq!(scientist["traits"].as_array().unwrap().len(), 2, "a duplicate trait is kept");
    let program = &reply["programs"][0];
    assert_eq!((program["readHour"].clone(), program["assignments"][1]["current"].clone()), (json!(28), json!(2)));
    assert_eq!(program["assignments"][0]["person"]["assigned"], json!(false), "an empty assignment is no scientist");
    assert_eq!((reply["breakthroughs"][0]["points"].clone(), reply["breakthroughs"][0]["readHour"].clone()), (json!(777), json!(28)));
    let project = &reply["projects"][0];
    assert_eq!((project["readHour"].clone(), project["label"]["token"].clone(), project["label"]["name"].clone()), (json!(PROJECT_READS[1]), json!("77"), Value::Null));
    assert!(has(&reply, "77"), "an unspelled definition keeps its project, named as missing: {reply}");
    assert_eq!(project["choices"].as_array().unwrap().len(), 4, "the completion marker is not a chosen option");
    assert_eq!(project["historyReadHour"], json!(FIRST), "the choices carry the history read they come from, apart from the project read: {project}");
    assert_eq!(project["choices"][0]["option"]["token"], json!("mod_option"), "a choice is spelled by the history read it comes from, as its effects are: {project}");
    assert_eq!(details(&source, 53)["programs"][0]["readHour"], json!(52), "the next day's read supplies the next day");
    let unstamped = details(&source, HISTORY_UNSTAMPED[1]);
    assert_eq!(unstamped["projects"][0]["choices"], Value::Null, "a complete history table with no proven read is not an empty one: {unstamped}");
    assert_eq!(unstamped["projects"][0]["historyReadHour"], Value::Null, "no choices, no history read");
    assert!(has(&unstamped, "Project choice history was not read"), "{unstamped}");
}

fn followed_as_direct(source: &Source) -> impl Fn(i32) -> Value + use<> {
    let panel = crate::products::at_hour::exact_panel::<SpecialProjectDetailsAt>(source, &load(), &json!({ "branchId": "1", "country": "GER" })).unwrap();
    let exact = panel.exact().unwrap();
    let cancelled = AtomicBool::new(false);
    for hour in FIRST..=LAST {
        let followed = (exact.advance)(hour, hour == FIRST, &cancelled, None);
        match (exact.cold)(hour, &cancelled) {
            Ok(direct) => assert_eq!(followed.value.as_ref().as_ref().ok(), Some(&direct), "hour {hour}"),
            Err(_) => assert!(followed.value.is_err(), "hour {hour}"),
        }
    }
    let cold = exact.cold.clone();
    move |hour| cold(hour, &AtomicBool::new(false)).unwrap()
}

#[test]
fn followed_details_answer_as_the_direct_query_as_the_history_read_expires_and_returns() {
    let at = followed_as_direct(&recording());
    assert_eq!(at(HISTORY_UNSTAMPED[1] - 1)["projects"][0]["choices"].as_array().map(Vec::len), Some(4));
    assert_eq!(at(HISTORY_UNSTAMPED[1])["projects"][0]["choices"], Value::Null, "an unproven history read is not recorded, never the last one's choices");
    let expired = FIRST + STAGGER_PERIOD;
    assert!(at(expired - 1)["projects"][0]["choices"].is_array());
    assert_eq!(at(expired)["projects"][0]["choices"], Value::Null, "a history read older than a day is not recorded, though no table changed");
    assert!(has(&at(expired), "Project choice history was not read"));
}

#[test]
fn followed_details_drop_an_owners_refusal_once_another_reason_withholds_its_choices() {
    const SHIFTED: u32 = 9003;
    const UNSTAMPED: u32 = 9004;
    const READ: i32 = 30;
    const EXPIRED: i32 = FIRST + STAGGER_PERIOD;
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        for &table in DETAIL_TABLES.iter().filter(|&&table| table != TableId::CountryLinks) {
            set(&mut frame, table, TableRows::empty(table));
        }
        let TableView::Complete(TableRows::CountryLinks(links)) = &mut *frame.tables[TableId::CountryLinks.index()] else { unreachable!() };
        if hour > FIRST {
            links.push(CountryLinkRow { id: SHIFTED, canonical_slot: 7 });
        }
        links.push(CountryLinkRow { id: UNSTAMPED, canonical_slot: 7 });
        let read = hour >= READ;
        let projects = [SHIFTED, UNSTAMPED].map(|owner| ProjectRow { owner, local_id: 1, definition_id: 77, ..Default::default() });
        set(&mut frame, TableId::Projects, TableRows::Projects(if read { projects.to_vec() } else { Vec::new() }));
        set(&mut frame, TableId::ProjectHistory, TableRows::empty(TableId::ProjectHistory));
        let stamp = |table: TableId, owner, sample_hour| SliceReadRow { table: table.index() as u32, owner, sample_hour };
        let slices = if read {
            vec![stamp(TableId::Projects, 11, READ), stamp(TableId::Projects, 9001, READ), stamp(TableId::Projects, SHIFTED, READ), stamp(TableId::Projects, UNSTAMPED, READ), stamp(TableId::ProjectHistory, SHIFTED, FIRST)]
        } else {
            Vec::new()
        };
        set(&mut frame, TableId::SliceReads, TableRows::SliceReads(slices));
        frame
    };
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, (FIRST..=LAST).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(crate::SourceKey { source_id: "special-project-refusals".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let at = followed_as_direct(&source);
    assert!(has(&at(EXPIRED - 1), OTHER_OWNER), "the history read before the owner joined is another country's");
    assert!(!has(&at(EXPIRED), OTHER_OWNER), "once that read is older than a day, only its age withholds the choices");
    assert!(has(&at(EXPIRED), "Project choice history was not read"));
}

#[test]
fn followed_details_spell_choices_again_when_unchanged_rows_are_read_again() {
    const SPELLED: i32 = 22;
    const READS: [i32; 2] = [FIRST, 40];
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        for &table in DETAIL_TABLES.iter().filter(|&&table| table != TableId::CountryLinks) {
            set(&mut frame, table, TableRows::empty(table));
        }
        set(&mut frame, TableId::Projects, TableRows::Projects(vec![ProjectRow { owner: 9001, local_id: 3, definition_id: 77, ..Default::default() }]));
        set(&mut frame, TableId::ProjectHistory, TableRows::ProjectHistory(vec![ProjectHistoryRow { owner: 9001, project: 3, ordinal: 0, prototype_reward_handle: 41, option_handle: 42, date: FIRST, ..Default::default() }]));
        let spelled = [(41, "mod_reward"), (42, "mod_option")].into_iter().filter(|&(token, _)| token != 42 || hour >= SPELLED);
        set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(spelled.map(|(token, literal)| LexicalTokenRow { token, literal: literal.as_bytes().to_vec() }).collect()));
        let read = READS.into_iter().filter(|&at| at <= hour).max().unwrap();
        let mut slices: Vec<SliceReadRow> =
            [TableId::Projects, TableId::ProjectHistory].into_iter().flat_map(|table| [11, 9001].map(|owner| SliceReadRow { table: table.index() as u32, owner, sample_hour: read })).collect();
        slices.sort_by_key(|row| (row.table, row.owner));
        set(&mut frame, TableId::SliceReads, TableRows::SliceReads(slices));
        frame
    };
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, (FIRST..=LAST).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(crate::SourceKey { source_id: "special-project-respelled".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let at = followed_as_direct(&source);
    let option = |hour: i32| at(hour)["projects"][0]["choices"][0]["option"]["token"].clone();
    assert_eq!(option(READS[1] - 1), json!("42"), "the read that listed the choice could not spell it, though a later hour can");
    assert_eq!(option(READS[1]), json!("mod_option"), "the next read of the same rows spells the choice and drops its refusal");
}

#[test]
fn a_chosen_option_sharing_its_token_with_an_option_of_another_name_is_withheld_by_name() {
    use content_reader_core::special_projects::PrototypeOption;
    let option = |token: &str, name: Option<&str>| PrototypeOption { token: token.into(), name: name.map(Into::into), ..Default::default() };
    assert_eq!(option_name(&[option("first", Some("first_name")), option("FIRST", Some("first_name"))], "first"), Ok(Some("first_name".into())));
    assert!(option_name(&[option("first", Some("first_name")), option("first", None)], "first").unwrap_err().contains("another name"));
    assert!(option_name(&[option("other", None)], "first").unwrap_err().contains("absent"));
}

#[test]
fn a_value_with_no_read_in_the_day_or_since_the_holder_took_the_tag_is_not_read_by_name() {
    let source = recording();
    let early = details(&source, 25);
    for roster in ["programs", "breakthroughs", "projects"] {
        assert_eq!(early[roster], Value::Null, "{roster} before any read is unknown, never empty");
    }
    assert!(has(&early, "Program assignments were not read in the day up to this hour"), "{early}");
    assert!(has(&early, "Projects were not read"), "{early}");
    assert_eq!(early["scientists"][0]["injuryDays"], Value::Null);
    assert_eq!(early["scientists"][0]["injuryHour"], Value::Null);
    let alias = details(&source, ALIAS_UNSTAMPED);
    assert_eq!(alias["projects"], Value::Null, "a linked owner with no proven read and no rows is not an owner without projects, as the list holds: {alias}");
    assert!(has(&alias, "Projects were not read"), "{alias}");
    let after = details(&source, UNHELD + 2);
    assert_eq!(after["programs"], Value::Null);
    assert!(has(&after, "Program assignments were last read before this country held the tag"), "{after}");
    assert_eq!(after["projects"], Value::Null, "the projects' read precedes the holder's run too");
}

#[test]
fn the_panel_over_whole_tables_answers_as_the_keyed_request() {
    let source = recording();
    let mut panel = (entry::<SpecialProjectDetailsAt>().panel)(&source, &load(), &json!({ "panel": "special-project-details", "branchId": "1", "country": "GER" })).unwrap();
    for hour in [25, 40, UNHELD + 2, 53] {
        let tables = panel.tables().to_vec();
        let SeekResult::Frame(frame) = source.replay_ref().seek_selected(1, hour, Some(&tables)).unwrap() else { panic!("hour {hour} is recorded") };
        assert_eq!(panel.data(&frame), Ok(details(&source, hour)), "hour {hour}");
    }
}

#[test]
fn effects_without_the_recorded_content_are_refused_by_name() {
    let source = recording();
    let error = asked::<SpecialProjectEffectsAt>(&source, json!({ "branchId": "1", "country": "GER", "owner": 9001, "project": 3, "hour": 40 })).unwrap_err();
    match error {
        dashboard_core::QueryError::Refused { text } => assert!(text.contains("content"), "{text}"),
        dashboard_core::QueryError::Failed { message } => panic!("not a refusal: {message}"),
    }
}

#[test]
fn project_output_is_the_game_text_with_its_links_and_names_what_it_cannot_show() {
    use dashboard_text::effect_description::Description;
    let shown = ProjectEffects::from(Description {
        text: "On research completed:\n\u{11}HUnlocks \u{1}rmod_tech\u{2}Gun Carriages\u{3}\u{11}!\nSub units: \u{1}smod_unit\u{2}Super-Heavy Artillery\u{3}\n".into(),
        refusals: BTreeMap::from([("Scope unavailable".to_owned(), 2)]),
    });
    assert_eq!(
        serde_json::to_value(&shown).unwrap(),
        json!({
            "runs": [
                { "kind": "text", "text": "On research completed:\n", "tone": null },
                { "kind": "text", "text": "Unlocks ", "tone": "H" },
                { "kind": "link", "entity": "technology", "token": "mod_tech", "databaseIndex": null, "text": "Gun Carriages", "tone": "H" },
                { "kind": "text", "text": "\nSub units: ", "tone": null },
                { "kind": "link", "entity": "subUnit", "token": "mod_unit", "databaseIndex": null, "text": "Super-Heavy Artillery", "tone": null },
                { "kind": "text", "text": "\n", "tone": null },
            ],
            "refusals": { "Scope unavailable": 2 },
        })
    );
    let broken = ProjectEffects::from(Description { text: "Unlocks \u{1}rmod_tech\u{2}unclosed".into(), refusals: BTreeMap::new() });
    assert!(broken.runs.is_empty(), "{broken:?}");
    assert_eq!(broken.refusals.values().sum::<usize>(), 1, "an output the links cannot frame is withheld by name, never shown broken: {broken:?}");
}

fn refusal(result: Result<Value, dashboard_core::QueryError>) -> String {
    match result {
        Err(dashboard_core::QueryError::Refused { text }) => text,
        other => panic!("not a refusal: {other:?}"),
    }
}

#[test]
fn an_iteration_is_the_listing_projects_choice_rendered_with_that_projects_definition() {
    const REPLACED: i32 = 35;
    const CHOICE: i32 = 30;
    const OWN_CHOICE: i32 = 24;
    const MARKER: i32 = 40;
    const UNPROVEN: i32 = 45;
    const UNHELD: i32 = 50;
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        if hour == UNHELD {
            *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
        }
        let replaced = hour >= REPLACED;
        set(&mut frame, TableId::Projects, TableRows::Projects(vec![ProjectRow { owner: 9001, local_id: 3, definition_id: if replaced { 78 } else { 77 }, ..Default::default() }]));
        let choice = |ordinal, date| ProjectHistoryRow { owner: 9001, project: 3, ordinal, prototype_reward_handle: 41, option_handle: 42, date, ..Default::default() };
        let history = if replaced {
            let mut rows = vec![choice(0, CHOICE), choice(1, CHOICE)];
            rows.extend((hour >= MARKER).then(|| ProjectHistoryRow { prototype_reward_handle: COMPLETION_MARKER, option_handle: COMPLETION_MARKER, ..choice(2, MARKER) }));
            rows
        } else {
            vec![choice(0, OWN_CHOICE)]
        };
        set(&mut frame, TableId::ProjectHistory, TableRows::ProjectHistory(history));
        set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(vec![
            LexicalTokenRow { token: 41, literal: b"mod_reward".to_vec() },
            LexicalTokenRow { token: 42, literal: b"mod_option".to_vec() },
            LexicalTokenRow { token: 78, literal: b"replacement".to_vec() },
        ]));
        let read = if hour == UNPROVEN { hour - 1 } else { hour };
        let mut slices: Vec<SliceReadRow> =
            [TableId::Projects, TableId::ProjectHistory].into_iter().flat_map(|table| [11, 9001].map(|owner| SliceReadRow { table: table.index() as u32, owner, sample_hour: read })).collect();
        slices.sort_by_key(|row| (row.table, row.owner));
        set(&mut frame, TableId::SliceReads, TableRows::SliceReads(slices));
        frame
    };
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, (FIRST..=LAST).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(crate::SourceKey { source_id: "special-project-iteration-listed".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let iteration = |ordinal: u32, hour: i32, read: i32| {
        asked::<SpecialProjectIterationAt>(&source, json!({ "branchId": "1", "country": "GER", "owner": 9001, "project": 3, "ordinal": ordinal, "hour": hour, "read": read }))
    };
    let unavailable = |ordinal, hour, read| iteration(ordinal, hour, read).unwrap()["unavailable"].as_str().unwrap().to_owned();
    assert!(refusal(iteration(0, FIRST - 1, REPLACED)).contains("hour is not recorded"), "an hour outside the recording is named, never guessed");
    assert!(refusal(iteration(1, CHOICE, REPLACED)).contains("content"), "the replacement's choice at an hour its predecessor held the id is found where it was listed");
    assert!(refusal(iteration(0, CHOICE, REPLACED)).contains("content"), "the predecessor's choice of the same ordinal at another date does not refuse it");
    assert_eq!(unavailable(0, OWN_CHOICE, REPLACED - 1), "lexical tokens lacks token 77", "the predecessor's own choice is rendered with its own definition, never the replacement's");
    assert_eq!(unavailable(0, OWN_CHOICE, REPLACED), "The asked hour is not this iteration's hour", "a listing holding the ordinal at another date is another choice");
    assert_eq!(unavailable(0, CHOICE, CHOICE - 1), "The iteration's choices were listed before its hour");
    assert_eq!(unavailable(2, MARKER, MARKER), "This history entry is the project's completion, not an iteration");
    assert_eq!(unavailable(0, CHOICE, UNPROVEN), "This iteration's choices were not read at the hour that lists them");
    assert_eq!(unavailable(0, CHOICE, UNHELD + 1), "This iteration's choice was listed after this country stopped holding the tag");
}

#[test]
fn effects_withhold_a_project_last_read_before_the_holder_took_the_tag() {
    let source = recording();
    let effects = asked::<SpecialProjectEffectsAt>(&source, json!({ "branchId": "1", "country": "GER", "owner": 9001, "project": 3, "hour": UNHELD + 2 })).unwrap();
    assert_eq!(effects["unavailable"], json!("Projects were not read in the day up to this hour while this country held the tag"), "{effects}");
}

#[test]
fn a_project_read_while_its_raw_owner_belonged_to_another_country_is_not_this_countrys() {
    let source = recording();
    let other = "The project was read while its owner belonged to another country";
    let at = MOVED_HELD.start;
    let effects = asked::<SpecialProjectEffectsAt>(&source, json!({ "branchId": "1", "country": "GER", "owner": MOVED_OWNER, "project": 5, "hour": at })).unwrap();
    assert_eq!(effects["unavailable"], json!(other), "the retained read predates the owner's move to this country: {effects}");
    assert!(has(&details(&source, at), other), "the details list withholds the same read");
    let iteration = asked::<SpecialProjectIterationAt>(&source, json!({ "branchId": "1", "country": "GER", "owner": MOVED_OWNER, "project": 5, "ordinal": 0, "hour": MOVED_ITERATION, "read": MOVED_READS[2] })).unwrap();
    assert_eq!(iteration["unavailable"], json!(other), "the choice was listed after the owner left this country: {iteration}");
}

#[test]
fn an_iteration_listed_by_a_read_in_part_is_withheld_by_name() {
    const CHOICE: i32 = 30;
    const IN_PART: i32 = 31;
    const COMPLETE: i32 = 35;
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        let stamp = |table: TableId, sample_hour| SliceReadRow { table: table.index() as u32, owner: 9001, sample_hour };
        let read = hour >= CHOICE;
        set(&mut frame, TableId::Projects, TableRows::Projects(if read { vec![ProjectRow { owner: 9001, local_id: 3, definition_id: 77, ..Default::default() }] } else { Vec::new() }));
        set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(vec![
            LexicalTokenRow { token: 41, literal: b"mod_reward".to_vec() },
            LexicalTokenRow { token: 42, literal: b"mod_option".to_vec() },
            LexicalTokenRow { token: 77, literal: b"project".to_vec() },
        ]));
        let choice = ProjectHistoryRow { owner: 9001, project: 3, ordinal: 0, prototype_reward_handle: 41, option_handle: 42, date: CHOICE, ..Default::default() };
        *frame.tables[TableId::ProjectHistory.index()] = match hour {
            IN_PART => TableView::Truncated { rows: TableRows::ProjectHistory(vec![choice.clone()]), missing: 1 },
            COMPLETE.. => TableView::Complete(TableRows::ProjectHistory(vec![choice])),
            _ => TableView::Complete(TableRows::empty(TableId::ProjectHistory)),
        };
        let mut slices = if read { vec![stamp(TableId::Projects, CHOICE)] } else { Vec::new() };
        slices.extend(match hour {
            IN_PART => Some(IN_PART),
            COMPLETE.. => Some(COMPLETE),
            _ => None,
        }
        .map(|read| stamp(TableId::ProjectHistory, read)));
        set(&mut frame, TableId::SliceReads, TableRows::SliceReads(slices));
        frame
    };
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, (FIRST..=LAST).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(crate::SourceKey { source_id: "special-project-iteration-in-part".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let iteration = |read: i32| asked::<SpecialProjectIterationAt>(&source, json!({ "branchId": "1", "country": "GER", "owner": 9001, "project": 3, "ordinal": 0, "hour": CHOICE, "read": read }));
    let in_part = iteration(IN_PART).unwrap();
    assert!(in_part["unavailable"].as_str().unwrap().starts_with("Project choice history is unavailable"), "a listing read in part proves no choice: {in_part}");
    assert!(refusal(iteration(COMPLETE)).contains("content"), "the complete listing proves the choice and goes on to the content");
}

#[test]
fn a_replacements_choice_renders_with_the_listings_spelling_and_never_its_predecessors_project_values() {
    use replay_data::rows::{ProjectVariable, ProjectVariableStore};
    engine_executable::executable_tables::initialize_for_tests().expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    const REPLACED: i32 = 35;
    const BEFORE: i32 = 30;
    const OWN: i32 = 38;
    const LISTED: i32 = 40;
    let root = std::env::temp_dir().join(format!("special-project-iteration-{}", uuid::Uuid::new_v4()));
    for (path, text) in [
        ("common/defines/test.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=5}}"),
        ("common/special_projects/specialization/test.txt", "mod_science={}"),
        ("common/special_projects/prototype_rewards/test.txt", "mod_reward={option={token=mod_option iteration_output={country_effects={add_political_power=1 custom_effect_tooltip=PROJECT_VARIABLE}}}}"),
        ("common/special_projects/projects/test.txt", "project={specialization=mod_science generic_prototype_rewards={mod_reward}} replacement={specialization=mod_science generic_prototype_rewards={mod_reward}}"),
        ("localisation/test_l_english.yml", "\u{feff}l_english:\n PROJECT_VARIABLE:0 \"Variable [?FROM.mod_value|2]\"\n POLITICS_ADD_POLITICAL_POWER:0 \"PP$VAL|0$\"\n SPECIAL_PROJECT_REWARDS_HEADER_TEXT:0 \"Outputs\"\n"),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let frame = |hour: i32| {
        let mut frame = source(1, hour, None);
        for table in effect_tables() {
            if table.key_bits() != 0 && matches!(frame.tables[table.index()].held(), TableView::Unavailable { .. }) {
                set(&mut frame, table, TableRows::empty(table));
            }
        }
        let replaced = hour >= REPLACED;
        let variables = ProjectVariableStore { seed: 1, entries: vec![ProjectVariable { name: b"mod_value".to_vec(), value_raw: if replaced { 200_000 } else { 100_000 } }] };
        let runtime = Recorded::Value(ProjectRuntime { variables: Some(variables.into()), ..Default::default() });
        let mut project = ProjectRow { owner: 9001, local_id: 3, definition_id: if replaced { 78 } else { 77 }, runtime, program_reference: 1, started: 1, ..Default::default() };
        project.state.current_offset = 0x18;
        project.state.not_completed = 1;
        set(&mut frame, TableId::Projects, TableRows::Projects(vec![project]));
        let choice = |ordinal, date| ProjectHistoryRow { owner: 9001, project: 3, ordinal, prototype_reward_handle: 41, option_handle: 42, date, ..Default::default() };
        let history = if replaced { [(0, BEFORE), (1, OWN)].into_iter().filter(|&(_, date)| date <= hour).map(|(ordinal, date)| choice(ordinal, date)).collect() } else { vec![choice(0, 24)] };
        set(&mut frame, TableId::ProjectHistory, TableRows::ProjectHistory(history));
        let spelled: Vec<LexicalTokenRow> = if hour == LISTED {
            [(41, "mod_reward"), (42, "mod_option"), (78, "replacement")].map(|(token, literal)| LexicalTokenRow { token, literal: literal.as_bytes().to_vec() }).into()
        } else {
            Vec::new()
        };
        set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(spelled));
        let mut slices: Vec<SliceReadRow> =
            [TableId::Projects, TableId::ProjectHistory].into_iter().flat_map(|table| [11, 9001].map(|owner| SliceReadRow { table: table.index() as u32, owner, sample_hour: hour })).collect();
        slices.sort_by_key(|row| (row.table, row.owner));
        set(&mut frame, TableId::SliceReads, TableRows::SliceReads(slices));
        frame
    };
    for (path, text) in [("hoi4.exe", "test"), ("user/settings.txt", "language=l_english force_pow2_textures=no"), ("user/dlc_load.json", r#"{"enabled_mods":[],"disabled_dlcs":[]}"#)] {
        std::fs::create_dir_all(root.join("user")).unwrap();
        std::fs::write(root.join(path), text).unwrap();
    }
    let (options, identity) = crate::test_fixtures::content_identity(&root);
    let mut input = records(1, (FIRST..=LAST).map(frame).collect());
    let replay::spool::Record::SessionStart(session) = &mut input[0] else { unreachable!() };
    session.identity = identity.clone();
    let path = root.join("synthetic.battleplan");
    crate::test_fixtures::write_with_facts(&path, &options, &identity, input);
    let replay = Replay::open(std::fs::read(&path).unwrap()).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(crate::SourceKey { source_id: "special-project-iteration-rendered".into(), revision: "1".into() }, path, Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let load = LoadContext { content: Some(options) };
    let iteration = |ordinal: u32, hour: i32| {
        let request = json!({ "branchId": "1", "country": "GER", "owner": 9001, "project": 3, "ordinal": ordinal, "hour": hour, "read": LISTED });
        let text = (entry::<SpecialProjectIterationAt>().registered.load)(&source, "lease", request, &load, &AtomicBool::new(false)).unwrap();
        let mut reply: Value = serde_json::from_str(&text).unwrap();
        reply["data"].take()
    };
    let own = iteration(1, OWN);
    assert_eq!(own["unavailable"], Value::Null, "{own}");
    assert!(own["sections"].to_string().contains("Variable 2"), "a choice made by the project the listing names shows its own values: {own}");
    let before = iteration(0, BEFORE);
    assert_eq!(before["unavailable"], Value::Null, "the listing read spells the choice and its project, though no other read does: {before}");
    assert!(!before["sections"].to_string().contains("Variable 1"), "the predecessor's value is never shown as the replacement's: {before}");
    assert!(before["refusals"].as_object().unwrap().keys().any(|why| why.contains("not proven to be this one")), "{before}");
    std::fs::remove_dir_all(&root).ok();
}
