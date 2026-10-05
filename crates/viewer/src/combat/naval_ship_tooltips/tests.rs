use super::*;
use crate::combat::binding::NoIcons;

#[test]
fn naval_convoy_chevrons_and_condition_bars_own_the_member_hover() {
    use crate::map::native_gui::{
        lifecycle::Tick,
        tests::{fixture, windows},
    };

    let mut values = super::super::Values::new();
    for key in ["escape_icon_left", "escape_icon_right"] {
        set_hover(&mut values, key, Some("formation explanation".into()), None);
    }
    group_hovers(&mut values, Some("convoy escape blocks".into()));
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
    }
    // Exercise the actual retained hit order: the chevrons overlap a grid
    // with the formation explanation, but their member callback wins.
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
    }
    group_hovers(&mut values, None);
    assert!(
        values
            .values()
            .all(|widget| widget.hover_owned && widget.tooltip.is_none())
    );
}

#[test]
fn naval_convoy_group_keeps_localised_separators_between_all_three_blocks() {
    let localisation: Localisation = [
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
    let country = replay::rows::CountryRow {
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
        None,
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
    let country = replay::rows::CountryRow {
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

#[test]
fn escape_tree_index_keeps_member_identity_native_order_and_shared_borrows() {
    let usage = |combat, side, battle_slot, order| replay::rows::NavalModifierUse {
        combat,
        side,
        battle_slot,
        order,
        ..Default::default()
    };
    let tree = |accumulator, uses| replay::rows::NavalModifierBreakdownRow {
        accumulator,
        key: engine::modifier_keys::NAVAL_RETREAT_SPEED,
        uses,
        nodes: vec![replay::rows::ModifierBreakdownNode::default()],
    };
    let rows = [
        tree(
            1,
            vec![
                usage(7, 0, 3, 2),
                usage(7, 1, 3, 3),
                usage(8, 0, 3, 0),
                usage(7, 0, u32::MAX, 4),
            ],
        ),
        tree(2, vec![usage(7, 0, 3, 1), usage(7, 0, 4, 5)]),
    ];
    let indexed = member_trees(&rows, 7);
    assert_eq!(indexed.len(), 3);
    let sources = &indexed[&(7, 0, 3)];
    assert_eq!(
        sources
            .iter()
            .map(|source| source.use_.order)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(std::ptr::eq(sources[0].tree, &rows[1]));
    assert!(std::ptr::eq(sources[0].use_, &rows[1].uses[0]));
    assert!(std::ptr::eq(sources[1].tree, indexed[&(7, 1, 3)][0].tree));
    assert!(std::ptr::eq(indexed[&(7, 0, 4)][0].use_, &rows[1].uses[1]));
}

#[test]
fn escaping_member_section_has_native_newlines_and_source_order() {
    let localisation: Localisation = [
        ("ESCAPE_PROGRESS", "Progress|"),
        ("CHANCE_TO_ESCAPE", "Chance|"),
        ("BASE_ESCAPE_SPEED", "Base|"),
        ("ESCAPE_SPEED_COMBAT_DURATION", "Duration|"),
        ("ESCAPE_SPEED_FROM_SHIP_SPEED", "Speed|"),
        ("HIDDEN_SUBS_ESCAPE_BONUS", "Hidden|"),
    ]
    .into_iter()
    .map(|(key, text)| (key.to_owned(), text.to_owned()))
    .collect();
    let row = NavalCombatShipRow {
        state: 8,
        live_ship: 1,
        detection_state: 0,
        escape_progress: 50_000,
        ..Default::default()
    };
    let parts = super::super::naval_escape::Parts {
        base: 10_000,
        duration: 20_000,
        modifiers: 0,
        ship_speed: 630,
        detection: 5_000,
        total: 35_630,
    };
    assert_eq!(
        escape_text(
            &row,
            parts,
            &[],
            &[],
            &BTreeMap::new(),
            "english",
            &NoIcons,
            &localisation
        )
        .unwrap(),
        "\nProgress|\nChance|\n  Base|\n  Duration|\n  Speed|\n  Hidden|\n"
    );
}

#[test]
fn state_eight_ship_and_convoy_tree_sections_render_native_sources() {
    use replay::rows::{
        NavalModifierBreakdownRow, NavalModifierUse, NavalShipModifierSource, NavalShipModifierTerm,
    };
    let mut loc: Localisation = [
        ("ESCAPE_PROGRESS", "Progress $NUM$"),
        ("CHANCE_TO_ESCAPE", "Chance $NUM$"),
        ("BASE_ESCAPE_SPEED", "Base $NUM$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    loc.language = "english".into();
    for ship in [true, false] {
        let key = if ship { 0x5b } else { 0x5d };
        let row = NavalCombatShipRow {
            state: 8,
            live_ship: u8::from(ship),
            live_convoy: u8::from(!ship),
            detection_state: 1,
            escape_progress: 50_000,
            weapon_context: replay::rows::NavalShipWeaponContext {
                sources: [0, 1, 4, 11, 12]
                    .into_iter()
                    .map(|scope| NavalShipModifierSource {
                        scope,
                        weights: match scope {
                            4 => [1, 2],
                            11 => [2, 0],
                            _ => [0; 2],
                        },
                        terms: vec![NavalShipModifierTerm {
                            key,
                            raw: 20_000,
                            ..Default::default()
                        }],
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            },
            ..Default::default()
        };
        let source_names: &[(u8, &[u8])] = if ship {
            &[
                (0, b"Country"),
                (1, b"Ship"),
                (4, b"Doctrine"),
                (11, b"Critical"),
                (12, b"HQ"),
            ]
        } else {
            &[
                (0, b"Country"),
                (6, b"Weather"),
                (5, b"Sea"),
                (9, b"Admiral"),
            ]
        };
        let trees: Vec<_> = source_names
            .iter()
            .enumerate()
            .map(|(order, (scope, name))| NavalModifierBreakdownRow {
                key,
                uses: vec![NavalModifierUse {
                    source_scope: *scope,
                    order: order as u32,
                    ..Default::default()
                }],
                nodes: vec![
                    replay::rows::ModifierBreakdownNode {
                        children: 1,
                        value: 20_000,
                        ..Default::default()
                    },
                    replay::rows::ModifierBreakdownNode {
                        value: 20_000,
                        name: name.to_vec(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            })
            .collect();
        let uses = super::super::naval_summary_tooltips::expanded_trees(&trees);
        let definition = ModifierDefinitionRow {
            index: key,
            precision: 2,
            ..Default::default()
        };
        let definitions = BTreeMap::from([(key, &definition)]);
        let modifier = if ship {
            super::super::naval_environment::modifier(&row.weapon_context, key).unwrap()
        } else {
            trees.iter().map(|tree| tree.nodes[0].value).sum()
        };
        let parts = super::super::naval_escape::chance(
            &row,
            0,
            modifier,
            0,
            &super::super::naval_escape::Defines {
                base: 10_000,
                per_day: 0,
                duration_max: 0,
                speed_factor: 0,
                hidden_sub: 0,
                sub_base: 0,
                out_of_fuel: 0,
                initial_duration: 24,
            },
        );
        let text = escape_text(
            &row,
            parts,
            &[(key, 0)],
            &uses,
            &definitions,
            "english",
            &NoIcons,
            &loc,
        )
        .unwrap();
        assert_eq!(
            text,
            if ship {
                concat!(
                    "\nProgress 0.50\nChance 1.20\n  Base 0.10\n",
                    "  Country: \u{11}R+0.20\u{11}!\n",
                    "  Ship: \u{11}R+0.20\u{11}!\n",
                    "  Doctrine: \u{11}R+0.10\u{11}!\n",
                    "  Critical: \u{11}R+0.40\u{11}!\n",
                    "  \u{11}R+0.20\u{11}!\n",
                )
            } else {
                concat!(
                    "\nProgress 0.50\nChance 0.90\n  Base 0.10\n",
                    "  Country: \u{11}R+0.20\u{11}!\n",
                    "  Weather: \u{11}R+0.20\u{11}!\n",
                    "  Sea: \u{11}R+0.20\u{11}!\n",
                    "  Admiral: \u{11}R+0.20\u{11}!\n",
                )
            }
        );
    }
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
    let text = member_tooltip(&row, "AAA", &variants, &NoIcons, &loc(), None, None).unwrap();
    let ordered = ["Escort Mk II", "incoming", "S", "O", "A3", "C2", "T", "L"];
    let mut cursor = 0;
    for term in ordered {
        let position = text[cursor..].find(term).unwrap();
        cursor += position + term.len();
    }
    assert!(text.contains(&format!("{INLINE_FLAG}AAA")));
}

#[test]
fn cached_ship_text_draws_in_recorded_language_and_escape_requires_context() {
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
    // 1417D3220 reserves the sunk branch for two null live references.
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
    assert!(reason.contains("naval escape tooltip context"), "{reason}");
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
    let country = replay::rows::CountryRow {
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
