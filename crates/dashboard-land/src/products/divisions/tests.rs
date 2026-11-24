use super::{
    equipment::{compare_division, short_count},
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
fn a_division_is_capped_at_its_own_need_and_a_held_type_it_does_not_need_misses_nothing() {
    let need = [("infantry_equipment".to_owned(), 100), ("artillery_equipment".to_owned(), 40)];
    let compared = compare_division(&need, &[(Some("Infantry_Equipment"), 130), (Some("artillery_equipment"), 10), (Some("motorized_equipment"), 5)]);
    assert_eq!(
        compared,
        vec![
            (Some("infantry_equipment".to_owned()), 100, 100, 0),
            (Some("artillery_equipment".to_owned()), 40, 10, 30),
            (Some("motorized_equipment".to_owned()), 0, 0, 0),
        ]
    );
    assert_eq!(compare_division(&need[..1], &[(Some("infantry_equipment"), -5)]), vec![(Some("infantry_equipment".to_owned()), 100, 0, 105)]);
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
        template: None,
        division: None,
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
