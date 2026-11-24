use super::*;
use replay_data::{rows::LexicalTokenRow, rows::NavalDamageSource};
use viewer_foundation::tooltip_text::NoIcons;

#[test]
fn flag_damage_summary_includes_naval_bomber_weapon_before_box_categories() {
    let loc: Localisation = [
        ("total_damage_dealt", "Total $NUM|0$;"),
        ("damage_dealt_light_guns", "Light $NUM|0$;"),
        ("damage_dealt_heavy_guns", "Heavy $NUM|0$;"),
        ("damage_dealt_air", "Bombers $NUM|0$ ($RATIO|%0$);"),
        ("damage_dealt_by_box", "BOX;"),
        ("damage_dealt_screen", "Screens $NUM|0$;"),
        ("damage_dealt_battle_box", "Capitals $NUM|0$;"),
        ("damage_dealt_by_ship_types", "TYPES;"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let mut side = NavalCombatSideDetails {
        total_damage: 42_900_000,
        ..Default::default()
    };
    side.damage_history[0][0] = 4_100_000;
    side.damage_history[0][1] = 32_200_000;
    side.damage_history[0][4] = 4_000_000;
    side.damage_history[1][4] = 2_500_000;
    let opponent = NavalCombatSideDetails {
        original_health: 50_000_000,
        ..Default::default()
    };
    assert_eq!(
        flag_description(
            &side,
            &opponent,
            &TokenNames::from_rows(&[], true),
            &SubUnits::default(),
            Ok("AAA"),
            &NoIcons,
            &loc,
        )
        .unwrap(),
        "Total 429;Light 41;Heavy 322;Bombers 65 (15%);\nBOX;Screens 403;Capitals 25;\nTYPES;\n"
    );
}

#[test]
fn flag_damage_summary_uses_native_sections_and_sorted_source_types() {
    content_reader_script::test_game::enter();
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
    content_reader_script::test_game::enter();
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
    let defines = content_reader_naval_combat_defines::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![100_000],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing: Ok(content_reader_naval_combat_defines::naval_combat_defines::Piercing {
            thresholds: vec![40_000, 60_000],
            critical_values: vec![10_000, 20_000],
        }),
        positioning_impacts: [0; 3],
    };
    assert_eq!(
        weapon_description(&side, &opponent, 0, Some(&defines), &NoIcons, &loc).unwrap(),
        "Total 1.00\nEnemy 0.25\nScreen 1.00 1.00\n\nInfo\nT0.40 D1.10;\nL0.40 D1.20;"
    );
    let mut invalid = defines.clone();
    invalid.piercing.as_mut().unwrap().critical_values.truncate(1);
    assert_eq!(
        weapon_description(&side, &opponent, 0, Some(&invalid), &NoIcons, &loc),
        Err("NNavy piercing threshold arrays differ or have fewer than two entries".into())
    );
    let mut unread = defines;
    unread.piercing = Err("missing NDefines.NNavy.NAVY_PIERCING_THRESHOLDS".into());
    assert!(
        weapon_description(&side, &opponent, 0, Some(&unread), &NoIcons, &loc)
            .unwrap_err()
            .contains("missing NDefines.NNavy.NAVY_PIERCING_THRESHOLDS")
    );
    assert!(weapon_description(&side, &opponent, 2, Some(&unread), &NoIcons, &loc).is_ok());
}

fn tree_row(
    kind: replay_data::naval_trees::Kind,
    owner: u64,
    key: u32,
    value: i64,
    name: &[u8],
) -> replay_data::rows::NavalModifierTreeRow {
    replay_data::rows::NavalModifierTreeRow {
        kind: kind as u8,
        owner,
        sub: 0,
        trees: vec![replay_data::rows::NavalModifierKeyTree {
            key,
            nodes: vec![
                replay_data::rows::ModifierBreakdownNode {
                    children: 1,
                    value,
                    name: Vec::new(),
                },
                replay_data::rows::ModifierBreakdownNode {
                    children: 0,
                    value,
                    name: name.to_vec(),
                },
            ],
        }],
    }
}

fn positioning_side(country: u32, commander: u64) -> NavalCombatSideDetails {
    let mut side = NavalCombatSideDetails {
        commander,
        ..Default::default()
    };
    side.summary.countries = vec![country];
    side
}

fn positioning_loc() -> Localisation {
    let mut loc: Localisation = [
        ("POSITIONING_INFO", "Positioning"),
        ("MODIFIER_BASE_VALUE_PERCENTAGE", "Base $VALUE|%0$"),
        ("POSITIONING_PENALTY_INFO", "Penalties"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    loc.language = "english".into();
    loc
}

#[test]
fn positioning_hovers_list_the_side_sources_and_refuse_without_trees() {
    use replay_data::naval_trees::Kind;
    let detail = replay_data::rows::NavalCombatDetailsRow {
        combat: 42,
        sides: [positioning_side(7, 66), positioning_side(8, 77)],
        ..Default::default()
    };
    let rows = [
        tree_row(Kind::Country, 7, 0x1b2, 10_000, b"Naval doctrine"),
        tree_row(Kind::Commander, 66, 0x1b2, 5_000, b"Superior tactician"),
        tree_row(Kind::Commander, 77, 0x5a, -20_000, b"Ambusher"),
    ];
    let definitions = [0x1b2, 0x5a].map(|index| replay_data::rows::ModifierDefinitionRow {
        index,
        precision: 2,
        ..Default::default()
    });
    let trees = crate::naval_trees::Sources::from_parts(
        Ok(&rows),
        &definitions,
        crate::naval_labels::english(),
    );
    let defines = content_reader_naval_combat_defines::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing: Ok(Default::default()),
        positioning_impacts: [0; 3],
    };
    let terms = crate::naval_positioning::Terms {
        value: 100_000,
        base: 100_000,
        escort: 0,
        detection: 0,
        ship_ratio: 0,
        carrier_ratio: 0,
        recent_join: 0,
        early_defender: true,
    };
    let loc = positioning_loc();
    let mut out = Bindings::default();
    positioning_hover(
        &mut out,
        42,
        "right",
        1,
        &detail,
        Some(terms),
        &trees,
        Some(&defines),
        &NoIcons,
        &loc,
    );
    assert!(out.refused().is_empty(), "{:?}", out.refused());
    let tooltip = out.values["header_container/right_wnd/stat_positioning"]
        .tooltip
        .clone()
        .expect("positioning hover");
    let text = format!("{tooltip:?}");
    assert!(text.contains("Positioning"), "{text}");
    assert!(text.contains("Base"), "{text}");
    assert!(!text.contains("Naval doctrine"), "{text}");
    assert!(!text.contains("Superior tactician"), "{text}");
    let mut out = Bindings::default();
    positioning_hover(
        &mut out,
        42,
        "left",
        0,
        &detail,
        Some(terms),
        &trees,
        Some(&defines),
        &NoIcons,
        &loc,
    );
    let text = format!(
        "{:?}",
        out.values["header_container/left_wnd/stat_positioning"].tooltip
    );
    assert!(text.contains("Naval doctrine"), "{text}");
    assert!(text.contains("Superior tactician"), "{text}");
    assert!(text.contains("Ambusher"), "{text}");
    let missing = crate::naval_trees::Sources::from_parts(
        Err("the naval source trees are not available this hour (NavalModifierTrees: not recorded at format 101)".into()),
        &definitions,
        crate::naval_labels::english(),
    );
    let mut out = Bindings::default();
    positioning_hover(
        &mut out,
        42,
        "left",
        0,
        &detail,
        Some(terms),
        &missing,
        Some(&defines),
        &NoIcons,
        &loc,
    );
    for key in [
        "header_container/left_wnd/stat_positioning",
        "header_container/left_wnd/stat_positioning_bg",
    ] {
        assert!(out.values[key].hover_owned);
        assert!(out.values[key].tooltip.is_none());
    }
    assert!(
        out.refused()
            .keys()
            .all(|reason| reason.contains("NavalModifierTrees")),
        "{:?}",
        out.refused()
    );
}

#[test]
fn screen_box_prefixes_the_country_correction_only_on_its_correction_branch() {
    use replay_data::naval_trees::Kind;
    let screening = |capital, country_bonus_applied| Screening {
        capital,
        carrier: 0,
        country_bonus_applied,
    };
    let census = Census::default();
    let submarines = Census {
        eligible_submarines: 1,
        ..Census::default()
    };
    let rows = [tree_row(
        Kind::Country,
        7,
        0x1b3,
        20_000,
        b"Screening doctrine",
    )];
    let definitions = [replay_data::rows::ModifierDefinitionRow {
        index: 0x1b3,
        precision: 2,
        ..Default::default()
    }];
    let trees = crate::naval_trees::Sources::from_parts(
        Ok(&rows),
        &definitions,
        crate::naval_labels::english(),
    );
    let side = positioning_side(7, 66);
    let empty = NavalCombatSideDetails::default();
    let opposing = positioning_side(8, 0);
    let loc = positioning_loc();
    let correction = |carrier, census: &Census, screening: Screening, side| {
        country_correction(
            carrier, census, &screening, 42, side, &opposing, &trees, &NoIcons, &loc,
        )
    };
    for text in [
        correction(false, &census, screening(0, false), &side).unwrap(),
        correction(false, &submarines, screening(60_000, true), &side).unwrap(),
    ] {
        assert!(text.contains("Screening doctrine"), "{text}");
        assert!(text.ends_with('\n'), "{text}");
    }
    assert_eq!(
        correction(true, &census, screening(0, false), &side),
        Ok(String::new())
    );
    assert_eq!(
        correction(false, &census, screening(60_000, false), &side),
        Ok(String::new())
    );
    assert_eq!(
        correction(false, &submarines, screening(0, false), &side),
        Ok(String::new())
    );
    assert_eq!(
        correction(false, &census, screening(0, false), &empty),
        Ok(String::new())
    );
    let missing = crate::naval_trees::Sources::from_parts(
        Err("NavalModifierTrees not recorded".into()),
        &definitions,
        crate::naval_labels::english(),
    );
    assert!(
        country_correction(
            false,
            &census,
            &screening(0, false),
            42,
            &side,
            &opposing,
            &missing,
            &NoIcons,
            &loc
        )
        .unwrap_err()
        .contains("NavalModifierTrees")
    );
    let mut loc: Localisation = [
        ("SCREENING_TOOLTIP_SCREEN_BOX", "Screens $NUM$"),
        ("SCREENING_TOOLTIP_NO_SCREENS", "No screens"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    loc.language = "english".into();
    let defines = content_reader_naval_combat_defines::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing: Ok(Default::default()),
        positioning_impacts: [0; 3],
    };
    assert_eq!(
        screening_description(
            false,
            &submarines,
            &screening(0, false),
            &defines,
            "",
            Err("not read".into()),
            Err("not read".into()),
            &NoIcons,
            &loc
        )
        .unwrap()
        .0,
        "Screens 0.00\n\nNo screens"
    );
}

#[test]
fn screening_hovers_keep_complete_submarine_counts_and_bonuses_without_unneeded_models() {
    let mut side = NavalCombatSideDetails::default();
    side.summary.countries = vec![1];
    side.positioning = 87_850;
    let ships = (0..8)
        .map(|slot| replay_data::rows::NavalCombatShipRow {
            combat: 42,
            side: 0,
            battle_slot: slot,
            group_index: 0,
            state: 2,
            live_ship: 1,
            formation_flags: 1 << 7,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let defines = content_reader_naval_combat_defines::naval_combat_defines::Defines {
        summary_fixed: {
            let mut values = [0; 14];
            values[7] = 50_000;
            values[8] = 300_000;
            values[9] = 200_000;
            values[10] = 100_000;
            values[11] = 500_000;
            values
        },
        summary_integer: [0; 3],
        silhouette_planes: [0; 6],
        screen_cap_reduction_factor: 0,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing: Ok(Default::default()),
        positioning_impacts: [0; 3],
    };
    let (census, screening, _) = screening_inputs(
        42,
        0,
        &side,
        Some(&ships),
        &Available::Unavailable {
            reason: "recorded model failure",
        },
        Some(&defines),
    )
    .unwrap();
    assert_eq!(census.eligible_submarines, 8);
    assert_eq!(census.screens, 0);
    assert_eq!(screening.capital, 100_000);
    assert_eq!(screening.carrier, 100_000);
    assert!(!screening.country_bonus_applied);
    let loc: Localisation = [
        ("SCREENING_TOOLTIP_SCREEN_BOX", "Screening Group: $NUM|%.0H$"),
        ("SCREENING_TOOLTIP_CAPITAL_BOX", "Battle Line: $NUM|%.0H$"),
        ("SCREENING_TOOLTIP_CAPITAL_BOX_COMBINED", "Combined: $NUM|%.0H$"),
        ("SCREENING_TOOLTIP_NO_SCREENS", "No screens warning."),
        ("SCREENING_TOOLTIP_FOR_CAPITAL_BOX", "Carriers $NUM_CARR|.0H$, convoys $NUM_CONV|.0H$, required $NUM_NEED|.0H$, capitals $NUM_CAPI|.0H$."),
        ("SCREENING_TOOLTIP_BONUSES_CAPITAL", "Carrier Group bonuses:\n"),
        ("sortie_bonus", "Sortie efficiency"),
        ("retreat_bonus", "Retreat speed"),
    ]
    .into_iter()
    .map(|(key, text)| (key.to_owned(), text.to_owned()))
    .collect();
    assert_eq!(
        screening_description(
            false,
            &census,
            &screening,
            &defines,
            "",
            Err("not needed".into()),
            Err("not needed".into()),
            &NoIcons,
            &loc
        )
        .unwrap(),
        (
            "Screening Group: \u{11}H100%\u{11}!\n\nNo screens warning.".into(),
            vec![]
        )
    );
    let definitions = [
        replay_data::rows::ModifierDefinitionRow {
            index: 1,
            name_key: b"sortie_bonus".to_vec(),
            flags: 3,
            precision: 0,
            category_mask: 1,
            ..Default::default()
        },
        replay_data::rows::ModifierDefinitionRow {
            index: 2,
            name_key: b"retreat_bonus".to_vec(),
            flags: 3,
            precision: 1,
            category_mask: 1,
            ..Default::default()
        },
    ];
    let definitions = Definitions::from_parts(
        &definitions,
        &[(b"sortie_bonus", 1), (b"retreat_bonus", 2)],
        None,
    );
    let authored = content_reader_parser::parser::parse(b"sortie_bonus=0.1 retreat_bonus=0.2").unwrap();
    let authored = content_reader_script::modifiers::targeted(&authored.items).modifiers;
    assert_eq!(
        screening_description(true, &census, &screening, &defines, "", Ok(&authored), Ok(&definitions), &NoIcons, &loc).unwrap(),
        (concat!("Battle Line: \u{11}H100%\u{11}!\nCombined: \u{11}H100%\u{11}!\n\n",
            "Carriers \u{11}H0\u{11}!, convoys \u{11}H0\u{11}!, required \u{11}H0\u{11}!, capitals \u{11}H0\u{11}!.\n\n",
            "Carrier Group bonuses:\nSortie efficiency: \u{11}G+10%\u{11}!\nRetreat speed: \u{11}G+20.0%\u{11}!").into(), vec![])
    );
    let mut unknown = ships[0].clone();
    unknown.formation_flags = 0;
    assert_eq!(
        screening_inputs(
            42,
            0,
            &side,
            Some(std::slice::from_ref(&unknown)),
            &Available::Complete(&[]),
            Some(&defines)
        )
        .err()
        .as_deref(),
        Some("current live ship has no ShipModels row")
    );
    for (models, expected) in [
        (
            Available::Truncated {
                rows: &[],
                missing: 17,
            },
            "ShipModels is truncated this hour (17 rows not reached)",
        ),
        (
            Available::Unavailable {
                reason: "recorded model failure",
            },
            "ShipModels is unavailable this hour (recorded model failure)",
        ),
    ] {
        assert!(screening_inputs(42, 0, &side, Some(&ships), &models, Some(&defines)).is_ok());
        assert_eq!(
            screening_inputs(
                42,
                0,
                &side,
                Some(std::slice::from_ref(&unknown)),
                &models,
                Some(&defines)
            )
            .err()
            .as_deref(),
            Some(expected),
        );
    }
    assert_eq!(
        screening_inputs(
            42,
            0,
            &side,
            None,
            &Available::Complete(&[]),
            Some(&defines)
        )
        .err()
        .as_deref(),
        Some("NavalCombatShips is incomplete")
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
