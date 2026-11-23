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
        content_reader_military::technology::Enablers {
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
    use content_reader_military::technology::{Enabler, Enablers};
    use content_reader_equipment_graphics::equipment_graphics;
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
    let frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
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
    use content_reader_equipment_graphics::equipment_graphics;
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
    let frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
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
    use content_reader_military::technology::{Enabler, Enablers};

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
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::products::equipment_icon::{Icon, IconContent, IconHour};
    use content_reader_military::technology::{Enabler, Enablers};
    use content_reader_equipment_graphics::equipment_graphics;
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
    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            [11, 21]
                .map(|country| replay_data::rows::CountryAppearanceRow {
                    country,
                    original_tag_reference: replay_data::rows::Recorded::Value(Some(0)),
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
    assert!(
        unknown.contains("country appearance table is unavailable"),
        "{unknown}"
    );
    let tagged = content(&[
        "GFX_ITA_infantry_weapons1_medium",
        "GFX_western_infantry_weapons1_medium",
        "GFX_infantry_weapons1_medium",
    ]);
    assert_eq!(
        icon_at(&no_appearance, &tagged, &from_italy, "rifles_1"),
        sprite("GFX_ITA_infantry_weapons1_medium")
    );
    let mut dynamic_no_appearance = no_appearance.clone();
    *dynamic_no_appearance.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            replay_data::rows::ReservedCountryNamingRow {
                dynamic_tag_boundary: replay_data::rows::Recorded::Value(Some(0)),
                ..Default::default()
            },
        ]));
    let unknown =
        icon_at(&dynamic_no_appearance, &by_technology, &from_italy, "rifles_1").unwrap_err();
    assert!(unknown.contains("original tag is unknown"), "{unknown}");
    let unknown = icon_at(&dynamic_no_appearance, &tagged, &from_italy, "rifles_1").unwrap_err();
    assert!(unknown.contains("original tag is unknown"), "{unknown}");
    let mut cultured = frame.clone();
    *cultured.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay_data::rows::CountryAppearanceRow {
                country: 11,
                original_tag_reference: replay_data::rows::Recorded::Value(Some(0)),
                ..Default::default()
            },
            replay_data::rows::CountryAppearanceRow {
                country: 21,
                graphical_culture_2d: b"western".to_vec(),
                original_tag_reference: replay_data::rows::Recorded::Value(Some(0)),
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
                original_tag_reference: replay_data::rows::Recorded::Value(Some(0)),
                ..Default::default()
            },
            replay_data::rows::CountryAppearanceRow {
                country: 31,
                name_tag: b"AAA".to_vec(),
                original_tag_reference: replay_data::rows::Recorded::Value(Some(41)),
                original_tag: replay_data::rows::Recorded::Value(Some(b"AAA".to_vec())),
                ..Default::default()
            },
            replay_data::rows::CountryAppearanceRow {
                country: 41,
                original_tag_reference: replay_data::rows::Recorded::Value(Some(0)),
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
    content_reader_script::test_game::enter();
    use crate::products::equipment_icon::{Icon, IconContent, IconHour};
    use content_reader_layers::ResolvedLayers;
    use content_reader_military::technology;
    use content_reader_equipment_graphics::equipment_graphics;
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
            "common/terrain/fixture.txt",
            "categories = { fixture = {} }",
        ),
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
            "technologies = { Laser_Tech = { enable_equipments = { LASER_RIFLE_1 } } Gauss_Tech = { enable_equipments = { limit = { has_dlc = \"X\" } gauss_rifle_1 } enable_equipments = { limit = { has_dlc = \"X\" } rifles_1 } } }",
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
    let sprites = content_reader_gfx::gfx::read(&layers).unwrap();
    let content = IconContent::from_parts(
        content_reader_key_names::equipment::Database::load_complete(&layers).unwrap(),
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
    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
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

#[test]
fn inactive_dot_icon_and_invalid_inactive_kind_follow_native_icon_entry_rules() {
    use super::{Icon, IconContent, IconHour};
    use content_reader_equipment_graphics::equipment_graphics;
    use replay_data::rows::{CountryAppearanceRow, PoliticalPartyRow, PoliticsRow, VariantDefinitionRow};

    content_reader_script::test_game::enter();
    let units = equipment_types(&[br#"
        equipments = {
            tanks = { is_archetype = yes type = armor }
            tanks_1 = { archetype = tanks }
        }
    "#])
    .unwrap();
    let pools = equipment_graphics::Content::from_texts(
        &[br#"
            GER = { tanks_1 = { pool = { ideologies = { fascism } icons = { GFX_fascist_tank } } } }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned()].into(),
            continents: Default::default(),
        },
    )
    .unwrap();
    let content = IconContent::from_parts(
        units,
        Default::default(),
        pools,
        Default::default(),
        ["GFX_fascist_tank".to_owned(), "GFX_tanks_1_medium".to_owned()].into(),
        Default::default(),
    );
    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 11,
            original_tag_reference: replay_data::rows::Recorded::Value(Some(0)),
            ..Default::default()
        }]));
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
        TableView::Complete(TableRows::Politics(vec![
            PoliticsRow {
                country: 21,
                ..Default::default()
            },
            PoliticsRow {
                country: 11,
                ..Default::default()
            },
        ]));
    let hour = IconHour::read(&frame, &content).unwrap();
    let inactive_dot = VariantDefinitionRow {
        id: 70,
        creator_link: 11,
        icon_active: 0,
        icon_kind: 1,
        icon: b".".to_vec(),
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &inactive_dot, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_fascist_tank".into())))
    );
    let empty_pool_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        Default::default(),
        Default::default(),
        ["GFX_tanks_1_medium".to_owned()].into(),
        Default::default(),
    );
    assert_eq!(
        chosen(&empty_pool_content, &hour, &inactive_dot, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_tanks_1_medium".into())))
    );
    let inactive_bad_kind = VariantDefinitionRow {
        id: 71,
        creator_link: 11,
        icon_active: 0,
        icon_kind: 7,
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &inactive_bad_kind, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_tanks_1_medium".into())))
    );
    let active_bad_kind = VariantDefinitionRow {
        id: 72,
        creator_link: 11,
        icon_active: 1,
        icon_kind: 7,
        ..Default::default()
    };
    assert!(chosen(&content, &hour, &active_bad_kind, "tanks_1").is_err());
}

#[test]
fn equipment_icons_resolve_mixed_continent_and_coastal_capital_states() {
    use super::{Icon, IconContent, IconHour};
    use content_reader_layers::ResolvedLayers;
    use content_reader_equipment_graphics::equipment_graphics;
    use replay_data::rows::{
        CountryAppearanceRow, LexicalTokenRow, PoliticalPartyRow, PoliticsRow, Recorded, StateRow,
        VariantDefinitionRow,
    };

    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_mixed_continent_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in [
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continent.txt\n",
        ),
        ("map/continent.txt", "continents = { europe asia }\n"),
        (
            "map/definition.csv",
            "0;0;0;0;land;false;unknown;0\n\
             1;1;0;0;land;false;plains;1\n\
             2;2;0;0;land;false;plains;2\n\
             3;3;0;0;sea;true;ocean;0\n\
             4;4;0;0;land;false;plains;2\n",
        ),
        (
            "common/defines/00_defines.lua",
            "NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 10 } }\n",
        ),
        (
            "history/states/1-mixed.txt",
            "state = { id = 1 name = \"STATE_1\" provinces = { 1 2 } }\n",
        ),
        (
            "history/states/2-sea-coastal-zero.txt",
            "state = { id = 2 name = \"STATE_2\" provinces = { 3 4 } }\n",
        ),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let default_map = content_reader_key_names::terrain::read_default_map(&layers).unwrap();
    let provinces = content_reader_map::map::parse_definitions(
        &layers.read(&default_map.file("definitions").unwrap()).unwrap(),
    )
    .unwrap();
    let defines = content_reader_defines::defines::read(&layers).unwrap();
    let geography =
        equipment_graphics::context::Content::load(&layers, &default_map, &provinces, &defines)
            .unwrap();
    let _ = std::fs::remove_dir_all(&root);

    let units = equipment_types(&[br#"
        equipments = {
            tanks = { is_archetype = yes type = armor }
            tanks_1 = { archetype = tanks }
        }
    "#])
    .unwrap();
    let pools = equipment_graphics::Content::from_texts(
        &[br#"
            europe = { tanks_1 = { pool = { icons = { GFX_europe_tank } } } }
            asia = { tanks_1 = { pool = { icons = { GFX_asia_tank } } } }
            default = { tanks_1 = { pool = { icons = { GFX_default_tank } } } }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned(), "ITA".to_owned()].into(),
            continents: ["europe".to_owned(), "asia".to_owned()].into(),
        },
    )
    .unwrap();
    let content = IconContent::from_parts(
        units,
        Default::default(),
        pools,
        Default::default(),
        [
            "GFX_europe_tank".to_owned(),
            "GFX_asia_tank".to_owned(),
            "GFX_default_tank".to_owned(),
        ]
        .into(),
        Default::default(),
    )
    .with_geography(geography.clone());

    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            [11, 21]
                .map(|country| CountryAppearanceRow {
                    country,
                    original_tag_reference: Recorded::Value(Some(0)),
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
    for table in [TableId::Factions, TableId::FactionMembers] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 101,
                literal: b"Europe".to_vec(),
            },
            LexicalTokenRow {
                token: 102,
                literal: b"Asia".to_vec(),
            },
        ]));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            continent: Recorded::Value(Some(101)),
            ..Default::default()
        },
        StateRow {
            id: 2,
            continent: Recorded::Value(Some(102)),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
        PoliticsRow {
            country: 11,
            original_capital: 1,
            capital: 2,
            ..Default::default()
        },
        PoliticsRow {
            country: 21,
            original_capital: 0,
            capital: 2,
            ..Default::default()
        },
    ]));
    let from_mixed = VariantDefinitionRow {
        id: 80,
        creator_link: 11,
        icon_active: 1,
        icon_kind: 1,
        icon: b".".to_vec(),
        ..Default::default()
    };
    let from_second = VariantDefinitionRow {
        id: 81,
        creator_link: 21,
        icon_active: 1,
        icon_kind: 1,
        icon: b".".to_vec(),
        ..Default::default()
    };
    let hour = IconHour::read(&frame, &content).unwrap();
    assert_eq!(
        chosen(&content, &hour, &from_mixed, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_europe_tank".into())))
    );
    assert_eq!(
        chosen(&content, &hour, &from_second, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_asia_tank".into())))
    );

    let mut null_vs_failed = frame.clone();
    *null_vs_failed.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![
            StateRow {
                id: 1,
                continent: Recorded::Value(Some(0)),
                ..Default::default()
            },
            StateRow {
                id: 2,
                continent: Recorded::Value(None),
                ..Default::default()
            },
        ]));
    let hour = IconHour::read(&null_vs_failed, &content).unwrap();
    assert_eq!(
        chosen(&content, &hour, &from_mixed, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_default_tank".into())))
    );
    assert!(
        chosen(&content, &hour, &from_second, "tanks_1")
            .unwrap_err()
            .contains("continent capture unavailable")
    );

    let no_continent_pools = equipment_graphics::Content::from_texts(
        &[br#"
            default = { tanks = { pool = { icons = { GFX_default_tank } } } }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned(), "ITA".to_owned()].into(),
            continents: ["europe".to_owned(), "asia".to_owned()].into(),
        },
    )
    .unwrap();
    let continent_independent = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        no_continent_pools,
        Default::default(),
        ["GFX_default_tank".to_owned(), "GFX_tanks_1_medium".to_owned()].into(),
        Default::default(),
    )
    .with_geography(geography.clone());
    assert_eq!(
        chosen(&continent_independent, &hour, &from_second, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_default_tank".into())))
    );
    let empty_pools = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        equipment_graphics::Content::default(),
        Default::default(),
        ["GFX_tanks_1_medium".to_owned()].into(),
        Default::default(),
    )
    .with_geography(geography);
    assert_eq!(
        chosen(&empty_pools, &hour, &from_second, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_tanks_1_medium".into())))
    );

    let mut older_format = frame.clone();
    *older_format.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![
            StateRow {
                id: 1,
                continent: Recorded::NotRecorded,
                ..Default::default()
            },
            StateRow {
                id: 2,
                continent: Recorded::NotRecorded,
                ..Default::default()
            },
        ]));
    let hour = IconHour::read(&older_format, &content).unwrap();
    assert_eq!(
        chosen(&content, &hour, &from_mixed, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_europe_tank".into())))
    );
    assert_eq!(
        chosen(&content, &hour, &from_second, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_asia_tank".into())))
    );
}

#[test]
fn dynamic_creator_fallback_uses_original_reference_and_witnessed_handle_tag() {
    use super::{Icon, IconContent, IconHour};
    use content_reader_equipment_graphics::equipment_graphics;
    use replay_data::rows::{
        CountryAppearanceRow, LexicalTokenRow, PoliticalPartyRow, PoliticsRow, Recorded,
        ReservedCountryNamingRow, StateRow, VariantDefinitionRow,
    };

    content_reader_script::test_game::enter();
    let units = equipment_types(&[br#"
        equipments = {
            tanks = { is_archetype = yes type = armor }
            tanks_1 = { archetype = tanks }
        }
    "#])
    .unwrap();
    let pools = equipment_graphics::Content::from_texts(
        &[br#"
            europe = { tanks_1 = { pool = { icons = { GFX_holder1_europe } } } }
            asia = { tanks_1 = { pool = { icons = { GFX_holder2_asia } } } }
            default = { tanks_1 = { pool = { icons = { GFX_default_tank } } } }
        "#],
        &equipment_graphics::Names {
            countries: [
                "AAA".to_owned(),
                "D01".to_owned(),
                "D02".to_owned(),
                "STA".to_owned(),
            ]
            .into(),
            continents: ["europe".to_owned(), "asia".to_owned()].into(),
        },
    )
    .unwrap();
    let content = IconContent::from_parts(
        units,
        Default::default(),
        pools,
        Default::default(),
        [
            "GFX_holder1_europe".to_owned(),
            "GFX_holder2_asia".to_owned(),
            "GFX_default_tank".to_owned(),
            "GFX_explicit".to_owned(),
            "GFX_STA_tanks_1_medium".to_owned(),
            "GFX_D02_tanks_1_medium".to_owned(),
        ]
        .into(),
        Default::default(),
    );

    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(41, 3, b"AAA"),
        country(51, 4, b"STA"),
        country(42, 9, b"AAA"),
        country(31, 50, b"D01"),
        country(32, 51, b"D02"),
    ]));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(41, 3), (51, 4), (42, 9), (31, 50), (32, 51), (88, 51), (99, 3)]
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .to_vec(),
    ));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                dynamic_tag_boundary: Recorded::Value(Some(50)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            CountryAppearanceRow {
                country: 41,
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 51,
                original_tag_reference: Recorded::Value(Some(41)),
                original_tag: Recorded::Value(Some(b"AAA".to_vec())),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 42,
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 31,
                name_tag: b"AAA".to_vec(),
                original_tag_reference: Recorded::Value(Some(41)),
                original_tag: Recorded::Value(Some(b"AAA".to_vec())),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 32,
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 31,
            is_ruling: 1,
            token: b"fascism".to_vec(),
            ..Default::default()
        }]));
    for table in [TableId::Factions, TableId::FactionMembers] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 101,
                literal: b"Europe".to_vec(),
            },
            LexicalTokenRow {
                token: 102,
                literal: b"Asia".to_vec(),
            },
        ]));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            continent: Recorded::Value(Some(101)),
            ..Default::default()
        },
        StateRow {
            id: 2,
            continent: Recorded::Value(Some(102)),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
        PoliticsRow {
            country: 41,
            original_capital: 1,
            ..Default::default()
        },
        PoliticsRow {
            country: 51,
            original_capital: 2,
            ..Default::default()
        },
        PoliticsRow {
            country: 42,
            original_capital: 2,
            ..Default::default()
        },
        PoliticsRow {
            country: 31,
            original_capital: 2,
            ..Default::default()
        },
        PoliticsRow {
            country: 32,
            original_capital: 2,
            ..Default::default()
        },
    ]));
    let hour = IconHour::read(&frame, &content).unwrap();
    let from_dynamic = VariantDefinitionRow {
        id: 90,
        creator_link: 31,
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &from_dynamic, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_holder1_europe".into())))
    );
    let unwitnessed_alias = VariantDefinitionRow {
        id: 91,
        origin_link: 99,
        ..Default::default()
    };
    assert!(
        chosen(&content, &hour, &unwitnessed_alias, "tanks_1")
            .unwrap_err()
            .contains("witnessed tag spelling")
    );
    let explicit_with_unwitnessed_alias = VariantDefinitionRow {
        id: 92,
        origin_link: 99,
        icon_active: 1,
        icon_kind: 0,
        icon: b"GFX_explicit".to_vec(),
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &explicit_with_unwitnessed_alias, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_explicit".into())))
    );
    let from_static_with_ref = VariantDefinitionRow {
        id: 93,
        creator_link: 51,
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &from_static_with_ref, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_STA_tanks_1_medium".into())))
    );
    let mut no_appearance = frame.clone();
    *no_appearance.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "appearance unavailable".into(),
    };
    let no_app_hour = IconHour::read(&no_appearance, &content).unwrap();
    assert_eq!(
        chosen(&content, &no_app_hour, &from_static_with_ref, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_STA_tanks_1_medium".into())))
    );
    let from_dynamic_alias_without_ref = VariantDefinitionRow {
        id: 94,
        creator_link: 88,
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &from_dynamic_alias_without_ref, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_D02_tanks_1_medium".into())))
    );
}

#[test]
fn static_slot_technology_tag_and_missing_boundary_agreement() {
    use super::{Icon, IconContent, IconHour};
    use content_reader_military::technology::{Enabler, Enablers};
    use replay_data::rows::{
        CountryAppearanceRow, Recorded, ReservedCountryNamingRow, VariantDefinitionRow,
    };

    content_reader_script::test_game::enter();
    let units = equipment_types(&[br#"
        equipments = {
            rifles = { is_archetype = yes type = infantry }
            rifles_1 = { archetype = rifles }
        }
    "#])
    .unwrap();
    let enablers = Enablers {
        plain: [(
            "rifles_1".to_owned(),
            vec![Enabler {
                id: "infantry_weapons1".into(),
                parent: Ok(None),
            }],
        )]
        .into(),
        ..Default::default()
    };
    let content = IconContent::from_parts(
        units,
        enablers,
        Default::default(),
        Default::default(),
        [
            "GFX_STA_infantry_weapons1_medium".to_owned(),
            "GFX_AAA_infantry_weapons1_medium".to_owned(),
            "GFX_D02_infantry_weapons1_medium".to_owned(),
        ]
        .into(),
        Default::default(),
    );

    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(41, 3, b"AAA"),
        country(51, 4, b"STA"),
        country(31, 50, b"D01"),
        country(32, 51, b"D02"),
    ]));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [(41, 3), (51, 4), (31, 50), (32, 51), (88, 51)]
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .to_vec(),
    ));
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                dynamic_tag_boundary: Recorded::Value(Some(50)),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            CountryAppearanceRow {
                country: 41,
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 51,
                original_tag_reference: Recorded::Value(Some(41)),
                original_tag: Recorded::Value(Some(b"AAA".to_vec())),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 31,
                original_tag_reference: Recorded::Value(Some(41)),
                original_tag: Recorded::Value(Some(b"AAA".to_vec())),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 32,
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
        ]));

    let hour = IconHour::read(&frame, &content).unwrap();
    let from_static_with_ref = VariantDefinitionRow {
        id: 1,
        creator_link: 51,
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &from_static_with_ref, "rifles_1"),
        Ok(Some(Icon::Sprite("GFX_STA_infantry_weapons1_medium".into())))
    );
    let from_dynamic_with_ref = VariantDefinitionRow {
        id: 2,
        creator_link: 31,
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &from_dynamic_with_ref, "rifles_1"),
        Ok(Some(Icon::Sprite("GFX_AAA_infantry_weapons1_medium".into())))
    );

    let mut no_boundary = frame.clone();
    *no_boundary.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                dynamic_tag_boundary: Recorded::NotRecorded,
                ..Default::default()
            },
        ]));
    let no_boundary_hour = IconHour::read(&no_boundary, &content).unwrap();
    let unambiguous_creator = VariantDefinitionRow {
        id: 3,
        creator_link: 41,
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &no_boundary_hour, &unambiguous_creator, "rifles_1"),
        Ok(Some(Icon::Sprite("GFX_AAA_infantry_weapons1_medium".into())))
    );
    assert!(
        chosen(&content, &no_boundary_hour, &from_static_with_ref, "rifles_1")
            .unwrap_err()
            .contains("dynamic-tag boundary")
    );
    let alias_creator = VariantDefinitionRow {
        id: 4,
        creator_link: 88,
        ..Default::default()
    };
    assert!(
        chosen(&content, &no_boundary_hour, &alias_creator, "rifles_1")
            .unwrap_err()
            .contains("dynamic-tag boundary")
    );
}

#[test]
fn unknown_continent_prepared_icon_choice_keeps_common_winner_across_continents() {
    use super::{Icon, IconContent, IconHour};
    use content_reader_equipment_graphics::equipment_graphics;
    use replay_data::rows::{
        CountryAppearanceRow, PoliticalPartyRow, PoliticsRow, Recorded, StateRow,
        VariantDefinitionRow,
    };

    content_reader_script::test_game::enter();
    let units = equipment_types(&[br#"
        equipments = {
            tanks = { is_archetype = yes type = armor }
            tanks_1 = { archetype = tanks }
        }
    "#])
    .unwrap();
    let pools = equipment_graphics::Content::from_texts(
        &[br#"
            GER = { tanks_1 = { pool = { weight = 10 icons = { GFX_shared_icon } } } }
            europe = { tanks_1 = { pool = { weight = 20 icons = { GFX_shared_icon } } } }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        },
    )
    .unwrap();
    let content = IconContent::from_parts(
        units,
        Default::default(),
        pools,
        Default::default(),
        ["GFX_shared_icon".to_owned()].into(),
        Default::default(),
    );

    let mut frame = production_frame(PRODUCTION_HOUR, true, true, 11, &[], Some(&[]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 11,
            original_tag_reference: Recorded::Value(Some(0)),
            ..Default::default()
        }]));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 11,
            is_ruling: 1,
            token: b"fascism".to_vec(),
            ..Default::default()
        }]));
    for table in [TableId::Factions, TableId::FactionMembers] {
        *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            continent: Recorded::Value(None),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(vec![
        PoliticsRow {
            country: 11,
            original_capital: 1,
            ..Default::default()
        },
        PoliticsRow {
            country: 21,
            original_capital: 1,
            ..Default::default()
        },
    ]));
    let hour = IconHour::read(&frame, &content).unwrap();
    let from_germany = VariantDefinitionRow {
        id: 1,
        creator_link: 11,
        ..Default::default()
    };
    assert_eq!(
        chosen(&content, &hour, &from_germany, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_shared_icon".into())))
    );
    let diverging_pools = equipment_graphics::Content::from_texts(
        &[br#"
            GER = { tanks_1 = { pool = { weight = 10 icons = { GFX_ger_only GFX_shared_icon } } } }
            europe = { tanks_1 = { pool = { weight = 20 icons = { GFX_eur_only GFX_shared_icon } } } }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        },
    )
    .unwrap();
    let diverging = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        diverging_pools,
        Default::default(),
        ["GFX_ger_only".to_owned(), "GFX_eur_only".to_owned()].into(),
        Default::default(),
    );
    let diverging_hour = IconHour::read(&frame, &diverging).unwrap();
    assert!(chosen(&diverging, &diverging_hour, &from_germany, "tanks_1").is_err());

    let fallback_pools = equipment_graphics::Content::from_texts(
        &[br#"
            GER = { tanks_1 = { pool = { weight = 10 icons = { GFX_shared } } } }
            europe = { tanks_1 = { pool = { weight = 20 icons = { GFX_europe } } } }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        },
    )
    .unwrap();
    let fallback_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        fallback_pools,
        Default::default(),
        ["GFX_shared".to_owned()].into(),
        Default::default(),
    );
    let fallback_hour = IconHour::read(&frame, &fallback_content).unwrap();
    assert_eq!(
        chosen(&fallback_content, &fallback_hour, &from_germany, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_shared".into())))
    );
    let prepared = super::icon_choice(&fallback_content, &fallback_hour, &from_germany, "tanks_1");
    assert_eq!(
        prepared.walk(|probe| match probe {
            super::Probe::Existing(name) if name == "GFX_shared" => {
                Ok(Some(Icon::Sprite(name.clone())))
            }
            _ => Ok(None),
        }),
        Ok(Some(Icon::Sprite("GFX_shared".into())))
    );
    assert!(
        prepared
            .walk(|probe| match probe {
                super::Probe::Existing(name) if name == "GFX_shared" || name == "GFX_europe" => {
                    Ok(Some(Icon::Sprite(name.clone())))
                }
                _ => Ok(None),
            })
            .is_err()
    );
    assert!(
        prepared
            .walk(|probe| match probe {
                super::Probe::Existing(name) if name == "GFX_europe" => {
                    Ok(Some(Icon::Sprite(name.clone())))
                }
                _ => Ok(None),
            })
            .is_err()
    );
    assert_eq!(prepared.walk(|_| Ok(None)), Ok(None));

    let family_pools = equipment_graphics::Content::from_texts(
        &[br#"
            europe = {
                tanks_family = { pool = { weight = 10 icons = { GFX_a } } }
                tanks_1 = { pool = { weight = 20 icons = { GFX_b } } }
            }
            default = {
                tanks_1 = { pool = { weight = 10 icons = { GFX_a } } }
            }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        },
    )
    .unwrap();
    let family_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_family = { archetype = tanks }
                tanks_1 = { archetype = tanks family = tanks_family }
            }
        "#])
        .unwrap(),
        Default::default(),
        family_pools,
        Default::default(),
        ["GFX_a".to_owned(), "GFX_b".to_owned()].into(),
        Default::default(),
    );
    let family_hour = IconHour::read(&frame, &family_content).unwrap();
    assert_eq!(
        chosen(&family_content, &family_hour, &from_germany, "tanks_1"),
        Ok(Some(Icon::Sprite("GFX_a".into())))
    );
    let family_choice =
        super::icon_choice(&family_content, &family_hour, &from_germany, "tanks_1");
    assert!(
        family_choice
            .walk(|probe| match probe {
                super::Probe::Existing(name) if name == "GFX_b" => {
                    Ok(Some(Icon::Sprite(name.clone())))
                }
                _ => Ok(None),
            })
            .is_err()
    );
    assert_eq!(family_choice.walk(|_| Ok(None)), Ok(None));

    let mut deep_icons = String::from("default = { tanks_1 = { pool = { icons = { GFX_tail_ok ");
    for i in 0..20_000 {
        deep_icons.push_str(&format!("GFX_miss_{i} "));
    }
    deep_icons.push_str("} } } }");
    let deep_pools = equipment_graphics::Content::from_texts(
        &[deep_icons.as_bytes()],
        &equipment_graphics::Names {
            countries: ["GER".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        },
    )
    .unwrap();
    let deep_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        deep_pools,
        Default::default(),
        ["GFX_tail_ok".to_owned()].into(),
        Default::default(),
    );
    let mut known_frame = frame.clone();
    *known_frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            continent: Recorded::Value(Some(0)),
            ..Default::default()
        },
    ]));
    let deep_hour = IconHour::read(&known_frame, &deep_content).unwrap();
    let deep_choice = super::icon_choice(&deep_content, &deep_hour, &from_germany, "tanks_1");
    let handle = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            deep_choice.walk(|probe| match probe {
                super::Probe::Existing(name) if name == "GFX_tail_ok" => {
                    Ok(Some(Icon::Sprite(name.clone())))
                }
                _ => Ok(None),
            })
        })
        .unwrap();
    assert_eq!(
        handle.join().unwrap(),
        Ok(Some(Icon::Sprite("GFX_tail_ok".into())))
    );

    let same_slot_pools = equipment_graphics::Content::from_texts(
        &[br#"
            europe = {
                tanks_1 = {
                    pool = { weight = 30 cultures = { western_european_gfx } icons = { GFX_a } }
                    pool = { weight = 20 icons = { GFX_b } }
                }
            }
            PRE = {
                tanks_1 = { pool = { weight = 10 icons = { GFX_a } } }
            }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned(), "PRE".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        },
    )
    .unwrap();
    let same_slot_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        same_slot_pools,
        Default::default(),
        ["GFX_a".to_owned(), "GFX_b".to_owned()].into(),
        Default::default(),
    );
    let mut same_slot_frame = frame.clone();
    *same_slot_frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 11,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 12,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 21,
                canonical_slot: 0,
            },
        ]));
    *same_slot_frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            replay_data::rows::ReservedCountryNamingRow {
                original_tag_reference: Recorded::Value(Some(12)),
                original_tag: Recorded::Value(Some(b"PRE".to_vec())),
                dynamic_tag_boundary: Recorded::Value(Some(50)),
                ..Default::default()
            },
        ]));
    *same_slot_frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 11,
            graphical_culture: b"western_european_gfx".to_vec(),
            original_tag_reference: Recorded::Value(Some(0)),
            ..Default::default()
        }]));
    let same_slot_hour = IconHour::read(&same_slot_frame, &same_slot_content).unwrap();
    let same_slot_variant = VariantDefinitionRow {
        id: 2,
        icon_kind: 1,
        icon: b".".to_vec(),
        creator_link: 11,
        origin_link: 12,
        ..Default::default()
    };
    assert_eq!(
        chosen(
            &same_slot_content,
            &same_slot_hour,
            &same_slot_variant,
            "tanks_1"
        ),
        Ok(Some(Icon::Sprite("GFX_a".into())))
    );

    let diff_country_pools = equipment_graphics::Content::from_texts(
        &[br#"
            europe = {
                tanks_1 = { pool = { weight = 20 icons = { GFX_creator_missing } } }
                tanks_root = { pool = { weight = 20 icons = { GFX_origin_eur } } }
            }
            default = {
                tanks_root = { pool = { weight = 10 icons = { GFX_origin_def } } }
            }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned(), "ITA".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        },
    )
    .unwrap();
    let diff_country_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_root = { archetype = tanks }
                tanks_mid = { archetype = tanks family = tanks_root }
                tanks_1 = { archetype = tanks family = tanks_mid }
            }
        "#])
        .unwrap(),
        Default::default(),
        diff_country_pools,
        Default::default(),
        ["GFX_origin_eur".to_owned(), "GFX_origin_def".to_owned()].into(),
        Default::default(),
    );
    let mut diff_country_frame = same_slot_frame.clone();
    *diff_country_frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        vec![
            StateRow {
                id: 1,
                continent: Recorded::Value(None),
                ..Default::default()
            },
            StateRow {
                id: 2,
                continent: Recorded::Value(None),
                ..Default::default()
            },
        ],
    ));
    *diff_country_frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![
            PoliticsRow {
                country: 11,
                original_capital: 1,
                ..Default::default()
            },
            PoliticsRow {
                country: 21,
                original_capital: 2,
                ..Default::default()
            },
        ]));
    *diff_country_frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            CountryAppearanceRow {
                country: 11,
                graphical_culture: b"western_european_gfx".to_vec(),
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 21,
                graphical_culture: b"western_european_gfx".to_vec(),
                original_tag_reference: Recorded::Value(Some(0)),
                ..Default::default()
            },
        ]));
    let diff_country_hour = IconHour::read(&diff_country_frame, &diff_country_content).unwrap();
    let diff_country_variant = VariantDefinitionRow {
        id: 3,
        icon_kind: 1,
        icon: b".".to_vec(),
        creator_link: 11,
        origin_link: 21,
        ..Default::default()
    };
    let diff_err = chosen(
        &diff_country_content,
        &diff_country_hour,
        &diff_country_variant,
        "tanks_1",
    )
    .unwrap_err();
    assert!(
        diff_err.contains("capital state 2"),
        "expected origin capital state 2 error, got: {diff_err}"
    );

    let shared_capital_pools = equipment_graphics::Content::from_texts(
        &[br#"
            europe = {
                tanks_1 = {
                    pool = { weight = 30 cultures = { western_european_gfx } icons = { GFX_a } }
                    pool = { weight = 20 icons = { GFX_b } }
                }
            }
            ITA = {
                tanks_1 = { pool = { weight = 10 icons = { GFX_a } } }
            }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned(), "ITA".to_owned()].into(),
            continents: ["europe".to_owned()].into(),
        },
    )
    .unwrap();
    let shared_capital_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        shared_capital_pools,
        Default::default(),
        ["GFX_a".to_owned(), "GFX_b".to_owned()].into(),
        Default::default(),
    );
    let mut shared_capital_frame = diff_country_frame.clone();
    *shared_capital_frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![
            PoliticsRow {
                country: 11,
                original_capital: 1,
                ..Default::default()
            },
            PoliticsRow {
                country: 21,
                original_capital: 1,
                ..Default::default()
            },
        ]));
    let shared_capital_hour =
        IconHour::read(&shared_capital_frame, &shared_capital_content).unwrap();
    let shared_capital_variant = VariantDefinitionRow {
        id: 4,
        icon_kind: 1,
        icon: b".".to_vec(),
        creator_link: 11,
        origin_link: 21,
        ..Default::default()
    };
    assert_eq!(
        chosen(
            &shared_capital_content,
            &shared_capital_hour,
            &shared_capital_variant,
            "tanks_1"
        ),
        Ok(Some(Icon::Sprite("GFX_a".into())))
    );

    let mut shared_token_frame = diff_country_frame.clone();
    *shared_token_frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        vec![
            StateRow {
                id: 1,
                continent: Recorded::Value(Some(777)),
                ..Default::default()
            },
            StateRow {
                id: 2,
                continent: Recorded::Value(Some(777)),
                ..Default::default()
            },
        ],
    ));
    let shared_token_hour =
        IconHour::read(&shared_token_frame, &shared_capital_content).unwrap();
    assert_eq!(
        chosen(
            &shared_capital_content,
            &shared_token_hour,
            &shared_capital_variant,
            "tanks_1"
        ),
        Ok(Some(Icon::Sprite("GFX_a".into())))
    );

    let masked_error_pools = equipment_graphics::Content::from_texts(
        &[br#"
            default = {
                tanks_1 = { pool = { weight = 10 icons = { GFX_a } } }
            }
            europe = {
                tanks_1 = { pool = { weight = 20 icons = { GFX_b } } }
            }
            oceania = {
                tanks_1 = { pool = { weight = 30 icons = { GFX_refused } } }
            }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned()].into(),
            continents: ["europe".to_owned(), "oceania".to_owned()].into(),
        },
    )
    .unwrap();
    let mut masked_error_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        masked_error_pools,
        Default::default(),
        ["GFX_a".to_owned(), "GFX_b".to_owned()].into(),
        Default::default(),
    );
    masked_error_content
        .refused_sprites
        .insert("GFX_refused".to_owned(), "refused".to_owned());
    let masked_error_hour = IconHour::read(&frame, &masked_error_content).unwrap();
    let masked_error_variant = VariantDefinitionRow {
        id: 5,
        origin_link: 11,
        ..Default::default()
    };
    let masked_err = chosen(
        &masked_error_content,
        &masked_error_hour,
        &masked_error_variant,
        "tanks_1",
    )
    .unwrap_err();
    assert!(
        masked_err.contains("capital state 1"),
        "expected proven continent disagreement to take precedence over later asset error, got: {masked_err}"
    );

    let blocking_error_pools = equipment_graphics::Content::from_texts(
        &[br#"
            default = {
                tanks_1 = { pool = { weight = 10 icons = { GFX_a } } }
            }
            europe = {
                tanks_1 = { pool = { weight = 20 icons = { GFX_a } } }
            }
            oceania = {
                tanks_1 = { pool = { weight = 30 icons = { GFX_refused } } }
            }
        "#],
        &equipment_graphics::Names {
            countries: ["GER".to_owned()].into(),
            continents: ["europe".to_owned(), "oceania".to_owned()].into(),
        },
    )
    .unwrap();
    let mut blocking_error_content = IconContent::from_parts(
        equipment_types(&[br#"
            equipments = {
                tanks = { is_archetype = yes type = armor }
                tanks_1 = { archetype = tanks }
            }
        "#])
        .unwrap(),
        Default::default(),
        blocking_error_pools,
        Default::default(),
        ["GFX_a".to_owned()].into(),
        Default::default(),
    );
    blocking_error_content
        .refused_sprites
        .insert("GFX_refused".to_owned(), "refused".to_owned());
    let blocking_error_hour = IconHour::read(&frame, &blocking_error_content).unwrap();
    assert_eq!(
        chosen(
            &blocking_error_content,
            &blocking_error_hour,
            &masked_error_variant,
            "tanks_1",
        ),
        Err("sprite GFX_refused is refused: refused".to_owned())
    );
}

#[test]
fn unverified_technology_parse_withholds_equipment_icon_fallback_and_warns() {
    content_reader_script::test_game::enter();
    use crate::products::equipment_icon::{Icon, IconContent, IconHour};
    use content_reader_layers::ResolvedLayers;
    use content_reader_military::technology;
    use content_reader_equipment_graphics::equipment_graphics;
    use replay_data::rows::VariantDefinitionRow;

    let root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_unverified_icons_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in [
        ("common/terrain/fixture.txt", "categories = { fixture = {} }"),
        (
            "common/units/equipment/infantry.txt",
            "equipments = { rifles = { is_archetype = yes type = infantry } rifles_1 = { archetype = rifles } lost_rifle = { archetype = rifles } }",
        ),
        (
            "common/technologies/broken.txt",
            "technologies = { broken = { on_research_complete = no { ignored = 1 } } consumed = { on_research_complete = no limit = {} enable_equipments = { lost_rifle } } }",
        ),
    ] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let content = IconContent::from_parts(
        content_reader_key_names::equipment::Database::load_complete(&layers).unwrap(),
        technology::read_enablers(&layers, |_| Err("no limit is decided here".to_owned())).unwrap(),
        equipment_graphics::Content::from_texts(
            &[],
            &equipment_graphics::Names {
                countries: Default::default(),
                continents: Default::default(),
            },
        )
        .unwrap(),
        Default::default(),
        [
            "GFX_rifles_1_medium".to_owned(),
            "GFX_consumed_medium".to_owned(),
        ]
        .into(),
        Default::default(),
    );
    let _ = std::fs::remove_dir_all(&root);

    let frame = production_frame(PRODUCTION_HOUR, true, true, 9001, &[], Some(&[]));
    let hour = IconHour::read(&frame, &content).unwrap();
    let row = VariantDefinitionRow::default();
    assert_eq!(
        chosen(&content, &hour, &row, "rifles_1"),
        Ok(Some(Icon::Sprite("GFX_rifles_1_medium".into())))
    );
    let withheld = chosen(&content, &hour, &row, "lost_rifle").unwrap_err();
    assert!(
        withheld.contains("common/technologies/broken.txt")
            && withheld.contains("on_research_complete"),
        "{withheld}"
    );
    let warnings = content.warnings();
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("common/technologies/broken.txt")
                && warning.contains("on_research_complete")),
        "{warnings:?}"
    );
    let prep = content.preparation_warnings();
    assert!(
        prep.iter().any(|(kind, warning)| *kind
            == content_reader_parser::DiagnosticKind::Unsettled
            && warning.contains("common/technologies/broken.txt")),
        "{prep:?}"
    );
    let authored = content.authored_icons().unwrap();
    let lost = authored
        .iter()
        .find(|entry| entry.token == "lost_rifle")
        .unwrap();
    assert!(
        lost.refusals
            .iter()
            .any(|reason| reason.contains("common/technologies/broken.txt")),
        "{:?}",
        lost.refusals
    );

    let unreadable_root = std::env::temp_dir().join(format!(
        "hoi4_dashboard_unreadable_tech_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&unreadable_root);
    for (path, text) in [
        ("common/terrain/fixture.txt", "categories = { fixture = {} }"),
        (
            "common/units/equipment/infantry.txt",
            "equipments = { rifles = { is_archetype = yes type = infantry } rifles_1 = { archetype = rifles } solo_rifle = { archetype = rifles } }",
        ),
        (
            "common/technologies/ok.txt",
            "technologies = { solo = { enable_equipments = { solo_rifle } } }",
        ),
        (
            "common/technologies/gone.txt",
            "technologies = { gone = { enable_equipments = { rifles_1 } } }",
        ),
    ] {
        let file = unreadable_root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&unreadable_root, &[][..])]).unwrap();
    std::fs::remove_file(unreadable_root.join("common/technologies/gone.txt")).unwrap();
    let content = IconContent::from_parts(
        content_reader_key_names::equipment::Database::load_complete(&layers).unwrap(),
        technology::read_enablers(&layers, |_| Err("no limit is decided here".to_owned())).unwrap(),
        equipment_graphics::Content::from_texts(
            &[],
            &equipment_graphics::Names {
                countries: Default::default(),
                continents: Default::default(),
            },
        )
        .unwrap(),
        Default::default(),
        ["GFX_rifles_1_medium".to_owned()].into(),
        Default::default(),
    );
    let _ = std::fs::remove_dir_all(&unreadable_root);
    let hour = IconHour::read(&frame, &content).unwrap();
    assert_eq!(
        chosen(&content, &hour, &row, "rifles_1"),
        Ok(Some(Icon::Sprite("GFX_rifles_1_medium".into())))
    );
    let withheld = chosen(&content, &hour, &row, "solo_rifle").unwrap_err();
    assert!(withheld.contains("common/technologies/gone.txt"), "{withheld}");
}
