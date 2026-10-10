use super::*;

fn catalog(input: &[u8]) -> Catalog {
    let (rows, warnings) = parse(input, &KeyNames::default()).unwrap();
    let mut result = Catalog {
        warnings,
        ..Default::default()
    };
    for row in rows {
        result.insert(row);
    }
    result.finalize();
    result
}

#[test]
fn mio_trait_names_follow_includes_overrides_and_first_match() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let db = catalog(
        br#"
        base={trait={token=one name=BASE_ONE} trait={token=two name=BASE_TWO}
            trait={token=three name=BASE_THREE} trait={token=duplicate name=BASE_DUP}}
        child={include=base remove_trait={three}
            trait={token=duplicate name=LOCAL_DUP}
            override_trait={token=duplicate name=OVERRIDE_DUP}
            override_trait={token=one name=""}
            override_trait={token=two delete_included_values={name}}
            override_trait={token=unmatched name=UNMATCHED}}
        named={include=base override_trait={token=one name=LOCAL delete_included_values={name}}}
    "#,
    );
    assert_eq!(
        db.trait_definition("CHILD", "ONE").unwrap().unwrap().name,
        "BASE_ONE"
    );
    assert_eq!(
        db.trait_definition("child", "two").unwrap().unwrap().name,
        ""
    );
    assert_eq!(
        db.trait_definition("child", "duplicate")
            .unwrap()
            .unwrap()
            .name,
        "LOCAL_DUP"
    );
    assert!(db.trait_definition("child", "three").unwrap().is_none());
    assert!(db.trait_definition("child", "unmatched").unwrap().is_none());
    assert_eq!(
        db.trait_definition("named", "one").unwrap().unwrap().name,
        "LOCAL"
    );
}

#[test]
fn mio_trait_include_pass_is_once_in_first_creation_order() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let db = catalog(
        br#"
        child={include=middle}
        middle={include=base}
        base={trait={token=obsolete name=OLD}}
        BASE={trait={token=active name=NEW}}
        after={include=middle}
    "#,
    );
    assert!(db.trait_definition("child", "active").unwrap().is_none());
    assert_eq!(
        db.trait_definition("middle", "active")
            .unwrap()
            .unwrap()
            .name,
        "NEW"
    );
    assert_eq!(
        db.trait_definition("after", "active")
            .unwrap()
            .unwrap()
            .name,
        "NEW"
    );
    assert!(db.trait_definition("base", "obsolete").unwrap().is_none());
}

#[test]
fn mio_trait_name_candidates_preserve_recorded_case_and_country_fallback() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let db = catalog(b"custom={trait={token=first} trait={token=second name=explicit_name}}");
    let first = db.trait_definition("custom", "first").unwrap().unwrap();
    assert_eq!(
        first.name_candidates("Recorded_MIO", "Recorded_Trait", Some("OWN")),
        [
            "OWN_Recorded_MIO_Recorded_Trait",
            "Recorded_MIO_Recorded_Trait"
        ]
    );
    let second = db.trait_definition("custom", "second").unwrap().unwrap();
    assert_eq!(
        second.name_candidates("Recorded_MIO", "Recorded_Trait", Some("OWN")),
        ["OWN_explicit_name", "explicit_name"]
    );
    assert_eq!(
        second.name_candidates("Recorded_MIO", "Recorded_Trait", None),
        ["explicit_name"]
    );
}

#[test]
fn mio_trait_readers_consume_names_modifiers_raw_lists_and_integer_points() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let db = catalog(
        br#"
        @name=FIRST
        custom={equipment_type={@name} research_categories={alpha}
            initial_trait={name=INITIAL position={x=1 y=2}}
            trait={token=first name=@name limit_to_equipment_type={@name}
                position={x=1 y=2} parent={traits={other} num_parents_needed=1}
                tree_header_text={@unreached=IGNORED}
                organization_modifier={@inside=SECOND @name=IGNORED political_power_gain=0.1}
                equipment_bonus={build_cost_ic=0.1}
                production_bonus={production_capacity_factor=0.1}
                available={always=yes} on_complete={set_country_flag=synthetic}
                ai_will_do={base=1 modifier={factor=2 always=yes}}}
            trait={token=second name=@inside} trait={token=third name=@name}}
    "#,
    );
    assert_eq!(
        db.trait_definition("custom", "first")
            .unwrap()
            .unwrap()
            .name,
        "FIRST"
    );
    assert_eq!(
        db.trait_definition("custom", "second")
            .unwrap()
            .unwrap()
            .name,
        "SECOND"
    );
    // The first file-scoped definition is kept and a duplicate is logged.
    assert_eq!(
        db.trait_definition("custom", "third")
            .unwrap()
            .unwrap()
            .name,
        "FIRST"
    );
    assert!(db.warnings.iter().any(|why| why.contains("already taken")));
    // Trait lookup scans only ordinary traits, excluding the initial trait.
    assert!(
        db.trait_definition("custom", "initial_trait")
            .unwrap()
            .is_none()
    );
    for input in [
        &b"custom={trait={token=bad position={1 2}}}"[..],
        b"custom={trait={token=bad position={x=1 y=2 z=3}}}",
    ] {
        assert!(
            parse(input, &KeyNames::default())
                .unwrap_err()
                .to_string()
                .contains("integer-point token recovery")
        );
    }
    let (rows, _) = parse(
        b"custom={trait={token=bad equipment_bonus={unknown={@name=LEAK}}} trait={token=next name=@name}}",
        &KeyNames::default(),
    ).unwrap();
    assert!(rows[0].traits[0].bonuses.is_empty());
    assert_eq!(rows[0].traits[1].name, "@name"); // Unknown block cannot leak its variable.
}

#[test]
fn mio_trait_bonuses_follow_duplicate_category_and_include_rules() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let db = catalog(br#"
        base={trait={token=one equipment_bonus={soft_attack=0.1 supply_consumption=-0.2}
            production_bonus={production_capacity_factor=0.3}
            organization_modifier={political_power_gain=0.4}}}
        child={include=base override_trait={token=one equipment_bonus={soft_attack=0.2 soft_attack=9}
            equipment_bonus={soft_attack=-0.1} delete_included_values={production_bonus}}}
        copied={include=base}
        invalid={trait={token=one equipment_bonus={soft_attack=0.1 production_cost_factor=0.2}
            production_bonus={soft_attack=0.3 production_capacity_factor=0.4}}}
        zero={include=base override_trait={token=one equipment_bonus={soft_attack=0}
            organization_modifier={hidden_modifier={political_power_gain=0}}}}
    "#);
    let child = db.trait_definition("child", "one").unwrap().unwrap();
    assert_eq!(
        child.bonuses,
        [Bonus {
            name: "soft_attack".into(),
            value: 10_000
        }]
    );
    assert_eq!(child.organisation_modifiers.values[0].value, 40_000);
    assert!(
        child
            .modifier_inheritance_refusal
            .as_deref()
            .unwrap()
            .contains("modifier tooltip-table copying")
    );
    let copied = db.trait_definition("copied", "one").unwrap().unwrap();
    assert_eq!(
        copied
            .bonuses
            .iter()
            .map(|bonus| (bonus.name.as_str(), bonus.value))
            .collect::<Vec<_>>(),
        [
            ("soft_attack", 10_000),
            ("supply_consumption", -20_000),
            ("production_capacity_factor", 30_000)
        ]
    );
    assert!(
        copied
            .modifier_inheritance_refusal
            .as_deref()
            .unwrap()
            .contains("modifier tooltip-table copying")
    );
    assert!(
        db.trait_definition("invalid", "one")
            .unwrap()
            .unwrap()
            .bonuses
            .is_empty()
    );
    let zero = db.trait_definition("zero", "one").unwrap().unwrap();
    assert_eq!(
        zero.bonuses
            .iter()
            .map(|bonus| (bonus.name.as_str(), bonus.value))
            .collect::<Vec<_>>(),
        [("soft_attack", 0), ("production_capacity_factor", 30_000)]
    );
    assert_eq!(zero.organisation_modifiers.values[0].value, 0);
    assert!(
        zero.organisation_modifiers
            .hidden
            .contains("political_power_gain")
    );
    assert!(
        zero.modifier_inheritance_refusal
            .as_deref()
            .unwrap()
            .contains("modifier effective-emptiness eligibility gate")
    );
}

#[test]
fn mio_trait_content_layers_replace_templates_and_keep_unread_suffix_named() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_mio_traits_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let base = root.join("base");
    let overlay = root.join("overlay");
    let directory = "common/military_industrial_organization/organizations";
    for folder in [&base, &overlay] {
        std::fs::create_dir_all(folder.join(directory)).unwrap();
    }
    std::fs::write(
        base.join(directory).join("a.txt"),
        b"custom={trait={token=old name=OLD}} removed={trait={token=old name=OLD}}",
    )
    .unwrap();
    std::fs::write(
        overlay.join(directory).join("b.txt"),
        b"CUSTOM={trait={token=new name=NEW}}",
    )
    .unwrap();
    let layers =
        ResolvedLayers::from_test_common_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
    let db = read(&layers).unwrap();
    assert!(db.trait_definition("custom", "old").unwrap().is_none());
    assert_eq!(
        db.trait_definition("custom", "new").unwrap().unwrap().name,
        "NEW"
    );
    let replaced =
        ResolvedLayers::from_test_common_folders(&[(&base, &[]), (&overlay, &[directory])])
            .unwrap();
    assert!(
        read(&replaced)
            .unwrap()
            .trait_definition("removed", "old")
            .unwrap()
            .is_none()
    );
    std::fs::write(
        overlay.join(directory).join("c.txt"),
        b"custom={trait={token=bad position={1 2}}}",
    )
    .unwrap();
    let layers =
        ResolvedLayers::from_test_common_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
    let why = read(&layers)
        .unwrap()
        .trait_definition("custom", "new")
        .unwrap_err();
    assert!(why.contains("c.txt") && why.contains("count: 1"), "{why}");
    let cleanup = std::fs::canonicalize(root).unwrap();
    assert_eq!(
        cleanup.parent(),
        Some(
            std::fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .as_path()
        )
    );
    std::fs::remove_dir_all(cleanup).unwrap();
}
