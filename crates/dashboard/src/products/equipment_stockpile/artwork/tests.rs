use super::*;
use crate::test_fixtures::{equipment_types, stockpile_frame};
use replay::{
    frame::TableView,
    rows::{
        AvailableEquipmentVariantRow, CountryLinkRow, LexicalTokenRow, TableId, TableRows,
        VariantDefinitionRow, VariantUpgradeRow,
    },
};

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
fn five_thousand_foreign_country_candidates_do_not_hide_the_selected_country() {
    let mut frame = frame();
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.extend((100..5100).map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        }));
        rows.sort_by_key(|row| row.id);
    }
    let mut candidates: Vec<_> = (100..5100)
        .map(|country| AvailableEquipmentVariantRow {
            country,
            ..candidate(0, C)
        })
        .collect();
    candidates.push(candidate(5000, B));
    *frame.tables[TableId::AvailableEquipmentVariants.index()] =
        TableView::Complete(TableRows::AvailableEquipmentVariants(candidates));
    assert_eq!(
        best_variants(&frame, &database(), 9001, 7).unwrap()["infantry_equipment"],
        B
    );
}
