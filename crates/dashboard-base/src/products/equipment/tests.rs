use super::*;
use crate::test_fixtures::{named_frame, slot_hour};
use replay_data::{rows::CountryRow, rows::TableRows};
use replay_frame::frame::TableView;

#[test]
fn variant_reads_keep_used_spellings_without_holding_the_dictionary_baseline() {
    let mut frame = named_frame(slot_hour(TableId::VariantDefinitions), Some(b"Named"));
    *frame.tables[TableId::VariantModules.index()] =
        TableView::Complete(TableRows::VariantModules(vec![VariantModuleRow {
            variant: 20,
            slot: 2,
            definition_token: 3,
            definition_active: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(vec![VariantUpgradeRow {
            variant: 20,
            definition_token: 4,
            level: 1,
            ..Default::default()
        }]));
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    {
        rows.extend([3, 4, 99].map(|token| LexicalTokenRow {
            token,
            literal: format!("token-{token}").into_bytes(),
        }));
    }
    let mut reads = VariantReads::default();
    let original = reads.build(&frame, false);
    assert_eq!(original.len(), 1);
    let old = std::mem::replace(
        &mut frame.tables[TableId::LexicalTokens.index()],
        TableView::Unavailable {
            reason: "replacing dictionary".into(),
        }
        .into(),
    );
    let TableView::Complete(TableRows::LexicalTokens(mut rows)) = old.try_unwrap().unwrap() else {
        panic!("variant reads must leave the decoder's dictionary unshared")
    };
    rows.last_mut().unwrap().literal = b"unrelated change".to_vec();
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(rows));
    frame.hour += 24;
    assert!(
        reads.build(&frame, false).is_empty(),
        "unused tokens do not rebuild names"
    );
    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    {
        rows[2].literal = b"changed module".to_vec();
    }
    frame.hour += 24;
    let changed = reads.build(&frame, false);
    assert_eq!(changed.len(), 1);
    assert_ne!(changed[0].1.design, original[0].1.design);
}

#[test]
fn variant_snapshot_spellings_and_comparisons_follow_only_current_tokens() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut input = named_frame(117, Some(b"mod design"));
    *input.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(vec![]));
    *input.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        (1..=67)
            .map(|token| LexicalTokenRow {
                token,
                literal: format!("mod_token_{token}").into_bytes(),
            })
            .collect(),
    ));
    let (mut encoder, mut decoder, mut tracker) = (
        FrameEncoder::new(),
        FrameDecoder::new(),
        FrameChangeTracker::new(),
    );
    let (mut actual, mut hourly) = (VariantCatalogue::default(), VariantCatalogue::default());
    for offset in 0..64 {
        input.hour = 117 + offset;
        let token = offset as u32 + 3;
        *input.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![VariantModuleRow {
                variant: 20,
                slot: 2,
                definition_token: token,
                definition_active: 1,
                ..Default::default()
            }]));
        // Force a new dictionary identity without changing any used spelling.
        if let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut *input.tables[TableId::LexicalTokens.index()]
        {
            rows.last_mut().unwrap().literal = format!("unused_{offset}").into_bytes();
        }
        let wire = Arc::new(encoder.encode(input.clone(), true).unwrap());
        let frame = decoder.advance(&wire).unwrap();
        let changes = tracker.observe_updates(frame, wire.clone(), format);
        let before = actual.work.row_visits;
        let found = actual.observe_changed(format, frame, &changes);
        let expected = hourly.observe_versions(format, frame, None);
        assert_eq!(
            serde_json::to_vec(&found).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&hourly).unwrap()
        );
        if offset > 0 {
            assert!(
                actual.work.row_visits - before <= 8,
                "read {offset} must compare only the three currently used spellings and naming rows"
            );
        }
        let referenced: BTreeSet<_> = actual.reads.inputs[&20].tokens().collect();
        let spellings = &actual.reads.lexical.as_ref().unwrap().1;
        assert!(spellings.len() <= referenced.len());
        assert_eq!(
            spellings.keys().copied().collect::<BTreeSet<_>>(),
            referenced
        );
        assert_eq!(actual.reads.token_variants.len(), referenced.len());
        assert!(
            actual
                .reads
                .token_variants
                .values()
                .all(|variants| variants == &BTreeSet::from([20]))
        );
    }
}

#[test]
fn variant_dependencies_release_definitions_on_deletes_snapshots_and_restore() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay_data::{rows::CountryAppearanceRow, rows::CountryLinkRow};
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    let format = replay_data::format::FormatVersion::CURRENT;
    for snapshots in [false, true] {
        let mut input = named_frame(117, Some(b"mod design"));
        *input.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                (1..=16)
                    .map(|token| LexicalTokenRow {
                        token,
                        literal: format!("mod_token_{token}").into_bytes(),
                    })
                    .collect(),
            ));
        let (mut encoder, mut decoder, mut tracker) = (
            FrameEncoder::new(),
            FrameDecoder::new(),
            FrameChangeTracker::new(),
        );
        let (mut actual, mut hourly) = (VariantCatalogue::default(), VariantCatalogue::default());
        for (offset, count) in [2, 1, 1, 0, 1, 1, 0].into_iter().enumerate() {
            input.hour = 117 + offset as i32;
            let generation = offset as u32 / 2;
            let raw = 100 + generation;
            let slot = 1 + generation;
            let token = 1 + 4 * generation;
            let origin = offset == 5;
            let ids: BTreeSet<_> = [20, 30].into_iter().take(count).collect();
            *input.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(vec![CountryRow {
                    id: 1,
                    canonical_slot: slot,
                    tag: b"mod country".to_vec(),
                    ..Default::default()
                }]));
            *input.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                    id: raw,
                    canonical_slot: slot,
                }]));
            *input.tables[TableId::CountryAppearance.index()] =
                TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
                    country: raw,
                    name_tag: b"mod original".to_vec(),
                    ..Default::default()
                }]));
            *input.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(
                    ids.iter()
                        .map(|&id| VariantDefinitionRow {
                            id,
                            equipment_token: token,
                            creator_link: raw,
                            origin_link: if origin { raw } else { 0 },
                            ..Default::default()
                        })
                        .collect(),
                ));
            // Orphan design rows must not keep a deleted definition's operands.
            *input.tables[TableId::VariantModules.index()] =
                TableView::Complete(TableRows::VariantModules(
                    [20, 30]
                        .map(|variant| VariantModuleRow {
                            variant,
                            slot: token + 1,
                            definition_token: token + 2,
                            definition_active: 1,
                            ..Default::default()
                        })
                        .to_vec(),
                ));
            *input.tables[TableId::VariantUpgrades.index()] =
                TableView::Complete(TableRows::VariantUpgrades(
                    [20, 30]
                        .map(|variant| VariantUpgradeRow {
                            variant,
                            definition_token: (token + 3) as i32,
                            level: 1,
                            ..Default::default()
                        })
                        .to_vec(),
                ));
            if [4, 6].contains(&offset) {
                actual = serde_json::from_slice(&serde_json::to_vec(&actual).unwrap()).unwrap();
                assert!(actual.reads.inputs.is_empty());
                assert!(actual.reads.lexical.is_none());
                assert!(actual.reads.token_variants.is_empty());
                assert!(actual.reads.tag_links.is_empty());
                assert!(actual.reads.tag_slots.is_empty());
                assert!(actual.reads.original_slots.is_empty());
                tracker.invalidate();
            }
            let wire = Arc::new(
                encoder
                    .encode(input.clone(), snapshots || offset == 0)
                    .unwrap(),
            );
            let frame = decoder.advance(&wire).unwrap();
            let changes = tracker.observe_updates(frame, wire.clone(), format);
            let found = actual.observe_changed(format, frame, &changes);
            let expected = hourly.observe_versions(format, frame, None);
            assert_eq!(
                serde_json::to_vec(&found).unwrap(),
                serde_json::to_vec(&expected).unwrap()
            );
            assert_eq!(
                serde_json::to_vec(&actual).unwrap(),
                serde_json::to_vec(&hourly).unwrap(),
                "snapshots={snapshots}, read {offset}"
            );
            let reads = &actual.reads;
            assert_eq!(reads.inputs.keys().copied().collect::<BTreeSet<_>>(), ids);
            assert_eq!(
                reads
                    .inputs
                    .values()
                    .map(|input| {
                        let (modules, upgrades) = input.design.as_ref().unwrap();
                        modules.len() + upgrades.len()
                    })
                    .sum::<usize>(),
                2 * ids.len()
            );
            let tokens: BTreeSet<_> = (token..token + 4).filter(|_| !ids.is_empty()).collect();
            assert_eq!(
                reads
                    .lexical
                    .as_ref()
                    .unwrap()
                    .1
                    .keys()
                    .copied()
                    .collect::<BTreeSet<_>>(),
                tokens
            );
            assert_eq!(
                reads.token_variants,
                tokens
                    .into_iter()
                    .map(|token| (token, ids.clone()))
                    .collect::<BTreeMap<_, _>>()
            );
            let links = if ids.is_empty() {
                BTreeMap::new()
            } else {
                BTreeMap::from([(raw, ids.clone())])
            };
            let slots = if ids.is_empty() {
                BTreeMap::new()
            } else {
                BTreeMap::from([(slot, ids.clone())])
            };
            assert_eq!(reads.tag_links, links);
            assert_eq!(reads.tag_slots, slots);
            assert_eq!(
                reads.original_slots,
                if origin { BTreeMap::new() } else { slots }
            );
        }
    }
}

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
    let mut types = installed_content_core::equipment::Database::default();
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
        let mut localisation = installed_content_localisation::localisation::Localisation::default();
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
        variants.observe(replay_data::format::FormatVersion::CURRENT, &frame);
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
fn a_variant_s_design_fingerprint_follows_its_name_and_modules() {
    use crate::products::equipment::VariantCatalogue;
    use replay_data::rows::VariantModuleRow;
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
        catalogue.observe(replay_data::format::FormatVersion::CURRENT, &frame);
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
        replay_data::format::FormatVersion::CURRENT,
        &named_frame(hour, Some(b"Name")),
    );
    catalogue.observe(
        replay_data::format::FormatVersion::CURRENT,
        &carried(hour + 1, Some(b"Name")),
    );
    assert_eq!(catalogue.current(20).unwrap().display_name, "Name");
    // The recording marked the carried table unavailable: unknown from here, and
    // still unknown once it is complete again, until the next read.
    catalogue.observe(
        replay_data::format::FormatVersion::CURRENT,
        &carried(hour + 2, None),
    );
    assert!(catalogue.current(20).is_none());
    catalogue.observe(
        replay_data::format::FormatVersion::CURRENT,
        &carried(hour + 3, Some(b"Name")),
    );
    assert!(catalogue.current(20).is_none());
    catalogue.observe(
        replay_data::format::FormatVersion::CURRENT,
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

fn catalogue_changes_carry_unchanged_operands(count: u32) {
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay_data::rows::CountryLinkRow;
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut input = named_frame(slot_hour(TableId::VariantDefinitions), Some(b"mod design"));
    *input.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod country {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *input.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    let definition = input.complete::<tables::VariantDefinitions>().unwrap()[0].clone();
    *input.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            (1..=count)
                .map(|id| VariantDefinitionRow {
                    id: u64::from(id),
                    creator_link: 1,
                    ..definition.clone()
                })
                .collect(),
        ));
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut tracker = FrameChangeTracker::new();
    let mut changed = VariantCatalogue::default();
    let mut whole = VariantCatalogue::default();
    let first = input.hour;
    let mut refused = None;
    for offset in 0..=18 {
        if offset == 12 {
            continue;
        }
        input.hour = first + offset;
        input.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
        match offset {
            1 => {
                if let TableView::Complete(TableRows::LexicalTokens(rows)) =
                    &mut *input.tables[TableId::LexicalTokens.index()]
                {
                    rows.push(LexicalTokenRow {
                        token: 1_000_000,
                        literal: b"unused token".to_vec(),
                    });
                }
            }
            2 => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                {
                    rows[0].political_power_raw = Some(123);
                }
            }
            3 => {
                if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                    &mut *input.tables[TableId::VariantDefinitions.index()]
                {
                    rows[0].display_name = b"renamed design".to_vec();
                }
            }
            6 => {
                refused = Some(std::mem::replace(
                    &mut input.tables[TableId::VariantDefinitions.index()],
                    TableView::Unavailable {
                        reason: "fixture refusal".into(),
                    }
                    .into(),
                ));
            }
            7 => {
                input.tables[TableId::VariantDefinitions.index()] = refused.take().unwrap();
                input.metrics.table_ns[TableId::VariantDefinitions.index()] = 0;
                input.metrics.table_ns[TableId::VariantModules.index()] = 0;
            }
            10 => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                {
                    rows[0].tag = b"mod transferred creator".to_vec();
                }
            }
            15 => {
                if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                    &mut *input.tables[TableId::VariantDefinitions.index()]
                {
                    rows.remove(0);
                }
            }
            _ => {}
        }
        let wire = Arc::new(
            encoder
                .encode(input.clone(), offset == 0 || offset == 5 || offset == 11)
                .unwrap(),
        );
        let frame = decoder.advance(&wire).unwrap();
        let changes = tracker.observe_updates(frame, Arc::clone(&wire), format);
        let before = changed.work.row_visits;
        let actual = changed.observe_changed(format, frame, &changes);
        let expected = whole.observe_versions(format, frame, None);
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&expected).unwrap(),
            "changed versions at {offset}"
        );
        assert_eq!(
            serde_json::to_vec(&changed).unwrap(),
            serde_json::to_vec(&whole).unwrap(),
            "hour {offset}"
        );
        if [1, 2, 4].contains(&offset) {
            assert!(
                changed.work.row_visits - before <= 1,
                "unchanged operands must not walk {count} countries or variants at hour {offset}"
            );
        }
        if offset == 3 {
            assert!(
                changed.work.row_visits - before <= 3,
                "one renamed definition must not revisit the other {count} definitions or countries"
            );
        }
        if [4, 11].contains(&offset) {
            changed = serde_json::from_slice(&serde_json::to_vec(&changed).unwrap()).unwrap();
            tracker.invalidate();
        }
    }
}

#[test]
fn catalogue_changes_match_whole_reads_across_snapshots_resume_append_gaps_and_refusals() {
    catalogue_changes_carry_unchanged_operands(8);
}

#[test]
fn catalogue_unchanged_operands_do_not_walk_5000_countries_or_variants() {
    catalogue_changes_carry_unchanged_operands(5000);
}

#[test]
fn variant_weak_country_identity_cannot_clear_pending_invalidation() {
    use crate::prepared::frame_changes::FrameChanges;
    for invalidation in [
        "identity",
        "delete",
        "count",
        "snapshot",
        "gap",
        "unavailable",
    ] {
        let first = named_frame(117, Some(b"mod design"));
        let mut reads = VariantReads::default();
        reads.build(&first, false);
        let keys = reads.keys.as_ref().unwrap().clone();
        let mut intervening = first.clone();
        intervening.hour = if invalidation == "gap" { 119 } else { 118 };
        match invalidation {
            "identity" => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *intervening.tables[TableId::Countries.index()]
                {
                    rows[0].tag = b"BBB".to_vec();
                }
            }
            "delete" => {
                *intervening.tables[TableId::Countries.index()] =
                    TableView::Complete(TableRows::Countries(vec![]))
            }
            "count" => {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *intervening.tables[TableId::Countries.index()]
                {
                    rows.push(CountryRow {
                        id: 10_000,
                        canonical_slot: 10_000,
                        tag: b"mod added".to_vec(),
                        ..Default::default()
                    });
                }
            }
            "unavailable" => {
                *intervening.tables[TableId::Countries.index()] = TableView::Unavailable {
                    reason: "fixture country refusal".into(),
                }
            }
            _ => {}
        }
        let changes =
            FrameChanges::alone(&intervening, replay_data::format::FormatVersion::CURRENT);
        reads.carry_keys(&intervening, &changes);
        // Even returning to the original live allocation cannot erase the
        // intervening invalidation. Only the full key comparison can confirm it.
        let mut restored = first.clone();
        restored.hour = intervening.hour + 1;
        let changes = FrameChanges {
            rows_changed: [false; replay_data::rows::TABLE_COUNT],
            table_read: [false; replay_data::rows::TABLE_COUNT],
            slot_read: [false; replay_data::rows::TABLE_COUNT],
            updates: None,
        };
        reads.carry_keys(&restored, &changes);
        let before = keys.row_visits();
        assert!(reads.build(&restored, false).is_empty());
        assert_eq!(
            keys.row_visits() - before,
            1,
            "{invalidation}: weak identity must not bypass the pending full comparison"
        );
    }
}

#[test]
fn variant_span_advance_requires_a_full_country_comparison_at_the_next_read() {
    use crate::prepared::frame_changes::FrameChanges;
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut input = named_frame(117, Some(b"mod design"));
    input.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
    let mut catalogue = VariantCatalogue::default();
    catalogue.observe_changed(format, &input, &FrameChanges::alone(&input, format));
    let keys = catalogue.reads.keys.as_ref().unwrap().clone();
    catalogue.advance(140);
    input.hour = 141;
    let mut changes = FrameChanges {
        rows_changed: [false; replay_data::rows::TABLE_COUNT],
        table_read: [false; replay_data::rows::TABLE_COUNT],
        slot_read: [false; replay_data::rows::TABLE_COUNT],
        updates: None,
    };
    changes.slot_read[TableId::VariantDefinitions.index()] = true;
    let before = keys.row_visits();
    assert!(
        catalogue
            .observe_changed(format, &input, &changes)
            .is_empty()
    );
    assert_eq!(
        keys.row_visits() - before,
        1,
        "an advance without country deltas must not restore the weak proof"
    );
}

fn pending_variant_operands(count: u32) {
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay_data::{rows::CountryAppearanceRow, rows::CountryLinkRow};
    use replay_frame::{frame::FrameDecoder, frame::FrameEncoder};
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut input = named_frame(117, Some(b"mod design"));
    *input.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=count)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod {id}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *input.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *input.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 1,
            ..Default::default()
        }]));
    *input.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            (1..=count)
                .map(|id| VariantDefinitionRow {
                    id: u64::from(id),
                    creator_link: id,
                    equipment_token: id,
                    display_name: b"mod design".to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    *input.tables[TableId::VariantModules.index()] =
        TableView::Complete(TableRows::VariantModules(
            (1..=count)
                .map(|id| VariantModuleRow {
                    variant: u64::from(id),
                    slot: 0,
                    definition_token: 0,
                    definition_active: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *input.tables[TableId::VariantUpgrades.index()] =
        TableView::Complete(TableRows::VariantUpgrades(vec![]));
    *input.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        (1..=count)
            .map(|token| LexicalTokenRow {
                token,
                literal: format!("mod_type_{token}").into_bytes(),
            })
            .collect(),
    ));
    let (mut encoder, mut decoder, mut tracker) = (
        FrameEncoder::new(),
        FrameDecoder::new(),
        FrameChangeTracker::new(),
    );
    let (mut actual, mut oracle) = (VariantCatalogue::default(), VariantCatalogue::default());
    for offset in 0..=8 {
        input.hour = 117 + offset;
        input.metrics.table_ns.fill(0);
        if [0, 4, 6, 8].contains(&offset) {
            input.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
        }
        if offset == 1
            && let TableView::Complete(TableRows::VariantModules(rows)) =
                &mut *input.tables[TableId::VariantModules.index()]
        {
            rows[0].definition_active = 0;
        }
        if offset == 2
            && let TableView::Complete(TableRows::LexicalTokens(rows)) =
                &mut *input.tables[TableId::LexicalTokens.index()]
        {
            rows[1].literal = b"changed_type".to_vec();
        }
        if offset == 3
            && let TableView::Complete(TableRows::CountryAppearance(rows)) =
                &mut *input.tables[TableId::CountryAppearance.index()]
        {
            rows[0].cosmetic_tag = b"unused appearance field".to_vec();
        }
        if offset == 7
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
        {
            rows[2].tag = b"changed creator".to_vec();
        }
        let wire = Arc::new(
            encoder
                .encode(input.clone(), offset == 0 || offset == 5)
                .unwrap(),
        );
        let frame = decoder.advance(&wire).unwrap();
        let changes = tracker.observe_updates(frame, wire.clone(), format);
        let before = actual.work.row_visits;
        let builds = VARIANT_BUILDS.with(std::cell::Cell::get);
        let found = actual.observe_changed(format, frame, &changes);
        let built = VARIANT_BUILDS.with(std::cell::Cell::get) - builds;
        let expected = oracle.observe_versions(format, frame, None);
        assert_eq!(found, expected);
        assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&oracle).unwrap()
        );
        if offset == 4 {
            assert_eq!(
                built, 2,
                "one design and one used spelling changed before this read"
            );
            assert!(
                actual.work.row_visits - before <= 6,
                "other {count} countries and variants must carry"
            );
        }
        if offset == 6 {
            assert_eq!(built, 0, "equal snapshot operands do not rebuild designs");
        }
        if offset == 8 {
            assert_eq!(built, 1, "only the renamed creator's variant changes");
        }
    }
}

#[test]
fn variant_invalidations_accumulate_through_skipped_reads_and_snapshots() {
    pending_variant_operands(8);
}

#[test]
fn two_changed_operands_visit_only_their_variants_at_five_thousand_countries() {
    pending_variant_operands(5000);
}
