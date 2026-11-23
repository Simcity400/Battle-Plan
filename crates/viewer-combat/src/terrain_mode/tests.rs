use super::*;
use replay_data::{
    rows::ModifierDefinitionRow, rows::ProvinceModifierEntry, rows::ProvinceWeatherModifiers,
};
use viewer_foundation::tooltip_text::NoIcons;

fn localisation() -> Localisation {
    [
        ("forest", "Forest"),
        ("MOD_SPEED", "Speed"),
        ("MOD_ATTRITION", "Attrition"),
        ("LOCAL_PROVINCE_MODIFIERS", "\nLocal province effects:"),
        ("TERRAIN_SUPPLY_FLOW_REDUCTION", "Flow $RIGHT|1%$"),
        ("MOVEMENT_COST", "Cost x$VALUE|2$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

fn definitions() -> Vec<ModifierDefinitionRow> {
    [(4, &b"MOD_SPEED"[..]), (9, b"MOD_ATTRITION")]
        .into_iter()
        .map(|(index, key)| ModifierDefinitionRow {
            index,
            name_key: key.to_vec(),
            flags: 3,
            category_mask: 1,
            precision: 0,
            ..Default::default()
        })
        .collect()
}

fn accumulator(entries: &[(u32, i64)]) -> ProvinceWeatherModifiers {
    ProvinceWeatherModifiers {
        entries: entries
            .iter()
            .map(|&(key, value)| ProvinceModifierEntry { key, value })
            .collect(),
    }
}

#[test]
fn a_land_province_shows_its_category_then_its_local_effects() {
    content_reader_script::test_game::enter();
    let content = content_reader_key_names::terrain::parse_definitions(
        b"categories={forest={movement_cost=1.5 supply_flow_penalty_factor=0.08}}",
    )
    .unwrap();
    let rows = definitions();
    let definitions = Definitions::from_parts(&rows, &[], None);
    let loc = localisation();
    let local = local::describe(
        &definitions,
        &NoIcons,
        &loc,
        &accumulator(&[(9, 10_000), (4, -25_000)]),
    );
    let (mut dropped, mut refused) = (Vec::new(), None);
    let text = land(
        &content.categories[0],
        &definitions,
        &NoIcons,
        &loc,
        local,
        &mut dropped,
        &mut refused,
    )
    .unwrap();
    assert_eq!(
        text,
        "\u{11}HForest\u{11}!\nFlow 8.0%\nCost x1.50\nLocal province effects:\nSpeed: \u{11}R-25%\u{11}!\nAttrition: \u{11}G+10%\u{11}!"
    );
    assert!(dropped.is_empty() && refused.is_none());
    for empty in [accumulator(&[]), accumulator(&[(4, 0)])] {
        assert_eq!(
            local::describe(&definitions, &NoIcons, &loc, &empty),
            Ok(String::new())
        );
    }
    let text = land(
        &content.categories[0],
        &definitions,
        &NoIcons,
        &loc,
        Err("not recorded".into()),
        &mut dropped,
        &mut refused,
    )
    .unwrap();
    assert_eq!(text, "\u{11}HForest\u{11}!\nFlow 8.0%\nCost x1.50");
    assert_eq!(refused.as_deref(), Some("not recorded"));
}
