use super::*;
use replay_data::{rows::DivisionTemplateIconRow, rows::LexicalTokenRow, rows::TemplateSubunit};

fn content() -> Content {
    content_with(b"")
}

/// [`content`] with `extra` read after it, as a later file would be.
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

/// Sub-units and equipment from documents read in order, as
/// [`Content::load`] reads the files.
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

/// Equipment type spellings; `Tanks_1` differs from its authored key
/// only in ASCII case, and `trucks` is not authored.
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

fn variants() -> BTreeMap<u64, &'static VariantDefinitionRow> {
    [(10, 101), (20, 102), (30, 103)]
        .into_iter()
        .map(|(id, equipment_token)| {
            let row: &'static VariantDefinitionRow = Box::leak(Box::new(VariantDefinitionRow {
                id,
                equipment_token,
                ..Default::default()
            }));
            (id, row)
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = content();
    let template = content
        .template(&row((2, 0), &[(2, 0, 1), (1, 0, 1)]), &sources(&[]))
        .unwrap();
    let mut variants = variants();
    // Variant 40's token is not in the hour's LexicalTokens.
    variants.insert(
        40,
        Box::leak(Box::new(VariantDefinitionRow {
            id: 40,
            equipment_token: 999,
            ..Default::default()
        })),
    );
    let rows = [equipment(40, 900, 0), equipment(20, 500, 2)];
    let refs: Vec<_> = rows.iter().collect();
    let error = content
        .select_pair(&template, None, &[], &refs, &variants, &names())
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "variant 40 equipment type: token 999 is not in LexicalTokens"
    );
}

#[test]
fn without_a_category_the_representative_largest_need_filters_by_quantity_then_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = content();
    let template = content
        .template(&row((2, 0), &[(2, 0, 1), (1, 0, 1)]), &sources(&[]))
        .unwrap();
    let variants = variants();
    // The representative is armor: tanks is its largest need, so rifles
    // never qualify, and equal quantities keep the older ordinal.
    let rows = [
        equipment(10, 900, 0),
        equipment(30, 500, 7),
        equipment(20, 500, 2),
    ];
    let refs: Vec<_> = rows.iter().collect();
    let pair = content
        .select_pair(&template, None, &[20], &refs, &variants, &names())
        .unwrap()
        .unwrap();
    assert_eq!((pair.variant.id, pair.subunit.key.as_str()), (20, "armor"));
    // A null representative draws no pair at all.
    let null = content
        .template(&row((0, 0), &[(2, 0, 1)]), &sources(&[]))
        .unwrap();
    assert!(
        content
            .select_pair(&null, None, &[], &refs, &variants, &names())
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_category_scores_the_stored_groups_and_skips_excluded_variants() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
    let refs: Vec<_> = rows.iter().collect();
    // Armor wins the tanks pair: the only group needing tanks.
    let pair = content
        .select_pair(&template, Some(1 << 2), &[20], &refs, &variants, &names())
        .unwrap()
        .unwrap();
    assert_eq!((pair.variant.id, pair.subunit.key.as_str()), (30, "armor"));
    // Rifles: infantry's two entries outscore armor's one.
    let pair = content
        .select_pair(&template, Some(1 << 5), &[], &refs, &variants, &names())
        .unwrap()
        .unwrap();
    assert_eq!(pair.subunit.key, "infantry");
    let missing = [equipment(99, 1, 0)];
    assert!(
        content
            .select_pair(
                &template,
                Some(1 << 5),
                &[],
                &[&missing[0]],
                &variants,
                &names()
            )
            .is_err()
    );
}

#[test]
fn country_override_requires_exact_category_and_signed_positive_origin() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = content();
    let subunit = content.subunit("armor").unwrap();
    let mut variant = VariantDefinitionRow {
        origin_link: u32::MAX,
        creator_link: 7,
        classifier_flags: 4,
        ..Default::default()
    };
    assert_eq!(variant_country(&variant, subunit, 9), 7);
    variant.origin_link = 8;
    assert_eq!(variant_country(&variant, subunit, 9), 8);
    variant.classifier_flags |= 32;
    assert_eq!(variant_country(&variant, subunit, 9), 9);
}

#[test]
fn a_local_representatives_recorded_need_selects_its_equipment_without_rescoring() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let content = content();
    let local = sources(&[((7, 1), vec![(5, 200_000)])]);
    let template = content
        .template(&row((1, 1), &[(1, 0, 1), (1, 1, 1), (2, 0, 1)]), &local)
        .unwrap();
    let variants = variants();
    let rows = [equipment(10, 900, 0), equipment(20, 100, 1)];
    let refs: Vec<_> = rows.iter().collect();
    let pair = content
        .select_pair(&template, None, &[], &refs, &variants, &names())
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
