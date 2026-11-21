use crate::test_fixtures::{PRODUCTION_HOUR, equipment_types, production_frame};
use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};

fn chosen(
    content: &super::IconContent,
    hour: &super::IconHour,
    row: &replay_data::rows::VariantDefinitionRow,
    spelling: &str,
) -> Result<Option<super::Icon>, String> {
    let direct = super::variant_icon(content, hour, row, spelling);
    let walked =
        super::icon_choice(content, hour, row, spelling).walk(|probe| content.answer(probe));
    assert_eq!(walked, direct, "{spelling}");
    direct
}

#[test]
fn equipment_icon_warnings_name_every_refused_technology() {
    let refused: Vec<_> = (0..9).map(|entry| format!("technology_{entry}")).collect();
    let content = crate::products::equipment_icon::IconContent::from_parts(
        Default::default(),
        installed_content_core::technology::Enablers {
            refused: refused.clone(),
            ..Default::default()
        },
        Default::default(),
        Default::default(),
        Default::default(),
        Default::default(),
    );
    let [warning] = content.warnings().try_into().unwrap();
    assert!(warning.starts_with("9 technologies"));
    for reason in refused {
        assert!(warning.contains(&reason), "{warning}");
    }
}

#[test]
fn equipment_icons_keep_null_archetype_fallbacks() {
    use super::{Icon, IconContent, IconHour};
    use installed_content_core::{technology::Enabler, technology::Enablers};
    use installed_content_equipment_graphics::equipment_graphics;
    use replay_data::rows::VariantDefinitionRow;

    let units = equipment_types(&[br#"
        equipments = {
            bare_type = { }
            technology_type = { }
            family_member = { family = bare_type }
            pooled_type = { }
            undrawn_type = { }
        }
    "#])
    .unwrap();
    let enablers = Enablers {
        plain: [
            (
                "technology_type".to_owned(),
                vec![Enabler {
                    id: "mod_technology".into(),
                    parent: Ok(None),
                }],
            ),
            (
                String::new(),
                vec![Enabler {
                    id: "null_technology".into(),
                    parent: Ok(None),
                }],
            ),
        ]
        .into(),
        ..Default::default()
    };
    let make_content = |pools: &[&[u8]]| {
        IconContent::from_parts(
            units.clone(),
            enablers.clone(),
            equipment_graphics::Content::from_texts(
                pools,
                &equipment_graphics::Names {
                    countries: Default::default(),
                    continents: Default::default(),
                },
            )
            .unwrap(),
            Default::default(),
            [
                "GFX_bare_type_medium",
                "GFX_mod_technology_medium",
                "GFX_null_technology_medium",
                "GFX_pool_picture",
                "GFX_null_pool",
                "GFX__medium",
                "GFX__capital_medium",
            ]
            .map(str::to_owned)
            .into(),
            Default::default(),
        )
    };
    let content = make_content(&[br#"
        default = { pooled_type = { pool = { icons = { GFX_pool_picture } } } }
    "#]);
    let frame = production_frame(PRODUCTION_HOUR, true, true, 9001, &[], Some(&[]));
    let hour = IconHour::read(&frame, &content).unwrap();
    let row = VariantDefinitionRow::default();
    for (token, expected) in [
        ("bare_type", "GFX_bare_type_medium"),
        ("technology_type", "GFX_mod_technology_medium"),
        ("family_member", "GFX_bare_type_medium"),
        ("pooled_type", "GFX_pool_picture"),
    ] {
        assert_eq!(
            chosen(&content, &hour, &row, token),
            Ok(Some(Icon::Sprite(expected.into()))),
            "{token}"
        );
    }
    let capital = VariantDefinitionRow {
        classifier_flags: 1 << 6,
        ..Default::default()
    };
    for row in [&row, &capital] {
        assert_eq!(chosen(&content, &hour, row, "undrawn_type"), Ok(None));
    }
    let auto = VariantDefinitionRow {
        icon_active: 1,
        icon_kind: 1,
        icon: b".".to_vec(),
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &auto, "pooled_type"),
        Ok(Some(Icon::Sprite("GFX_pool_picture".into())))
    );
    let null_pool = make_content(&[br#"
        default = { "" = { pool = { icons = { GFX_null_pool } } } }
    "#]);
    assert_eq!(
        chosen(&null_pool, &hour, &row, "undrawn_type"),
        Ok(None)
    );
    let reason = chosen(&content, &hour, &row, "undeclared_type").unwrap_err();
    assert!(reason.contains("undeclared_type"), "{reason}");
}

#[test]
fn equipment_icons_distinguish_real_empty_names_from_null_archetypes() {
    use super::{Icon, IconContent, IconHour};
    use installed_content_equipment_graphics::equipment_graphics;
    use replay_data::rows::VariantDefinitionRow;

    let units = equipment_types(&[br#"
        equipments = {
            "" = { is_archetype = yes }
            empty_parent = { archetype = "" }
            family_member = { family = "" }
            bare_type = { }
        }
    "#])
    .unwrap();
    let content = IconContent::from_parts(
        units,
        Default::default(),
        equipment_graphics::Content::from_texts(
            &[br#"default = { "" = { pool = { icons = { GFX_empty_pool } } } }"#],
            &equipment_graphics::Names {
                countries: Default::default(),
                continents: Default::default(),
            },
        )
        .unwrap(),
        Default::default(),
        ["GFX_empty_pool".to_owned()].into(),
        Default::default(),
    );
    let frame = production_frame(PRODUCTION_HOUR, true, true, 9001, &[], Some(&[]));
    let hour = IconHour::read(&frame, &content).unwrap();
    let row = VariantDefinitionRow::default();
    for token in ["", "empty_parent", "family_member"] {
        assert_eq!(
            chosen(&content, &hour, &row, token),
            Ok(Some(Icon::Sprite("GFX_empty_pool".into()))),
            "{token:?}"
        );
    }
    assert_eq!(chosen(&content, &hour, &row, "bare_type"), Ok(None));
}

#[test]
fn authored_equipment_icons_keep_null_archetype_candidates() {
    use super::{Icon, IconContent};
    use installed_content_core::{technology::Enabler, technology::Enablers};

    let content = IconContent::from_parts(
        equipment_types(&[b"equipments = { bare_type = { } }"]).unwrap(),
        Enablers {
            plain: [(
                "bare_type".to_owned(),
                vec![Enabler {
                    id: "mod_technology".into(),
                    parent: Ok(None),
                }],
            )]
            .into(),
            ..Default::default()
        },
        Default::default(),
        Default::default(),
        [
            "GFX_bare_type_medium",
            "GFX_mod_technology_medium",
            "GFX__medium",
        ]
        .map(str::to_owned)
        .into(),
        Default::default(),
    );
    let authored = content.authored_icons().unwrap();
    assert_eq!(authored.len(), 1);
    let authored = authored.into_iter().next().unwrap();
    assert_eq!(authored.token, "bare_type");
    assert!(authored.refusals.is_empty(), "{:?}", authored.refusals);
    assert_eq!(
        authored.icons,
        ["GFX_bare_type_medium", "GFX_mod_technology_medium"].map(|name| Icon::Sprite(name.into()))
    );
}

#[test]
fn equipment_icons_follow_the_production_rows_rule() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::equipment_icon::{Icon, IconContent, IconHour};
    use installed_content_core::{technology::Enabler, technology::Enablers};
    use installed_content_equipment_graphics::equipment_graphics;
    use replay_data::{rows::PoliticalPartyRow, rows::PoliticsRow, rows::VariantDefinitionRow};
    let units = equipment_types(&[br#"
        equipments = {
            tanks = { is_archetype = yes type = armor }
            tanks_1 = { archetype = tanks }
            rifles = { is_archetype = yes type = infantry }
            rifles_1 = { archetype = rifles }
        }
    "#])
    .unwrap();
    let names = equipment_graphics::Names {
        countries: ["GER".to_owned()].into(),
        continents: Default::default(),
    };
    let pools = equipment_graphics::Content::from_texts(
        &[br#"
            default = { tanks = { pool = { icons = { GFX_default_tank } } } }
            GER = { tanks_1 = { pool = { ideologies = { fascism } icons = { GFX_fascist_tank } } } }
        "#],
        &names,
    )
    .unwrap();
    let enablers = Enablers {
        plain: [(
            "rifles_1".to_owned(),
            vec![Enabler {
                id: "infantry_weapons1".into(),
                parent: Ok(Some("infantry_weapons".into())),
            }],
        )]
        .into(),
        ..Enablers::default()
    };
    let content = |sprites: &[&str]| {
        IconContent::from_parts(
            units.clone(),
            enablers.clone(),
            pools.clone(),
            Default::default(),
            sprites.iter().map(|name| (*name).to_owned()).collect(),
            ["gfx/designs/my_tank.dds".to_owned()].into(),
        )
    };
    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 9001, &[], Some(&[]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            [11, 21]
                .map(|country| replay_data::rows::CountryAppearanceRow {
                    country,
                    ..Default::default()
                })
                .to_vec(),
        ));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 11,
            is_ruling: 1,
            token: b"fascism".to_vec(),
            ..Default::default()
        }]));
    for table in [TableId::Factions, TableId::FactionMembers, TableId::States] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
        PoliticsRow {
            country: 21,
            ..Default::default()
        },
        PoliticsRow {
            country: 11,
            ..Default::default()
        },
    ]));
    let variant = |token: u32, active: u8, kind: u32, icon: &[u8]| VariantDefinitionRow {
        id: 70,
        equipment_token: token,
        creator_link: 11,
        icon_active: active,
        icon_kind: kind,
        icon: icon.to_vec(),
        ..Default::default()
    };
    let icon_at = |frame: &DecodedFrame,
                   content: &IconContent,
                   row: &VariantDefinitionRow,
                   spelling: &str| {
        let hour = IconHour::read(frame, content).unwrap();
        chosen(content, &hour, row, spelling)
    };
    let icon = |content: &IconContent, row: &VariantDefinitionRow, spelling: &str| {
        icon_at(&frame, content, row, spelling)
    };
    let sprite = |name: &str| Ok(Some(Icon::Sprite(name.to_owned())));
    let plain = content(&["GFX_fascist_tank", "GFX_default_tank"]);
    assert_eq!(
        icon(&plain, &variant(0, 1, 1, b"."), "tanks_1"),
        sprite("GFX_fascist_tank")
    );
    assert_eq!(
        icon(
            &content(&["GFX_default_tank"]),
            &variant(0, 1, 1, b"."),
            "tanks_1"
        ),
        sprite("GFX_default_tank")
    );
    let mut unread = frame.clone();
    *unread.tables[TableId::PoliticalParties.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    assert!(
        icon_at(&unread, &plain, &variant(0, 1, 1, b"."), "tanks_1")
            .unwrap_err()
            .contains("ruling party")
    );
    let mut unplaced = frame.clone();
    *unplaced.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![]));
    assert!(
        icon_at(&unplaced, &plain, &variant(0, 1, 1, b"."), "tanks_1")
            .unwrap_err()
            .contains("Politics")
    );
    let designed = content(&["GFX_my_tank", "GFX_tanks_1_medium"]);
    assert_eq!(
        icon(&designed, &variant(0, 1, 0, b"GFX_my_tank"), "tanks_1"),
        sprite("GFX_my_tank")
    );
    assert_eq!(
        icon(
            &designed,
            &variant(0, 1, 1, b"gfx/designs/my_tank.dds"),
            "tanks_1"
        ),
        Ok(Some(Icon::Path("gfx/designs/my_tank.dds".into())))
    );
    assert_eq!(
        icon(&designed, &variant(0, 1, 0, b"GFX_gone"), "tanks_1"),
        sprite("GFX_tanks_1_medium")
    );
    let named = content(&[
        "GFX_GER_tanks_1_medium",
        "GFX_tanks_1_medium",
        "GFX_tanks_medium",
    ]);
    assert_eq!(
        icon(&named, &variant(0, 0, 1, b""), "tanks_1"),
        sprite("GFX_GER_tanks_1_medium")
    );
    assert_eq!(
        icon(
            &content(&["GFX_tanks_medium"]),
            &variant(0, 0, 1, b""),
            "tanks_1"
        ),
        sprite("GFX_tanks_medium")
    );
    let mut capital = variant(0, 0, 1, b"");
    capital.classifier_flags = 1 << 6;
    assert_eq!(
        icon(
            &content(&["GFX_tanks_1_capital_medium", "GFX_GER_tanks_1_medium"]),
            &capital,
            "tanks_1"
        ),
        sprite("GFX_tanks_1_capital_medium")
    );
    assert_eq!(
        icon(
            &content(&["GFX_infantry_weapons1_medium"]),
            &variant(0, 0, 1, b""),
            "rifles_1"
        ),
        sprite("GFX_infantry_weapons1_medium")
    );
    assert_eq!(
        icon(
            &content(&["GFX_GER_infantry_weapons_medium"]),
            &variant(0, 0, 1, b""),
            "rifles_1"
        ),
        sprite("GFX_GER_infantry_weapons_medium")
    );
    assert_eq!(
        icon(
            &content(&["GFX_default_tank", "GFX_fascist_tank"]),
            &variant(0, 0, 1, b""),
            "tanks_1"
        ),
        sprite("GFX_default_tank")
    );
    assert_eq!(
        icon(&content(&[]), &variant(0, 0, 1, b""), "rifles_1"),
        Ok(None)
    );
    assert!(icon(&plain, &variant(0, 1, 7, b"x"), "tanks_1").is_err());
    assert!(icon(&plain, &variant(0, 0, 1, b""), "modded_thing").is_err());
    let mut no_appearance = frame.clone();
    *no_appearance.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let mut from_italy = variant(0, 0, 1, b"");
    from_italy.origin_link = 21;
    let italian = content(&["GFX_ITA_tanks_1_medium", "GFX_tanks_1_medium"]);
    assert_eq!(
        icon_at(&no_appearance, &italian, &from_italy, "tanks_1"),
        sprite("GFX_ITA_tanks_1_medium")
    );
    let by_technology = content(&["GFX_infantry_weapons1_medium"]);
    let unknown = icon_at(&no_appearance, &by_technology, &from_italy, "rifles_1").unwrap_err();
    assert!(unknown.contains("original tag is unknown"), "{unknown}");
    let tagged = content(&[
        "GFX_ITA_infantry_weapons1_medium",
        "GFX_western_infantry_weapons1_medium",
        "GFX_infantry_weapons1_medium",
    ]);
    let unknown = icon_at(&no_appearance, &tagged, &from_italy, "rifles_1").unwrap_err();
    assert!(unknown.contains("original tag is unknown"), "{unknown}");
    let mut cultured = frame.clone();
    *cultured.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay_data::rows::CountryAppearanceRow {
                country: 11,
                ..Default::default()
            },
            replay_data::rows::CountryAppearanceRow {
                country: 21,
                graphical_culture_2d: b"western".to_vec(),
                ..Default::default()
            },
        ]));
    assert_eq!(
        icon_at(&cultured, &tagged, &from_italy, "rifles_1"),
        sprite("GFX_ITA_infantry_weapons1_medium")
    );
    assert_eq!(
        icon_at(
            &cultured,
            &content(&[
                "GFX_western_infantry_weapons1_medium",
                "GFX_infantry_weapons1_medium"
            ]),
            &from_italy,
            "rifles_1"
        ),
        sprite("GFX_western_infantry_weapons1_medium")
    );
    assert_eq!(
        icon(&by_technology, &from_italy, "rifles_1"),
        sprite("GFX_infantry_weapons1_medium")
    );
    let mut dynamic = frame.clone();
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *dynamic.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 0, b"ITA"),
        country(41, 3, b"AAA"),
        country(9001, 7, b"GER"),
        country(31, 50, b"D01"),
    ]));
    *dynamic.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(11, 7), (21, 0), (9001, 7), (31, 50), (41, 3)]
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .to_vec(),
    ));
    *dynamic.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay_data::rows::CountryAppearanceRow {
                country: 11,
                ..Default::default()
            },
            replay_data::rows::CountryAppearanceRow {
                country: 31,
                name_tag: b"AAA".to_vec(),
                ..Default::default()
            },
            replay_data::rows::CountryAppearanceRow {
                country: 41,
                ..Default::default()
            },
        ]));
    let mut from_dynamic = variant(0, 0, 1, b"");
    from_dynamic.origin_link = 31;
    assert_eq!(
        icon_at(
            &dynamic,
            &content(&[
                "GFX_AAA_infantry_weapons1_medium",
                "GFX_D01_infantry_weapons1_medium",
                "GFX_infantry_weapons1_medium"
            ]),
            &from_dynamic,
            "rifles_1"
        ),
        sprite("GFX_AAA_infantry_weapons1_medium")
    );
    assert_eq!(
        icon_at(
            &dynamic,
            &content(&["GFX_AAA_rifles_1_medium", "GFX_D01_rifles_1_medium"]),
            &from_dynamic,
            "rifles_1"
        ),
        sprite("GFX_D01_rifles_1_medium")
    );
}

#[test]
fn equipment_icons_resolve_a_mods_types_technologies_and_sprites() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::equipment_icon::{Icon, IconContent, IconHour};
    use installed_content_core::{ResolvedLayers, technology};
    use installed_content_equipment_graphics::equipment_graphics;
    use replay_data::{rows::PoliticalPartyRow, rows::PoliticsRow, rows::VariantDefinitionRow};
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_modded_icons_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let (vanilla, modded) = (root.join("game"), root.join("mod"));
    for (folder, path, text) in [
        (
            &vanilla,
            "common/units/equipment/infantry.txt",
            "equipments = { rifles = { is_archetype = yes type = infantry } rifles_1 = { archetype = rifles } }",
        ),
        (
            &vanilla,
            "interface/equipment.gfx",
            r#"spriteTypes = { spriteType = { name = "GFX_rifles_1_medium" texturefile = "gfx/interface/rifle.dds" } }"#,
        ),
        (
            &modded,
            "common/units/equipment/lasers.txt",
            "equipments = { lasers = { is_archetype = yes type = infantry } laser_rifle_1 = { archetype = lasers } gauss_rifle_1 = { archetype = lasers } }",
        ),
        (
            &modded,
            "common/technologies/lasers.txt",
            "technologies = { Laser_Tech = { enable_equipments = { LASER_RIFLE_1 } } Gauss_Tech = { enable_equipments = { if = { limit = { has_dlc = \"X\" } gauss_rifle_1 rifles_1 } } } }",
        ),
        (
            &modded,
            "interface/equipment.gfx",
            r#"spriteTypes = {
                spriteType = { name = "GFX_rifles_1_medium" texturefile = "gfx/interface/mod_rifle.dds" }
                spriteType = { name = "GFX_Laser_Tech_medium" texturefile = "gfx/interface/laser.dds" }
            }"#,
        ),
    ] {
        let file = folder.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&vanilla, &[][..]), (&modded, &[][..])]).unwrap();
    let sprites = installed_content_gfx::gfx::read(&layers).unwrap();
    let content = IconContent::from_parts(
        installed_content_core::equipment::Database::load_complete(&layers).unwrap(),
        technology::read_enablers(&layers, |_| Err("no limit is decided here".to_owned())).unwrap(),
        equipment_graphics::Content::from_texts(
            &[],
            &equipment_graphics::Names {
                countries: ["GER".to_owned()].into(),
                continents: Default::default(),
            },
        )
        .unwrap(),
        Default::default(),
        sprites.iter().map(|(key, _)| key.clone()).collect(),
        Default::default(),
    );
    let _ = std::fs::remove_dir_all(&root);
    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 9001, &[], Some(&[]));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 11,
            is_ruling: 1,
            token: b"fascism".to_vec(),
            ..Default::default()
        }]));
    for table in [TableId::Factions, TableId::FactionMembers, TableId::States] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 11,
            ..Default::default()
        }]));
    let hour = IconHour::read(&frame, &content).unwrap();
    let row = VariantDefinitionRow {
        id: 70,
        creator_link: 11,
        ..Default::default()
    };
    let sprite = |name: &str| Ok(Some(Icon::Sprite(name.to_owned())));
    assert_eq!(
        chosen(&content, &hour, &row, "laser_rifle_1"),
        sprite("GFX_Laser_Tech_medium")
    );
    assert_eq!(
        chosen(&content, &hour, &row, "rifles_1"),
        sprite("GFX_rifles_1_medium")
    );
    assert_eq!(
        sprites
            .lookup("GFX_rifles_1_medium")
            .unwrap()
            .texture
            .as_deref(),
        Some("gfx/interface/mod_rifle.dds")
    );
    let refused = chosen(&content, &hour, &row, "plasma_rifle_1").unwrap_err();
    assert!(refused.contains("plasma_rifle_1"), "{refused}");
    let [warning] = content.warnings().try_into().unwrap();
    assert!(
        warning.starts_with("1 technologies") && warning.contains("Gauss_Tech"),
        "{warning}"
    );
    let unknown = chosen(&content, &hour, &row, "gauss_rifle_1").unwrap_err();
    assert!(unknown.contains("Gauss_Tech"), "{unknown}");
}
