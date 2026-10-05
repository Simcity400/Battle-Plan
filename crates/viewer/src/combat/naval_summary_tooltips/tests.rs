use super::*;
use crate::combat::binding::NoIcons;
use replay::rows::{LexicalTokenRow, NavalDamageSource};

#[test]
fn shared_tree_uses_borrow_the_same_decoded_nodes() {
    let tree = replay::rows::NavalModifierBreakdownRow {
        accumulator: 17,
        key: 0x1b2,
        uses: [1, 2]
            .into_iter()
            .map(|combat| replay::rows::NavalModifierUse {
                combat,
                order: combat as u32,
                ..Default::default()
            })
            .collect(),
        nodes: vec![replay::rows::ModifierBreakdownNode::default()],
    };
    let expanded = expanded_trees(std::slice::from_ref(&tree));
    assert_eq!(expanded.len(), 2);
    assert!(std::ptr::eq(expanded[0].tree, expanded[1].tree));
    assert!(std::ptr::eq(expanded[0].use_, &tree.uses[0]));
    assert!(std::ptr::eq(expanded[1].use_, &tree.uses[1]));
    assert!(std::ptr::eq(
        expanded[0].nodes.as_ptr(),
        tree.nodes.as_ptr()
    ));
}

#[test]
fn flag_damage_summary_uses_native_sections_and_sorted_source_types() {
    let loc: Localisation = [
        ("total_damage_dealt", "T $NUM$ $RATIO$;"),
        ("damage_dealt_light_guns", "L $NUM$ $RATIO$;"),
        ("damage_dealt_torpedoes", "Torp $NUM$ $RATIO$;"),
        ("damage_dealt_by_box", "BOX|"),
        ("damage_dealt_screen", "S $NUM$ $RATIO$;"),
        ("damage_dealt_convoys", "C $NUM$ $RATIO$;"),
        ("damage_dealt_by_ship_types", "CLASS|"),
        ("damage_dealt_by_type", "$TYPE$ $NUM$ $RATIO$;"),
        ("AAA_destroyer", "Escorts"),
        ("AAA_carrier", "Carriers"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let mut side = NavalCombatSideDetails {
        total_damage: 400_000,
        damage_by_type: vec![
            NavalDamageSource {
                token: 2,
                damage: 300_000,
            },
            NavalDamageSource {
                token: 1,
                damage: 100_000,
            },
        ],
        ..Default::default()
    };
    side.damage_history[0][0] = 100_000;
    side.damage_history[3][2] = 200_000;
    let opponent = NavalCombatSideDetails {
        original_health: 800_000,
        ..Default::default()
    };
    let rows = [
        LexicalTokenRow {
            token: 1,
            literal: b"destroyer".to_vec(),
        },
        LexicalTokenRow {
            token: 2,
            literal: b"carrier".to_vec(),
        },
    ];
    let tokens = TokenNames::from_rows(&rows, true);
    let mut sub_units = SubUnits::default();
    sub_units.read(
        b"sub_units = { destroyer = {} carrier = {} }",
        "common/units/test.txt",
    );
    assert_eq!(
        flag_description(
            &side,
            &opponent,
            &tokens,
            &sub_units,
            Ok("AAA"),
            &NoIcons,
            &loc
        )
        .unwrap(),
        "T 4.00 0.50;L 1.00 0.25;Torp 2.00 0.50;\nBOX|S 1.00 0.25;C 2.00 0.50;\nCLASS|Escorts 1.00 0.25;Carriers 3.00 0.75;\n"
    );
    assert_eq!(
        flag_description(
            &side,
            &NavalCombatSideDetails {
                original_health: -1,
                ..Default::default()
            },
            &tokens,
            &sub_units,
            Ok("AAA"),
            &NoIcons,
            &loc
        )
        .unwrap(),
        ""
    );
}

#[test]
fn flag_damage_summary_keeps_blank_null_type_and_empty_localisation_rows() {
    let loc: Localisation = [
        ("total_damage_dealt", "TOTAL $NUM$;"),
        ("damage_dealt_by_box", "BOX;"),
        ("damage_dealt_by_ship_types", "TYPES;"),
        ("damage_dealt_by_type", "[$TYPE$ $NUM$ $RATIO$]"),
        ("AAA_destroyer", ""),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let side = NavalCombatSideDetails {
        total_damage: 1_000_000,
        damage_by_type: vec![
            NavalDamageSource {
                token: 2,
                damage: 300_000,
            },
            NavalDamageSource {
                token: 1,
                damage: 200_000,
            },
            NavalDamageSource {
                token: 0,
                damage: 100_000,
            },
        ],
        ..Default::default()
    };
    let opponent = NavalCombatSideDetails {
        original_health: 2_000_000,
        ..Default::default()
    };
    let rows = [
        LexicalTokenRow {
            token: 1,
            literal: b"destroyer".to_vec(),
        },
        LexicalTokenRow {
            token: 2,
            literal: b"not_a_subunit".to_vec(),
        },
    ];
    let tokens = TokenNames::from_rows(&rows, true);
    let mut sub_units = SubUnits::default();
    sub_units.read(b"sub_units={destroyer={}}", "common/units/test.txt");
    assert_eq!(
        flag_description(
            &side,
            &opponent,
            &tokens,
            &sub_units,
            Ok("AAA"),
            &NoIcons,
            &loc
        )
        .unwrap(),
        "TOTAL 10.00;\nBOX;\nTYPES;[ 1.00 0.10][ 2.00 0.20][ 3.00 0.30]\n"
    );
    // A missing localisation shows the key itself (142230940), while an
    // empty authored localisation stays empty: neither drops the type row.
    assert!(
        flag_description(
            &side,
            &opponent,
            &tokens,
            &sub_units,
            Ok("BBB"),
            &NoIcons,
            &loc
        )
        .unwrap()
        .contains("[destroyer 2.00 0.20]")
    );
}

#[test]
fn torpedo_hover_reports_damage_history_not_current_attack_counter() {
    let loc: Localisation = [
        ("damage_done_by_weapon_type", "$NUM|0$"),
        ("relative_damage_ratio_to_enemy_health", "Ratio $NUM$"),
        ("damage_dealt_screen", "Screens $NUM|0$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let mut side = NavalCombatSideDetails::default();
    side.damage_history[0][2] = 18_000_000;
    let opponent = NavalCombatSideDetails {
        original_health: 30_000_000,
        ..Default::default()
    };
    // 1417D1A90 -> 14160C600 reads the historical matrix's torpedo column,
    // independently of 1417D9C60's current active-member attack summary.
    assert_eq!(
        weapon_description(&side, &opponent, 2, None, &NoIcons, &loc).unwrap(),
        "180\nRatio 0.60\nScreens 180"
    );
}

#[test]
fn weapon_damage_summary_includes_piercing_in_native_order() {
    let loc: Localisation = [
        ("damage_done_by_weapon_type", "Total $NUM$"),
        ("relative_damage_ratio_to_enemy_health", "Enemy $NUM$"),
        ("damage_dealt_screen", "Screen $NUM$ $RATIO$"),
        ("NAVY_PIERCING_TOOLTIP_INFO", "Info"),
        (
            "NAVAL_PIERCING_THRESHOLD_TO_DAMAGE",
            "T$THRESHOLD$ D$DAMAGE$;",
        ),
        (
            "NAVAL_PIERCING_THRESHOLD_TO_DAMAGE_LOWEST",
            "L$THRESHOLD$ D$DAMAGE$;",
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let mut side = NavalCombatSideDetails::default();
    side.damage_history[0][0] = 100_000;
    let opponent = NavalCombatSideDetails {
        original_health: 400_000,
        ..Default::default()
    };
    let defines = installed_content::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![100_000],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing_thresholds: vec![40_000, 60_000],
        piercing_critical_values: vec![10_000, 20_000],
        positioning_impacts: [0; 3],
    };
    assert_eq!(
        weapon_description(&side, &opponent, 0, Some(&defines), &NoIcons, &loc).unwrap(),
        "Total 1.00\nEnemy 0.25\nScreen 1.00 1.00\n\nInfo\nT0.40 D1.10;\nL0.40 D1.20;"
    );
    let mut invalid = defines.clone();
    invalid.piercing_critical_values.truncate(1);
    assert_eq!(
        weapon_description(&side, &opponent, 0, Some(&invalid), &NoIcons, &loc),
        Err("NNavy piercing threshold arrays differ or have fewer than two entries".into())
    );
}

#[test]
fn positioning_hover_has_native_heading_terms_and_low_positioning_consequences() {
    let loc: Localisation = [
        ("POSITIONING_INFO", "Positioning"),
        ("MODIFIER_BASE_VALUE_PERCENTAGE", "Base $VALUE$"),
        ("CONVOY_ESCORT_POSITIONING_BONUS", "Escort $VALUE$"),
        (
            "POSITIONING_SURFACE_DETECTION_SUPERIORITY",
            "Detection $NUM$",
        ),
        ("POSITIONING_SHIP_RATIO_PENALTY", "Ship $NUM$"),
        ("POSITIONING_CARRIER_RATIO_PENALTY", "Carrier $NUM$"),
        ("POSITIONING_RECENTLY_JOINED_SHIPS", "Recent $NUM$"),
        ("POSITIONING_PENALTY_INFO", "Penalty"),
        ("BM_ATTACK_VALUE", "Attack $VALUE$"),
        ("SCREENING_VALUE", "Screen $VALUE$"),
        ("AA_MODIFIER", "AA $VALUE$"),
        ("STAT_NAVY_SUB_VISIBILITY", "Sub "),
        ("STAT_NAVY_SUB_VISIBILITY_MOD_DIFF", "$VALUE$"),
    ]
    .into_iter()
    .map(|(key, text)| (key.to_owned(), text.to_owned()))
    .collect();
    let terms = super::super::naval_positioning::Terms {
        value: 50_000,
        base: 40_000,
        escort: 10_000,
        detection: 20_000,
        ship_ratio: 10_000,
        carrier_ratio: 5_000,
        recent_join: 5_000,
        early_defender: true,
    };
    let defines = installed_content::naval_combat_defines::Defines {
        summary_fixed: {
            let mut values = [0; 14];
            values[7] = 20_000;
            values
        },
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing_thresholds: vec![],
        piercing_critical_values: vec![],
        positioning_impacts: [40_000, 60_000, 30_000],
    };
    assert_eq!(
        positioning_description(
            terms,
            "Country +0.03\nWeather -0.01\n",
            &defines,
            &NoIcons,
            &loc
        )
        .unwrap(),
        "Positioning\nBase 0.40\nCountry +0.03\nWeather -0.01\nEscort 0.10\nDetection 0.20\nShip -0.10\nCarrier -0.05\nRecent -0.05\n\nPenalty\nAttack -0.20\nScreen -0.10\nAA -0.30\nSub 0.15\n"
    );
}

#[test]
fn positioning_hover_reads_captured_side_trees_in_native_source_order() {
    let tree = |scope, key, name: &str| replay::rows::NavalModifierBreakdownRow {
        uses: vec![replay::rows::NavalModifierUse {
            combat: 42,
            side: 1,
            battle_slot: u32::MAX,
            source_scope: scope,
            ..Default::default()
        }],
        key,
        nodes: vec![
            replay::rows::ModifierBreakdownNode {
                children: 1,
                ..Default::default()
            },
            replay::rows::ModifierBreakdownNode {
                value: 20_000,
                name: name.as_bytes().to_vec(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let sources = [
        tree(0, 0x1b2, "Country"),
        tree(6, 0x1b2, "Weather"),
        tree(5, 0x1b2, "Sea"),
        tree(9, 0x1b2, "Admiral"),
        tree(0, 0x5a, "Early defender"),
    ];
    let sources = sources.iter().map(test_tree_use).collect::<Vec<_>>();
    let definitions = [0x1b2, 0x5a].map(|key| replay::rows::ModifierDefinitionRow {
        index: key,
        precision: 2,
        ..Default::default()
    });
    let mut loc = Localisation::default();
    loc.language = "english".into();
    assert_eq!(
        positioning_sources(
            42,
            1,
            false,
            &sources,
            &definitions,
            "english",
            &NoIcons,
            &loc
        )
        .unwrap(),
        concat!(
            "Country: \u{11}R+0.20\u{11}!\n",
            "Weather: \u{11}R+0.20\u{11}!\n",
            "Sea: \u{11}R+0.20\u{11}!\n",
            "Admiral: \u{11}R+0.20\u{11}!\n"
        )
    );
    assert_eq!(
        positioning_sources(
            42,
            1,
            true,
            &sources,
            &definitions,
            "english",
            &NoIcons,
            &loc
        )
        .unwrap(),
        concat!(
            "Country: \u{11}R+0.20\u{11}!\n",
            "Weather: \u{11}R+0.20\u{11}!\n",
            "Sea: \u{11}R+0.20\u{11}!\n",
            "Admiral: \u{11}R+0.20\u{11}!\n",
            "Early defender: \u{11}R+0.20\u{11}!\n"
        )
    );
}

#[test]
fn screen_hover_keeps_country_correction_before_the_zero_screens_message() {
    let mut loc: Localisation = [
        ("SCREENING_TOOLTIP_SCREEN_BOX", "Screens $NUM$"),
        ("SCREENING_TOOLTIP_NO_SCREENS", "No screens"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    loc.language = "english".into();
    let correction = ship_modifier_breakdown(
        &[
            replay::rows::ModifierBreakdownNode {
                children: 1,
                ..Default::default()
            },
            replay::rows::ModifierBreakdownNode {
                value: -10_000,
                name: b"Country".to_vec(),
                ..Default::default()
            },
        ],
        &replay::rows::ModifierDefinitionRow {
            index: 0x1b3,
            precision: 2,
            ..Default::default()
        },
        super::super::fixed::ONE,
        "",
        "english",
        &NoIcons,
        &loc,
        true,
    )
    .unwrap();
    let census = Census::default();
    let screening = Screening {
        capital: 0,
        carrier: 0,
        country_bonus_applied: false,
    };
    let defines = installed_content::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing_thresholds: vec![],
        piercing_critical_values: vec![],
        positioning_impacts: [0; 3],
    };
    assert_eq!(
        screening_description(
            false,
            &census,
            &screening,
            &defines,
            &correction,
            Err("not read".into()),
            Err("not read".into()),
            &NoIcons,
            &loc
        )
        .unwrap()
        .0,
        "Country: \u{11}G-0.10\u{11}!\nScreens 0.00\n\nNo screens"
    );
}

#[test]
fn native_stat_headers_pair_text_and_background_with_delayed_damage() {
    let loc: Localisation = [
        (
            "combat_stat_light_guns_info".to_owned(),
            "Light guns".to_owned(),
        ),
        (
            "screening_efficiency_info".to_owned(),
            "Screening".to_owned(),
        ),
        ("combat_stat_heavy_guns_info".into(), "Heavy guns".into()),
        ("combat_stat_torpedoes_info".into(), "Torpedoes".into()),
        (
            "combat_stat_depth_charges_info".into(),
            "Depth charges".into(),
        ),
        ("damage_done_by_weapon_type".into(), "Total $NUM$".into()),
        (
            "relative_damage_ratio_to_enemy_health".into(),
            "Enemy $NUM$".into(),
        ),
        ("damage_dealt_screen".into(), "Screens $NUM$ $RATIO$".into()),
    ]
    .into_iter()
    .collect();
    let mut out = Bindings::default();
    let mut side = NavalCombatSideDetails::default();
    side.damage_history[0][2] = 250_000;
    let opponent = NavalCombatSideDetails {
        original_health: 1_000_000,
        ..Default::default()
    };
    side_hovers(&mut out, &NoIcons, &loc, 42, "left", &side, &opponent, None);
    let text = &out.values["header_container/left_wnd/stat_light_gun"];
    let background = &out.values["header_container/left_wnd/stat_light_gun_bg"];
    assert_eq!(text.tooltip.as_deref(), Some("Light guns"));
    assert_eq!(background.tooltip, text.tooltip);
    assert!(background.hover_owned);
    assert!(background.tooltip_description.is_none());
    assert_eq!(
        out.values["header_container/left_wnd/stat_torpedo"]
            .tooltip_description
            .as_deref(),
        Some("Total 2.50\nEnemy 0.25\nScreens 2.50 1.00")
    );
    assert_eq!(
        out.values["left_wnd_capital_screening_efficiency"]
            .tooltip
            .as_deref(),
        Some("Screening")
    );
    assert_eq!(out.refused().len(), 2);
    assert!(
        out.refused()
            .keys()
            .any(|reason| reason.contains("NNavy piercing defines"))
    );
}
