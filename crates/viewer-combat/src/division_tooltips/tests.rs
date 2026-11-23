use super::*;
use replay_data::rows::Recorded;

fn member(row: &LandCombatDivisionRow) -> LandCombatMember<'_> {
    LandCombatMember {
        row,
        values: row.own.as_ref(),
    }
}
#[test]
fn the_reserve_chance_arm_sums_the_recorded_operands_and_rejections_keep_their_text() {
    use replay_data::rows::LandDivisionModifier;
    let loc: Localisation = [
        ("BATTLE_REINFORCE_CHANCE2", "Chance $VALUE|3$ "),
        ("COMREIC_TOO_MUCH_WIDTH", "Too wide"),
        ("MODIFIER_BASE_VALUE_PERCENTAGE", "Base $VALUE|3$"),
        ("STAT_ARMY_INITIATIVE", "Initiative"),
        ("STAT_ARMY_INITIATIVE_DIFF", " $VALUE|3$"),
        ("SPEED_REINFORCEMENT_BONUS", "Speed $VAL|3$"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    assert_eq!(
        reinforcement_text(
            &NoIcons,
            &loc,
            &Reinforcement::Rejected("COMREIC_TOO_MUCH_WIDTH"),
            None,
        ),
        Ok(("Chance 0.000 Too wide\n".into(), Vec::new()))
    );
    let defines = LandDefines {
        reinforce: super::super::defines::Reinforce {
            chance: 2_000,
            initiative_factor: 25_000,
            speed_bonus: 1_000,
            ..Default::default()
        },
        ..Default::default()
    };
    let present = |raw| LandDivisionModifier { raw, present: 1 };
    let mut row = LandCombatDivisionRow {
        role: 2,
        reinforcement_speed: Some(50_000),
        own: Some(replay_data::rows::LandCombatMemberValues {
            initiative: 50_000,
            ..Default::default()
        }),
        ..Default::default()
    };
    row.context.country_modifiers[0] = present(5_000);
    row.context.country_modifiers[2] = present(20_000);
    row.context.leader_modifier_kind = 1;
    row.context.leader_modifiers[0] = present(3_000);
    row.context.leader_modifiers[2] = present(10_000);
    let condition = Condition {
        strength: ADVANCE_STRENGTH,
        organisation: ADVANCE_ORGANISATION,
        ..Default::default()
    };
    let chance = |row: &LandCombatDivisionRow| {
        reinforcement(&member(row), &condition, Ok((100_000, 0)), Ok(&defines))
    };
    let result = chance(&row).unwrap();
    assert_eq!(
        result,
        Reinforcement::Chance {
            value: 27_000,
            base: 2_000,
            initiative: 16_500,
            speed: 500,
            leader: 3_000,
        }
    );
    let sources = |country: Result<&str, &str>, commander: Option<Result<(&str, &str), &str>>| {
        Some(ReserveSources {
            country: country.map(str::to_owned).map_err(str::to_owned),
            commander: commander.map(|commander| {
                commander
                    .map(|(name, text)| (name.to_owned(), text.to_owned()))
                    .map_err(str::to_owned)
            }),
        })
    };
    assert_eq!(
        reinforcement_text(
            &NoIcons,
            &loc,
            &result,
            sources(Ok("Ally: 5%\n"), Some(Ok(("Rommel", "Trait: 3%\n"))))
        ),
        Ok((
            "Chance 0.270 Base 0.020\nAlly: 5%\nInitiative 0.165\nSpeed 0.005\nRommel: Trait: 3%\n"
                .into(),
            Vec::new()
        ))
    );
    assert_eq!(
        reinforcement_text(
            &NoIcons,
            &loc,
            &result,
            sources(Err("no country"), Some(Err("no commander")))
        ),
        Ok((
            "Chance 0.270 Base 0.020\nInitiative 0.165\nSpeed 0.005\n".into(),
            vec!["no country".to_owned(), "no commander".to_owned()]
        ))
    );
    row.context.leader_modifiers[0] = present(0);
    let result = chance(&row).unwrap();
    assert_eq!(
        reinforcement_text(&NoIcons, &loc, &result, sources(Ok(""), None)),
        Ok((
            "Chance 0.240 Base 0.020\nInitiative 0.165\nSpeed 0.005\n".into(),
            Vec::new()
        ))
    );
    row.context.leader_modifier_kind = 0;
    row.reinforcement_speed = Some(0);
    assert_eq!(
        chance(&row),
        Ok(Reinforcement::Chance {
            value: 22_000,
            base: 2_000,
            initiative: 15_000,
            speed: 0,
            leader: 0,
        })
    );
    row.reinforcement_speed = None;
    let result = chance(&row).unwrap();
    assert_eq!(
        reinforcement_text(&NoIcons, &loc, &result, None),
        Err(RESERVE_SPEED_REFUSED.into())
    );
    let tired = Condition {
        organisation: ADVANCE_ORGANISATION - 1,
        ..condition
    };
    row.reinforcement_speed = None;
    assert_eq!(
        reinforcement(&member(&row), &tired, Ok((100_000, 0)), Ok(&defines)),
        Ok(Reinforcement::Rejected(COMREIC_NOT_FIGHT))
    );
}

use viewer_foundation::tooltip_text::NoIcons;

#[test]
fn refused_stat_and_reserve_hovers_name_their_key() {
    let loc: Localisation = [
        ("STAT_ARMY_SOFT_ATTACK", "Soft attack"),
        ("STAT_ARMY_SOFT_ATTACK_VALUE", "$VALUE$"),
        ("COMBAT_STAT", "$STAT_NAME$: $VALUE$"),
        ("TOTAL_MODIFIER_IS", "Total $VALUE$"),
        ("BATTLE_REINFORCE_CHANCE2", "Chance $VALUE$"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    let modifiers = LandCombatDivisionModifierRow::default();
    assert_eq!(
        combat_stat(
            &NoIcons,
            &loc,
            "STAT_ARMY_SOFT_ATTACK",
            [1, 1],
            &modifiers,
            false,
            Err("x")
        ),
        Ok("Soft attack: 0.00
BASE_VALUE
Total 0.00
"
        .into())
    );
    assert_eq!(
        reinforcement_text(
            &NoIcons,
            &loc,
            &Reinforcement::Rejected("COMREIC_NOT_FIGHT"),
            None,
        ),
        Ok((
            "Chance 0.00COMREIC_NOT_FIGHT
"
            .into(),
            Vec::new()
        ))
    );
    let flagged: Localisation = [("BATTLE_REINFORCE_CHANCE2", "@GER")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    assert_eq!(
            reinforcement_text(&NoIcons, &flagged, &Reinforcement::Rejected("X"), None),
            Err(
                "localisation BATTLE_REINFORCE_CHANCE2 holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw"
                    .into()
            )
        );
}
use replay_data::{rows::DivisionMovementRow, rows::DivisionRow, rows::ProvinceRow};

#[test]
fn invasion_section_follows_the_native_order_and_branches() {
    let loc: Localisation = [
        ("NAVAL_INVASION_COMBAT_PROGRESS_DESC", "Landing fixture."),
        ("NAVAL_INVASION_PROGRESS", "Landing ratio: $VALUE|Y0%$"),
        (
            "NAVAL_INVASION_PROGRESS_COMPLETE",
            "Landing ratio: $VALUE|Y0%$
(Fixture complete).
",
        ),
        (
            "NAVAL_INVASION_ATTACK_SCALE",
            "Offence factor: $VALUE|=+0%$",
        ),
        ("NAVAL_INVASION_DEFEND_SCALE", "Guard factor: $VALUE|=+0%$"),
    ]
    .iter()
    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
    .collect();
    let neutral = [0, 0, 100_000, 100_000];
    let partial = invasion_section(&NoIcons, &loc, 50_000, neutral).unwrap();
    assert!(partial.starts_with(
        "Landing fixture.

Landing ratio: "
    ));
    assert!(partial.ends_with(
        "

"
    ));
    assert!(!partial.contains("Offence factor"));
    let complete = invasion_section(&NoIcons, &loc, 4_294_967_295, neutral).unwrap();
    assert!(complete.contains("(Fixture complete)"));
    let scaled = invasion_section(&NoIcons, &loc, 100_000, [0, 0, 50_000, 125_000]).unwrap();
    let attack = scaled
        .find(
            "
Offence factor: ",
        )
        .unwrap();
    let defense = scaled
        .find(
            "
Guard factor: ",
        )
        .unwrap();
    assert!(attack < defense);
    assert_eq!(
        invasion_section(&NoIcons, &Localisation::default(), 0, neutral),
        Ok("NAVAL_INVASION_COMBAT_PROGRESS_DESC\n\nNAVAL_INVASION_PROGRESS\n\n".into())
    );
}

#[test]
fn the_division_summary_names_its_missing_or_unrenderable_key() {
    let entries = |extra: &[(&str, &str)]| -> Localisation {
        [
            ("COMBAT_DIVISION_ATTACKER", "$NAME$"),
            ("NAVAL_INVASION_COMBAT_PROGRESS_DESC", "Landing fixture."),
            ("NAVAL_INVASION_PROGRESS", "$VALUE|Y0%$"),
            ("STAT_COMMON_ARMOR", "Armor"),
            ("COMBAT_STAT", "$STAT_NAME$ $VALUE$"),
            ("BASE_VALUE", "$VALUE$"),
            ("HARDNESS_COMBAT_DESC", "$HARDNESS$"),
            ("COMBAT_TOOLTIP_SEPARATOR", "--"),
            ("STAT_COMMON_PIERCING", "Piercing"),
            ("PIERCING_COMBAT_DESC", "Pierce"),
        ]
        .iter()
        .chain(extra)
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
    };
    let row = LandCombatDivisionRow {
        scales: [0, 0, 50_000, 100_000],
        own: Some(replay_data::rows::LandCombatMemberValues {
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(
        division_summary(
            &NoIcons,
            &entries(&[]),
            "1st",
            &member(&row),
            Some(10_000),
            None
        )
        .unwrap()
        .contains("\nNAVAL_INVASION_ATTACK_SCALE\nNAVAL_INVASION_DEFEND_SCALE\n")
    );
    let flagged = entries(&[
        ("NAVAL_INVASION_ATTACK_SCALE", "@GER"),
        ("NAVAL_INVASION_DEFEND_SCALE", "$VALUE$"),
    ]);
    assert_eq!(
            division_summary(&NoIcons, &flagged, "1st", &member(&row), Some(10_000), None),
            Err(
                "localisation NAVAL_INVASION_ATTACK_SCALE holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw"
                    .into()
            )
        );
    let complete = entries(&[
        ("NAVAL_INVASION_ATTACK_SCALE", "$VALUE$"),
        ("NAVAL_INVASION_DEFEND_SCALE", "$VALUE$"),
    ]);
    assert!(
        division_summary(
            &NoIcons,
            &complete,
            "1st",
            &member(&row),
            Some(10_000),
            None
        )
        .is_ok()
    );
}

#[test]
fn naval_invasion_needs_a_sea_to_land_step_against_an_enemy_controller() {
    let mut movement = [DivisionMovementRow {
        division: 7,
        path: vec![2],
        path_total: 1,
        ..Default::default()
    }];
    let divisions = [DivisionRow {
        id: 7,
        province: 1,
        logical_owner: 5,
        ..Default::default()
    }];
    let provinces = [ProvinceRow {
        id: 2,
        controller: 6,
        ..Default::default()
    }];
    let links = [
        replay_data::rows::CountryLinkRow {
            id: 5,
            canonical_slot: 5,
        },
        replay_data::rows::CountryLinkRow {
            id: 6,
            canonical_slot: 6,
        },
    ];
    let war = [replay_data::rows::RelationTupleRow {
        type_id: engine_native::cwar_relation::TYPE,
        first_country: 6,
        second_country: 5,
        ..Default::default()
    }];
    let invasion = |movement: &[DivisionMovementRow], sea: bool, relations: &[_]| {
        let wars = Wars::new(
            &Available::Complete(&links),
            &Available::Complete(relations),
            Vec::new(),
        );
        naval_invasion(
            7,
            &Available::Complete(movement),
            &Available::Complete(&divisions),
            &Available::Complete(&provinces),
            &|province| {
                Some(if province == 1 && sea {
                    ProvinceKind::Sea
                } else {
                    ProvinceKind::Land
                })
            },
            &wars,
        )
    };
    assert_eq!(invasion(&movement, false, &war), Ok(false));
    assert_eq!(invasion(&movement, true, &war), Ok(true));
    assert_eq!(invasion(&movement, true, &[]), Ok(false));
    let mut retired = war.clone();
    retired[0].retired = 1;
    assert_eq!(invasion(&movement, true, &retired), Ok(false));
    let mut unlinked = war.clone();
    unlinked[0].first_country = 99;
    assert!(
        invasion(&movement, true, &unlinked)
            .unwrap_err()
            .contains("without a CountryLinks row")
    );
    let both = [unlinked[0].clone(), war[0].clone()];
    assert_eq!(invasion(&movement, true, &both), Ok(true));
    movement[0].path.clear();
    assert_eq!(
        invasion(&movement, true, &war),
        Err("its route's next step is not recorded".into())
    );
    movement[0].path_total = 0;
    assert_eq!(invasion(&movement, true, &war), Ok(false));
    movement[0].path_total = 1;
    let wars = Wars::new(
        &Available::Complete(&links),
        &Available::Truncated {
            rows: &war,
            missing: 1,
        },
        vec!["Provinces"],
    );
    assert_eq!(
        naval_invasion(
            7,
            &Available::Complete(&movement),
            &Available::Complete(&divisions),
            &Available::Complete(&provinces),
            &|_| Some(ProvinceKind::Land),
            &wars,
        ),
        Err("the Provinces, RelationTuples tables are incomplete this hour".into())
    );
}

#[test]
fn strength_uses_native_fixed_arithmetic_and_cap() {
    assert_eq!(combat_strength(150_000, 200_000, 120_000), Some(90_000));
    assert_eq!(combat_strength(300_000, 200_000, 120_000), Some(100_000));
    assert_eq!(combat_strength(-1, 200_000, 120_000), Some(0));
    assert_eq!(combat_strength(i64::MAX, 200_000, 120_000), Some(100_000));
}

#[test]
fn organization_is_preformatted_text_not_localization_integer_precision() {
    assert_eq!(one_decimal(1_234_567), "12.3");
    assert_eq!(one_decimal(-19_999), "-0.1");
    assert_eq!(one_decimal(-1), "-0.0");
    assert_eq!(one_decimal(214_748_364_800_000), "-2147483648.0");
}

#[test]
fn a_stat_headline_formats_as_the_native_trimmed_formatter() {
    assert_eq!(fixed_decimal(150_000), "1.5");
    assert_eq!(fixed_decimal(100_000), "1");
    assert_eq!(fixed_decimal(-1_234_567), "-12.34567");
    assert_eq!(fixed_decimal(214_748_364_800_000), "-2147483648");
    assert_eq!(
        fixed_decimal(i64::MIN),
        format!("-{}.4192", (i64::MIN / 100_000) as i32)
    );
}

#[test]
fn reserve_rejection_precedes_movement_inputs() {
    let mut row = LandCombatDivisionRow::default();
    let mut condition = Condition::default();
    assert_eq!(
        reinforcement(&member(&row), &condition, Err("unused"), Err("unused")),
        Ok(Reinforcement::Rejected("COMREIC_NOT_FIGHT"))
    );
    condition.strength = ADVANCE_STRENGTH;
    condition.organisation = ADVANCE_ORGANISATION;
    row.context.template = 1;
    row.width_terms
        .push(replay_data::rows::LandDivisionWidthTerm {
            subunit: 1,
            count: 1,
            width: 200_000,
        });
    let mut defines = LandDefines::default();
    defines.reinforce.over_width_penalty = -200_000;
    defines.reinforce.over_width_penalty_max = -100_000;
    assert_eq!(
        reinforcement(&member(&row), &condition, Ok((100_000, 0)), Ok(&defines)),
        Ok(Reinforcement::Rejected("COMREIC_TOO_MUCH_WIDTH"))
    );
    assert_eq!(
        reinforcement(
            &member(&row),
            &condition,
            Ok((100_000, 0)),
            Err("no NMilitary")
        ),
        Err("the land combat defines are unavailable (no NMilitary)".into())
    );
    assert_eq!(
        reinforcement(
            &member(&row),
            &condition,
            Err("width unknown"),
            Ok(&defines)
        ),
        Err("width unknown".into())
    );
}

#[test]
fn the_commander_icon_reads_the_hours_unit_leader_and_its_traits() {
    use replay_data::{rows::UnitLeaderRow, rows::UnitLeaderTraitRow};
    let handle = (7_u64 << 32) | 0x55;
    let leaders = [
        UnitLeaderRow {
            id: 8,
            reference_type: 0x55,
            skill_status: 2,
            skill_level: 1,
            inherited_text_a: b"other".to_vec(),
            ..Default::default()
        },
        UnitLeaderRow {
            id: 7,
            reference_type: 0x55,
            skill_status: 1,
            skill_level: 4,
            inherited_text_a: b"rommel".to_vec(),
            inherited_text_b: b"Localised cache".to_vec(),
            ..Default::default()
        },
    ];
    let trait_row = |leader_reference, ordinal, token: &[u8]| UnitLeaderTraitRow {
        leader_reference,
        ordinal,
        token: token.to_vec(),
        ..Default::default()
    };
    let traits = [
        trait_row(handle, 1, b"panzer_leader"),
        trait_row((8 << 32) | 0x55, 0, b"unrelated"),
        trait_row(handle, 0, b"desert_fox"),
        trait_row(handle, 2, b"desert_fox"),
    ];
    let complete: Available<'_, tables::UnitLeaders> = Available::Complete(&leaders);
    let all_traits: Available<'_, tables::UnitLeaderTraits> = Available::Complete(&traits);
    let commander = division_commander(handle, Ok(&complete), Ok(&all_traits)).unwrap();
    assert_eq!(
        commander,
        DivisionCommander {
            name_token: b"rommel",
            skill: 4,
            traits: vec![
                &b"desert_fox"[..],
                &b"panzer_leader"[..],
                &b"desert_fox"[..]
            ],
        }
    );
    let loc: Localisation = [
        ("COMBAT_OTHER_COMMANDER_DESC", "$NAME$ ($SKILL$)"),
        ("COMBAT_TRAIT_LIST_HEADER", ": "),
        ("COMBAT_DETAIL_TRAIT_SEPARATOR", ", "),
        ("COMBAT_DETAIL_TRAIT_NAME", "$NAME$"),
        ("DIFFERENT_COMMANDER_DESC", "Not the army's commander"),
        ("rommel", "Rommel"),
        ("desert_fox", "Desert Fox"),
        ("panzer_leader", "Panzer Leader"),
        ("GER_panzer_leader", "Panzerführer"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    assert_eq!(
        commander_icon_tooltip(&NoIcons, &loc, Ok("GER"), &commander),
        Ok((
            "Rommel (4): Desert Fox, Panzerführer, Desert Fox".into(),
            Some(Ok("Not the army's commander".into()))
        ))
    );
    let bare = DivisionCommander {
        traits: Vec::new(),
        ..commander
    };
    assert_eq!(
        commander_icon_tooltip(&NoIcons, &loc, Ok("GER"), &bare)
            .unwrap()
            .0,
        "Rommel (4)"
    );
    assert_eq!(
        division_commander((9 << 32) | 0x55, Ok(&complete), Ok(&all_traits)),
        Err(format!(
            "the division commander {} has no UnitLeaders row this hour",
            (9_u64 << 32) | 0x55
        ))
    );
    let null_skill = [UnitLeaderRow {
        skill_status: 0,
        ..leaders[1].clone()
    }];
    let null_skill: Available<'_, tables::UnitLeaders> = Available::Complete(&null_skill);
    assert!(
        division_commander(handle, Ok(&null_skill), Ok(&all_traits))
            .unwrap_err()
            .contains("records no skill object")
    );
    let truncated: Available<'_, tables::UnitLeaderTraits> = Available::Truncated {
        rows: &traits,
        missing: 1,
    };
    assert!(
        division_commander(handle, Ok(&complete), Ok(&truncated))
            .unwrap_err()
            .starts_with("UnitLeaderTraits is incomplete or unavailable this hour")
    );
    assert_eq!(
        division_commander(handle, Err("decode failed"), Ok(&all_traits)),
        Err("UnitLeaders is unreadable (decode failed)".into())
    );
}

#[test]
fn reserve_sources_join_the_recorded_country_and_commander_trees() {
    use replay_data::{
        rows::DivisionRow, rows::ModifierBreakdownNode, rows::ModifierDefinitionRow,
        rows::ReserveModifierTreeRow, rows::TableId, rows::TableRows,
    };
    use viewer_foundation::test_support::{test_frame, test_put};
    let mut loc = Localisation::default();
    loc.language = "english".into();
    let leaf = |value: i64, name: &[u8]| {
        vec![
            ModifierBreakdownNode {
                children: 1,
                value,
                name: Vec::new(),
            },
            ModifierBreakdownNode {
                children: 0,
                value,
                name: name.to_vec(),
            },
        ]
    };
    let mut frame = test_frame(5);
    test_put(
        &mut frame,
        TableRows::ModifierDefinitions(vec![ModifierDefinitionRow {
            index: engine_executable::modifier_keys::LAND_REINFORCE_RATE,
            precision: 2,
            ..Default::default()
        }]),
    );
    test_put(
        &mut frame,
        TableRows::Divisions(vec![
            DivisionRow {
                id: 11,
                roster_owner: 4,
                expeditionary_owner: 0,
                ..Default::default()
            },
            DivisionRow {
                id: 12,
                roster_owner: 4,
                expeditionary_owner: 9,
                ..Default::default()
            },
        ]),
    );
    let tree = |kind, owner, name: &[u8], value, label: &[u8]| ReserveModifierTreeRow {
        kind,
        owner,
        name: name.to_vec(),
        nodes: Some(leaf(value, label)),
    };
    test_put(
        &mut frame,
        TableRows::ReserveModifierTrees(vec![
            tree(0, 4, b"", 5_000, b"Home"),
            tree(0, 9, b"", 7_000, b"Host"),
            tree(1, 77, b"Rommel", 3_000, b"Trait"),
        ]),
    );
    let divisions = rows::<tables::Divisions>(&frame).unwrap();
    let mut row = LandCombatDivisionRow {
        division: 11,
        commander: 77,
        own: Some(replay_data::rows::LandCombatMemberValues {
            ..Default::default()
        }),
        ..Default::default()
    };
    row.context.leader_modifier_kind = 1;
    let text = |row: &LandCombatDivisionRow, leader, loc: &Localisation| {
        let sources = reserve_sources(
            &frame,
            crate::naval_labels::english(),
            row,
            &divisions,
            &NoIcons,
            loc,
            leader,
        );
        (sources.country, sources.commander)
    };
    let (country, commander) = text(&row, 3_000, &loc);
    assert!(country.unwrap().starts_with("Home: "));
    let (name, sources) = commander.unwrap().unwrap();
    assert_eq!(name, "Rommel");
    assert!(sources.starts_with("Trait: "));
    row.division = 12;
    let (country, commander) = text(&row, 0, &loc);
    assert!(country.unwrap().starts_with("Host: "));
    assert!(commander.is_none());
    row.commander = 78;
    let (_, commander) = text(&row, 1, &loc);
    assert!(
        commander
            .unwrap()
            .unwrap_err()
            .contains("no row for the reserve's commander")
    );
    let mut german = loc.clone();
    german.language = "german".into();
    let (country, _) = text(&row, 0, &german);
    assert!(country.unwrap_err().contains("stored in english"));
    let mut refused = test_frame(5);
    for table in [TableId::ModifierDefinitions, TableId::Divisions] {
        refused.tables[table.index()] = frame.tables[table.index()].clone();
    }
    test_put(
        &mut refused,
        TableRows::ReserveModifierTrees(vec![
            tree(0, 4, b"", 5_000, b"Home"),
            ReserveModifierTreeRow {
                kind: 0,
                owner: 9,
                ..Default::default()
            },
            tree(1, 77, b"Rommel", 3_000, b"Trait"),
        ]),
    );
    let refused_divisions = rows::<tables::Divisions>(&refused).unwrap();
    row.commander = 77;
    let sources = reserve_sources(
        &refused,
        crate::naval_labels::english(),
        &row,
        &refused_divisions,
        &NoIcons,
        &loc,
        1,
    );
    assert!(sources.country.unwrap_err().contains("refused at capture"));
    assert_eq!(sources.commander.unwrap().unwrap().0, "Rommel");
}

#[test]
fn division_experience_hover_uses_localized_levels_and_exact_progress() {
    let loc: Localisation = [
        ("UNIT_LEVEL_7", "Custom rank"),
        (
            "EXP_TOOLTIP_ARMY",
            "Rank $NUMBER$ $LEVEL$ $COMBAT$ progress $EXP|%0$",
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let text = division_experience_tooltip(
        &NoIcons,
        &loc,
        content_reader_key_names::unit_leader::DivisionExperienceProgress {
            level: 7,
            next_level_fraction: 25_000,
            combat_bonus: -12_500,
        },
    )
    .unwrap();
    assert!(text.contains("Rank 8 Custom rank"), "{text}");
    assert!(text.contains("25%"), "{text}");
    assert!(text.contains("-12%"), "{text}");
}

#[test]
fn both_conversion_products_resolve_country_specific_template_name_keys() {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::DivisionTemplateIconRow,
        rows::DivisionTemplateRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};
    let loc: Localisation = [
        ("KEY".into(), "Generic".into()),
        ("AAA_KEY".into(), "Country name".into()),
    ]
    .into_iter()
    .collect();
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 0,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    for rows in [
        TableRows::DivisionTemplates(vec![DivisionTemplateRow {
            template: 7,
            name_key: Recorded::Value(b"KEY".to_vec()),
            ..Default::default()
        }]),
        TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
            template: 7,
            country: 9,
            ..Default::default()
        }]),
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 9,
            canonical_slot: 3,
        }]),
        TableRows::Countries(vec![CountryRow {
            id: 9,
            canonical_slot: 3,
            tag: b"AAA".to_vec(),
            ..Default::default()
        }]),
    ] {
        let index = rows.table().index();
        frame.tables[index] = TableView::Complete(rows).into();
    }
    assert_eq!(
        template_display_name(&loc, &frame, 7).unwrap(),
        "Country name"
    );
    let TableView::Complete(TableRows::DivisionTemplates(rows)) =
        &mut *frame.tables[TableId::DivisionTemplates.index()]
    else {
        panic!()
    };
    rows[0].name = b"Literal".to_vec();
    assert_eq!(template_display_name(&loc, &frame, 7).unwrap(), "Literal");
}
