use super::*;
use crate::test_fixtures::{named_frame, replay, slot_hour};
use replay::frame::TableView;
use replay::rows::TableRows;
use std::sync::atomic::AtomicBool;

#[test]
fn equipment_localisation_keeps_names_past_the_old_length_limit() {
    struct LongName(String);
    impl Texts for LongName {
        fn has(&self, _: &str) -> bool {
            true
        }
        fn text(&self, _: &str) -> Option<String> {
            Some(self.0.clone())
        }
    }
    let texts = LongName("Equipment name ".repeat(30));
    assert_eq!(
        localise_for(&texts, Some("TAG"), "type"),
        Some(texts.0.clone())
    );
    let named = production::Named {
        texts: Some(&texts),
        units: None,
    };
    assert_eq!(named.type_name("type"), Some(texts.0));
}

#[test]
fn equipment_names_preserve_authored_empty_and_key_equal_titles() {
    let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "localisation/english/equipment_l_english.yml",
        "\u{feff}l_english:\n empty_type:0 \"\"\n type:0 \"Base type\"\n GER_type:0 \"\"\n same_type:0 \"same_type\"\n",
    )]);
    assert_eq!(
        localise_for(&catalogue, None, "empty_type"),
        Some(String::new())
    );
    assert_eq!(
        localise_for(&catalogue, Some("GER"), "type"),
        Some(String::new())
    );
    assert_eq!(
        localise_for(&catalogue, None, "same_type").as_deref(),
        Some("same_type")
    );
    assert_eq!(localise_for(&catalogue, None, "missing_type"), None);
    let named = production::Named {
        texts: Some(&catalogue),
        units: None,
    };
    assert_eq!(named.type_name("empty_type"), Some(String::new()));
    assert_eq!(named.type_name("same_type").as_deref(), Some("same_type"));
    assert_eq!(named.type_name("missing_type"), None);
    assert_eq!(
        named.type_name(&format!("{UNRESOLVED_MARK}empty_type")),
        None
    );
    let raw = RawVariant {
        token: "type".into(),
        name_tag: Some("GER".into()),
        type_tag: Some("GER".into()),
        ..Default::default()
    };
    assert_eq!(
        variant_names(&raw, Some(&catalogue)),
        (Some(String::new()), Some(String::new()))
    );
}

#[test]
fn equipment_reader_warnings_keep_every_named_refusal() {
    let mut types = installed_content::equipment::Database::default();
    types.misreads = (0..9)
        .map(|entry| format!("equipment file: refused {entry}"))
        .collect();
    assert_eq!(equipment_warnings(&types), types.misreads);
}

#[test]
fn weekly_units_are_the_game_s_truncated_fixed5() {
    // Speed 3.60045 IC a day and cost 22.5 IC a unit give 1.12014 units a week.
    assert_eq!(weekly_units_raw(360_045, 2_250_000), Some(112_014));
    // Truncated toward zero, and none without a cost.
    assert_eq!(weekly_units_raw(1, 700_001), Some(0));
    assert_eq!(weekly_units_raw(-1, 700_001), Some(0));
    assert_eq!(weekly_units_raw(1, 0), None);
    assert_eq!(weekly_units_raw(i64::MAX, 1), None);
}

#[test]
fn daily_units_are_the_game_s_own_truncation_not_a_seventh_of_the_week() {
    // Speed 3.60045 IC a day and cost 22.5 IC a unit give 0.16002 units a day.
    assert_eq!(daily_units_raw(360_045, 2_250_000), Some(16_002));
    // 24.6609 IC a day at 0.5 IC a unit: 49.3218 a day.
    assert_eq!(daily_units_raw(2_466_090, 50_000), Some(4_932_180));
    // 1.97752 IC a day at 19.504 IC a unit: 0.10139 a day, truncated toward zero.
    assert_eq!(daily_units_raw(197_752, 1_950_400), Some(10_139));
    assert_eq!(daily_units_raw(7, 100_001), Some(6));
    assert_eq!(daily_units_raw(-1, 100_001), Some(0));
    assert_eq!(daily_units_raw(1, 0), None);
    assert_eq!(daily_units_raw(i64::MAX, 1), None);
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
    fn stored_text(&self, text: &str, what: &str) -> Result<String, String> {
        let mut localisation = installed_content::localisation::Localisation::default();
        localisation.language = "english".into();
        crate::scripted_text::stored_text(&localisation, "english", text.as_bytes(), what)
    }
}

#[test]
fn variant_names_use_country_localisation_own_names_and_suffixes() {
    let texts = Fixture(&[
        ("infantry_equipment_1", "Fixture equipment"),
        ("GER_infantry_equipment_1", "Country fixture equipment"),
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
            Some("Country fixture equipment".into()),
            Some("Fixture equipment".into())
        )
    );
    let designed = RawVariant {
        display_name: "Custom design".into(),
        suffix: "Series Q".into(),
        append: true,
        ..raw.clone()
    };
    assert_eq!(
        variant_names(&designed, Some(&texts)).0.as_deref(),
        Some("Custom design Series Q")
    );
    assert_eq!(variant_names(&designed, None), (None, None));
    assert_eq!(variant_names(&raw, None), (None, None));
}

#[test]
fn cached_variant_names_and_suffixes_require_the_recorded_language() {
    let (mut catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[(
        "localisation/english/equipment_l_english.yml",
        "\u{feff}l_english:\n type:0 \"Viewer type\"\n cached_name:0 \"Resolved key\"\n",
    )]);
    let raw = RawVariant {
        id: 17,
        token: "type".into(),
        ..Default::default()
    };
    catalogue.recorded_language = "german".into();
    // Genuine authored keys resolve in the viewer language even for another recording language.
    assert_eq!(
        variant_names(&raw, Some(&catalogue)),
        (Some("Viewer type".into()), Some("Viewer type".into()))
    );
    let own = RawVariant {
        display_name: "cached_name".into(),
        ..raw.clone()
    };
    assert_eq!(variant_names(&own, Some(&catalogue)).0, None);
    let suffix = RawVariant {
        suffix: "cached_name".into(),
        append: true,
        ..raw.clone()
    };
    assert_eq!(variant_names(&suffix, Some(&catalogue)).0, None);
    let warnings = catalogue.refusal_lines();
    assert!(
        warnings
            .iter()
            .any(|why| why.contains("equipment display name")
                && why.contains("german")
                && why.contains("17"))
    );
    assert!(
        warnings
            .iter()
            .any(|why| why.contains("equipment variant suffix") && why.contains("17"))
    );
    catalogue.recorded_language = "english".into();
    // Stored text is used directly, even when its spelling also names a key.
    assert_eq!(
        variant_names(&own, Some(&catalogue)).0.as_deref(),
        Some("cached_name")
    );
    assert_eq!(
        variant_names(&suffix, Some(&catalogue)).0.as_deref(),
        Some("Viewer type cached_name")
    );
}

#[test]
fn invalid_variant_bytes_refuse_names_and_fingerprints_without_dropping_identity() {
    let hour = slot_hour(TableId::VariantDefinitions);
    for field in [
        "display name",
        "suffix",
        "token",
        "name tag",
        "type tag",
        "design",
    ] {
        let mut frame = named_frame(hour, Some(b"Stored name"));
        *frame.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![]));
        *frame.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(vec![]));
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        {
            match field {
                "display name" => rows[0].display_name = vec![0xff],
                "suffix" => {
                    rows[0].variant_name = vec![0xff];
                    rows[0].append_variant_name = 1;
                }
                "name tag" => rows[0].display_name.clear(),
                _ => {}
            }
        }
        if field == "token" || field == "design" {
            if let TableView::Complete(TableRows::LexicalTokens(rows)) =
                &mut *frame.tables[TableId::LexicalTokens.index()]
            {
                if field == "token" {
                    rows[0].literal = vec![0xff];
                } else {
                    rows.push(LexicalTokenRow {
                        token: 99,
                        literal: vec![0xff],
                    });
                }
            }
            if field == "design" {
                *frame.tables[TableId::VariantUpgrades.index()] =
                    TableView::Complete(TableRows::VariantUpgrades(vec![VariantUpgradeRow {
                        variant: 20,
                        source_ordinal: 0,
                        definition_token: 99,
                        level: 1,
                    }]));
            }
        }
        if (field == "name tag" || field == "type tag")
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            if field == "name tag" {
                rows[0].tag = vec![0xff];
            } else {
                rows.push(CountryRow {
                    canonical_slot: 0,
                    tag: vec![0xff],
                    ..Default::default()
                });
            }
        }
        let mut variants = VariantCatalogue::default();
        variants.observe(replay::format::FormatVersion::CURRENT, &frame);
        let raw = variants.current(20).unwrap();
        assert_eq!(raw.id, 20, "{field}");
        assert!(
            raw.refusals.iter().any(|(name, _)| *name == field),
            "{field}: {:?}",
            raw.refusals
        );
        assert!(
            !raw.token.contains('\u{fffd}')
                && !raw.display_name.contains('\u{fffd}')
                && !raw.suffix.contains('\u{fffd}')
        );
        if ["display name", "suffix", "token", "design"].contains(&field) {
            assert_eq!(raw.design, None, "{field}");
        }
        assert!(
            variants
                .warnings()
                .iter()
                .any(|why| why.contains("1 equipment variants") && why.contains("20")),
            "{field}"
        );
        assert!(variants.names(20, None)[0].known, "{field}");
    }
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
        catalogue.observe(replay::format::FormatVersion::CURRENT, &frame);
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
        catalogue.observe(replay::format::FormatVersion::CURRENT, &frame(hour, rows));
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
        catalogue.observe(replay::format::FormatVersion::CURRENT, &frame);
        catalogue.current(20).unwrap().design.clone()
    };
    assert!(modules(1).is_some());
    assert!(modules(0).is_some());
    assert_eq!(modules(999), None);
    // Reads at later hours retain the new levels even with unchanged naming operands.
    let mut catalogue = VariantCatalogue::default();
    catalogue.observe(
        replay::format::FormatVersion::CURRENT,
        &frame(hour, Some(vec![upgrade(20, 0, 1, 1)])),
    );
    catalogue.observe(
        replay::format::FormatVersion::CURRENT,
        &frame(hour + 1, Some(vec![upgrade(20, 0, 1, 2)])),
    );
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
    catalogue.observe(
        replay::format::FormatVersion::CURRENT,
        &named_frame(hour, Some(b"Name")),
    );
    catalogue.observe(
        replay::format::FormatVersion::CURRENT,
        &carried(hour + 1, Some(b"Name")),
    );
    assert_eq!(catalogue.current(20).unwrap().display_name, "Name");
    // The recording marked the carried table unavailable: unknown from here, and
    // still unknown once it is complete again, until the next read.
    catalogue.observe(
        replay::format::FormatVersion::CURRENT,
        &carried(hour + 2, None),
    );
    assert!(catalogue.current(20).is_none());
    catalogue.observe(
        replay::format::FormatVersion::CURRENT,
        &carried(hour + 3, Some(b"Name")),
    );
    assert!(catalogue.current(20).is_none());
    catalogue.observe(
        replay::format::FormatVersion::CURRENT,
        &named_frame(hour + 4, Some(b"Renamed")),
    );
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
