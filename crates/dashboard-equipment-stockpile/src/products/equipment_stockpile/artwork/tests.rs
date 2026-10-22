use super::*;
use dashboard_core::test_fixtures::{equipment_types, stockpile_frame};
use replay_data::{
    rows::AvailableEquipmentVariantRow, rows::CountryLinkRow, rows::LexicalTokenRow, rows::TableId,
    rows::TableRows, rows::VariantDefinitionRow, rows::VariantUpgradeRow,
};
use replay_frame::frame::TableView;

const A: u64 = (1 << 32) | 1;
const B: u64 = (2 << 32) | 1;
const C: u64 = (3 << 32) | 1;

fn database() -> Database {
    equipment_types(&[
        b"equipments = { infantry_equipment = { is_archetype = yes type = infantry }
        infantry_equipment_1 = { archetype = infantry_equipment parent = infantry_equipment }
        infantry_equipment_2 = { archetype = infantry_equipment parent = infantry_equipment_1 } }",
    ])
    .unwrap()
}

fn candidate(ordinal: u32, variant: u64) -> AvailableEquipmentVariantRow {
    AvailableEquipmentVariantRow {
        country: 11,
        roster_kind: 2,
        ordinal,
        variant,
    }
}

fn frame() -> DecodedFrame {
    // The most-stocked design is not the native best-variant selector's input.
    let mut frame = stockpile_frame(1000, false, &[(A, i64::MAX), (B, 1), (C, 0)], Some(&[]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"infantry_equipment_1".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"infantry_equipment_2".to_vec(),
            },
        ]));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![
            VariantDefinitionRow {
                id: A,
                equipment_token: 1,
                creator_link: 9001,
                ..Default::default()
            },
            VariantDefinitionRow {
                id: B,
                equipment_token: 2,
                creator_link: 21,
                obsolete: 1,
                ..Default::default()
            },
            VariantDefinitionRow {
                id: C,
                equipment_token: 2,
                creator_link: 9001,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(Vec::new()));
    *frame.tables[TableId::AvailableEquipmentVariants.index()] =
        TableView::Complete(TableRows::AvailableEquipmentVariants(vec![
            candidate(0, A),
            candidate(1, B),
            candidate(2, C),
        ]));
    frame
}

#[test]
fn best_uses_depth_upgrade_owner_and_native_id_instead_of_stock_or_obsolete() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let database = database();
    let mut frame = frame();
    let best = |frame: &DecodedFrame| best_variants(frame, &database, 9001, 7).unwrap()["infantry_equipment"];
    // Depth beats quantity, then the country's own creator beats a foreign one.
    assert_eq!(best(&frame), C);
    *frame.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(vec![VariantUpgradeRow {
            variant: B,
            level: 255,
            ..Default::default()
        }]));
    // Unsigned upgrade levels precede creator preference; obsolete is not filtered.
    assert_eq!(best(&frame), B);
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *frame.tables[TableId::VariantDefinitions.index()]
    {
        rows[0].equipment_token = 2;
    }
    *frame.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(Vec::new()));
    assert_eq!(best(&frame), C); // larger native ID among the same creator country
    *frame.tables[TableId::AvailableEquipmentVariants.index()] =
        TableView::Complete(TableRows::AvailableEquipmentVariants(vec![
            candidate(0, A),
            AvailableEquipmentVariantRow {
                roster_kind: 0,
                ..candidate(1, C)
            },
        ]));
    assert_eq!(best(&frame), A); // primary roster entries are not stored best candidates
}

#[test]
fn incomplete_candidates_refuse_a_guess_and_count_named_failures() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = frame();
    *frame.tables[TableId::AvailableEquipmentVariants.index()] =
        TableView::Complete(TableRows::AvailableEquipmentVariants(vec![
            candidate(0, A),
            candidate(1, 999),
            candidate(2, 999),
            AvailableEquipmentVariantRow {
                country: 404,
                ..candidate(3, C)
            },
        ]));
    let reason = best_variants(&frame, &database(), 9001, 7).unwrap_err();
    assert!(
        reason.contains("2 refused: best candidate 999 has no recorded definition"),
        "{reason}"
    );
    assert!(
        reason.contains("1 refused: best-candidate country link 404 is not recorded"),
        "{reason}"
    );
    *frame.tables[TableId::AvailableEquipmentVariants.index()] = TableView::Unavailable {
        reason: "old recording".into(),
    };
    assert!(best_variants(&frame, &database(), 9001, 7).is_err());
}

#[test]
fn foreign_country_candidates_do_not_hide_the_selected_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frame = frame();
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.extend((100..124).map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        }));
        rows.sort_by_key(|row| row.id);
    }
    let mut candidates: Vec<_> = (100..124)
        .map(|country| AvailableEquipmentVariantRow {
            country,
            ..candidate(0, C)
        })
        .collect();
    candidates.push(candidate(24, B));
    *frame.tables[TableId::AvailableEquipmentVariants.index()] =
        TableView::Complete(TableRows::AvailableEquipmentVariants(candidates));
    assert_eq!(
        best_variants(&frame, &database(), 9001, 7).unwrap()["infantry_equipment"],
        B
    );
}

/// The fixture frame with every table the reply reads recorded: Germany's appearance,
/// ruling party, politics and capital state, Italy, and icon entries that reach the
/// creator's pool and the fallback.
fn recorded_frame(hour: i32, ruling: &[u8]) -> DecodedFrame {
    use replay_data::rows::{
        CountryAppearanceRow, CountryRow, PoliticalPartyRow, PoliticsRow, SimulationFeatureRow,
        StateRow,
    };
    let mut frame = frame();
    frame.hour = hour;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 21,
            canonical_slot: 8,
            tag: b"ITA".to_vec(),
            ..Default::default()
        });
        rows.sort_by_key(|row| row.id);
    }
    let appearance = |country, culture: &[u8]| CountryAppearanceRow {
        country,
        graphical_culture: culture.to_vec(),
        graphical_culture_2d: b"western_european_2d".to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            appearance(21, b"southern_european_gfx"),
            appearance(9001, b"western_european_gfx"),
        ]));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 9001,
            id: 1,
            is_ruling: 1,
            token: ruling.to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::Politics.index()] =
        TableView::Complete(TableRows::Politics(vec![PoliticsRow {
            country: 9001,
            capital: 1,
            ruling_party: Some(1),
            ..Default::default()
        }]));
    *frame.tables[TableId::Factions.index()] = TableView::Complete(TableRows::Factions(Vec::new()));
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(Vec::new()));
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner: 9001,
            controller: 9001,
            ..Default::default()
        }]));
    *frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: u32::MAX,
            ..Default::default()
        }]));
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *frame.tables[TableId::VariantDefinitions.index()]
    {
        for row in rows {
            row.icon_active = 1;
            (row.icon_kind, row.icon) = if row.id == C {
                (0, b"GFX_infantry_equipment_test".to_vec())
            } else {
                (1, b".".to_vec())
            };
        }
    }
    frame
}

#[test]
fn the_reply_from_the_shared_frame_is_the_reply_from_the_whole_frame() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!("stockpile-artwork-{}", uuid::Uuid::new_v4()));
    let _folder = dashboard_text::catalogue::tests::Folder(root.clone());
    // The equipment types and the map, defines and continents icon content reads.
    for (path, text) in [
        (
            "common/units/equipment/test.txt",
            "equipments = { infantry_equipment = { is_archetype = yes type = infantry }
            infantry_equipment_1 = { archetype = infantry_equipment parent = infantry_equipment }
            infantry_equipment_2 = { archetype = infantry_equipment parent = infantry_equipment_1 } }",
        ),
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continent.txt",
        ),
        ("map/definition.csv", "0;0;0;0;land;false;unknown;0\n"),
        ("map/continent.txt", "continents={synthetic_continent}"),
        (
            "common/defines/synthetic.lua",
            "NDefines={NBuildings={MAX_BUILDING_LEVELS=15}}",
        ),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    // The ruling party changes between the frames, so the queried hour is reconstructed
    // through a change rather than read from the keyframe alone.
    let frames = vec![
        recorded_frame(1000, b"fascism"),
        recorded_frame(1001, b"democratic"),
    ];
    let (source, context) = dashboard_base::test_fixtures::source_with_content(frames, &root);
    let cancelled = AtomicBool::new(false);
    for (hour, variants) in [
        (1000, vec![]),
        (1001, vec![]),
        (1001, vec![A.to_string(), C.to_string()]),
    ] {
        let params = StockpileArtworkParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            hour,
            variants,
        };
        let whole =
            StockpileArtwork::load_with(&source, &params, &context, &cancelled, |branch, hour| {
                source.replay_ref().seek(branch, hour)
            })
            .unwrap();
        let shared = StockpileArtwork::load(&source, &params, &context, &cancelled).unwrap();
        assert!(!whole.entries.is_empty(), "{:?}", whole.warnings);
        assert_eq!(
            serde_json::to_value(&shared).unwrap(),
            serde_json::to_value(&whole).unwrap()
        );
    }
    // Each hour was reconstructed once through the shared cache.
    assert_eq!(dashboard_base::prepared::query::frames::reads(&source).1, 2);
}
