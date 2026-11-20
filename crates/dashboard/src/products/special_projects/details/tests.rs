use super::*;
use crate::{
    products::at_hour::entry,
    test_fixtures::{CAMPAIGN, records, source},
};
use replay::campaign::SeekResult;
use replay_data::rows::{
    CharacterRoleTraitRow, LexicalTokenRow, ProjectBreakthroughRow, ProjectHistoryRow, ProjectProgramRow, ProjectScientistAssignment, ScientistRow,
    ScientistSkillRow, SliceReadRow, TableRows,
};
use replay_frame::frame::TableView;
use serde_json::{Value, json};
use std::sync::atomic::AtomicBool;

const FIRST: i32 = 20;
const LAST: i32 = 55;
/// The daily tables' read hours (their slot of the day).
const DAILY_READS: [i32; 2] = [28, 52];
/// The hour GER's projects were last read, as SliceReads names it.
const PROJECTS_READ: i32 = 30;
/// No country holds GER at this hour.
const UNHELD: i32 = 45;

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    *frame.tables[table.index()] = TableView::Complete(rows);
}

/// GER (raw ids 9001 and 11) with one scientist, one facility and its specialization points read
/// daily, and one project of an unspelled definition read from `PROJECTS_READ`; a scientist of
/// another country beside it.
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
    let read = hour >= PROJECTS_READ;
    set(&mut frame, TableId::Projects, TableRows::Projects(if read { vec![ProjectRow { owner: 9001, local_id: 3, definition_id: 77, ..Default::default() }] } else { Vec::new() }));
    set(&mut frame, TableId::ProjectHistory, TableRows::ProjectHistory(if read {
        vec![
            ProjectHistoryRow { owner: 9001, project: 3, ordinal: 0, prototype_reward_handle: 41, option_handle: 42, date: PROJECTS_READ, ..Default::default() },
            ProjectHistoryRow { owner: 9001, project: 3, ordinal: 1, prototype_reward_handle: COMPLETION_MARKER, option_handle: COMPLETION_MARKER, date: PROJECTS_READ, ..Default::default() },
        ]
    } else {
        Vec::new()
    }));
    let projects = TableId::Projects.index() as u32;
    set(&mut frame, TableId::SliceReads, TableRows::SliceReads(if read { vec![SliceReadRow { table: projects, owner: 9001, sample_hour: PROJECTS_READ }] } else { Vec::new() }));
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
    assert_eq!((project["readHour"].clone(), project["label"]["token"].clone(), project["label"]["name"].clone()), (json!(PROJECTS_READ), json!("77"), Value::Null));
    assert!(has(&reply, "77"), "an unspelled definition keeps its project, named as missing: {reply}");
    assert_eq!(project["choices"].as_array().unwrap().len(), 1, "the completion marker is not a chosen option");
    assert_eq!(details(&source, 53)["programs"][0]["readHour"], json!(52), "the next day's read supplies the next day");
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
