use super::*;
use crate::test_fixtures::{named_frame, replay, slot_hour};
use replay::frame::TableView;
use replay::rows::TableRows;
use std::sync::atomic::AtomicBool;

#[test]
fn weekly_units_are_the_game_s_truncated_fixed5() {
    // Germany's Bf 109 E line in the W3 save: speed 3.60045, cost 22.5 IC.
    assert_eq!(weekly_units_raw(360_045, 2_250_000), Some(112_014));
    // Truncated toward zero, and none without a cost.
    assert_eq!(weekly_units_raw(1, 700_001), Some(0));
    assert_eq!(weekly_units_raw(-1, 700_001), Some(0));
    assert_eq!(weekly_units_raw(1, 0), None);
    assert_eq!(weekly_units_raw(i64::MAX, 1), None);
}

#[test]
fn domains_follow_the_engine_s_plane_and_ship_families() {
    assert_eq!(Domain::of(1 << 10), Domain::Air); // fighter
    assert_eq!(Domain::of(1 << 32), Domain::Air); // heavy fighter
    assert_eq!(Domain::of(1), Domain::Naval); // convoy
    assert_eq!(Domain::of(1 << 6), Domain::Naval); // capital ship
    assert_eq!(Domain::of(1 << 5), Domain::Land); // infantry
    assert_eq!(Domain::of(1 << 31), Domain::Land); // train
    assert_eq!(Domain::of(0), Domain::Land);
}

struct Fixture(&'static [(&'static str, &'static str)]);
impl Texts for Fixture {
    fn has(&self, key: &str) -> bool {
        self.0.iter().any(|(k, _)| *k == key)
    }
    fn text(&self, key: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_owned())
    }
}

#[test]
fn variant_names_follow_get_display_name() {
    let texts = Fixture(&[
        ("infantry_equipment_1", "Infantry Equipment I"),
        ("GER_infantry_equipment_1", "Gewehr 98"),
    ]);
    let raw = RawVariant {
        token: "infantry_equipment_1".into(),
        name_tag: Some("GER".into()),
        type_tag: Some("ENG".into()),
        ..Default::default()
    };
    assert_eq!(
        variant_names(&raw, Some(&texts)),
        (
            Some("Gewehr 98".into()),
            Some("Infantry Equipment I".into())
        )
    );
    let designed = RawVariant {
        display_name: "Panzer II".into(),
        suffix: "Ausf. F".into(),
        append: true,
        ..raw.clone()
    };
    assert_eq!(
        variant_names(&designed, None).0.as_deref(),
        Some("Panzer II Ausf. F")
    );
    assert_eq!(variant_names(&raw, None), (None, None));
}

#[test]
fn variant_names_stop_being_known_at_a_gap_or_a_read_that_did_not_land() {
    use crate::products::equipment_stockpile::scan;
    let hour = slot_hour(TableId::VariantDefinitions);
    let mut carried = named_frame(hour + 1, Some(b"Old name"));
    carried.metrics.table_ns[TableId::VariantModules.index()] = 0;
    let frames = vec![
        named_frame(hour, Some(b"Old name")),
        carried,
        // A gap (hours +2 to +4), then a due read that did not land: renamed meanwhile,
        // the old name must not stay.
        named_frame(hour + 5, None),
        // No gap, but a due read that did not land.
        named_frame(hour + 6, Some(b"New name")),
        named_frame(hour + 7, None),
        named_frame(hour + 8, Some(b"New name")),
    ];
    let scanned = scan(&replay(frames), 1, "GER", &AtomicBool::new(false)).unwrap();
    let versions: Vec<(i32, Option<String>)> = scanned.variants.versions[&20]
        .iter()
        .map(|version| {
            (
                version.first_hour - hour,
                version.raw.as_ref().map(|raw| raw.display_name.clone()),
            )
        })
        .collect();
    assert_eq!(
        versions,
        [
            (0, Some("Old name".to_owned())),
            // Known until the gap: unknown from its first missing hour.
            (2, None),
            (6, Some("New name".to_owned())),
            (7, None),
            (8, Some("New name".to_owned())),
        ]
    );
    assert_eq!(scanned.variants.token(20), "infantry_equipment_1");
}

#[test]
fn a_variant_s_design_fingerprint_follows_its_name_and_modules() {
    use crate::products::equipment::VariantCatalogue;
    use replay::rows::VariantModuleRow;
    let hour = slot_hour(TableId::VariantDefinitions);
    let module = |source_ordinal, slot, definition_token| VariantModuleRow {
        variant: 20,
        source_ordinal,
        slot,
        definition_token,
        definition_active: 1,
    };
    let design = |name: &[u8], modules: Option<Vec<VariantModuleRow>>| {
        let mut frame = named_frame(hour, Some(name));
        *frame.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(vec![]));
        *frame.tables[TableId::VariantModules.index()] = match modules {
            Some(rows) => TableView::Complete(TableRows::VariantModules(rows)),
            None => TableView::Unavailable {
                reason: "fixture".into(),
            },
        };
        let mut catalogue = VariantCatalogue::default();
        catalogue.observe(&frame);
        catalogue.current(20).unwrap().design.clone()
    };
    let design_a = design(b"Panzer", Some(vec![module(0, 1, 1)])).unwrap();
    // The same design reads the same wherever it is read.
    assert_eq!(
        design(b"Panzer", Some(vec![module(0, 1, 1)])).as_deref(),
        Some(design_a.as_str())
    );
    assert_eq!(design_a.len(), 16);
    // Another name, or other modules, is another design.
    assert_ne!(
        design(b"Panther", Some(vec![module(0, 1, 1)])).unwrap(),
        design_a
    );
    assert_ne!(
        design(b"Panzer", Some(vec![module(0, 1, 1), module(1, 1, 1)])).unwrap(),
        design_a
    );
    // A complete table without the variant's rows: no modules, still a design.
    assert!(design(b"Panzer", Some(vec![])).is_some());
    // Without the module table the design is not known.
    assert_eq!(design(b"Panzer", None), None);
}

#[test]
fn variant_designs_include_recorded_upgrades_and_missing_is_not_zero() {
    use crate::products::equipment::VariantCatalogue;
    use replay::rows::VariantUpgradeRow;
    let hour = slot_hour(TableId::VariantUpgrades);
    let upgrade = |variant, source_ordinal, definition_token, level| VariantUpgradeRow {
        variant,
        source_ordinal,
        definition_token,
        level,
    };
    let frame = |at, upgrades: Option<Vec<VariantUpgradeRow>>| {
        let mut frame = named_frame(at, Some(b"Same design name"));
        *frame.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![]));
        *frame.tables[TableId::VariantUpgrades.index()] = match upgrades {
            Some(rows) => TableView::Complete(TableRows::VariantUpgrades(rows)),
            None => TableView::Unavailable {
                reason: "fixture".into(),
            },
        };
        frame
    };
    let design = |rows| {
        let mut catalogue = VariantCatalogue::default();
        catalogue.observe(&frame(hour, rows));
        catalogue.current(20).unwrap().design.clone()
    };
    let level_one = design(Some(vec![upgrade(20, 0, 1, 1)])).unwrap();
    let level_two = design(Some(vec![upgrade(20, 0, 1, 2)])).unwrap();
    assert_ne!(level_one, level_two);
    assert_ne!(level_one, design(Some(vec![upgrade(20, 0, 1, 0)])).unwrap());
    let none = design(Some(vec![])).unwrap();
    assert_ne!(none, design(Some(vec![upgrade(20, 0, 1, 0)])).unwrap());
    assert_eq!(design(None), None);
    // Other variants' upgrades do not enter this design.
    assert_eq!(design(Some(vec![upgrade(99, 0, 1, 4)])), Some(none));
    // The recorded ordinal decides the order, never a caller's row order.
    assert_eq!(
        design(Some(vec![upgrade(20, 1, 1, 2), upgrade(20, 0, 1, 1)])),
        design(Some(vec![upgrade(20, 0, 1, 1), upgrade(20, 1, 1, 2)]))
    );
    // An unspelled upgrade is an unknown design, not an empty definition.
    assert_eq!(design(Some(vec![upgrade(20, 0, 999, 1)])), None);
    // So is an unspelled module; token 0 is no module, a known empty spelling.
    let modules = |definition_token| {
        let mut frame = frame(hour, Some(vec![]));
        *frame.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![
                replay::rows::VariantModuleRow {
                    variant: 20,
                    source_ordinal: 0,
                    slot: 1,
                    definition_token,
                    definition_active: 1,
                },
            ]));
        let mut catalogue = VariantCatalogue::default();
        catalogue.observe(&frame);
        catalogue.current(20).unwrap().design.clone()
    };
    assert!(modules(1).is_some());
    assert!(modules(0).is_some());
    assert_eq!(modules(999), None);
    // Reads at later hours retain the new levels even with unchanged naming operands.
    let mut catalogue = VariantCatalogue::default();
    catalogue.observe(&frame(hour, Some(vec![upgrade(20, 0, 1, 1)])));
    catalogue.observe(&frame(hour + 1, Some(vec![upgrade(20, 0, 1, 2)])));
    assert_eq!(catalogue.versions[&20].len(), 2);
    assert_eq!(
        catalogue.current(20).unwrap().design.as_deref(),
        Some(level_two.as_str())
    );
    // The product's selected-table replay decode must retain the upgrade table too.
    for (rows, expected) in [
        (Some(vec![upgrade(20, 0, 1, 1)]), Some(level_one)),
        (Some(vec![upgrade(20, 0, 1, 2)]), Some(level_two)),
        (None, None),
    ] {
        let scanned = crate::products::equipment_stockpile::scan(
            &replay(vec![frame(hour, rows)]),
            1,
            "GER",
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(scanned.variants.current(20).unwrap().design, expected);
    }
}

#[test]
fn an_hour_between_reads_whose_variant_table_is_not_complete_ends_what_is_known() {
    use crate::products::equipment::VariantCatalogue;
    let hour = slot_hour(TableId::VariantDefinitions);
    // Not due: the carried table, complete or not.
    let carried = |at: i32, name: Option<&[u8]>| {
        let mut frame = named_frame(at, name);
        frame.metrics.table_ns[TableId::VariantModules.index()] = 0;
        frame
    };
    let mut catalogue = VariantCatalogue::default();
    catalogue.observe(&named_frame(hour, Some(b"Name")));
    catalogue.observe(&carried(hour + 1, Some(b"Name")));
    assert_eq!(catalogue.current(20).unwrap().display_name, "Name");
    // The recording marked the carried table unavailable: unknown from here, and
    // still unknown once it is complete again, until the next read.
    catalogue.observe(&carried(hour + 2, None));
    assert!(catalogue.current(20).is_none());
    catalogue.observe(&carried(hour + 3, Some(b"Name")));
    assert!(catalogue.current(20).is_none());
    catalogue.observe(&named_frame(hour + 4, Some(b"Renamed")));
    assert_eq!(catalogue.current(20).unwrap().display_name, "Renamed");
    let versions: Vec<(i32, bool)> = catalogue.versions[&20]
        .iter()
        .map(|version| (version.first_hour - hour, version.raw.is_some()))
        .collect();
    assert_eq!(versions, [(0, true), (2, false), (4, true)]);
    assert_eq!(catalogue.current(99), None);
}

#[test]
fn a_read_after_a_gap_starts_after_the_unknown_stretch_never_at_its_hour() {
    use crate::products::equipment_stockpile::scan;
    let hour = slot_hour(TableId::VariantDefinitions);
    // Known at +0, the recording skips to +9, which reads the variants again.
    let frames = vec![
        named_frame(hour, Some(b"Name")),
        named_frame(hour + 9, Some(b"Name")),
    ];
    let scanned = scan(&replay(frames), 1, "GER", &AtomicBool::new(false)).unwrap();
    let hours: Vec<(i32, bool)> = scanned.variants.versions[&20]
        .iter()
        .map(|version| (version.first_hour - hour, version.raw.is_some()))
        .collect();
    assert_eq!(hours, [(0, true), (1, false), (9, true)]);
    // Strictly increasing, as the reply requires.
    assert!(hours.windows(2).all(|pair| pair[0].0 < pair[1].0));
}
