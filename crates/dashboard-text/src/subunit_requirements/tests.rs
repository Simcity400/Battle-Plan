use super::*;
use replay_data::{rows::LexicalTokenRow, rows::TemplateSubunit};

fn content() -> Content {
    content_with(b"")
}

/// [`content`] with `extra` read after it, as a later file would be.
fn content_with(extra: &[u8]) -> Content {
    from_documents(&[
        &br#"
        sub_units = {
            infantry = { type = infantry need = { rifles = 100 } }
            armor = { type = armor need = { tanks = 50 rifles = 20 } }
            support = { type = { infantry support }
                division_3d_model_priority = 99999 need = { rifles = 20 } }
        }
        equipments = {
            rifles = { is_archetype = yes type = infantry }
            tanks = { is_archetype = yes type = armor }
            rifles_1 = { archetype = rifles }
            tanks_1 = { archetype = tanks }
            tanks_2 = { archetype = tanks parent = tanks_1 visual_tech_level_addition = 2 }
        }
    "#[..],
        extra,
    ])
}

/// Sub-units and equipment from documents read in order, as
/// [`Content::load`] reads the files.
fn from_documents(documents: &[&[u8]]) -> Content {
    Content::from_test_documents(documents)
}

type Overrides = [((u32, u32), Vec<(u32, i64)>)];

fn sources(local: &Overrides) -> Sources<'static> {
    let tokens: &'static [LexicalTokenRow] = Box::leak(
        [
            &b"infantry"[..],
            b"armor",
            b"support",
            b"rifles",
            b"tanks",
            b"rifles_1",
        ]
        .iter()
        .zip(1..)
        .map(|(literal, token)| LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice(),
    );
    Sources {
        tokens: TokenNames::from_rows(tokens, true),
        links: [(10, 7)].into(),
        local: local.iter().cloned().collect(),
        local_complete: true,
    }
}

/// Equipment type spellings for token resolution.
fn names() -> TokenNames<'static> {
    let rows: &'static [LexicalTokenRow] = Box::leak(
        [
            (101, &b"rifles_1"[..]),
            (102, b"Tanks_1"),
            (103, b"tanks_2"),
            (104, b"trucks"),
        ]
        .into_iter()
        .map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice(),
    );
    TokenNames::from_rows(rows, true)
}

fn row(representative: (u32, u8), subunits: &[(u32, u8, i32)]) -> DivisionTemplateIconRow {
    DivisionTemplateIconRow {
        country: 10,
        representative: representative.0,
        representative_local: representative.1,
        subunits: subunits
            .iter()
            .map(|&(token, local, count)| TemplateSubunit {
                token,
                local,
                count,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn an_archetype_lists_the_subunits_needing_it_in_database_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content =
        content_with(b"sub_units = { militia = { type = infantry need = { rifles_1 = 5 } } }");
    let keys = |key: &str| -> Vec<String> {
        content
            .archetype_subunits(key)
            .unwrap()
            .iter()
            .map(|unit| unit.key.clone())
            .collect()
    };
    // Authored order, not key order; a need of a rifles type counts as
    // rifles, the canonical archetype.
    assert_eq!(keys("rifles"), ["infantry", "armor", "support", "militia"]);
    assert_eq!(keys("tanks"), ["armor"]);
    // Only archetypes carry the list.
    assert!(keys("rifles_1").is_empty());
    assert!(content.archetype_subunits("zeppelins").is_err());
}

#[test]
fn a_need_naming_no_equipment_is_dropped() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = content_with(
        b"sub_units = { police = { type = infantry need = { motorbikes = 20 rifles_1 = 5 } } }",
    );
    assert_eq!(
        content.subunit("police").unwrap().requirements,
        [("rifles".to_owned(), 500_000)]
    );
    assert_eq!(
        content.dropped,
        ["sub-unit police needs motorbikes, which is no equipment"]
    );
}

#[test]
fn equipment_types_resolve_their_token_under_the_lexers_fold() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let authored = content();
    let names = names();
    let variant = |equipment_token| VariantDefinitionRow {
        id: 1,
        equipment_token,
        ..Default::default()
    };
    assert_eq!(authored.equipment_key(&names, &variant(102)), Ok("tanks_1"));
    let refused = |token| authored.equipment_key(&names, &variant(token)).unwrap_err();
    assert_eq!(
        refused(104),
        "variant 1 equipment type trucks is not authored equipment"
    );
    assert_eq!(
        refused(0),
        "variant 1 equipment type: token 0 stands for every key the lexer rejects"
    );
    assert_eq!(
        refused(105),
        "variant 1 equipment type: token 105 is not in LexicalTokens"
    );
    // A later key equal under the fold redefines the same type
    // (lookup uses its hash and case-insensitive name), which keeps its
    // first spelling.
    let redefined = content_with(b"equipments = { Tanks_1 = { archetype = rifles } }");
    assert_eq!(
        redefined.equipment_key(&names, &variant(102)),
        Ok("tanks_1")
    );
    assert_eq!(redefined.archetype("TANKS_1").unwrap(), "rifles");
}

#[test]
fn an_authored_need_past_five_decimals_truncates_like_the_engine() {
    let document = parser::parse(b"x = { need = { rifles = 0.1234567 } }").unwrap();
    let fields = document.items[0].value.items().unwrap();
    let subunit = read_subunit("x", fields).unwrap();
    assert_eq!(subunit.requirements, [("rifles".to_owned(), 12_345)]);
}

#[test]
fn duplicate_archetypes_copy_model_inputs_and_remap_the_parent_chain() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = from_documents(&[br#"
        equipment_modules = { turret = { add_equipment_type = artillery } }
        equipments = {
            tanks = { is_archetype = yes type = armor model = tank_mesh }
            tanks_0 = { archetype = tanks visual_level = 3 model = tier_mesh
                default_modules = { weapon = turret } }
            tanks_1 = { archetype = tanks parent = tanks_0 visual_tech_level_addition = 2 }
            infantry = { is_archetype = yes type = infantry }
            old_tank = { archetype = tanks parent = infantry }
        }
        duplicate_archetypes = {
            flame = { archetype = tanks type = { armor flame } }
            amphibious = { archetype = tanks type = { armor amphibious } module_slots = none }
            root_only = { archetype = tanks type = armor only_duplicate_archetype = yes }
        }
    "#]);
    let model = content.equipment_model("flame_1").unwrap();
    assert_eq!(model.archetype, "flame");
    assert_eq!(model.family, "tanks_1");
    assert_eq!((model.model_priority, model.depth), (3, 1));
    let first = content.equipment_model("flame_0").unwrap();
    assert_eq!((first.visual_level, first.model), (3, "tier_mesh"));
    assert_eq!(
        content.equipment_category("flame_0").unwrap(),
        (1 << 2) | (1 << 25) | (1 << 28)
    );
    assert_eq!(
        content.equipment_category("amphibious_0").unwrap(),
        (1 << 2) | (1 << 26)
    );
    // An original parent outside the cloned family becomes a native self-link.
    let outside = content.equipment_content().get("flameold_tank").unwrap();
    assert_eq!(content.equipment("flameold_tank").unwrap().parent, outside);
    // An authored type is its own family.
    assert_eq!(
        content.equipment_model("tanks_1").unwrap().family,
        "tanks_1"
    );
    assert!(content.equipment_model("root_only").is_ok());
    assert!(content.equipment_model("root_only_0").is_err());
    // A generated copy is authored equipment its own token names.
    let rows = [LexicalTokenRow {
        token: 7,
        literal: b"Flame_1".to_vec(),
    }];
    let variant = VariantDefinitionRow {
        id: 1,
        equipment_token: 7,
        ..Default::default()
    };
    assert_eq!(
        content.equipment_key(&TokenNames::from_rows(&rows, true), &variant),
        Ok("flame_1")
    );
}

#[test]
fn enabled_module_slots_follow_copied_slots_and_effective_category() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = from_documents(&[br#"
        equipments = {
            tanks = { is_archetype = yes type = armor
                module_slots = { main = {} secondary = main } }
            empty = { is_archetype = yes type = infantry }
            tanks_0 = { archetype = tanks module_slots = inherit }
            tanks_empty_parent = { archetype = tanks parent = empty module_slots = inherit }
        }
        duplicate_archetypes = { legacy = { archetype = tanks type = armor module_slots = none } }
    "#]);
    for key in ["tanks", "tanks_0"] {
        assert!(
            content.has_enabled_module_slots(key, 1 << 2).unwrap(),
            "{key}"
        );
        assert!(
            !content.has_enabled_module_slots(key, 1 << 5).unwrap(),
            "{key}"
        );
    }
    for key in ["empty", "tanks_empty_parent", "legacy", "legacy_0"] {
        assert!(
            !content.has_enabled_module_slots(key, u64::MAX).unwrap(),
            "{key}"
        );
    }
    assert!(
        content
            .has_enabled_module_slots("missing", u64::MAX)
            .is_err()
    );
}

#[test]
fn requirement_aliases_overwrite_in_source_order_after_archetype_resolution() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = content_with(
        b"sub_units = { aliases = { need = { rifles = 1 tanks = 9 rifles_1 = 2 rifles = 3 } } }",
    );
    assert_eq!(
        content.subunit("aliases").unwrap().requirements,
        [
            ("rifles".to_owned(), 300_000),
            ("tanks".to_owned(), 900_000)
        ]
    );
    assert!(
        Content::default()
            .read_subunits(&parser::parse(b"sub_units = { bad = { type = 4 } }").unwrap())
            .is_err()
    );
}

#[test]
fn local_clones_group_apart_and_take_their_recorded_requirements() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = content();
    // Local infantry needs tanks, not rifles.
    let local = sources(&[((7, 1), vec![(5, 200_000)])]);
    let template = content
        .template(&row((1, 1), &[(1, 0, 1), (1, 1, 1), (2, 0, 1)]), &local)
        .unwrap();
    assert_eq!(
        template.groups[1].identity,
        Definition::Local {
            country: 7,
            token: 1
        }
    );
    let representative = template.representative.as_ref().unwrap();
    assert_eq!(Template::first_requirement(representative), Some("tanks"));
    // A clone absent from a complete table has the global requirements;
    // from an incomplete one it is unknown.
    let equal = content.template(&row((1, 1), &[]), &sources(&[])).unwrap();
    assert_eq!(
        Template::first_requirement(equal.representative.as_ref().unwrap()),
        Some("rifles")
    );
    let incomplete = Sources {
        local_complete: false,
        ..sources(&[])
    };
    assert!(content.template(&row((1, 1), &[]), &incomplete).is_err());
    // Two tokens resolving to one archetype cannot come from a native pool.
    let repeated = sources(&[((7, 1), vec![(4, 1), (6, 2)])]);
    assert!(content.template(&row((1, 1), &[]), &repeated).is_err());
}

#[test]
fn sub_unit_and_requirement_tokens_resolve_under_the_lexers_fold() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = content();
    // The hour spells each token as the lexer first saw it, which may
    // differ in ASCII case from the authored key.
    let rows: &'static [LexicalTokenRow] = Box::leak(Box::new([
        LexicalTokenRow {
            token: 1,
            literal: b"Infantry".to_vec(),
        },
        LexicalTokenRow {
            token: 2,
            literal: b"ARMOR".to_vec(),
        },
        LexicalTokenRow {
            token: 5,
            literal: b"Tanks".to_vec(),
        },
        LexicalTokenRow {
            token: 8,
            literal: b"trucks".to_vec(),
        },
    ]));
    let folded = |local: &Overrides| Sources {
        tokens: TokenNames::from_rows(rows, true),
        ..sources(local)
    };
    let local = folded(&[((7, 1), vec![(5, 200_000)])]);
    let template = content
        .template(&row((1, 1), &[(2, 0, 1)]), &local)
        .unwrap();
    assert_eq!(template.groups[0].subunit.key, "armor");
    let representative = template.representative.as_ref().unwrap();
    assert_eq!(representative.subunit.key, "infantry");
    assert_eq!(Template::first_requirement(representative), Some("tanks"));
    // A spelling no authored key folds to, or an absent token, is refused.
    let unknown = folded(&[((7, 1), vec![(8, 1)])]);
    assert_eq!(
        content
            .template(&row((1, 1), &[]), &unknown)
            .unwrap_err()
            .to_string(),
        "local sub-unit 7:1 requirement trucks is not authored equipment"
    );
    assert_eq!(
        content
            .template(&row((3, 0), &[]), &folded(&[]))
            .unwrap_err()
            .to_string(),
        "sub-unit: token 3 is not in LexicalTokens"
    );
    // Two authored sub-unit keys equal under the fold are refused by
    // name; two equipment keys are one type, the later redefining it
    let ambiguous = content_with(
        br#"
        sub_units = { ARMOR = { type = armor need = { tanks = 50 } } }
        equipments = { TANKS = { is_archetype = yes type = armor } }
    "#,
    );
    assert_eq!(
        ambiguous
            .template(&row((1, 0), &[(2, 0, 1)]), &folded(&[]))
            .unwrap_err()
            .to_string(),
        "token ARMOR names several content keys: ARMOR, armor"
    );
    let redefined = ambiguous.template(&row((1, 1), &[]), &local).unwrap();
    assert_eq!(
        Template::first_requirement(redefined.representative.as_ref().unwrap()),
        Some("tanks")
    );
}

#[test]
fn sub_unit_keys_come_from_the_effective_direct_unit_files() {
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_subunit_keys_{}_{:?}",
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
        "common/units/infantry.txt",
        "sub_units = { infantry = { } replaced = { } }",
    );
    write(
        "base",
        "common/units/armor.txt",
        "sub_units = { armor = { } }",
    );
    // Child directories hold equipment and unrelated data, not sub-units.
    write(
        "base",
        "common/units/equipment/tanks.txt",
        "sub_units = { equipment_child = { } } equipments = { tanks = { } }",
    );
    write(
        "base",
        "common/units/names_divisions/names.txt",
        "sub_units = { names_child = { } }",
    );
    // The mod's file of the same path replaces the base's whole.
    write(
        "mod",
        "common/units/infantry.txt",
        "sub_units = { infantry = { } Railway_Gun = { } }",
    );
    let layers = ResolvedLayers::from_folders(&[
        (&root.join("base"), &[][..]),
        (&root.join("mod"), &[][..]),
    ])
    .unwrap();
    assert_eq!(
        subunit_keys(&layers).unwrap(),
        BTreeSet::from(["Railway_Gun", "armor", "infantry"].map(str::to_owned))
    );
    std::fs::remove_dir_all(&root).unwrap();
}
