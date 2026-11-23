use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn organisation_gates_inherit_parent_local_bodies_and_filters_copy() {
    content_reader_script::test_game::enter();
    let db = catalog(br#"
      base={visible={always=yes} available={always=no} equipment_type={armor}}
      child={include=base}
      grandchild={include=child}
      deleted={include=base delete_included_values={visible available equipment_type}}
      local={include=base visible={always=no} equipment_type={air} delete_included_values={visible equipment_type}}
    "#);
    let child = db.tree("child").unwrap().unwrap();
    assert_eq!(child.visible[0].value.scalar_str(), Some("yes"));
    assert_eq!(child.available[0].value.scalar_str(), Some("no"));
    assert_eq!(child.equipment_types, [b"armor".to_vec()]);
    let grandchild = db.tree("grandchild").unwrap().unwrap();
    assert!(grandchild.visible.is_empty() && grandchild.available.is_empty());
    assert_eq!(grandchild.equipment_types, [b"armor".to_vec()]);
    let deleted = db.tree("deleted").unwrap().unwrap();
    assert!(
        deleted.visible.is_empty()
            && deleted.available.is_empty()
            && deleted.equipment_types.is_empty()
    );
    let local = db.tree("local").unwrap().unwrap();
    assert_eq!(local.visible[0].value.scalar_str(), Some("no"));
    assert_eq!(local.equipment_types, [b"air".to_vec()]);
}

#[test]
fn research_categories_and_trait_conditions_are_retained_and_inherited() {
    content_reader_script::test_game::enter();
    let db = catalog(br#"
      base={research_categories={cat_a cat_b}
          trait={token=gated available={always=no} visible={always=yes}}
          trait={token=open}}
      child={include=base
          override_trait={token=gated visible={always=no}}}
      local={include=base research_categories={cat_c}
          override_trait={token=gated delete_included_values={available}}}
      deleted={include=base delete_included_values={research_categories}}
      grandchild={include=child}
      copied_again={include=deleted}
      repeated={research_categories={cat_x} research_categories={cat_y}}
    "#);
    let base = db.tree("base").unwrap().unwrap();
    assert_eq!(base.research_categories, [b"cat_a".to_vec(), b"cat_b".to_vec()]);
    let gated = db.trait_definition("base", "gated").unwrap().unwrap();
    assert_eq!(gated.available()[0].value.scalar_str(), Some("no"));
    assert_eq!(gated.visible()[0].value.scalar_str(), Some("yes"));
    let open = db.trait_definition("base", "open").unwrap().unwrap();
    assert!(open.available().is_empty() && open.visible().is_empty());
    assert_eq!(
        db.tree("child").unwrap().unwrap().research_categories,
        [b"cat_a".to_vec(), b"cat_b".to_vec()]
    );
    let gated = db.trait_definition("child", "gated").unwrap().unwrap();
    assert_eq!(gated.available()[0].value.scalar_str(), Some("no"));
    assert_eq!(gated.visible()[0].value.scalar_str(), Some("no"));
    assert_eq!(
        db.tree("local").unwrap().unwrap().research_categories,
        [b"cat_c".to_vec()]
    );
    let gated = db.trait_definition("local", "gated").unwrap().unwrap();
    assert!(gated.available().is_empty());
    assert_eq!(gated.visible()[0].value.scalar_str(), Some("yes"));
    let copied = db.trait_definition("deleted", "gated").unwrap().unwrap();
    assert_eq!(copied.available()[0].value.scalar_str(), Some("no"));
    let again = db.trait_definition("copied_again", "gated").unwrap().unwrap();
    assert!(again.available().is_empty() && again.visible().is_empty());
    assert_eq!(
        db.tree("grandchild").unwrap().unwrap().research_categories,
        [b"cat_a".to_vec(), b"cat_b".to_vec()]
    );
    assert_eq!(
        db.tree("repeated").unwrap().unwrap().research_categories,
        [b"cat_x".to_vec(), b"cat_y".to_vec()]
    );
    assert!(
        db.tree("deleted")
            .unwrap()
            .unwrap()
            .research_categories
            .is_empty()
    );
}

fn catalog(input: &[u8]) -> Catalog {
    let (rows, warnings) = parse(input, &KeyNames::default(), &mut BTreeSet::new()).unwrap();
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
    content_reader_script::test_game::enter();
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
    content_reader_script::test_game::enter();
    let db = catalog(
        br#"
        child={include=middle}
        middle={include=base}
        base={initial_trait={name=FIRST_INITIAL} trait={token=kept name=OLD}}
        BASE={initial_trait={name=SKIPPED_INITIAL} trait={token=skipped name=NEW}}
        after={include=middle}
    "#,
    );
    assert!(db.trait_definition("child", "kept").unwrap().is_none());
    assert!(db.tree("child").unwrap().unwrap().initial.is_none());
    for organisation in ["middle", "after", "base"] {
        assert_eq!(
            db.trait_definition(organisation, "kept")
                .unwrap()
                .unwrap()
                .name,
            "OLD",
            "{organisation}"
        );
        assert!(
            db.trait_definition(organisation, "skipped")
                .unwrap()
                .is_none(),
            "{organisation}"
        );
        assert_eq!(
            db.tree(organisation).unwrap().unwrap().initial.unwrap().name,
            "FIRST_INITIAL",
            "{organisation}"
        );
    }
    assert!(
        db.warnings
            .iter()
            .any(|why| why.contains("MIO BASE at line") && why.contains("duplicate definition skipped"))
    );
}

#[test]
fn a_later_file_cannot_redefine_an_organisation() {
    content_reader_script::test_game::enter();
    let mut seen = BTreeSet::new();
    let mut db = Catalog::default();
    for input in [
        &b"org={trait={token=first name=FIRST}}"[..],
        b"ORG={trait={token=second name=SECOND}} other={include=org}",
    ] {
        let (rows, warnings) = parse(input, &KeyNames::default(), &mut seen).unwrap();
        db.warnings.extend(warnings);
        for row in rows {
            db.insert(row);
        }
    }
    db.finalize();
    for organisation in ["org", "other"] {
        assert!(db.trait_definition(organisation, "first").unwrap().is_some());
        assert!(db.trait_definition(organisation, "second").unwrap().is_none());
    }
    assert!(db.warnings.iter().any(|why| why.contains("duplicate definition skipped")));
}

#[test]
fn mio_trait_name_candidates_preserve_recorded_case_and_country_fallback() {
    content_reader_script::test_game::enter();
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
    content_reader_script::test_game::enter();
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
    assert_eq!(
        db.trait_definition("custom", "third")
            .unwrap()
            .unwrap()
            .name,
        "FIRST"
    );
    assert!(db.warnings.iter().any(|why| why.contains("already taken")));
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
            parse(input, &KeyNames::default(), &mut BTreeSet::new())
                .unwrap_err()
                .to_string()
                .contains("integer-point token recovery")
        );
    }
    let (rows, _) = parse(
        b"custom={trait={token=bad equipment_bonus={unknown={@name=LEAK}}} trait={token=next name=@name}}",
        &KeyNames::default(),
        &mut BTreeSet::new(),
    ).unwrap();
    assert!(rows[0].traits[0].bonuses.is_empty());
    assert_eq!(rows[0].traits[1].name, "@name");
}

#[test]
fn mio_trait_bonuses_follow_duplicate_category_and_include_rules() {
    content_reader_script::test_game::enter();
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
    assert_eq!(
        child
            .resolved_organisation_modifiers(|_| Ok(true))
            .unwrap()
            .values[0]
            .value,
        40_000
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
    assert_eq!(
        copied
            .resolved_organisation_modifiers(|_| Ok(true))
            .unwrap()
            .values[0]
            .value,
        40_000
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
    assert_eq!(
        zero.resolved_organisation_modifiers(|_| Ok(true))
            .unwrap()
            .values[0]
            .value,
        0
    );
    assert_eq!(
        zero.resolved_organisation_modifiers(|_| Ok(false))
            .unwrap()
            .values[0]
            .value,
        40_000
    );
}

#[test]
fn mio_trait_content_layers_replace_templates_and_keep_unread_suffix_named() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_mio_traits_{}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let base = root.join("base");
    let overlay = root.join("overlay");
    let directory = "common/military_industrial_organization/organizations";
    for folder in [&base, &overlay] {
        std::fs::create_dir_all(folder.join(directory)).unwrap();
    }
    std::fs::write(
        base.join(directory).join("a.txt"),
        b"custom={trait={token=old name=OLD}}",
    )
    .unwrap();
    std::fs::write(
        base.join(directory).join("b.txt"),
        b"removed={trait={token=old name=OLD}}",
    )
    .unwrap();
    std::fs::write(
        overlay.join(directory).join("a.txt"),
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
    assert_eq!(
        db.trait_definition("removed", "old").unwrap().unwrap().name,
        "OLD"
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
        b"malformed={trait={token=bad position={1 2}}}",
    )
    .unwrap();
    let layers =
        ResolvedLayers::from_test_common_folders(&[(&base, &[]), (&overlay, &[])]).unwrap();
    let why = read(&layers)
        .unwrap()
        .trait_definition("malformed", "bad")
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

#[test]
fn mio_tree_retains_native_positions_groups_and_initial_trait_inheritance() {
    content_reader_script::test_game::enter();
    let db = catalog(
        br#"
        base={initial_trait={name=INITIAL position={x=3 y=4}}
            trait={token=anchor icon=GFX_SYNTHETIC position={x=-2 y=7}}
            trait={token=leaf position={x=5 y=2} relative_position_id=anchor
                parent={traits={a b a} num_parents_needed=2}
                any_parent={c d} all_parents={e f}
                mutually_exclusive={other other} limit_to_equipment_type={synthetic_group}
                special_trait_background=yes}}
        child={include=base override_trait={token=leaf position={x=-1 y=-1}}}
    "#,
    );
    let tree = db.tree("child").unwrap().unwrap();
    assert_eq!(tree.initial.unwrap().local_position(), [3, 4]);
    let leaf = db.trait_definition("child", "leaf").unwrap().unwrap();
    assert_eq!(leaf.local_position(), [5, 2]);
    assert_eq!(db.position("child", "leaf").unwrap(), Some([3, 9]));
    assert_eq!(
        leaf.parents,
        vec![
            ParentGroup {
                traits: vec!["a".into(), "b".into(), "a".into()],
                required: 2
            },
            ParentGroup {
                traits: vec!["c".into(), "d".into()],
                required: 1
            },
            ParentGroup {
                traits: vec!["e".into(), "f".into()],
                required: 2
            },
        ]
    );
    assert_eq!(leaf.mutually_exclusive, ["other", "other"]);
    assert_eq!(leaf.equipment_types, vec![b"synthetic_group".to_vec()]);
    assert!(leaf.special_background);
}
#[test]
fn mio_tree_missing_anchors_return_zero_and_cycles_are_named() {
    content_reader_script::test_game::enter();
    let db = catalog(
        br#"m={trait={token=missing relative_position_id=absent position={x=9 y=8}}
        trait={token=outer relative_position_id=missing position={x=3 y=2}}
        trait={token=a relative_position_id=b} trait={token=b relative_position_id=a}}"#,
    );
    assert_eq!(db.position("m", "missing").unwrap(), Some([0, 0]));
    assert_eq!(db.position("m", "outer").unwrap(), Some([3, 2]));
    assert!(
        db.position("m", "a")
            .unwrap_err()
            .contains("relative-position cycle (count: 1)")
    );
}

#[test]
fn modifier_inheritance_is_lazy_and_preserves_each_copy_boundary() {
    content_reader_script::test_game::enter();
    let db = catalog(br#"
      base={trait={token=t organization_modifier={political_power_gain=0.4}}}
      child={include=base override_trait={token=t organization_modifier={political_power_gain=0}}}
      copied={include=child}
      deleted={include=base override_trait={token=t delete_included_values={organization_modifier}}}
      tooltip={include=base override_trait={token=t organization_modifier={custom_modifier_tooltip=TIP}}}
      nested={include=base override_trait={token=t organization_modifier={added_modifier={political_power_gain=0}}}}
    "#);
    let copied = db.trait_definition("copied", "t").unwrap().unwrap();
    assert_eq!(
        copied
            .resolved_organisation_modifiers(|_| Ok(true))
            .unwrap()
            .values[0]
            .value,
        0
    );
    assert_eq!(
        copied
            .resolved_organisation_modifiers(|_| Ok(false))
            .unwrap()
            .values[0]
            .value,
        40_000
    );
    assert!(
        copied
            .resolved_organisation_modifiers(|name| Err(format!("missing {name}")))
            .unwrap_err()
            .contains("political_power_gain")
    );
    for name in ["deleted", "tooltip", "nested"] {
        let row = db.trait_definition(name, "t").unwrap().unwrap();
        let resolved = row
            .resolved_organisation_modifiers(|_| panic!("unreached eligibility"))
            .unwrap();
        assert!(resolved.values.is_empty());
    }
}
