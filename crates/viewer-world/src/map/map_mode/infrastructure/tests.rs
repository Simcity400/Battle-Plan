use super::*;
use installed_content_core::{key_names::KeyNames, technology::Definition};

fn defines() -> Defines {
    let defines = Defines::new().unwrap();
    defines
        .load(
            "common/defines/synthetic.lua",
            b"NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 15 } }",
        )
        .unwrap();
    defines
}

fn technology(id: &str, building: &str, level: i32) -> Definition {
    Definition {
        id: id.as_bytes().to_vec(),
        buildings: vec![(building.as_bytes().to_vec(), level)],
        ..Default::default()
    }
}

/// The content read for one building file, with the other inputs as given.
fn read(
    buildings: &str,
    categories: &str,
    locations: &str,
    technologies: Result<Vec<Definition>, String>,
) -> Content {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let names = KeyNames::default();
    let buildings = installed_content_core::buildings::parse(buildings.as_bytes(), &names).unwrap();
    let categories = state_categories::parse(categories.as_bytes(), &names).unwrap();
    let locations = strategic_locations::parse(locations.as_bytes(), &names).unwrap();
    let catalog = technologies.map(|definitions| {
        let mut catalog = Catalog::default();
        catalog.definitions = definitions;
        catalog
    });
    Content::new(
        &buildings,
        &defines(),
        &categories,
        &locations,
        catalog.as_ref().map_err(Clone::clone),
    )
    .unwrap()
}

const ROADS: &str = "buildings={plain={} roads={infrastructure=yes level_cap={state_max=5}}}";

#[test]
fn the_first_infrastructure_type_gives_the_maximum_and_its_enabling_technologies() {
    let content = read(
        ROADS,
        "state_categories={isle={buildings_max_level={naval_base=2}} Ridge={buildings_max_level={Roads=3}}}",
        "harbor={naval_base=2}",
        Ok(vec![
            technology("Rail_A", "ROADS", 3),
            technology("rail_b", "naval_base", 2),
            technology("rail_a", "roads", 3),
        ]),
    );
    assert!(content.refused.is_empty(), "{:?}", content.refused);
    let building = content.building.unwrap();
    assert_eq!((building.name.as_str(), building.maximum), ("roads", 5));
    assert!(building.in_states);
    assert_eq!(
        building.categories,
        BTreeMap::from([(b"ridge".to_vec(), 3)])
    );
    // Every declaration in database order, each with its own unlock.
    assert_eq!(
        building.technologies.unwrap().occurrences,
        [
            (b"rail_a".to_vec(), Some(3)),
            (b"rail_b".to_vec(), None),
            (b"rail_a".to_vec(), Some(3)),
        ]
    );
    // Without a cap the content's MAX_BUILDING_LEVELS is the maximum, and no technology means none gates it.
    let uncapped = read(
        "buildings={roads={infrastructure=yes}}",
        "",
        "",
        Ok(Vec::new()),
    );
    let building = uncapped.building.unwrap();
    assert_eq!((building.maximum, building.technologies), (15, None));
    assert_eq!(
        read("buildings={plain={}}", "", "", Ok(Vec::new())).building,
        Content::default().building
    );
}

#[test]
fn each_unrecorded_limit_operand_refuses_the_layer_by_name() {
    let refused = |buildings: &str, categories: &str, locations: &str, technologies| {
        let content = read(buildings, categories, locations, technologies);
        assert!(content.building.is_ok());
        assert_eq!(content.refused.len(), 1, "{:?}", content.refused);
        content.refused[0].clone()
    };
    for (building, named) in [
        ("land_fort=1", "fort"),
        ("naval_fort=2", "fort"),
        (
            "level_cap={state_max=5 exclusive_with=canal}",
            "exclusive_with",
        ),
        ("level_cap={state_max=5 shares_slots=yes}", "shares_slots"),
        ("level_cap={state_max=5 group_by=paths}", "group_by"),
    ] {
        let why = refused(
            &format!("buildings={{roads={{infrastructure=yes {building}}}}}"),
            "",
            "",
            Ok(Vec::new()),
        );
        assert!(why.contains(named), "{why}");
    }
    let why = refused(
        ROADS,
        "state_categories={isle={buildings_max_level={Roads=many}} town={}}",
        "",
        Ok(Vec::new()),
    );
    assert!(
        why.contains("isle") && why.contains("not an integer"),
        "{why}"
    );
    let why = refused(
        ROADS,
        "",
        "pass={roads=1} harbor={naval_base=2}",
        Ok(Vec::new()),
    );
    assert!(
        why.starts_with("1 strategic locations") && why.contains("pass"),
        "{why}"
    );
    let why = refused(ROADS, "", "", Err("technology files unread".into()));
    assert!(why.contains("technology files unread"), "{why}");
    // Repeated declarations keep each one's unlock, including none; nothing is refused.
    let repeated = read(
        ROADS,
        "",
        "",
        Ok(vec![
            technology("t", "naval_base", 2),
            technology("T", "roads", 3),
            technology("t", "roads", 4),
        ]),
    );
    assert!(repeated.refused.is_empty(), "{:?}", repeated.refused);
    assert_eq!(
        repeated.building.unwrap().technologies.unwrap().occurrences,
        [
            (b"t".to_vec(), None),
            (b"t".to_vec(), Some(3)),
            (b"t".to_vec(), Some(4)),
        ]
    );
}

#[test]
fn a_type_no_state_status_can_hold_needs_no_limit_operand() {
    for scope in [
        "level_cap={province_max=3}",
        "only_costal=yes level_cap={state_max=5}",
    ] {
        let content = read(
            &format!("buildings={{roads={{infrastructure=yes land_fort=1 {scope}}}}}"),
            "state_categories={isle={buildings_max_level={roads=2}}}",
            "pass={roads=1}",
            Err("technology files unread".into()),
        );
        assert!(content.refused.is_empty(), "{:?}", content.refused);
        assert!(!content.building.unwrap().in_states);
    }
}

#[test]
fn a_partly_read_technology_catalogue_refuses_the_layer() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let names = KeyNames::default();
    let buildings = installed_content_core::buildings::parse(ROADS.as_bytes(), &names).unwrap();
    let categories = state_categories::parse(b"", &names).unwrap();
    let locations = strategic_locations::parse(b"", &names).unwrap();
    // The definitions read so far enable nothing; the refusal names the unread file.
    let mut catalog = Catalog::default();
    catalog.definitions = vec![technology("rail", "naval_base", 1)];
    catalog.refusal = Some("common/technologies/b.txt: technology description unread".into());
    let content = Content::new(
        &buildings,
        &defines(),
        &categories,
        &locations,
        Ok(&catalog),
    )
    .unwrap();
    assert_eq!(content.refused.len(), 1, "{:?}", content.refused);
    assert!(
        content.refused[0].contains("b.txt"),
        "{:?}",
        content.refused
    );
}

#[test]
fn a_child_technology_inherits_its_parents_infrastructure_unlock() {
    let child = Definition {
        id: b"rail_child".to_vec(),
        parent: b"RAIL_BASE".to_vec(),
        ..Default::default()
    };
    let content = read(
        ROADS,
        "",
        "",
        Ok(vec![technology("rail_base", "roads", 4), child]),
    );
    assert!(content.refused.is_empty(), "{:?}", content.refused);
    // The child's declaration carries its parent's level 4, so completing it caps at 4.
    assert_eq!(
        content.building.unwrap().technologies.unwrap().occurrences,
        [
            (b"rail_base".to_vec(), Some(4)),
            (b"rail_child".to_vec(), Some(4))
        ]
    );
}
