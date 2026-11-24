use super::*;
use content_reader_key_names::equipment::Database;
use content_reader_layers::ResolvedLayers;
use std::fs;

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
    let mut values = [None; 10];
    values[VariantStatsRow::MAXIMUM_SPEED] = Some(3_500_000);
    values[VariantStatsRow::RELIABILITY] = Some(85_000);
    values[VariantStatsRow::FUEL_CONSUMPTION] = Some(800_000);
    let recorded = RecordedStats::Available(values);
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

    let unavailable = RecordedStats::Unavailable("table truncated".to_owned());
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
fn designer_classification_respects_dlc_masks_refuses_missing_features_and_falls_back_cleanly() {
    let db = test_database();
    let ship = db.named("ship_hull_light_2").unwrap();
    let tank = db.named("light_tank_chassis_1").unwrap();
    let plane = db.named("small_plane_airframe_1").unwrap();
    let car = db.named("armored_car_equipment_1").unwrap();

    let all_dlc = SHIP_DLC_BIT | TANK_DLC_BIT | PLANE_DLC_BIT;
    assert_eq!(
        classify_designer(&db, ship, Some(all_dlc), None).unwrap(),
        DesignerKind::Ship
    );
    assert_eq!(
        classify_designer(&db, tank, Some(all_dlc), None).unwrap(),
        DesignerKind::Tank
    );
    assert_eq!(
        classify_designer(&db, plane, Some(all_dlc), None).unwrap(),
        DesignerKind::Plane
    );
    assert_eq!(
        classify_designer(&db, car, Some(all_dlc), None).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Land)
    );

    assert_eq!(
        classify_designer(&db, ship, Some(0), None).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Naval)
    );
    assert_eq!(
        classify_designer(&db, tank, Some(0), None).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Land)
    );
    assert_eq!(
        classify_designer(&db, plane, Some(0), None).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Air)
    );

    let refused = classify_designer(&db, ship, None, Some("SimulationFeatures unavailable"))
        .unwrap_err();
    assert!(refused.contains("SimulationFeatures unavailable"));
    assert!(classify_designer(&db, tank, None, None).is_err());
    assert!(classify_designer(&db, plane, None, None).is_err());
    assert_eq!(
        classify_designer(&db, car, None, None).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Land)
    );
}

#[test]
fn roles_resources_read_hour_and_missing_inputs_never_guess() {
    content_reader_script::test_game::enter();
    let loc: Localisation = [
        ("medium_tank_destroyer_brigade", "Medium Tank Destroyer"),
        ("plane_designer_cas", "Close Air Support"),
        ("custom_ore", "Custom Ore"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();

    let td_mask = category_mask("armor").unwrap() | category_mask("anti_tank").unwrap();
    assert_eq!(
        resolve_tank_role_and_frame("medium_tank_chassis", td_mask, &loc),
        Some(("Medium Tank Destroyer".to_owned(), Some(1)))
    );
    assert_eq!(
        resolve_tank_role_and_frame("custom_mod_mech_chassis", category_mask("armor").unwrap(), &loc),
        None
    );

    let cas_mask = category_mask("cas").unwrap();
    assert_eq!(
        resolve_plane_role(cas_mask, &loc).as_deref(),
        Some("Close Air Support")
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
    let bill = vec![("custom_ore".to_owned(), 300_000)];
    assert_eq!(
        format_resources_text(Some(&bill), &empty_resources, &loc, &mut warnings),
        "3 Custom Ore"
    );
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("custom_ore"));
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
    };
    let design_state_with_cost = VariantDesignState {
        build_cost: 900_000,
        manpower: 250,
        auto_upgraded: 1,
        role_icon_index: 2,
        ..VariantDesignState::default()
    };

    let withheld_cost = extract_design_data(
        DesignerKind::Ship,
        "GER",
        "ship_hull_light_2",
        "ship_hull_light",
        ship,
        &db,
        &definition,
        Some(&design_state_with_cost),
        None,
        Some(&stats_row),
        RecordedStats::Available(stats_values),
        Err("VariantModules unavailable".to_owned()),
        Err("VariantUpgrades unavailable".to_owned()),
        &[],
        None,
    );
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

    let missing_design = extract_design_data(
        DesignerKind::Ship,
        "GER",
        "ship_hull_light_2",
        "ship_hull_light",
        ship,
        &db,
        &definition,
        None,
        Some("Not recorded".to_owned()),
        None,
        RecordedStats::MissingRow,
        Ok(&[]),
        Ok(&[]),
        &[],
        None,
    );
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
    let data = extract_design_data(
        DesignerKind::Ship,
        "GER",
        "ship_hull_light_2",
        "ship_hull_light",
        ship,
        &db,
        &VariantDefinitionRow::default(),
        None,
        None,
        None,
        RecordedStats::MissingRow,
        Ok(&[]),
        Ok(&[]),
        &[],
        None,
    );
    let mut plan = ComposedPlan::default();
    let err = compose_module_slots(
        &layout,
        root,
        slot_root,
        &resources,
        &Localisation::default(),
        &data,
        &db,
        &mut plan,
    )
    .unwrap_err();
    assert!(matches!(err, GameWindowError::Refused(_)));
    assert!(err.to_string().contains("exceeding"));
}
