use super::*;
use replay_data::{rows::DivisionTemplateIconRow, rows::LexicalTokenRow, rows::TemplateSubunit};

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
    let mut frame = replay_frame::frame::DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Default::default(),
        tables: std::array::from_fn(|i| {
            replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::empty(
                replay_data::rows::TableId::ALL[i],
            ))
            .into()
        }),
    };
    use replay_data::rows::{
        CountryLinkRow, CountryLocalSubunitRow, LocalSubunitRequirement, TableId, TableRows,
    };
    use replay_frame::frame::TableView;
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(tokens.to_vec()));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 10,
            canonical_slot: 7,
        }]));
    *frame.tables[TableId::CountryLocalSubunits.index()] =
        TableView::Complete(TableRows::CountryLocalSubunits(
            local
                .iter()
                .map(|((_, token), needs)| CountryLocalSubunitRow {
                    country: 10,
                    token: *token,
                    requirements: needs
                        .iter()
                        .map(|(token, amount)| LocalSubunitRequirement {
                            archetype_token: *token,
                            amount_raw: *amount,
                        })
                        .collect(),
                })
                .collect(),
        ));
    Sources::read(Box::leak(Box::new(frame))).unwrap()
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

fn variants() -> Vec<VariantDefinitionRow> {
    [(10, 101), (20, 102), (30, 103)]
        .into_iter()
        .map(|(id, equipment_token)| VariantDefinitionRow {
            id,
            equipment_token,
            ..Default::default()
        })
        .collect()
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

fn equipment(handle: u64, amount_raw: i64, ordinal: u32) -> DivisionEquipmentRow {
    DivisionEquipmentRow {
        variant_handle: handle,
        amount_raw,
        ordinal,
        ..Default::default()
    }
}

#[test]
fn an_unnamed_candidate_refuses_the_pair_instead_of_ranking_the_next() {
    content_reader_script::test_game::enter();
    let content = content();
    let template = content
        .template(&row((2, 0), &[(2, 0, 1), (1, 0, 1)]), &sources(&[]))
        .unwrap();
    let mut variants = variants();
    variants.push(VariantDefinitionRow {
        id: 40,
        equipment_token: 999,
        ..Default::default()
    });
    let rows = [equipment(40, 900, 0), equipment(20, 500, 2)];
    let refs = &rows[..];
    let error = content
        .select_pair(&template, None, &[], refs, Variants::new(&variants[..]), &names())
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "variant 40 equipment type: token 999 is not in lexical tokens"
    );
}

#[test]
fn without_a_category_the_representative_largest_need_filters_by_quantity_then_order() {
    content_reader_script::test_game::enter();
    let content = content();
    let template = content
        .template(&row((2, 0), &[(2, 0, 1), (1, 0, 1)]), &sources(&[]))
        .unwrap();
    let variants = variants();
    let rows = [
        equipment(10, 900, 0),
        equipment(30, 500, 7),
        equipment(20, 500, 2),
    ];
    let refs = &rows[..];
    let pair = content
        .select_pair(&template, None, &[20], refs, Variants::new(&variants[..]), &names())
        .unwrap()
        .unwrap();
    assert_eq!((pair.variant.id, pair.subunit.key.as_str()), (20, "armor"));
    let null = content
        .template(&row((0, 0), &[(2, 0, 1)]), &sources(&[]))
        .unwrap();
    assert!(
        content
            .select_pair(&null, None, &[], refs, Variants::new(&variants[..]), &names())
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_category_scores_the_stored_groups_and_skips_excluded_variants() {
    content_reader_script::test_game::enter();
    let category = content_reader_script::test_game::category;
    let content = content();
    let template = content
        .template(&row((3, 0), &[(2, 0, 1), (1, 0, 2)]), &sources(&[]))
        .unwrap();
    let variants = variants();
    let rows = [
        equipment(20, 500, 0),
        equipment(30, 500, 1),
        equipment(10, 1, 2),
    ];
    let refs = &rows[..];
    let pair = content
        .select_pair(&template, Some(category("armor")), &[20], refs, Variants::new(&variants[..]), &names())
        .unwrap()
        .unwrap();
    assert_eq!((pair.variant.id, pair.subunit.key.as_str()), (30, "armor"));
    let pair = content
        .select_pair(&template, Some(category("infantry")), &[], refs, Variants::new(&variants[..]), &names())
        .unwrap()
        .unwrap();
    assert_eq!(pair.subunit.key, "infantry");
    let missing = [equipment(99, 1, 0)];
    assert!(
        content
            .select_pair(
                &template,
                Some(category("infantry")),
                &[],
                &missing,
                Variants::new(&variants[..]),
                &names()
            )
            .is_err()
    );
}

#[test]
fn country_override_requires_exact_category_and_signed_positive_origin() {
    content_reader_script::test_game::enter();
    let content = content();
    let subunit = content.subunit("armor").unwrap();
    let mut variant = VariantDefinitionRow {
        origin_link: u32::MAX,
        creator_link: 7,
        classifier_flags: content_reader_script::test_game::category("armor"),
        ..Default::default()
    };
    assert_eq!(variant_country(&variant, subunit, 9), 7);
    variant.origin_link = 8;
    assert_eq!(variant_country(&variant, subunit, 9), 8);
    variant.classifier_flags |= 0x20;
    assert_eq!(variant_country(&variant, subunit, 9), 9);
}

#[test]
fn a_local_representatives_recorded_need_selects_its_equipment_without_rescoring() {
    content_reader_script::test_game::enter();
    let content = content();
    let local = sources(&[((7, 1), vec![(5, 200_000)])]);
    let template = content
        .template(&row((1, 1), &[(1, 0, 1), (1, 1, 1), (2, 0, 1)]), &local)
        .unwrap();
    let variants = variants();
    let rows = [equipment(10, 900, 0), equipment(20, 100, 1)];
    let refs = &rows[..];
    let pair = content
        .select_pair(&template, None, &[], refs, Variants::new(&variants[..]), &names())
        .unwrap()
        .unwrap();
    assert_eq!(pair.variant.id, 20);
    assert_eq!(
        pair.identity,
        Definition::Local {
            country: 7,
            token: 1
        }
    );
}

#[test]
fn coarse_pair_scopes_keep_stable_ties_nulls_exclusions_and_first_errors() {
    content_reader_script::test_game::enter();
    let armor = content_reader_script::test_game::category("armor");
    let content = content();
    let source = sources(&[]);
    let template = content
        .template(&row((2, 0), &[(2, 0, 1), (1, 0, 1)]), &source)
        .unwrap();
    let null = content
        .template(&row((0, 0), &[(2, 0, 1)]), &source)
        .unwrap();
    let mut variants = variants();
    variants.push(VariantDefinitionRow {
        id: 40,
        equipment_token: 999,
        ..Default::default()
    });
    let names = names();
    let snapshot = |result: io::Result<Option<SelectedPair<'_>>>| {
        result
            .map(|pair| pair.map(|pair| (pair.variant.id, pair.subunit.key.clone(), pair.identity)))
            .map_err(|error| (error.kind(), error.to_string()))
    };
    let stages = profile::Stages::new(true);
    for frame in 0_usize..128 {
        let amount = [i64::MIN, -1, 0, 500, i64::MAX][frame % 5];
        let mut rows = vec![
            equipment(20, amount, 2),
            equipment(30, amount, 2),
            equipment(10, 900, 0),
        ];
        if frame.is_multiple_of(2) {
            rows.reverse();
        }
        if frame.is_multiple_of(7) {
            rows.insert(0, equipment(40, 0, 9));
        }
        if frame.is_multiple_of(11) {
            rows.insert(0, equipment(99, 0, 9));
        }
        let refs = &rows[..];
        let category = frame.is_multiple_of(3).then_some(armor);
        let excluded: &[u64] = if frame.is_multiple_of(4) { &[20] } else { &[] };
        let template = if frame.is_multiple_of(13) {
            &null
        } else {
            &template
        };
        let ordinary =
            snapshot(content.select_pair(template, category, excluded, refs, Variants::new(&variants[..]), &names));
        let measured = snapshot(content.select_pair_profiled(
            template, category, excluded, refs, Variants::new(&variants[..]), &names, &stages,
        ));
        assert_eq!(ordinary, measured, "frame {frame}");
    }
    let mut spans = viewer_runtime::profiling::BuilderSpans::new(true);
    stages.append_to(&mut spans);
    assert_eq!(
        spans
            .into_records()
            .iter()
            .map(|record| record.name)
            .collect::<Vec<_>>(),
        [
            "units.selection.pair_classify",
            "units.selection.pair_sorts",
            "units.selection.pair_rank",
        ]
    );
    for handles in [[20, 30], [30, 20]] {
        let rows = handles.map(|handle| equipment(handle, -1, 7));
        let refs = &rows[..];
        let pair = content
            .select_pair(&template, None, &[], refs, Variants::new(&variants[..]), &names)
            .unwrap()
            .unwrap();
        assert_eq!(pair.variant.id, handles[0]);
    }
    let rows = [equipment(40, 0, 9), equipment(99, 0, 0)];
    let refs = &rows[..];
    let stages = profile::Stages::new(true);
    assert_eq!(
        content
            .select_pair_profiled(
                &template,
                Some(armor),
                &[40],
                refs,
                Variants::new(&variants[..]),
                &names,
                &stages
            )
            .unwrap_err()
            .to_string(),
        "variant 40 equipment type: token 999 is not in lexical tokens"
    );
    let mut spans = viewer_runtime::profiling::BuilderSpans::new(true);
    stages.append_to(&mut spans);
    assert_eq!(
        spans
            .into_records()
            .iter()
            .map(|record| record.name)
            .collect::<Vec<_>>(),
        ["units.selection.pair_classify"]
    );
    let stages = profile::Stages::new(true);
    assert!(
        content
            .select_pair_profiled(&null, None, &[], refs, Variants::new(&variants[..]), &names, &stages)
            .unwrap()
            .is_none()
    );
    let mut spans = viewer_runtime::profiling::BuilderSpans::new(true);
    stages.append_to(&mut spans);
    assert!(spans.into_records().is_empty());
}

#[test]
fn retained_classification_keeps_multi_hour_keys_failures_ties_and_original_demand() {
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::{DecodedFrame, SharedView, TableView};
    content_reader_script::test_game::enter();
    let armor = content_reader_script::test_game::category("armor");
    let mut content = content();
    let mut held = DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Default::default(),
        tables: std::array::from_fn(|i| {
            TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
        }),
    };
    let definitions = variants();
    let tokens: Vec<_> = [
        (101, b"rifles_1".as_slice()),
        (102, b"Tanks_1"),
        (103, b"tanks_2"),
        (104, b"trucks"),
    ]
    .into_iter()
    .map(|(token, literal)| LexicalTokenRow {
        token,
        literal: literal.to_vec(),
    })
    .collect();
    held.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(definitions.clone())).into();
    held.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(tokens.clone())).into();
    let snapshot = |result: io::Result<Option<SelectedPair<'_>>>| {
        result
            .map(|pair| pair.map(|pair| (pair.variant.id, pair.subunit.key.clone(), pair.identity)))
            .map_err(|error| (error.kind(), error.to_string()))
    };
    let mut random = 0xe91a_62d3_u64;
    let mut successful = 0;
    let mut refused = 0;
    let mut reasons = std::collections::BTreeMap::<String, (usize, usize)>::new();
    for hour in 0_usize..384 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        if hour.is_multiple_of(47) {
            content = content_with(b"");
        }
        let mut frame = held.clone();
        frame.hour = hour as i32;
        frame.branch = 1 + u64::from(hour > 290);
        if hour.is_multiple_of(11) {
            for id in [TableId::VariantDefinitions, TableId::LexicalTokens] {
                frame.tables[id.index()] = SharedView::from((*frame.tables[id.index()]).clone());
            }
        }
        if hour.is_multiple_of(9) {
            let mut rows = definitions.clone();
            rows[1].equipment_token = 103;
            rows.push(VariantDefinitionRow {
                id: 40,
                equipment_token: 999,
                ..Default::default()
            });
            frame.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(rows)).into();
        }
        if hour.is_multiple_of(19) {
            let mut rows = tokens.clone();
            rows[1].literal = if hour.is_multiple_of(38) {
                vec![0xff]
            } else {
                b"rifles_1".to_vec()
            };
            frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(rows)).into();
        }
        if hour.is_multiple_of(17) {
            held = frame.clone();
        }
        let variants = frame
            .complete::<replay_data::rows::tables::VariantDefinitions>()
            .unwrap();
        let names = TokenNames::new(&frame).unwrap();
        let requirements = if hour.is_multiple_of(7) {
            vec![(5, 200_000)]
        } else {
            vec![(4, 100_000)]
        };
        let sources = sources(&[((7, 1), requirements)]);
        let template_row = row((2, 0), &[(2, 0, 1), (1, 0, 2)]);
        let template = content.template(&template_row, &sources).unwrap();
        let local_row = row((1, 1), &[(1, 1, 1), (2, 0, 1)]);
        let local = content.template(&local_row, &sources).unwrap();
        let null_row = row((0, 0), &[(2, 0, 1)]);
        let null = content.template(&null_row, &sources).unwrap();
        let facts = content.classification_facts(&frame).unwrap();
        for query in 0_usize..16 {
            let amount = [i64::MIN, -1, 0, 500, i64::MAX][(hour + query) % 5];
            let mut rows = vec![
                equipment(20, amount, 2),
                equipment(30, amount, 2),
                equipment(10, 900, 0),
            ];
            if (random + query as u64).is_multiple_of(2) {
                rows.reverse();
            }
            if (hour + query).is_multiple_of(7) {
                rows.insert(0, equipment(40, 0, 9));
            }
            if (hour + query).is_multiple_of(13) {
                rows.insert(0, equipment(99, 0, 0));
            }
            let refs = &rows[..];
            let category = query.is_multiple_of(3).then_some(armor);
            let excluded: &[u64] = if query.is_multiple_of(4) {
                &[20, 40]
            } else {
                &[]
            };
            let template = match (hour + query) % 17 {
                0 => &null,
                1 => &local,
                _ => &template,
            };
            let expected = snapshot(
                content.select_pair(template, category, excluded, refs, Variants::new(&variants[..]), &names),
            );
            let actual = snapshot(content.select_pair_classified(
                template,
                category,
                excluded,
                refs,
                Variants::new(&variants[..]),
                &names,
                &Stages::new(false),
                Some(&facts),
            ));
            assert_eq!(expected, actual, "hour {hour} query {query}");
            successful += usize::from(actual.as_ref().is_ok_and(Option::is_some));
            refused += usize::from(actual.is_err());
            if let Err((_, why)) = &actual {
                reasons.entry(why.clone()).or_insert((0, hour)).0 += 1;
            }
        }
        let cloned = content.clone();
        let facts = cloned.classification_facts(&frame).unwrap();
        let rows = [equipment(20, 500, 0), equipment(30, 500, 0)];
        let refs = &rows[..];
        assert_eq!(
            snapshot(content.select_pair(&template, None, &[], refs, Variants::new(&variants[..]), &names)),
            snapshot(cloned.select_pair_classified(
                &template,
                None,
                &[],
                refs,
                Variants::new(&variants[..]),
                &names,
                &Stages::new(false),
                Some(&facts)
            ))
        );
    }
    assert!(
        successful > 100 && refused > 100,
        "successful={successful}, refused={refused}, refusals (count, first hour) {reasons:?}"
    );
}
