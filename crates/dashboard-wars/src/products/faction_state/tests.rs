use super::{country_goals, details, history};
use dashboard_follow::live::{CellValue, Cells};
use replay_data::rows::{
    CountryLinkRow, FactionGoalEntryRow, FactionGoalStatusRow, FactionManpowerRow, FactionMemberRow, LexicalTokenRow, ReservedCountryNamingRow,
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
    details::goal_lists(&frame, FACTION, None, None, &mut reply, &cells, None).unwrap();
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
    details::goal_lists(&goal_frame(vec![goal(FACTION, 0, 0, Some(2), 0), goal(FACTION, 0, 2, Some(1), 0)]), FACTION, None, None, &mut reply, &cells, None).unwrap();
    assert_eq!(reply.goal_reason.as_deref(), Some("Faction goal ordinals are not contiguous"));
    assert!(reply.goals.is_empty() && reply.definitions.is_empty());
    assert!(cells.finish().is_empty());
    let mut missing = goal_frame(vec![]);
    set(&mut missing, TableId::FactionGoalStatus, TableRows::FactionGoalStatus(vec![FactionGoalStatusRow { faction: OTHER, manifest: None }]));
    let mut reply = details::blank(&params());
    details::goal_lists(&missing, FACTION, None, None, &mut reply, &Cells::default(), None).unwrap();
    assert_eq!(reply.goal_reason.as_deref(), Some("Faction goal status is absent"));
    let mut unread = goal_frame(vec![]);
    *unread.tables[TableId::FactionGoalEntries.index()] = TableView::Unavailable { reason: "writer not observed".into() };
    let mut reply = details::blank(&params());
    details::goal_lists(&unread, FACTION, None, None, &mut reply, &Cells::default(), None).unwrap();
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
fn a_country_s_faction_is_the_one_listing_any_country_linked_to_its_slot() {
    let links = [CountryLinkRow { id: 7, canonical_slot: 3 }, CountryLinkRow { id: 8, canonical_slot: 3 }, CountryLinkRow { id: 9, canonical_slot: 4 }];
    let members = [FactionMemberRow { country: 9, faction: OTHER, ordinal: 0 }, FactionMemberRow { country: 8, faction: FACTION, ordinal: 1 }];
    assert_eq!(country_goals::faction_of(&members, &links, 3), Some(FACTION));
    assert_eq!(country_goals::faction_of(&members, &links, 4), Some(OTHER));
    assert_eq!(country_goals::faction_of(&members, &links, 5), None, "a slot no member links to belongs to no faction");
}

#[test]
fn faction_goals_resolve_name_and_follow_only_the_goals_the_filter_shows_and_an_unknown_state_is_never_in_progress() {
    use super::country_goals::{GoalFilter, GoalState, shown_goals};
    let with = |status: i32, row: FactionGoalEntryRow| FactionGoalEntryRow { status, ..row };
    let frame = goal_frame(vec![
        goal(FACTION, 0, 0, Some(2), 25_000),
        with(9, goal(FACTION, 0, 1, Some(1), 50_000)),
        with(3, goal(FACTION, 0, 2, Some(2), 75_000)),
        goal(FACTION, 1, 0, Some(3), 0),
    ]);
    let listed = |filter: GoalFilter| {
        let mut reply = details::blank(&params());
        let cells = Cells::default();
        details::goal_lists(&frame, FACTION, None, None, &mut reply, &cells, Some(filter)).unwrap();
        let live = cells.finish();
        let rows = if live.is_empty() { serde_json::json!([]) } else { live.json(&live.values(&frame).into_iter().map(dashboard_follow::live::GroupState::Values).collect::<Vec<_>>())["rows"].clone() };
        (reply, rows)
    };
    let (progress, rows) = listed(GoalFilter::InProgress);
    assert_eq!(progress.manifest, None, "the manifest is not resolved for a list of goals");
    let tokens: Vec<Option<&str>> = progress.definitions.iter().map(|definition| definition.token.as_deref()).collect();
    assert_eq!(tokens, [Some("mod_goal")], "a hidden goal's definition is never resolved");
    assert_eq!(rows, serde_json::json!(["goal:0", "goal:2"]), "a hidden goal registers no progress cell");
    assert!(progress.warnings.iter().any(|warning| warning.starts_with("1 faction goals are in a state")));
    let (goals, definitions, _) = shown_goals(progress);
    let shown: Vec<_> = goals.iter().map(|goal| (goal.key.as_str(), definitions[goal.definition as usize].token.as_deref(), goal.state)).collect();
    assert_eq!(shown, [("goal:0", Some("mod_goal"), GoalState::InProgress), ("goal:2", Some("mod_goal"), GoalState::Completing)]);
    let (all, _) = listed(GoalFilter::All);
    let (all, _, _) = shown_goals(all);
    assert_eq!(all.iter().find(|goal| goal.key == "goal:1").map(|goal| (goal.state, goal.unrecognised_status)), Some((GoalState::Unrecognised, Some(9))));
    let (done, rows) = listed(GoalFilter::Completed);
    assert_eq!(rows, serde_json::json!([]));
    let (done, done_definitions, _) = shown_goals(done);
    assert_eq!((done.len(), done[0].ordinal, done_definitions[0].token.as_deref()), (1, None, Some("mod_done")));
}

#[test]
fn faction_pools_orders_resources_tracks_quiet_hours_and_resolves_slot_membership() {
    use super::pools::FactionPools;
    use dashboard_base::{
        Source, SourceKey,
        products::{LoadContext, at_hour::exact_panel},
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{
        CountryRow, FactionDoctrineRow, FactionResearchRow, FactionResourceRow, FactionRow, FactionUnlockRow, ResourceDefinitionRow,
    };
    use serde_json::json;
    use std::sync::{Arc, atomic::AtomicBool};

    let frames: Vec<DecodedFrame> = (100..=105)
        .map(|hour| {
            let mut frame = source(1, hour, None);
            let tag = if hour >= 104 { b"BBB".to_vec() } else { b"AAA".to_vec() };
            set(
                &mut frame,
                TableId::Countries,
                TableRows::Countries(vec![
                    CountryRow { id: 7, canonical_slot: 3, tag, ..Default::default() },
                    CountryRow { id: 8, canonical_slot: 4, tag: b"CCC".to_vec(), ..Default::default() },
                ]),
            );
            set(
                &mut frame,
                TableId::CountryLinks,
                TableRows::CountryLinks(vec![
                    CountryLinkRow { id: 7, canonical_slot: 3 },
                    CountryLinkRow { id: 8, canonical_slot: 4 },
                ]),
            );
            set(
                &mut frame,
                TableId::Factions,
                TableRows::Factions(vec![FactionRow { id: FACTION, ..Default::default() }]),
            );
            set(
                &mut frame,
                TableId::FactionMembers,
                TableRows::FactionMembers(vec![
                    FactionMemberRow { country: 7, faction: FACTION, ordinal: 0 },
                    FactionMemberRow { country: 8, faction: FACTION, ordinal: 1 },
                ]),
            );
            set(
                &mut frame,
                TableId::ResourceDefinitions,
                TableRows::ResourceDefinitions(vec![
                    ResourceDefinitionRow { id: 1, token: 10, active: 1, factory_cost: 0 },
                    ResourceDefinitionRow { id: 2, token: 11, active: 1, factory_cost: 0 },
                    ResourceDefinitionRow { id: 3, token: 12, active: 1, factory_cost: 0 },
                ]),
            );
            set(
                &mut frame,
                TableId::LexicalTokens,
                TableRows::LexicalTokens(vec![
                    LexicalTokenRow { token: 10, literal: b"oil".to_vec() },
                    LexicalTokenRow { token: 11, literal: b"steel".to_vec() },
                    LexicalTokenRow { token: 12, literal: b"rubber".to_vec() },
                    LexicalTokenRow { token: 20, literal: b"land_doctrine".to_vec() },
                ]),
            );
            if hour == 105 {
                *frame.tables[TableId::FactionResources.index()] = TableView::Unavailable { reason: "resources unread".into() };
            } else {
                let steel = if hour >= 103 { 500_000 } else { 200_000 };
                set(
                    &mut frame,
                    TableId::FactionResources,
                    TableRows::FactionResources(vec![
                        FactionResourceRow { faction: FACTION, ordinal: 0, token: 0, raw: 0 },
                        FactionResourceRow { faction: FACTION, ordinal: 1, token: 10, raw: 200_000 },
                        FactionResourceRow { faction: FACTION, ordinal: 2, token: 11, raw: steel },
                        FactionResourceRow { faction: FACTION, ordinal: 3, token: 12, raw: 90_000 },
                    ]),
                );
            }
            set(
                &mut frame,
                TableId::FactionUnlocks,
                TableRows::FactionUnlocks(vec![FactionUnlockRow { faction: FACTION, manpower: u8::from(hour >= 102), research: 1 }]),
            );
            set(
                &mut frame,
                TableId::FactionManpower,
                TableRows::FactionManpower(vec![
                    FactionManpowerRow { faction: FACTION, ordinal: 0, country: 8, value: 150 },
                    FactionManpowerRow { faction: FACTION, ordinal: 1, country: 7, value: 400 },
                ]),
            );
            set(
                &mut frame,
                TableId::FactionResearch,
                TableRows::FactionResearch(vec![FactionResearchRow { faction: FACTION, ordinal: 0, scientist_base_raw: 120_000, bonus_raw: 0, countries: vec![7, 8] }]),
            );
            set(
                &mut frame,
                TableId::FactionDoctrine,
                TableRows::FactionDoctrine(vec![FactionDoctrineRow { faction: FACTION, ordinal: 0, token: 20 }]),
            );
            set(
                &mut frame,
                TableId::ReservedCountryNaming,
                TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow { tag: b"RSV".to_vec(), ..Default::default() }]),
            );
            frame
        })
        .collect();

    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(SourceKey { source_id: "wars".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let load = LoadContext { content: None };
    let params = json!({ "branchId": "1", "country": 3, "pool": "resources" });
    let panel = exact_panel::<FactionPools>(&source, &load, &params).unwrap();
    let exact = panel.exact().unwrap();
    let h100 = (exact.advance)(100, true, &false.into(), None);
    assert!(h100.fresh);
    let r100 = h100.value.as_ref().as_ref().unwrap();
    assert_eq!(r100["status"], "ok");
    assert_eq!(r100["decimals"], 5);
    let ordinals: Vec<u64> = r100["resources"].as_array().unwrap().iter().map(|r| r["ordinal"].as_u64().unwrap()).collect();
    assert_eq!(ordinals, [1, 2, 3], "equal amounts tie-break by ordinal, skipping zero sentinel ordinal 0");
    let h101 = (exact.advance)(101, false, &false.into(), None);
    let h102 = (exact.advance)(102, false, &false.into(), None);
    assert!(!h101.fresh && !h102.fresh, "quiet hours with unchanged resource inputs do not recompute");
    let h103 = (exact.advance)(103, false, &false.into(), None);
    assert!(h103.fresh);
    let r103 = h103.value.as_ref().as_ref().unwrap();
    let ordinals103: Vec<u64> = r103["resources"].as_array().unwrap().iter().map(|r| r["ordinal"].as_u64().unwrap()).collect();
    assert_eq!(ordinals103, [2, 1, 3], "steel rises to the top once its amount exceeds oil");
    let h104 = (exact.advance)(104, false, &false.into(), None);
    assert_eq!(h104.value.as_ref().as_ref(), (exact.cold)(104, &false.into()).as_ref(), "slot lookup holds across AAA -> BBB tag change");
    let h105 = (exact.advance)(105, false, &false.into(), None);
    let r105 = h105.value.as_ref().as_ref().unwrap();
    assert_eq!(r105["status"], "poolNotRecorded");
    assert!(r105["reason"].as_str().unwrap().contains("resources unread"));

    let mp_panel = exact_panel::<FactionPools>(&source, &load, &json!({ "branchId": "1", "country": 3, "pool": "manpower" })).unwrap();
    let mp_exact = mp_panel.exact().unwrap();
    assert_eq!((mp_exact.cold)(101, &false.into()).unwrap()["status"], "poolLocked");
    let mp102 = (mp_exact.cold)(102, &false.into()).unwrap();
    assert_eq!(mp102["status"], "ok");
    assert_eq!(mp102["manpowerTotal"], "550");
    assert_eq!(mp102["manpower"][0]["slot"], 3);
    assert_eq!(mp102["manpower"][0]["amount"], "400");
    assert_eq!(mp102["manpower"][1]["slot"], 4);

    let res_panel = exact_panel::<FactionPools>(&source, &load, &json!({ "branchId": "1", "faction": FACTION.to_string(), "pool": "research" })).unwrap();
    let res = (res_panel.exact().unwrap().cold)(100, &false.into()).unwrap();
    assert_eq!(res["status"], "ok");
    assert_eq!(res["research"]["scientistBase"], "120000");
    assert_eq!(res["research"]["members"].as_array().unwrap().len(), 2);

    let absent_panel = exact_panel::<FactionPools>(&source, &load, &json!({ "branchId": "1", "faction": OTHER.to_string(), "pool": "doctrine" })).unwrap();
    assert_eq!((absent_panel.exact().unwrap().cold)(100, &false.into()).unwrap()["status"], "factionAbsent");
}

#[test]
fn an_unlinked_roster_entry_leaves_membership_unknown_and_an_unknown_unlock_is_never_an_empty_pool() {
    use super::pools::{FactionPoolKind, FactionPoolStatus, blank, build_manpower, pooled_faction};
    let members = [FactionMemberRow { country: 9, faction: FACTION, ordinal: 0 }];
    let links = [CountryLinkRow { id: 7, canonical_slot: 3 }];
    assert!(matches!(pooled_faction(&members, &links, 3), Err((FactionPoolStatus::MembershipNotRecorded, _))), "the unlinked entry could be slot 3's membership");
    let linked = [CountryLinkRow { id: 7, canonical_slot: 3 }, CountryLinkRow { id: 9, canonical_slot: 4 }];
    assert_eq!(pooled_faction(&members, &linked, 3), Ok(None), "every entry resolves to another country: slot 3 is in no faction");
    assert_eq!(pooled_faction(&members, &linked, 4), Ok(Some(FACTION)));

    let mut unread = frame();
    set(&mut unread, TableId::FactionManpower, TableRows::FactionManpower(Vec::new()));
    *unread.tables[TableId::FactionUnlocks.index()] = TableView::Unavailable { reason: "unlocks unread".into() };
    let mut reply = blank(FactionPoolKind::Manpower, FactionPoolStatus::Ok, None, Some(FACTION));
    build_manpower(&unread, FACTION, &mut reply);
    assert_eq!(reply.status, FactionPoolStatus::PoolNotRecorded, "a locked pool and an unlocked empty one look alike without the unlock");
    assert!(reply.manpower.is_empty() && reply.manpower_total.is_none());
}
