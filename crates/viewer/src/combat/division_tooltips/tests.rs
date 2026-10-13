use super::*;
/// The reinforcement chance arm from the recorded initiative, context keys and
/// speed share: REINFORCE_CHANCE, the country's 0xFB, the initiative scaled
/// by both 0x250 keys and the factor, the share times the bonus and the
/// leader's 0xFB. The country's and commander's 0xFB source fragments are
/// the parts it omits by name; a recording without the share refuses the
/// arm, while the rejection arms keep their zero chance and reason.
#[test]
fn the_reserve_chance_arm_sums_the_recorded_operands_and_rejections_keep_their_text() {
    use replay::rows::LandDivisionModifier;
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
            &Reinforcement::Rejected("COMREIC_TOO_MUCH_WIDTH")
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
        initiative: 50_000,
        reinforcement_speed: Recorded::Value(Some(50_000)),
        ..Default::default()
    };
    row.context.country_modifiers[0] = present(5_000);
    row.context.country_modifiers[2] = present(20_000);
    row.context.leader_modifier_kind = 1;
    row.context.leader_modifiers[0] = present(3_000);
    row.context.leader_modifiers[2] = present(10_000);
    // A fighting division within the width reaches the chance arm.
    let condition = Condition {
        strength: ADVANCE_STRENGTH,
        organisation: ADVANCE_ORGANISATION,
        ..Default::default()
    };
    let chance = |row: &LandCombatDivisionRow| {
        reinforcement(row, &condition, Ok((100_000, 0)), Ok(&defines))
    };
    // Initiative: 1.2 x 0.5 = 0.6, x 1.1 = 0.66, x 0.25 = 0.165; speed:
    // 0.5 x 0.01 = 0.005; 0.02 + 0.05 + 0.165 + 0.005 + 0.03 = 0.27.
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
    assert_eq!(
        reinforcement_text(&NoIcons, &loc, &result),
        Ok((
            "Chance 0.270 Base 0.020\nInitiative 0.165\nSpeed 0.005\n".into(),
            vec![COUNTRY_SOURCES_UNRECORDED, COMMANDER_SOURCES_UNRECORDED]
        ))
    );
    // A leader whose 0xFB is zero shows no commander line.
    row.context.leader_modifiers[0] = present(0);
    let result = chance(&row).unwrap();
    assert_eq!(
        reinforcement_text(&NoIcons, &loc, &result),
        Ok((
            "Chance 0.240 Base 0.020\nInitiative 0.165\nSpeed 0.005\n".into(),
            vec![COUNTRY_SOURCES_UNRECORDED]
        ))
    );
    // Without a contextual leader neither its 0x250 nor its 0xFB applies,
    // and a zero term prints no line.
    row.context.leader_modifier_kind = 0;
    row.reinforcement_speed = Recorded::Value(Some(0));
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
    // A recording without the share, or a share refused at capture,
    // refuses the arm by name.
    row.reinforcement_speed = Recorded::NotRecorded;
    let result = chance(&row).unwrap();
    assert_eq!(
        reinforcement_text(&NoIcons, &loc, &result),
        Err(RESERVE_SPEED_UNRECORDED.into())
    );
    row.reinforcement_speed = Recorded::Value(None);
    let result = chance(&row).unwrap();
    assert_eq!(
        reinforcement_text(&NoIcons, &loc, &result),
        Err(RESERVE_SPEED_REFUSED.into())
    );
    // The rejection arms need no share.
    let tired = Condition {
        organisation: ADVANCE_ORGANISATION - 1,
        ..condition
    };
    row.reinforcement_speed = Recorded::NotRecorded;
    assert_eq!(
        reinforcement(&row, &tired, Ok((100_000, 0)), Ok(&defines)),
        Ok(Reinforcement::Rejected(COMREIC_NOT_FIGHT))
    );
}

use crate::combat::binding::NoIcons;

/// Each refused hover part names the key it could not localise, so the
/// bind can count repeats by key.
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
        // A missing top-level key shows its own bytes.
        Ok("Soft attack: 0.00
BASE_VALUE
Total 0.00
"
        .into())
    );
    // A missing top-level key shows its own bytes.
    assert_eq!(
        reinforcement_text(
            &NoIcons,
            &loc,
            &Reinforcement::Rejected("COMREIC_NOT_FIGHT")
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
            reinforcement_text(&NoIcons, &flagged, &Reinforcement::Rejected("X")),
            Err(
                "localisation BATTLE_REINFORCE_CHANCE2 holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw"
                    .into()
            )
        );
}
use replay::rows::{DivisionMovementRow, DivisionRow, ProvinceRow};

/// Invasion section with fixture text: the
/// description, the progress (complete from 100000 up, the zero-extended
/// sentinel included), and the two scales only when either differs.
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
    // A missing top-level key shows its own bytes.
    assert_eq!(
        invasion_section(&NoIcons, &Localisation::default(), 0, neutral),
        Ok("NAVAL_INVASION_COMBAT_PROGRESS_DESC\n\nNAVAL_INVASION_PROGRESS\n\n".into())
    );
}

/// The default hover names the exact key it lacks or cannot render, even
/// deep inside the invasion section.
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
        scales: [0, 0, 50_000, 100_000].map(replay::rows::Recorded::Value),
        ..Default::default()
    };
    // A missing top-level key shows its own bytes.
    assert!(
        division_summary(&NoIcons, &entries(&[]), "1st", &row, Some(10_000), None)
            .unwrap()
            .contains("\nNAVAL_INVASION_ATTACK_SCALE\nNAVAL_INVASION_DEFEND_SCALE\n")
    );
    let flagged = entries(&[
        ("NAVAL_INVASION_ATTACK_SCALE", "@GER"),
        ("NAVAL_INVASION_DEFEND_SCALE", "$VALUE$"),
    ]);
    assert_eq!(
            division_summary(&NoIcons, &flagged, "1st", &row, Some(10_000), None),
            Err(
                "localisation NAVAL_INVASION_ATTACK_SCALE holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw"
                    .into()
            )
        );
    let complete = entries(&[
        ("NAVAL_INVASION_ATTACK_SCALE", "$VALUE$"),
        ("NAVAL_INVASION_DEFEND_SCALE", "$VALUE$"),
    ]);
    assert!(division_summary(&NoIcons, &complete, "1st", &row, Some(10_000), None).is_ok());
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
        replay::rows::CountryLinkRow {
            id: 5,
            canonical_slot: 5,
        },
        replay::rows::CountryLinkRow {
            id: 6,
            canonical_slot: 6,
        },
    ];
    let war = [replay::rows::RelationTupleRow {
        type_id: engine::cwar_relation::TYPE,
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
    // A retired war is peace; an unlinked live war leaves peace unknown,
    // but cannot hide a war that is known.
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
    // An incomplete table is named, not folded into a list of suspects.
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
    // The whole part is the native signed i32.
    assert_eq!(one_decimal(214_748_364_800_000), "-2147483648.0");
}

/// trimmed fixed-point formatting: the whole part through the signed
/// i32 formatter, the magnitude by wrapping negation, so neither a huge
/// value nor i64::MIN is refused.
#[test]
fn a_stat_headline_formats_as_the_native_trimmed_formatter() {
    assert_eq!(fixed_decimal(150_000), "1.5");
    assert_eq!(fixed_decimal(100_000), "1");
    assert_eq!(fixed_decimal(-1_234_567), "-12.34567");
    assert_eq!(fixed_decimal(214_748_364_800_000), "-2147483648");
    // -(i64::MIN) wraps to itself: `-`, then (MIN / 100000) as i32, then
    // (MIN % 100000) + 100000 = 24192 with its first digit made `.`.
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
        reinforcement(&row, &condition, Err("unused"), Err("unused")),
        Ok(Reinforcement::Rejected("COMREIC_NOT_FIGHT"))
    );
    condition.strength = ADVANCE_STRENGTH;
    condition.organisation = ADVANCE_ORGANISATION;
    row.context.template = 1;
    row.width_terms.push(replay::rows::LandDivisionWidthTerm {
        subunit: 1,
        count: 1,
        width: 200_000,
    });
    let mut defines = LandDefines::default();
    defines.reinforce.over_width_penalty = -200_000;
    defines.reinforce.over_width_penalty_max = -100_000;
    assert_eq!(
        reinforcement(&row, &condition, Ok((100_000, 0)), Ok(&defines)),
        Ok(Reinforcement::Rejected("COMREIC_TOO_MUCH_WIDTH"))
    );
    // Without the defines or the widths the hover names the operand.
    assert_eq!(
        reinforcement(&row, &condition, Ok((100_000, 0)), Err("no NMilitary")),
        Err("the land combat defines are unavailable (no NMilitary)".into())
    );
    assert_eq!(
        reinforcement(&row, &condition, Err("width unknown"), Ok(&defines)),
        Err("width unknown".into())
    );
}

/// The commander icon hover joins the division row's handle to the
/// hour's UnitLeaders row (name token +0x20, skill level) and its
/// UnitLeaderTraits rows in ordinal order; a missing row refuses by name.
#[test]
fn the_commander_icon_reads_the_hours_unit_leader_and_its_traits() {
    use replay::rows::{UnitLeaderRow, UnitLeaderTraitRow};
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
    // No traits: no list header.
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
    // Each missing operand refuses by name.
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
