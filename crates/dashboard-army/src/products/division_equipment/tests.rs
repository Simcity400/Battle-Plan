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
    assert_eq!(
        (rows[1].manpower.as_deref(), rows[1].hp_raw.as_deref()),
        (Some("300"), Some("1000000"))
    );
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
            pool.types[0].held_raw.as_str(),
            pool.types[0].needed_raw.as_deref(),
            pool.types[0].missing_raw.as_deref()
        ),
        ("5000000", Some("8000000"), Some("3000000"))
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
            .all(|row| row.pool_known && row.needed_raw.is_none())
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
    assert_eq!(row.types_short, 1);
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
