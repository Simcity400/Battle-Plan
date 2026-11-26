use super::*;
use dashboard_core::test_fixtures::game;
use replay_data::{rows::LexicalTokenRow, rows::TemplateSubunit};

fn content() -> Content {
    content_with(b"")
}

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
        index: Arc::new(SourceIndex {
            links: [(10, 7)].into(),
            local: local.iter().cloned().collect(),
            local_complete: true,
        }),
    }
}

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
    game::enter();
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
    assert_eq!(keys("rifles"), ["infantry", "armor", "support", "militia"]);
    assert_eq!(keys("tanks"), ["armor"]);
    assert!(keys("rifles_1").is_empty());
    assert!(content.archetype_subunits("zeppelins").is_err());
}

#[test]
fn a_need_naming_no_equipment_is_dropped() {
    game::enter();
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
    game::enter();
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
        "variant 1 equipment type: token 105 is not in lexical tokens"
    );
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
    let subunit = read_subunit("x", None, fields).unwrap();
    assert_eq!(subunit.requirements, [("rifles".to_owned(), 12_345)]);
}

#[test]
fn duplicate_archetypes_copy_model_inputs_and_remap_the_parent_chain() {
    game::enter();
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
            amphibious = { archetype = tanks type = { armor train } module_slots = none }
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
        ["armor", "flame", "artillery"].map(content_reader_script::test_game::category)
            .into_iter().fold(0, |bits, category| bits | category)
    );
    assert_eq!(
        content.equipment_category("amphibious_0").unwrap(),
        content_reader_script::test_game::category("armor") | content_reader_script::test_game::category("train")
    );
    let outside = content.equipment_content().get("flameold_tank").unwrap();
    assert_eq!(content.equipment("flameold_tank").unwrap().parent, outside);
    assert_eq!(
        content.equipment_model("tanks_1").unwrap().family,
        "tanks_1"
    );
    assert!(content.equipment_model("root_only").is_ok());
    assert!(content.equipment_model("root_only_0").is_err());
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
    game::enter();
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
    game::enter();
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
    let unknown = content_with(b"sub_units = { bad = { type = 4 } }");
    assert_eq!(unknown.subunit("bad").unwrap().category, 0);
}

#[test]
fn local_clones_group_apart_and_take_their_recorded_requirements() {
    game::enter();
    let content = content();
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
    let equal = content.template(&row((1, 1), &[]), &sources(&[])).unwrap();
    assert_eq!(
        Template::first_requirement(equal.representative.as_ref().unwrap()),
        Some("rifles")
    );
    let complete = sources(&[]);
    let incomplete = Sources {
        index: Arc::new(SourceIndex {
            links: complete.index.links.clone(),
            local: complete.index.local.clone(),
            local_complete: false,
        }),
        ..complete
    };
    assert!(content.template(&row((1, 1), &[]), &incomplete).is_err());
    let repeated = sources(&[((7, 1), vec![(4, 1), (6, 2)])]);
    assert!(content.template(&row((1, 1), &[]), &repeated).is_err());
}

#[test]
fn sub_unit_and_requirement_tokens_resolve_under_the_lexers_fold() {
    game::enter();
    let content = content();
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
        "sub-unit: token 3 is not in lexical tokens"
    );
    let redeclared = content_with(
        br#"
        sub_units = { ARMOR = { type = armor need = { tanks = 50 } } }
        equipments = { TANKS = { is_archetype = yes type = armor } }
    "#,
    );
    assert_eq!(redeclared.order, ["infantry", "armor", "support"]);
    let template = redeclared
        .template(&row((1, 0), &[(2, 0, 1)]), &folded(&[]))
        .unwrap();
    assert_eq!(template.groups.len(), 1);
    assert_eq!(template.groups[0].subunit.key, "armor");
    assert_eq!(
        template.groups[0].requirements.as_ref(),
        &[("tanks".to_owned(), 5_000_000)]
    );
    let redefined = redeclared.template(&row((1, 1), &[]), &local).unwrap();
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
    write(
        "mod",
        "common/units/infantry.txt",
        "sub_units = { infantry = { } Railway_Gun = { } }",
    );
    write(
        "mod",
        "common/units/unit_modifiers/modifiers.txt",
        "SUB_UNITS = { modified = { } }",
    );
    write(
        "mod",
        "common/unit_tags/tags.txt",
        "sub_unit_categories = { category_synthetic } sub_units = { tagged = { } }",
    );
    write(
        "mod",
        "common/units/critical_parts/parts.txt",
        "sub_units = { parted = { } }",
    );
    write(
        "mod",
        "common/units/quoted.txt",
        "\"sub_units\" = { quoted_root = { } }",
    );
    let layers = ResolvedLayers::from_folders(&[
        (&root.join("base"), &[][..]),
        (&root.join("mod"), &[][..]),
    ])
    .unwrap();
    assert_eq!(
        subunit_keys(&layers).unwrap(),
        BTreeSet::from(
            ["Railway_Gun", "armor", "infantry", "modified", "parted", "tagged"]
                .map(str::to_owned)
        )
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_mod_sub_unit_reads_its_stat_sheet() {
    let content = content_with(
        br#"
        sub_units = {
            synthetic_ranger = {
                type = infantry
                group = support
                categories = { category_synthetic category_line }
                essential = { rifles }
                combat_width = 1.5
                manpower = 750
                training_time = 120
                night_penalty = -0.25
                soft_attack = 0.1
                soft_attack = -0.2
                can_be_parachuted = yes
                battalion_mult = { category = category_line add = yes entrenchment = 0.5 }
                need = { rifles = 50 }
            }
            synthetic_broken = {
                can_be_parachuted = maybe
                battalion_mult = { category = category_line synthetic_unknown = 1 }
            }
        }"#,
    );
    let ranger = content.subunit("synthetic_ranger").unwrap();
    let sheet = &ranger.sheet;
    assert_eq!(sheet.combat_width, 150_000);
    assert_eq!((sheet.manpower, sheet.training_time), (750, 120));
    assert_eq!(sheet.group.as_deref(), Some("support"));
    assert_eq!(sheet.categories, ["category_synthetic", "category_line"]);
    assert_eq!(sheet.essential, ["rifles"]);
    let words: Vec<(&str, &str)> = sheet
        .words
        .iter()
        .map(|(key, text)| (key.as_str(), text.as_str()))
        .collect();
    assert_eq!(
        words,
        [
            ("night_penalty", "-0.25"),
            ("soft_attack", "0.1"),
            ("soft_attack", "-0.2")
        ]
    );
    assert_eq!(
        sheet.grants,
        [Grant {
            category: "category_line".into(),
            add: true,
            words: vec![("entrenchment".into(), "0.5".into())],
        }]
    );
    assert!(sheet.refused.is_empty(), "{:?}", sheet.refused);
    assert_eq!(ranger.can_be_parachuted, Ok(true));
    let broken = content.subunit("synthetic_broken").unwrap();
    assert!(broken.can_be_parachuted.is_err());
    assert_eq!(
        broken.sheet.grants[0].words,
        [("synthetic_unknown".to_owned(), "1".to_owned())]
    );
    assert!(content.subunit("SYNTHETIC_RANGER").is_some());
}

#[test]
fn a_quoted_type_sets_no_category() {
    let content = content_with(
        br#"sub_units = {
            quoted = { type = "support" }
            quoted_list = { type = { "support" infantry } }
            bare = { type = SUPPORT }
        }"#,
    );
    let support = content_reader_script::equipment_stats::category_mask("support").unwrap();
    let infantry = content_reader_script::equipment_stats::category_mask("infantry").unwrap();
    assert_eq!(content.subunit("quoted").unwrap().category, 0);
    assert_eq!(content.subunit("quoted_list").unwrap().category, infantry);
    assert_eq!(content.subunit("bare").unwrap().category, support);
}

#[test]
fn a_redefinition_keeps_what_the_reset_leaves() {
    let content = content_with(
        br#"sub_units = {
            ranger = {
                type = { infantry support }
                group = support
                division_3d_model_priority = 7
                categories = { category_line }
                essential = { rifles }
                soft_attack = 0.5
                need = { rifles = 10 }
            }
        }
        sub_units = {
            ranger = {
                categories = { category_extra CATEGORY_LINE "category_quoted" }
                essential = { Rifles tanks }
                need = { tanks = 5 }
            }
        }"#,
    );
    let ranger = content.subunit("ranger").unwrap();
    assert_eq!(ranger.sheet.group.as_deref(), Some("support"));
    assert_eq!(ranger.model_priority, Some(7));
    assert_eq!(ranger.sheet.categories, ["category_line", "category_extra"]);
    assert_eq!(ranger.sheet.essential, ["rifles", "tanks"]);
    assert_eq!(ranger.category, 0);
    assert!(ranger.sheet.words.is_empty());
    assert_eq!(ranger.requirements, [("tanks".to_owned(), 500_000)]);
    let regrouped = content_with(
        b"sub_units = { ranger = { group = support } } sub_units = { ranger = { group = mobile } }",
    );
    assert_eq!(
        regrouped.subunit("ranger").unwrap().sheet.group.as_deref(),
        Some("mobile")
    );
}

#[test]
fn a_redefinition_keeps_transport_fuel_and_kept_words_and_adds_battalion_multipliers() {
    let content = content_with(
        br#"sub_units = {
            ranger = {
                transport = motorized
                own_equipment_fuel_consumption_mult = 0.5
                affects_speed = no
                hit_profile_mult = 0.75
                same_support_type = recon
                soft_attack = 0.5
                combat_width = 2
                manpower = 1000
                battalion_mult = { category = category_line add = yes entrenchment = 0.5 }
            }
        }
        sub_units = {
            ranger = {
                defense = 3
                same_support_type = scouts
                battalion_mult = { category = category_line add = yes entrenchment = 0.5 }
            }
        }"#,
    );
    let ranger = content.subunit("ranger").unwrap();
    assert_eq!(ranger.sheet.transport.as_deref(), Some("motorized"));
    assert_eq!(ranger.sheet.own_fuel_consumption, Some(50_000));
    let words: Vec<(&str, &str)> = ranger
        .sheet
        .words
        .iter()
        .map(|(word, value)| (word.as_str(), value.as_str()))
        .collect();
    assert_eq!(
        words,
        [
            ("affects_speed", "no"),
            ("hit_profile_mult", "0.75"),
            ("same_support_type", "recon"),
            ("defense", "3"),
            ("same_support_type", "scouts"),
        ]
    );
    assert_eq!(ranger.sheet.combat_width, 0);
    assert_eq!(ranger.sheet.manpower, 0);
    assert_eq!(ranger.sheet.grants.len(), 2);
    let refused = content_with(
        br#"sub_units = { ranger = { battalion_mult = yes combat_width = wide } }
        sub_units = { ranger = { combat_width = 1 } }"#,
    );
    let ranger = refused.subunit("ranger").unwrap();
    let whys = |subunit: &SubUnit| subunit.sheet.refused.iter().map(|refusal| refusal.why.clone()).collect::<Vec<_>>();
    assert_eq!(whys(ranger), ["sub-unit ranger: battalion_mult is not a block"]);
    let mut entry = content_with(b"");
    for document in [&b"sub_units = { ranger = { categories = { \xff } } }"[..], b"sub_units = { ranger = { } }"] {
        entry.read_subunits(&parser::parse(document).unwrap()).unwrap();
    }
    assert_eq!(whys(entry.subunit("ranger").unwrap()), ["sub-unit ranger: a categories entry is not UTF-8"]);
    let fuel = |second: &[u8]| {
        let mut source = b"sub_units = { ranger = { own_equipment_fuel_consumption_mult = broken } } ".to_vec();
        source.extend_from_slice(second);
        content_with(&source)
    };
    let unassigned = fuel(b"sub_units = { ranger = { } }");
    assert_eq!(whys(unassigned.subunit("ranger").unwrap()), ["sub-unit ranger: own_equipment_fuel_consumption_mult is not a number"]);
    let corrected = fuel(b"sub_units = { ranger = { own_equipment_fuel_consumption_mult = 0.5 } }");
    let ranger = corrected.subunit("ranger").unwrap();
    assert!(ranger.sheet.refused.is_empty(), "{:?}", ranger.sheet.refused);
    assert_eq!(ranger.sheet.own_fuel_consumption, Some(50_000));
    let speed = content_with(
        b"sub_units = { ranger = { affects_speed = invalid } } sub_units = { ranger = { affects_speed = no } }",
    );
    assert_eq!(speed.subunit("ranger").unwrap().sheet.words, [("affects_speed".to_owned(), "no".to_owned())]);
}

#[test]
fn a_case_only_redefinition_is_the_same_sub_unit() {
    let content = content_with(
        b"sub_units = { ranger = { need = { rifles = 10 } combat_width = 1 categories = { a } } } \
          sub_units = { RANGER = { need = { tanks = 5 } categories = { b } } }",
    );
    let ranger = content.subunit("Ranger").unwrap();
    assert_eq!(ranger.key, "ranger");
    assert_eq!(ranger.requirements, [("tanks".to_owned(), 500_000)]);
    assert_eq!(ranger.sheet.combat_width, 0);
    assert_eq!(ranger.sheet.categories, ["a", "b"]);
    assert_eq!(content.len(), 4);
    let tokens = [LexicalTokenRow {
        token: 7,
        literal: b"RANGER".to_vec(),
    }];
    let named = content
        .subunit_of_token(TokenNames::from_rows(&tokens, true), 7)
        .unwrap();
    assert!(std::ptr::eq(named, ranger));
    let armored: Vec<_> = content
        .archetype_subunits("tanks")
        .unwrap()
        .iter()
        .map(|unit| unit.key.as_str())
        .collect();
    assert_eq!(armored, ["armor", "ranger"]);
}

#[test]
fn sub_unit_fields_dispatch_by_keyword() {
    let content = content_with(
        br#"Sub_Units = {
            cased = { NEED = { rifles = 100 } Combat_Width = 2 SOFT_ATTACK = 0.5 }
            quoted = { "need" = { rifles = 100 } "combat_width" = 2 }
        }"#,
    );
    let cased = content.subunit("cased").unwrap();
    assert_eq!(cased.requirements, [("rifles".to_owned(), 10_000_000)]);
    assert_eq!(cased.sheet.combat_width, 200_000);
    assert_eq!(
        cased.sheet.words,
        [("soft_attack".to_owned(), "0.5".to_owned())]
    );
    let quoted = content.subunit("quoted").unwrap();
    assert!(quoted.requirements.is_empty());
    assert_eq!(quoted.sheet.combat_width, 0);
    assert!(quoted.sheet.words.is_empty());
}

#[test]
fn content_reads_every_sub_unit_database_directory_in_order() {
    game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_viewer_subunit_directories_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in [
        (
            "common/units/equipment/rifles.txt",
            "equipments = { rifles = { is_archetype = yes type = infantry } }",
        ),
        (
            "common/units/a_line.txt",
            "sub_units = { early = { need = { rifles = 1 } } line = { need = { rifles = 2 } } }",
        ),
        (
            "common/units/unit_modifiers/modifiers.txt",
            "sub_units = { early = { group = support need = { rifles = 9 } } }",
        ),
        (
            "common/unit_tags/tags.txt",
            "sub_unit_categories = { category_synthetic } sub_units = { tagged = { need = { rifles = 3 } } }",
        ),
        (
            "common/units/critical_parts/parts.txt",
            "sub_units = { parted = { need = { rifles = 4 } } }",
        ),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let content = Content::load(&layers);
    std::fs::remove_dir_all(&root).unwrap();
    let content = content.unwrap();
    let keys: Vec<_> = content
        .archetype_subunits("rifles")
        .unwrap()
        .iter()
        .map(|unit| unit.key.clone())
        .collect();
    assert_eq!(keys, ["early", "tagged", "parted", "line"]);
    let early = content.subunit("early").unwrap();
    assert_eq!(early.sheet.group.as_deref(), Some("support"));
    assert_eq!(early.requirements, [("rifles".to_owned(), 100_000)]);
    assert_eq!(
        content.subunit("tagged").unwrap().requirements,
        [("rifles".to_owned(), 300_000)]
    );
}
