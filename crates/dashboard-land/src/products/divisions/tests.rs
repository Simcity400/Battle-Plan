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
fn division_equipment_overview_reads_division_templates_for_template_names() {
    use super::equipment::{DivisionEquipment, DivisionEquipmentParams};
    use crate::products::army::ArmyAttribution;
    use dashboard_base::products::at_hour::{AtHour, Keys, Owner};
    use dashboard_base::test_fixtures::source;
    use replay_data::rows::{DivisionEquipmentTemplateRow, TableId, TableRows};
    use replay_frame::frame::TableView;
    let params = DivisionEquipmentParams {
        branch_id: "1".into(),
        country: "GER".into(),
        count_for: ArmyAttribution::Owner,
    };
    assert!(
        DivisionEquipment::tables(&params).contains(&TableId::DivisionTemplates),
        "overview must declare DivisionTemplates so template names resolve"
    );
    let mut held = source(1, 100, Some(1_000));
    held.tables[TableId::DivisionEquipmentByTemplate.index()] = TableView::Complete(
        TableRows::DivisionEquipmentByTemplate(vec![DivisionEquipmentTemplateRow {
            slot: 7,
            side: 0,
            template: 42,
            ..Default::default()
        }]),
    )
    .into();
    let reads = DivisionEquipment::reads(&params, Some(Owner { slot: 7, id: 9001 }), 1, &held);
    let template_read = reads
        .iter()
        .find(|read| read.table == TableId::DivisionTemplates)
        .expect("step 1 must read DivisionTemplates for overview template names");
    match &template_read.keys {
        Keys::These(keys) => assert_eq!(keys, &[42u128]),
        other => panic!("expected Keys::These([42]), got {other:?}"),
    }
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
    let division = |id: u64, medals: Option<i32>, experience: Option<i64>, casualties_taken: Option<i32>| Measures { id, medals, experience, casualties_taken };
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
    assert_eq!(ids(first_in_order(world, DecoratedRank::CasualtiesTaken, true, 2)), [2, 5]);
}

#[test]
fn a_medal_total_above_the_identified_awards_keeps_the_rest_as_unidentified() {
    use super::careers::unidentified;
    assert_eq!(unidentified(5, 2).map(|award| (award.medal, award.count)), Some((None, 3)));
    assert_eq!(unidentified(2, 2), None);
    assert_eq!(unidentified(-1, 0), None);
}
