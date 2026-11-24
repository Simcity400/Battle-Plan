use super::*;
use replay_data::rows::VariantDefinitionRow;

#[test]
fn held_model_facts_keep_the_original_census_and_refusal_order_across_hours() {
    content_reader_script::test_game::enter();
    fn facts_content(replaced: bool) -> model_selection::Content {
        let prefix = if replaced {
            b"equipments = { padding = { is_archetype = yes } }".as_slice()
        } else {
            b""
        };
        let overrides = if replaced {
            b"equipments = { rifles_2 = { archetype = rifles visual_level = 7 model = replacement_model } }".as_slice()
        } else {
            b""
        };
        model_selection::Content::from_test_documents(&[
            prefix,
            br"
            sub_units = { infantry = { type = infantry need = { rifles = 100 } } }
            equipments = {
                rifles = { is_archetype = yes type = infantry }
                rifles_1 = { archetype = rifles }
                rifles_2 = { archetype = rifles visual_level = 2 model = rifle_model }
                frames = { is_archetype = yes type = fighter
                    module_slots = { main = {} secondary = main } }
                frames_1 = { archetype = frames module_slots = inherit visual_level = 3 }
                orphan = { type = infantry }
            }",
            overrides,
        ])
    }
    fn census(frame: &DecodedFrame, content: &model_selection::Content) -> Result<String, String> {
        bridge::Prepared::read(frame, content).map(|census| census.comparison_state())
    }
    fn check_bank(content: &model_selection::Content) {
        let mut keys = BTreeSet::new();
        let mut enabled = BTreeSet::new();
        let mut masks = 0;
        let category = content_reader_script::test_game::category;
        for mask in [
            0,
            category("armor"),
            category("screen_ship"),
            category("screen_ship") | category("armor") | category("fighter"),
        ] {
            let view = content.model_facts().census(mask);
            let mut present = false;
            for key in ["rifles", "rifles_1", "rifles_2", "frames", "frames_1"] {
                let Some(fact) = view.get(key) else {
                    continue;
                };
                present = true;
                let model = content.equipment_model(key).unwrap();
                assert_eq!(
                    (
                        Some(fact.equipment),
                        fact.archetype,
                        fact.visual_level,
                        fact.model_priority,
                        fact.depth
                    ),
                    (
                        content.equipment_content().get(key),
                        model.archetype_ordinal,
                        model.visual_level,
                        model.model_priority,
                        model.depth
                    ),
                    "{key}"
                );
                assert_eq!(
                    fact.has_enabled_module_slots,
                    content.has_enabled_module_slots(key, mask).unwrap(),
                    "{key} {mask:#x}"
                );
                keys.insert(key.to_owned());
                enabled.insert((key.to_owned(), fact.has_enabled_module_slots));
            }
            for invalid in ["orphan", "nothing"] {
                assert_eq!(view.get(invalid), None);
            }
            masks += usize::from(present);
        }
        assert_eq!(
            keys,
            BTreeSet::from(
                ["rifles", "rifles_1", "rifles_2", "frames", "frames_1"].map(String::from)
            )
        );
        assert!(masks > 1, "facts are held per module-category mask");
        assert!(enabled.contains(&("frames".to_owned(), true)));
        assert!(enabled.contains(&("frames".to_owned(), false)));
    }
    let spellings: [&[u8]; 9] = [
        b"infantry",
        b"rifles",
        b"rifles_1",
        b"RIFLES_2",
        b"frames",
        b"frames_1",
        b"orphan",
        b"nothing",
        b"Rifles_1",
    ];
    let tokens = |hour: i32| -> Vec<LexicalTokenRow> {
        spellings
            .iter()
            .enumerate()
            .map(|(at, literal)| LexicalTokenRow {
                token: at as u32 + 1,
                literal: match (at, hour % 17) {
                    (2, 3) => b"RIFLES_1".to_vec(),
                    (3, 8) => b"frames".to_vec(),
                    _ => literal.to_vec(),
                },
            })
            .collect()
    };
    let handle = |at: usize| ((at as u64 + 1) << 32) | 70;
    let base = frame();
    let mut warm = facts_content(false);
    let mut retained = RetainedBridge::default();
    let mut built = 0;
    let mut refused = BTreeSet::new();
    for hour in 0..360_i32 {
        let mut frame = base.clone();
        frame.hour = hour;
        frame.branch = 1 + u64::from(hour > 250);
        let count = 4 + (hour % 7) as usize;
        let mut definitions: Vec<_> = (0..count)
            .map(|at| VariantDefinitionRow {
                id: handle(at),
                equipment_token: [1, 2, 3, 4, 5, 6, 7, 8, 9, 999]
                    [(hour as usize * 5 + at * 3) % 10],
                archetype_token: 2,
                creator_link: [100, 101, 0, u32::MAX][(hour as usize + at) % 4],
                ..Default::default()
            })
            .collect();
        if hour % 37 == 5 {
            definitions.push(definitions[0].clone());
        }
        let mut upgrades = vec![
            VariantUpgradeRow {
                variant: handle(0),
                level: 255,
                ..Default::default()
            },
            VariantUpgradeRow {
                variant: handle(0),
                level: (hour * 37 % 256) as u8,
                ..Default::default()
            },
            VariantUpgradeRow {
                variant: handle(1),
                level: (hour % 256) as u8,
                ..Default::default()
            },
        ];
        if hour % 19 == 7 {
            upgrades.push(VariantUpgradeRow {
                variant: handle(77),
                level: 1,
                ..Default::default()
            });
        }
        let mut available: Vec<_> = [100, 101]
            .into_iter()
            .flat_map(|country| {
                (0..3).flat_map(move |roster_kind| {
                    (0..count).map(move |ordinal| AvailableEquipmentVariantRow {
                        country,
                        roster_kind,
                        ordinal: ordinal as u32,
                        variant: handle((ordinal * 3 + hour as usize) % count),
                    })
                })
            })
            .collect();
        for (every, at, roster_kind, ordinal, variant) in [
            (43, 7, 0, 99, 999),
            (47, 9, 3, 98, handle(0)),
            (53, 11, 1, 0, handle(1)),
        ] {
            if hour % every == at {
                available.push(AvailableEquipmentVariantRow {
                    country: 100,
                    roster_kind,
                    ordinal,
                    variant,
                });
            }
        }
        let mut links = vec![
            CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 101,
                canonical_slot: 2,
            },
        ];
        if hour % 23 == 11 {
            links.pop();
        }
        let dlc_mask = [0, 0x80, 0x190, 0x10][(hour / 3) as usize % 4];
        let mut features = vec![SimulationFeatureRow {
            dlc_mask,
            ..Default::default()
        }];
        if hour % 29 == 13 {
            features.push(SimulationFeatureRow::default());
        }
        set(&mut frame, TableRows::LexicalTokens(tokens(hour)));
        set(
            &mut frame,
            TableRows::VariantDefinitions(definitions.clone()),
        );
        set(&mut frame, TableRows::VariantUpgrades(upgrades));
        set(&mut frame, TableRows::AvailableEquipmentVariants(available));
        set(&mut frame, TableRows::CountryLinks(links));
        set(&mut frame, TableRows::SimulationFeatures(features));
        if hour % 31 == 17 {
            frame.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
                reason: format!("hour {hour} unavailable"),
            }
            .into();
        }
        if hour % 41 == 9 {
            frame.tables[TableId::VariantDefinitions.index()] = TableView::Truncated {
                rows: TableRows::VariantDefinitions(definitions),
                missing: 1,
            }
            .into();
        }
        if hour == 180 {
            check_bank(&warm);
            warm = facts_content(true);
            retained = RetainedBridge::default();
        }
        let original =
            bridge::reference::read(&frame, &warm).map(|census| census.comparison_state());
        assert_eq!(census(&frame, &warm), original, "held hour {hour}");
        assert_eq!(
            census(&frame, &warm.clone()),
            original,
            "shared clone hour {hour}"
        );
        assert_eq!(
            census(&frame, &facts_content(hour >= 180)),
            original,
            "cold hour {hour}"
        );
        assert_eq!(
            retained
                .read(&frame, &warm)
                .map(|census| census.prepared().comparison_state()),
            original,
            "retained hour {hour}"
        );
        match original {
            Ok(_) => built += 1,
            Err(error) => {
                refused.insert(error);
            }
        }
    }
    check_bank(&warm);
    assert!(built > 100, "{refused:?}");
    for reason in [
        "VariantDefinitions is not complete for hour",
        "SimulationFeatures is not complete for hour",
        "SimulationFeatures must have one complete row",
        "duplicate equipment variant handle",
        "creator 101 has no CountryLinks row",
        "upgrade references missing equipment variant",
        "availability references missing variant",
        "invalid equipment roster kind",
        "repeated availability ordinals",
    ] {
        assert!(
            refused.iter().any(|error| error.contains(reason)),
            "{reason}: {refused:?}"
        );
    }
    let countries = 5000_u32;
    let mut sizing = base;
    set(&mut sizing, TableRows::LexicalTokens(tokens(0)));
    set(
        &mut sizing,
        TableRows::CountryLinks(
            (0..countries)
                .map(|at| CountryLinkRow {
                    id: 1000 + at,
                    canonical_slot: at,
                })
                .collect(),
        ),
    );
    set(
        &mut sizing,
        TableRows::VariantDefinitions(
            (0..countries as usize)
                .map(|at| VariantDefinitionRow {
                    id: handle(at),
                    equipment_token: [2, 3, 4, 5, 6, 9][at % 6],
                    archetype_token: 2,
                    creator_link: 1000 + at as u32,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(&mut sizing, TableRows::VariantUpgrades(Vec::new()));
    set(
        &mut sizing,
        TableRows::AvailableEquipmentVariants(
            (0..countries)
                .flat_map(|at| {
                    (0..3).map(move |roster_kind| AvailableEquipmentVariantRow {
                        country: 1000 + at,
                        roster_kind,
                        ordinal: 0,
                        variant: handle(at as usize),
                    })
                })
                .collect(),
        ),
    );
    set(
        &mut sizing,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            dlc_mask: 0x190,
            ..Default::default()
        }]),
    );
    let original = bridge::reference::read(&sizing, &warm).map(|census| census.comparison_state());
    assert!(original.is_ok(), "{original:?}");
    assert_eq!(census(&sizing, &warm), original);
    assert_eq!(census(&sizing, &facts_content(true)), original);
}
