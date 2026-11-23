use super::*;
use viewer_foundation::test_support::localisation::loaded;

#[test]
fn night_picture_uses_viewer_pause_at_max_recorded_speed() {
    assert!(night_picture(3, false));
    assert!(!night_picture(4, false));
    assert!(night_picture(4, true));
}

#[test]
fn custom_sea_grid_keeps_active_template_order_and_signed_words() {
    let names = ["reef".into(), "storm".into(), "reef".into(), "ice".into()];
    let bytes = [1_i16, 0, -2, 4]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();
    let values = custom_sea_words(&bytes);
    assert_eq!(values, [1, 0, -2, 4]);
    assert_eq!(
        active_custom_sea(&names, &values)
            .unwrap()
            .collect::<Vec<_>>(),
        ["reef", "reef", "ice"]
    );
    assert!(active_custom_sea(&names, &values[..values.len() - 1]).is_none());
}

#[test]
fn a_regions_custom_sea_comes_from_a_live_combat_else_its_weather_row() {
    use replay_data::rows::{NavalCombatDetailsRow, TableRows, WeatherRegionRow};
    use viewer_foundation::test_support::{test_frame, test_put as put};
    let region_of = std::collections::BTreeMap::from([(10, 3)]);
    let weather = |custom_sea| WeatherRegionRow { id: 3, custom_sea, ..Default::default() };
    let mut frame = test_frame(0);
    put(&mut frame, TableRows::NavalCombatDetails(Vec::new()));
    assert_eq!(region_custom_sea(&region_of, &frame, &weather(Recorded::Value(Some(vec![0, 2]))), 3), Ok(vec![0, 2]));
    assert_eq!(region_custom_sea(&region_of, &frame, &weather(Recorded::Value(None)), 3), Err("the region's custom sea values were refused when read".to_owned()));
    assert_eq!(region_custom_sea(&region_of, &frame, &weather(Recorded::NotRecorded), 3), Err("the region's custom sea values are not recorded".to_owned()));
    put(&mut frame, TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow { province: 10, custom_sea: Some(vec![5, 0, 0, 0]), ..Default::default() }]));
    assert_eq!(region_custom_sea(&region_of, &frame, &weather(Recorded::NotRecorded), 3), Ok(vec![5, 0]));
}

#[test]
fn modifier_tooltip_weather_grid_keeps_native_order_and_hover_description() {
    use replay_data::rows::ModifierDefinitionRow;
    use viewer_foundation::tooltip_text::NoIcons;
    let weather = WeatherRegionRow {
        heavy_rain: 1,
        snow: 1,
        arctic_water: 1,
        ..Default::default()
    };
    assert_eq!(
        active_weather_events(&weather).collect::<Vec<_>>(),
        vec![
            (1, "weather_rain_heavy", "rain_heavy_icon"),
            (2, "weather_snow", "snow_icon"),
            (5, "weather_arctic_water", "arctic_water_icon"),
        ]
    );
    let row = ModifierDefinitionRow {
        index: 7,
        name_key: b"MOD_RANGE".to_vec(),
        category_mask: 1,
        ..Default::default()
    };
    let air = ModifierDefinitionRow {
        index: 8,
        name_key: b"AIR_ONLY".to_vec(),
        category_mask: 2,
        ..Default::default()
    };
    let rows = [row, air];
    let definitions = Definitions::from_parts(&rows, &[], None);
    let block = Block {
        values: BTreeMap::from([(7, 10_000), (8, 15_000)]),
        ..Block::default()
    };
    let loc: Localisation = [
        ("weather_rain_heavy".into(), "Heavy Rain".into()),
        ("MOD_RANGE".into(), "Range".into()),
        ("AIR_ONLY".into(), "Air only".into()),
    ]
    .into_iter()
    .collect();
    assert!(block.has_category_one(&definitions).unwrap());
    let tooltip =
        weather_tooltip("weather_rain_heavy", &block, &definitions, &NoIcons, &loc).unwrap();
    assert!(tooltip.starts_with("\u{11}HHeavy Rain\u{11}!\n\nRange: "));
    assert!(!tooltip.contains("Air only"), "{tooltip}");
}

#[test]
fn terrain_hover_uses_recorded_region_then_province_modifier_sources() {
    use replay_data::{rows::ModifierDefinitionRow, rows::NavalShipModifierTerm};
    use viewer_foundation::tooltip_text::NoIcons;
    let definitions_rows = [
        ModifierDefinitionRow {
            index: 7,
            name_key: b"WEATHER_RANGE".to_vec(),
            category_mask: 1,
            ..Default::default()
        },
        ModifierDefinitionRow {
            index: 8,
            name_key: b"NIGHT_RANGE".to_vec(),
            category_mask: 1,
            ..Default::default()
        },
        ModifierDefinitionRow {
            index: 9,
            name_key: b"AIR_ONLY".to_vec(),
            category_mask: 2,
            ..Default::default()
        },
    ];
    let definitions = Definitions::from_parts(&definitions_rows, &[], None);
    let source = |scope, key| NavalShipModifierSource {
        scope,
        terms: vec![NavalShipModifierTerm {
            key,
            raw: 10_000,
            ..Default::default()
        }],
        ..Default::default()
    };
    let context = NavalShipWeaponContext {
        terrain_name: b"REGION".to_vec(),
        night: ONE,
        sources: vec![
            NavalShipModifierSource {
                terms: vec![
                    NavalShipModifierTerm {
                        key: 7,
                        raw: 10_000,
                        ..Default::default()
                    },
                    NavalShipModifierTerm {
                        key: 9,
                        raw: 15_000,
                        ..Default::default()
                    },
                ],
                ..source(6, 7)
            },
            source(7, 8),
        ],
        ..Default::default()
    };
    let weather = WeatherRegionRow {
        heavy_rain: 1,
        snow: 1,
        ..Default::default()
    };
    let loc: Localisation = [
        ("REGION", "Western Sea"),
        ("weather_rain_heavy", "Heavy Rain"),
        ("weather_snow", "Snow"),
        ("WEATHER_MODIFIERS", "Weather: $WEA$"),
        ("NIGHT_MODIFIERS", "Night"),
        ("WEATHER_RANGE", "Weather range"),
        ("NIGHT_RANGE", "Night range"),
        ("AIR_ONLY", "Air only"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let terrain = super::super::naval_terrain::Content::test_empty(b"REGION");
    let hover = terrain_hover(
        &weather,
        &context,
        &terrain,
        &definitions,
        &NoIcons,
        &loc,
        Ok("AAA"),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        hover,
        "\u{11}HWestern Sea\u{11}!\n\n\u{11}HWeather: Heavy Rain, Snow\u{11}!\n  - Weather range: \u{11}R+0\u{11}!\n\n\n\u{11}HNight\u{11}!\n  - Night range: \u{11}R+0\u{11}!"
    );
    let mut only_air_weather = context.clone();
    only_air_weather.sources[0]
        .terms
        .retain(|term| term.key == 9);
    let hover = terrain_hover(
        &weather,
        &only_air_weather,
        &terrain,
        &definitions,
        &NoIcons,
        &loc,
        Ok("AAA"),
        &mut Vec::new(),
    )
    .unwrap();
    assert_eq!(
        hover,
        "\u{11}HWestern Sea\u{11}!\n\n\u{11}HWeather: Heavy Rain, Snow\u{11}!\n\n\n\u{11}HNight\u{11}!\n  - Night range: \u{11}R+0\u{11}!"
    );
}

#[test]
fn terrain_hover_joins_live_ship_at_battle_province() {
    let ship = |task_force, night| NavalCombatShipRow {
        live_ship: 1,
        weapon_context: NavalShipWeaponContext {
            task_force,
            night,
            ..Default::default()
        },
        ..Default::default()
    };
    let elsewhere = ship(10, 0);
    let battle = ship(20, ONE);
    let tasks = BTreeMap::from([(10, 9), (20, 7)]);
    let ships = [&elsewhere, &battle];
    assert_eq!(
        joined_weather_context(&ships, &tasks, 7).unwrap().night,
        ONE
    );
    let disputed = ship(20, 0);
    let ships = [&battle, &disputed];
    assert!(
        joined_weather_context(&ships, &tasks, 7)
            .unwrap_err()
            .contains("disagree")
    );
}

#[test]
fn a_weather_icon_is_localised_and_a_missing_key_is_its_bytes() {
    let localisation = loaded(concat!(
        "l_english:\n",
        " snow_icon:0 \"GFX_weather:3\"\n",
        " rain_light_icon:0 \"\"\n",
    ));
    let sprites = |sprite: &str| ["GFX_weather", "blizzard_icon"].contains(&sprite);
    assert_eq!(
        weather_icon(&localisation, "snow_icon", sprites),
        Ok(Some(("GFX_weather".to_owned(), 2)))
    );
    assert_eq!(
        weather_icon(&localisation, "rain_light_icon", sprites),
        Ok(None)
    );
    assert_eq!(
        weather_icon(&localisation, "blizzard_icon", sprites),
        Ok(Some(("blizzard_icon".to_owned(), 0)))
    );
    assert_eq!(
        weather_icon(&localisation, "arctic_water_icon", sprites),
        Err("its sprite arctic_water_icon is missing from the content".to_owned())
    );
}
#[test]
fn weather_picture_uses_ui_precedence_and_terrain_fallback() {
    let mut w = WeatherRegionRow {
        light_rain: 1,
        heavy_rain: 1,
        blizzard: 1,
        snow: 1,
        ..Default::default()
    };
    assert_eq!(weather_suffix(&w), "storm");
    w.heavy_rain = 0;
    assert_eq!(weather_suffix(&w), "rain");
    w.light_rain = 0;
    assert_eq!(weather_suffix(&w), "snow_storm");
    assert_eq!(
        picture_names(Some("water_deep_ocean"), true, "rain", true),
        vec![
            "GFX_terrain_ongoing_sea_battle_water_deep_ocean_night_rain",
            "GFX_terrain_ongoing_sea_battle_water_deep_ocean_night",
            "GFX_terrain_ongoing_sea_battle_water_night_rain",
            "GFX_terrain_ongoing_sea_battle_water_night"
        ]
    );
}
#[test]
fn weather_icon_text_reads_sprite_and_one_based_frame() {
    assert_eq!(weather_icon_text(""), Ok(None));
    assert_eq!(weather_icon_text("GFX_rain"), Ok(Some(("GFX_rain", 0))));
    assert_eq!(weather_icon_text("GFX_rain:3"), Ok(Some(("GFX_rain", 2))));
    assert_eq!(
        weather_icon_text("GFX_rain: 3x:9"),
        Ok(Some(("GFX_rain", 2)))
    );
    assert!(
        weather_icon_text("GFX_rain:x")
            .unwrap_err()
            .contains("frame 0")
    );
    assert!(weather_icon_text("GFX_rain:-2").is_err());
}

#[test]
fn a_convoy_total_is_shown_only_when_no_allowed_value_changes_the_whole_number() {
    assert_eq!(bounded_total(0, 1, Ok([0, 25_000])), Some(0));
    assert_eq!(bounded_total(8_190_000, 0, Ok([0, 25_000])), Some(81));
    assert_eq!(bounded_total(8_190_000, 1, Ok([0, 25_000])), None);
    assert_eq!(bounded_total(0, 4, Ok([0, 25_000])), None);
    assert_eq!(bounded_total(0, 4, Ok([25_000, 25_000])), Some(1));
    assert_eq!(bounded_total(0, 2, Ok([25_000, 25_000])), Some(0));
    assert_eq!(bounded_total(100_000, 1, Ok([-1, 0])), None);
    assert_eq!(bounded_total(0, 1, Err("x")), None);
}

#[test]
fn torpedo_counter_sums_current_attack_before_truncating_to_177() {
    let ship = NavalCombatShipRow {
        live_ship: 1,
        weapon_context: NavalShipWeaponContext {
            weapon_bases: [0, 0, 0, 8_875_000, 0],
            sources: vec![NavalShipModifierSource {
                scope: 26,
                ..Default::default()
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let attack = weapon(&ship, 3, false).unwrap();
    assert_eq!(attack, 8_875_000);
    assert_eq!(bounded_total(add(attack, attack), 0, Ok([0, 0])), Some(177));
}

#[test]
fn carrier_penalty_uses_opponent_silhouette_and_actual_multiplier() {
    use replay_data::{rows::NavalCombatPlane, rows::ShipCounterInputs};
    let defines = content_reader_naval_combat_defines::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0, 0, 80],
        silhouette_planes: [10, 5, 16, 3, 4, 7],
        screen_cap_reduction_factor: 2_000,
        plane_stacking_penalty_effect: 500,
        sortie_stance_ratios: vec![ONE],
        base_carrier_sortie_efficiency: ONE,
        selected_sortie_initial_time: 0,
        piercing: Ok(Default::default()),
        positioning_impacts: [0; 3],
    };
    let model = ShipModelRow {
        ship: 1,
        counter: Some(ShipCounterInputs {
            positive_equipment_flags: 1 << 37,
            ..Default::default()
        }),
        ..Default::default()
    };
    let models = [model];
    let models = Keyed::new(&models, |model| model.ship).unwrap();
    let opponent = NavalCombatShipRow {
        combat: 1,
        side: 1,
        group_index: 0,
        ship: 1,
        live_ship: 1,
        state: 2,
        detection_state: 1,
        ..Default::default()
    };
    let opponents = [&opponent];
    let mut side = NavalCombatSideDetails::default();
    assert_eq!(opposing_silhouette(&opponents, models, 1, &defines), Ok(16));
    assert_eq!(
        carrier_penalty_values(&side, &opponents, models, 1, &defines).map(|value| value.0),
        Ok(ONE)
    );
    side.planes = vec![
        NavalCombatPlane {
            source: 0,
            carrier_state: 2,
            current_planes: 80,
            ..Default::default()
        },
        NavalCombatPlane {
            source: 1,
            carrier_state: 2,
            current_planes: 200,
            ..Default::default()
        },
        NavalCombatPlane {
            source: 0,
            carrier_state: 32,
            current_planes: 200,
            ..Default::default()
        },
    ];
    assert_eq!(
        carrier_penalty_values(&side, &opponents, models, 1, &defines).map(|value| value.0),
        Ok(ONE)
    );
    side.planes[0].current_planes += 1;
    assert_eq!(
        carrier_penalty_values(&side, &opponents, models, 1, &defines).map(|value| value.0 < ONE),
        Ok(true)
    );
    let mut capital = opponent.clone();
    capital.ship = 2;
    capital.formation_flags = 1 << 6;
    let opposing: Vec<_> = std::iter::repeat_n(&capital, 9).collect();
    assert_eq!(opposing_silhouette(&opposing, models, 1, &defines), Ok(90));
    assert_eq!(
        carrier_penalty_values(&side, &opposing, models, 1, &defines).map(|value| value.0),
        Ok(ONE)
    );
    let mut no_effect = defines.clone();
    no_effect.plane_stacking_penalty_effect = 0;
    assert_eq!(
        carrier_penalty_values(&side, &opponents, models, 1, &no_effect).map(|value| value.0),
        Ok(ONE)
    );
}
#[test]
fn carrier_laurel_has_native_alpha_and_hides_on_missing_inputs() {
    let mut out = Bindings::default();
    bind_carrier_penalty(&mut out, 0, Ok((99_999, 3, 2)));
    bind_carrier_penalty(&mut out, 1, Ok((ONE, 3, 2)));
    assert!(!out.values["header_container/carrier_penalty_attacker"].hidden);
    assert!(!out.values["header_container/carrier_penalty_defender"].hidden);
    assert_eq!(
        out.values["header_container/carrier_penalty_attacker"].opacity,
        Some(1.0)
    );
    assert_eq!(
        out.values["header_container/carrier_penalty_defender"].opacity,
        Some(0.5)
    );
    assert_eq!(out.naval_penalties[0], Some(Ok((99_999, 3, 2))));
    bind_carrier_penalty(&mut out, 0, Err("missing model".into()));
    assert!(out.values["header_container/carrier_penalty_attacker"].hidden);
    assert_eq!(
        out.values["header_container/carrier_penalty_attacker"].opacity,
        None
    );
}
#[test]
fn opposing_silhouette_rounds_each_forward_group() {
    let defines = content_reader_naval_combat_defines::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [10, 5, 0, 0, 0, 0],
        screen_cap_reduction_factor: 2_000,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing: Ok(Default::default()),
        positioning_impacts: [0; 3],
    };
    let capital = |group_index| NavalCombatShipRow {
        side: 1,
        group_index,
        member_source: 0,
        state: 2,
        detection_state: 1,
        live_ship: 1,
        formation_flags: 1 << 6,
        ..Default::default()
    };
    let screen = |group_index| NavalCombatShipRow {
        side: 1,
        group_index,
        member_source: 0,
        state: 2,
        detection_state: 1,
        live_ship: 1,
        formation_flags: 1 << 8,
        ..Default::default()
    };
    let rows = [capital(0), screen(0), capital(1), screen(1)];
    let members: Vec<_> = rows.iter().collect();
    let models: [ShipModelRow; 0] = [];
    let models = Keyed::new(&models, |row| row.ship).unwrap();
    assert_eq!(opposing_silhouette(&members, models, 1, &defines), Ok(30));
}
#[test]
fn pride_bonus_follows_the_pride_ship_task_force_and_country_alias() {
    let row = NavalCombatShipRow {
        owner: 7,
        weapon_context: NavalShipWeaponContext {
            task_force: 102,
            ..Default::default()
        },
        ..Default::default()
    };
    let countries = [CountryCounterInputRow {
        forward_visibility: replay_data::rows::Recorded::NotRecorded,
        country: 8,
        pride_of_the_fleet: 29,
        ..Default::default()
    }];
    let ships = [ShipRow {
        id: 29,
        task_force: 101,
        ..Default::default()
    }];
    let countries = Keyed::new(&countries, |row| u64::from(row.country)).unwrap();
    let ships = Keyed::new(&ships, |row| row.id).unwrap();
    let tasks = [TaskForceRow {
        id: 102,
        logical_owner: 7,
        roster_owner: 8,
        ..Default::default()
    }];
    let tasks = Keyed::new(&tasks, |row| row.id).unwrap();
    let mut sides = [
        NavalCombatSideDetails::default(),
        NavalCombatSideDetails::default(),
    ];
    sides[0].summary.countries = vec![9];
    sides[0].units = vec![101, 102];
    let aliases = BTreeMap::from([(7, 70), (9, 70)]);
    assert_eq!(
        pride_bonus_applies(&row, &sides, countries, ships, tasks, &aliases),
        Ok(true)
    );
    sides[0].summary.countries = vec![8];
    assert!(pride_bonus_applies(&row, &sides, countries, ships, tasks, &aliases).is_err());
    sides[0].summary.countries.clear();
    assert_eq!(
        pride_bonus_applies(&row, &sides, countries, ships, tasks, &aliases),
        Ok(false)
    );
    let mut other_force = row.clone();
    other_force.weapon_context.task_force = 103;
    assert!(pride_bonus_applies(&other_force, &sides, countries, ships, tasks, &aliases).is_err());
    let mut other_roster_row = row;
    other_roster_row.owner = 99;
    other_roster_row.side = 1;
    sides[0].summary.countries = vec![9];
    assert_eq!(
        pride_bonus_applies(&other_roster_row, &sides, countries, ships, tasks, &aliases),
        Ok(true)
    );
}
#[test]
fn doctrine_weight_and_hq_maximum_are_not_flat_sums() {
    use replay_data::rows::NavalShipModifierTerm;
    let value = |raw| {
        vec![NavalShipModifierTerm {
            key: 7,
            raw,
            ..Default::default()
        }]
    };
    let context = NavalShipWeaponContext {
        sources: vec![
            NavalShipModifierSource {
                scope: 4,
                weights: [1, 2],
                terms: value(20_000),
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 12,
                index: 0,
                terms: value(3_000),
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 13,
                index: 0,
                terms: value(4_000),
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 12,
                index: 1,
                terms: value(5_000),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    assert_eq!(modifier(&context, 7), Some(17_000));
}

#[test]
fn equipment_fields_are_read_as_their_readers_read_them() {
    content_reader_script::test_game::enter();
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_convoy_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let path = root.join("common/units/equipment/convoys.txt");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        "equipments = { convoy_1 = { type = convoy lg_attack = 0.25
            attrition = { plains = 0.1 } can_convert_from = { a b }
            odd = { lg_attack = 5 } } }",
    )
    .unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (bound, misreads) = convoy_weapon_bound(&layers).unwrap();
    assert_eq!(bound, Ok([0, 25_000]));
    assert!(misreads.is_empty(), "{misreads:?}");
    fs::remove_dir_all(&root).unwrap();
}

fn bound_of(text: &str) -> ConvoyBound {
    use std::fs;
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_convoy_fields_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let path = root.join("common/units/equipment/convoys.txt");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, text).unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let bound = convoy_weapon_bound(&layers).unwrap();
    fs::remove_dir_all(&root).unwrap();
    bound
}

#[test]
fn equipment_fields_follow_their_value_readers() {
    content_reader_script::test_game::enter();
    let (bound, misreads) = bound_of(
        "upgrades = { a = { } b = { } }
         equipments = { convoy_1 = { TYPE = convoy lg_attack = 0.25
            upgrades = { a b } resources = { steel = 1 } default_modules = { s = empty }
            allow_mission_type = { x } forbid_mission_type = y allowed_types = { convoy }
            ai_type = convoy alias = { c }
            module_slots = { s = { required = yes allowed_module_categories = { m } gfx = g } }
            modifier_stat = { type = x value = 1 modifier = m }
            upgrade_window_override = { override = o upgrade = u }
            module_count_limit = { category = c count < 2 skipped = { lg_attack = 3 } }
            torpedo_attack = 0.5 } }",
    );
    assert_eq!(bound, Ok([0, 50_000]), "{misreads:?}");
    assert!(misreads.is_empty(), "{misreads:?}");
    let (bound, misreads) = bound_of(
        "equipments = { convoy_1 = { type = convoy
            module_slots = { s = { required = yes } S = { @v = 0.9 } t = { @w = 0.75 gfx = g } }
            lg_attack = @v hg_attack = @w } }",
    );
    assert_eq!(bound, Ok([0, 75_000]), "{misreads:?}");
    let (bound, misreads) = bound_of(
        "equipments = { base = { is_archetype = yes module_slots = { s = { } } }
            convoy_1 = { archetype = base type = convoy module_slots = inherit
            module_slots = { s = { @v = 0.9 gfx = g } t = { @w = 0.75 gfx = g } }
            lg_attack = @v hg_attack = @w } }",
    );
    assert_eq!(bound, Ok([0, 75_000]), "{misreads:?}");
}

#[test]
fn a_long_archetype_chain_links_in_load_order() {
    content_reader_script::test_game::enter();
    let mut text = String::from("equipments = { base = { is_archetype = yes lg_attack = 7 }");
    for link in 1..=40 {
        text.push_str(&format!(
            " link_{link} = {{ is_archetype = yes archetype = {} }}",
            if link == 1 {
                "BASE".to_owned()
            } else {
                format!("link_{}", link - 1)
            }
        ));
    }
    text.push_str(
        " late = { archetype = LINK_40 type = convoy torpedo_attack = 0.5 }
          early_stat = { lg_attack = 30 archetype = link_40 type = convoy }
          child = { parent = late type = convoy hg_attack = 0.75 }
          ahead = { archetype = later type = convoy }
          later = { is_archetype = yes type = convoy sub_attack = 90 } }",
    );
    let (bound, misreads) = bound_of(&text);
    assert_eq!(bound, Ok([0, 9_000_000]), "{misreads:?}");
    assert_eq!(misreads.len(), 1, "{misreads:?}");
    assert!(
        misreads[0].contains("archetype later is no equipment defined before it"),
        "{misreads:?}"
    );
    let (bound, _) = bound_of(&text.replace("sub_attack = 90", ""));
    assert_eq!(bound, Ok([0, 700_000]));
    let (bound, _) = bound_of(
        &text
            .replace("sub_attack = 90", "")
            .replace("lg_attack = 7", ""),
    );
    assert_eq!(bound, Ok([0, 75_000]));
}

#[test]
fn a_link_cycle_is_refused_by_name() {
    content_reader_script::test_game::enter();
    let (bound, _) = bound_of(
        "equipments = { a = { type = convoy lg_attack = 0.25 } b = { parent = a }
          A = { parent = b } }",
    );
    let reason = bound.unwrap_err();
    assert!(
        reason.contains("equipment a -> b -> a link back")
            && reason.contains("link")
            && reason.contains("never finish loading"),
        "{reason}"
    );
    let (bound, _) = bound_of(
        "equipments = { x = { is_archetype = yes } y = { is_archetype = yes archetype = x }
          x = { is_archetype = yes archetype = y } }",
    );
    assert!(bound.is_err_and(|reason| reason.contains("x -> y -> x")));
    let (bound, misreads) = bound_of(
        "equipments = { s = { is_archetype = yes archetype = s parent = s type = convoy } }",
    );
    assert_eq!(bound, Ok([0, 0]), "{misreads:?}");
}

#[test]
fn convoy_weapon_bound_accounts_for_reliability_scaling_and_module_multipliers() {
    content_reader_script::test_game::enter();
    let (bound, misreads) = bound_of(
        "equipment_modules = { gun = { add_stats = { lg_attack = 2 } multiply_stats = { lg_attack = 0.25 } } }
         equipments = {
            hull = { is_archetype = yes type = convoy reliability = 0.8
                scaled_by_reliability = { hg_attack = 0.5 }
                module_add_stats_multiplier = { lg_attack = 0.5 }
                module_multiply_stats_multiplier = { lg_attack = 1.0 } }
            hull_1 = { archetype = hull default_modules = { main = gun } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([0, 450_000]));
    let (bound, misreads) = bound_of(
        "equipments = {
            hull = { is_archetype = yes type = convoy reliability = 0.5
                scaled_by_reliability = { hg_attack = 0.5 sub_attack = -0.25 } }
            hull_1 = { archetype = hull }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-25_000, 50_000]));
    let (bound, _) = bound_of(
        "equipments = {
            modular_convoy = { type = convoy
                module_slots = { main = { } }
                module_add_stats_multiplier = { lg_attack = 0.5 } }
         }",
    );
    assert!(
        bound.is_err_and(|why| why.contains("modular_convoy")),
        "modular convoy with module stat multipliers must be refused by name"
    );
    let (bound, _) = bound_of(
        "equipment_modules = { neg_gun = { category = ship_gun add_stats = { lg_attack = -2 } } }
         equipments = {
            optional_convoy = { type = convoy lg_attack = 2
                module_slots = { main = { required = no allowed_module_categories = { ship_gun } } }
                default_modules = { main = neg_gun } }
         }",
    );
    let refused = bound.unwrap_err();
    assert!(refused.contains("optional_convoy"), "{refused}");
    assert_eq!(bounded_total(8_190_000, 0, Err(&refused)), Some(81));
    assert_eq!(bounded_total(8_190_000, 1, Err(&refused)), None);
    let (bound, misreads) = bound_of(
        "equipment_modules = { neg_gun = { add_stats = { lg_attack = -2 } } }
         equipments = {
            hull = { type = convoy lg_attack = 2 default_modules = { main = neg_gun } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([0, 200_000]));
    let (bound, _) = bound_of(
        "equipment_modules = {
            basic = { category = turret }
            advanced = { category = turret allowed_module_categories = { gun_slot = { heavy } } }
            light = { category = light }
            heavy = { category = heavy add_stats = { lg_attack = 2 } }
         }
         equipments = {
            transitive_convoy = { type = convoy
                module_slots = {
                    turret_slot = { required = yes allowed_module_categories = { turret } }
                    gun_slot = { required = yes allowed_module_categories = { light } }
                }
                default_modules = { turret_slot = basic gun_slot = light } }
         }",
    );
    assert!(
        bound.is_err_and(|why| why.contains("transitive_convoy")),
        "convoy with transitively unlocked weapon module must be refused by name"
    );
    let (bound, _) = bound_of(
        "upgrades = { convoy_gun = { lg_attack = 1 } }
         equipments = {
            upgraded_convoy = { type = convoy upgrades = { convoy_gun } }
         }",
    );
    assert!(
        bound.is_err_and(|why| why.contains("upgraded_convoy")),
        "slotless convoy with weapon-modifying upgrade must be refused by name"
    );
    let (bound, _) = bound_of(
        "equipment_modules = { tuning = { add_stats = { reliability = -0.75 } } }
         equipments = {
            wrapping_convoy = { type = convoy reliability = 1.0
                scaled_by_reliability = { lg_attack = 1844674407.37095 }
                default_modules = { engine = tuning } }
         }",
    );
    assert!(
        bound.is_err_and(|why| why.contains("wrapping_convoy")),
        "convoy whose scaled_by_reliability can wrap fixed-point arithmetic must be refused by name"
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = { tiny = { add_stats = { lg_attack = 0.01 } } }
         equipments = {
            nine_defaults = { type = convoy
                default_modules = {
                    s1 = tiny s2 = tiny s3 = tiny s4 = tiny s5 = tiny
                    s6 = tiny s7 = tiny s8 = tiny s9 = tiny
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([0, 9_000]));
    assert_eq!(bounded_total(0, 1, Ok([0, 9_000])), Some(0));
    let (bound, misreads) = bound_of(
        "equipment_modules = { cancel = { add_stats = { lg_attack = 1 } multiply_stats = { lg_attack = -1.0 } } }
         equipments = {
            correlated_convoy = { type = convoy default_modules = { main = cancel } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([0, 0]));
    assert_eq!(bounded_total(0, 1, Ok([0, 0])), Some(0));
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            cancel = { add_stats = { lg_attack = 1 } multiply_stats = { lg_attack = -1.0 } }
            zero_avg = { add_average_stats = { lg_attack = 0 } }
         }
         equipments = {
            thirteen_defaults = { type = convoy
                default_modules = {
                    main = cancel
                    z1 = zero_avg z2 = zero_avg z3 = zero_avg z4 = zero_avg
                    z5 = zero_avg z6 = zero_avg z7 = zero_avg z8 = zero_avg
                    z9 = zero_avg z10 = zero_avg z11 = zero_avg z12 = zero_avg
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([0, 0]));
    assert_eq!(bounded_total(0, 1, Ok([0, 0])), Some(0));
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            lead = { add_stats = { lg_attack = 1 } add_average_stats = { lg_attack = -0.1 } }
            avg_only = { add_average_stats = { lg_attack = -0.1 } }
         }
         equipments = {
            thirteen_avg_defaults = { type = convoy
                default_modules = {
                    main = lead
                    a1 = avg_only a2 = avg_only a3 = avg_only a4 = avg_only
                    a5 = avg_only a6 = avg_only a7 = avg_only a8 = avg_only
                    a9 = avg_only a10 = avg_only a11 = avg_only a12 = avg_only
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-10_000, 90_000]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            lead = { add_stats = { lg_attack = 1 } add_average_stats = { lg_attack = -0.1 } }
            d1 = { add_average_stats = { lg_attack = -0.1 } }
            d2 = { add_average_stats = { lg_attack = -0.11 } }
            d3 = { add_average_stats = { lg_attack = -0.12 } }
            d4 = { add_average_stats = { lg_attack = -0.13 } }
            d5 = { add_average_stats = { lg_attack = -0.14 } }
            d6 = { add_average_stats = { lg_attack = -0.15 } }
            d7 = { add_average_stats = { lg_attack = -0.16 } }
            d8 = { add_average_stats = { lg_attack = -0.17 } }
            d9 = { add_average_stats = { lg_attack = -0.18 } }
            d10 = { add_average_stats = { lg_attack = -0.19 } }
            d11 = { add_average_stats = { lg_attack = -0.2 } }
            d12 = { add_average_stats = { lg_attack = -0.21 } }
            d13 = { add_average_stats = { lg_attack = -0.22 } }
         }
         equipments = {
            fourteen_distinct_avg = { type = convoy
                default_modules = {
                    main = lead
                    s1 = d1 s2 = d2 s3 = d3 s4 = d4 s5 = d5 s6 = d6 s7 = d7
                    s8 = d8 s9 = d9 s10 = d10 s11 = d11 s12 = d12 s13 = d13
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-22_000, 90_000]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            combined = { add_stats = { lg_attack = 0.08 } add_average_stats = { lg_attack = -0.1 } }
         }
         equipments = {
            thirteen_combined = { type = convoy
                default_modules = {
                    s1 = combined s2 = combined s3 = combined s4 = combined
                    s5 = combined s6 = combined s7 = combined s8 = combined
                    s9 = combined s10 = combined s11 = combined s12 = combined
                    s13 = combined
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-2_000, 94_000]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            m1 = { add_stats = { lg_attack = 0.074 } add_average_stats = { lg_attack = -0.1 } }
            m2 = { add_stats = { lg_attack = 0.075 } add_average_stats = { lg_attack = -0.1 } }
            m3 = { add_stats = { lg_attack = 0.076 } add_average_stats = { lg_attack = -0.1 } }
            m4 = { add_stats = { lg_attack = 0.077 } add_average_stats = { lg_attack = -0.1 } }
            m5 = { add_stats = { lg_attack = 0.078 } add_average_stats = { lg_attack = -0.1 } }
            m6 = { add_stats = { lg_attack = 0.079 } add_average_stats = { lg_attack = -0.1 } }
            m7 = { add_stats = { lg_attack = 0.080 } add_average_stats = { lg_attack = -0.1 } }
            m8 = { add_stats = { lg_attack = 0.081 } add_average_stats = { lg_attack = -0.1 } }
            m9 = { add_stats = { lg_attack = 0.082 } add_average_stats = { lg_attack = -0.1 } }
            m10 = { add_stats = { lg_attack = 0.083 } add_average_stats = { lg_attack = -0.1 } }
            m11 = { add_stats = { lg_attack = 0.084 } add_average_stats = { lg_attack = -0.1 } }
            m12 = { add_stats = { lg_attack = 0.085 } add_average_stats = { lg_attack = -0.1 } }
            m13 = { add_stats = { lg_attack = 0.086 } add_average_stats = { lg_attack = -0.1 } }
         }
         equipments = {
            thirteen_distinct_combined = { type = convoy
                default_modules = {
                    s1 = m1 s2 = m2 s3 = m3 s4 = m4 s5 = m5 s6 = m6 s7 = m7
                    s8 = m8 s9 = m9 s10 = m10 s11 = m11 s12 = m12 s13 = m13
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-2_600, 94_000]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            m1 = { add_stats = { lg_attack = 0.074 } add_average_stats = { lg_attack = -0.1 } }
            m2 = { add_stats = { lg_attack = 0.075 } add_average_stats = { lg_attack = -0.1 } }
            m3 = { add_stats = { lg_attack = 0.076 } add_average_stats = { lg_attack = -0.1 } }
            m4 = { add_stats = { lg_attack = 0.077 } add_average_stats = { lg_attack = -0.1 } }
            m5 = { add_stats = { lg_attack = 0.078 } add_average_stats = { lg_attack = -0.1 } }
            m6 = { add_stats = { lg_attack = 0.079 } add_average_stats = { lg_attack = -0.1 } }
            m7 = { add_stats = { lg_attack = 0.080 } add_average_stats = { lg_attack = -0.1 } }
            m8 = { add_stats = { lg_attack = 0.081 } add_average_stats = { lg_attack = -0.1 } }
            m9 = { add_stats = { lg_attack = 0.082 } add_average_stats = { lg_attack = -0.1 } }
            m10 = { add_stats = { lg_attack = 0.083 } add_average_stats = { lg_attack = -0.1 } }
            m11 = { add_stats = { lg_attack = 0.084 } add_average_stats = { lg_attack = -0.1 } }
            m12 = { add_stats = { lg_attack = 0.085 } add_average_stats = { lg_attack = -0.1 } }
            m13 = { add_stats = { lg_attack = 0.086 } add_average_stats = { lg_attack = 0 } }
         }
         equipments = {
            thirteen_distinct_with_zero_avg = { type = convoy
                default_modules = {
                    s1 = m1 s2 = m2 s3 = m3 s4 = m4 s5 = m5 s6 = m6 s7 = m7
                    s8 = m8 s9 = m9 s10 = m10 s11 = m11 s12 = m12 s13 = m13
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-2_600, 94_770]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            m1 = { add_stats = { lg_attack = 0.074 } add_average_stats = { lg_attack = 0.046 } }
            m2 = { add_stats = { lg_attack = 0.075 } add_average_stats = { lg_attack = 0.046 } }
            m3 = { add_stats = { lg_attack = 0.076 } add_average_stats = { lg_attack = 0.046 } }
            m4 = { add_stats = { lg_attack = 0.077 } add_average_stats = { lg_attack = 0.046 } }
            m5 = { add_stats = { lg_attack = 0.078 } add_average_stats = { lg_attack = 0.046 } }
            m6 = { add_stats = { lg_attack = 0.079 } add_average_stats = { lg_attack = 0.046 } }
            m7 = { add_stats = { lg_attack = 0.080 } add_average_stats = { lg_attack = 0.04599 } }
            m8 = { add_stats = { lg_attack = 0.081 } add_average_stats = { lg_attack = 0.04599 } }
            m9 = { add_stats = { lg_attack = 0.082 } add_average_stats = { lg_attack = 0.04599 } }
            m10 = { add_stats = { lg_attack = 0.083 } add_average_stats = { lg_attack = 0.04599 } }
            m11 = { add_stats = { lg_attack = 0.084 } add_average_stats = { lg_attack = 0.04599 } }
            m12 = { add_stats = { lg_attack = 0.085 } add_average_stats = { lg_attack = 0.04599 } }
            m13 = { add_stats = { lg_attack = 0.001 } add_average_stats = { lg_attack = -0.6 } }
         }
         equipments = {
            thirteen_mixed_sign_avg = { type = convoy
                default_modules = {
                    s1 = m1 s2 = m2 s3 = m3 s4 = m4 s5 = m5 s6 = m6 s7 = m7
                    s8 = m8 s9 = m9 s10 = m10 s11 = m11 s12 = m12 s13 = m13
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-59_900, 99_999]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            a1 = { add_stats = { lg_attack = 0.08327 } add_average_stats = { lg_attack = 0.00004 } }
            a2 = { add_stats = { lg_attack = 0.08328 } add_average_stats = { lg_attack = 0.00004 } }
            a3 = { add_stats = { lg_attack = 0.08329 } add_average_stats = { lg_attack = 0.00004 } }
            a4 = { add_stats = { lg_attack = 0.08330 } add_average_stats = { lg_attack = 0.00004 } }
            a5 = { add_stats = { lg_attack = 0.08331 } add_average_stats = { lg_attack = 0.00004 } }
            a6 = { add_stats = { lg_attack = 0.08332 } add_average_stats = { lg_attack = 0.00004 } }
            a7 = { add_stats = { lg_attack = 0.08334 } add_average_stats = { lg_attack = 0.00003 } }
            a8 = { add_stats = { lg_attack = 0.08335 } add_average_stats = { lg_attack = 0.00003 } }
            a9 = { add_stats = { lg_attack = 0.08336 } add_average_stats = { lg_attack = 0.00003 } }
            a10 = { add_stats = { lg_attack = 0.08337 } add_average_stats = { lg_attack = 0.00003 } }
            a11 = { add_stats = { lg_attack = 0.08338 } add_average_stats = { lg_attack = 0.00003 } }
            a12 = { add_stats = { lg_attack = 0.08339 } add_average_stats = { lg_attack = 0.00003 } }
            a13 = { add_stats = { lg_attack = 0.08333 } add_average_stats = { lg_attack = -1.08332 } }
         }
         equipments = {
            thirteen_mixed_sign_high_add = { type = convoy
                default_modules = {
                    s1 = a1 s2 = a2 s3 = a3 s4 = a4 s5 = a5 s6 = a6 s7 = a7
                    s8 = a8 s9 = a9 s10 = a10 s11 = a11 s12 = a12 s13 = a13
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-99_999, 99_999]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            a1 = { add_stats = { lg_attack = 0.00327 } add_average_stats = { lg_attack = 0.00004 } }
            a2 = { add_stats = { lg_attack = 0.08328 } add_average_stats = { lg_attack = 0.00004 } }
            a3 = { add_stats = { lg_attack = 0.08329 } add_average_stats = { lg_attack = 0.00004 } }
            a4 = { add_stats = { lg_attack = 0.08330 } add_average_stats = { lg_attack = 0.00004 } }
            a5 = { add_stats = { lg_attack = 0.08331 } add_average_stats = { lg_attack = 0.00004 } }
            a6 = { add_stats = { lg_attack = 0.08332 } add_average_stats = { lg_attack = 0.00004 } }
            a7 = { add_stats = { lg_attack = 0.08334 } add_average_stats = { lg_attack = 0.00003 } }
            a8 = { add_stats = { lg_attack = 0.08335 } add_average_stats = { lg_attack = 0.00003 } }
            a9 = { add_stats = { lg_attack = 0.08336 } add_average_stats = { lg_attack = 0.00003 } }
            a10 = { add_stats = { lg_attack = 0.08337 } add_average_stats = { lg_attack = 0.00003 } }
            a11 = { add_stats = { lg_attack = 0.08338 } add_average_stats = { lg_attack = 0.00003 } }
            a12 = { add_stats = { lg_attack = 0.16339 } add_average_stats = { lg_attack = 0.00003 } }
            a13 = { add_stats = { lg_attack = 0.08333 } add_average_stats = { lg_attack = -1.08332 } }
         }
         equipments = {
            thirteen_integrality_gap_upper = { type = convoy
                default_modules = {
                    s1 = a1 s2 = a2 s3 = a3 s4 = a4 s5 = a5 s6 = a6 s7 = a7
                    s8 = a8 s9 = a9 s10 = a10 s11 = a11 s12 = a12 s13 = a13
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-99_999, 99_999]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
    let (bound, misreads) = bound_of(
        "equipment_modules = {
            b1 = { add_stats = { lg_attack = -0.00327 } add_average_stats = { lg_attack = -0.00004 } }
            b2 = { add_stats = { lg_attack = -0.08328 } add_average_stats = { lg_attack = -0.00004 } }
            b3 = { add_stats = { lg_attack = -0.08329 } add_average_stats = { lg_attack = -0.00004 } }
            b4 = { add_stats = { lg_attack = -0.08330 } add_average_stats = { lg_attack = -0.00004 } }
            b5 = { add_stats = { lg_attack = -0.08331 } add_average_stats = { lg_attack = -0.00004 } }
            b6 = { add_stats = { lg_attack = -0.08332 } add_average_stats = { lg_attack = -0.00004 } }
            b7 = { add_stats = { lg_attack = -0.08334 } add_average_stats = { lg_attack = -0.00003 } }
            b8 = { add_stats = { lg_attack = -0.08335 } add_average_stats = { lg_attack = -0.00003 } }
            b9 = { add_stats = { lg_attack = -0.08336 } add_average_stats = { lg_attack = -0.00003 } }
            b10 = { add_stats = { lg_attack = -0.08337 } add_average_stats = { lg_attack = -0.00003 } }
            b11 = { add_stats = { lg_attack = -0.08338 } add_average_stats = { lg_attack = -0.00003 } }
            b12 = { add_stats = { lg_attack = -0.16339 } add_average_stats = { lg_attack = -0.00003 } }
            b13 = { add_stats = { lg_attack = -0.08333 } add_average_stats = { lg_attack = 1.08332 } }
         }
         equipments = {
            thirteen_integrality_gap_lower = { type = convoy
                default_modules = {
                    s1 = b1 s2 = b2 s3 = b3 s4 = b4 s5 = b5 s6 = b6 s7 = b7
                    s8 = b8 s9 = b9 s10 = b10 s11 = b11 s12 = b12 s13 = b13
                } }
         }",
    );
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(bound, Ok([-99_999, 99_999]));
    assert_eq!(
        bounded_total(0, 1, bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
}

#[test]
fn convoy_weapon_bound_accounts_for_recorded_variant_bonuses_and_global_upgrades() {
    use replay_data::rows::{
        LexicalTokenRow, Recorded, TableRows, VariantBonusSnapshotRow, VariantBonusSource,
        VariantDefinitionRow, VariantDesignState, VariantStatEntry, VariantUpgradeRow,
        variant_bonus_snapshot_id,
    };
    use std::fs;
    use viewer_foundation::test_support::{test_frame, test_put as put};

    content_reader_script::test_game::enter();
    let lg_attack = content_reader_script::equipment_stats::stat_index(b"lg_attack").unwrap() as u32;
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_convoy_variants_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let path = root.join("common/units/equipment/convoys.txt");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        "upgrades = {
            global_gun = { lg_attack = 1.0 }
            avg_first = { add_stats = { lg_attack = 2 } add_average_stats = { lg_attack = -2 } }
            avg_zero = { add_average_stats = { lg_attack = -2 } }
         }
         equipments = {
            convoy_1 = { type = convoy lg_attack = 0.6 }
            vanilla_convoy = { type = convoy }
         }",
    )
    .unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let database = content_reader_key_names::equipment::Database::load(&layers).unwrap();
    fs::remove_dir_all(&root).unwrap();
    let (static_bound, misreads) = convoy_weapon_bound_from_database(&database);
    assert!(misreads.is_empty(), "{misreads:?}");
    assert_eq!(static_bound, Ok([0, 60_000]));

    let base_frame = |design: Recorded<VariantDesignState>,
                      upgrades: Vec<VariantUpgradeRow>,
                      snapshots: Vec<VariantBonusSnapshotRow>| {
        let mut frame = test_frame(1);
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 10,
                    literal: b"convoy_1".to_vec(),
                },
                LexicalTokenRow {
                    token: 11,
                    literal: b"lg_attack".to_vec(),
                },
                LexicalTokenRow {
                    token: 12,
                    literal: b"global_gun".to_vec(),
                },
                LexicalTokenRow {
                    token: 13,
                    literal: b"build_cost_ic".to_vec(),
                },
                LexicalTokenRow {
                    token: 14,
                    literal: b"vanilla_convoy".to_vec(),
                },
                LexicalTokenRow {
                    token: 15,
                    literal: b"avg_first".to_vec(),
                },
                LexicalTokenRow {
                    token: 16,
                    literal: b"avg_zero".to_vec(),
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: 70,
                equipment_token: 10,
                design,
                ..Default::default()
            }]),
        );
        put(&mut frame, TableRows::VariantUpgrades(upgrades));
        put(&mut frame, TableRows::VariantModules(Vec::new()));
        put(&mut frame, TableRows::VariantBonusSnapshots(snapshots));
        frame
    };

    let mio_frame = base_frame(
        Recorded::Value(VariantDesignState {
            design_team_bonus: vec![VariantStatEntry {
                index: 11,
                value: 100_000,
            }],
            ..Default::default()
        }),
        Vec::new(),
        Vec::new(),
    );
    let mio_bound = convoy_weapon_bound_for_frame(
        static_bound.as_ref().copied().map_err(String::as_str),
        Some(&database),
        &mio_frame,
    )
    .unwrap();
    assert_eq!(mio_bound, Ok([0, 120_000]));
    assert_eq!(
        bounded_total(0, 1, mio_bound.as_ref().copied().map_err(String::as_str)),
        None
    );

    let bonus = VariantBonusSource {
        stats: vec![VariantStatEntry {
            index: lg_attack,
            value: 100_000,
        }],
        ..Default::default()
    };
    let bonus_id = variant_bonus_snapshot_id(&bonus);
    let source_frame = base_frame(
        Recorded::Value(VariantDesignState {
            bonus_sources: vec![bonus_id],
            ..Default::default()
        }),
        Vec::new(),
        vec![VariantBonusSnapshotRow {
            id: bonus_id,
            source: bonus,
        }],
    );
    let source_bound = convoy_weapon_bound_for_frame(
        static_bound.as_ref().copied().map_err(String::as_str),
        Some(&database),
        &source_frame,
    )
    .unwrap();
    assert_eq!(source_bound, Ok([0, 120_000]));

    let unresolved_frame = base_frame(
        Recorded::Value(VariantDesignState {
            named_bonuses: vec![9],
            unresolved_stats: (0..content_reader_script::equipment_stats::STAT_COUNT as u32)
                .filter(|index| {
                    ![0x2e, 0x2f, 0x30, 0x31, 0x3e, 0x41, 0x43, 0x44, 0x45, 0x4d].contains(index)
                })
                .map(|index| VariantStatEntry {
                    index,
                    value: if index == lg_attack { 120_000 } else { 0 },
                })
                .collect(),
            ..Default::default()
        }),
        Vec::new(),
        Vec::new(),
    );
    let unresolved_bound = convoy_weapon_bound_for_frame(
        static_bound.as_ref().copied().map_err(String::as_str),
        Some(&database),
        &unresolved_frame,
    )
    .unwrap();
    assert_eq!(unresolved_bound, Ok([0, 120_000]));

    let upgrade_frame = base_frame(
        Recorded::Value(VariantDesignState::default()),
        vec![VariantUpgradeRow {
            variant: 70,
            source_ordinal: 0,
            definition_token: 12,
            level: 1,
        }],
        Vec::new(),
    );
    let upgrade_bound = convoy_weapon_bound_for_frame(
        static_bound.as_ref().copied().map_err(String::as_str),
        Some(&database),
        &upgrade_frame,
    )
    .unwrap();
    assert_eq!(upgrade_bound, Ok([0, 120_000]));

    let unrecorded_frame = base_frame(Recorded::NotRecorded, Vec::new(), Vec::new());
    assert!(
        convoy_weapon_bound_for_frame(
            static_bound.as_ref().copied().map_err(String::as_str),
            Some(&database),
            &unrecorded_frame,
        )
        .unwrap()
        .is_err()
    );

    let mut non_weapon_frame = base_frame(
        Recorded::Value(VariantDesignState {
            design_team_bonus: vec![VariantStatEntry {
                index: 13,
                value: -10_000,
            }],
            ..Default::default()
        }),
        Vec::new(),
        Vec::new(),
    );
    put(
        &mut non_weapon_frame,
        TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 71,
            equipment_token: 14,
            design: Recorded::Value(VariantDesignState {
                design_team_bonus: vec![VariantStatEntry {
                    index: 13,
                    value: -10_000,
                }],
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    let non_weapon_bound = convoy_weapon_bound_for_frame(
        Ok([0, 0]),
        Some(&database),
        &non_weapon_frame,
    )
    .unwrap();
    assert_eq!(non_weapon_bound, Ok([0, 0]));
    assert_eq!(
        bounded_total(8_190_000, 1, non_weapon_bound.as_ref().copied().map_err(String::as_str)),
        Some(81)
    );

    let mut level_zero_frame = base_frame(
        Recorded::Value(VariantDesignState::default()),
        vec![
            VariantUpgradeRow {
                variant: 71,
                source_ordinal: 0,
                definition_token: 15,
                level: 1,
            },
            VariantUpgradeRow {
                variant: 71,
                source_ordinal: 1,
                definition_token: 16,
                level: 0,
            },
        ],
        Vec::new(),
    );
    put(
        &mut level_zero_frame,
        TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 71,
            equipment_token: 14,
            design: Recorded::Value(VariantDesignState::default()),
            ..Default::default()
        }]),
    );
    let level_zero_bound = convoy_weapon_bound_for_frame(
        Ok([0, 0]),
        Some(&database),
        &level_zero_frame,
    )
    .unwrap();
    assert_eq!(level_zero_bound, Ok([0, 100_000]));
    assert_eq!(
        bounded_total(0, 1, level_zero_bound.as_ref().copied().map_err(String::as_str)),
        None
    );

    let mut cached_unavailable_frame = base_frame(
        Recorded::Value(VariantDesignState::default()),
        Vec::new(),
        Vec::new(),
    );
    put(
        &mut cached_unavailable_frame,
        TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 72,
            equipment_token: 14,
            design: Recorded::Value(VariantDesignState {
                named_bonuses: vec![9],
                bonus_sources: vec![bonus_id],
                unresolved_stats: (0..content_reader_script::equipment_stats::STAT_COUNT as u32)
                    .filter(|index| {
                        ![0x2e, 0x2f, 0x30, 0x31, 0x3e, 0x41, 0x43, 0x44, 0x45, 0x4d]
                            .contains(index)
                    })
                    .map(|index| VariantStatEntry { index, value: 0 })
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    *cached_unavailable_frame.tables[replay_data::rows::TableId::VariantBonusSnapshots.index()] =
        replay_frame::frame::TableView::Unavailable {
            reason: "snapshots unavailable".into(),
        };
    *cached_unavailable_frame.tables[replay_data::rows::TableId::VariantUpgrades.index()] =
        replay_frame::frame::TableView::Unavailable {
            reason: "upgrades unavailable".into(),
        };
    *cached_unavailable_frame.tables[replay_data::rows::TableId::VariantModules.index()] =
        replay_frame::frame::TableView::Unavailable {
            reason: "modules unavailable".into(),
        };
    let cached_bound = convoy_weapon_bound_for_frame(
        Ok([0, 0]),
        Some(&database),
        &cached_unavailable_frame,
    )
    .unwrap();
    assert_eq!(cached_bound, Ok([0, 0]));
    assert_eq!(
        bounded_total(0, 1, cached_bound.as_ref().copied().map_err(String::as_str)),
        Some(0)
    );
}
