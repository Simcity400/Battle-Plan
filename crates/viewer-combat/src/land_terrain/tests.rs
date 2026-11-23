use super::*;
use viewer_foundation::tooltip_text::NoIcons;

#[test]
fn terrain_effects_read_content_and_keep_native_order_and_zero_branches() {
    content_reader_script::test_game::enter();
    let content = content_reader_key_names::terrain::parse_definitions(b"categories={synthetic={movement_cost=1.75 combat_width=47 combat_support_width=19 supply_flow_penalty_factor=0.12 units={attack=-0.23 movement=0.15 defence=0.07} example_effect=0.2}} ").unwrap();
    let terrain = &content.categories[0];
    let loc: Localisation = [
        ("UNIT_TERRAIN_MODIFIER", "Unit effects:"),
        ("STAT_ADJUSTER_MOVEMENT", "Movement "),
        ("STAT_ADJUSTER_MOVEMENT_VALUE", "$VALUE|1%$\n"),
        ("STAT_ADJUSTER_ATTACK", "Attack "),
        ("STAT_ADJUSTER_ATTACK_VALUE", "$VALUE|1%$\n"),
        ("STAT_ADJUSTER_DEFENCE", "Defence "),
        ("STAT_ADJUSTER_DEFENCE_VALUE", "$VALUE|1%$\n"),
        (
            "TERRAIN_COMBAT_WIDTH_COMB",
            "Width $VALUE|0$ (additional $ADD|0$)",
        ),
        ("MOD_EXAMPLE", "Example effect"),
        ("TERRAIN_SUPPLY_FLOW_REDUCTION", "Flow $RIGHT|1%$"),
        ("MOVEMENT_COST", "Cost x$VALUE|2$"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let rows = [replay_data::rows::ModifierDefinitionRow {
        index: 0,
        name_key: b"MOD_EXAMPLE".to_vec(),
        flags: 3,
        category_mask: 1,
        precision: 0,
        ..Default::default()
    }];
    let definitions = Definitions::from_parts(&rows, &[(&b"example_effect"[..], 0)], None);
    let mut dropped = Vec::new();
    assert_eq!(
        effects(terrain, &definitions, &NoIcons, &loc, &mut dropped).unwrap(),
        "\u{11}HUnit effects:\u{11}!\nMovement 115.0%\nAttack 77.0%\nDefence 107.0%\nWidth 47 (additional 19)\nExample effect: \u{11}G+20%\u{11}!\nFlow 12.0%\nCost x1.75"
    );
    assert!(dropped.is_empty());
    let empty = content_reader_key_names::terrain::parse_definitions(b"categories={empty={}}").unwrap();
    assert_eq!(
        effects(
            &empty.categories[0],
            &definitions,
            &NoIcons,
            &loc,
            &mut dropped
        )
        .unwrap(),
        "Flow 0.0%\nCost x0.00"
    );
}
