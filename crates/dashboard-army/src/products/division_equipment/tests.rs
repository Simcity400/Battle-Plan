use super::*;
use replay_data::rows::*;
use replay_frame::frame::TableView;

const DOCUMENT: &str = r#"
sub_units = { test_line = { type = infantry need = { widgets = 100 } } }
equipments = {
    widgets = { is_archetype = yes type = infantry }
    widgets_1 = { archetype = widgets }
}
"#;
fn frame() -> DecodedFrame {
    let mut frame = dashboard_base::test_fixtures::source(1, 60_000_010, None);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.insert(
            0,
            CountryRow {
                id: 21,
                canonical_slot: 8,
                tag: b"ITA".to_vec(),
                ..Default::default()
            },
        );
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 21,
            canonical_slot: 8,
        });
        rows.sort_by_key(|row| row.id);
    }
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"test_line".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"widgets".to_vec(),
            },
            LexicalTokenRow {
                token: 3,
                literal: b"widgets_1".to_vec(),
            },
        ]));
    *frame.tables[TableId::CountryLocalSubunits.index()] =
        TableView::Complete(TableRows::CountryLocalSubunits(vec![]));
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 5,
                country: 11,
                subunits: vec![TemplateSubunit {
                    token: 1,
                    local: 0,
                    count: 1,
                }],
                settings: Recorded::Value(Some(Arc::new(TemplateSettings {
                    total_manpower: 1000,
                    ..Default::default()
                }))),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        division(1, 11),
        division(2, 9001),
    ]));
    *frame.tables[TableId::DivisionEquipment.index()] =
        TableView::Complete(TableRows::DivisionEquipment(vec![
            equipment(1, 0, 70, 15_000_000),
            equipment(2, 0, 70, 5_000_000),
        ]));
    *frame.tables[TableId::NewDivisionEquipment.index()] =
        TableView::Complete(TableRows::NewDivisionEquipment(vec![]));
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![status(1), status(2)]));
    *frame.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(vec![
            DivisionLandInputsRow {
                division: 1,
                manpower: vec![
                    DivisionManpowerContributor {
                        country: 11,
                        amount: i32::MAX,
                    },
                    DivisionManpowerContributor {
                        country: 77,
                        amount: 2,
                    },
                ],
                ..Default::default()
            },
            DivisionLandInputsRow {
                division: 2,
                manpower: vec![DivisionManpowerContributor {
                    country: 11,
                    amount: 300,
                }],
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::Stockpiles.index()] =
        TableView::Complete(TableRows::Stockpiles(vec![StockpileRow {
            country: 9001,
            variant_handle: 70,
            amount_raw: 900_000_000,
            ..Default::default()
        }]));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 70,
            equipment_token: 3,
            creator_link: 11,
            display_name: b"Synthetic model".to_vec(),
            ..Default::default()
        }]));
    frame
}
fn division(id: u64, roster_owner: u32) -> DivisionRow {
    DivisionRow {
        id,
        roster_owner,
        template: 5,
        hit_points: 1_000_000,
        ..Default::default()
    }
}
fn equipment(
    division: u64,
    ordinal: u32,
    variant_handle: u64,
    amount_raw: i64,
) -> DivisionEquipmentRow {
    DivisionEquipmentRow {
        division,
        ordinal,
        variant_handle,
        amount_raw,
        template: 5,
        country: 11,
    }
}
fn status(division: u64) -> DivisionStatusRow {
    let name = format!("Synthetic {division}").into_bytes();
    DivisionStatusRow {
        division,
        maximum_hit_points: 2_000_000,
        name: Recorded::Value(Some(name.clone())),
        naming: Recorded::Value(Some(NameOperands {
            custom: name,
            ..Default::default()
        })),
        pending_template: Recorded::Value(Some(0)),
        ..Default::default()
    }
}
fn params() -> DivisionEquipmentParams {
    DivisionEquipmentParams {
        branch_id: "1".into(),
        hour: 60_000_010,
        tag: "GER".into(),
        count_for: ArmyAttribution::Owner,
        template: None,
        division: None,
    }
}
fn reply(
    frame: &DecodedFrame,
    daily: Option<&DecodedFrame>,
    stock: Option<&DecodedFrame>,
    p: &DivisionEquipmentParams,
) -> DivisionEquipmentReply {
    let (cat, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        ("common/units/equipment/test.txt", DOCUMENT),
        ("common/units/test.txt", DOCUMENT),
        (
            "localisation/english/test_l_english.yml",
            "l_english:\n widgets:0 \"Widgets\"\n",
        ),
    ]);
    let cat = Arc::new(cat);
    let needs = Needs::from_documents(&[DOCUMENT.as_bytes()], frame);
    let designs = army_content::Designs::new(Some(frame), Some(&cat));
    let stock_designs = army_content::Designs::new(stock, Some(&cat));
    let mut refusals = Refusals::default();
    let mut reply = build(
        frame,
        p,
        &Context {
            daily,
            stock,
            cat: Some(&cat),
            designs: &designs,
            stock_designs: &stock_designs,
            needs: &needs,
            names: None,
            icon: &|_| Ok("test-icon".into()),
            labels: Labels::Named,
        },
        &mut refusals,
        &AtomicBool::new(false),
    )
    .unwrap();
    reply.warnings = refusals.lines();
    reply
}

#[test]
fn surplus_never_cancels_another_divisions_shortfall_and_stock_is_context_only() {
    let f = frame();
    let mut daily = f.clone();
    daily.hour -= 8;
    let mut stock = f.clone();
    stock.hour -= 2;
    let r = reply(&f, Some(&daily), Some(&stock), &params());
    assert_eq!(
        (r.divisions, r.unknown_divisions, r.pool_hour),
        (2, 0, Some(daily.hour))
    );
    let kind = &r.types[0];
    assert_eq!(
        (
            kind.held_raw.as_str(),
            kind.needed_raw.as_deref(),
            kind.missing_raw.as_deref()
        ),
        ("20000000", Some("20000000"), Some("5000000"))
    );
    assert_eq!(kind.divisions_short, 1);
    assert_eq!(
        (kind.stockpile_raw.as_deref(), kind.stockpile_hour),
        (Some("900000000"), Some(stock.hour))
    );
    let template = &r.templates[0];
    assert_eq!(
        (
            template.needed_raw.as_deref(),
            template.held_raw.as_deref(),
            template.types_short
        ),
        (Some("20000000"), Some("15000000"), 1)
    );
    assert_eq!(
        template.manpower.as_deref(),
        Some((i64::from(i32::MAX.wrapping_add(2)) + 300).to_string().as_str())
    );
    assert_eq!(
        (template.hp_raw.as_deref(), template.hp_max_raw.as_deref()),
        (Some("2000000"), Some("4000000"))
    );
    assert!(r.division_rows.is_none() && r.pool.is_none());
}

#[test]
fn unknown_pool_is_counted_without_zeroing_independent_manpower_and_hp() {
    let f = frame();
    let mut daily = f.clone();
    *daily.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "no membership witness".into(),
    };
    *daily.tables[TableId::DivisionEquipment.index()] = TableView::Complete(
        TableRows::DivisionEquipment(vec![equipment(1, 0, 70, 15_000_000)]),
    );
    let mut p = params();
    p.template = Some("5".into());
    let r = reply(&f, Some(&daily), None, &p);
    assert_eq!(r.unknown_divisions, 1);
    assert_eq!(r.types[0].unknown_divisions, 1);
    assert_eq!(r.types[0].needed_raw.as_deref(), Some("10000000"));
    let rows = r.division_rows.unwrap();
    assert_eq!(rows[0].held_raw.as_deref(), Some("10000000"));
    assert!(!rows[1].pool_known && rows[1].needed_raw.is_none() && rows[1].held_raw.is_none());
    assert_eq!((rows[0].types_short.is_some(), rows[1].types_short), (true, None));
    assert_eq!(
        (rows[1].manpower.as_deref(), rows[1].hp_raw.as_deref()),
        (Some("300"), Some("1000000"))
    );
    let mut p = params();
    p.division = Some("2".into());
    let pool = reply(&f, Some(&daily), None, &p).pool.unwrap();
    assert!(!pool.pool_known);
    assert_eq!(pool.types.len(), 1);
    assert_eq!(pool.types[0].held_raw, None);
    assert_eq!(pool.types[0].needed_raw.as_deref(), Some("10000000"));
    assert_eq!(pool.types[0].missing_raw, None);
    assert_eq!(pool.manpower.as_deref(), Some("300"));
    let mut empty = f.clone();
    *empty.tables[TableId::DivisionEquipment.index()] =
        TableView::Complete(TableRows::DivisionEquipment(vec![]));
    let pool = reply(&f, Some(&empty), None, &p).pool.unwrap();
    assert!(pool.pool_known);
    assert_eq!(pool.types[0].held_raw.as_deref(), Some("0"));
    assert_eq!(pool.types[0].missing_raw.as_deref(), Some("10000000"));
    p.division = Some("1".into());
    assert!(reply(&f, Some(&daily), None, &p).pool.unwrap().pool_known);
}

#[test]
fn daily_empty_pools_and_newly_minted_pools_have_distinct_membership_evidence() {
    let mut f = frame();
    let daily = f.clone();
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *f.tables[TableId::Divisions.index()]
    {
        rows.push(division(3, 11));
        rows.push(division(4, 11));
    }
    *f.tables[TableId::NewDivisionEquipment.index()] = TableView::Complete(
        TableRows::NewDivisionEquipment(vec![equipment(3, 0, 70, 4_000_000)]),
    );
    let r = reply(&f, Some(&daily), None, &params());
    assert_eq!((r.divisions, r.unknown_divisions), (4, 0));
    assert_eq!(r.types[0].missing_raw.as_deref(), Some("21000000"));
    *f.tables[TableId::NewDivisionEquipment.index()] = TableView::Unavailable {
        reason: "new census refused".into(),
    };
    let r = reply(&f, Some(&daily), None, &params());
    assert_eq!(r.unknown_divisions, 2);
    assert_eq!(r.types[0].missing_raw.as_deref(), Some("5000000"));
    let mut empty = daily.clone();
    *empty.tables[TableId::DivisionEquipment.index()] =
        TableView::Complete(TableRows::DivisionEquipment(vec![]));
    let r = reply(&daily, Some(&empty), None, &params());
    assert_eq!(r.unknown_divisions, 0);
    assert_eq!(r.types[0].missing_raw.as_deref(), Some("20000000"));
}

#[test]
fn reveal_copies_are_excluded_and_owner_host_share_army_attribution() {
    let mut f = frame();
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *f.tables[TableId::Divisions.index()]
    {
        rows[1].expeditionary_owner = 21;
        let mut copy = division(3, 11);
        copy.reveal_skip = 1;
        rows.push(copy);
    }
    let owner = reply(&f, Some(&f), None, &params());
    assert_eq!(owner.divisions, 1);
    let mut p = params();
    p.count_for = ArmyAttribution::Host;
    let host = reply(&f, Some(&f), None, &p);
    assert_eq!(host.divisions, 2);
    p.count_for = ArmyAttribution::Owner;
    p.tag = "ITA".into();
    assert_eq!(reply(&f, Some(&f), None, &p).divisions, 1);
}

#[test]
fn local_clones_change_need_but_models_keep_actual_holdings() {
    let mut f = frame();
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *f.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].subunits[0].local = 1;
    }
    *f.tables[TableId::CountryLocalSubunits.index()] =
        TableView::Complete(TableRows::CountryLocalSubunits(vec![
            CountryLocalSubunitRow {
                country: 9001,
                token: 1,
                requirements: vec![LocalSubunitRequirement {
                    archetype_token: 2,
                    amount_raw: 8_000_000,
                }],
            },
        ]));
    let mut p = params();
    p.division = Some("2".into());
    let r = reply(&f, Some(&f), None, &p);
    let pool = r.pool.unwrap();
    assert_eq!(
        (pool.name.as_deref(), pool.manpower.as_deref()),
        (Some("Synthetic 2"), Some("300"))
    );
    assert_eq!(
        (
            pool.types[0].held_raw.as_deref(),
            pool.types[0].needed_raw.as_deref(),
            pool.types[0].missing_raw.as_deref()
        ),
        (Some("5000000"), Some("8000000"), Some("3000000"))
    );
    assert_eq!(pool.types[0].variants[0].held_raw, "5000000");
}

#[test]
fn unclassified_designs_keep_their_signed_quantity_with_counted_refusals() {
    let mut f = frame();
    *f.tables[TableId::DivisionEquipment.index()] =
        TableView::Complete(TableRows::DivisionEquipment(vec![
            equipment(1, 0, 999, -123_456),
            equipment(2, 0, 999, 1_234_567),
        ]));
    let r = reply(&f, Some(&f), None, &params());
    assert_eq!(r.unknown_divisions, 2);
    let unknown = r.types.iter().find(|row| row.archetype.is_none()).unwrap();
    assert_eq!(unknown.held_raw, "1111111");
    assert!(unknown.needed_raw.is_none() && unknown.missing_raw.is_none());
    assert!(
        r.warnings
            .iter()
            .any(|warning| warning.contains("holdings are unclassified (2)"))
    );
}

#[test]
fn unknown_need_and_unreadable_status_never_become_zero() {
    let mut f = frame();
    *f.tables[TableId::DivisionTemplateIcons.index()] = TableView::Unavailable {
        reason: "composition missing".into(),
    };
    *f.tables[TableId::DivisionStatus.index()] = TableView::Unavailable {
        reason: "status missing".into(),
    };
    let mut p = params();
    p.template = Some("5".into());
    let r = reply(&f, Some(&f), None, &p);
    assert_eq!(r.unknown_divisions, 2);
    let template = &r.templates[0];
    assert!(
        template.needed_raw.is_none()
            && template.held_raw.is_none()
            && template.hp_max_raw.is_none()
    );
    assert_eq!(template.hp_raw.as_deref(), Some("2000000"));
    assert!(
        r.division_rows
            .unwrap()
            .iter()
            .all(|row| row.pool_known && row.needed_raw.is_none() && row.types_short.is_none())
    );
}

#[test]
fn params_are_strict() {
    let mut p = params();
    p.template = Some("5".into());
    p.division = Some("1".into());
    assert!(DivisionEquipment::validate(&p).is_err());
    p.template = None;
    p.division = Some("18446744073709551616".into());
    assert!(DivisionEquipment::validate(&p).is_err());
    let mut wire = serde_json::to_value(params()).unwrap();
    wire["extra"] = true.into();
    assert!(serde_json::from_value::<DivisionEquipmentParams>(wire).is_err());
}

#[test]
fn divisions_across_countries_keep_wide_totals_exact() {
    // regression: 18e091de4d Add army products: losses by month, careers, templates, training queue, division equipment
    let mut f = frame();
    let countries: Vec<_> = (0..4u32)
        .map(|i| CountryRow {
            id: i + 1,
            canonical_slot: i,
            tag: format!("T{i:04}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links: Vec<_> = countries
        .iter()
        .map(|country| CountryLinkRow {
            id: country.id,
            canonical_slot: country.canonical_slot,
        })
        .collect();
    *f.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
    *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links));
    let members: Vec<_> = (1..=4u64).map(|i| division(i, i as u32)).collect();
    let rows: Vec<_> = (1..=4u64)
        .map(|i| equipment(i, 0, 70, if i % 2 == 0 { i64::MAX } else { -3 }))
        .collect();
    *f.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(members));
    *f.tables[TableId::DivisionEquipment.index()] =
        TableView::Complete(TableRows::DivisionEquipment(rows));
    let mut p = params();
    p.tag = "T0003".into();
    let r = reply(&f, Some(&f), None, &p);
    assert_eq!((r.divisions, r.unknown_divisions), (1, 0));
    assert_eq!(r.types[0].held_raw, i64::MAX.to_string());
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *f.tables[TableId::Divisions.index()]
    {
        for row in rows {
            row.roster_owner = 4;
        }
    }
    let r = reply(&f, Some(&f), None, &p);
    assert_eq!((r.divisions, r.unknown_divisions), (4, 0));
    assert_eq!(
        r.types[0].held_raw,
        (2i128 * (i128::from(i64::MAX) - 3)).to_string()
    );
    assert_eq!(
        r.types[0].missing_raw,
        Some((2i128 * 10_000_003).to_string())
    );
    assert_eq!(r.types[0].divisions_short, 2);
}

#[test]
fn the_registered_loader_uses_shared_supplier_frames_without_future_pool_reads() {
    let first = frame();
    let _needs = Needs::from_documents(&[DOCUMENT.as_bytes()], &first);
    let root = std::env::temp_dir().join(format!(
        "division-equipment-query-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _folder = dashboard_text::catalogue::tests::Folder(root.clone());
    std::fs::create_dir_all(root.join("common/units/equipment")).unwrap();
    std::fs::write(root.join("common/units/equipment/test.txt"), DOCUMENT).unwrap();
    std::fs::write(root.join("common/units/test.txt"), DOCUMENT).unwrap();
    let frames = (2..=10)
        .map(|offset| {
            let mut f = frame();
            f.hour = 60_000_000 + offset;
            // The recording starts between design census slots. Time its initial
            // census explicitly rather than claiming an off-slot snapshot is a read.
            if offset == 2 {
                f.metrics.table_ns[TableId::VariantModules.index()] = 1;
            }
            if offset >= 4 {
                if let TableView::Complete(TableRows::Divisions(rows)) =
                    &mut *f.tables[TableId::Divisions.index()]
                {
                    rows.push(division(3, 11));
                }
                *f.tables[TableId::NewDivisionEquipment.index()] = TableView::Complete(
                    TableRows::NewDivisionEquipment(vec![equipment(3, 0, 70, 3_000_000)]),
                );
            }
            f
        })
        .collect();
    let (memory, context) = dashboard_base::test_fixtures::source_with_content(frames, &root);
    // Reached replies need a real recording binding; an in-memory fixture
    // intentionally bypasses that observation cache.
    let path = root.join("campaign.battleplan");
    std::fs::write(&path, memory.replay_ref().bytes()).unwrap();
    let flag = AtomicBool::new(false);
    let source = Source::from_replay(
        dashboard_base::SourceKey {
            source_id: root.to_string_lossy().into_owned(),
            revision: dashboard_base::protocol::replay_revision(&path).unwrap(),
        },
        path,
        memory.replay(),
        &flag,
    )
    .unwrap();
    let p = params();
    let r = DivisionEquipment::load(&source, &p, &context, &flag).unwrap();
    assert_eq!(
        (r.divisions, r.unknown_divisions, r.pool_hour),
        (3, 0, Some(60_000_002)),
        "{:?}",
        r.warnings
    );
    assert_eq!(r.types[0].missing_raw.as_deref(), Some("12000000"));
    assert_eq!(r.types[0].stockpile_hour, Some(60_000_008));
    let reads = dashboard_base::prepared::query::frames::reads(&source);
    let (again, decoded) = progressive::without_block_decodes(|| {
        DivisionEquipment::load(&source, &p, &context, &flag)
    });
    assert!(!decoded);
    assert_eq!(
        serde_json::to_value(again.unwrap()).unwrap(),
        serde_json::to_value(r).unwrap()
    );
    assert_eq!(
        dashboard_base::prepared::query::frames::reads(&source),
        reads
    );
    let mut earlier = p;
    earlier.hour = 60_000_003;
    let early = DivisionEquipment::load(&source, &earlier, &context, &flag).unwrap();
    assert_eq!(
        early.divisions, 2,
        "no future minted division enters the earlier census"
    );
    assert!(DivisionEquipment::load(&source, &earlier, &context, &AtomicBool::new(true)).is_err());
}

#[test]
fn stockpile_failure_or_identity_replacement_cannot_change_deployed_shortfalls() {
    let f = frame();
    let mut stock = f.clone();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *stock.tables[TableId::Countries.index()]
    {
        rows.iter_mut()
            .find(|row| row.canonical_slot == 7)
            .unwrap()
            .id = 9999;
    }
    let r = reply(&f, Some(&f), Some(&stock), &params());
    assert_eq!(r.types[0].missing_raw.as_deref(), Some("5000000"));
    assert!(r.types[0].stockpile_raw.is_none() && r.types[0].stockpile_hour.is_none());
    let mut p = params();
    p.tag = "not present".into();
    let absent = reply(&f, Some(&f), None, &p);
    assert!(absent.absent && absent.divisions == 0 && absent.templates.is_empty());
}

#[test]
fn repeated_variants_and_negative_holdings_sum_before_each_division_comparison() {
    let mut f = frame();
    *f.tables[TableId::DivisionEquipment.index()] =
        TableView::Complete(TableRows::DivisionEquipment(vec![
            equipment(1, 0, 70, 6_000_000),
            equipment(1, 1, 70, -1_000_000),
            equipment(2, 0, 70, 5_000_000),
        ]));
    let r = reply(&f, Some(&f), None, &params());
    assert_eq!(r.types[0].held_raw, "10000000");
    assert_eq!(r.types[0].missing_raw.as_deref(), Some("10000000"));
    assert_eq!(r.types[0].variants.len(), 1);
    assert_eq!(r.types[0].variants[0].held_raw, "10000000");
    assert_eq!(r.types[0].divisions_short, 2);
}

#[test]
fn conversion_uses_current_need_without_redating_the_sampled_pool() {
    let mut f = frame();
    let mut daily = f.clone();
    daily.hour -= 8;
    if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
        &mut *f.tables[TableId::DivisionTemplateIcons.index()]
    {
        rows[0].subunits[0].count = 2;
        rows.push(DivisionTemplateIconRow {
            template: 9,
            display_name: Recorded::Value(Some(TemplateDisplayName {
                value: TemplateNameValue::Changed {
                    literal: b"Previous formation".to_vec(),
                    fallback_key: None,
                },
                ..Default::default()
            })),
            ..Default::default()
        });
    }
    *f.tables[TableId::DivisionTemplates.index()] =
        TableView::Complete(TableRows::DivisionTemplates(vec![DivisionTemplateRow {
            template: 9,
            ..Default::default()
        }]));
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *f.tables[TableId::DivisionStatus.index()]
    {
        rows[0].pending_template = Recorded::Value(Some(9));
    }
    let mut p = params();
    p.template = Some("5".into());
    let r = reply(&f, Some(&daily), None, &p);
    assert_eq!(r.pool_hour, Some(daily.hour));
    let row = &r.division_rows.unwrap()[0];
    assert_eq!(row.converting_from.as_deref(), Some("Previous formation"));
    assert_eq!(row.needed_raw.as_deref(), Some("20000000"));
    assert_eq!(row.held_raw.as_deref(), Some("15000000"));
    assert_eq!(row.types_short, Some(1));
}

#[test]
fn a_division_of_an_unlinked_country_is_left_out_and_counted_not_fatal() {
    let mut f = frame();
    if let TableView::Complete(TableRows::Divisions(rows)) = &mut *f.tables[TableId::Divisions.index()] {
        rows.push(division(3, 4_242));
    }
    let r = reply(&f, Some(&f), None, &params());
    assert_eq!(r.divisions, 2, "the unlinked division counts for no country");
    assert_eq!(r.divisions_short, 1, "distinct divisions with a known shortfall");
    assert!(r.warnings.iter().any(|warning| warning.contains("no recorded link")));
}

#[test]
fn a_types_fill_sums_each_divisions_capped_fulfilment_so_surplus_never_hides_a_shortfall() {
    // Each division needs 100 widgets; one holds 150, the other 50.
    let f = frame();
    let r = reply(&f, Some(&f), None, &params());
    let kind = &r.types[0];
    assert_eq!(
        (kind.held_raw.as_str(), kind.needed_raw.as_deref(), kind.filled_raw.as_deref(), kind.missing_raw.as_deref()),
        ("20000000", Some("20000000"), Some("15000000"), Some("5000000")),
        "held stays the actual holding; filled counts no division past its own need"
    );
}

#[test]
fn classified_holdings_count_when_a_divisions_comparison_is_unknown() {
    let mut f = frame();
    *f.tables[TableId::DivisionTemplateIcons.index()] = TableView::Unavailable {
        reason: "composition missing".into(),
    };
    let r = reply(&f, Some(&f), None, &params());
    assert_eq!(r.unknown_divisions, 2);
    let widgets = r.types.iter().find(|row| row.archetype.as_deref() == Some("widgets")).unwrap();
    assert_eq!(widgets.held_raw, "20000000", "both pools were read and their designs classified");
    assert!(widgets.needed_raw.is_none() && widgets.filled_raw.is_none() && widgets.missing_raw.is_none(),
        "unknown need and shortfall stay unknown");
    assert_eq!(widgets.unknown_divisions, 2);
}

#[test]
fn supplying_reads_made_at_once_are_the_reads_made_in_order() {
    // Daily pools, stockpiles and designs dated apart from the reached hour, across days.
    let frames: Vec<_> = (2..=60)
        .map(|offset| {
            let mut f = frame();
            f.hour = 60_000_000 + offset;
            if offset == 2 {
                f.metrics.table_ns[TableId::VariantModules.index()] = 1;
            }
            if offset >= 30 {
                if let TableView::Complete(TableRows::Divisions(rows)) =
                    &mut *f.tables[TableId::Divisions.index()]
                {
                    rows.push(division(3, 11));
                }
            }
            f
        })
        .collect();
    let replay = Arc::new(
        replay::container::Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(1, frames)],
        )
        .unwrap(),
    );
    let flag = AtomicBool::new(false);
    // Each way reads its own source, so neither answers from the other's cache.
    let source = |id: &str| {
        Source::from_replay(
            dashboard_base::SourceKey {
                source_id: id.into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::clone(&replay),
            &flag,
        )
        .unwrap()
    };
    let (together, apart) = (source("at-once"), source("in-order"));
    for hour in [60_000_002, 60_000_013, 60_000_030, 60_000_047, 60_000_060] {
        let fields = |supplied: Supplied| {
            (
                supplied.frame,
                supplied.daily,
                supplied.stock,
                supplied.designs,
                supplied.stock_designs,
            )
        };
        let at_once = fields(supplied_with(&together, 1, hour, &flag, true).unwrap());
        let in_order = fields(supplied_with(&apart, 1, hour, &flag, false).unwrap());
        assert_eq!(at_once, in_order, "hour {hour}");
    }
}

/// A branch with dated reads, a minted division, a refused stockpile, a missing hour and an identity change.
fn history_frames() -> Vec<DecodedFrame> {
    (2..=80)
        .filter(|offset| *offset != 61)
        .map(|offset| {
            let mut f = frame();
            f.hour = 60_000_000 + offset;
            if offset == 2 {
                f.metrics.table_ns[TableId::VariantModules.index()] = 1;
            }
            *f.tables[TableId::DivisionTemplates.index()] =
                TableView::Complete(TableRows::DivisionTemplates(vec![DivisionTemplateRow {
                    template: 5,
                    owner: 11,
                    ..Default::default()
                }]));
            if let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
                &mut *f.tables[TableId::DivisionTemplateIcons.index()]
            {
                rows[0].display_name = Recorded::Value(Some(TemplateDisplayName {
                    sample_hour: PacketHour::Host,
                    value: TemplateNameValue::Changed {
                        literal: b"Line division".to_vec(),
                        fallback_key: None,
                    },
                }));
            }
            if offset >= 4 {
                if let TableView::Complete(TableRows::Divisions(rows)) =
                    &mut *f.tables[TableId::Divisions.index()]
                {
                    rows.push(division(3, 11));
                }
                *f.tables[TableId::NewDivisionEquipment.index()] = TableView::Complete(
                    TableRows::NewDivisionEquipment(vec![equipment(3, 0, 70, 3_000_000)]),
                );
            }
            if offset >= 30
                && let TableView::Complete(TableRows::Divisions(rows)) =
                    &mut *f.tables[TableId::Divisions.index()]
            {
                rows[0].hit_points = 400_000;
            }
            if (12..=14).contains(&offset) {
                *f.tables[TableId::Stockpiles.index()] = TableView::Unavailable {
                    reason: "stockpile walk refused".into(),
                };
            }
            if offset >= 50 {
                // GER's slot is held by another country identity from here.
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *f.tables[TableId::Countries.index()]
                    && let Some(row) = rows.iter_mut().find(|row| row.tag == b"GER")
                {
                    row.id = 9002;
                    rows.sort_by_key(|row| row.id);
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *f.tables[TableId::CountryLinks.index()]
                {
                    let slot = rows.iter().find(|row| row.id == 11).map(|row| row.canonical_slot);
                    rows.push(CountryLinkRow {
                        id: 9002,
                        canonical_slot: slot.unwrap_or(7),
                    });
                    rows.sort_by_key(|row| row.id);
                }
            }
            f
        })
        .collect()
}

const HISTORY_FIRST: i32 = 60_000_002;
const HISTORY_LAST: i32 = 60_000_080;
const CONTINUATION: &str = "division-equipment-continuation-1";

/// Prepare branch 1 as its family does, saving and restoring after `resume_after`.
fn prepare_history(
    source: &Source,
    context: &LoadContext,
    fixture: &mut dashboard_base::prepared::series::tests::Fixture,
    resume_after: Option<i32>,
) {
    let flag = AtomicBool::new(false);
    let replay = source.replay_ref();
    let mut prepared = history::Prepared::new();
    let mut writer = fixture.writer();
    for index in replay.index.iter().filter(|index| index.branch == 1) {
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        let mut ended = None;
        prepared.configure(source, context);
        replay
            .read_block_recorded_alone(
                index,
                history::tables(),
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, raw, replay.format);
                    if ended.is_none() {
                        prepared.observe(frame, &changes, replay)?;
                    }
                    if resume_after == Some(frame.hour) {
                        ended = Some(frame.hour);
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        let Some(resumed) = ended else {
            continue;
        };
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer
            .write_at(CONTINUATION, 0, serde_json::to_vec(&prepared).unwrap().as_slice())
            .unwrap();
        writer.commit().unwrap();
        let saved = fixture.store.read(&fixture.key, CONTINUATION, 0).unwrap().unwrap();
        prepared = serde_json::from_reader(io::BufReader::new(saved)).unwrap();
        prepared
            .restore(|at| fixture.store.read(&fixture.key, &history::catalogue_section(1), at))
            .unwrap();
        prepared.configure(source, context);
        writer = fixture.append_writer();
        // The later run reads the block again and goes on after the hour it saved.
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        replay
            .read_block_recorded_alone(
                index,
                history::tables(),
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, raw, replay.format);
                    if frame.hour > resumed {
                        prepared.observe(frame, &changes, replay)?;
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
}

fn history_of(
    source: &Source,
    context: &LoadContext,
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    tag: &str,
    count_for: ArmyAttribution,
    through: Option<i32>,
) -> history::DivisionEquipmentHistoryReply {
    let flag = AtomicBool::new(false);
    let params = serde_json::json!({"branchId": "1", "tag": tag, "countFor": count_for});
    let prefix = dashboard_base::prepared::prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: HISTORY_FIRST,
            last_hour: through.unwrap_or(HISTORY_LAST),
            complete: through.is_none(),
        },
        pending: Vec::new(),
    };
    let mut history = history::history(&dashboard_keyed::query::Query {
        source,
        params: &params,
        content: context.content.as_ref(),
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    })
    .unwrap();
    // As its server adds them to every serving.
    history.served = history::served_lines(source, context.content.as_ref());
    history
}

/// The reached product's reply as its server serves it.
fn served(source: &Source, p: &DivisionEquipmentParams, context: &LoadContext) -> serde_json::Value {
    let flag = AtomicBool::new(false);
    let (reply, own) =
        dashboard_text::catalogue::capture(|| DivisionEquipment::load(source, p, context, &flag));
    let mut reply = reply.unwrap();
    source.append_content_warnings(&mut reply.warnings);
    source.append_presentation_warnings(&mut reply.warnings, context.content.as_ref());
    reply.warnings.extend(own.fallback_line());
    serde_json::to_value(reply).unwrap()
}

fn history_source() -> (Source, LoadContext, dashboard_text::catalogue::tests::Folder) {
    content_blocks(history_frames(), &[], "history")
}

/// `frames` against the synthetic content, a new block starting at each of `starts`.
fn content_blocks(
    frames: Vec<DecodedFrame>,
    starts: &[i32],
    name: &str,
) -> (Source, LoadContext, dashboard_text::catalogue::tests::Folder) {
    content_blocks_with(frames, starts, name, DOCUMENT)
}

/// [`content_blocks`] with `document` as the content's units and equipment.
fn content_blocks_with(
    frames: Vec<DecodedFrame>,
    starts: &[i32],
    name: &str,
    document: &str,
) -> (Source, LoadContext, dashboard_text::catalogue::tests::Folder) {
    let root = std::env::temp_dir().join(format!(
        "division-equipment-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let folder = dashboard_text::catalogue::tests::Folder(root.clone());
    std::fs::create_dir_all(root.join("common/units/equipment")).unwrap();
    std::fs::create_dir_all(root.join("localisation/english")).unwrap();
    std::fs::write(root.join("common/units/equipment/test.txt"), document).unwrap();
    std::fs::write(root.join("common/units/test.txt"), document).unwrap();
    std::fs::write(
        root.join("localisation/english/test_l_english.yml"),
        "\u{feff}l_english:\n widgets:0 \"Widgets\"\n",
    )
    .unwrap();
    let mut blocks: Vec<Vec<DecodedFrame>> = vec![Vec::new()];
    for frame in frames {
        if starts.contains(&frame.hour) {
            blocks.push(Vec::new());
        }
        blocks.last_mut().unwrap().push(frame);
    }
    let mut blocks = blocks.into_iter();
    let (first, context) =
        dashboard_base::test_fixtures::source_with_content(blocks.next().unwrap(), &root);
    let mut replay: Option<replay::container::Replay> = None;
    for block in blocks {
        let last = replay.as_ref().unwrap_or_else(|| first.replay_ref());
        let records = dashboard_base::test_fixtures::continuation_records(last, 1, block);
        replay = Some(last.append(&[records]).unwrap());
    }
    let Some(replay) = replay else {
        return (first, context, folder);
    };
    assert_eq!(replay.index.len(), starts.len() + 1);
    let source = Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "content-fixture".into(),
            revision: format!("{}", starts.len() + 1),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    (source, context, folder)
}

fn recorded(source: &Source, hour: i32) -> bool {
    source
        .replay_ref()
        .index
        .iter()
        .any(|block| block.branch == 1 && block.first_hour <= hour && hour <= block.last_hour)
        && hour != 60_000_061
}

#[test]
fn every_hour_of_the_prepared_history_is_the_served_reached_reply() {
    let (source, context, _folder) = history_source();
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    prepare_history(&source, &context, &mut fixture, None);
    for tag in ["GER", "ITA", "SOV"] {
        for count_for in [ArmyAttribution::Owner, ArmyAttribution::Host] {
            let history = history_of(&source, &context, &fixture, tag, count_for, None);
            for hour in (HISTORY_FIRST..=HISTORY_LAST).filter(|hour| recorded(&source, *hour)) {
                let mut p = params();
                (p.tag, p.count_for, p.hour) = (tag.into(), count_for, hour);
                let held = history::reply_at(&history, hour)
                    .unwrap_or_else(|| panic!("{tag} {count_for:?} {hour} is not in the history"));
                assert_eq!(
                    serde_json::to_value(&held).unwrap(),
                    served(&source, &p, &context),
                    "{tag} {count_for:?} at {hour}"
                );
            }
        }
    }
    // Names resolved in the shown language reach the rebuilt reply.
    let history = history_of(&source, &context, &fixture, "GER", ArmyAttribution::Owner, None);
    let reply = history::reply_at(&history, 60_000_020).unwrap();
    assert_eq!(reply.types[0].name.as_deref(), Some("Widgets"));
    assert_eq!(reply.templates[0].name.as_deref(), Some("Line division"));
    assert_eq!(reply.types[0].variants[0].name.as_deref(), Some("Synthetic model"));
}

#[test]
fn unreadable_template_tables_kept_across_blocks_answer_as_the_served_reply_each_hour() {
    let mut frames = history_frames();
    for f in &mut frames {
        let offset = f.hour - 60_000_000;
        if (20..=40).contains(&offset) {
            *f.tables[TableId::DivisionTemplateIcons.index()] = TableView::Unavailable {
                reason: "template icon walk refused".into(),
            };
        }
        if (45..=58).contains(&offset) {
            *f.tables[TableId::DivisionTemplates.index()] = TableView::Truncated {
                rows: TableRows::DivisionTemplates(vec![DivisionTemplateRow {
                    template: 5,
                    owner: 11,
                    ..Default::default()
                }]),
                missing: 1,
            };
        }
    }
    // Each stretch is unchanged across a block's first hour, and its refusals name each hour.
    let (source, context, _folder) =
        content_blocks(frames, &[60_000_030, 60_000_052], "unreadable");
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    prepare_history(&source, &context, &mut fixture, None);
    for tag in ["GER", "ITA"] {
        for count_for in [ArmyAttribution::Owner, ArmyAttribution::Host] {
            let history = history_of(&source, &context, &fixture, tag, count_for, None);
            let mut answered = BTreeSet::new();
            for hour in (HISTORY_FIRST..=HISTORY_LAST).filter(|hour| recorded(&source, *hour)) {
                let Some(held) = history::reply_at(&history, hour) else {
                    continue;
                };
                let mut p = params();
                (p.tag, p.count_for, p.hour) = (tag.into(), count_for, hour);
                assert_eq!(
                    serde_json::to_value(&held).unwrap(),
                    served(&source, &p, &context),
                    "{tag} {count_for:?} at {hour}"
                );
                answered.insert(hour - 60_000_000);
            }
            for offset in [29, 30, 31, 51, 52, 53] {
                assert!(answered.contains(&offset), "{tag} {count_for:?} at +{offset}");
            }
        }
    }
}

#[test]
fn a_resumed_history_keeps_its_published_hours_and_proves_its_reads_again() {
    let (source, context, _folder) = history_source();
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let resume = 60_000_010;
    prepare_history(&source, &context, &mut fixture, Some(resume));
    let history = history_of(&source, &context, &fixture, "GER", ArmyAttribution::Owner, None);
    let mut answered_after = 0;
    for hour in (HISTORY_FIRST..=HISTORY_LAST).filter(|hour| recorded(&source, *hour)) {
        let mut p = params();
        p.hour = hour;
        match history::reply_at(&history, hour) {
            Some(held) => {
                assert_eq!(serde_json::to_value(&held).unwrap(), served(&source, &p, &context), "at {hour}");
                answered_after += usize::from(hour > resume);
            }
            None => assert!(hour > resume, "{hour}, published before the resume, is missing"),
        }
    }
    // A window ending by the resume reads only the page its hours name, and answers alike.
    let early = history_of(&source, &context, &fixture, "GER", ArmyAttribution::Owner, Some(resume));
    for hour in (HISTORY_FIRST..=resume).filter(|hour| recorded(&source, *hour)) {
        assert_eq!(
            history::reply_at(&early, hour).map(|reply| serde_json::to_value(reply).unwrap()),
            history::reply_at(&history, hour).map(|reply| serde_json::to_value(reply).unwrap()),
            "at {hour}"
        );
    }
    // Within two days of the resume every carry is proved again.
    for hour in (resume + 48..=HISTORY_LAST).filter(|hour| recorded(&source, *hour)) {
        assert!(history::reply_at(&history, hour).is_some(), "{hour}");
    }
    assert!(answered_after > 0);
}

#[test]
fn a_history_answers_only_through_its_prefix_and_window_and_keeps_absence() {
    let (source, context, _folder) = history_source();
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    prepare_history(&source, &context, &mut fixture, None);
    let history = history_of(
        &source,
        &context,
        &fixture,
        "GER",
        ArmyAttribution::Owner,
        Some(60_000_020),
    );
    assert!(history::reply_at(&history, 60_000_020).is_some());
    assert!(history::reply_at(&history, 60_000_021).is_none());
    // A tag no country holds is absent at a prepared hour, not unknown.
    let absent = history_of(&source, &context, &fixture, "SOV", ArmyAttribution::Owner, None);
    assert!(history::reply_at(&absent, 60_000_020).is_some_and(|reply| reply.absent));
    // A window answers its own hours only.
    let flag = AtomicBool::new(false);
    let params = serde_json::json!({"branchId": "1", "tag": "GER", "countFor": "owner",
        "firstHour": 60_000_030, "lastHour": 60_000_040});
    let prefix = dashboard_base::prepared::prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: HISTORY_FIRST,
            last_hour: HISTORY_LAST,
            complete: true,
        },
        pending: Vec::new(),
    };
    let window = history::history(&dashboard_keyed::query::Query {
        source: &source,
        params: &params,
        content: context.content.as_ref(),
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    })
    .unwrap();
    assert!(history::reply_at(&window, 60_000_029).is_none());
    assert!(history::reply_at(&window, 60_000_035).is_some());
    assert!(history::reply_at(&window, 60_000_041).is_none());
}

/// The countries a scale case holds, their links, and the frame of an hour holding them.
fn scale_frame(count: u32, offset: i32) -> DecodedFrame {
    let countries: Vec<_> = (0..count)
        .map(|at| CountryRow {
            id: 100 + at,
            canonical_slot: 1_000 + at,
            tag: format!("C{at:04}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links: Vec<_> = countries
        .iter()
        .map(|row| CountryLinkRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
        })
        .collect();
    let mut f = frame();
    f.hour = 60_000_000 + offset;
    *f.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
    *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links));
    *f.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        division(1, 100),
        division(2, 101),
    ]));
    f
}

/// Each hour's builds and scanned rows, observed as preparation reads each block: alone,
/// every table changed at its first hour.
fn observed_hours(
    source: &Source,
    context: &LoadContext,
) -> (history::Prepared, Vec<(i32, usize, usize)>) {
    let replay = source.replay_ref();
    let mut prepared = history::Prepared::new();
    prepared.configure(source, context);
    let mut hours = Vec::new();
    for index in replay.index.iter().filter(|index| index.branch == 1) {
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        replay
            .read_block_recorded_alone(
                index,
                history::tables(),
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, raw, replay.format);
                    let (builds, scans) = (prepared.builds, history::SCANNED.get());
                    prepared.observe(frame, &changes, replay)?;
                    hours.push((
                        frame.hour,
                        prepared.builds - builds,
                        history::SCANNED.get() - scans,
                    ));
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    (prepared, hours)
}

/// [`scale_frame`] with no supplying read: its pools, stockpile and designs are unavailable.
fn unsupplied_frame(count: u32, offset: i32) -> DecodedFrame {
    let mut f = scale_frame(count, offset);
    for table in [
        TableId::DivisionEquipment,
        TableId::Stockpiles,
        TableId::VariantDefinitions,
    ] {
        *f.tables[table.index()] = TableView::Unavailable {
            reason: "supplying walk not recorded".into(),
        };
    }
    f
}

/// Branch 1's history of `tag`'s owner replies, prepared whole.
fn whole_history(
    source: &Source,
    context: &LoadContext,
    prepared: &mut history::Prepared,
    tag: &str,
    last_hour: i32,
) -> history::DivisionEquipmentHistoryReply {
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    let asked = serde_json::json!({"branchId": "1", "tag": tag, "countFor": "owner"});
    let prefix = dashboard_base::prepared::prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: 60_000_002,
            last_hour,
            complete: true,
        },
        pending: Vec::new(),
    };
    let mut reply = history::history(&dashboard_keyed::query::Query {
        source,
        params: &asked,
        content: context.content.as_ref(),
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &AtomicBool::new(false),
    })
    .unwrap();
    reply.served = history::served_lines(source, context.content.as_ref());
    reply
}

#[test]
fn template_icons_unavailable_all_session_cost_quiet_hours_nothing_and_keep_one_note() {
    let count = 1_500;
    // Every hour's template icons are unavailable alike, across a block's first hour, and
    // no supplying read refreshes anything.
    let frames: Vec<_> = (2..=40)
        .map(|offset| {
            let mut f = unsupplied_frame(count, offset);
            *f.tables[TableId::DivisionTemplateIcons.index()] = TableView::Unavailable {
                reason: "template hook failed".into(),
            };
            f
        })
        .collect();
    let (source, context, _folder) = content_blocks(frames, &[60_000_020], "unavailable");
    let (mut prepared, hours) = observed_hours(&source, &context);
    assert_eq!(hours.len(), 39);
    assert_eq!((hours[0].0, hours[0].1), (60_000_002, 2 * count as usize));
    for &(hour, builds, scans) in &hours[1..] {
        assert_eq!(builds, 0, "{hour} rebuilt {builds} replies");
        if hour != 60_000_020 {
            assert_eq!(scans, 0, "{hour} scanned {scans} rows");
        }
    }
    let reply = whole_history(&source, &context, &mut prepared, "C0000", 60_000_040);
    // Refusals that hold every hour keep one text each: one run across the window.
    let notes: Vec<_> = reply
        .window
        .series
        .iter()
        .filter(|series| series.name.starts_with("note|"))
        .collect();
    assert_eq!(notes.len(), 4, "{notes:?}");
    for series in &notes {
        assert_eq!(series.runs.len(), 1, "{series:?}");
        assert_eq!((series.runs[0].0, series.runs[0].1), (60_000_002, 60_000_040));
    }
    assert_eq!(reply.hours.len(), 1, "{:?}", reply.hours);
    assert_eq!(
        (reply.hours[0].first_hour, reply.hours[0].last_hour),
        (60_000_002, 60_000_040)
    );
    // Each hour's reply, its refusals and missing names and icons included, names its own
    // hour as the reached product's does.
    for hour in 60_000_002..=60_000_040 {
        let held = history::reply_at(&reply, hour).expect("an answered hour");
        let mut p = params();
        (p.tag, p.hour) = ("C0000".into(), hour);
        assert_eq!(serde_json::to_value(&held).unwrap(), served(&source, &p, &context), "at {hour}");
    }
}

#[test]
fn a_content_key_worded_as_an_unreadable_table_stays_as_written_at_every_hour() {
    // A mod's missing equipment key worded exactly as the first hour's icons refusal.
    let key = "hour 60000002 DivisionTemplateIcons is unavailable: template hook failed";
    let document = DOCUMENT.replace(
        "need = { widgets = 100 }",
        &format!("need = {{ widgets = 100 \"{key}\" = 1 }}"),
    );
    let frames: Vec<_> = (2..=6)
        .map(|offset| {
            let mut f = unsupplied_frame(3, offset);
            *f.tables[TableId::DivisionTemplateIcons.index()] = TableView::Unavailable {
                reason: "template hook failed".into(),
            };
            f
        })
        .collect();
    let (source, context, _folder) = content_blocks_with(frames, &[], "collision", &document);
    let (mut prepared, _) = observed_hours(&source, &context);
    let reply = whole_history(&source, &context, &mut prepared, "C0000", 60_000_006);
    for hour in 60_000_002..=60_000_006 {
        let held = history::reply_at(&reply, hour).expect("an answered hour");
        let mut p = params();
        (p.tag, p.hour) = ("C0000".into(), hour);
        assert_eq!(serde_json::to_value(&held).unwrap(), served(&source, &p, &context), "at {hour}");
        // The content's words keep the hour they were written with.
        assert!(
            held.warnings.iter().any(|line| line.contains(key)),
            "{hour}: {:?}",
            held.warnings
        );
    }
}

#[test]
fn a_missing_dictionary_row_is_refused_before_unreadable_icons_as_served() {
    // The template's dictionary is recorded without its row, and its icons are unavailable:
    // the naming refuses the missing row first.
    let frames: Vec<_> = (2..=4)
        .map(|offset| {
            let mut f = unsupplied_frame(3, offset);
            *f.tables[TableId::DivisionTemplates.index()] =
                TableView::Complete(TableRows::DivisionTemplates(vec![DivisionTemplateRow {
                    template: 6,
                    owner: 11,
                    ..Default::default()
                }]));
            *f.tables[TableId::DivisionTemplateIcons.index()] = TableView::Unavailable {
                reason: "template hook failed".into(),
            };
            f
        })
        .collect();
    let (source, context, _folder) = content_blocks(frames, &[], "dictionary");
    let (mut prepared, _) = observed_hours(&source, &context);
    let reply = whole_history(&source, &context, &mut prepared, "C0000", 60_000_004);
    for hour in 60_000_002..=60_000_004 {
        let held = history::reply_at(&reply, hour).expect("an answered hour");
        let mut p = params();
        (p.tag, p.hour) = ("C0000".into(), hour);
        assert_eq!(serde_json::to_value(&held).unwrap(), served(&source, &p, &context), "at {hour}");
        assert!(
            held.templates.iter().any(|template| template
                .name_missing
                .as_deref()
                .is_some_and(|missing| missing.contains("dictionary missing"))),
            "{hour}: {:?}",
            held.templates
        );
    }
}

#[test]
fn a_stockpile_read_rebuilds_every_country_that_hour() {
    let count = 3;
    // Only stockpiles are read: their first read is at 8, and nothing else changes.
    let frames: Vec<_> = (2..=10)
        .map(|offset| {
            let mut f = unsupplied_frame(count, offset);
            f.tables[TableId::Stockpiles.index()] = frame().tables[TableId::Stockpiles.index()].clone();
            f
        })
        .collect();
    let (source, context, _folder) = content_blocks(frames, &[], "stockpile");
    let (_, hours) = observed_hours(&source, &context);
    let builds: Vec<_> = hours.iter().map(|&(hour, builds, _)| (hour, builds)).collect();
    let every = 2 * count as usize;
    assert_eq!(
        builds,
        [
            (60_000_002, every),
            (60_000_003, 0),
            (60_000_004, 0),
            (60_000_005, 0),
            (60_000_006, 0),
            (60_000_007, 0),
            (60_000_008, every),
            (60_000_009, 0),
            (60_000_010, 0),
        ]
    );
}

#[test]
fn an_hour_rebuilds_and_scans_only_what_its_changes_reach_among_fifteen_hundred() {
    let count = 1_500;
    // Two blocks: the second's first hour, 5, marks every table changed. At 6 the one
    // template both divisions use changes its manpower.
    let frames: Vec<_> = (2..=8)
        .map(|offset| {
            let mut f = scale_frame(count, offset);
            let (mut first, mut second) = (division(1, 100), division(2, 101));
            if offset >= 4 {
                first.hit_points = 500_000;
            }
            if offset >= 5 {
                second.hit_points = 500_000;
            }
            *f.tables[TableId::Divisions.index()] =
                TableView::Complete(TableRows::Divisions(vec![first, second]));
            if offset >= 6
                && let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
                    &mut *f.tables[TableId::DivisionTemplateIcons.index()]
            {
                rows[0].settings = Recorded::Value(Some(Arc::new(TemplateSettings {
                    total_manpower: 2000,
                    ..Default::default()
                })));
            }
            f
        })
        .collect();
    let (source, context, _folder) = content_blocks(frames, &[60_000_005], "scale");
    let (mut prepared, hours) = observed_hours(&source, &context);
    let builds: Vec<_> = hours.iter().map(|&(hour, builds, _)| (hour, builds)).collect();
    // Every country at the first hour, none at a quiet one, one country after one change,
    // including at the second block's first hour, which compares its rows with the hour
    // before, and the two countries using a template after it changes.
    let every = 2 * count as usize;
    assert_eq!(
        builds,
        [
            (60_000_002, every),
            (60_000_003, 0),
            (60_000_004, 2),
            (60_000_005, 2),
            (60_000_006, 4),
            (60_000_007, 0),
            (60_000_008, 0),
        ]
    );
    // A quiet hour, one changing a division by its delta, and one changing a template list
    // no country's rows: the template's need resolves through the sources kept since the
    // first hour.
    for &(hour, _, scans) in &hours {
        if hour != 60_000_002 && hour != 60_000_005 {
            assert!(scans < 10, "{hour} scanned {scans} rows among {count}");
        }
    }
    // Pending is what the next commit publishes and releases; held is what stays, the
    // continuation included: together they cover all it writes.
    let (pending, held) = (prepared.bytes(), prepared.held_bytes());
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    let continuation = serde_json::to_vec(&prepared).unwrap();
    writer.write_at(CONTINUATION, 0, continuation.as_slice()).unwrap();
    writer.commit().unwrap();
    let length = |section: &str, range: u64| {
        let mut bytes = Vec::new();
        if let Some(mut read) = fixture.store.read(&fixture.key, section, range).unwrap() {
            io::Read::read_to_end(&mut read, &mut bytes).unwrap();
        }
        bytes.len()
    };
    let page = length(&history::catalogue_section(1), 0);
    let runs: usize = (0..)
        .map(|range| length(&dashboard_keyed::series_section(history::NAME, 1), range))
        .take_while(|bytes| *bytes > 0)
        .sum();
    let written = runs + page + length(CONTINUATION, 0);
    assert!(pending + held >= written, "{pending} pending and {held} held, {written} written");
    assert!(pending >= runs + page, "{pending} pending, {runs} of runs and {page} of catalogue written");
    // A flush releases what was pending; the continuation stays held.
    assert!(prepared.bytes() < pending);
    // One country's history reads the page's directory, its own names and the operands it
    // names, not every country's.
    let read = history::CATALOGUE_READ.get();
    let params = serde_json::json!({"branchId": "1", "tag": "C0001", "countFor": "owner"});
    let prefix = dashboard_base::prepared::prefix::Prefix {
        coverage: dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: 60_000_002,
            last_hour: 60_000_008,
            complete: true,
        },
        pending: Vec::new(),
    };
    let reply = history::history(&dashboard_keyed::query::Query {
        source: &source,
        params: &params,
        content: context.content.as_ref(),
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &AtomicBool::new(false),
    })
    .unwrap();
    assert!(history::reply_at(&reply, 60_000_005).is_some());
    let read = history::CATALOGUE_READ.get() - read;
    assert!(read * 4 < page as u64, "{read} of a {page} byte page read for one country");
}
