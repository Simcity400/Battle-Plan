use super::*;

#[test]
fn a_refused_flag_preserves_known_naval_counts_and_other_country_flags() {
    let reason = "country 7 original tag is not recorded".to_owned();
    let flags = Flags::from([(7, Err(reason.clone())), (8, Ok("BBB".to_owned()))]);
    let mut out = Bindings::default();
    for side in 0..2 {
        let mut values = Values::new();
        descriptions(&mut out, &mut values, 7, 3, side, 56.0, &flags);
        let (count, flag) = if side == 0 {
            ("desc_right", "desc_left")
        } else {
            ("desc_left", "desc_right")
        };
        assert_eq!(values[count].text.as_deref(), Some("3"));
        assert!(!values[count].hidden);
        assert!(values[flag].hidden);
        assert!(values[flag].text.is_none());
    }
    assert_eq!(out.refused().get(&reason), Some(&2));
}

#[test]
fn forward_land_and_carrier_air_share_the_native_group_rule() {
    let make = |source| NavalCombatPlane {
        source: 0,
        group_index: 2,
        variant: 31,
        efficiency: replay::rows::NavalCombatAirEfficiency {
            source,
            ..Default::default()
        },
        ..Default::default()
    };
    let land = make(2);
    let carrier = make(3);
    assert!(display_air_plane(&land));
    assert!(display_air_plane(&carrier));
    let groups = [AirGroup {
        side: 0,
        model: 7,
        owner: 4,
        planes: vec![&land],
    }];
    assert_eq!(air_group_slot(&groups, 0, 7, 4), Some(0));
    assert_eq!(air_group_slot(&groups, 0, 8, 4), None);
    assert_eq!(air_group_slot(&groups, 1, 7, 4), None);
    assert_eq!(air_group_slot(&groups, 0, 7, 5), None);
    let mut retired = carrier.clone();
    retired.carrier_state = 4;
    assert!(!display_air_plane(&retired));
    let mut paired = carrier.clone();
    paired.group_index = u32::MAX;
    assert!(!display_air_plane(&paired));
}

#[test]
fn air_count_uses_current_over_max_and_native_side_colour_order() {
    assert_eq!(
        air_count_label(0, "AAA", 120, 120, false),
        format!("\u{11}W {}AAA 120/120\u{11}!", super::super::INLINE_FLAG)
    );
    assert_eq!(
        air_count_label(0, "AAA", 3, 7, false),
        format!("\u{11}W {}AAA 3/7\u{11}!", super::super::INLINE_FLAG)
    );
    assert_eq!(
        air_count_label(1, "BBB", 2, 9, true),
        format!("\u{11}R 2/9 {}BBB\u{11}!", super::super::INLINE_FLAG)
    );
}

#[test]
fn air_icon_frames_and_dynamic_hover_are_explicit() {
    let left = air_icon_widget(Some("GFX_test_air".into()), 0);
    let right = air_icon_widget(Some("GFX_test_air".into()), 1);
    assert_eq!(left.frame, Some(2));
    assert_eq!(right.frame, Some(1));
    assert!(left.hover_owned && right.hover_owned);
    assert!(left.tooltip.is_none() && right.tooltip.is_none());
    let missing = air_icon_widget(None, 0);
    assert!(missing.hidden && missing.sprite.is_none());
}

#[test]
fn air_entries_bind_before_carrier_ships_on_both_grids() {
    let mut out = Bindings::default();
    let mut heights = BTreeMap::new();
    push_air_entry(
        &mut out,
        &mut heights,
        0,
        Some("GFX_air_left".into()),
        [40.0, 20.0],
        Some("3/5".into()),
        Some("Air left".into()),
    );
    push_air_entry(
        &mut out,
        &mut heights,
        1,
        Some("GFX_air_right".into()),
        [42.0, 24.0],
        Some("2/8".into()),
        Some("Air right".into()),
    );
    for side in 0..2 {
        out.naval_entries.push(NavalEntry {
            side,
            unknown: false,
            grid: format!(
                "battle_window/top_wnd/{}_wnd/grid_carrier",
                if side == 0 { "left" } else { "right" }
            ),
            offset: Some([0.0, heights[&(side, "carrier")]]),
            size: [40.0, 20.0],
            values: Values::new(),
            battle_slots: vec![1],
        });
    }
    for side in 0..2 {
        let air = out
            .naval_entries
            .iter()
            .position(|entry| entry.side == side && entry.battle_slots.is_empty())
            .unwrap();
        let ship = out
            .naval_entries
            .iter()
            .position(|entry| entry.side == side && !entry.battle_slots.is_empty())
            .unwrap();
        assert!(air < ship, "side {side} ship precedes its air group");
    }
    assert_eq!(
        out.naval_entries[0].grid,
        "battle_window/top_wnd/left_wnd/grid_carrier"
    );
    assert_eq!(
        out.naval_entries[1].grid,
        "battle_window/top_wnd/right_wnd/grid_carrier"
    );
    assert_eq!(out.naval_entries[0].values["icon"].frame, Some(2));
    assert_eq!(out.naval_entries[1].values["icon"].frame, Some(1));
    assert_eq!(
        out.naval_entries[0].values["icon"].tooltip.as_deref(),
        Some("Air left")
    );
    assert_eq!(
        out.naval_entries[1].values["icon"].tooltip.as_deref(),
        Some("Air right")
    );
    assert_eq!(
        out.naval_entries[0].values["desc_left"].text.as_deref(),
        Some("3/5")
    );
    assert!(!out.naval_entries[0].values["desc_left"].hidden);
    assert_eq!(
        out.naval_entries[1].values["desc_right"].text.as_deref(),
        Some("2/8")
    );
    assert!(!out.naval_entries[1].values["desc_right"].hidden);
    assert_eq!(heights[&(0, "carrier")], 23.0);
    assert_eq!(heights[&(1, "carrier")], 27.0);
    push_air_entry(
        &mut out,
        &mut heights,
        0,
        None,
        [1.0, 3.0],
        Some("4/7".into()),
        None,
    );
    let missing = out.naval_entries.last().unwrap();
    assert!(missing.values["icon"].hidden);
    assert!(missing.values["icon"].sprite.is_none());
    assert_eq!(missing.values["desc_left"].text.as_deref(), Some("4/7"));
}

#[test]
fn recorded_air_hover_reaches_the_air_icon_binding() {
    use crate::combat::binding::NoIcons;
    let mut loc: installed_content::localisation::Localisation = [(
        "NAVAL_COMBAT_AIRWING_TOOLTIP_HEADER".to_owned(),
        "$MOD$ from $FROM$".to_owned(),
    )]
    .into_iter()
    .collect();
    loc.language = "english".into();
    let plane = NavalCombatPlane {
        ..Default::default()
    };
    let built = super::super::naval_air_tooltips::group_hover(
        &[&plane],
        &[("Mediterranean".to_owned(), "Spitfire, Hurricane".to_owned())],
        &[String::new()],
        "",
        0,
        100_000,
        "english",
        &loc,
        &NoIcons,
    )
    .unwrap();
    let mut out = Bindings::default();
    push_air_entry(
        &mut out,
        &mut BTreeMap::new(),
        0,
        Some("GFX_air".into()),
        [40.0, 20.0],
        Some("0/0".into()),
        hover_text(&built),
    );
    assert_eq!(
        out.naval_entries[0].values["icon"].tooltip.as_deref(),
        Some("Spitfire, Hurricane from Mediterranean")
    );
}
#[test]
fn organisation_uses_default_leader_and_recorded_hq_terms() {
    use replay::rows::{NavalShipModifierSource, NavalShipModifierTerm};
    let mut row = NavalCombatShipRow {
        live_ship: 1,
        base_organisation: 3_000_000,
        ..Default::default()
    };
    row.weapon_context.sources = vec![
        NavalShipModifierSource {
            scope: 9,
            terms: vec![NavalShipModifierTerm {
                key: 0x47,
                raw: 50_000,
                ..Default::default()
            }],
            ..Default::default()
        },
        NavalShipModifierSource {
            scope: 14,
            index: 0,
            terms: vec![NavalShipModifierTerm {
                key: 0x47,
                raw: 10_000,
                ..Default::default()
            }],
            ..Default::default()
        },
        NavalShipModifierSource {
            scope: 31,
            terms: vec![NavalShipModifierTerm {
                key: 0x46,
                raw: 200_000,
                ..Default::default()
            }],
            ..Default::default()
        },
    ];
    assert_eq!(organisation_maximum(&row), Ok(3_520_000));
    row.weapon_context.sources.pop();
    assert_eq!(organisation_maximum(&row), Ok(3_300_000));
    row.live_ship = 0;
    row.live_convoy = 1;
    row.base_organisation = 0;
    assert_eq!(
        organisation_maximum(&row),
        Err("the convoy records base organisation 0, not a positive value".into())
    );
}
#[test]
fn the_left_column_takes_the_second_frame_unless_reversed() {
    assert_eq!(icon_frame(0, false), 1);
    assert_eq!(icon_frame(0, true), 0);
    assert_eq!(icon_frame(1, true), 1);
    assert_eq!(icon_frame(1, false), 0);
    // pairJ_naval_5.png: submarines (48) at +77, convoys (52) at +75.
    assert_eq!(combat_box_x(48.0), 77.0);
    assert_eq!(combat_box_x(52.0), 75.0);
}

#[test]
fn reveal_and_exact_state_select_boxes() {
    let mut row = NavalCombatShipRow {
        state: 1,
        live_ship: 1,
        formation_flags: 1 << 7,
        ..Default::default()
    };
    assert_eq!(column(&row, 0), Some("hidden"));
    row.detection_state = 2;
    assert_eq!(column(&row, 0), Some("visible"));
    row.state = 32;
    row.live_ship = 0;
    assert_eq!(column(&row, 0), Some("sunk"));
    row.state = 33;
    assert_eq!(column(&row, 0), None);
}
#[test]
fn grouping_normalizes_state_but_preserves_damage() {
    let mut row = NavalCombatShipRow {
        state: 1,
        live_ship: 1,
        strength: 100,
        maximum_hit_points: 100,
        ..Default::default()
    };
    let healthy = key(&row, 7, false, 0);
    row.state = 2;
    assert!(healthy == key(&row, 7, false, 0));
    row.strength = 99;
    assert!(healthy != key(&row, 7, false, 0));
    row.state = 4;
    let unknown = key(&row, 7, true, 0);
    row.subunit_index = 42;
    row.owner = 8;
    row.strength = 100;
    assert!(unknown == key(&row, 8, true, 0));
}

#[test]
fn carrier_equipment_flags_select_the_carrier_group_before_screen_flags() {
    use replay::rows::{ShipCounterInputs, ShipModelRow};
    let mut row = NavalCombatShipRow {
        state: 2,
        live_ship: 1,
        ship: 123,
        formation_flags: 1 << 8,
        ..Default::default()
    };
    for carrier_flag in [22, 37] {
        let model = ShipModelRow {
            ship: row.ship,
            counter: Some(ShipCounterInputs {
                positive_equipment_flags: 1 << carrier_flag,
                ..Default::default()
            }),
            ..Default::default()
        };
        let models = BTreeMap::from([(model.ship, &model)]);
        let flags = pool_flags(&row, &models, None).unwrap();
        assert_eq!(column(&row, flags), Some("carrier"));
        assert_eq!(key(&row, 7, false, flags).unwrap().column, "carrier");
        row.formation_flags |= 1 << 7;
        assert_eq!(column(&row, flags), Some("hidden"));
        row.formation_flags = 1 << 8;
    }
    assert_eq!(column(&row, 0), Some("screen"));
    assert!(
        pool_flags(&row, &BTreeMap::new(), Some("17 rows not reached"))
            .unwrap_err()
            .contains("17 rows not reached")
    );
    row.formation_flags = 1 << 22;
    assert_eq!(pool_flags(&row, &BTreeMap::new(), None), Ok(0));
    assert_eq!(column(&row, 0), Some("carrier"));
}
