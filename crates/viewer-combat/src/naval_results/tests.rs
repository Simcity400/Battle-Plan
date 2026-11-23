use super::*;
use replay_data::{rows::CountryLinkRow, rows::NavalResultCountryRow};
use viewer_foundation::tooltip_text::NoIcons;

fn links() -> Vec<CountryLinkRow> {
    [(10, 8), (20, 2), (30, 4), (40, 8)]
        .into_iter()
        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .collect()
}

#[test]
fn aircraft_classify_by_archetype_and_admit_survivors_and_losses_independently() {
    let side = NavalResultSideDetails {
        planes: vec![
            NavalResultPlane {
                variant: 1,
                current_planes: 7,
                lost_planes: 2,
                ..Default::default()
            },
            NavalResultPlane {
                variant: 2,
                current_planes: 0,
                lost_planes: 3,
                ..Default::default()
            },
            NavalResultPlane {
                variant: 3,
                current_planes: 4,
                lost_planes: 0,
                ..Default::default()
            },
            NavalResultPlane {
                variant: 4,
                current_planes: -1,
                lost_planes: 0,
                ..Default::default()
            },
            NavalResultPlane {
                variant: 5,
                current_planes: 1,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let (groups, refused) = classify(&side, &|plane| match plane.variant {
        1 | 2 => Ok(9),
        3 => Ok(11),
        4 => panic!("nonpositive counts must not demand content"),
        _ => Err("modded archetype is unresolved".into()),
    });
    assert_eq!(
        groups
            .planes
            .iter()
            .map(|group| group.model)
            .collect::<Vec<_>>(),
        [9, 11]
    );
    assert_eq!(groups.planes[0].members[0].current_planes, 7);
    assert_eq!(groups.lost_planes.len(), 1);
    assert_eq!(
        groups.lost_planes[0]
            .members
            .iter()
            .map(|plane| plane.variant)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(refused, [(5, "modded archetype is unresolved".into())]);
}

#[test]
fn aircraft_icons_use_native_small_and_loss_prefixes_with_archetype_then_sprite() {
    let both = |_: &str| true;
    assert_eq!(
        air_icon(b"mod_frame", b"mod_fallback", true, &both).unwrap(),
        "GFX_navalcombat_air_small_icon_mod_frame"
    );
    let fallback = |sprite: &str| sprite == "GFX_navalcombat_air_icon_mod_fallback";
    assert_eq!(
        air_icon(b"mod_frame", b"mod_fallback", false, &fallback).unwrap(),
        "GFX_navalcombat_air_icon_mod_fallback"
    );
    assert!(
        air_icon(b"mod_frame", b"mod_fallback", true, &|_| false)
            .unwrap_err()
            .contains("mod_frame")
    );
    assert!(
        air_icon(&[0xff], b"mod_fallback", false, &both)
            .unwrap_err()
            .contains("UTF-8")
    );
}

#[test]
fn sunk_by_classifies_ships_and_convoys_stay_separate_from_matching_sprites() {
    let ship = |member, sunk, convoy| NavalResultShip {
        member,
        subunit_index: 7,
        formation_type: 2,
        sprite: b"mod_ship".to_vec(),
        sunk_by: if sunk { b"killer".to_vec() } else { Vec::new() },
        is_convoy: convoy,
        ..Default::default()
    };
    let side = NavalResultSideDetails {
        ships: vec![
            ship(1, false, 0),
            ship(2, true, 1),
            ship(3, true, 0),
            ship(4, false, 0),
        ],
        ..Default::default()
    };
    let survived = ship_groups(&side, false);
    assert_eq!(survived.len(), 1);
    assert_eq!(
        survived[0]
            .iter()
            .map(|ship| ship.member)
            .collect::<Vec<_>>(),
        [1, 4]
    );
    let lost = ship_groups(&side, true);
    assert_eq!(
        lost.iter().map(|group| group[0].member).collect::<Vec<_>>(),
        [3, 2]
    );
}

#[test]
fn survivor_representatives_use_country_aliases_ic_strength_and_convoys_not_pride() {
    let links = links();
    let order = Countries::new(Available::Complete(&links), 8);
    let ship = |member, owner, cost, strength, convoy, pride| NavalResultShip {
        member,
        owner,
        build_cost_ic: cost,
        strength_ratio: strength,
        is_convoy: convoy,
        pride,
        sprite: format!("sprite_{member}").into_bytes(),
        ..Default::default()
    };
    let entries = [
        ship(1, 30, 100_000, 100_000, 0, 0),
        ship(2, 10, 400_000, 50_000, 0, 1),
        ship(3, 40, 400_000, 100_000, 0, 1),
    ];
    let mut group: Vec<_> = entries.iter().collect();
    native_sort(&mut group, &|a, b| ship_before(a, b, &order));
    assert_eq!(group[0].sprite, b"sprite_3");
    assert_eq!(
        group.iter().map(|ship| ship.member).collect::<Vec<_>>(),
        [3, 2, 1]
    );
    let a = ship(4, 10, 400_000, 50_000, 1, 0);
    let b = ship(5, 40, 400_000, 100_000, 1, 0);
    assert!(
        !ship_before(&b, &a, &order),
        "equal-IC convoys keep order despite strength"
    );
    let canonical_lower = ship(6, 20, 100_000, 100_000, 0, 0);
    assert!(ship_before(&a, &canonical_lower, &order));
    assert!(ship_before(&canonical_lower, &a, &order));
    let mut shortcut = vec![&canonical_lower, &a, &entries[0]];
    native_sort(&mut shortcut, &|a, b| ship_before(a, b, &order));
    assert_eq!(
        shortcut.iter().map(|ship| ship.member).collect::<Vec<_>>(),
        [1, 4, 6],
        "native first-element insertion moves canonical slot 4 before viewer slot 8, even though slot 2 precedes slot 4"
    );
}

#[test]
fn country_groups_coalesce_aliases_and_use_viewer_count_and_canonical_order() {
    let links = links();
    let order = Countries::new(Available::Complete(&links), 8);
    let entries = [(30, 2), (20, 2), (10, 1), (40, 2)];
    assert_eq!(order.counts(entries).unwrap(), [(10, 3), (20, 2), (30, 2)]);
    assert_eq!(
        order.counts([(20, 1), (30, 5), (10, 2)]).unwrap(),
        [(10, 2), (30, 5), (20, 1)]
    );
    assert!(
        order
            .counts([(999, 1)])
            .unwrap_err()
            .contains("country links")
    );
}

#[test]
fn aircraft_survivor_counts_order_both_flags_and_origin_hover_lines() {
    let links = links();
    let order = Countries::new(Available::Complete(&links), 8);
    let side = NavalResultSideDetails {
        planes: [(20, 1), (30, 2), (10, 1), (30, 3)]
            .map(|(country, current_planes)| NavalResultPlane {
                country,
                current_planes,
                variant: 1,
                ..Default::default()
            })
            .to_vec(),
        ..Default::default()
    };
    let (groups, refused) = classify(&side, &|_| Ok(9));
    assert!(refused.is_empty());
    let owners = order
        .counts(
            groups.planes[0]
                .members
                .iter()
                .map(|plane| (plane.country, plane.current_planes)),
        )
        .unwrap();
    assert_eq!(owners, [(10, 1), (30, 5), (20, 1)]);
    let flag_map: Flags = [(10, "viewer"), (20, "one"), (30, "five")]
        .into_iter()
        .map(|(country, flag)| (country, Ok(flag.into())))
        .collect();
    let mut out = Bindings::default();
    let mut values = Values::new();
    flags(&mut out, &mut values, &owners, &flag_map);
    assert_eq!(values["country_flag_left"].flag.as_deref(), Some("viewer"));
    assert_eq!(values["country_flag_right"].flag.as_deref(), Some("five"));
    assert!(values["country_flag_center"].hidden);
    assert!(out.refused().is_empty());
    let loc: Localisation = [(AIR_COUNT_ORIGIN, "$COUNTRY$:$COUNT$")]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    assert_eq!(
        air_origins(&NoIcons, &loc, &owners, |country| country_flag(
            &flag_map, country
        ))
        .unwrap(),
        "\nviewer:1\nfive:5\none:1"
    );
}

#[test]
fn result_country_order_has_no_country_capacity_limit() {
    let links: Vec<_> = (1..=5000)
        .map(|slot| CountryLinkRow {
            id: 10_000 - slot,
            canonical_slot: slot,
        })
        .collect();
    let order = Countries::new(Available::Complete(&links), 4000);
    let counts = order
        .counts(links.iter().rev().map(|row| (row.id, 1)))
        .unwrap();
    assert_eq!(counts.len(), 5000);
    assert_eq!(order.slot(counts[0].0).unwrap(), 4000);
    assert_eq!(order.slot(counts[1].0).unwrap(), 1);
    assert_eq!(order.slot(counts[4999].0).unwrap(), 5000);
}

#[test]
fn joint_priorities_truncate_before_accumulation_and_promote_the_best_aircraft() {
    let ships = [
        NavalResultShip {
            build_cost_ic: 199_999,
            ..Default::default()
        },
        NavalResultShip {
            build_cost_ic: 199_999,
            ..Default::default()
        },
    ];
    assert_eq!(priority_ships(&ships.iter().collect::<Vec<_>>()), -2);
    let plane = NavalResultPlane {
        variant_summary_inputs: [150_000, 100_000],
        current_planes: 9,
        ..Default::default()
    };
    assert_eq!(priority_planes(&[&plane]), -1);
    let mut entries = vec![
        (-9, false, "ship-a"),
        (-8, false, "ship-b"),
        (-7, false, "ship-c"),
        (-1, true, "plane-a"),
        (0, true, "plane-b"),
    ];
    order_survivors(&mut entries, Some(2));
    assert_eq!(
        entries.iter().map(|entry| entry.2).collect::<Vec<_>>(),
        ["ship-a", "plane-a", "ship-c", "ship-b", "plane-b"]
    );
    let mut ties = vec![(-1, false, "ship"), (-1, true, "plane")];
    order_survivors(&mut ties, None);
    assert_eq!(ties[0].2, "ship");
}

#[test]
fn outcome_uses_the_explicit_winner_and_refuses_an_unproven_defeat() {
    let links = links();
    let order = Countries::new(Available::Complete(&links), 8);
    let countries = [
        NavalResultCountryRow {
            result: 7,
            list: 0,
            country: 20,
            ..Default::default()
        },
        NavalResultCountryRow {
            result: 7,
            list: 1,
            country: 40,
            ..Default::default()
        },
    ];
    let mut result = NavalResultRow {
        id: 7,
        winning_side: 2,
        ..Default::default()
    };
    assert_eq!(
        outcome(&result, &Available::Complete(&countries), &order).unwrap(),
        Some(NAVAL_COMBAT_RESULT_VICTORY)
    );
    result.winning_side = 1;
    assert_eq!(
        outcome(&result, &Available::Complete(&countries), &order).unwrap(),
        Some(NAVAL_COMBAT_RESULT_DEFEAT)
    );
    assert!(
        outcome(
            &result,
            &Available::Truncated {
                rows: &countries,
                missing: 1
            },
            &order
        )
        .is_err()
    );
    result.winning_side = 2;
    assert_eq!(
        outcome(
            &result,
            &Available::Truncated {
                rows: &countries,
                missing: 1
            },
            &order
        )
        .unwrap(),
        Some(NAVAL_COMBAT_RESULT_VICTORY)
    );
    result.winning_side = 0;
    assert!(outcome(&result, &Available::Complete(&countries), &order).is_err());
    result.readiness = 1;
    assert_eq!(
        outcome(
            &result,
            &Available::Unavailable {
                reason: "historical"
            },
            &order
        )
        .unwrap(),
        None
    );
    result.readiness = 0;
    result.winning_side = 2;
    let unresolved = Countries::new(
        Available::Unavailable {
            reason: "historical",
        },
        8,
    );
    assert!(
        outcome(&result, &Available::Complete(&countries), &unresolved)
            .unwrap_err()
            .contains("unavailable")
    );
}

fn loc() -> Localisation {
    [
        (NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN, "|offense|"),
        (NAVAL_COMBAT_DEFENSE_AIRWING_TOOLTIP_SHOT_DOWN, "|defense|"),
        (NAVAL_COMBAT_AIRWING_TOOLTIP_SHOT_DOWN_ENTRY, "$NUM$:$MOD$;"),
        (NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED, "|naval|"),
        (
            NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_ENTRY,
            "$NAVY$:$PERC|%0$;",
        ),
        (NAVAL_COMBAT_AIRWING_TOOLTIP_DAMAGED_LAST_HIT, "last-hit;"),
        (SHIP_RESULT_SUNK, "sunk:$SHIP$"),
        (HEADER, "[$KEY$]\n"),
        (AND_X_MORE_SUNK_SHIPS, "\nmore:$VAL$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

#[test]
fn all_three_hit_lanes_keep_order_and_last_hit_and_stored_language_guards() {
    let mut loc = loc();
    loc.language = "english".into();
    let air = [
        vec![NavalCombatAirHit {
            variant: 1,
            count: 2,
            ..Default::default()
        }],
        vec![NavalCombatAirHit {
            variant: 2,
            count: 3,
            ..Default::default()
        }],
    ];
    let naval = [NavalCombatHit {
        target_name: b"Target".to_vec(),
        damage: 25_000,
        denominator: 100_000,
        last_hit: 1,
        ..Default::default()
    }];
    let build = |loc: &Localisation| {
        hits(
            &NoIcons,
            loc,
            &air,
            &naval,
            &|id| Ok(format!("variant-{id}")),
            &|bytes| {
                dashboard_text::scripted_text::stored_text(
                    loc,
                    "english",
                    bytes,
                    "retained naval hit target",
                )
            },
        )
    };
    assert_eq!(
        build(&loc).unwrap(),
        "|offense|2:variant-1;|defense|3:variant-2;|naval|Target:25%;last-hit;"
    );
    assert_eq!(hit_score(&air, &naval), 5);
    loc.language = "french".into();
    assert!(
        build(&loc)
            .unwrap_err()
            .contains("text the game stored in english")
    );
    let sum_first = [
        NavalCombatHit {
            damage: 60_000,
            ..Default::default()
        },
        NavalCombatHit {
            damage: 60_000,
            ..Default::default()
        },
    ];
    assert_eq!(hit_score(&[Vec::new(), Vec::new()], &sum_first), 1);
}

#[test]
fn detail_text_orders_viewer_canonical_kind_score_and_keeps_equal_admission_order() {
    let links = links();
    let order = Countries::new(Available::Complete(&links), 8);
    let entries = [
        (30, 0, 9, "other"),
        (10, 1, 99, "ship"),
        (40, 0, 5, "plane-a"),
        (10, 0, 5, "plane-b"),
        (10, 0, 8, "plane-high"),
        (20, 1, 0, "canonical-low"),
    ]
    .into_iter()
    .map(|(owner, kind, score, text)| (owner, kind, score, text.to_owned()))
    .collect();
    assert_eq!(
        join_details(&order, entries).unwrap(),
        "plane-high\n\nplane-a\n\nplane-b\n\nship\n\ncanonical-low\n\nother"
    );
    assert!(join_details(&order, vec![(999, 0, 1, "unknown".into())]).is_err());
}

#[test]
fn ship_loss_hover_limits_member_lines_to_twenty_and_reports_the_remainder() {
    let ships: Vec<_> = (1..=22)
        .map(|member| NavalResultShip {
            member,
            ..Default::default()
        })
        .collect();
    let text = lost_ship_text(
        &NoIcons,
        &loc(),
        "modded-type",
        &ships.iter().collect::<Vec<_>>(),
        &|ship| Ok(format!("ship-{}", ship.member)),
    )
    .unwrap();
    assert!(text.starts_with("[sunk:modded-type]\nship-1\nship-2\n"));
    assert!(text.ends_with("ship-20\nmore:2"));
    assert!(!text.contains("ship-21"));
}

#[test]
fn survivor_hover_uses_the_native_convoy_full_strength_and_damage_branches() {
    let loc: Localisation = [
        (
            NAVAL_COMBAT_RESULT_SURVIVOR_CONVOYS,
            "$FLAG$ convoy:$COUNT$",
        ),
        (
            NAVAL_COMBAT_RESULT_SURVIVOR_COUNT_UNDAMAGED,
            "$FLAG$ full:$COUNT$:$TYPE$",
        ),
        (NAVAL_COMBAT_RESULT_SURVIVOR, "$FLAG$ ship:$TYPE$"),
        (
            NAVAL_COMBAT_RESULT_SURVIVOR_DMG,
            "$FLAG$ damaged:$TYPE$:$DMG|%0$",
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let mut ship = NavalResultShip {
        strength_ratio: 100_000,
        ..Default::default()
    };
    assert_eq!(
        survivor_description(&NoIcons, &loc, &ship, 1, "flag", "name").unwrap(),
        "flag ship:name"
    );
    assert_eq!(
        survivor_description(&NoIcons, &loc, &ship, 2, "flag", "type").unwrap(),
        "flag full:2:type"
    );
    ship.strength_ratio = 99_000;
    assert_eq!(
        survivor_description(&NoIcons, &loc, &ship, 1, "flag", "name").unwrap(),
        "flag damaged:name:99%"
    );
    ship.is_convoy = 1;
    assert_eq!(
        survivor_description(&NoIcons, &loc, &ship, 2, "flag", "").unwrap(),
        "flag convoy:2"
    );
}

#[test]
fn mixed_grids_keep_each_template_and_a_refused_flag_keeps_counts() {
    let mut out = Bindings::default();
    let mut ship = Values::new();
    text(&mut ship, "number", "3");
    let flags_map = Flags::from([
        (10, Err("original flag tag is not recorded".into())),
        (20, Ok("MOD-FLAG".into())),
    ]);
    flags(&mut out, &mut ship, &[(10, 3)], &flags_map);
    assert_eq!(ship["number"].text.as_deref(), Some("3"));
    assert!(ship["country_flag_center"].hidden);
    assert_eq!(
        out.refused().get("original flag tag is not recorded"),
        Some(&1)
    );
    flags(&mut out, &mut ship, &[(10, 1), (20, 2)], &flags_map);
    assert_eq!(ship["number"].text.as_deref(), Some("3"));
    assert!(ship["country_flag_left"].hidden && ship["country_flag_center"].hidden);
    assert_eq!(ship["country_flag_right"].flag.as_deref(), Some("MOD-FLAG"));
    assert!(!ship["country_flag_right"].hidden);
    assert_eq!(
        out.refused().get("original flag tag is not recorded"),
        Some(&2)
    );
    repeat_mixed(
        &mut out.values,
        "survivors",
        "ship-entry".into(),
        vec![
            ("ship-entry".into(), ship),
            ("air-entry".into(), Values::new()),
        ],
    );
    assert_eq!(out.values["survivors"].entry_templates[&1], "air-entry");
    assert_eq!(out.values["survivors"].entries.len(), 2);
    show_details(&mut out.values, true);
    assert!(!out.values["attacker_details"].hidden && !out.values["defender_details"].hidden);
    assert_eq!(out.values["open_details"].frame, Some(1));
    show_details(&mut out.values, false);
    assert!(out.values["attacker_details"].hidden && out.values["defender_details"].hidden);
    assert_eq!(out.values["open_details"].frame, Some(0));
    out.values.get_mut("open_details").unwrap().enabled = Some(false);
    show_details(&mut out.values, true);
    assert!(out.values["attacker_details"].hidden);
}

#[test]
fn pride_loss_tint_reads_stacked_rgba_or_hsv_and_preserves_the_sprite_on_refusal() {
    let defines = content_reader_defines::defines::Defines::new().unwrap();
    defines.load("synthetic.lua", b"NDefines = { NGraphics = { PRIDE_OF_THE_FLEET_MODULATE = { 0.5, 0.25, 1, 0.75, 99 } } }").unwrap();
    assert_eq!(
        pride_color(&defines).unwrap(),
        [0.5_f32.powf(2.2), 0.25_f32.powf(2.2), 1.0, 0.75]
    );
    defines
        .load(
            "override.lua",
            b"NDefines.NGraphics.PRIDE_OF_THE_FLEET_MODULATE = { 0.5, 1, 1 }",
        )
        .unwrap();
    assert_eq!(pride_color(&defines).unwrap(), [0.0, 1.0, 1.0, 1.0]);
    defines
        .load(
            "unsupported.lua",
            b"NDefines.NGraphics.PRIDE_OF_THE_FLEET_MODULATE = { 0.5, 1 }",
        )
        .unwrap();
    let refused = pride_color(&defines);
    assert!(
        refused
            .as_ref()
            .unwrap_err()
            .contains("three HSV or four RGBA")
    );

    let mut out = Bindings::default();
    let mut values = Values::from([(
        "equipment".into(),
        Widget {
            sprite: Some("mod_ship".into()),
            ..Default::default()
        },
    )]);
    loss_color(&mut out, &mut values, true, &refused);
    assert_eq!(values["equipment"].sprite.as_deref(), Some("mod_ship"));
    assert_eq!(values["equipment"].color, None);
    assert_eq!(out.refused().values().sum::<usize>(), 1);
    loss_color(&mut out, &mut values, false, &refused);
    assert_eq!(values["equipment"].color, Some([1.0; 4]));
    loss_color(&mut out, &mut values, true, &Ok([0.1, 0.2, 0.3, 0.4]));
    assert_eq!(values["equipment"].color, Some([0.1, 0.2, 0.3, 0.4]));
}
