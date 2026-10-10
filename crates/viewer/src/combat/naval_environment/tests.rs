use super::*;

#[test]
fn ship_child_breakdown_uses_doctrine_weight_and_critical_count() {
    use replay::rows::NavalShipModifierTerm;
    let context = NavalShipWeaponContext {
        sources: vec![
            NavalShipModifierSource {
                scope: 0,
                terms: vec![NavalShipModifierTerm {
                    key: 7,
                    raw: 10_000,
                    ..Default::default()
                }],
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 4,
                index: 2,
                weights: [3, 4],
                dynamic_keys: [7, 0],
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 11,
                index: 5,
                weights: [2, 0],
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 12,
                index: 0,
                terms: vec![NavalShipModifierTerm {
                    key: 9,
                    raw: 4_000,
                    ..Default::default()
                }],
                ..Default::default()
            },
            NavalShipModifierSource {
                scope: 12,
                index: 1,
                terms: vec![NavalShipModifierTerm {
                    key: 9,
                    raw: 8_000,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    // Breakdown labels use the doctrine's 82.5% weight and two critical
    // occurrences as 200%.
    assert_eq!(breakdown_scale(&context, 4, 2), Some(82_500));
    assert_eq!(breakdown_scale(&context, 11, 5), Some(200_000));
    assert_eq!(breakdown_scale(&context, 6, 0), Some(100_000));
    assert_eq!(breakdown_scale(&context, 4, 9), None);
    assert_eq!(breakdown_visible(&context, 9, 12, 0), Some(false));
    assert_eq!(breakdown_visible(&context, 9, 12, 1), Some(true));
}

#[test]
fn night_picture_uses_viewer_pause_at_max_recorded_speed() {
    assert!(night_picture(3, false));
    assert!(!night_picture(4, false));
    assert!(night_picture(4, true));
}

#[test]
fn custom_sea_grid_keeps_active_template_order_and_signed_words() {
    let names = ["reef".into(), "storm".into(), "reef".into(), "ice".into()];
    let values = [1_i16, 0, -2, 4]
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<_>>();
    assert_eq!(
        active_custom_sea(&names, &values)
            .unwrap()
            .collect::<Vec<_>>(),
        ["reef", "reef", "ice"]
    );
    assert!(active_custom_sea(&names, &values[..values.len() - 2]).is_none());
}

#[test]
fn modifier_tooltip_weather_grid_keeps_native_order_and_hover_description() {
    use crate::combat::binding::NoIcons;
    use replay::rows::ModifierDefinitionRow;
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
    use crate::combat::binding::NoIcons;
    use replay::rows::{ModifierDefinitionRow, NavalShipModifierTerm};
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

/// A weather key is localised; an absent key shows its own bytes, so its
/// sprite is looked up and refused by that name when missing.
#[test]
fn a_weather_icon_is_localised_and_a_missing_key_is_its_bytes() {
    let localisation = crate::map::labels::tests::loaded(concat!(
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
/// The sprite precedes the first `:`, with a one-based frame after it
/// read as a signed decimal prefix; empty text sets nothing.
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
    // One convoy with weapon bound 0.25 shows whole-number total 0.
    assert_eq!(bounded_total(0, 1, Ok([0, 25_000])), Some(0));
    assert_eq!(bounded_total(8_190_000, 0, Ok([0, 25_000])), Some(81));
    assert_eq!(bounded_total(8_190_000, 1, Ok([0, 25_000])), None);
    assert_eq!(bounded_total(0, 4, Ok([0, 25_000])), None);
    assert_eq!(bounded_total(0, 4, Ok([25_000, 25_000])), Some(1));
    assert_eq!(bounded_total(0, 2, Ok([25_000, 25_000])), Some(0));
    // A negative convoy value can lower the whole number.
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
    // Sum fixed5 attack before whole-number formatting; per-ship truncation
    // and historical damage never enter this counter.
    let attack = weapon(&ship, 3, false).unwrap();
    assert_eq!(attack, 8_875_000);
    assert_eq!(bounded_total(add(attack, attack), 0, Ok([0, 0])), Some(177));
}

#[test]
fn carrier_penalty_uses_opponent_silhouette_and_actual_multiplier() {
    use replay::rows::{NavalCombatPlane, ShipCounterInputs};
    let defines = installed_content::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0, 0, 80],
        silhouette_planes: [10, 5, 16, 3, 4, 7],
        screen_cap_reduction_factor: 2_000,
        plane_stacking_penalty_effect: 500,
        sortie_stance_ratios: vec![ONE],
        base_carrier_sortie_efficiency: ONE,
        selected_sortie_initial_time: 0,
        piercing_thresholds: Vec::new(),
        piercing_critical_values: Vec::new(),
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
    // A larger opposing silhouette outranks the content's free allowance.
    let mut capital = opponent.clone();
    capital.ship = 2;
    capital.formation_flags = 1 << 6;
    let opposing: Vec<_> = std::iter::repeat_n(&capital, 9).collect();
    assert_eq!(opposing_silhouette(&opposing, models, 1, &defines), Ok(90));
    assert_eq!(
        carrier_penalty_values(&side, &opposing, models, 1, &defines).map(|value| value.0),
        Ok(ONE)
    );
    // Excess planes retain the half-opacity laurel when the authored penalty is zero.
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
    let defines = installed_content::naval_combat_defines::Defines {
        summary_fixed: [0; 14],
        summary_integer: [0; 3],
        silhouette_planes: [10, 5, 0, 0, 0, 0],
        screen_cap_reduction_factor: 2_000,
        plane_stacking_penalty_effect: 0,
        sortie_stance_ratios: vec![],
        base_carrier_sortie_efficiency: 0,
        selected_sortie_initial_time: 0,
        piercing_thresholds: Vec::new(),
        piercing_critical_values: Vec::new(),
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
        forward_visibility: replay::rows::Recorded::NotRecorded,
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
    use replay::rows::NavalShipModifierTerm;
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

/// Unknown fields go to the stat reader, then are reported and their blocks
/// are skipped. Their nested lg_attack never becomes the convoy's stat.
/// `attrition` and `can_convert_from` read their own blocks.
#[test]
fn equipment_fields_are_read_as_their_readers_read_them() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (bound, misreads) = convoy_weapon_bound(&layers).unwrap();
    assert_eq!(bound, Ok([0, 25_000]));
    assert!(misreads.is_empty(), "{misreads:?}");
    fs::remove_dir_all(&root).unwrap();
}

/// One equipment file through the loader, and its convoy bound.
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
    let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let bound = convoy_weapon_bound(&layers).unwrap();
    fs::remove_dir_all(&root).unwrap();
    bound
}

/// Fields retain their individual value-reading rules: `upgrades` reads to
/// the closing brace; type and mission lists require a block; resources and
/// default modules read single-value entries. Module slots, modifier stats and
/// count limits skip unknown blocks; upgrade overrides read a value and return.
#[test]
fn equipment_fields_follow_their_value_readers() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // Each field reader consumes exactly its own value, so every
    // field after them is still the convoy's: 0.25 and 0.5.
    let (bound, misreads) = bound_of(
        "equipments = { convoy_1 = { TYPE = convoy lg_attack = 0.25
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
    // A repeated slot name is case-insensitive and skips its block without
    // defining @v. A new slot reads nested fields and defines @w.
    let (bound, misreads) = bound_of(
        "equipments = { convoy_1 = { type = convoy
            module_slots = { s = { required = yes } S = { @v = 0.9 } t = { @w = 0.75 gfx = g } }
            lg_attack = @v hg_attack = @w } }",
    );
    assert_eq!(bound, Ok([0, 75_000]), "{misreads:?}");
    // After `module_slots = inherit` the slots the loaded source holds
    // decide a block: the archetype's `s` is skipped, the new `t` read.
    let (bound, misreads) = bound_of(
        "equipments = { base = { is_archetype = yes module_slots = { s = { } } }
            convoy_1 = { archetype = base type = convoy module_slots = inherit
            module_slots = { s = { @v = 0.9 gfx = g } t = { @w = 0.75 gfx = g } }
            lg_attack = @v hg_attack = @w } }",
    );
    assert_eq!(bound, Ok([0, 75_000]), "{misreads:?}");
}

/// No step limit: a chain of any length links as the file loop reads
/// it. `archetype` copies the stat block as it stands,
/// so later stats overwrite the copy; `parent` passes the parent's
/// archetype on while none is set, but no stats; the type is the own
/// one, else the archetype's, names found without case.
#[test]
fn a_long_archetype_chain_links_in_load_order() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // `base` is no convoy, and a type goes one step only, so only the
    // convoys below carry its 7, copied down 40 archetypes.
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
    // `ahead` names a definition not read yet: reported, unlinked.
    assert_eq!(misreads.len(), 1, "{misreads:?}");
    assert!(
        misreads[0].contains("archetype later is no equipment defined before it"),
        "{misreads:?}"
    );
    // `early_stat`'s 30 is overwritten by the copy, and `child` takes
    // no stats from its parent: 7 is the largest.
    let (bound, _) = bound_of(&text.replace("sub_attack = 90", ""));
    assert_eq!(bound, Ok([0, 700_000]));
    // Without `base`'s 7 nothing reaches the chain's end.
    let (bound, _) = bound_of(
        &text
            .replace("sub_attack = 90", "")
            .replace("lg_attack = 7", ""),
    );
    assert_eq!(bound, Ok([0, 75_000]));
}

/// The game follows parent, else archetype, with no cycle guard. A
/// repeated name, read again into the same definition, can close a
/// loop, which the game never leaves: refused, naming the loop.
#[test]
fn a_link_cycle_is_refused_by_name() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    // Archetype links close a loop the same way.
    let (bound, _) = bound_of(
        "equipments = { x = { is_archetype = yes } y = { is_archetype = yes archetype = x }
          x = { is_archetype = yes archetype = y } }",
    );
    assert!(bound.is_err_and(|reason| reason.contains("x -> y -> x")));
    // A self link ends the walk: no cycle.
    let (bound, misreads) = bound_of(
        "equipments = { s = { is_archetype = yes archetype = s parent = s type = convoy } }",
    );
    assert_eq!(bound, Ok([0, 0]), "{misreads:?}");
}
