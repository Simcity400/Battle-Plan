use super::*;
use replay_data::{
    rows::CountryLinkRow, rows::DivisionRow, rows::DivisionStatusRow, rows::DivisionSupplyFooter,
    rows::TableId, rows::TableRows, rows::UnitSupplyOrigin,
};
use replay_frame::frame::TableView;
use viewer_native_gui::counter_hovers::land::CounterControl;
use viewer_foundation::test_support::{test_frame, test_put as put};

fn loc() -> Localisation {
    [
        ("NEEDED_SUPPLY_WITH_VALUE", "needed $VAL|2$"),
        ("SUPPLY_DIVISION_DEMAND", "divisions $AMOUNT$ $VAL|2$"),
        ("SUPPLY_AIRPLANE_DEMAND", "planes $AMOUNT$ $VAL|2$"),
        ("SUPPLY_SHIP_DEMAND", "ships $AMOUNT$ $VAL|2$"),
        ("RECEIVED_SUPPLY_WITH_VALUE", "received $VAL|2$ $RATIO$"),
        ("FROM_STATE", "state"),
        ("FROM_SUPPLY_NODES", "nodes"),
        ("FROM_AIR_DROPS", "air"),
        ("CONSUMER_SUPPLY_TOOLTIP", "$LABEL$ $VAL|2$"),
        ("SUPPLY_FLOW_REDUCTION", "reduced $PERCENT$"),
        ("SUPPLY_FLOW_DISRUPTION", "disruption $SUPPLY|2$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

fn with_context<R>(run: impl FnOnce(&Context<'_>) -> R) -> R {
    let loc = loc();
    let frame = DecodedFrame {
        branch: 0,
        hour: 0,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    with_frame_context(&frame, &loc, run)
}

fn with_frame_context<R>(
    frame: &DecodedFrame,
    loc: &Localisation,
    run: impl FnOnce(&Context<'_>) -> R,
) -> R {
    super::super::task_force::tests::with_test_context(frame, loc, "unused", None, |context| {
        run(&context)
    })
}

#[test]
fn supply_origin_names_use_exact_static_and_reserved_original_tags_and_keep_refusals() {
    use replay_data::{
        rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::ProvinceRow,
        rows::Recorded, rows::ReservedCountryNamingRow, rows::StateRow,
    };
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 0,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 100,
            canonical_slot: 1,
            tag: b"OWN".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 100,
            canonical_slot: 1,
        }]));
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 100,
            original_tag_reference: Recorded::Value(Some(1)),
            original_tag: Recorded::Value(Some(b"STATIC".to_vec())),
            ..Default::default()
        }]));
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 5,
            state: 64,
            ..Default::default()
        }]));
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 64,
            owner: 100,
            ..Default::default()
        }]));
    let loc = [
        ("STATIC_VICTORY_POINTS_5", "Static original town"),
        ("ZERO_ORIGINAL_VICTORY_POINTS_5", "Reserved original town"),
        ("VICTORY_POINTS_5", "Generic town"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    assert_eq!(
        with_frame_context(&frame, &loc, |context| province_name(context, 5)).as_deref(),
        Ok("Static original town")
    );
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag = Recorded::NotRecorded;
    }
    assert!(
        with_frame_context(&frame, &loc, |context| province_name(context, 5))
            .unwrap_err()
            .contains("original tag of country appearance not recorded")
    );
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *frame.tables[TableId::States.index()]
    {
        rows[0].owner = 0;
    }
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZERO".to_vec(),
                original_tag_reference: Recorded::Value(Some(1)),
                original_tag: Recorded::Value(Some(b"ZERO_ORIGINAL".to_vec())),
                ..Default::default()
            },
        ]));
    assert_eq!(
        with_frame_context(&frame, &loc, |context| province_name(context, 5)).as_deref(),
        Ok("Reserved original town")
    );
    if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
        &mut *frame.tables[TableId::ReservedCountryNaming.index()]
    {
        rows[0].original_tag = Recorded::Value(None);
    }
    assert!(
        with_frame_context(&frame, &loc, |context| province_name(context, 5))
            .unwrap_err()
            .contains("original tag of reserved country naming capture refused")
    );
}

fn render(data: &Aggregate, name: impl FnMut(i32) -> Result<String, String>) -> Tooltip {
    with_context(|context| {
        let mut out = Tooltip::default();
        footer(context, data, &mut out, name);
        out
    })
}

fn division_frame(footers: [DivisionSupplyFooter; 2]) -> DecodedFrame {
    let mut frame = test_frame(0);
    put(
        &mut frame,
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 7,
            canonical_slot: 1,
        }]),
    );
    put(
        &mut frame,
        TableRows::Divisions(
            [1, 2]
                .into_iter()
                .map(|id| DivisionRow {
                    id,
                    roster_owner: 7,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(
            footers
                .into_iter()
                .enumerate()
                .map(|(index, supply)| DivisionStatusRow {
                    division: index as u64 + 1,
                    supply: Recorded::Value(Some(supply)),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    frame
}

fn append_footer(frame: &DecodedFrame, railway_gun: bool, members: &[u64], body: &str) -> Tooltip {
    with_frame_context(frame, &loc(), |context| {
        let input = CounterHover {
            control: CounterControl::Supply,
            authored: None,
            rect: [0.0; 4],
            remaining: None,
            ours: true,
            coarse: false,
            hidden: false,
            railway_gun,
            members: members.to_vec().into(),
            timing: Arc::from([]),
        };
        let mut out = Tooltip {
            title: "header".into(),
            body: body.into(),
            ..Default::default()
        };
        super::super::children::append(context, &input, &mut out);
        out
    })
}

#[test]
fn ordinary_supply_footer_uses_title_and_skips_dangling_refs_before_last_disruption() {
    use replay_data::rows::RailwayGunCounterInputRow;
    let first = UnitSupplyInputs {
        asked: 100_000,
        received: 50_000,
        disruption: 90_000,
        origins: Some(vec![]),
        ..Default::default()
    };
    let last = UnitSupplyInputs {
        asked: 200_000,
        received: 150_000,
        disruption: 2_000,
        origins: Some(vec![]),
        ..Default::default()
    };
    let mut frame = division_frame([first.clone(), last.clone()].map(DivisionSupplyFooter::from));
    put(
        &mut frame,
        TableRows::RailwayGunCounterInputs(
            [first, last]
                .into_iter()
                .enumerate()
                .map(|(index, supply)| RailwayGunCounterInputRow {
                    gun: index as u64 + 1,
                    roster_owner: 7,
                    supply: Recorded::Value(Some(supply)),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let dangling = (1_u64 << 32) | 1;
    let build = |frame: &DecodedFrame, railway_gun, members: &[u64]| {
        append_footer(frame, railway_gun, members, "arrival 2h\n")
    };
    for railway_gun in [false, true] {
        let baseline = build(&frame, railway_gun, &[1, 2]);
        assert_eq!(baseline.body, "arrival 2h\n");
        assert!(
            baseline
                .title
                .starts_with("header\n\nneeded 3.00\n\nreceived 2.00"),
            "{railway_gun} {:?}",
            baseline.title
        );
        assert!(baseline.title.ends_with("  disruption 0.02\n"));
        assert!(baseline.refused.is_empty(), "{:?}", baseline.refused);
        for members in [&[1, dangling, 2][..], &[1, 2, dangling][..]] {
            assert_eq!(build(&frame, railway_gun, members), baseline);
        }
        assert!(
            build(&frame, railway_gun, &[1, dangling, 2, 2])
                .title
                .contains("needed 5.00")
        );
        let mut unreadable = frame.clone();
        if let TableView::Complete(TableRows::DivisionStatus(rows)) =
            &mut *unreadable.tables[TableId::DivisionStatus.index()]
        {
            rows[0].supply = Recorded::Value(None);
        }
        if let TableView::Complete(TableRows::RailwayGunCounterInputs(rows)) =
            &mut *unreadable.tables[TableId::RailwayGunCounterInputs.index()]
        {
            rows[0].supply = Recorded::Value(None);
        }
        let partial = build(&unreadable, railway_gun, &[1, dangling, 2, dangling]);
        assert_eq!(partial.title, "header\n\n  disruption 0.02\n");
        assert_eq!(partial.body, "arrival 2h\n");
        assert_eq!(partial.refused.values().sum::<usize>(), 1);
        let table = if railway_gun {
            TableId::RailwayGunCounterInputs
        } else {
            TableId::Divisions
        };
        *unreadable.tables[table.index()] = TableView::Unavailable {
            reason: "fixture incomplete unit census".into(),
        };
        let unknown = build(&unreadable, railway_gun, &[1, dangling, 2]);
        assert_eq!(unknown.title, "header");
        assert_eq!(unknown.body, "arrival 2h\n");
        assert_eq!(unknown.refused.values().sum::<usize>(), 1);
    }
    let mut older = frame.clone();
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *older.tables[TableId::DivisionStatus.index()]
    {
        for row in rows {
            row.supply = Recorded::NotRecorded;
        }
    }
    let refused = build(&older, false, &[1, 2]);
    assert_eq!(refused.title, "header");
    assert_eq!(refused.body, "arrival 2h\n");
    assert_eq!(refused.refused.values().sum::<usize>(), 1);
    assert!(
        refused
            .refused
            .keys()
            .any(|reason| reason.contains("consumer supply inputs were not recorded")),
        "{:?}",
        refused.refused
    );
}

#[test]
fn a_division_footer_sums_its_recorded_hundredths() {
    let footers = [
        DivisionSupplyFooter {
            asked: 123_000,
            received: 100_000,
            from_state: 40_000,
            division_demand: UnitSupplyDemand {
                count: 1,
                value: 123_000,
            },
            origins: Some(vec![UnitSupplyOrigin {
                province: 0,
                requested: 60_000,
                received: 60_000,
            }]),
            ..Default::default()
        },
        DivisionSupplyFooter {
            asked: 1_000,
            received: 1_000,
            division_demand: UnitSupplyDemand {
                count: 1,
                value: 1_000,
            },
            origins: Some(vec![]),
            ..Default::default()
        },
    ];
    let mut frame = division_frame(footers);
    put(&mut frame, TableRows::Provinces(Vec::new()));
    let out = append_footer(&frame, false, &[1, 2], "");
    assert!(out.refused.is_empty(), "{:?}", out.refused);
    assert!(
        out.title.starts_with("header\n\nneeded 1.24\n"),
        "{:?}",
        out.title
    );
    assert!(out.title.contains("  divisions 2 1.24"), "{:?}", out.title);
    assert!(out.title.contains("received 1.01"), "{:?}", out.title);
    assert!(out.title.contains("  state 0.40\n"), "{:?}", out.title);
    assert!(out.title.contains("  nodes 0.60\n"), "{:?}", out.title);
}

#[test]
fn unreadable_earlier_supply_pack_keeps_only_the_known_last_member_disruption() {
    let last = UnitSupplyInputs {
        disruption: 1_501,
        ..Default::default()
    };
    let out = with_context(|context| {
        let mut out = Tooltip::default();
        member_footer(
            context,
            &[Err("first member supply unreadable".into()), Ok(&last)],
            &mut out,
        );
        out
    });
    assert_eq!(out.body, "  disruption 0.02\n");
    assert_eq!(out.refused.values().sum::<usize>(), 1);
    let out = with_context(|context| {
        let mut out = Tooltip::default();
        member_footer(
            context,
            &[Ok(&last), Err("last member supply unreadable".into())],
            &mut out,
        );
        out
    });
    assert!(out.body.is_empty());
    assert_eq!(out.refused.values().sum::<usize>(), 1);
}

#[test]
fn supply_footer_preserves_native_member_order_demands_rounding_and_disruption_overwrite() {
    let first = UnitSupplyInputs {
        asked: 200_499,
        received: 99_501,
        from_state: 10_500,
        air_drops: 20_499,
        disruption: 90_000,
        division_demand: UnitSupplyDemand {
            count: 3,
            value: 120_000,
        },
        airplane_demand: UnitSupplyDemand {
            count: 7,
            value: 10_001,
        },
        ship_demand: UnitSupplyDemand {
            count: 2,
            value: 30_000,
        },
        origins: Some(vec![
            UnitSupplyOrigin {
                province: 9,
                requested: 40_000,
                received: 20_000,
            },
            UnitSupplyOrigin {
                province: 5,
                requested: 30_000,
                received: 20_000,
            },
            UnitSupplyOrigin {
                province: 9,
                requested: 10_000,
                received: -10_000,
            },
        ]),
    };
    let second = UnitSupplyInputs {
        asked: 99_501,
        received: 50_499,
        disruption: 1_501,
        origins: Some(vec![UnitSupplyOrigin {
            province: 3,
            requested: 30_000,
            received: 20_000,
        }]),
        ..Default::default()
    };
    let data = aggregate([&first, &second]);
    assert_eq!(data.scalars.disruption, 1_501);
    let out = render(&data, |id| Ok(format!("province {id}")));
    assert_eq!(
        out.body,
        "needed 3.00\n  divisions 3 1.20\n  planes 7 0.10\n  ships 2 0.30\n\nreceived 1.50 \u{11}R50%\u{11}!\n  state 0.11\n  nodes 0.50\n    province 9 0.20\n    province 5 0.20\n    province 3 0.20\n    reduced 55%\n  air 0.20\n  disruption 0.02\n"
    );
    assert!(out.refused.is_empty(), "{:?}", out.refused);
}

#[test]
fn supply_origin_failure_and_bad_name_preserve_the_other_scalar_and_node_fragments() {
    let mut row = UnitSupplyInputs {
        asked: 100_000,
        received: 100_000,
        from_state: 1_000,
        air_drops: 2_000,
        disruption: 3_000,
        origins: None,
        ..Default::default()
    };
    let out = render(&aggregate([&row]), |_| {
        panic!("unknown origin rows must not resolve names")
    });
    assert!(out.body.contains(
        "received 1.00 \u{11}W100%\u{11}!\n  state 0.01\n  air 0.02\n  disruption 0.03\n"
    ));
    assert_eq!(out.refused.values().sum::<usize>(), 1);
    row.origins = Some(vec![
        UnitSupplyOrigin {
            province: 7,
            requested: 20_000,
            received: 10_000,
        },
        UnitSupplyOrigin {
            province: 8,
            requested: 20_000,
            received: 10_000,
        },
    ]);
    let out = render(&aggregate([&row]), |id| {
        if id == 7 {
            Err("missing origin name".into())
        } else {
            Ok("known".into())
        }
    });
    assert!(
        out.body
            .contains("nodes 0.20\n    known 0.10\n    reduced 50%\n  air")
    );
    assert_eq!(out.refused.values().sum::<usize>(), 1);
}

#[test]
fn supply_origin_list_is_stable_merged_and_bounded_by_native_text_rules() {
    let row = UnitSupplyInputs {
        origins: Some(
            (0..8)
                .map(|id| UnitSupplyOrigin {
                    province: id,
                    requested: 100_000,
                    received: if id == 7 { 4_999 } else { 10_000 },
                })
                .collect(),
        ),
        ..Default::default()
    };
    let out = render(&aggregate([&row]), |id| Ok(format!("origin{id}")));
    assert!(out.body.contains("origin0"));
    assert!(out.body.contains("origin5"));
    assert!(!out.body.contains("origin6"));
    assert!(!out.body.contains("origin7"));
    for (value, want) in [
        (0, 0),
        (499, 0),
        (500, 1_000),
        (-499, 0),
        (-500, -1_000),
        (12_499, 12_000),
        (12_500, 13_000),
        (-12_500, -13_000),
    ] {
        assert_eq!(round_two(value), want);
    }
    assert_eq!(ratio(1, 0).unwrap(), 100_000);
    assert_eq!(ratio(i64::MIN, 1).unwrap(), 0xffff_ffff);
}
