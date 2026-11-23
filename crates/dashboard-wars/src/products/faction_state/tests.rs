use super::{details, history, rewards};
use dashboard_follow::live::{CellValue, Cells};
use replay_data::rows::{
    CountryCounterInputRow, CountryLinkRow, CountryRow, FactionGoalEntryRow, FactionGoalStatusRow, FactionManpowerRow, FactionMemberRow, LexicalTokenRow, ReservedCountryNamingRow,
    TableId, TableRows,
};
use replay_frame::frame::{DecodedFrame, Metrics, TableView, not_selected};
use std::collections::HashMap;

const FACTION: u64 = 12_884_901_976;
const OTHER: u64 = 4_294_967_384;

fn frame() -> DecodedFrame {
    DecodedFrame { branch: 1, hour: 100, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() }
}

fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn goal(faction: u64, kind: u8, ordinal: u32, token: Option<u32>, progress_raw: i64) -> FactionGoalEntryRow {
    FactionGoalEntryRow { faction, kind, ordinal, token, status: 0, progress_raw, active_sections: vec![], date: 0, slots: 0 }
}

fn goal_frame(entries: Vec<FactionGoalEntryRow>) -> DecodedFrame {
    let mut frame = frame();
    set(&mut frame, TableId::FactionGoalStatus, TableRows::FactionGoalStatus(vec![FactionGoalStatusRow { faction: OTHER, manifest: None }, FactionGoalStatusRow { faction: FACTION, manifest: Some(1) }]));
    set(&mut frame, TableId::FactionGoalEntries, TableRows::FactionGoalEntries(entries));
    set(
        &mut frame,
        TableId::LexicalTokens,
        TableRows::LexicalTokens(["mod_manifest", "mod_goal", "mod_done"].into_iter().zip(1..).map(|(literal, token)| LexicalTokenRow { token, literal: literal.as_bytes().to_vec() }).collect()),
    );
    frame
}

fn params() -> details::FactionDetailParams {
    details::FactionDetailParams { branch_id: "1".into(), faction: FACTION.to_string(), viewpoint: None }
}

#[test]
fn a_factions_goal_lists_name_each_goal_once_and_follow_progress_by_its_own_rows() {
    let frame = goal_frame(vec![
        goal(OTHER, 0, 0, Some(2), 7),
        goal(FACTION, 0, 0, Some(2), 25_000),
        goal(FACTION, 0, 1, Some(1), 50_000),
        goal(FACTION, 1, 0, Some(3), 0),
        goal(FACTION, 1, 1, None, 0),
        goal(FACTION, 2, 0, Some(2), 0),
        goal(FACTION, 3, 0, None, 0),
    ]);
    let mut reply = details::blank(&params());
    let cells = Cells::default();
    details::goal_lists(&frame, FACTION, None, None, &mut reply, &cells).unwrap();
    assert_eq!(reply.goal_reason, None);
    let tokens: Vec<Option<&str>> = reply.definitions.iter().map(|definition| definition.token.as_deref()).collect();
    assert_eq!(tokens, [Some("mod_manifest"), Some("mod_goal"), Some("mod_done"), None], "the manifest first, then each goal once");
    assert_eq!(reply.manifest, Some(0));
    assert_eq!(reply.goals.iter().map(|goal| (goal.ordinal, goal.definition)).collect::<Vec<_>>(), [(0, 1), (1, 0)]);
    assert_eq!(reply.completed, [2, 3]);
    assert_eq!(reply.cancelled, [1], "a cancelled goal shares the held goal's definition");
    assert!(reply.definitions[1].reason.as_deref().is_some_and(|why| why.contains("recorded game folders")), "no content: named by token, why said");
    let live = cells.finish();
    let json = live.json(&live.values(&frame).into_iter().map(dashboard_follow::live::GroupState::Values).collect::<Vec<_>>());
    assert_eq!(json["rows"], serde_json::json!(["goal:0", "goal:1"]));
    let key = |ordinal: u32| (u128::from(FACTION) << 40 | u128::from(ordinal)).to_string();
    assert_eq!(json["groups"][0]["keys"], serde_json::json!([key(0), key(1)]));
    assert_eq!(live.values(&frame)[0], [CellValue::Number(25_000), CellValue::Number(50_000)]);
}

#[test]
fn goal_lists_that_skip_an_ordinal_or_lack_their_status_are_not_known() {
    let mut reply = details::blank(&params());
    let cells = Cells::default();
    details::goal_lists(&goal_frame(vec![goal(FACTION, 0, 0, Some(2), 0), goal(FACTION, 0, 2, Some(1), 0)]), FACTION, None, None, &mut reply, &cells).unwrap();
    assert_eq!(reply.goal_reason.as_deref(), Some("Faction goal ordinals are not contiguous"));
    assert!(reply.goals.is_empty() && reply.definitions.is_empty());
    assert!(cells.finish().is_empty());
    let mut missing = goal_frame(vec![]);
    set(&mut missing, TableId::FactionGoalStatus, TableRows::FactionGoalStatus(vec![FactionGoalStatusRow { faction: OTHER, manifest: None }]));
    let mut reply = details::blank(&params());
    details::goal_lists(&missing, FACTION, None, None, &mut reply, &Cells::default()).unwrap();
    assert_eq!(reply.goal_reason.as_deref(), Some("Faction goal status is absent"));
    let mut unread = goal_frame(vec![]);
    *unread.tables[TableId::FactionGoalEntries.index()] = TableView::Unavailable { reason: "writer not observed".into() };
    let mut reply = details::blank(&params());
    details::goal_lists(&unread, FACTION, None, None, &mut reply, &Cells::default()).unwrap();
    assert!(reply.goal_reason.unwrap().contains("writer not observed"));
}

#[test]
fn manpower_contributions_are_each_occurrence_named_by_the_hours_holder_and_summed() {
    let mut frame = frame();
    let row = |ordinal, country, value| FactionManpowerRow { faction: FACTION, ordinal, country, value };
    set(&mut frame, TableId::FactionManpower, TableRows::FactionManpower(vec![row(0, 7, 1000), row(1, 0, 250), row(2, 7, 30), row(3, 99, 5)]));
    set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: 7, canonical_slot: 3 }]));
    set(&mut frame, TableId::ReservedCountryNaming, TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow { tag: b"RSV".to_vec(), ..Default::default() }]));
    let slots = HashMap::from([(3u32, ("AAA", 7u32))]);
    let mut reply = details::blank(&params());
    let cells = Cells::default();
    details::manpower(&frame, FACTION, &details::Tags::new(&frame, slots), &mut reply, &cells).unwrap();
    let tags: Vec<Option<&str>> = reply.manpower.as_ref().unwrap().iter().map(|entry| entry.tag.as_deref()).collect();
    assert_eq!(tags, [Some("AAA"), Some("RSV"), Some("AAA"), None]);
    assert!(reply.warnings.iter().any(|warning| warning.contains("Country 99 has no link")));
    let live = cells.finish();
    let values = live.values(&frame);
    assert_eq!(values[0], [1000, 250, 30, 5].map(CellValue::Number));
    let json = live.json(&values.into_iter().map(dashboard_follow::live::GroupState::Values).collect::<Vec<_>>());
    assert_eq!(json["rows"], serde_json::json!(["manpower:0", "manpower:1", "manpower:2", "manpower:3", details::FACTION_ROW]));
    assert!(json["kinds"].as_array().unwrap().iter().any(|kind| kind["op"] == "sum"), "the total is the sum of every occurrence");
    set(&mut frame, TableId::FactionManpower, TableRows::FactionManpower(vec![row(0, 7, 1000), row(2, 7, 30)]));
    let mut reply = details::blank(&params());
    let cells = Cells::default();
    details::manpower(&frame, FACTION, &details::Tags::new(&frame, HashMap::new()), &mut reply, &cells).unwrap();
    assert_eq!(reply.manpower, None);
    assert!(cells.finish().is_empty());
}

#[test]
fn a_lane_joins_a_held_selection_and_starts_a_segment_at_a_break_or_a_gap() {
    let mut lane = history::Lane::default();
    for (first, last, value, starts) in [(10, 19, 5, true), (20, 29, 5, false), (30, 39, 6, false), (40, 49, 6, true), (60, 69, 6, false)] {
        lane.push(first, last, value, starts);
    }
    assert_eq!(lane.runs, [(10, 29, 5, true), (30, 39, 6, false), (40, 49, 6, true), (60, 69, 6, true)]);
}

#[test]
fn a_goals_completion_reward_is_described_for_the_factions_leader_and_said_missing_by_name() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let (cat, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        ("common/defines/test.lua", "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}"),
        (
            "common/factions/goals/test.txt",
            "mod_goal={ category=short_term complete_effect={ add_political_power=2 } }\nmod_quiet={ category=short_term }\n\
             mod_oil={ category=short_term complete_effect={ every_faction_member={ limit={ has_resources_in_country={ resource=oil amount=1 } } add_political_power=1 } } }\n",
        ),
        ("localisation/test_l_english.yml", "\u{feff}l_english:\n POLITICS_ADD_POLITICAL_POWER:0 \"PP$VAL|0$\"\n"),
    ]);
    let cat = dashboard_base::catalogue::tests::with_effect_describer(cat, dashboard_effects::effect_describer);
    cat.scripted_content().unwrap().set_synthetic_environment(Default::default());
    let mut frame = frame();
    for table in rewards::tables() {
        set(&mut frame, table, TableRows::empty(table));
    }
    set(
        &mut frame,
        TableId::FactionMembers,
        TableRows::FactionMembers(vec![
            FactionMemberRow { country: 7, faction: FACTION, ordinal: 0 },
            FactionMemberRow { country: 8, faction: FACTION, ordinal: 1 },
            FactionMemberRow { country: 9, faction: OTHER, ordinal: 1 },
        ]),
    );
    set(&mut frame, TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: 7, canonical_slot: 3 }, CountryLinkRow { id: 8, canonical_slot: 4 }]));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow { id, canonical_slot, tag: tag.to_vec(), ..Default::default() };
    set(&mut frame, TableId::Countries, TableRows::Countries(vec![country(7, 3, b"AAA"), country(8, 4, b"BBB")]));
    let owns_a_state = |country| CountryCounterInputRow { country, owned_state_count: 1, ..Default::default() };
    set(&mut frame, TableId::CountryCounterInputs, TableRows::CountryCounterInputs(vec![owns_a_state(7), owns_a_state(8)]));

    let reply = rewards::rewards(&frame, FACTION, "mod_goal", &cat).unwrap();
    assert_eq!(reply.reason, None, "{:?}", reply.warnings);
    assert!(reply.text.as_deref().is_some_and(|text| text.contains("PP2")), "{:?} {:?}", reply.text, reply.warnings);

    let quiet = rewards::rewards(&frame, FACTION, "mod_quiet", &cat).unwrap();
    assert_eq!((quiet.text, quiet.reason), (None, None), "no completion effect: nothing to describe, nothing missing");

    let leaderless = rewards::rewards(&frame, OTHER, "mod_goal", &cat).unwrap();
    assert_eq!(leaderless.text, None);
    assert_eq!(leaderless.reason.as_deref(), Some("The faction has no leader at this hour"));

    let oil = rewards::rewards(&frame, FACTION, "mod_oil", &cat).unwrap();
    assert!(
        oil.text.is_some() || oil.reason.as_deref().is_some_and(|reason| reason.contains("has_resources_in_country")),
        "a reward with nothing describable says why, never nothing: {:?} {:?}",
        oil.reason,
        oil.warnings
    );

    let gone = rewards::rewards(&frame, FACTION, "gone_goal", &cat).unwrap_err();
    assert_eq!(dashboard_core::refusal(&gone), Some("Faction goal \"gone_goal\" is absent from recorded content"));
}
