//! What content says of the stockpile's equipment: type titles, archetypes and arms through
//! the recorded content stack, and where need can be derived.

use super::*;
use dashboard_base::products::equipment::{archetype_of, equipment_warnings};

/// Equipment from one authored file.
fn types(text: &[u8]) -> Database {
    installed_content_core::equipment::fixture::load(&[(
        "common/units/equipment/fixture.txt",
        std::str::from_utf8(text).unwrap(),
    )])
    .0
}

#[test]
fn stockpile_type_titles_preserve_empty_and_key_equal_localisations() {
    let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
        "localisation/english/equipment_l_english.yml",
        "\u{feff}l_english:\n empty_type:0 \"\"\n same_type:0 \"same_type\"\n",
    )]);
    let catalogue = dashboard_text::catalogue::Catalogue::of_layers_for_check(
        catalogue.content.layers.clone(),
        "english",
    )
    .unwrap();
    assert_eq!(type_name(&catalogue, "empty_type"), Some(String::new()));
    assert_eq!(
        type_name(&catalogue, "same_type").as_deref(),
        Some("same_type")
    );
    assert_eq!(type_name(&catalogue, "missing_type"), None);
    let findings = catalogue.take_findings();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].subject, "missing_type");
    assert_eq!(findings[0].message, "equipment type title is unavailable");
}

#[test]
fn native_equipment_redefinitions_and_diagnostics_are_kept() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let units = types(
        b"equipments = { guns = { is_archetype = maybe } tanks = { is_archetype = yes type = armor } TANKS = { is_archetype = yes type = fighter } }",
    );
    assert!(units.lost.is_none());
    assert_eq!(
        archetype_of(&units, "tanks"),
        Ok(("tanks".into(), 1, Domain::Air))
    );
    assert!(
        archetype_of(&units, "guns")
            .unwrap_err()
            .contains("no known archetype")
    );
    let unknown = types(b"duplicate_archetypes = { copy = { archetype = missing } }");
    assert!(
        equipment_warnings(&unknown)
            .iter()
            .any(|note| note.contains("copy") && note.contains("missing"))
    );
    assert!(archetype_of(&unknown, "copy").is_err());
}

#[test]
fn a_variant_s_type_groups_by_its_archetype_in_database_order_with_its_domain() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let units = types(
        br#"
        equipments = {
            infantry_equipment = { is_archetype = yes type = infantry }
            infantry_equipment_1 = { archetype = infantry_equipment }
            fighter_equipment = { is_archetype = yes type = fighter }
            fighter_equipment_0 = { archetype = fighter_equipment }
            convoy = { is_archetype = yes type = convoy }
        }
    "#,
    );
    // The recorded spelling may differ in ASCII case from the authored key.
    assert_eq!(
        archetype_of(&units, "Infantry_Equipment_1"),
        Ok(("infantry_equipment".to_owned(), 0, Domain::Land))
    );
    assert_eq!(
        archetype_of(&units, "fighter_equipment_0"),
        Ok(("fighter_equipment".to_owned(), 2, Domain::Air))
    );
    assert_eq!(
        archetype_of(&units, "convoy").map(|(_, _, domain)| domain),
        Ok(Domain::Naval)
    );
    assert_eq!(
        archetype_of(&units, "mod_gun"),
        Err("mod_gun is not authored equipment".to_owned())
    );
}

/// A mod layer adds its own archetypes (one in a new arm for a vanilla
/// chassis via `duplicate_archetypes`), and replaces a vanilla file whole,
/// dropping `supply_truck` from trucks. Types, arms, database order and need
/// rules follow the recorded content stack, never vanilla names.
#[test]
fn a_mod_s_equipment_types_resolve_through_the_recorded_content_stack() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_mod_equipment_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let write = |layer: &str, path: &str, text: &str| {
        let path = root.join(layer).join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "base",
        "common/units/equipment/infantry.txt",
        "equipments = { infantry_equipment = { is_archetype = yes type = infantry }
           infantry_equipment_1 = { archetype = infantry_equipment }
           motorized_equipment = { is_archetype = yes type = motorized supply_truck = yes } }",
    );
    write(
        "base",
        "common/units/equipment/tanks.txt",
        "equipments = { light_tank_chassis = { is_archetype = yes type = armor } }",
    );
    write(
        "mod",
        "common/units/equipment/infantry.txt",
        "equipments = { infantry_equipment = { is_archetype = yes type = infantry }
           infantry_equipment_1 = { archetype = infantry_equipment }
           motorized_equipment = { is_archetype = yes type = motorized } }",
    );
    write(
        "mod",
        "common/units/equipment/mod_walkers.txt",
        "equipments = { mod_walker_equipment = { is_archetype = yes type = armor }
           mod_walker_equipment_1 = { archetype = mod_walker_equipment }
           mod_airship_equipment = { is_archetype = yes type = strategic_bomber } }",
    );
    // Copies load after what they copy: the game reads the folder in path order.
    write(
        "mod",
        "common/units/equipment/x_mod_duplicates.txt",
        "duplicate_archetypes = { mod_sky_tank = { archetype = light_tank_chassis type = fighter } }",
    );
    let layers = installed_content_core::ResolvedLayers::from_folders(&[
        (&root.join("base"), &[][..]),
        (&root.join("mod"), &[][..]),
    ])
    .unwrap();
    let units = Database::load_complete(&layers).unwrap();
    // The mod's own types, by the recorded spelling, in the arm their category puts them.
    let (walker, _, walker_domain) = archetype_of(&units, "MOD_WALKER_EQUIPMENT_1").unwrap();
    assert_eq!(
        (walker.as_str(), walker_domain),
        ("mod_walker_equipment", Domain::Land)
    );
    assert_eq!(
        archetype_of(&units, "mod_airship_equipment").unwrap().2,
        Domain::Air
    );
    // A vanilla chassis cloned into a new arm by the mod.
    assert_eq!(archetype_of(&units, "mod_sky_tank").unwrap().2, Domain::Air);
    // Database order follows the stack's files, the mod's among them.
    let archetypes: Vec<_> = units
        .equipment
        .iter()
        .filter(|kind| kind.is_archetype)
        .collect();
    let place = |key: &str| {
        archetypes
            .iter()
            .position(|item| item.name == key.as_bytes())
            .unwrap()
    };
    assert!(place("motorized_equipment") < place("mod_walker_equipment"));
    assert!(place("mod_airship_equipment") < place("light_tank_chassis"));
    // The mod's trucks drop supply_truck, so their need is derived; its own types' too.
    assert_eq!(need_note(&units, "motorized_equipment").unwrap(), None);
    assert_eq!(need_note(&units, "mod_walker_equipment").unwrap(), None);
    // A token no layer declares is refused by name, never assumed.
    assert_eq!(
        archetype_of(&units, "vanilla_only_equipment"),
        Err("vanilla_only_equipment is not authored equipment".to_owned())
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn need_is_derived_only_where_the_available_stock_is_the_recorded_stock() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let units = types(
        br#"
        equipments = {
            infantry_equipment = { is_archetype = yes type = infantry }
            motorized_equipment = { is_archetype = yes type = motorized supply_truck = yes }
            mechanized_equipment = { is_archetype = yes type = mechanized supply_truck = no }
            train_equipment = { is_archetype = yes type = train }
            convoy = { is_archetype = yes type = convoy }
        }
    "#,
    );
    let note = |key| need_note(&units, key).unwrap();
    assert_eq!(note("infantry_equipment"), None);
    assert_eq!(note("mechanized_equipment"), None);
    for key in ["motorized_equipment", "train_equipment"] {
        let refusal = note(key).unwrap();
        assert!(refusal.contains("same balance write"));
        assert!(!refusal.contains("recording lacks"));
    }
    assert!(note("convoy").unwrap().contains("convoy"));
}
