use super::*;
use viewer_foundation::tooltip_text::NoIcons;

#[test]
fn aircraft_groups_follow_model_then_wing_hash_and_report_unrecorded_ties() {
    let low = NavalCombatPlane {
        wings: [vec![], vec![1 << 32]],
        ..Default::default()
    };
    let high = NavalCombatPlane {
        wings: [vec![], vec![2 << 32]],
        ..Default::default()
    };
    let mut groups = [
        AirGroup {
            side: 0,
            model: 2,
            owner: 10,
            planes: vec![&low],
        },
        AirGroup {
            side: 0,
            model: 7,
            owner: 11,
            planes: vec![&low],
        },
        AirGroup {
            side: 0,
            model: 7,
            owner: 12,
            planes: vec![&high],
        },
        AirGroup {
            side: 1,
            model: 9,
            owner: 13,
            planes: vec![&high],
        },
    ];
    assert!(sort_air_groups(&mut groups).is_empty());
    assert_eq!(groups.map(|group| group.owner), [12, 11, 10, 13]);
    let mut tied = [
        AirGroup {
            side: 0,
            model: 7,
            owner: 11,
            planes: vec![&low],
        },
        AirGroup {
            side: 0,
            model: 7,
            owner: 12,
            planes: vec![&low],
        },
    ];
    assert_eq!(sort_air_groups(&mut tied), BTreeSet::from([0]));
}

#[test]
fn distinct_nonnegative_ship_slots_resolve_order_before_the_address_tie() {
    let mut first = NavalCombatShipRow {
        state: 2,
        battle_slot: 1,
        live_ship: 1,
        ..Default::default()
    };
    let mut second = NavalCombatShipRow {
        battle_slot: 2,
        ..first.clone()
    };
    let first_key = key(&first, 7, false, 0).unwrap();
    let second_key = key(&second, 8, false, 0).unwrap();
    assert!(ship_order(&second_key, &[&second]) < ship_order(&first_key, &[&first]));
    first.battle_slot = u32::MAX;
    second.battle_slot = u32::MAX - 1;
    assert_eq!(
        ship_order(&first_key, &[&first]),
        ship_order(&second_key, &[&second])
    );
}

#[test]
fn incoming_groups_take_the_first_unknown_or_matching_cached_identity() {
    let row = NavalCombatShipRow {
        state: 4,
        live_ship: 1,
        ..Default::default()
    };
    let known = key(&row, 7, false, 0).unwrap();
    let mut unknown = key(&row, 8, true, 0).unwrap();
    let mut slots = GroupSlots::default();
    assert_eq!(slots.insert(&known, 0), 0);
    assert_eq!(slots.insert(&unknown, 1), 1);
    unknown.owner = 7;
    assert_eq!(slots.insert(&unknown, 2), 0);
    unknown.owner = 9;
    assert_eq!(slots.insert(&unknown, 2), 1);
    let mut disclosed = unknown.clone();
    disclosed.unknown = false;
    disclosed.owner = 8;
    assert_eq!(slots.insert(&disclosed, 2), 1);
    disclosed.owner = 9;
    assert_eq!(slots.insert(&disclosed, 2), 2);
}

#[test]
fn live_pride_uses_current_roster_ownership_and_sunk_pride_uses_the_cached_flag() {
    let mut row = NavalCombatShipRow {
        ship: 11,
        live_ship: 1,
        pride: 1,
        ..Default::default()
    };
    row.weapon_context.task_force = 22;
    let force = replay_data::rows::TaskForceRow {
        id: 22,
        roster_owner: 3,
        ..Default::default()
    };
    let mut country = replay_data::rows::CountryCounterInputRow {
        country: 4,
        pride_of_the_fleet: 12,
        ..Default::default()
    };
    let forces = BTreeMap::from([(22, &force)]);
    let aliases = BTreeMap::from([(3, 30), (4, 30)]);
    assert_eq!(
        group_pride(
            &[&row],
            false,
            &forces,
            &BTreeMap::from([(30, &country)]),
            &aliases
        ),
        Ok(false)
    );
    country.pride_of_the_fleet = 11;
    row.pride = 0;
    assert_eq!(
        group_pride(
            &[&row],
            false,
            &forces,
            &BTreeMap::from([(30, &country)]),
            &aliases
        ),
        Ok(true)
    );
    assert_eq!(
        group_pride(&[&row], true, &BTreeMap::new(), &BTreeMap::new(), &aliases),
        Ok(false)
    );
    row.pride = 1;
    assert_eq!(
        group_pride(&[&row], true, &BTreeMap::new(), &BTreeMap::new(), &aliases),
        Ok(true)
    );
    assert!(group_pride(&[&row], false, &BTreeMap::new(), &BTreeMap::new(), &aliases).is_err());
}

#[test]
fn condition_means_preserve_native_rounding_and_exceptional_operands() {
    let mut values = Values::new();
    for (raw, percent) in [(49_499, 49), (49_500, 50), (100_500, 101), (-500, -1)] {
        bar(&mut values, "bar_str", raw);
        assert_eq!(values["bar_str"].progress, Some(percent));
    }
    assert_eq!(fixed_ratio(i64::MAX, 100), super::super::fixed::EXCEPTIONAL);
    assert_eq!(mean(49_999, 2), 24_999);
    assert_eq!(mean(0, 0), 0);
    bar(&mut values, "bar_org", fixed_ratio(1_485_001, 3_000_001));
    assert_eq!(values["bar_org"].progress, Some(50));
    bar(&mut values, "escape_progress", mean(49_499 + 49_501, 2));
    assert_eq!(values["escape_progress"].progress, Some(50));
}

#[test]
fn country_aliases_name_the_canonical_slots_row() {
    let countries = [
        CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"AAA".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 9,
            canonical_slot: 2,
            tag: b"BBB".to_vec(),
            ..Default::default()
        },
    ];
    let links = BTreeMap::from([(7, 1), (3, 1), (9, 2), (5, 4)]);
    let by_reference = countries_by_reference(&links, &countries);
    assert_eq!(by_reference[&3].tag, b"AAA");
    assert_eq!(by_reference[&7].tag, b"AAA");
    assert_eq!(by_reference[&9].tag, b"BBB");
    assert!(!by_reference.contains_key(&5));
    assert!(!by_reference.contains_key(&8));
}

#[test]
fn a_refused_flag_preserves_known_naval_counts_and_other_country_flags() {
    let reason = "country 7 original tag is not recorded".to_owned();
    let flags = Flags::from([(7, Err(reason.clone())), (8, Ok("BBB".to_owned()))]);
    let mut out = Bindings::default();
    for side in 0..2 {
        let mut values = Values::new();
        descriptions(&mut out, &mut values, 7, 3, side, &flags);
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
        efficiency: replay_data::rows::NavalCombatAirEfficiency {
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
fn only_a_linked_carrier_below_full_sortie_efficiency_reaches_its_member_sources() {
    let mut plane = NavalCombatPlane {
        wings: [vec![], vec![7]],
        efficiency: replay_data::rows::NavalCombatAirEfficiency {
            source: 3,
            ..Default::default()
        },
        ..Default::default()
    };
    let detail = replay_data::rows::NavalCombatDetailsRow::default();
    let joins = super::super::naval_air_groups::CarrierJoins::new(None, None, None, None);
    let rows: [replay_data::rows::NavalModifierTreeRow; 0] = [];
    let trees = super::super::naval_trees::Sources::from_parts(
        Ok(&rows),
        &[],
        crate::naval_labels::english(),
    );
    let content = super::super::naval_air_groups::Content::default();
    let loc = content_reader_localisation::localisation::Localisation::default();
    let breakdown = |plane: &NavalCombatPlane, efficiency| {
        sortie_member_breakdown(
            plane, efficiency, &detail, 0, &joins, &trees, &content, &NoIcons, &loc,
        )
    };
    assert_eq!(breakdown(&plane, 40_000), Ok(String::new()));
    plane.carrier_member_present = 1;
    assert_eq!(breakdown(&plane, 100_000), Ok(String::new()));
    assert_eq!(
        breakdown(&plane, 40_000),
        Err("linked carrier member is unavailable".into())
    );
}

#[test]
fn air_entries_bind_before_carrier_ships_on_both_grids() {
    let mut out = Bindings::default();
    push_air_entry(
        &mut out,
        0,
        Some("GFX_air_left".into()),
        [40.0, 20.0],
        Some("3/5".into()),
        Some("Air left".into()),
    );
    push_air_entry(
        &mut out,
        1,
        Some("GFX_air_right".into()),
        [42.0, 24.0],
        Some("2/8".into()),
        Some("Air right".into()),
    );
    for side in 0..2 {
        out.naval_entries.push(NavalEntry {
            layout: crate::binding::NavalLayout::Ship,
            side,
            unknown: false,
            grid: format!(
                "battle_window/top_wnd/{}_wnd/grid_carrier",
                if side == 0 { "left" } else { "right" }
            ),
            offset: Some([0.0; 2]),
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
    assert_eq!(out.naval_entries[0].values["icon"].frame, Some(1));
    assert_eq!(out.naval_entries[1].values["icon"].frame, Some(0));
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
    push_air_entry(&mut out, 0, None, [1.0, 3.0], Some("4/7".into()), None);
    let missing = out.naval_entries.last().unwrap();
    assert!(missing.values["icon"].hidden);
    assert!(missing.values["icon"].sprite.is_none());
    assert_eq!(missing.values["desc_left"].text.as_deref(), Some("4/7"));
}

#[test]
fn recorded_air_hover_reaches_the_air_icon_binding() {
    let mut loc: content_reader_localisation::localisation::Localisation = [(
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
    use engine_executable::modifier_keys::{NAVY_ORG, NAVY_ORG_FACTOR};
    use replay_data::{rows::NavalShipModifierSource, rows::NavalShipModifierTerm};
    let mut row = NavalCombatShipRow {
        live_ship: 1,
        base_organisation: 3_000_000,
        ..Default::default()
    };
    row.weapon_context.sources = vec![
        NavalShipModifierSource {
            scope: 9,
            terms: vec![NavalShipModifierTerm {
                key: NAVY_ORG_FACTOR,
                raw: 50_000,
                ..Default::default()
            }],
            ..Default::default()
        },
        NavalShipModifierSource {
            scope: 14,
            index: 0,
            terms: vec![NavalShipModifierTerm {
                key: NAVY_ORG_FACTOR,
                raw: 10_000,
                ..Default::default()
            }],
            ..Default::default()
        },
        NavalShipModifierSource {
            scope: 31,
            terms: vec![NavalShipModifierTerm {
                key: NAVY_ORG,
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
    assert_eq!(organisation_maximum(&row), Ok(100));
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
    assert!(unknown != key(&row, 8, true, 0));
}

#[test]
fn carrier_equipment_flags_select_the_carrier_group_before_screen_flags() {
    use replay_data::{rows::ShipCounterInputs, rows::ShipModelRow};
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
