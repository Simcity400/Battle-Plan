use super::*;
use viewer_foundation::tooltip_text::NoIcons;

#[test]
fn naval_convoy_chevrons_and_condition_bars_own_the_member_hover() {
    content_reader_script::test_game::enter();
    use viewer_native_gui::{native_gui::lifecycle::Tick, test_support::gui::{fixture, windows}};

    let mut values = super::super::Values::new();
    for key in ["escape_icon_left", "escape_icon_right"] {
        set_hover(&mut values, key, Some("formation explanation".into()), None);
    }
    group_hovers(
        &mut values,
        Some("convoy escape blocks".into()),
        super::super::TooltipAnchor::NavalWindow,
    );
    for key in [
        "icon",
        "bar_str",
        "bar_org",
        "escape_progress",
        "escape_icon_left",
        "escape_icon_right",
    ] {
        assert_eq!(values[key].tooltip.as_deref(), Some("convoy escape blocks"));
        assert!(values[key].hover_owned);
        assert_eq!(
            values[key].tooltip_anchor,
            super::super::TooltipAnchor::NavalWindow
        );
    }
    let windows = windows(
        br#"guiTypes={
        containerWindowType={name=navalcombatview size={width=300 height=200} clipping=no
            gridboxType={name=grid_carrier position={x=20 y=20} size={width=80 height=60}
                slotsize={width=80 height=60} max_slots_horizontal=1 max_slots_vertical=1}}
        containerWindowType={name=navalcombat_ship_entry size={width=80 height=60} clipping=no
            iconType={name=escape_icon_left position={x=4 y=40} spriteType=GFX_even}
            iconType={name=escape_icon_right position={x=50 y=40} spriteType=GFX_even}}
    }"#,
    );
    let mut bindings = super::super::binding::Bindings::default();
    let chevrons = values
        .iter()
        .filter(|(key, _)| matches!(key.as_str(), "escape_icon_left" | "escape_icon_right"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    bindings.values.insert(
        "grid_carrier".into(),
        super::super::Widget {
            entry_template: Some("navalcombat_ship_entry".into()),
            entries: vec![chevrons],
            ..Default::default()
        },
    );
    set_hover(
        &mut bindings.values,
        "grid_carrier",
        Some("formation explanation".into()),
        None,
    );
    let mut gui = super::super::gui::Windows::default();
    let drawn = gui
        .draw(
            &windows,
            &fixture(),
            &bindings,
            1,
            super::super::Selection::Naval(7),
            [0.; 2],
            false,
            &mut None,
            [300., 200.],
            true,
            Tick { time: 0., dt: 0. },
            false,
            false,
            None,
        )
        .unwrap();
    assert!(drawn.refused.is_empty(), "{:?}", drawn.refused);
    for position in [[24., 60.], [70., 60.]] {
        let hit = drawn
            .tooltips
            .iter()
            .find(|hit| hit.rect[..2] == position)
            .unwrap();
        let point = [
            hit.rect[0] + hit.rect[2] / 2.,
            hit.rect[1] + hit.rect[3] / 2.,
        ];
        let top = drawn
            .tooltips
            .iter()
            .rev()
            .find(|hit| super::super::contains(hit.rect, point))
            .unwrap();
        assert_eq!(top.text.as_deref(), Some("convoy escape blocks"));
        assert_eq!(top.anchor, super::super::TooltipAnchor::NavalWindow);
    }
    group_hovers(&mut values, None, super::super::TooltipAnchor::NavalWindow);
    assert!(
        values
            .values()
            .all(|widget| widget.hover_owned && widget.tooltip.is_none())
    );
}

#[test]
fn naval_sunk_group_keeps_every_separator_native_newlines_and_pointer_anchor() {
    let base_localisation = loc();
    let ships: Vec<_> = ["Alpha", "Beta", "Gamma"]
        .into_iter()
        .enumerate()
        .map(|(index, name)| NavalCombatShipRow {
            side: 1,
            battle_slot: index as u32,
            owner: 4,
            ship: index as u64 + 1,
            state: 32,
            name: name.as_bytes().to_vec(),
            type_name: b"Destroyer".to_vec(),
            ..Default::default()
        })
        .collect();
    let by_slot = ships
        .iter()
        .map(|row| ((row.side, row.battle_slot), row))
        .collect();
    let country = replay_data::rows::CountryRow {
        id: 4,
        tag: b"AAA".to_vec(),
        ..Default::default()
    };
    let slots = [0, 1, 2];
    for separator in ["stacked divider\n", "stacked divider"] {
        let mut localisation: Localisation = base_localisation
            .iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .chain(std::iter::once((
                "SEPARATION_LINE".into(),
                separator.into(),
            )))
            .collect();
        localisation.language = "english".into();
        let text = group_tooltip(
            &slots,
            1,
            &by_slot,
            &BTreeMap::new(),
            &BTreeMap::from([(4, &country)]),
            &NoIcons,
            &localisation,
            None,
            Some("english"),
        )
        .unwrap();
        assert_eq!(
            text,
            format!(
                "{INLINE_FLAG}AAA \u{11}GAlpha:Destroyer\u{11}!\nsunk\n{separator}{INLINE_FLAG}AAA \u{11}GBeta:Destroyer\u{11}!\nsunk\n{separator}{INLINE_FLAG}AAA \u{11}GGamma:Destroyer\u{11}!\nsunk\n"
            )
        );
        let anchor = group_anchor(&slots, 1, &by_slot);
        assert_eq!(anchor, super::super::TooltipAnchor::Pointer);
        let mut values = super::super::Values::new();
        group_hovers(&mut values, Some(text.clone().into()), anchor);
        for widget in values.values() {
            assert_eq!(widget.tooltip.as_deref(), Some(text.as_str()));
            assert_eq!(widget.tooltip_anchor, super::super::TooltipAnchor::Pointer);
        }
    }
}

#[test]
fn naval_convoy_group_keeps_localised_separators_between_all_three_blocks() {
    let mut localisation: Localisation = [
        ("SHIP_AND_TYPE", "convoy"),
        ("NAVAL_COMBAT_SHIP_TOOLTIP", "member|"),
        ("FEX_STATE_ON_THE_WAY", "incoming"),
        ("NAVAL_COMBAT_SHIP_STRENGTH", "HP|"),
        ("NAVAL_COMBAT_SHIP_ORGANIZATION_FACTOR", "ORG|"),
        ("NAVAL_COMBAT_HOURS_TO_ARRIVE", "arrival|"),
        ("SEPARATION_LINE", "stacked divider\n"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    localisation.language = "english".into();
    let ships: Vec<_> = (0..3)
        .map(|battle_slot| NavalCombatShipRow {
            battle_slot,
            owner: 4,
            state: 4,
            live_convoy: 1,
            variant: 7,
            strength: 100_000,
            maximum_hit_points: 100_000,
            base_organisation: 100_000,
            ..Default::default()
        })
        .collect();
    let by_slot = ships
        .iter()
        .map(|row| ((row.side, row.battle_slot), row))
        .collect();
    let country = replay_data::rows::CountryRow {
        id: 4,
        tag: b"AAA".to_vec(),
        ..Default::default()
    };
    let variant = VariantDefinitionRow {
        id: 7,
        display_name: b"Transport".to_vec(),
        ..Default::default()
    };
    let text = group_tooltip(
        &[0, 1, 2],
        0,
        &by_slot,
        &BTreeMap::from([(7, &variant)]),
        &BTreeMap::from([(4, &country)]),
        &NoIcons,
        &localisation,
        None,
        Some("english"),
    )
    .unwrap();
    assert_eq!(
        text,
        concat!(
            "member|\u{11}WHP|\u{11}!ORG|arrival|\n",
            "stacked divider\n",
            "member|\u{11}WHP|\u{11}!ORG|arrival|\n",
            "stacked divider\n",
            "member|\u{11}WHP|\u{11}!ORG|arrival|\n",
        )
    );
}

#[test]
fn naval_hidden_submarine_hover_warns_but_keeps_native_member_details() {
    let mut localisation: Localisation = loc()
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain(std::iter::once((
            "NAVAL_COMBAT_HIDDEN_SUBS".into(),
            "unknown position\n".into(),
        )))
        .collect();
    localisation.language = "english".into();
    let mut row = NavalCombatShipRow {
        side: 1,
        battle_slot: 7,
        owner: 4,
        ship: 11,
        live_ship: 1,
        state: 2,
        detection_state: 0,
        formation_flags: 1 << 7,
        name: b"Recorded hull".to_vec(),
        type_name: b"Recorded class".to_vec(),
        base_organisation: 100_000,
        ..Default::default()
    };
    let country = replay_data::rows::CountryRow {
        id: 4,
        tag: b"AAA".to_vec(),
        ..Default::default()
    };
    let build = |row: &NavalCombatShipRow| {
        group_tooltip(
            &[7],
            1,
            &BTreeMap::from([((1, 7), row)]),
            &BTreeMap::new(),
            &BTreeMap::from([(4, &country)]),
            &NoIcons,
            &localisation,
            None,
            Some("english"),
        )
    };
    let text = build(&row).unwrap();
    assert_eq!(
        text,
        format!(
            concat!(
                "unknown position\n",
                "{}AAA Recorded hull:Recorded class fighting|",
                "\u{11}RS0.00|\u{11}!O0.00|\n",
            ),
            INLINE_FLAG
        )
    );
    row.detection_state = 1;
    let visible = build(&row).unwrap();
    assert_eq!(visible, text.strip_prefix("unknown position\n").unwrap());
}

fn loc() -> Localisation {
    [
        ("SHIP_AND_TYPE", "$SHIP$:$TYPE$"),
        ("NAVAL_COMBAT_SHIP_TOOLTIP", "$FLAG$ $NAME$ $STATUS$|"),
        ("FEX_STATE_FIGHTING", "fighting"),
        ("FEX_STATE_ON_THE_WAY", "incoming"),
        ("FEX_STATE_ESCAPING", "escaping"),
        ("FEX_STATE_SUNK", "sunk"),
        ("SHIP_SUNK_BY", "sunk by $NAME$"),
        ("NAVAL_COMBAT_SHIP_STRENGTH", "S$STR$|"),
        ("NAVAL_COMBAT_SHIP_ORGANIZATION_FACTOR", "O$ORG$|"),
        ("NAVAL_COMBAT_HOURS_TO_ARRIVE", "A$NUM$|"),
        ("NAVAL_COMBAT_RECEIVED_CRITICALS", "C$NUM$|"),
        ("total_damage_received", "T$NUM$|"),
        ("damage_dealt_light_guns", "L$NUM$/$RATIO$|"),
        ("damage_dealt_heavy_guns", "H$NUM$/$RATIO$|"),
        ("damage_dealt_torpedoes", "P$NUM$/$RATIO$|"),
        ("damage_dealt_depth_charges", "D$NUM$/$RATIO$|"),
        ("damage_dealt_air", "F$NUM$/$RATIO$|"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

#[test]
fn explicit_convoy_hover_orders_condition_arrival_criticals_and_damage() {
    let row = NavalCombatShipRow {
        state: 4,
        live_convoy: 1,
        variant: 7,
        strength: 50_000,
        maximum_hit_points: 100_000,
        organisation: 25_000,
        base_organisation: 100_000,
        arrival_hours: 3,
        received_criticals: 2,
        received_damage: [20_000, 0, 0, 0, 0, 5_000],
        ..Default::default()
    };
    let variant = VariantDefinitionRow {
        id: 7,
        display_name: b"Escort".to_vec(),
        variant_name: b"Mk II".to_vec(),
        append_variant_name: 1,
        ..Default::default()
    };
    let variants = BTreeMap::from([(7, &variant)]);
    let mut localisation = loc();
    localisation.language = "english".into();
    let tooltip = |language| {
        member_tooltip(
            &row,
            "AAA",
            &variants,
            &NoIcons,
            &localisation,
            None,
            language,
        )
    };
    let text = tooltip(Some("english")).unwrap();
    let ordered = ["Escort Mk II", "incoming", "S", "O", "A3", "C2", "T", "L"];
    let mut cursor = 0;
    for term in ordered {
        let position = text[cursor..].find(term).unwrap();
        cursor += position + term.len();
    }
    assert!(text.contains(&format!("{INLINE_FLAG}AAA")));
    let reason = tooltip(Some("german")).unwrap_err();
    assert!(reason.contains("variant 7 explicit name"), "{reason}");
    assert!(reason.contains("stored in german"), "{reason}");
    assert!(
        tooltip(None)
            .unwrap_err()
            .contains("has no recording language")
    );
}

#[test]
fn cached_ship_text_draws_in_recorded_language_and_escaping_members_refuse() {
    let ship = NavalCombatShipRow {
        side: 1,
        battle_slot: 9,
        ship: 11,
        live_ship: 0,
        state: 32,
        name: b"client-language ship".to_vec(),
        type_name: b"client-language type".to_vec(),
        sunk_by: b"the carrier".to_vec(),
        ..Default::default()
    };
    let mut localisation = loc();
    localisation.language = "english".into();
    let text = member_tooltip(
        &ship,
        "AAA",
        &BTreeMap::new(),
        &NoIcons,
        &localisation,
        None,
        Some("english"),
    )
    .unwrap();
    assert!(text.contains("client-language ship:client-language type"));
    assert!(text.contains("sunk by the carrier"));
    let reason = member_tooltip(
        &ship,
        "AAA",
        &BTreeMap::new(),
        &NoIcons,
        &localisation,
        None,
        Some("german"),
    )
    .unwrap_err();
    assert!(reason.contains("stored in german"), "{reason}");
    let mut still_live = ship.clone();
    still_live.live_ship = 1;
    let live_text = member_tooltip(
        &still_live,
        "AAA",
        &BTreeMap::new(),
        &NoIcons,
        &localisation,
        None,
        Some("english"),
    )
    .unwrap();
    assert!(!live_text.contains("sunk by the carrier"));
    let mut escaping = ship;
    escaping.state = 8;
    escaping.live_convoy = 1;
    escaping.base_organisation = 100_000;
    let reason = member_tooltip(
        &escaping,
        "AAA",
        &BTreeMap::new(),
        &NoIcons,
        &localisation,
        None,
        Some("english"),
    )
    .unwrap_err();
    assert_eq!(reason, "side 1 slot 9 has no naval escape tooltip context");
}

#[test]
fn an_escaping_member_withholds_its_group_by_name_while_others_still_build() {
    let mut localisation = loc();
    localisation.language = "english".into();
    let ships: Vec<_> = [(2, 2), (3, 8)]
        .into_iter()
        .map(|(battle_slot, state)| NavalCombatShipRow {
            side: 0,
            battle_slot,
            owner: 4,
            ship: u64::from(battle_slot),
            live_ship: 1,
            state,
            escape_progress: 50_000,
            name: b"Hull".to_vec(),
            type_name: b"Class".to_vec(),
            base_organisation: 100_000,
            ..Default::default()
        })
        .collect();
    let by_slot = ships
        .iter()
        .map(|row| ((row.side, row.battle_slot), row))
        .collect();
    let country = replay_data::rows::CountryRow {
        id: 4,
        tag: b"AAA".to_vec(),
        ..Default::default()
    };
    let countries = BTreeMap::from([(4, &country)]);
    let build = |slots: &[u32]| {
        group_tooltip(
            slots,
            0,
            &by_slot,
            &BTreeMap::new(),
            &countries,
            &NoIcons,
            &localisation,
            None,
            Some("english"),
        )
    };
    assert_eq!(
        build(&[2]).unwrap(),
        format!("{INLINE_FLAG}AAA Hull:Class fighting|\u{11}RS0.00|\u{11}!O0.00|\n")
    );
    assert_eq!(
        build(&[2, 3]).unwrap_err(),
        ["side 0 slot 3 has no naval escape tooltip context"]
    );
}

fn escape_tree(
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

#[test]
fn an_escaping_member_lists_its_source_trees_after_the_chance() {
    use replay_data::naval_trees::Kind;
    let mut localisation: Localisation = [
        ("ESCAPE_PROGRESS", "Progress $NUM|%0$"),
        ("CHANCE_TO_ESCAPE", "Chance $NUM|%0$"),
        ("BASE_ESCAPE_SPEED", "Base $NUM|%0$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    localisation.language = "english".into();
    let source = |scope| replay_data::rows::NavalShipModifierSource {
        scope,
        ..Default::default()
    };
    let ship = NavalCombatShipRow {
        combat: 42,
        side: 0,
        battle_slot: 3,
        ship: 41,
        state: 8,
        live_ship: 1,
        detection_state: 1,
        escape_progress: 50_000,
        weapon_context: replay_data::rows::NavalShipWeaponContext {
            task_force: 77,
            country: 7,
            commander: 66,
            sources: vec![source(0), source(9)],
            ..Default::default()
        },
        ..Default::default()
    };
    let mut combat = replay_data::rows::NavalCombatDetailsRow {
        combat: 42,
        ..Default::default()
    };
    combat.sides[0].summary.countries = vec![7];
    combat.sides[0].commander = 66;
    let mut country = escape_tree(Kind::Country, 7, 0x5b, 10_000, b"Doctrine");
    country
        .trees
        .extend(escape_tree(Kind::Country, 7, 0x5d, 30_000, b"Convoy doctrine").trees);
    let rows = [
        country,
        escape_tree(Kind::Commander, 66, 0x5b, 5_000, b"Trait"),
    ];
    let definitions = [0x5b, 0x5d].map(|index| replay_data::rows::ModifierDefinitionRow {
        index,
        precision: 2,
        ..Default::default()
    });
    let sources = crate::naval_trees::Sources::from_parts(
        Ok(&rows),
        &definitions,
        crate::naval_labels::english(),
    );
    let defines = crate::naval_escape::Defines {
        base: 10_000,
        per_day: 0,
        duration_max: 0,
        speed_factor: 0,
        hidden_sub: 0,
        sub_base: 0,
        out_of_fuel: 0,
        initial_duration: 24,
    };
    let screening = crate::naval_escape::Screening::zero();
    let text = escape_text(
        &ship,
        &combat,
        &sources,
        &defines,
        &screening,
        &NoIcons,
        &localisation,
    )
    .unwrap();
    let doctrine = text.find("  Doctrine: ").expect(&text);
    let trait_line = text.find("  Trait: ").expect(&text);
    assert!(text.find("Chance").unwrap() < doctrine, "{text}");
    assert!(doctrine < trait_line, "{text}");
    assert!(!text.contains("Convoy doctrine"), "{text}");
    let convoy = NavalCombatShipRow {
        live_ship: 0,
        live_convoy: 1,
        weapon_context: Default::default(),
        ..ship.clone()
    };
    let text = escape_text(
        &convoy,
        &combat,
        &sources,
        &defines,
        &screening,
        &NoIcons,
        &localisation,
    )
    .unwrap();
    assert!(text.contains("  Convoy doctrine: "), "{text}");
    assert!(text.contains("Chance 40"), "{text}");
    let missing = crate::naval_trees::Sources::from_parts(
        Err("NavalModifierTrees not recorded".into()),
        &definitions,
        crate::naval_labels::english(),
    );
    assert!(
        escape_text(
            &ship,
            &combat,
            &missing,
            &defines,
            &screening,
            &NoIcons,
            &localisation
        )
        .unwrap_err()
        .contains("NavalModifierTrees")
    );
}

#[test]
fn one_unavailable_member_withholds_the_whole_group_and_counts_each_gap() {
    let ships = [
        NavalCombatShipRow {
            side: 0,
            battle_slot: 2,
            owner: 4,
            ship: 10,
            live_ship: 1,
            ..Default::default()
        },
        NavalCombatShipRow {
            side: 0,
            battle_slot: 3,
            owner: 4,
            ship: 11,
            live_ship: 1,
            ..Default::default()
        },
    ];
    let by_slot = ships
        .iter()
        .map(|row| ((row.side, row.battle_slot), row))
        .collect();
    let country = replay_data::rows::CountryRow {
        id: 4,
        tag: b"AAA".to_vec(),
        ..Default::default()
    };
    let countries = BTreeMap::from([(4, &country)]);
    let result = group_tooltip(
        &[2, 3],
        0,
        &by_slot,
        &BTreeMap::new(),
        &countries,
        &NoIcons,
        &loc(),
        None,
        None,
    );
    let errors = result.unwrap_err();
    assert_eq!(errors.len(), 2);
    assert!(errors[0].contains("slot 2"));
    assert!(errors[1].contains("slot 3"));
}
