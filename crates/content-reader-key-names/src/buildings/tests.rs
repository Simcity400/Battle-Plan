use super::*;

fn parse(input: &[u8]) -> io::Result<Buildings> {
    let resources = ["mineral_y", "mineral_x"].map(|token| crate::resources::Resource {
        token: token.into(),
        icon_frame: None,
    });
    super::parse_with_resources(input, &resources, &KeyNames::default())
}

const SAMPLE: &[u8] = br#"
buildings = {
workshop = { show_on_map = 4 level_cap = { state_max = 17 } }
harbor_store = {
    show_on_map = 2
    show_on_map_meshes = 4
    always_shown = yes
    is_port = yes
    spawn_point = harbor_store_spawn
    level_cap = { province_max = 7 }
}
signal_tower = { show_on_map = 2 need_detection = yes specialization = { survey_role } level_cap = { state_max = 3 } }
coast_office = { show_on_map = 3 naval_headquarter = yes only_costal = yes level_cap = { province_max = 4 } }
lookout = { show_on_map = 2 drawn_at_distance = yes level_cap = { province_max = 2 } }
transfer_depot = { show_on_map = 3 supply_node = yes level_cap = { province_max = 5 } }
trail_network = { level_cap = { state_max = 9 } }
}
spawn_points = {
harbor_store_spawn = { max = 2 type = province show_on_map_meshes = 4 }
}
"#;

#[test]
fn icon_frames_are_scanned_and_a_differing_repeat_is_a_repeated_type() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"@f=4 buildings={ a={icon_frame=3} b={} c={icon_frame=@f} d={icon_frame=bad} e={icon_frame=2} e={icon_frame=5} }",
    )
    .unwrap();
    let frame = |name: &str| buildings.types[name].icon_frame;
    assert_eq!(
        (frame("a"), frame("b"), frame("c"), frame("d")),
        (3, 0, 4, 0)
    );
    assert_eq!(frame("e"), 5);
    assert_eq!(buildings.repeated_types["e"].len(), 2);
    assert!(!buildings.repeated_types.contains_key("a"));
}

#[test]
fn keywords_scans_and_resource_registry_control_recovery() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        br#"@cap=010 BUILDINGS={a={
        "show_on_map"=99 SHOW_ON_MAP=@cap show_on_map=bad
        always_shown=yes always_shown=YES tags={a} tags={b}
        local_resources_missing={show_on_map=90}
        local_resources_MINERAL_X=1.25 local_resources_mineral_x=0.75
    }} spawn_points={s={unknown={max=7} max=99}}"#,
    )
    .unwrap();
    let a = &buildings.types["a"];
    assert_eq!(a.show_on_map, Some(8));
    assert!(a.always_shown);
    assert_eq!(a.tags, ["a", "b"]);
    assert_eq!(a.local_resources, [("mineral_x".into(), 200_000)]);
    assert_eq!(buildings.spawn_points["s"].max, 7);
    assert!(
        super::parse(b"buildings={a={local_resources_x=1}}", &KeyNames::default())
            .unwrap_err()
            .to_string()
            .contains("resource registry required")
    );
}

#[test]
fn type_tokens_fold_ascii_and_refuse_differing_declarations() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        "buildings={CUSTOM_FACTORY={icon_frame=3} custom_factory={icon_frame=3} Ä={icon_frame=1} ä={icon_frame=2}}"
            .as_bytes(),
    )
    .unwrap();
    for token in ["custom_factory", "CUSTOM_FACTORY", "Custom_Factory"] {
        let building = buildings.by_token(token).unwrap().unwrap();
        assert_eq!(
            (building.name.as_str(), building.icon_frame),
            ("CUSTOM_FACTORY", 3)
        );
    }
    assert_eq!(buildings.by_token("missing").unwrap(), None);
    assert_eq!(buildings.by_token("Ä").unwrap().unwrap().icon_frame, 1);
    assert_eq!(buildings.by_token("ä").unwrap().unwrap().icon_frame, 2);
    for source in [
        b"buildings={CUSTOM_FACTORY={icon_frame=3} custom_factory={icon_frame=4}}".as_slice(),
        b"buildings={CUSTOM_FACTORY={icon_frame=3} CUSTOM_FACTORY={icon_frame=4}}",
    ] {
        let buildings = parse(source).unwrap();
        for token in ["custom_factory", "CUSTOM_FACTORY", "Custom_Factory"] {
            assert!(
                buildings
                    .by_token(token)
                    .unwrap_err()
                    .to_string()
                    .contains(token)
            );
        }
    }
}

#[test]
fn slot_lookup_folds_ascii_without_changing_authored_names() {
    content_reader_script::test_game::enter();
    let buildings =
        parse(b"buildings={Port={is_port=yes spawn_point=Harbour}} spawn_points={Harbour={}}")
            .unwrap();
    let bucket = |name| match buildings.slot(name) {
        Some(Slot::Bucket(spawn)) => spawn,
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(bucket("PORT").name, "Port");
    assert_eq!(bucket("harbour").name, "Harbour");
    assert!(bucket("hArBoUr").is_port);
    assert_eq!(buildings.slot("depot"), None);
    let repeated = parse(b"buildings={Port={} Port={}}").unwrap();
    assert!(matches!(repeated.slot("port"), Some(Slot::Bucket(_))));
}

#[test]
fn folded_slot_names_take_the_first_bucket_and_refuse_only_differing_repeats() {
    content_reader_script::test_game::enter();
    for source in [
        b"buildings={Port={} port={} depot={}}".as_slice(),
        b"buildings={Port={} depot={}} spawn_points={port={}}",
    ] {
        let buildings = parse(source).unwrap();
        for name in ["Port", "port", "PORT"] {
            assert!(
                matches!(buildings.slot(name), Some(Slot::Bucket(spawn)) if spawn.name == "Port"),
                "{name}"
            );
        }
        assert!(matches!(buildings.slot("depot"), Some(Slot::Bucket(_))));
    }
    for source in [
        b"buildings={depot={}} spawn_points={Port={} port={}}".as_slice(),
        b"buildings={port={} port={show_on_map=1} depot={}}",
        b"buildings={Port={} port={show_on_map=1} depot={}}",
    ] {
        let buildings = parse(source).unwrap();
        for name in ["Port", "port", "PORT"] {
            assert!(
                matches!(buildings.slot(name), Some(Slot::Ambiguous(_))),
                "{name}"
            );
        }
        assert!(matches!(buildings.slot("depot"), Some(Slot::Bucket(_))));
    }
    let buildings = parse("buildings={Ä={} ä={}}".as_bytes()).unwrap();
    assert!(matches!(buildings.slot("Ä"), Some(Slot::Bucket(spawn)) if spawn.name == "Ä"));
    assert!(matches!(buildings.slot("ä"), Some(Slot::Bucket(spawn)) if spawn.name == "ä"));
}

#[test]
fn reads_the_map_keys_and_the_redirected_port_bucket() {
    content_reader_script::test_game::enter();
    let buildings = parse(SAMPLE).unwrap();
    let naval = &buildings.types["harbor_store"];
    assert_eq!(naval.show_on_map, Some(2));
    assert_eq!(naval.show_on_map_meshes, 4);
    assert!(naval.always_shown && naval.is_port && !naval.need_detection);
    assert!(buildings.types["lookout"].drawn_at_distance);
    assert!(buildings.types["signal_tower"].need_detection);
    assert_eq!(
        buildings.types["signal_tower"].specialization,
        ["survey_role"]
    );
    let headquarters = &buildings.types["coast_office"];
    assert!(headquarters.naval_headquarter && headquarters.only_costal);
    assert!(headquarters.specialization.is_empty() && !naval.naval_headquarter);
    assert!(buildings.types["transfer_depot"].supply_node);
    assert_eq!(naval.slot_name(), "harbor_store_spawn");
    assert!(buildings.spawn_points["harbor_store_spawn"].is_port);
    assert!(buildings.spawn_points["harbor_store_spawn"].provincial);
    assert_eq!(buildings.types["trail_network"].show_on_map, None);
    assert_eq!(
        buildings.slot_candidates("harbor_store_spawn").next(),
        Some(naval)
    );
    assert_eq!(
        buildings.slot_candidates("harbor_store").next(),
        Some(naval)
    );
}

#[test]
fn generated_and_explicit_buckets_have_independent_defaults() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        br#"
        buildings = {
            hidden = { level_cap = { province_max = 0 } }
            base = { show_on_map = 2 show_on_map_meshes = 3 spawn_point = remote level_cap = { province_max = 10 } }
            signed = { show_on_map = -1 show_on_map_meshes = -2 }
            unknown = { spawn_point = missing }
        }
        spawn_points = {
            defaults = { }
            remote = { type = state max = 7 show_on_map_meshes = 9 }
            base = { max = 9 }
        }
        "#,
    )
    .unwrap();
    let spawn = |name: &str, max, meshes, provincial| SpawnPoint {
        name: name.to_owned(),
        max,
        meshes,
        provincial,
        is_port: false,
    };
    assert_eq!(
        buildings.spawn_points["hidden"],
        spawn("hidden", 0, 1, false)
    );
    assert_eq!(buildings.spawn_points["base"], spawn("base", 2, 3, true));
    assert_eq!(
        buildings.spawn_points["signed"],
        spawn("signed", -1, -2, false)
    );
    assert_eq!(
        buildings.spawn_points["defaults"],
        spawn("defaults", 1, 1, false)
    );
    assert_eq!(
        buildings.spawn_points["remote"],
        spawn("remote", 7, 9, false)
    );
    assert_eq!(
        buildings.slot_candidates("remote").next().unwrap().name,
        "base"
    );
    assert_eq!(buildings.types["unknown"].slot_name(), "unknown");
    assert!(buildings.types["signed"].entity_names().is_empty());
    assert!(
        !parse(b"spawn_points={bad={type=country}}")
            .unwrap()
            .spawn_points["bad"]
            .provincial
    );
}

#[test]
fn local_resources_keep_first_order_and_sum_repeats() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={refinery={local_resources_mineral_y=1 local_resources_mineral_x=-0.5 local_resources_mineral_y=0.25}}",
    )
    .unwrap();
    assert_eq!(
        buildings.types["refinery"].local_resources,
        [
            ("mineral_y".to_owned(), 125_000),
            ("mineral_x".to_owned(), -50_000)
        ]
    );
}

#[test]
fn failed_bool_scan_retains_the_constructor_value() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={bunker={show_on_map=2 air_base=maybe tags={a}} fort={show_on_map=1 local_resources_mineral_x=010}}",
    ).unwrap();
    let bunker = &buildings.types["bunker"];
    assert_eq!(bunker.show_on_map, Some(2));
    assert_eq!(bunker.raid_refused, None);
    assert_eq!(bunker.tags, ["a"]);
    assert!(!bunker.air_base);
    assert_eq!(
        buildings.types["fort"].local_resources,
        [("mineral_x".to_owned(), 800_000)]
    );
    assert_eq!(buildings.types["fort"].raid_refused, None);
}

#[test]
fn shared_buckets_keep_database_order_and_repeats_are_ambiguous() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={z={spawn_point=shared show_on_map=1} a={spawn_point=shared show_on_map=1}} spawn_points={shared={}}",
    )
    .unwrap();
    let names: Vec<_> = buildings
        .slot_candidates("shared")
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(names, ["z", "a"]);
    assert!(buildings.ambiguous_slots.is_empty());
    let repeated = parse(
        b"buildings={air={spawn_point=old} air={spawn_point=new}} spawn_points={old={} new={} s={max=1} s={max=0}}",
    )
    .unwrap();
    assert_eq!(
        repeated.ambiguous_slots,
        BTreeSet::from(["air", "new", "old", "s"].map(String::from))
    );
    let identical = parse(b"buildings={air={show_on_map=1} air={show_on_map=1}}").unwrap();
    assert!(identical.ambiguous_slots.is_empty());
    assert!(identical.repeated_types.is_empty());
}

#[test]
fn factory_flags_are_signed_integers_and_differing_repeats_are_ambiguous() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={arms={military_production=1 naval_production=0} mod_works={general_production=2 military_production=-1 military_production=3 rocket_production=-1 rocket_production=2} depot={rocket_launch_capacity=3} yard={naval_production=1} yard={naval_production=1} odd={general_production=1} odd={general_production=0} repeat={rocket_production=0} repeat={rocket_production=1} retired={rocket_production=-2}}",
    )
    .unwrap();
    let flags = |name: &str| buildings.factory_flags[name];
    assert!(flags("arms").military() && !flags("arms").naval() && !flags("arms").civilian());
    assert_eq!(
        flags("mod_works"),
        FactoryFlags {
            military_production: 3,
            naval_production: 0,
            general_production: 2,
            rocket_production: 2,
        }
    );
    assert!(flags("mod_works").rocket());
    assert_eq!(flags("depot"), FactoryFlags::default());
    assert_eq!(buildings.types["depot"].rocket_launch_capacity, 3);
    assert!(!flags("retired").rocket());
    assert!(flags("yard").naval());
    assert_eq!(
        buildings.ambiguous_factory_types,
        BTreeSet::from(["odd".to_owned(), "repeat".to_owned()])
    );
    let decimal =
        parse(b"buildings={arms={military_production=1.5 rocket_production=2.5}}").unwrap();
    assert_eq!(decimal.factory_flags["arms"].military_production, 1);
    assert_eq!(decimal.factory_flags["arms"].rocket_production, 2);
}

#[cfg(feature = "test-support")]
#[test]
fn factory_flags_merge_across_building_files() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_building_flags_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (file, text) in [
        (
            "00_a.txt",
            "buildings = { air_base = { air_base = yes } workshop = { military_production = 1 } }",
        ),
        (
            "01_b.txt",
            "buildings = { industrial_complex = { general_production = 1 } workshop = { military_production = 2 } }",
        ),
    ] {
        let path = root.join("common/buildings").join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let buildings = read(&ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(buildings.factory_flags["air_base"], FactoryFlags::default());
    assert!(buildings.factory_flags["industrial_complex"].civilian());
    assert_eq!(buildings.factory_flags["workshop"].military_production, 2);
    assert!(buildings.ambiguous_factory_types.contains("workshop"));
    assert!(!buildings.ambiguous_factory_types.contains("air_base"));
}

#[test]
fn only_direct_building_files_load() {
    assert!(is_building_file(b"COMMON/BUILDINGS/UPPER.TXT"));
    assert!(!is_building_file(b"common/buildings/nested/air.txt"));
    assert!(!is_building_file(b"common/buildings/air.asset"));
}

#[test]
fn entity_names_and_variants_use_the_content_level_denominator() {
    content_reader_script::test_game::enter();
    let buildings = parse(SAMPLE).unwrap();
    let naval = &buildings.types["harbor_store"];
    assert_eq!(
        buildings.types["workshop"].entity_names(),
        ["building_workshop"]
    );
    assert_eq!(
        naval.entity_names(),
        [
            "building_harbor_store_1",
            "building_harbor_store_2",
            "building_harbor_store_3",
            "building_harbor_store_4"
        ]
    );
    assert_eq!(
        naval.destroyed_entity_name(),
        "building_harbor_store_destroyed"
    );
    for (level, variant) in [(0, 0), (3, 0), (4, 1), (8, 2), (12, 3), (99, 3)] {
        assert_eq!(naval.variant_for_level(level, 15), variant);
    }
    assert_eq!(naval.variant_for_level(4, 10), 1);
    assert_eq!(buildings.types["workshop"].variant_for_level(20, 15), 0);
}
#[test]
fn bucket_references_match_under_the_lexers_fold() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={
            port={is_port=yes spawn_point=Shared}
            hq={spawn_point=SHARED_TOO}
            dock={spawn_point=PORT}
        }
        spawn_points={shared={} Port={} shared_too={} Shared_Too={max=2}}",
    )
    .unwrap();
    assert_eq!(
        buildings.types["port"].spawn_point.as_deref(),
        Some("shared")
    );
    assert!(buildings.spawn_points["shared"].is_port);
    assert!(!buildings.spawn_points.contains_key("Port"));
    assert_eq!(buildings.types["dock"].spawn_point, None);
    assert_eq!(
        buildings.types["hq"].spawn_point.as_deref(),
        Some("Shared_Too")
    );
    assert_eq!(
        buildings.ambiguous_slots,
        BTreeSet::from(["Shared_Too".to_owned()])
    );
    let reached = |name| buildings.slot(name).map(|slot| slot.spawn().name.as_str());
    assert_eq!(reached("SHARED"), Some("shared"));
    assert_eq!(reached("Port"), Some("port"));
    assert_eq!(reached("shared_too"), Some("Shared_Too"));
    assert!(matches!(
        buildings.slot("shared_too"),
        Some(Slot::Ambiguous(_))
    ));
    assert_eq!(reached("absent"), None);
}

#[test]
fn a_differing_repeat_keeps_every_declaration() {
    content_reader_script::test_game::enter();
    let repeated = parse(
        b"buildings={hq={naval_headquarter=yes} dock={} hq={show_on_map=1} hq={show_on_map=1}}",
    )
    .unwrap();
    assert!(!repeated.types["hq"].naval_headquarter);
    let all = &repeated.repeated_types["hq"];
    assert_eq!(
        all.iter()
            .map(|kind| kind.naval_headquarter)
            .collect::<Vec<_>>(),
        [true, false]
    );
    assert_eq!(repeated.repeated_types.len(), 1);
    assert_eq!(repeated.declaration_order, ["hq", "dock"]);
}

#[test]
fn empty_replacement_files_contribute_no_buildings() {
    content_reader_script::test_game::enter();
    for input in [
        b"".as_slice(),
        b"# disabled by content\n",
        b"buildings={} spawn_points={}",
    ] {
        assert_eq!(parse(input).unwrap(), Buildings::default());
    }
    for input in [b"buildings={".as_slice(), b"buildings=not_a_block"] {
        assert!(parse(input).unwrap().types.is_empty());
    }
}

#[test]
fn base_maximum_prefers_positive_caps_without_reading_the_default() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={
            both={level_cap={state_max=9 province_max=4}}
            state_only={level_cap={state_max=9}}
            province_only={level_cap={province_max=4}}
        }",
    )
    .unwrap();
    let defines = content_reader_defines::defines::Defines::new().unwrap();
    for (name, maximum) in [("both", 4), ("state_only", 9), ("province_only", 4)] {
        assert_eq!(
            buildings.types[name].base_maximum(&defines).unwrap(),
            maximum
        );
    }
    assert_eq!(buildings.types["state_only"].province_max, None);
    assert_eq!(buildings.types["province_only"].state_max, 0);
}

#[test]
fn level_caps_normalize_negative_values_and_province_above_positive_state() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={
            oversized={level_cap={state_max=4 province_max=6}}
            negative={level_cap={state_max=-5 province_max=-3}}
            negative_state={level_cap={state_max=-5 province_max=7}}
            negative_province={level_cap={state_max=4 province_max=-3}}
            zero_state={level_cap={state_max=0 province_max=7}}
            equal={level_cap={state_max=4 province_max=4}}
        }",
    )
    .unwrap();
    for (name, state_max, province_max) in [
        ("oversized", 4, 0),
        ("negative", 0, 0),
        ("negative_state", 0, 7),
        ("negative_province", 4, 0),
        ("zero_state", 0, 7),
        ("equal", 4, 4),
    ] {
        let building = &buildings.types[name];
        assert_eq!(building.state_max, state_max, "{name}");
        assert_eq!(building.province_max, Some(province_max), "{name}");
        assert_eq!(building.provincial(), province_max > 0, "{name}");
    }
}

#[test]
fn repeated_level_cap_blocks_validate_before_later_assignments() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={
            rejected_first={
                level_cap={state_max=4 province_max=6}
                level_cap={state_max=8}
            }
            rejected_later={
                level_cap={province_max=6}
                level_cap={state_max=4}
                level_cap={state_max=8}
            }
            raised_same_block={level_cap={state_max=4 province_max=6 state_max=8}}
        }",
    )
    .unwrap();
    for name in ["rejected_first", "rejected_later"] {
        assert_eq!(buildings.types[name].state_max, 8);
        assert_eq!(buildings.types[name].province_max, Some(0));
    }
    assert_eq!(buildings.types["raised_same_block"].state_max, 8);
    assert_eq!(buildings.types["raised_same_block"].province_max, Some(6));
}

#[test]
fn failed_cap_scans_retain_the_previous_normalized_values() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        b"buildings={
            rejected={
                level_cap={state_max=3 province_max=7}
                level_cap={state_max=bad province_max=bad}
            }
            negative={
                level_cap={state_max=-4 province_max=-2}
                level_cap={state_max=bad province_max=bad}
            }
            retained={
                level_cap={state_max=014 province_max=0x5}
                level_cap={state_max=bad province_max=bad}
            }
        }",
    )
    .unwrap();
    for (name, state_max, province_max) in
        [("rejected", 3, 0), ("negative", 0, 0), ("retained", 12, 5)]
    {
        assert_eq!(buildings.types[name].state_max, state_max, "{name}");
        assert_eq!(
            buildings.types[name].province_max,
            Some(province_max),
            "{name}"
        );
    }
}

#[test]
fn differing_state_caps_refuse_repeated_type_tokens() {
    content_reader_script::test_game::enter();
    for source in [
        b"buildings={Depot={level_cap={state_max=3}} depot={level_cap={state_max=4}}}".as_slice(),
        b"buildings={Depot={level_cap={state_max=3}} Depot={level_cap={state_max=4}}}",
    ] {
        let buildings = parse(source).unwrap();
        assert!(
            buildings
                .by_token("DEPOT")
                .unwrap_err()
                .to_string()
                .contains("differing fields")
        );
    }
    let equal =
        parse(b"buildings={Depot={level_cap={state_max=3}} depot={level_cap={state_max=3}}}")
            .unwrap();
    assert_eq!(equal.by_token("DEPOT").unwrap().unwrap().state_max, 3);
}

#[test]
fn nonpositive_caps_use_the_exact_signed_default() {
    content_reader_script::test_game::enter();
    let buildings =
        parse(b"buildings={absent={} zero={level_cap={state_max=0 province_max=0}}}").unwrap();
    let defines = content_reader_defines::defines::Defines::new().unwrap();
    assert!(buildings.types["absent"].base_maximum(&defines).is_err());
    for (source, maximum) in [
        (
            b"NDefines={NBuildings={MAX_BUILDING_LEVELS=-7}}".as_slice(),
            -7,
        ),
        (b"NDefines={NBuildings={MAX_BUILDING_LEVELS=0}}", 0),
        (
            b"NDefines={NBuildings={MAX_BUILDING_LEVELS=2147483648}}",
            i32::MIN,
        ),
    ] {
        defines.load("caps.lua", source).unwrap();
        for name in ["absent", "zero"] {
            assert_eq!(
                buildings.types[name].base_maximum(&defines).unwrap(),
                maximum
            );
        }
    }
}

#[test]
fn infrastructure_is_the_first_flagged_type_and_reads_its_level_cap_forms() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        br#"buildings={
        plain={level_cap={state_max=3}}
        roads={infrastructure=yes naval_fort=2 level_cap={state_max=5 shares_slots=yes group_by=paths exclusive_with={canal ferry}}}
        rails={infrastructure=yes land_fort=1 level_cap={exclusive_with=canal}}
        }"#,
    )
    .unwrap();
    let roads = &buildings.types["roads"];
    assert!(roads.infrastructure && roads.shares_slots);
    assert_eq!(roads.naval_fort, 2);
    assert_eq!(roads.group_by.as_deref(), Some(b"paths".as_slice()));
    assert_eq!(roads.exclusive_with, [b"canal".to_vec(), b"ferry".to_vec()]);
    assert_eq!(buildings.types["rails"].exclusive_with, [b"canal".to_vec()]);
    let plain = &buildings.types["plain"];
    assert!(!plain.infrastructure && !plain.shares_slots && plain.group_by.is_none());
    let unheld = parse(
        b"buildings={roads={infrastructure=yes level_cap={group_by=pa\xe9ths exclusive_with={\xe9}}}}",
    )
    .unwrap();
    let roads = &unheld.types["roads"];
    assert_eq!(roads.group_by.as_deref(), Some(b"pa\xe9ths".as_slice()));
    assert_eq!(roads.exclusive_with, [b"\xe9".to_vec()]);
    assert_eq!(
        buildings.infrastructure().unwrap().map(|b| b.name.as_str()),
        Some("roads")
    );
    assert_eq!(
        parse(b"buildings={plain={}}").unwrap().infrastructure(),
        Ok(None)
    );
    let repeated =
        parse(b"buildings={roads={} roads={infrastructure=yes} rails={infrastructure=yes}}")
            .unwrap();
    assert!(repeated.infrastructure().unwrap_err().contains("roads"));
}

#[test]
fn state_view_fields_keep_the_landmark_icon_display_gate_dlc_trigger_and_state_modifiers() {
    content_reader_script::test_game::enter();
    let buildings = parse(
        br#"buildings={ landmark={ only_display_if_exists=yes special_icon="GFX_landmark" dlc_allowed={ always=yes } state_modifiers={ stability_factor=0.1 } } plain={} }"#,
    )
    .unwrap();
    let landmark = &buildings.types["landmark"];
    assert!(landmark.only_display_if_exists);
    assert_eq!(landmark.special_icon, "GFX_landmark");
    assert_eq!(landmark.dlc_allowed.len(), 1);
    assert_eq!(landmark.state_modifiers.len(), 1);
    let plain = &buildings.types["plain"];
    assert!(!plain.only_display_if_exists && plain.special_icon.is_empty());
    assert!(plain.dlc_allowed.is_empty() && plain.state_modifiers.is_empty());
}
