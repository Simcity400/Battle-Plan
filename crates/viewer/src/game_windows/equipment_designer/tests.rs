use super::*;
use content_reader_key_names::equipment::Database;
use content_reader_layers::ResolvedLayers;
use content_reader_script::equipment_stats::category_mask;
use replay_data::rows::{
    CountryAppearanceRow, CountryLinkRow, CountryRow, TableRows, VariantStatEntry,
};
use replay_frame::frame::Metrics;
use std::fs;

fn localisation(pairs: &[(&str, &str)]) -> Localisation {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn no_residuals() -> BTreeMap<usize, i64> {
    BTreeMap::new()
}

fn inputs<'a>(
    kind: DesignerKind,
    equipment: &'a Equipment,
    db: &'a Database,
    definition: &'a VariantDefinitionRow,
) -> ExtractInputs<'a> {
    ExtractInputs {
        kind,
        tag: "AAA",
        equipment_key: "synthetic",
        archetype_key: "synthetic",
        equipment,
        equipment_db: db,
        definition,
        design_state: None,
        design_state_reason: None,
        stats_row: None,
        stats: RecordedStats::MissingRow {
            residuals: no_residuals(),
        },
        module_rows: Ok(&[]),
        upgrade_rows: Ok(&[]),
        lexical: &[],
        mio_lookup: None,
    }
}

fn frame_with(views: Vec<(TableId, TableView)>) -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "fixture".into(),
    });
    for (id, view) in views {
        tables[id.index()] = view;
    }
    DecodedFrame {
        branch: 1,
        hour: 0,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

fn test_database() -> Database {
    content_reader_script::test_game::enter();
    let dir = std::env::temp_dir().join(format!(
        "battleplan-equipment-designer-test-{}",
        uuid::Uuid::new_v4()
    ));
    let eq_dir = dir.join("common").join("units").join("equipment");
    fs::create_dir_all(&eq_dir).unwrap();
    fs::write(
        eq_dir.join("synthetic.txt"),
        br#"
        equipments = {
            ship_hull_light = {
                is_archetype = yes
                type = screen_ship
                module_slots = {
                    fixed_ship_battery_slot = { required = yes }
                }
            }
            ship_hull_light_2 = {
                archetype = ship_hull_light
                module_slots = inherit
            }
            light_tank_chassis = {
                is_archetype = yes
                type = armor
                module_slots = {
                    turret_type_slot = { required = yes }
                }
            }
            light_tank_chassis_1 = {
                archetype = light_tank_chassis
                module_slots = inherit
            }
            small_plane_airframe = {
                is_archetype = yes
                type = fighter
                module_slots = {
                    fixed_main_weapon_slot = { required = yes }
                }
            }
            small_plane_airframe_1 = {
                archetype = small_plane_airframe
                module_slots = inherit
            }
            armored_car_equipment = {
                is_archetype = yes
                type = armor
            }
            armored_car_equipment_1 = {
                archetype = armored_car_equipment
            }
        }
        "#,
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&dir, &[][..])]).unwrap();
    let db = Database::load(&layers).unwrap();
    let _ = fs::remove_dir_all(&dir);
    db
}

#[test]
fn module_and_slot_sprite_resolution_prefers_direct_then_gfx_then_fallback() {
    assert_eq!(
        resolve_fitted_module_sprite("ship_light_battery_2", "custom_gun", |name| {
            name == "GFX_EMI_ship_light_battery_2" || name == "GFX_EMI_custom_gun"
        }),
        "GFX_EMI_ship_light_battery_2"
    );
    assert_eq!(
        resolve_fitted_module_sprite("ship_light_battery_2", "custom_gun", |name| {
            name == "GFX_EMI_custom_gun"
        }),
        "GFX_EMI_custom_gun"
    );
    assert_eq!(
        resolve_fitted_module_sprite("ship_light_battery_2", "custom_gun", |name| {
            name == "GFX_SMI_ship_light_battery_2"
        }),
        "GFX_SMI_ship_light_battery_2"
    );
    assert_eq!(
        resolve_fitted_module_sprite("ship_light_battery_2", "GFX_mod_override", |name| {
            name == "GFX_mod_override"
        }),
        "GFX_mod_override"
    );
    assert_eq!(
        resolve_fitted_module_sprite("unknown_mod", "", |_| false),
        "GFX_EMI_unknown"
    );

    assert_eq!(
        resolve_empty_slot_sprite("fixed_ship_radar_slot", "radar_slot", |name| {
            name == "GFX_EMSI_fixed_ship_radar_slot"
        }),
        "GFX_EMSI_fixed_ship_radar_slot"
    );
    assert_eq!(
        resolve_empty_slot_sprite("custom_slot_1", "ship_torpedo_slot", |name| {
            name == "GFX_EMSI_ship_torpedo_slot"
        }),
        "GFX_EMSI_ship_torpedo_slot"
    );
    assert_eq!(
        resolve_empty_slot_sprite("custom_slot_1", "", |_| false),
        "GFX_EMSI_empty"
    );
    assert_eq!(locked_slot_sprite(), "GFX_EMSI_locked");
}

#[test]
fn stat_columns_distinguish_recorded_values_from_not_recorded_rows() {
    content_reader_script::test_game::enter();
    let mut values = [None; 10];
    values[VariantStatsRow::MAXIMUM_SPEED] = Some(3_500_000);
    values[VariantStatsRow::RELIABILITY] = Some(85_000);
    values[VariantStatsRow::FUEL_CONSUMPTION] = Some(800_000);
    let recorded = RecordedStats::Available {
        values,
        residuals: no_residuals(),
    };
    let ship = build_stat_columns(DesignerKind::Ship, &recorded, Some(300), None);
    assert_eq!(ship.base[0].value, None);
    assert_eq!(ship.base[1].value, None);
    assert!(ship.base[1].detail.is_some());
    assert_eq!(ship.base[4].value.as_deref(), Some("85.0%"));
    assert_eq!(ship.base[6].value.as_deref(), Some("300"));
    assert_eq!(ship.misc[0].value.as_deref(), Some("8.00"));
    assert!(ship.combat.iter().all(|line| line.value.is_none()));

    let tank = build_stat_columns(DesignerKind::Tank, &recorded, None, None);
    assert_eq!(tank.base[0].value.as_deref(), Some("35.0 km/h"));
    assert_eq!(tank.base[1].value.as_deref(), Some("85.0%"));

    let classic_naval = build_stat_columns(
        DesignerKind::Upgrade(UpgradeDomain::Naval),
        &recorded,
        None,
        None,
    );
    assert_eq!(classic_naval.base[0].label_key, "STAT_NAVY_MAXIMUM_SPEED");
    assert_eq!(classic_naval.base[0].value, None);
    assert!(
        classic_naval.base[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("Not recorded"))
    );

    let unavailable = RecordedStats::Unavailable {
        reason: "table truncated".to_owned(),
        residuals: no_residuals(),
    };
    let missing_all = build_stat_columns(DesignerKind::Plane, &unavailable, None, None);
    assert!(missing_all.base.iter().all(|line| line.value.is_none()));
    assert!(
        missing_all.base[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("table truncated"))
    );
    assert!(missing_all.combat.iter().all(|line| line.value.is_none()));
    assert!(missing_all.misc.iter().all(|line| line.value.is_none()));
}

#[test]
fn designer_classification_reads_the_designs_recorded_categories_and_refuses_an_undecidable_one_by_name() {
    let db = test_database();
    let ship = db.named("ship_hull_light_2").unwrap();
    let tank = db.named("light_tank_chassis_1").unwrap();
    let plane = db.named("small_plane_airframe_1").unwrap();
    let car = db.named("armored_car_equipment_1").unwrap();
    let screen = category_mask("screen_ship").unwrap();
    let armor = category_mask("armor").unwrap();
    let fighter = category_mask("fighter").unwrap();
    let convoy = category_mask("convoy").unwrap();

    let all_dlc = Ok(u32::MAX);
    assert_eq!(classify_designer(ship, screen, all_dlc).unwrap(), DesignerKind::Ship);
    assert_eq!(classify_designer(tank, armor, all_dlc).unwrap(), DesignerKind::Tank);
    assert_eq!(classify_designer(plane, fighter, all_dlc).unwrap(), DesignerKind::Plane);
    assert_eq!(
        classify_designer(car, armor, all_dlc).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Land)
    );
    assert_eq!(
        classify_designer(tank, fighter, all_dlc).unwrap(),
        DesignerKind::Plane,
        "the design's recorded categories decide, not its script type"
    );

    assert_eq!(
        classify_designer(ship, screen, Ok(0)).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Naval)
    );
    assert_eq!(
        classify_designer(tank, armor, Ok(0)).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Land)
    );
    assert_eq!(
        classify_designer(plane, fighter, Ok(0)).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Air)
    );

    let refused = classify_designer(ship, screen, Err("SimulationFeatures unavailable")).unwrap_err();
    assert!(refused.contains("SimulationFeatures unavailable"));
    assert!(classify_designer(tank, armor, Err("absent")).is_err());
    assert!(classify_designer(plane, fighter, Err("absent")).is_err());
    assert_eq!(
        classify_designer(car, armor, Err("absent")).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Land)
    );

    let unmapped = classify_designer(ship, convoy, all_dlc).unwrap_err();
    assert!(unmapped.contains("ship_hull_light_2"), "{unmapped}");
    assert!(unmapped.contains("convoy"), "{unmapped}");
    assert!(unmapped.contains("no modular designer"), "{unmapped}");
    let uncategorised = classify_designer(car, 0, all_dlc).unwrap_err();
    assert!(uncategorised.contains("armored_car_equipment_1"), "{uncategorised}");

    content_reader_script::test_game::with(|game| {
        game.auxiliary(|tables| {
            tables.category_names = Err(engine_executable::availability::Withheld::new(
                "equipment categories",
                "not decoded from this build",
            ));
        })
    });
    let withheld = classify_designer(tank, armor, all_dlc).unwrap_err();
    assert!(withheld.contains("does not hold the game's equipment categories"), "{withheld}");
    assert!(withheld.contains("not decoded from this build"), "{withheld}");
    content_reader_script::test_game::enter();
}

#[test]
fn a_withheld_category_table_refuses_the_designer_by_equipment_name_before_any_script_is_read() {
    let dir = std::env::temp_dir().join(format!(
        "battleplan-designer-withheld-{}",
        uuid::Uuid::new_v4()
    ));
    let eq_dir = dir.join("common").join("units").join("equipment");
    fs::create_dir_all(&eq_dir).unwrap();
    fs::write(
        eq_dir.join("synthetic.txt"),
        b"equipments = { synthetic_tank_chassis = { is_archetype = yes type = armor } }",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&dir, &[][..])]).unwrap();
    content_reader_script::test_game::with(|game| {
        game.auxiliary(|tables| {
            tables.category_names = Err(engine_executable::availability::Withheld::new(
                "equipment categories",
                "not decoded from this build",
            ));
        })
    });
    let refused = designer_database(&layers, "synthetic_tank_chassis").err();
    content_reader_script::test_game::enter();
    let _ = fs::remove_dir_all(&dir);
    let Some(GameWindowError::Refused(reason)) = refused else {
        panic!("a withheld category table must refuse the designer, got {refused:?}");
    };
    assert!(reason.contains("synthetic_tank_chassis"), "{reason}");
    assert!(reason.contains("equipment categories withheld: not decoded from this build"), "{reason}");
}

#[test]
fn loc_refs_expand_nested_tokens_and_preserve_unknown_tokens() {
    let loc: Localisation = [
        ("tank_engine_upgrade", "$ARMOR_ENGINE_LABEL$"),
        ("ARMOR_ENGINE_LABEL", "§YEngine§!"),
        ("tank_armor_upgrade", "$ARMOR_PLATE_LABEL$"),
        ("ARMOR_PLATE_LABEL", "Armor"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    assert_eq!(loc_or(&loc, "tank_engine_upgrade"), "Engine");
    assert_eq!(loc_or(&loc, "tank_armor_upgrade"), "Armor");
    assert_eq!(
        expand_loc_refs(&loc, "$UNKNOWN_TOKEN$ + $ARMOR_ENGINE_LABEL$", 4),
        "$UNKNOWN_TOKEN$ + Engine"
    );
}

#[test]
fn extract_design_data_preserves_missing_tables_and_withheld_cost() {
    let db = test_database();
    let ship = db.named("ship_hull_light_2").unwrap();
    let mut definition = VariantDefinitionRow::default();
    definition.id = 42;
    definition.design = Recorded::NotRecorded;

    let mut stats_values = [None; 10];
    stats_values[VariantStatsRow::RELIABILITY] = Some(80_000);
    let stats_row = VariantStatsRow {
        variant: 42,
        organisation: 0,
        values: stats_values,
        ..VariantStatsRow::default()
    };
    let design_state_with_cost = VariantDesignState {
        build_cost: 900_000,
        manpower: 250,
        auto_upgraded: 1,
        role_icon_index: 2,
        ..VariantDesignState::default()
    };

    let withheld_cost = extract_design_data(ExtractInputs {
        kind: DesignerKind::Ship,
        tag: "GER",
        equipment_key: "ship_hull_light_2",
        archetype_key: "ship_hull_light",
        equipment: ship,
        equipment_db: &db,
        definition: &definition,
        design_state: Some(&design_state_with_cost),
        design_state_reason: None,
        stats_row: Some(&stats_row),
        stats: RecordedStats::Available {
            values: stats_values,
            residuals: no_residuals(),
        },
        module_rows: Err("VariantModules unavailable".to_owned()),
        upgrade_rows: Err("VariantUpgrades unavailable".to_owned()),
        lexical: &[],
        mio_lookup: None,
    });
    assert_eq!(withheld_cost.build_cost, None);
    assert!(matches!(
        withheld_cost.fitted_modules,
        RecordedModules::Unavailable(_)
    ));
    assert!(matches!(
        withheld_cost.upgrades,
        RecordedUpgrades::Unavailable { .. }
    ));
    let resources = viewer_native_gui::test_support::gui::fixture();
    let slot_icon = slot_icon_for(
        "fixed_ship_battery_slot",
        "",
        &withheld_cost,
        &db,
        &resources,
    );
    assert!(matches!(
        slot_icon,
        SlotIcon::Unknown { ref sprite, .. } if sprite == "GFX_EMI_unknown"
    ));

    let missing_design = extract_design_data(ExtractInputs {
        kind: DesignerKind::Ship,
        tag: "GER",
        equipment_key: "ship_hull_light_2",
        archetype_key: "ship_hull_light",
        equipment: ship,
        equipment_db: &db,
        definition: &definition,
        design_state: None,
        design_state_reason: Some("Not recorded".to_owned()),
        stats_row: None,
        stats: RecordedStats::MissingRow {
            residuals: no_residuals(),
        },
        module_rows: Ok(&[]),
        upgrade_rows: Ok(&[]),
        lexical: &[],
        mio_lookup: None,
    });
    assert_eq!(missing_design.auto_upgraded, None);
    assert_eq!(missing_design.build_resources, None);
    assert_eq!(missing_design.niche_frame, None);
    assert_eq!(missing_design.manpower, None);
    assert_eq!(missing_design.build_cost, None);
}

#[test]
fn compose_module_slots_refuses_when_mod_slots_exceed_window_markers() {
    content_reader_script::test_game::enter();
    let db = test_database();
    let ship = db.named("ship_hull_light_2").unwrap();
    let gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "countryequipmentdesignerview"
                size = { width = 200 height = 200 }
                containerWindowType = {
                    name = "equipment_modules"
                    size = { width = 200 height = 200 }
                }
            }
            containerWindowType = {
                name = "equipment_designer_module_slot_entry"
                size = { width = 20 height = 20 }
            }
        }"#,
        [400, 300],
    );
    let root = gui.template("countryequipmentdesignerview").unwrap();
    let slot_root = gui
        .template("equipment_designer_module_slot_entry")
        .unwrap();
    let resources = viewer_native_gui::test_support::gui::fixture();
    let layout = layout_window(root, &resources, [400, 300], |_| {}).unwrap();
    let data = extract_design_data(ExtractInputs {
        kind: DesignerKind::Ship,
        tag: "GER",
        equipment_key: "ship_hull_light_2",
        archetype_key: "ship_hull_light",
        equipment: ship,
        equipment_db: &db,
        definition: &VariantDefinitionRow::default(),
        design_state: None,
        design_state_reason: None,
        stats_row: None,
        stats: RecordedStats::MissingRow {
            residuals: no_residuals(),
        },
        module_rows: Ok(&[]),
        upgrade_rows: Ok(&[]),
        lexical: &[],
        mio_lookup: None,
    });
    let mut plan = ComposedPlan::default();
    let err = compose_module_slots(
        &layout,
        root,
        slot_root,
        &resources,
        &Localisation::default(),
        (&data, &db),
        &mut plan,
    )
    .unwrap_err();
    assert!(matches!(err, GameWindowError::Refused(_)));
    assert!(err.to_string().contains("exceeding"));
}

#[test]
fn roles_resources_read_hour_and_missing_inputs_never_guess() {
    content_reader_script::test_game::enter();
    let loc = localisation(&[
        ("tank_designer_anti_tank", "Synthetic role AT"),
        ("plane_designer_cas", "Synthetic role CAS"),
        ("custom_ore", "Custom Ore"),
    ]);

    let td_mask = category_mask("armor").unwrap() | category_mask("anti_tank").unwrap();
    assert_eq!(
        resolve_tank_role_and_frame("medium_tank_chassis", td_mask, &loc),
        Some(("Synthetic role AT".to_owned(), None))
    );
    assert_eq!(
        resolve_tank_role_and_frame("custom_mod_mech_chassis", category_mask("armor").unwrap(), &loc),
        None
    );

    let cas_mask = category_mask("cas").unwrap();
    assert_eq!(
        resolve_plane_role(cas_mask, &loc).as_deref(),
        Some("Synthetic role CAS")
    );
    assert_eq!(resolve_plane_role(0, &loc), None);

    let (hour, mixed) = compute_read_hour(
        22,
        &[
            (TableId::VariantDefinitions, 22),
            (TableId::IndustrialOrganisations, 8),
        ],
    );
    assert_eq!(hour, 8);
    assert!(
        mixed
            .as_deref()
            .is_some_and(|text| text.contains("IndustrialOrganisations") && text.contains("hour 8"))
    );

    let (same_hour, none_mixed) = compute_read_hour(
        22,
        &[
            (TableId::VariantDefinitions, 22),
            (TableId::VariantStats, 22),
        ],
    );
    assert_eq!(same_hour, 22);
    assert_eq!(none_mixed, None);

    let empty_resources = ResourceDatabase::default();
    let mut warnings = Vec::new();
    assert_eq!(
        format_resources_text(None, &empty_resources, &loc, &mut warnings),
        "n/a"
    );
    let bill = vec![
        RecordedResource {
            key: Some("custom_ore".to_owned()),
            token: 1,
            amount: 300_000,
        },
        RecordedResource {
            key: None,
            token: 99,
            amount: 200_000,
        },
    ];
    assert_eq!(
        format_resources_text(Some(&bill), &empty_resources, &loc, &mut warnings),
        "3 Custom Ore 2 n/a"
    );
    assert_eq!(warnings.len(), 2);
    assert!(warnings[0].contains("custom_ore"));
    assert!(warnings[1].contains("99"));
}

#[test]
fn the_role_comes_from_the_variants_own_classification() {
    let db = test_database();
    let tank = db.named("light_tank_chassis_1").unwrap();
    let definition = VariantDefinitionRow {
        classifier_flags: 0b1010,
        ..VariantDefinitionRow::default()
    };
    let data = extract_design_data(inputs(DesignerKind::Tank, tank, &db, &definition));
    assert_eq!(data.category_mask, 0b1010);
    assert_ne!(db.category(tank), 0b1010);
}

#[test]
fn unresolved_modules_upgrades_and_manufacturers_stay_visible_as_unknown() {
    let db = test_database();
    let ship = db.named("ship_hull_light_2").unwrap();
    let definition = VariantDefinitionRow::default();
    let lexical = [LexicalTokenRow {
        token: 5,
        literal: b"fixed_ship_battery_slot".to_vec(),
    }];
    let modules = [VariantModuleRow {
        slot: 5,
        definition_token: 77,
        definition_active: 1,
        ..VariantModuleRow::default()
    }];
    let upgrades = [VariantUpgradeRow {
        definition_token: 88,
        level: 3,
        ..VariantUpgradeRow::default()
    }];
    let state = VariantDesignState {
        design_team: 12,
        design_team_traits: 0,
        ..VariantDesignState::default()
    };
    let mut fed = inputs(DesignerKind::Ship, ship, &db, &definition);
    fed.lexical = &lexical;
    fed.module_rows = Ok(&modules);
    fed.upgrade_rows = Ok(&upgrades);
    fed.design_state = Some(&state);
    fed.mio_lookup = Some(Err("IndustrialOrganisations is truncated".to_owned()));
    let data = extract_design_data(fed);

    let RecordedModules::Available { slots, .. } = &data.fitted_modules else {
        panic!("modules were recorded");
    };
    assert!(matches!(
        slots.get("fixed_ship_battery_slot"),
        Some(FittedSlot::Unresolved { token: 77, .. })
    ));
    let resources = viewer_native_gui::test_support::gui::fixture();
    assert!(matches!(
        slot_icon_for("fixed_ship_battery_slot", "", &data, &db, &resources),
        SlotIcon::Unknown { ref sprite, .. } if sprite == "GFX_EMI_unknown"
    ));

    let RecordedUpgrades::Available {
        unresolved_reason, ..
    } = &data.upgrades
    else {
        panic!("upgrades were recorded");
    };
    assert!(unresolved_reason.as_deref().is_some_and(|why| why.contains("88")));

    assert_eq!(
        data.mio,
        RecordedMio::Unresolved {
            id: 12,
            trait_count: Some(0),
            reason: "Not recorded: IndustrialOrganisations is unavailable at this hour (IndustrialOrganisations is truncated).".to_owned(),
        }
    );
}

#[test]
fn a_named_bonus_designs_recorded_residual_stats_are_shown() {
    content_reader_script::test_game::enter();
    let attack = stat_index(b"lg_attack").unwrap();
    let state = VariantDesignState {
        named_bonuses: vec![1],
        unresolved_stats: vec![VariantStatEntry {
            index: attack as u32,
            value: 2_850_000,
        }],
        ..VariantDesignState::default()
    };
    let mut warnings = Vec::new();
    let residuals = extract_unresolved_residuals(Some(&state), &mut warnings);
    assert_eq!(residuals.get(&attack), Some(&2_850_000));
    let stats = RecordedStats::Available {
        values: [None; 10],
        residuals,
    };
    let ship = build_stat_columns(DesignerKind::Ship, &stats, None, None);
    assert_eq!(ship.combat[0].value.as_deref(), Some("28.5"));

    let plain = VariantDesignState {
        unresolved_stats: state.unresolved_stats.clone(),
        ..VariantDesignState::default()
    };
    assert!(extract_unresolved_residuals(Some(&plain), &mut warnings).is_empty());
}

#[test]
fn a_truncated_table_answers_for_a_key_it_witnessed_and_refuses_one_it_did_not() {
    let rows = TableRows::VariantDefinitions(vec![VariantDefinitionRow {
        id: 5,
        ..VariantDefinitionRow::default()
    }]);
    let frame = frame_with(vec![(
        TableId::VariantDefinitions,
        TableView::Truncated { rows, missing: 3 },
    )]);
    let found =
        witnessed_single_key::<tables::VariantDefinitions, _>(&frame, 5_u64, |row| row.id).unwrap();
    assert_eq!(found.map(|row| row.id), Some(5));
    let refused =
        witnessed_single_key::<tables::VariantDefinitions, _>(&frame, 6_u64, |row| row.id).unwrap_err();
    assert!(refused.contains("truncated"));
}

#[test]
fn the_designs_country_is_joined_through_country_links_and_never_borrowed() {
    let frame = frame_with(vec![
        (
            TableId::Countries,
            TableView::Complete(TableRows::Countries(vec![
                CountryRow {
                    id: 3,
                    canonical_slot: 3,
                    tag: b"CCC".to_vec(),
                    ..CountryRow::default()
                },
                CountryRow {
                    id: 77,
                    canonical_slot: 77,
                    tag: b"EEE".to_vec(),
                    ..CountryRow::default()
                },
            ])),
        ),
        (
            TableId::CountryLinks,
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 3,
                    canonical_slot: 77,
                },
                CountryLinkRow {
                    id: 77,
                    canonical_slot: 3,
                },
            ])),
        ),
        (
            TableId::CountryAppearance,
            TableView::Complete(TableRows::CountryAppearance(vec![
                CountryAppearanceRow {
                    country: 3,
                    ..CountryAppearanceRow::default()
                },
                CountryAppearanceRow {
                    country: 77,
                    ..CountryAppearanceRow::default()
                },
            ])),
        ),
    ]);
    let created_by = |link| VariantDefinitionRow {
        creator_link: link,
        ..VariantDefinitionRow::default()
    };
    assert_eq!(resolve_variant_tag(&frame, &created_by(77)).0, "CCC");
    assert_eq!(resolve_variant_tag(&frame, &created_by(3)).0, "EEE");
    assert_eq!(resolve_variant_tag(&frame, &created_by(0)).0, "");
}

#[test]
fn a_mod_designer_with_upgrade_controls_reads_variant_upgrades() {
    content_reader_script::test_game::enter();
    let gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "plane_designer_view"
                size = { width = 200 height = 200 }
                containerWindowType = {
                    name = "equipment_modules"
                    size = { width = 200 height = 200 }
                    containerWindowType = { name = "equipment_upgrade_0" size = { width = 20 height = 20 } }
                }
            }
            containerWindowType = {
                name = "plain_plane_view"
                size = { width = 200 height = 200 }
                containerWindowType = { name = "equipment_modules" size = { width = 200 height = 200 } }
            }
        }"#,
        [400, 300],
    );
    assert!(reads_upgrades(DesignerKind::Plane, gui.template("plane_designer_view")));
    assert!(!reads_upgrades(DesignerKind::Plane, gui.template("plain_plane_view")));
    assert!(reads_upgrades(DesignerKind::Upgrade(UpgradeDomain::Air), None));
}

fn hidden(layout: &Layout, name: &str) -> bool {
    !layout.parts.iter().any(|part| part.path.ends_with(name))
}

#[test]
fn an_unknown_auto_upgrade_hides_the_box_and_shows_n_a_with_its_reason() {
    content_reader_script::test_game::enter();
    let db = test_database();
    let tank = db.named("light_tank_chassis_1").unwrap();
    let gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "tank_designer_view"
                size = { width = 300 height = 200 }
                buttonType = {
                    name = "auto_upgraded_checkbox_button"
                    position = { x = 40 y = 50 }
                    spriteType = "GFX_even"
                }
            }
            containerWindowType = {
                name = "stat_entry"
                size = { width = 60 height = 20 }
                instantTextBoxType = { name = "label" font = "Main_14" maxWidth = 40 maxHeight = 20 }
                instantTextBoxType = { name = "value" position = { x = 40 y = 0 } font = "Main_14" maxWidth = 20 maxHeight = 20 }
            }
        }"#,
        [400, 300],
    );
    let root = gui.template("tank_designer_view").unwrap();
    let stat_root = gui.template("stat_entry").unwrap();
    let resources = viewer_native_gui::test_support::gui::fixture();
    let loc = Localisation::default();
    let definition = VariantDefinitionRow::default();
    let state = VariantDesignState::default();
    let resource_db = ResourceDatabase::default();
    let lay_out = |data: &EquipmentDesignData| {
        let mut tooltips = BTreeMap::new();
        layout_window(root, &resources, [400, 300], |instance| {
            configure_main_window(
                root,
                &resources,
                &loc,
                instance,
                MainWindowContext {
                    data,
                    resource_db: &resource_db,
                    equipment_icon: &ResolvedEquipmentIcon::NoneExpected,
                    mio_sprite: None,
                },
                &mut tooltips,
            );
        })
        .unwrap()
    };

    let mut unknown_inputs = inputs(DesignerKind::Tank, tank, &db, &definition);
    unknown_inputs.design_state_reason = Some("Not recorded: synthetic reason.".to_owned());
    let unknown = extract_design_data(unknown_inputs);
    assert_eq!(unknown.auto_upgraded, None);
    let layout = lay_out(&unknown);
    assert!(hidden(&layout, "auto_upgraded_checkbox_button"));
    let mut plan = ComposedPlan::default();
    compose_unknown_auto_upgrade(&layout, root, stat_root, &resources, &loc, &unknown, &mut plan).unwrap();
    assert_eq!(plan.sub_windows.len(), 1);
    assert_eq!(plan.sub_windows[0].path_prefix, "auto_upgraded_missing/");
    let region = &plan.extra_regions[0];
    assert!(region.path.ends_with("auto_upgraded_checkbox_button"));
    assert_eq!(
        region.tooltip.as_ref().and_then(|tip| tip.body.as_deref()),
        Some("Not recorded: synthetic reason.")
    );

    let mut known_inputs = inputs(DesignerKind::Tank, tank, &db, &definition);
    known_inputs.design_state = Some(&state);
    let known = extract_design_data(known_inputs);
    assert_eq!(known.auto_upgraded, Some(false));
    let layout = lay_out(&known);
    assert!(!hidden(&layout, "auto_upgraded_checkbox_button"));
    let mut plan = ComposedPlan::default();
    compose_unknown_auto_upgrade(&layout, root, stat_root, &resources, &loc, &known, &mut plan).unwrap();
    assert!(plan.sub_windows.is_empty() && plan.extra_regions.is_empty());
}

#[test]
fn a_missing_or_refused_icon_hides_the_template_art_and_says_why() {
    content_reader_script::test_game::enter();
    let gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "designer"
                size = { width = 200 height = 200 }
                containerWindowType = {
                    name = "icon_selection_container"
                    size = { width = 100 height = 100 }
                    buttonType = { name = "btn_equipment_icon" spriteType = "GFX_even" }
                }
            }
        }"#,
        [400, 300],
    );
    let root = gui.template("designer").unwrap();
    let resources = viewer_native_gui::test_support::gui::fixture();
    for icon in [
        ResolvedEquipmentIcon::Missing {
            reason: "no layer holds it".to_owned(),
        },
        ResolvedEquipmentIcon::Refused {
            reason: "not a plain sprite".to_owned(),
            construct: "corneredTileSpriteType".to_owned(),
        },
    ] {
        let mut tooltips = BTreeMap::new();
        let layout = layout_window(root, &resources, [400, 300], |instance| {
            apply_equipment_icon(
                instance,
                root,
                "icon_selection_container/btn_equipment_icon",
                &icon,
                &resources,
                &mut tooltips,
            );
        })
        .unwrap();
        assert!(hidden(&layout, "btn_equipment_icon"), "{icon:?}");
        let parent = find_direct(root, "icon_selection_container").unwrap();
        assert!(tooltips.get(&parent.path).and_then(|tip| tip.body.as_deref()).is_some());
    }
}

#[test]
fn stat_rows_follow_the_declared_grid_show_a_missing_key_by_name_and_refuse_a_renamed_value() {
    content_reader_script::test_game::enter();
    let gui = native::read_at(
        br#"guiTypes = {
            containerWindowType = {
                name = "designer"
                size = { width = 400 height = 200 }
                gridBoxType = {
                    name = "stats_grid"
                    position = { x = 0 y = 0 }
                    size = { width = 300 height = 100 }
                    slotsize = { width = 150 height = 25 }
                    max_slots_horizontal = 2
                    add_horizontal = yes
                }
            }
            containerWindowType = {
                name = "stat_entry"
                size = { width = 100 height = 20 }
                instantTextBoxType = { name = "label" font = "Main_14" maxWidth = 60 maxHeight = 20 }
                instantTextBoxType = { name = "value" position = { x = 60 y = 0 } font = "Main_14" maxWidth = 40 maxHeight = 20 }
            }
            containerWindowType = {
                name = "renamed_entry"
                size = { width = 100 height = 20 }
                instantTextBoxType = { name = "label" font = "Main_14" maxWidth = 60 maxHeight = 20 }
                instantTextBoxType = { name = "amount" font = "Main_14" maxWidth = 40 maxHeight = 20 }
            }
        }"#,
        [400, 300],
    );
    let root = gui.template("designer").unwrap();
    let grid = find_direct(root, "stats_grid").unwrap();
    let resources = viewer_native_gui::test_support::gui::fixture();
    let layout = layout_window(root, &resources, [400, 300], |_| {}).unwrap();
    let loc = localisation(&[("STAT_SYNTHETIC_A", "Alpha")]);
    let lines = ["STAT_SYNTHETIC_A", "STAT_SYNTHETIC_B", "STAT_SYNTHETIC_C"].map(|key| StatLine {
        label_key: key,
        value: Some("1".to_owned()),
        detail: None,
    });
    let mut plan = ComposedPlan::default();
    compose_stat_grid(
        &layout,
        grid,
        gui.template("stat_entry").unwrap(),
        StatGridSpec {
            stat_window_name: "stat_entry",
            value_child: "value",
            lines: &lines,
        },
        &resources,
        &loc,
        &mut plan,
    )
    .unwrap();
    let origins: Vec<[f32; 2]> = plan
        .sub_windows
        .iter()
        .map(|sub| {
            let [x, y, ..] = sub.regions[0].rect;
            [x, y]
        })
        .collect();
    assert_eq!(origins[1][0] - origins[0][0], 150.0);
    assert_eq!(origins[2][1] - origins[0][1], 25.0);
    assert_eq!(origins[2][0], origins[0][0]);
    let missing_title = |index: usize| {
        plan.sub_windows[index]
            .tooltip_overrides
            .values()
            .next()
            .map(|tip| tip.title.clone())
    };
    assert_eq!(missing_title(0).as_deref(), Some("Alpha"));
    assert_eq!(missing_title(1).as_deref(), Some("STAT_SYNTHETIC_B"));
    assert!(plan.warnings.iter().any(|line| line.contains("STAT_SYNTHETIC_B")));

    let refused = compose_stat_grid(
        &layout,
        grid,
        gui.template("renamed_entry").unwrap(),
        StatGridSpec {
            stat_window_name: "renamed_entry",
            value_child: "value",
            lines: &lines,
        },
        &resources,
        &loc,
        &mut ComposedPlan::default(),
    )
    .unwrap_err();
    assert!(matches!(refused, GameWindowError::Refused(ref why) if why.contains("'value'")));
}

#[test]
fn the_cached_frame_changes_with_the_selected_icon() {
    let db = test_database();
    let ship = db.named("ship_hull_light_2").unwrap();
    let definition = VariantDefinitionRow::default();
    let data = extract_design_data(inputs(DesignerKind::Ship, ship, &db, &definition));
    let sprite = |name: &str| Icon::Sprite(name.to_owned());
    assert_ne!(
        fingerprint_frame(&data, &sprite("GFX_icon_democratic")),
        fingerprint_frame(&data, &sprite("GFX_icon_fascist"))
    );
}
