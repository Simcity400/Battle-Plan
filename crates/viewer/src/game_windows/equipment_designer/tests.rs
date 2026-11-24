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
    let ship = build_stat_columns(DesignerKind::Ship, Some(&values), Some(300));
    assert_eq!(ship.base[0].value, None);
    assert_eq!(ship.base[1].value, None);
    assert!(ship.base[1].detail.is_some());
    assert_eq!(ship.base[4].value.as_deref(), Some("85.0%"));
    assert_eq!(ship.base[6].value.as_deref(), Some("300"));
    assert_eq!(ship.misc[0].value.as_deref(), Some("8.00"));
    assert!(ship.combat.iter().all(|line| line.value.is_none()));

    let tank = build_stat_columns(DesignerKind::Tank, Some(&values), None);
    assert_eq!(tank.base[0].value.as_deref(), Some("35.0 km/h"));
    assert_eq!(tank.base[1].value.as_deref(), Some("85.0%"));

    let missing_all = build_stat_columns(DesignerKind::Plane, None, None);
    assert!(missing_all.base.iter().all(|line| line.value.is_none()));
    assert!(missing_all.combat.iter().all(|line| line.value.is_none()));
    assert!(missing_all.misc.iter().all(|line| line.value.is_none()));
}

#[test]
fn designer_classification_respects_dlc_masks_and_classic_upgrade_fallback() {
    let db = test_database();
    let ship = db.named("ship_hull_light_2").unwrap();
    let tank = db.named("light_tank_chassis_1").unwrap();
    let plane = db.named("small_plane_airframe_1").unwrap();
    let car = db.named("armored_car_equipment_1").unwrap();

    let all_dlc = SHIP_DLC_BIT | TANK_DLC_BIT | PLANE_DLC_BIT;
    assert_eq!(
        classify_designer(&db, ship, all_dlc).unwrap(),
        DesignerKind::Ship
    );
    assert_eq!(
        classify_designer(&db, tank, all_dlc).unwrap(),
        DesignerKind::Tank
    );
    assert_eq!(
        classify_designer(&db, plane, all_dlc).unwrap(),
        DesignerKind::Plane
    );
    assert_eq!(
        classify_designer(&db, car, all_dlc).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Land)
    );

    assert_eq!(
        classify_designer(&db, ship, 0).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Naval)
    );
    assert_eq!(
        classify_designer(&db, tank, 0).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Land)
    );
    assert_eq!(
        classify_designer(&db, plane, 0).unwrap(),
        DesignerKind::Upgrade(UpgradeDomain::Air)
    );
}
