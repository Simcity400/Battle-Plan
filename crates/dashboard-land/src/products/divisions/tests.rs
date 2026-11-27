use super::{
    equipment::short_count,
    icon_key,
    templates::slot_side_range,
};
use replay_data::rows::Recorded;

#[test]
fn a_template_icon_is_its_custom_sprite_when_declared_else_its_priority_units() {
    let declared = |names: &'static [&'static str]| move |key: &str| Ok::<_, String>(names.contains(&key));
    assert_eq!(icon_key(7, 3, b"infantry", declared(&["GFX_div_templ_3_large", "GFX_unit_infantry_icon_medium"])), Ok("GFX_div_templ_3_large".to_owned()));
    assert_eq!(icon_key(7, 3, b"infantry", declared(&["GFX_unit_infantry_icon_medium"])), Ok("GFX_unit_infantry_icon_medium".to_owned()));
    assert_eq!(icon_key(7, -1, b"infantry", declared(&["GFX_div_templ_-1_large", "GFX_unit_infantry_icon_medium"])), Ok("GFX_unit_infantry_icon_medium".to_owned()));
    assert!(icon_key(0, 3, b"infantry", declared(&["GFX_div_templ_3_large"])).unwrap_err().contains("null or invalid"));
    assert_eq!(icon_key(7, -1, b"infantry", declared(&[])), Err("Division sprite GFX_unit_infantry_icon_medium is unavailable".to_owned()));
    assert_eq!(icon_key(7, -1, b"", declared(&[])), Err("Division priority unit name is empty".to_owned()));
}

#[test]
fn a_slots_rows_of_one_side_are_one_key_range() {
    let (lo, hi) = slot_side_range(5, 1);
    let key = |slot: u128, side: u128, template: u128| slot << 72 | side << 64 | template;
    assert!((lo..=hi).contains(&key(5, 1, 0)) && (lo..=hi).contains(&key(5, 1, u64::MAX as u128)));
    assert!(!(lo..=hi).contains(&key(5, 0, u64::MAX as u128)) && !(lo..=hi).contains(&key(6, 1, 0)) && !(lo..=hi).contains(&key(4, 1, u64::MAX as u128)));
}

#[test]
fn divisions_short_is_not_known_when_no_division_was_compared() {
    assert_eq!(short_count(&Recorded::Value(0), false), None);
    assert_eq!(short_count(&Recorded::Value(0), true), Some(0));
    assert_eq!(short_count(&Recorded::Value(3), true), Some(3));
    assert_eq!(short_count(&Recorded::NotRecorded, true), None);
}

#[test]
fn each_division_equipment_view_reads_only_its_own_operands_and_a_type_is_keyed_by_its_recorded_token() {
    use super::equipment::{DivisionEquipment, DivisionEquipmentParams, DivisionEquipmentView, type_key};
    use crate::products::army::ArmyAttribution;
    use dashboard_base::products::at_hour::{AtHour, Keys, Owner};
    use dashboard_base::test_fixtures::source;
    use replay_data::rows::{DivisionEquipmentTemplateRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    let params = |view| DivisionEquipmentParams { branch_id: "1".into(), country: "GER".into(), count_for: ArmyAttribution::Owner, view };
    let templates = DivisionEquipment::tables(&params(DivisionEquipmentView::Templates));
    let types = DivisionEquipment::tables(&params(DivisionEquipmentView::Types));
    assert!(templates.contains(&TableId::DivisionTemplates) && templates.contains(&TableId::DivisionEquipmentByTemplate));
    assert!(!templates.contains(&TableId::DivisionEquipmentByDesign) && !templates.contains(&TableId::VariantDefinitions), "the template view neither reads nor waits on the design tables");
    assert!(types.contains(&TableId::DivisionEquipmentByDesign) && !types.contains(&TableId::DivisionEquipmentByTemplate) && !types.contains(&TableId::DivisionTemplates));
    let mut held = source(1, 100, Some(1_000));
    held.tables[TableId::DivisionEquipmentByTemplate.index()] = TableView::Complete(
        TableRows::DivisionEquipmentByTemplate(vec![DivisionEquipmentTemplateRow { slot: 7, side: 0, template: 42, ..Default::default() }]),
    )
    .into();
    held.tables[TableId::DivisionEquipmentCounts.index()] = TableView::Complete(
        TableRows::DivisionEquipmentCounts(vec![replay_data::rows::DivisionEquipmentCountRow { slot: 7, side: 0, ..Default::default() }]),
    ).into();
    let owner = Some(Owner { slot: 7, id: 9001 });
    let reads = DivisionEquipment::reads(&params(DivisionEquipmentView::Templates), owner, 1, &held);
    let template_read = reads.iter().find(|read| read.table == TableId::DivisionTemplates).expect("the template view names its templates");
    assert!(matches!(&template_read.keys, Keys::These(keys) if keys == &[42u128]));
    assert!(DivisionEquipment::reads(&params(DivisionEquipmentView::Types), owner, 1, &held).iter().all(|read| read.table != TableId::DivisionTemplates));
    assert_eq!([type_key(Some(17)), type_key(Some(18)), type_key(None)], ["17", "18", "unclassified"], "two types whose spelling was not read keep their own rows");
}

#[test]
fn a_medal_s_picture_is_its_definition_s_sprite_at_its_authored_frame() {
    use content_reader_military::unit_medals::Medal;
    let medal = Medal { name: b"medal_one".to_vec(), icon: Some(b"GFX_medal_strip".to_vec()), frame: Some(3), ..Default::default() };
    let sprite = super::careers::sprite_of(&medal).unwrap();
    assert_eq!((sprite.key.as_str(), sprite.frame), ("GFX_medal_strip", 2), "the authored frame counts from one; the handle from zero");
    let unframed = Medal { frame: None, ..medal.clone() };
    assert_eq!(super::careers::sprite_of(&unframed).unwrap().frame, 0);
    let bare = Medal { icon: None, ..medal };
    assert_eq!(super::careers::sprite_of(&bare).unwrap_err(), "The medal's definition names no icon");
}

#[test]
fn decorated_divisions_answer_the_first_in_order_with_unread_measures_last_and_ties_by_the_other_measures() {
    use super::careers::{DecoratedRank, Measures, ranked};
    let first_in_order = |measured: Vec<Measures>, by: DecoratedRank, descending: bool, count: usize| ranked(measured, by, descending, count).0;
    let division = |id: u64, medals: Option<i32>, experience: Option<i64>, casualties_taken: Option<i32>| Measures { id, medals, experience, casualties_taken, enemy_killed: None, name: None };
    let world = vec![
        division(1, Some(0), Some(10), Some(5)),
        division(2, None, Some(90), Some(900)),
        division(3, Some(2), Some(10), Some(5)),
        division(4, Some(0), Some(30), Some(5)),
        division(5, Some(2), Some(10), Some(7)),
        division(6, Some(0), Some(10), Some(5)),
    ];
    let ids = |rows: Vec<Measures>| rows.iter().map(|row| row.id).collect::<Vec<_>>();
    assert_eq!(ids(first_in_order(world.clone(), DecoratedRank::Medals, true, 10)), [5, 3, 4, 1, 6, 2], "ties fall to experience, then casualties taken, then id; an unread medal total is last");
    assert_eq!(ids(first_in_order(world.clone(), DecoratedRank::Medals, true, 3)), [5, 3, 4], "only the first of the order is answered");
    assert_eq!(ids(first_in_order(world.clone(), DecoratedRank::Medals, false, 3)), [4, 1, 6], "ascending keeps the unread last too");
    assert_eq!(ids(first_in_order(world.clone(), DecoratedRank::CasualtiesTaken, true, 2)), [2, 5]);
    let named = |at: usize, name: Option<&str>| Measures { name: name.map(super::careers::name_key), ..world[at].clone() };
    let by_name = vec![named(0, Some("bravo")), named(1, None), named(2, Some("Alpha")), named(3, Some("Bravo")), named(4, Some("Échelon"))];
    assert_eq!(ids(first_in_order(by_name.clone(), DecoratedRank::DivisionName, false, 10)), [3, 4, 1, 5, 2], "names fold case first; a division without a name follows the named ones");
    assert_eq!(ids(first_in_order(by_name, DecoratedRank::CountryName, true, 10)), [5, 1, 4, 3, 2], "descending keeps the unnamed last too");
}

#[test]
fn a_medal_total_above_the_identified_awards_keeps_the_rest_as_unidentified() {
    use super::careers::unidentified;
    assert_eq!(unidentified(5, 2).map(|award| (award.medal, award.count)), Some((None, 3)));
    assert_eq!(unidentified(2, 2), None);
    assert_eq!(unidentified(-1, 0), None);
}

#[test]
fn decorated_divisions_hold_until_the_country_filter_next_changes() {
    use super::careers::{CountryHours, filter_valid_through};
    let span = |first_hour: i32, last_hour: i32| CountryHours { tag: "GER".into(), first_hour, last_hour };
    let spans = [span(100, 199), span(300, 399)];
    assert_eq!(filter_valid_through(None, 150), i32::MAX, "every country never changes membership");
    assert_eq!(filter_valid_through(Some(&spans), 50), 99, "a country joins at 100");
    assert_eq!(filter_valid_through(Some(&spans), 150), 199, "and leaves after 199");
    assert_eq!(filter_valid_through(Some(&spans), 250), 299);
    assert_eq!(filter_valid_through(Some(&spans), 399), 399, "and leaves again after 399");
    assert_eq!(filter_valid_through(Some(&spans), 400), i32::MAX);
}

const DECORATED: [u64; 3] = [1, 2, 3];

fn decorated_world(hour: i32, experience: impl Fn(u64) -> i64) -> replay_frame::frame::DecodedFrame {
    use replay_data::rows::{DivisionHistoryRow, DivisionLandInputsRow, DivisionManpowerContributor, DivisionRow, DivisionStatusRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
    let divisions = DECORATED.iter().map(|&id| DivisionRow { id, roster_owner: 9001, logical_owner: 9001, template: 1, province: if id == 1 { 100 + hour as u32 } else { 7 }, ..Default::default() }).collect();
    let status = DECORATED.iter().map(|&id| DivisionStatusRow { division: id, entrenchment: i64::from(hour), experience: Recorded::Value(Some(experience(id))), career_counter: Recorded::Value(Some(10)), ..Default::default() }).collect();
    let inputs = DECORATED.iter().map(|&id| DivisionLandInputsRow { division: id, manpower: vec![DivisionManpowerContributor { country: 9001, amount: 10 }], ..Default::default() }).collect();
    let history = DECORATED.iter().map(|&id| DivisionHistoryRow { division: id, list: DivisionHistoryRow::SUMMARY, available: 1, amount: 1, ..Default::default() }.into()).collect();
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divisions));
    *frame.tables[TableId::DivisionStatus.index()] = TableView::Complete(TableRows::DivisionStatus(status));
    *frame.tables[TableId::DivisionLandInputs.index()] = TableView::Complete(TableRows::DivisionLandInputs(inputs));
    *frame.tables[TableId::DivisionHistory.index()] = TableView::Complete(TableRows::DivisionHistory(history));
    frame
}

fn decorated_source(frames: Vec<replay_frame::frame::DecodedFrame>) -> dashboard_base::Source {
    use dashboard_base::test_fixtures::{CAMPAIGN, records};
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    dashboard_base::Source::from_replay(dashboard_base::SourceKey { source_id: "decorated".into(), revision: "1".into() }, "decorated.battleplan".into(), std::sync::Arc::new(replay), &std::sync::atomic::AtomicBool::new(false)).unwrap()
}

#[test]
fn a_quiet_hour_of_decorated_divisions_visits_no_row_while_a_division_moves_and_tokens_stay_unread() {
    use super::careers::{DecoratedDivisions, DecoratedDivisionsParams, DecoratedRank};
    use dashboard_base::{products::{LoadContext, at_hour::frame_panel}, test_fixtures::{materialized_frame, played}};
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::Arc;
    const FIRST: i32 = 1_000;
    const LAST: i32 = 1_020;
    const VETERAN: i32 = 1_015;
    let source = Arc::new(decorated_source((FIRST..=LAST).map(|hour| {
        let mut frame = decorated_world(hour, |id| if id == 1 && hour >= VETERAN { 300 } else { 30 * id as i64 });
        *frame.tables[TableId::DivisionTemplateIcons.index()] = TableView::Complete(TableRows::DivisionTemplateIcons(Vec::new()));
        if (FIRST + 8..FIRST + 10).contains(&hour) {
            *frame.tables[TableId::Divisions.index()] = TableView::Unavailable { reason: "damaged division census".into() };
        }
        frame
    }).collect()));
    let asked = serde_json::to_value(DecoratedDivisionsParams { branch_id: "1".into(), countries: None, by: DecoratedRank::Experience, descending: true, count: 2 }).unwrap();
    let load = LoadContext { content: None };
    let registered = crate::registrations::DIVISIONS_CAREERS_DECORATEDDIVISIONS.panel.unwrap();
    let panel = (registered.open)(&source, &load, &asked).unwrap();
    let mut original = frame_panel::<DecoratedDivisions>(&source, &load, &asked).unwrap();
    let reads = played(&source.replay(), panel.exact().unwrap(), FIRST, LAST);
    assert!(reads.len() >= 15 && reads.iter().all(|read| !read.pending));
    let ranked = |value: &Arc<dashboard_follow::panel::PanelData>| -> Vec<String> {
        value.as_ref().as_ref().unwrap()["divisions"].as_array().unwrap().iter().map(|row| row["division"].as_str().unwrap().to_owned()).collect()
    };
    let at = |hour: i32| reads.iter().position(|read| read.hour == hour).unwrap();
    assert_eq!(ranked(&reads[1].value), ["3", "2"], "the two most experienced, from the populated recording");
    assert_eq!(ranked(&reads[at(VETERAN)].value), ["1", "3"], "a ranked measure that changes is ranked again");
    for read in &reads {
        let expected = materialized_frame(&mut *original, source.replay_ref(), 1, read.hour);
        assert_eq!(*read.value, expected, "played reply at {}", read.hour);
        if read.hour > FIRST {
            assert_eq!(read.product_calls, u64::from(read.hour == VETERAN || (FIRST + 8..=FIRST + 10).contains(&read.hour)), "product calls at {}", read.hour);
        }
    }
    for hour in [VETERAN, FIRST, FIRST + 9] {
        assert_eq!(*(panel.exact().unwrap().advance)(hour, true, &false.into(), None).value, materialized_frame(&mut *original, source.replay_ref(), 1, hour));
    }
    for (place, read) in reads.iter().enumerate().skip(2).filter(|(_, read)| read.hour != VETERAN && !(FIRST + 8..=FIRST + 10).contains(&read.hour)) {
        assert_eq!((read.rows, read.visited), (0, 0), "hour {}: a move and a dug-in change are not ranked or shown, and unread tokens stay unread", read.hour);
        assert!(Arc::ptr_eq(&read.value, &reads[place - 1].value), "hour {} keeps the held reply", read.hour);
    }
}

#[test]
fn unread_country_links_keep_the_ranking_under_all_and_make_a_filtered_ranking_unknown_not_empty() {
    use super::careers::{CountryHours, DecoratedDivisions, DecoratedDivisionsParams, DecoratedRank};
    use dashboard_base::products::{LoadContext, at_hour::exact_panel};
    use replay_data::rows::TableId;
    use replay_frame::frame::TableView;
    let unread = |hour: i32| {
        let mut frame = decorated_world(hour, |id| 30 * id as i64);
        *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "links unread in the fixture".into() };
        frame
    };
    let source = decorated_source((1_000..=1_002).map(unread).collect());
    let answer = |countries: Option<Vec<CountryHours>>| {
        let asked = serde_json::to_value(DecoratedDivisionsParams { branch_id: "1".into(), countries, by: DecoratedRank::Experience, descending: true, count: 2 }).unwrap();
        let panel = exact_panel::<DecoratedDivisions>(&source, &LoadContext { content: None }, &asked).unwrap();
        (panel.exact().unwrap().cold)(1_001, &std::sync::atomic::AtomicBool::new(false)).unwrap()
    };
    let all = answer(None);
    let shown: Vec<(&str, &serde_json::Value)> = all["divisions"].as_array().unwrap().iter().map(|row| (row["division"].as_str().unwrap(), &row["tag"])).collect();
    assert_eq!(shown, [("3", &serde_json::Value::Null), ("2", &serde_json::Value::Null)], "every division still ranks; its country is the missing part");
    assert_eq!(all["missing"], serde_json::Value::Null);
    assert!(all["warnings"].as_array().unwrap().iter().any(|line| line.as_str().unwrap().starts_with("Division countries unavailable")), "{all}");
    let filtered = answer(Some(vec![CountryHours { tag: "GER".into(), first_hour: 1_000, last_hour: 1_002 }]));
    assert_eq!(filtered["divisions"].as_array().unwrap().len(), 0);
    assert_eq!(filtered["missing"], "3 divisions' countries were not recorded at this hour, so which belong to the chosen countries is not known", "never an empty list that reads as no divisions");
}

#[test]
fn a_tie_at_the_cut_or_between_shown_divisions_decided_by_an_unread_measure_is_not_a_proven_order() {
    use super::careers::{DecoratedRank, Measures, ranked};
    let division = |id: u64, medals: Option<i32>, experience: Option<i64>, casualties_taken: Option<i32>| Measures { id, medals, experience, casualties_taken, enemy_killed: None, name: None };
    let ids = |(rows, unproven): (Vec<Measures>, bool)| (rows.iter().map(|row| row.id).collect::<Vec<_>>(), unproven);
    let tied = vec![division(1, Some(4), Some(10), Some(1)), division(2, Some(4), None, Some(9)), division(3, Some(1), Some(90), Some(1))];
    assert_eq!(ids(ranked(tied.clone(), DecoratedRank::Medals, true, 1)), (vec![1], true), "equal medals at the cut, one experience unread");
    assert_eq!(ids(ranked(tied.clone(), DecoratedRank::Medals, true, 3)), (vec![1, 2, 3], true), "the same tie between shown rows");
    let known = vec![division(1, Some(4), Some(10), Some(1)), division(2, Some(4), Some(5), None), division(3, Some(1), None, None)];
    assert_eq!(ids(ranked(known, DecoratedRank::Medals, true, 1)), (vec![1], false), "a tie the read experience decides is proven, though a later measure is unread");
    assert_eq!(ids(ranked(tied, DecoratedRank::Experience, true, 2)), (vec![3, 1], false), "no tie on the ranked measure");
}

#[test]
fn a_service_record_pages_one_division_and_follows_the_decorated_leader_through_gaps() {
    use super::careers::service_record::{DivisionServiceRecordChoices, DivisionServiceRecordTail};
    use dashboard_base::products::{LoadContext, Product};
    use replay_data::rows::{DivisionHistoryRow, LandCombatEdgesRow, LandCombatRow, LandDivisionDamageRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    use serde_json::json;
    let experience = |id: u64, hour: i32| 20 * id as i64 + 1_000 * i64::from(hour - 100);
    let source = decorated_source((100..=107).map(|hour| {
        let mut frame = decorated_world(hour, |id| experience(id, hour));
        let (damage, exits) = if hour == 106 {
            (vec![LandDivisionDamageRow { id: 50 << 16, hour, combat: 4, target: 9, attacker: 2, army: None, strength_raw: 100 }],
                vec![LandCombatEdgesRow { id: 50, kind: LandCombatEdgesRow::EXITED, hour, combat: 4, division: Some(9), casualties_before: Some(0), casualties_after: Some(3), pending_strength_raw: Some(100), ..Default::default() }])
        } else if hour == 107 {
            (vec![LandDivisionDamageRow { id: 60 << 16, hour, combat: 4, target: 8, attacker: 2, army: None, strength_raw: 100 }],
                vec![LandCombatEdgesRow { id: 60, kind: LandCombatEdgesRow::EXITED, hour, combat: 4, division: Some(8), casualties_before: Some(0), casualties_after: Some(3), pending_strength_raw: None, ..Default::default() }])
        } else {
            (Vec::new(), Vec::new())
        };
        *frame.tables[TableId::LandDivisionDamage.index()] = TableView::Complete(TableRows::LandDivisionDamage(damage));
        *frame.tables[TableId::LandCombatEdges.index()] = TableView::Complete(TableRows::LandCombatEdges(exits));
        frame.metrics.table_ns[TableId::Divisions.index()] = 1;
        frame.metrics.table_ns[TableId::DivisionHistory.index()] = 1;
        if let TableView::Complete(TableRows::Divisions(rows)) = &mut *frame.tables[TableId::Divisions.index()] {
            rows.retain(|row| row.id != 3 || (102..104).contains(&hour));
        }
        frame.metrics.table_ns[TableId::LandCombats.index()] = 1;
        *frame.tables[TableId::LandCombats.index()] = TableView::Complete(TableRows::LandCombats(vec![LandCombatRow {
            id: 4, province: 9, attackers: vec![1], defenders: vec![2], ..Default::default()
        }]));
        let mut history = Vec::new();
        for &id in &DECORATED {
            history.push(DivisionHistoryRow { division: id, available: u8::from(hour != 102 || id != 1), amount: id as i32, ..Default::default() }.into());
            history.push(DivisionHistoryRow { division: id, list: DivisionHistoryRow::CAREER, hour: 90 + id as i32, ..Default::default() }.into());
        }
        *frame.tables[TableId::DivisionHistory.index()] = if hour == 103 {
            TableView::Unavailable { reason: "history unread".into() }
        } else {
            TableView::Complete(TableRows::DivisionHistory(history))
        };
        frame
    }).collect());
    let ask = |hour, division: Option<&str>| DivisionServiceRecordTail::load(&source,
        &serde_json::from_value(json!({"branchId": "1", "hour": hour, "division": division, "limit": 12})).unwrap(),
        &LoadContext { content: None }, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    let choices = |hour| DivisionServiceRecordChoices::load(&source,
        &serde_json::from_value(json!({"branchId": "1", "hour": hour})).unwrap(),
        &LoadContext { content: None }, &std::sync::atomic::AtomicBool::new(false)).unwrap().choices;
    let early = ask(101, None);
    assert_eq!(early.selected.as_ref().map(|d| d.division.as_str()), Some("2"));
    assert_eq!(choices(101).len(), 2);
    assert_eq!(early.next_change, Some(102));
    assert_eq!(ask(102, None).selected.as_ref().map(|d| d.division.as_str()), Some("3"));
    assert!(ask(103, Some("1")).rows.iter().any(|row| row.kind == "career" && row.hour == 91));
    let pinned = ask(104, Some("1"));
    assert_eq!(choices(104).len(), 3);
    assert_eq!(pinned.rows.iter().filter(|row| row.kind == "battle").map(|row| (row.hour, row.province)).collect::<Vec<_>>(), [(100, Some(9))]);
    assert!(ask(104, Some("3")).rows.iter().all(|row| row.kind != "battle"));
    assert!(pinned.missing.iter().any(|gap| gap.first_hour == 102 && gap.reason.contains("division's history")));
    assert!(ask(104, Some("2")).missing.iter().all(|gap| !gap.reason.contains("division's history")));
    let career: Vec<_> = pinned.rows.iter().filter(|row| row.kind == "career").map(|row| row.hour).collect();
    assert_eq!(career, [91]);
    let measured = |id, hour| {
        replay_data::rows::DivisionStatusRow { experience: Recorded::Value(Some(experience(id, hour))), ..Default::default() }.normalized_experience(10).map(|raw| raw.to_string())
    };
    let facts = pinned.facts.as_ref().unwrap();
    assert_eq!((facts.experience_raw.clone(), facts.casualties_taken), (measured(1, 104), Some(10)), "a serving division's facts are its own at the hour");
    let gone = ask(104, Some("3"));
    assert_eq!(gone.selected.as_ref().map(|d| d.last_seen), Some(Some(103)));
    assert_eq!(pinned.selected.as_ref().map(|d| d.last_seen), Some(None), "a division in the census at the hour asked holds no hour that would change with it");
    assert_ne!(measured(3, 103), measured(3, 104));
    let facts = gone.facts.as_ref().unwrap();
    assert_eq!((facts.experience_raw.clone(), facts.casualties_taken), (measured(3, 103), Some(10)), "a division out of the census shows its facts when last seen");
    assert!(pinned.missing.iter().any(|gap| gap.first_hour == 103 && gap.last_hour == 103 && gap.reason.contains("history")), "{:?}", pinned.missing);
    let quiet = ask(105, Some("2"));
    assert_eq!(quiet.facts.as_ref().and_then(|facts| facts.enemy_killed), Some(0));
    assert_eq!(quiet.next_change, Some(106), "an hour when only the division's kills change is a change");
    let counted = ask(106, Some("2"));
    assert_eq!(counted.facts.as_ref().and_then(|facts| facts.enemy_killed), Some(3));
    assert_eq!(counted.next_change, Some(107), "a total that becomes unknown is a change");
    let unknown = ask(107, Some("2")).facts.unwrap();
    assert!(unknown.enemy_killed.is_none() && unknown.enemy_killed_missing.is_some());
}

#[test]
fn a_service_record_reads_a_status_read_in_part_at_a_run_s_first_hour_as_its_change_hours_do_whichever_hour_is_asked_first() {
    use super::careers::service_record::DivisionServiceRecordTail;
    use dashboard_base::products::{LoadContext, Product, at_hour};
    use replay_data::{
        identity::{CaptureMode, Identity, Roster, SampleBoundary, SessionStart, Verification},
        rows::{CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionHistoryRow, DivisionLandInputsRow, DivisionManpowerContributor, DivisionRow, DivisionStatusRow, TableId},
    };
    use replay_frame::branches::{BranchReason, BranchStart, Parentage};
    use replay_journal::synthetic::{Input, Sample};
    let session = SessionStart {
        id: 1, process_id: 1, process_start_time: 1, qpc_frequency: 10_000_000, sample_boundary: SampleBoundary::EndOfHour, capture_mode: CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(), exe_hash: [1; 32], exe_size: 1, vanilla_hash: [2; 32], dlc: vec![], mods: vec![], coverage_policy: 1,
            language: "l_english".into(), recorder_version: "test".into(), collector_hash: [3; 32],
            verification: Verification::Verified { launcher: Roster::default(), memory: Roster::default() },
        },
    };
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: 120, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let campaign = CampaignRow { id: b"opening-status".to_vec() };
    let countries = [CountryRow { id: 9001, canonical_slot: 7, tag: b"GER".to_vec(), ..CountryRow::default() }];
    let links = [CountryLinkRow { id: 9001, canonical_slot: 7 }];
    let divisions = [DivisionRow { id: 1, roster_owner: 9001, logical_owner: 9001, template: 1, ..Default::default() }];
    let status = [DivisionStatusRow { division: 1, experience: Recorded::Value(Some(40)), career_counter: Recorded::Value(Some(10)), ..Default::default() }];
    let inputs = [DivisionLandInputsRow { division: 1, manpower: vec![DivisionManpowerContributor { country: 9001, amount: 10 }], ..Default::default() }];
    let history = [DivisionHistoryRow { division: 1, list: DivisionHistoryRow::SUMMARY, available: 1, amount: 0, ..Default::default() }];
    let snapshot = |partial| (partial, true, Sample::FromReadBit);
    let make = |status_partial: &dyn Fn(i32) -> Option<u32>| {
        let replay = replay::campaign::Replay::from_synthetic(recorder_derive::writers(), campaign.id.clone(), session.clone(), branch.clone(), 122, |frame, hour| {
            let (partial, read, sample) = snapshot(None);
            frame.table(TableId::Clock, Input::Snapshot { rows: &[ClockRow { hour, ..ClockRow::default() }], partial, read, sample })?;
            frame.table(TableId::Campaign, Input::Snapshot { rows: std::slice::from_ref(&campaign), partial, read, sample })?;
            frame.table(TableId::Countries, Input::Snapshot { rows: &countries, partial, read, sample })?;
            frame.table(TableId::CountryLinks, Input::Snapshot { rows: &links, partial, read, sample })?;
            frame.table(TableId::Divisions, Input::Snapshot { rows: &divisions, partial, read, sample })?;
            let (inputs_partial, read, sample) = snapshot((hour == 120).then_some(1));
            frame.table(TableId::DivisionLandInputs, Input::Snapshot { rows: &inputs, partial: inputs_partial, read, sample })?;
            frame.table(TableId::DivisionHistory, Input::Snapshot { rows: &history, partial, read, sample })?;
            frame.table(TableId::DivisionStatus, Input::Snapshot { rows: &status, partial: status_partial(hour), read, sample })
        })
        .unwrap()
        .into_timelines(&Default::default())
        .unwrap();
        dashboard_base::Source::from_replay(dashboard_base::SourceKey { source_id: "opening".into(), revision: "1".into() }, "opening.battleplan".into(), std::sync::Arc::new(replay), &std::sync::atomic::AtomicBool::new(false)).unwrap()
    };
    let source = || make(&|hour| (hour == 120).then_some(1));
    let asked = |source: &dashboard_base::Source, hour: i32| {
        let params = serde_json::from_value(serde_json::json!({"branchId": "1", "hour": hour, "division": "1", "limit": 12})).unwrap();
        DivisionServiceRecordTail::load(source, &params, &LoadContext { content: None }, &std::sync::atomic::AtomicBool::new(false)).unwrap()
    };
    let casualties = |source: &dashboard_base::Source, hour: i32| {
        let reply = asked(source, hour);
        (reply.facts.and_then(|facts| facts.casualties_taken), reply.next_change)
    };
    let first = source();
    let query = dashboard_base::queries::query(&first, 1).unwrap();
    let keyed = at_hour::frame(&query, 120, &[at_hour::Read::these(TableId::DivisionStatus, [1])], "Opening").unwrap();
    assert!(dashboard_query::coverage::witnessed::<replay_data::rows::tables::DivisionStatus>(&keyed, 1).row().is_some(), "the keyed read at the opening hour sees the row");
    assert_eq!(casualties(&first, 120), (None, Some(121)), "the summary leaves it unread there, as its change hours do, and holds only to the next hour");
    assert_eq!(casualties(&first, 121).0, Some(10));
    let later = source();
    assert_eq!((casualties(&later, 121).0, casualties(&later, 120).0), (Some(10), None), "whichever hour is asked first");
    let measured = DivisionStatusRow { experience: Recorded::Value(Some(40)), ..Default::default() }.normalized_experience(10).map(|raw| raw.to_string());
    let complete = make(&|_| None);
    assert_eq!(asked(&complete, 120).facts.and_then(|facts| facts.experience_raw), measured, "manpower read in part at the opening hour is still its witnessed row");
}

#[test]
fn a_service_record_of_chosen_countries_names_the_hours_whose_holders_are_not_known_and_changes_at_both_ends() {
    use super::careers::service_record::DivisionServiceRecordTail;
    use dashboard_base::products::{LoadContext, Product};
    use replay_data::rows::TableId;
    use replay_frame::frame::TableView;
    use serde_json::json;
    let source = decorated_source((100..=107).map(|hour| {
        let mut frame = decorated_world(hour, |id| 20 * id as i64);
        if (103..=104).contains(&hour) {
            *frame.tables[TableId::Countries.index()] = TableView::Unavailable { reason: "countries unread".into() };
        }
        frame
    }).collect());
    let ask = |hour| DivisionServiceRecordTail::load(&source,
        &serde_json::from_value(json!({"branchId": "1", "hour": hour, "countries": ["GER"], "limit": 12})).unwrap(),
        &LoadContext { content: None }, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    let named = |reply: &super::careers::service_record::ServiceRecordTailReply| reply.warnings.iter().any(|warning| warning.starts_with("Who holds"));
    let before = ask(102);
    assert!(!named(&before), "{:?}", before.warnings);
    assert!(before.next_change.is_some_and(|at| at <= 103), "playback re-reads where the holders stop being known: {:?}", before.next_change);
    let during = ask(103);
    assert!(named(&during), "{:?}", during.warnings);
    assert!(during.next_change.is_some_and(|at| at <= 105), "and where they are known again: {:?}", during.next_change);
    assert!(!named(&ask(105)), "the reason does not outlive the gap");

    let no_divisions = decorated_source((100..=107).map(|hour| {
        let mut frame = decorated_world(hour, |_| 0);
        frame.metrics.table_ns[TableId::Divisions.index()] = 1;
        frame.metrics.table_ns[TableId::DivisionHistory.index()] = 1;
        *frame.tables[TableId::Divisions.index()] = TableView::Complete(replay_data::rows::TableRows::Divisions(Vec::new()));
        if (103..=104).contains(&hour) {
            *frame.tables[TableId::DivisionHistory.index()] = TableView::Unavailable { reason: "history unread".into() };
        }
        frame
    }).collect());
    let empty = |hour| DivisionServiceRecordTail::load(&no_divisions,
        &serde_json::from_value(json!({"branchId": "1", "hour": hour, "limit": 12})).unwrap(),
        &LoadContext { content: None }, &std::sync::atomic::AtomicBool::new(false)).unwrap();
    let during = empty(103);
    assert_eq!(during.unavailable.as_deref(), Some("The most decorated division could not be determined from the recorded census and medals."));
    assert_eq!(during.next_change, Some(105));
    assert!(empty(105).unavailable.is_none());
}

#[test]
fn the_type_view_keeps_its_rows_when_the_designs_behind_its_tooltip_are_unread() {
    use super::equipment::{DivisionEquipment, DivisionEquipmentParams, DivisionEquipmentView};
    use crate::products::army::ArmyAttribution;
    use dashboard_base::products::{Country, LoadContext, at_hour::{At, AtHour, Owner}};
    use dashboard_follow::live::Cells;
    use dashboard_query::identity::Holders;
    use replay_data::rows::{DivisionEquipmentCountRow, DivisionEquipmentTypeRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    let side = super::side(ArmyAttribution::Owner);
    let mut frame = dashboard_base::test_fixtures::source(1, 1_000, Some(1));
    *frame.tables[TableId::DivisionEquipmentCounts.index()] = TableView::Complete(TableRows::DivisionEquipmentCounts(vec![DivisionEquipmentCountRow {
        slot: 7, side, divisions: Recorded::Value(2), short: Recorded::Value(1), unknown: Recorded::Value(0), unpooled: Recorded::Value(0), unclassified: Recorded::Value(0), ..Default::default()
    }]));
    *frame.tables[TableId::DivisionEquipmentByType.index()] = TableView::Complete(TableRows::DivisionEquipmentByType(vec![DivisionEquipmentTypeRow {
        slot: 7, side, archetype: None, held: Recorded::Value(300_000), needed: Recorded::Value(500_000), filled: Recorded::Value(300_000), missing: Recorded::Value(200_000), short: Recorded::Value(1), stock: Recorded::NotRecorded,
    }]));
    *frame.tables[TableId::DivisionEquipmentByDesign.index()] = TableView::Unavailable { reason: "designs unread in the fixture".into() };
    let source = decorated_source(vec![decorated_world(1_000, |id| 30 * id as i64)]);
    let context = DivisionEquipment::context(&source, &LoadContext { content: None }).unwrap();
    let (holders, live, cancelled) = (Holders::default(), Cells::default(), std::sync::atomic::AtomicBool::new(false));
    let at = At { owner: Some(Owner { slot: 7, id: 9001 }), holders: &holders, hour: 1_000, context: &context, cancelled: &cancelled, live: &live };
    let params = DivisionEquipmentParams { branch_id: "1".into(), country: Country::from("GER"), count_for: ArmyAttribution::Owner, view: DivisionEquipmentView::Types };
    let reply = DivisionEquipment::reply(&frame, &params, &at).unwrap();
    assert_eq!(reply.missing, None, "the by-type holdings are complete");
    let row = &reply.types[0];
    assert_eq!((row.held_raw.as_deref(), row.missing_raw.as_deref(), row.divisions_short), (Some("300000"), Some("200000"), Some(1)));
    assert!(row.variants.is_empty() && row.variants_missing.as_deref().is_some_and(|why| why.contains("designs unread in the fixture")), "the design list says why it is missing");
}

#[test]
fn enemy_killed_is_the_recorded_count_rounded_to_men_zero_when_absent_and_missing_when_not_known() {
    use super::careers::{EnemyKilled, enemy_killed};
    use dashboard_base::test_fixtures::source;
    use replay_data::rows::{DivisionKillsRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    let mut frame = source(1, 100, Some(1_000));
    let rows = vec![
        DivisionKillsRow { division: 5, killed_raw: 48_049_999, incomplete_from: None },
        DivisionKillsRow { division: 6, killed_raw: 7, incomplete_from: Some(90) },
    ];
    frame.tables[TableId::DivisionKills.index()] = TableView::Complete(TableRows::DivisionKills(rows)).into();
    assert_eq!(enemy_killed(&frame, 5), EnemyKilled { killed: Some(480), missing: None });
    assert_eq!(enemy_killed(&frame, 9), EnemyKilled { killed: Some(0), missing: None }, "a division the complete count never credited killed no one it saw");
    assert!(enemy_killed(&frame, 6).killed.is_none() && enemy_killed(&frame, 6).missing.is_some());
    frame.tables[TableId::DivisionKills.index()] = TableView::Unavailable { reason: "Division damage has not been recorded yet in this branch".into() }.into();
    let unknown = enemy_killed(&frame, 5);
    assert_eq!(unknown.killed, None);
    assert!(unknown.missing.unwrap().ends_with("Division damage has not been recorded yet in this branch"));
}

#[test]
fn a_service_record_reads_lexical_changes_only_for_its_award_tokens() {
    use super::careers::service_record::{DivisionServiceRecordTail, award_token_changes};
    use dashboard_base::products::{LoadContext, Product};
    use replay_data::rows::{DivisionHistoryRow, LexicalTokenRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    use serde_json::json;
    let awarded = decorated_source((100..=107).map(|hour| {
        let mut frame = decorated_world(hour, |_| 40);
        let history = vec![
            DivisionHistoryRow { division: 1, list: DivisionHistoryRow::SUMMARY, available: 1, amount: 1, ..Default::default() }.into(),
            DivisionHistoryRow {
                division: 1,
                list: DivisionHistoryRow::AWARDS,
                ordinal: 0,
                hour: 100,
                medal_token: Some(7),
                medal_enabled: 1,
                medal_count: 1,
                ..Default::default()
            }.into(),
        ];
        *frame.tables[TableId::DivisionHistory.index()] = TableView::Complete(TableRows::DivisionHistory(history));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow { token: 7, literal: if hour < 105 { b"medal_a".to_vec() } else { b"medal_b".to_vec() } },
            LexicalTokenRow { token: 99, literal: format!("noise_{hour}").into_bytes() },
        ]));
        frame.metrics.table_ns[TableId::LexicalTokens.index()] = 1;
        frame
    }).collect());
    let run = dashboard_base::queries::query(&awarded, 1).unwrap().timeline.stretches[0].run;
    let recording = awarded.replay_ref().recording();
    let (changes, statuses) = award_token_changes(recording, run, &[7], 100, 107).unwrap();
    assert_eq!(changes, [(7, 100), (7, 105)], "only requested award token changes are returned, never unrelated token 99");
    assert_eq!(statuses.len(), 1);
    assert_eq!(award_token_changes(recording, run, &[], 100, 107).unwrap(), (Vec::new(), Vec::new()));
    let ask = |hour: i32| {
        DivisionServiceRecordTail::load(
            &awarded,
            &serde_json::from_value(json!({"branchId": "1", "hour": hour, "division": "1", "limit": 12})).unwrap(),
            &LoadContext { content: None },
            &std::sync::atomic::AtomicBool::new(false),
        ).unwrap()
    };
    let early = ask(101);
    assert_eq!(early.next_change, Some(105), "award spelling change breaks next_change; unrelated token changes do not");
    assert_eq!(early.medals.iter().map(|medal| medal.key.as_str()).collect::<Vec<_>>(), ["medal_a"]);
    let renamed = ask(105);
    assert_eq!(renamed.medals.iter().map(|medal| medal.key.as_str()).collect::<Vec<_>>(), ["medal_b"]);
}
