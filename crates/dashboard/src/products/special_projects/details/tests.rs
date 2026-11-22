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
const NEXT_DAY_ITERATION: i32 = 31;
const UNHELD_NEXT_ITERATION: i32 = 42;
const MOVED_OWNER: u32 = 9002;
const MOVED_READS: [i32; 3] = [47, 50, 53];
const MOVED_HELD: std::ops::Range<i32> = 49..53;
const MOVED_ITERATION: i32 = 51;
const HISTORY_UNSTAMPED: i32 = 35;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, None);
    if hour == UNHELD {
        *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "Synthetic identity refusal".into() };
    }
    set(&mut frame, TableId::Scientists, TableRows::Scientists(vec![
        ScientistRow { character: 7, country: 11, active: Recorded::Value(Some(1)), ..Default::default() },
        ScientistRow { character: 8, country: 12, active: Recorded::Value(Some(0)), ..Default::default() },
    ]));
    set(&mut frame, TableId::ScientistDaily, TableRows::ScientistDaily(vec![ScientistDailyRow { character: 7, country: 11, injury_days: Recorded::Value(Some(8)), template_visibility: Recorded::Value(Some(0)), ..Default::default() }]));
    set(&mut frame, TableId::ScientistSkills, TableRows::ScientistSkills(vec![ScientistSkillRow { character: 7, ordinal: 0, token: 41, level: -2, experience_raw: i64::MAX }]));
    set(&mut frame, TableId::CharacterRoleTraits, TableRows::CharacterRoleTraits(vec![CharacterRoleTraitRow { character: 7, kind: CharacterRoleTraitRow::SCIENTIST, tokens: vec![42, 42], ..Default::default() }]));
    set(&mut frame, TableId::LexicalTokens, TableRows::LexicalTokens(vec![LexicalTokenRow { token: 41, literal: b"mod_skill".to_vec() }, LexicalTokenRow { token: 42, literal: b"mod_trait".to_vec() }]));
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
    let history = |ordinal, date, first: i32| read.filter(|&read| read >= first).map(|_| ProjectHistoryRow { owner: 9001, project: 3, ordinal, prototype_reward_handle: 41, option_handle: 42, date, ..Default::default() });
    set(&mut frame, TableId::Projects, TableRows::Projects(
        [read.map(|_| ProjectRow { owner: 9001, local_id: 3, definition_id: 77, ..Default::default() }), moved.map(|_| ProjectRow { owner: MOVED_OWNER, local_id: 5, definition_id: 77, ..Default::default() })]
            .into_iter()
            .flatten()
            .collect(),
    ));
    set(&mut frame, TableId::ProjectHistory, TableRows::ProjectHistory(
        [
            history(0, PROJECT_READS[0], PROJECT_READS[0]),
            history(1, PROJECT_READS[0], PROJECT_READS[0]).map(|row| ProjectHistoryRow { prototype_reward_handle: COMPLETION_MARKER, option_handle: COMPLETION_MARKER, ..row }),
            history(2, LATE_ITERATION, PROJECT_READS[2]),
            history(3, NEXT_DAY_ITERATION, PROJECT_READS[1]),
            history(4, UNHELD_NEXT_ITERATION, PROJECT_READS[2]),
            moved.filter(|&read| read >= MOVED_READS[2]).map(|_| ProjectHistoryRow { owner: MOVED_OWNER, project: 5, ordinal: 0, prototype_reward_handle: 41, option_handle: 42, date: MOVED_ITERATION, ..Default::default() }),
        ]
        .into_iter()
        .flatten()
        .collect(),
    ));
    let mut slices: Vec<SliceReadRow> = [(9001, read), (MOVED_OWNER, moved)]
        .into_iter()
        .filter_map(|(owner, read)| read.map(|sample_hour| [TableId::Projects, TableId::ProjectHistory].map(|table| SliceReadRow { table: table.index() as u32, owner, sample_hour })))
        .flatten()
        .filter(|row| hour != HISTORY_UNSTAMPED || row.table != TableId::ProjectHistory.index() as u32)
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
    assert_eq!(project["choices"].as_array().unwrap().len(), 2, "the completion marker is not a chosen option");
    assert_eq!(details(&source, 53)["programs"][0]["readHour"], json!(52), "the next day's read supplies the next day");
    let unstamped = details(&source, HISTORY_UNSTAMPED);
    assert_eq!(unstamped["projects"][0]["choices"], Value::Null, "a complete history table with no proven read is not an empty one: {unstamped}");
    assert!(has(&unstamped, "Project choice history was not read"), "{unstamped}");
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

#[test]
fn an_iteration_is_read_at_its_own_hour_and_names_what_was_not_recorded() {
    let source = recording();
    let iteration = |ordinal: u32, hour: i32| asked::<SpecialProjectIterationAt>(&source, json!({ "branchId": "1", "country": "GER", "owner": 9001, "project": 3, "ordinal": ordinal, "hour": hour }));
    let refused = |result: Result<Value, dashboard_core::QueryError>| match result {
        Err(dashboard_core::QueryError::Refused { text }) => text,
        other => panic!("not a refusal: {other:?}"),
    };
    assert!(refused(iteration(0, FIRST - 1)).contains("hour is not recorded"), "an hour outside the recording is named, never guessed");
    let unavailable = |ordinal, hour| iteration(ordinal, hour).unwrap()["unavailable"].as_str().unwrap().to_owned();
    let first = PROJECT_READS[0];
    assert_eq!(unavailable(0, first + 1), "The asked hour is not this iteration's hour");
    assert_eq!(unavailable(1, first), "This history entry is the project's completion, not an iteration");
    assert_eq!(unavailable(9, NEXT_DAY_ITERATION), "This iteration's choice was not read within a day of its hour");
    assert!(refused(iteration(0, first)).contains("content"), "an iteration read at its hour goes on to the recorded content");
    assert!(refused(iteration(3, NEXT_DAY_ITERATION)).contains("content"), "a choice first read later in its day, under the same holder, is found for its own hour");
    assert_eq!(
        unavailable(4, UNHELD_NEXT_ITERATION),
        "This iteration's choice was next read after this country stopped holding the tag",
        "a later read is a reading of this country's history only while it held the tag throughout"
    );
    assert_eq!(
        unavailable(2, LATE_ITERATION),
        "Projects were not read in the day up to this hour while this country held the tag",
        "the project state retained from before the holder's run is not this country's"
    );
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
    let iteration = asked::<SpecialProjectIterationAt>(&source, json!({ "branchId": "1", "country": "GER", "owner": MOVED_OWNER, "project": 5, "ordinal": 0, "hour": MOVED_ITERATION })).unwrap();
    assert_eq!(iteration["unavailable"], json!(other), "the choice was next read after the owner left this country: {iteration}");
}
