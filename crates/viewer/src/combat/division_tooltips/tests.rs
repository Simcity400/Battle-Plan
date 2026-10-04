use super::*;
#[test]
fn reserve_hover_prints_cached_sources_in_native_order_and_refuses_other_languages() {
    use replay::rows::{ModifierBreakdownNode as Node, ModifierDefinitionRow};
    let mut loc: Localisation = [
        ("BATTLE_REINFORCE_CHANCE2", "Chance $VALUE$"),
        ("MODIFIER_BASE_VALUE_PERCENTAGE", "Base"),
        ("STAT_ARMY_INITIATIVE", "Initiative"),
        ("STAT_ARMY_INITIATIVE_DIFF", "$VALUE$"),
        ("SPEED_REINFORCEMENT_BONUS", "Speed $VAL$"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    loc.language = "english".into();
    let node = |children, value, name: &[u8]| Node {
        children,
        value,
        name: name.to_vec(),
    };
    let nodes = [
        node(3, 99_000, b"aggregate"),
        node(0, 5_000, b"First"),
        node(0, 0, b"Zero"),
        node(1, 25_000, b"Nested aggregate"),
        node(0, -1_000, b"Last"),
    ];
    let definition = ModifierDefinitionRow {
        flags: 3,
        precision: 2,
        ..Default::default()
    };
    let sources = reserve_tree(&nodes, &definition, "english", &NoIcons, &loc).unwrap();
    let text = reinforcement_text(
        &NoIcons,
        &loc,
        &Reinforcement::Chance {
            value: 10_000,
            base: 1,
            initiative: 2_000,
            speed: 3_000,
        },
        Ok(&(
            sources,
            Some(("Commander".into(), "Leader source\n".into())),
        )),
    )
    .unwrap();
    assert_eq!(
        text,
        "Chance 0.10Base\nFirst: \u{11}G+5.00%\u{11}!\nLast: \u{11}R-1.00%\u{11}!\nInitiative0.02\nSpeed 0.03\nCommander: Leader source\n"
    );
    loc.language = "french".into();
    let reason = reserve_tree(&nodes, &definition, "english", &NoIcons, &loc).unwrap_err();
    assert!(reason.contains("reserve reinforcement modifier source") && reason.contains("french"));
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
    let modifiers = DivisionCombatModifiers::default();
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
        // A missing top-level key shows its own bytes (142230940).
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
            &Reinforcement::Chance {
                value: 1,
                base: 1,
                initiative: 0,
                speed: 0
            },
            Ok(&(String::new(), None)),
        ),
        Ok("Chance 0.00MODIFIER_BASE_VALUE_PERCENTAGE
"
        .into())
    );
    let flagged: Localisation = [("BATTLE_REINFORCE_CHANCE2", "@GER")]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    assert_eq!(
            reinforcement_text(&NoIcons, &flagged, &Reinforcement::Rejected("X"), Ok(&(String::new(), None))),
            Err(
                "localisation BATTLE_REINFORCE_CHANCE2 holds the inline flag 0x12 \"GER\" (an authored `@`), which the viewer does not draw"
                    .into()
            )
        );
}
use replay::rows::{DivisionMovementRow, DivisionRow, ProvinceRow};

/// 1415BF4B0's invasion section with the shipped English strings: the
/// description, the progress (complete from 100000 up, the zero-extended
/// sentinel included), and the two scales only when either differs.
#[test]
fn invasion_section_follows_the_native_order_and_branches() {
    let loc: Localisation = [
        ("NAVAL_INVASION_COMBAT_PROGRESS_DESC", "Invading."),
        ("NAVAL_INVASION_PROGRESS", "Progress: $VALUE|Y0%$"),
        (
            "NAVAL_INVASION_PROGRESS_COMPLETE",
            "Progress: $VALUE|Y0%$
(All ashore).
",
        ),
        ("NAVAL_INVASION_ATTACK_SCALE", "Attack: $VALUE|=+0%$"),
        ("NAVAL_INVASION_DEFEND_SCALE", "Defense: $VALUE|=+0%$"),
    ]
    .iter()
    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
    .collect();
    let neutral = [0, 0, 100_000, 100_000];
    let partial = invasion_section(&NoIcons, &loc, 50_000, neutral).unwrap();
    assert!(partial.starts_with(
        "Invading.

Progress: "
    ));
    assert!(partial.ends_with(
        "

"
    ));
    assert!(!partial.contains("Attack"));
    let complete = invasion_section(&NoIcons, &loc, 4_294_967_295, neutral).unwrap();
    assert!(complete.contains("(All ashore)"));
    let scaled = invasion_section(&NoIcons, &loc, 100_000, [0, 0, 50_000, 125_000]).unwrap();
    let attack = scaled
        .find(
            "
Attack: ",
        )
        .unwrap();
    let defense = scaled
        .find(
            "
Defense: ",
        )
        .unwrap();
    assert!(attack < defense);
    // A missing top-level key shows its own bytes (142230940).
    assert_eq!(
        invasion_section(&NoIcons, &Localisation::default(), 0, neutral),
        Ok("NAVAL_INVASION_COMBAT_PROGRESS_DESC\n\nNAVAL_INVASION_PROGRESS\n\n".into())
    );
}

/// The default hover names the exact key it lacks or cannot render, even
/// deep inside the invasion section (round-6 review: a missing
/// NAVAL_INVASION_ATTACK_SCALE lost its identity).
#[test]
fn the_division_summary_names_its_missing_or_unrenderable_key() {
    let entries = |extra: &[(&str, &str)]| -> Localisation {
        [
            ("COMBAT_DIVISION_ATTACKER", "$NAME$"),
            ("NAVAL_INVASION_COMBAT_PROGRESS_DESC", "Invading."),
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
        ..Default::default()
    };
    // A missing top-level key shows its own bytes (142230940).
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
    // The whole part is the native signed i32 (1424B7A80).
    assert_eq!(one_decimal(214_748_364_800_000), "-2147483648.0");
}

/// FormatFixed5Trimmed (1424B60D0): the whole part through the signed
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
