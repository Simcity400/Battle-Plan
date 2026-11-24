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
    use super::careers::{DecoratedRank, Measures, first_in_order};
    let division = |id: u64, medals: Option<i32>, experience: Option<i64>, casualties_taken: Option<i32>| Measures { id, medals, experience, casualties_taken, name: None };
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

#[test]
fn a_quiet_hour_of_decorated_divisions_visits_no_row_while_a_division_moves_and_tokens_stay_unread() {
    use super::careers::{DecoratedDivisions, DecoratedDivisionsParams, DecoratedRank};
    use dashboard_base::{Source, SourceKey, products::{LoadContext, at_hour::exact_panel}, test_fixtures::{CAMPAIGN, played, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{DivisionHistoryRow, DivisionLandInputsRow, DivisionManpowerContributor, DivisionRow, DivisionStatusRow, TableId, TableRows};
    use replay_frame::frame::{DecodedFrame, TableView};
    use std::sync::{Arc, atomic::AtomicBool};
    const FIRST: i32 = 1_000;
    const LAST: i32 = 1_020;
    const VETERAN: i32 = 1_015;
    let ids = [1u64, 2, 3];
    let frame = |hour: i32| -> DecodedFrame {
        let mut frame = source(1, hour, Some(1));
        let experience = |id: u64| if id == 1 && hour >= VETERAN { 300 } else { 30 * id as i64 };
        let divisions = ids.iter().map(|&id| DivisionRow { id, roster_owner: 9001, logical_owner: 9001, template: 1, province: if id == 1 { 100 + hour as u32 } else { 7 }, ..Default::default() }).collect();
        let status = ids.iter().map(|&id| DivisionStatusRow { division: id, entrenchment: i64::from(hour), experience: Recorded::Value(Some(experience(id))), career_counter: Recorded::Value(Some(10)), ..Default::default() }).collect();
        let inputs = ids.iter().map(|&id| DivisionLandInputsRow { division: id, manpower: vec![DivisionManpowerContributor { country: 9001, amount: 10 }], ..Default::default() }).collect();
        let history = ids.iter().map(|&id| DivisionHistoryRow { division: id, list: DivisionHistoryRow::SUMMARY, available: 1, amount: 1, ..Default::default() }.into()).collect();
        *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divisions));
        *frame.tables[TableId::DivisionStatus.index()] = TableView::Complete(TableRows::DivisionStatus(status));
        *frame.tables[TableId::DivisionLandInputs.index()] = TableView::Complete(TableRows::DivisionLandInputs(inputs));
        *frame.tables[TableId::DivisionHistory.index()] = TableView::Complete(TableRows::DivisionHistory(history));
        frame
    };
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, (FIRST..=LAST).map(frame).collect())]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(SourceKey { source_id: "decorated-quiet".into(), revision: "1".into() }, "decorated.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
    let asked = serde_json::to_value(DecoratedDivisionsParams { branch_id: "1".into(), countries: None, by: DecoratedRank::Experience, descending: true, count: 2 }).unwrap();
    let panel = exact_panel::<DecoratedDivisions>(&source, &LoadContext { content: None }, &asked).unwrap();
    let reads = played(&source.replay(), panel.exact().unwrap(), FIRST, LAST);
    assert!(reads.len() >= 15 && reads.iter().all(|read| !read.pending));
    let ranked = |value: &Arc<dashboard_follow::panel::PanelData>| -> Vec<String> {
        value.as_ref().as_ref().unwrap()["divisions"].as_array().unwrap().iter().map(|row| row["division"].as_str().unwrap().to_owned()).collect()
    };
    let at = |hour: i32| reads.iter().position(|read| read.hour == hour).unwrap();
    assert_eq!(ranked(&reads[1].value), ["3", "2"], "the two most experienced, from the populated recording");
    assert_eq!(ranked(&reads[at(VETERAN)].value), ["1", "3"], "a ranked measure that changes is ranked again");
    for (place, read) in reads.iter().enumerate().skip(2).filter(|(_, read)| read.hour != VETERAN) {
        assert_eq!((read.rows, read.visited), (0, 0), "hour {}: a move and a dug-in change are not ranked or shown, and unread tokens stay unread", read.hour);
        assert!(Arc::ptr_eq(&read.value, &reads[place - 1].value), "hour {} keeps the held reply", read.hour);
    }
}
