use super::*;
use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::container::Replay;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow, rows::ProductionLineOutputRow,
    rows::ProductionLineRow, rows::Recorded, rows::RocketProductionOutputRow, rows::TableId,
    rows::TableRows, rows::VariantDefinitionRow,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

pub(super) fn hour(day: i32) -> i32 {
    let replay_data::cadence::Cadence::Daily(slot) =
        replay_data::cadence::cadence(TableId::ProductionLineOutput)
    else {
        panic!("output is daily");
    };
    day * replay_data::cadence::STAGGER_PERIOD + i32::try_from(slot).unwrap()
}

pub(super) fn line(handle: u64, variant: u64, cost_raw: i64) -> ProductionLineRow {
    ProductionLineRow {
        handle,
        country: 1,
        variant,
        cost_raw,
        speed_raw: i64::MAX,
        ..Default::default()
    }
}

fn output(line: u64, variant: u64, units: i64) -> ProductionLineOutputRow {
    applied(line, variant, units, units * 100_000)
}

pub(super) fn applied(
    line: u64,
    variant: u64,
    units: i64,
    spent_raw: i64,
) -> ProductionLineOutputRow {
    ProductionLineOutputRow {
        line,
        variant,
        country: 1,
        units,
        since_hour: hour(0),
        exact: true,
        spent_raw: Recorded::Value(Some(spent_raw)),
        ..Default::default()
    }
}

pub(super) fn day(
    day: i32,
    lines: Vec<ProductionLineRow>,
    outputs: Option<Vec<ProductionLineOutputRow>>,
) -> Vec<DecodedFrame> {
    (0..24)
        .map(|offset| {
            let mut frame = source(1, hour(day) + offset, Some(1));
            *frame.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(vec![CountryRow {
                    id: 1,
                    canonical_slot: 1,
                    tag: b"opaque|tag".to_vec(),
                    ..Default::default()
                }]));
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                    id: 1,
                    canonical_slot: 1,
                }]));
            *frame.tables[TableId::ProductionLines.index()] =
                TableView::Complete(TableRows::ProductionLines(lines.clone()));
            *frame.tables[TableId::ProductionLineOutput.index()] = match &outputs {
                Some(outputs) => {
                    TableView::Complete(TableRows::ProductionLineOutput(outputs.clone()))
                }
                None => TableView::Unavailable {
                    reason: "synthetic missed publication".into(),
                },
            };
            *frame.tables[TableId::RocketProductionOutput.index()] =
                TableView::Complete(TableRows::RocketProductionOutput(vec![
                    RocketProductionOutputRow {
                        since_hour: hour(0),
                        exact: true,
                        units_exact: true,
                        ..Default::default()
                    },
                ]));
            let tokens = ["mod_rifle", "mod_plane", "mod_ship", "mod_rail"];
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(
                    tokens
                        .iter()
                        .enumerate()
                        .map(|(index, token)| LexicalTokenRow {
                            token: (index + 1) as u32,
                            literal: token.as_bytes().to_vec(),
                        })
                        .collect(),
                ));
            *frame.tables[TableId::VariantDefinitions.index()] =
                TableView::Complete(TableRows::VariantDefinitions(
                    (1..=4)
                        .map(|id| VariantDefinitionRow {
                            id: u64::from(id),
                            equipment_token: id,
                            creator_link: 1,
                            display_name: format!("Design {id} day {day}").into_bytes(),
                            ..Default::default()
                        })
                        .collect(),
                ));
            if offset == 0 {
                frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
                frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
            }
            frame
        })
        .collect()
}

fn scan(frames: Vec<DecodedFrame>) -> ScannedProduced {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    production::scan(&replay, 1, &AtomicBool::new(false)).unwrap()
}

fn units() -> installed_content_core::equipment::Database {
    installed_content_core::equipment::fixture::load(&[(
        "common/units/equipment/mod.txt",
        "equipments = {
        mod_land = { is_archetype = yes type = infantry }
        mod_rifle = { archetype = mod_land }
        mod_air = { is_archetype = yes type = fighter }
        mod_plane = { archetype = mod_air }
        mod_naval = { is_archetype = yes type = screen_ship }
        mod_ship = { archetype = mod_naval }
        mod_rail_type = { is_archetype = yes type = railway_gun }
        mod_rail = { archetype = mod_rail_type }
    }",
    )])
    .0
}

fn params(refits: bool) -> ProductionSpendingParams {
    ProductionSpendingParams {
        branch_id: "1".into(),
        tag: "opaque|tag".into(),
        refits,
    }
}
fn render_result(scan: &ScannedProduced, refits: bool) -> ProductionSpendingReply {
    let catalogue = crate::catalogue::tests::loaded("l_english:\n");
    reply(
        scan,
        &params(refits),
        Named {
            units: Some(&units()),
            texts: Some(&catalogue),
        },
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn production_spending_empty_evidence_needs_a_complete_census_and_independent_rocket_read() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for missing in [TableId::ProductionLines, TableId::RocketProductionOutput] {
        let mut frames = day(0, Vec::new(), Some(Vec::new()));
        for frame in &mut frames {
            *frame.tables[missing.index()] = TableView::Unavailable {
                reason: "unread source".into(),
            };
        }
        let result = render_result(&scan(frames), false);
        assert!(
            result
                .production
                .iter()
                .all(|run| run.empty_census_hour.is_none())
        );
    }
    let mut frames = day(0, vec![line(1, 1, 1)], Some(vec![applied(1, 1, 0, 0)]));
    let mut ended = applied(1, 1, 0, 0);
    ended.ended = true;
    frames.extend(day(1, Vec::new(), Some(vec![ended])));
    frames.extend(day(2, Vec::new(), Some(Vec::new())));
    let result = render_result(&scan(frames), false);
    assert!(
        result
            .total
            .iter()
            .all(|run| run.total_raw.as_deref() == Some("0"))
    );
    assert!(
        result
            .production
            .iter()
            .all(|run| run.present == Some(true) && run.empty_census_hour.is_none())
    );
}
fn at(runs: &[ProductionSpendingRun], hour: i32) -> Option<&ProductionSpendingRun> {
    runs.iter()
        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
}

fn rockets(frames: &mut [DecodedFrame], spent: i64, units: i64, exact: bool) {
    for frame in frames {
        *frame.tables[TableId::RocketProductionOutput.index()] =
            TableView::Complete(TableRows::RocketProductionOutput(vec![
                RocketProductionOutputRow {
                    since_hour: hour(0),
                    exact: true,
                    units_exact: true,
                    ticks: 1,
                    deliveries: 1,
                    inexact_ticks: u64::from(!exact),
                    ..Default::default()
                },
                RocketProductionOutputRow {
                    country: 1,
                    since_hour: hour(0),
                    exact,
                    units_exact: true,
                    ticks: 1,
                    deliveries: 1,
                    inexact_ticks: u64::from(!exact),
                    ..Default::default()
                },
                RocketProductionOutputRow {
                    country: 1,
                    variant: 1,
                    since_hour: hour(0),
                    spent_raw: spent,
                    instant_raw: 9_000_000,
                    units,
                    exact,
                    units_exact: true,
                    ticks: 1,
                    deliveries: 1,
                    inexact_ticks: u64::from(!exact),
                    ..Default::default()
                },
            ]));
    }
}

fn rocket_counts(frames: &mut [DecodedFrame], ticks: u64, deliveries: u64) {
    for frame in frames {
        if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
            &mut *frame.tables[TableId::RocketProductionOutput.index()]
        {
            for row in rows {
                row.ticks = ticks;
                row.deliveries = deliveries;
            }
        }
    }
}

#[test]
fn production_spending_missing_rocket_coverage_keeps_equipment_leaf_but_refuses_combined_zero() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(
        0,
        vec![line(1, 1, 1)],
        Some(vec![applied(1, 1, 0, 123_456)]),
    );
    for frame in &mut frames {
        *frame.tables[TableId::RocketProductionOutput.index()] = TableView::Unavailable {
            reason: "rocket publication refused".into(),
        };
    }
    let result = render_result(&scan(frames), false);
    assert_eq!(
        result.leaves[0].runs[0].total_raw.as_deref(),
        Some("123456")
    );
    assert_eq!(
        result.total[0].reason,
        Some(ProductionSpendingUnknown::RocketTableUnavailable)
    );
    assert!(result.total[0].total_raw.is_none());
}

#[test]
fn production_spending_rocket_signed_totals_survive_removal_and_share_no_design_leaf() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(
        0,
        vec![line(1, 1, 1)],
        Some(vec![applied(1, 1, 0, 200_000)]),
    );
    rockets(&mut frames, 100_001, 2, true);
    let mut next = day(
        1,
        vec![],
        Some(vec![{
            let mut row = applied(1, 1, 0, 200_000);
            row.ended = true;
            row
        }]),
    );
    rockets(&mut next, -300_003, -1, true);
    rocket_counts(&mut next, 2, 2);
    frames.extend(next);
    let result = render_result(&scan(frames), false);
    assert_eq!(result.leaves.len(), 2);
    assert_eq!(
        result
            .leaves
            .iter()
            .map(|leaf| (leaf.variant.as_str(), leaf.rocket))
            .collect::<Vec<_>>(),
        [("1", false), ("1", true)]
    );
    assert_eq!(
        at(&result.rocket_total, hour(1))
            .unwrap()
            .total_raw
            .as_deref(),
        Some("-300003")
    );
    assert_eq!(
        at(&result.total, hour(1) + 23)
            .unwrap()
            .total_raw
            .as_deref(),
        Some("-100003")
    );
}

#[test]
fn production_rocket_retained_spending_refusals_preserve_deliveries_and_local_scope() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // Changed spending/instant amounts without a tick, and regressed design,
    // country and world tick counts are independently valid row encodings.
    for (scope, mode) in [(2, 0), (2, 1), (2, 2), (1, 2), (0, 2)] {
        let mut frames = day(0, vec![line(1, 1, 7)], Some(vec![applied(1, 1, 0, 111)]));
        rockets(&mut frames, 17, 3, true);
        rocket_counts(&mut frames, 2, 2);
        let mut next = day(1, vec![line(1, 1, 7)], Some(vec![applied(1, 1, 0, 111)]));
        rockets(&mut next, 17, -7, true);
        rocket_counts(&mut next, 2, 3);
        for frame in &mut next {
            if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
                &mut *frame.tables[TableId::RocketProductionOutput.index()]
            {
                match mode {
                    0 => rows[scope].spent_raw = -41,
                    1 => rows[scope].instant_raw += 1,
                    _ => rows[scope].ticks = 1,
                }
            }
        }
        frames.extend(next);
        let mut later = day(2, vec![line(1, 1, 7)], Some(vec![applied(1, 1, 0, 111)]));
        rockets(&mut later, -41, -7, true);
        rocket_counts(&mut later, 3, 4);
        frames.extend(later);
        for frame in &mut frames {
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows.push(CountryRow {
                    id: 2,
                    canonical_slot: 2,
                    tag: b"unaffected".to_vec(),
                    ..Default::default()
                });
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows.push(CountryLinkRow {
                    id: 2,
                    canonical_slot: 2,
                });
            }
        }
        let scanned = scan(frames);
        assert_eq!(scanned.rocket.inconsistent, 1);
        let result = render_result(&scanned, false);
        for day in [1, 2] {
            assert_eq!(
                at(&result.rocket_total, hour(day)).unwrap().reason,
                Some(ProductionSpendingUnknown::RocketInputInexact)
            );
            assert_eq!(
                at(
                    &result.leaves.iter().find(|leaf| !leaf.rocket).unwrap().runs,
                    hour(day)
                )
                .unwrap()
                .total_raw
                .as_deref(),
                Some("111")
            );
        }
        let totals = crate::products::equipment_produced::country_totals(
            &scanned,
            &["opaque|tag".into()],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            totals["opaque|tag"]
                .iter()
                .map(|run| run.amount)
                .collect::<Vec<_>>(),
            [Some(3), Some(-7), Some(-7)]
        );
        let unaffected = reply(
            &scanned,
            &ProductionSpendingParams {
                tag: "unaffected".into(),
                ..params(false)
            },
            Named::default(),
            Vec::new(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            at(&unaffected.total, hour(1)).unwrap().total_raw.as_deref(),
            (scope != 0).then_some("0")
        );
    }
}

#[test]
fn production_rocket_retained_delivery_refusals_preserve_spending_and_stay_unknown() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (scope, changed_amount) in [(2, true), (2, false), (1, false), (0, false)] {
        let mut frames = day(0, vec![line(1, 1, 7)], Some(vec![applied(1, 1, 0, 111)]));
        rockets(&mut frames, 17, 3, true);
        rocket_counts(&mut frames, 2, 2);
        let mut next = day(1, vec![line(1, 1, 7)], Some(vec![applied(1, 1, 0, 111)]));
        rockets(&mut next, -41, 3, true);
        rocket_counts(&mut next, 3, 2);
        for frame in &mut next {
            if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
                &mut *frame.tables[TableId::RocketProductionOutput.index()]
            {
                if changed_amount {
                    rows[scope].units = -7;
                } else {
                    rows[scope].deliveries = 1;
                }
            }
        }
        frames.extend(next);
        let mut later = day(2, vec![line(1, 1, 7)], Some(vec![applied(1, 1, 0, 111)]));
        rockets(&mut later, -41, -7, true);
        rocket_counts(&mut later, 4, 3);
        frames.extend(later);
        let scanned = scan(frames);
        assert_eq!(scanned.rocket.inconsistent, 1);
        let result = render_result(&scanned, false);
        for day in [1, 2] {
            assert_eq!(
                at(&result.rocket_total, hour(day))
                    .unwrap()
                    .total_raw
                    .as_deref(),
                Some("-41")
            );
            assert_eq!(
                at(&result.total, hour(day)).unwrap().total_raw.as_deref(),
                Some("70")
            );
        }
        let totals = crate::products::equipment_produced::country_totals(
            &scanned,
            &["opaque|tag".into()],
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            totals["opaque|tag"]
                .iter()
                .map(|run| run.amount)
                .collect::<Vec<_>>(),
            [Some(3), None, None]
        );
    }
}

#[test]
fn production_rocket_partial_amount_changes_do_not_invalidate_the_other_measure() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for spending_partial in [true, false] {
        let mut frames = day(0, vec![line(1, 1, 7)], Some(vec![applied(1, 1, 0, 111)]));
        rockets(&mut frames, 17, 3, true);
        let mut next = day(1, vec![line(1, 1, 7)], Some(vec![applied(1, 1, 0, 111)]));
        rockets(
            &mut next,
            if spending_partial { -41 } else { 17 },
            if spending_partial { 3 } else { -7 },
            true,
        );
        frames.extend(next);
        for frame in &mut frames {
            if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
                &mut *frame.tables[TableId::RocketProductionOutput.index()]
            {
                for row in rows.iter_mut() {
                    if spending_partial {
                        row.inexact_ticks = 1;
                    } else {
                        row.inexact_deliveries = 1;
                    }
                    if row.country != 0 {
                        if spending_partial {
                            row.exact = false;
                        } else {
                            row.units_exact = false;
                        }
                    }
                }
                if spending_partial && frame.hour >= hour(1) {
                    rows[2].instant_raw += 1;
                }
            }
        }
        let scanned = scan(frames);
        assert_eq!(scanned.rocket.inconsistent, 0);
        if spending_partial {
            let totals = crate::products::equipment_produced::country_totals(
                &scanned,
                &["opaque|tag".into()],
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(
                totals["opaque|tag"]
                    .iter()
                    .map(|run| run.amount)
                    .collect::<Vec<_>>(),
                [Some(3), Some(3)]
            );
        } else {
            assert_eq!(
                at(&render_result(&scanned, false).rocket_total, hour(1))
                    .unwrap()
                    .total_raw
                    .as_deref(),
                Some("17")
            );
        }
    }
}

#[test]
fn production_spending_rocket_is_independent_of_equipment_census_and_delivery_exactness() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(0, vec![], None);
    rockets(&mut frames, 456_789, 0, true);
    for frame in &mut frames {
        *frame.tables[TableId::ProductionLines.index()] = TableView::Unavailable {
            reason: "census unread".into(),
        };
        if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
            &mut *frame.tables[TableId::RocketProductionOutput.index()]
        {
            rows[1].units_exact = false;
            rows[2].units_exact = false;
            rows[0].inexact_deliveries = 1;
            rows[1].inexact_deliveries = 1;
            rows[2].inexact_deliveries = 1;
        }
    }
    let result = render_result(&scan(frames), false);
    assert!(result.leaves[0].rocket);
    assert_eq!(
        result.leaves[0].runs[0].total_raw.as_deref(),
        Some("456789")
    );
    assert_eq!(result.rocket_total[0].total_raw.as_deref(), Some("456789"));
    assert!(result.total[0].total_raw.is_none());
}

#[test]
fn production_spending_rocket_world_owner_failure_does_not_erase_known_design_amounts() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(0, vec![], Some(vec![]));
    rockets(&mut frames, 17, 2, true);
    for frame in &mut frames {
        if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
            &mut *frame.tables[TableId::RocketProductionOutput.index()]
        {
            rows[0].exact = false;
            rows[0].unknown_country_ticks = 1;
            rows[0].inexact_ticks = 1;
        }
    }
    let result = render_result(&scan(frames), false);
    assert_eq!(result.leaves[0].runs[0].total_raw.as_deref(), Some("17"));
    assert_eq!(
        result.rocket_total[0].reason,
        Some(ProductionSpendingUnknown::RocketOwnerUnreadable)
    );
    assert!(result.total[0].total_raw.is_none());
}

#[test]
fn production_spending_rocket_missing_scope_and_disappearing_retained_rows_are_not_zero() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(0, vec![], Some(vec![]));
    rockets(&mut frames, 21, 2, true);
    let mut next = day(1, vec![], Some(vec![]));
    rockets(&mut next, 21, 2, true);
    for frame in &mut next {
        if let TableView::Complete(TableRows::RocketProductionOutput(rows)) =
            &mut *frame.tables[TableId::RocketProductionOutput.index()]
        {
            rows.retain(|row| row.variant == 0);
        }
    }
    frames.extend(next);
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.rocket_total, hour(1)).unwrap().reason,
        Some(ProductionSpendingUnknown::LostOutput)
    );
    let mut frames = day(0, vec![], Some(vec![]));
    for frame in &mut frames {
        *frame.tables[TableId::RocketProductionOutput.index()] =
            TableView::Complete(TableRows::RocketProductionOutput(vec![]));
    }
    assert_eq!(
        render_result(&scan(frames), false).total[0].reason,
        Some(ProductionSpendingUnknown::RocketTableUnavailable)
    );
}

#[test]
fn production_spending_rocket_source_uses_content_classification_and_refuses_ambiguity() {
    use installed_content_core::{buildings::Buildings, buildings::FactoryFlags};
    let mut buildings = Buildings {
        declaration_order: vec!["mod_launch_works".into()],
        factory_flags: BTreeMap::from([(
            "mod_launch_works".into(),
            FactoryFlags {
                rocket_production: 7,
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    assert_eq!(
        rocket_name(&buildings, None).unwrap(),
        ("mod_launch_works".into(), None)
    );
    buildings
        .ambiguous_factory_types
        .insert("mod_launch_works".into());
    assert!(
        rocket_name(&buildings, None)
            .unwrap_err()
            .contains("ambiguous")
    );
    buildings.ambiguous_factory_types.clear();
    buildings.declaration_order.push("mod_other_launch".into());
    buildings.factory_flags.insert(
        "mod_other_launch".into(),
        FactoryFlags {
            rocket_production: 1,
            ..Default::default()
        },
    );
    assert!(
        rocket_name(&buildings, None)
            .unwrap_err()
            .contains("2 content building definitions")
    );
}

#[test]
fn production_spending_known_country_rocket_failure_does_not_taint_another_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(0, vec![], Some(vec![]));
    for frame in &mut frames {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows.push(CountryRow {
                id: 2,
                canonical_slot: 2,
                tag: b"unaffected".to_vec(),
                ..Default::default()
            });
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 2,
                canonical_slot: 2,
            });
        }
        *frame.tables[TableId::RocketProductionOutput.index()] =
            TableView::Complete(TableRows::RocketProductionOutput(vec![
                RocketProductionOutputRow {
                    since_hour: hour(0),
                    exact: true,
                    units_exact: true,
                    ticks: 1,
                    inexact_ticks: 1,
                    unknown_design_ticks: 1,
                    ..Default::default()
                },
                RocketProductionOutputRow {
                    country: 1,
                    since_hour: hour(0),
                    exact: false,
                    units_exact: true,
                    ticks: 1,
                    inexact_ticks: 1,
                    unknown_design_ticks: 1,
                    ..Default::default()
                },
            ]));
    }
    let scanned = scan(frames);
    assert_eq!(
        render_result(&scanned, false).rocket_total[0].reason,
        Some(ProductionSpendingUnknown::RocketDesignUnreadable)
    );
    let result = reply(
        &scanned,
        &ProductionSpendingParams {
            tag: "unaffected".into(),
            ..params(false)
        },
        Named::default(),
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.total[0].total_raw.as_deref(), Some("0"));
    assert_eq!(result.production[0].present, Some(false));
}

#[test]
fn production_spending_protocol_zero_never_resolves_through_a_native_country_link() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(0, vec![], Some(vec![]));
    for frame in &mut frames {
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.insert(
                0,
                CountryLinkRow {
                    id: 0,
                    canonical_slot: 1,
                },
            );
        }
        *frame.tables[TableId::RocketProductionOutput.index()] =
            TableView::Complete(TableRows::RocketProductionOutput(vec![
                RocketProductionOutputRow {
                    since_hour: hour(0),
                    exact: false,
                    units_exact: true,
                    ticks: 1,
                    inexact_ticks: 1,
                    unknown_country_ticks: 1,
                    ..Default::default()
                },
                RocketProductionOutputRow {
                    variant: 1,
                    since_hour: hour(0),
                    exact: false,
                    units_exact: true,
                    spent_raw: 31,
                    ticks: 1,
                    inexact_ticks: 1,
                    unknown_country_ticks: 1,
                    ..Default::default()
                },
            ]));
    }
    let result = render_result(&scan(frames), false);
    assert!(result.leaves.is_empty());
    assert_eq!(
        result.total[0].reason,
        Some(ProductionSpendingUnknown::RocketOwnerUnreadable)
    );
    assert_eq!(result.production[0].present, None);
}

#[test]
fn production_spending_uses_applied_inputs_without_delivery_or_census_repricing() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(
        0,
        vec![line(1, 1, 100_001)],
        Some(vec![applied(1, 1, 0, 200_002)]),
    );
    frames.extend(day(
        1,
        vec![line(1, 1, 250_003)],
        Some(vec![applied(1, 1, 0, 450_005)]),
    ));
    frames.extend(day(
        2,
        vec![line(1, 1, 900_000)],
        Some(vec![applied(1, 1, 3, 450_005)]),
    ));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(0)).unwrap().total_raw.as_deref(),
        Some("200002")
    );
    assert_eq!(
        at(&result.total, hour(1)).unwrap().total_raw.as_deref(),
        Some("450005")
    );
    assert_eq!(
        at(&result.total, hour(2) + 23)
            .unwrap()
            .total_raw
            .as_deref(),
        Some("450005")
    );
    assert_eq!(result.decimals, 5);
    assert_eq!(result.leaves[0].type_key, "mod_land");
    assert!(
        result.leaves[0]
            .names
            .iter()
            .any(|name| name.first_hour == hour(1))
    );
}

#[test]
fn production_spending_gap_ends_known_and_observation_restart_cannot_recover() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(0, vec![line(1, 1, 100_000)], Some(vec![output(1, 1, 2)]));
    frames.remove(1);
    frames.extend(day(
        1,
        vec![line(1, 1, 100_000)],
        Some(vec![output(1, 1, 3)]),
    ));
    let mut restarted = output(1, 1, 1);
    restarted.since_hour = hour(2);
    frames.extend(day(2, vec![line(1, 1, 100_000)], Some(vec![restarted])));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(0)).unwrap().total_raw.as_deref(),
        Some("200000")
    );
    assert!(at(&result.total, hour(0) + 1).is_none());
    assert_eq!(
        at(&result.total, hour(1)).unwrap().reason,
        Some(ProductionSpendingUnknown::RecordingGap)
    );
    assert!(at(&result.total, hour(2)).unwrap().total_raw.is_none());
}

#[test]
fn production_spending_restart_without_gap_is_unknown_campaign_total() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(0, vec![line(1, 1, 100_000)], Some(vec![output(1, 1, 2)]));
    let mut restarted = output(1, 1, 1);
    restarted.since_hour = hour(1);
    frames.extend(day(1, vec![line(1, 1, 100_000)], Some(vec![restarted])));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(1)).unwrap().reason,
        Some(ProductionSpendingUnknown::ObservationRestarted)
    );
}

#[test]
fn production_spending_leading_and_trailing_explicit_gaps_are_unknown() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay::spool::Record;
    use replay_frame::branches::Gap;
    let mut frames = day(0, vec![line(1, 1, 100_000)], Some(vec![output(1, 1, 2)]));
    frames.drain(..4);
    let mut recorded = records(1, frames);
    let Record::BranchStart(start) = &mut recorded[1] else {
        panic!("branch start");
    };
    start.first_hour = hour(0);
    recorded.insert(
        2,
        Record::Gap(Gap {
            branch: 1,
            first_hour: hour(0),
            last_hour: hour(0) + 3,
            reason: "leading frame write lost".into(),
        }),
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[recorded]).unwrap();
    let scanned = production::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let result = render_result(&scanned, false);
    assert!(result.total.iter().all(|run| run.total_raw.is_none()));

    let mut recorded = records(
        1,
        day(0, vec![line(1, 1, 100_000)], Some(vec![output(1, 1, 2)])),
    );
    recorded.push(Record::Gap(Gap {
        branch: 1,
        first_hour: hour(1),
        last_hour: hour(1) + 3,
        reason: "trailing frames lost".into(),
    }));
    let replay = Replay::from_records(CAMPAIGN.into(), &[recorded]).unwrap();
    let scanned = production::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let result = render_result(&scanned, false);
    assert_eq!(
        at(&result.total, hour(0)).unwrap().total_raw.as_deref(),
        Some("200000")
    );
    assert!(at(&result.total, hour(1)).is_none());
    assert!(!scanned.spending_tainted.is_empty());
}

#[test]
fn production_spending_missing_publication_never_becomes_zero() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(0, vec![line(1, 1, 100_000)], Some(vec![output(1, 1, 2)]));
    frames.extend(day(1, vec![line(1, 1, 100_000)], None));
    frames.extend(day(
        2,
        vec![line(1, 1, 100_000)],
        Some(vec![output(1, 1, 3)]),
    ));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(1)).unwrap().reason,
        Some(ProductionSpendingUnknown::NotRecorded)
    );
    assert!(at(&result.total, hour(1)).unwrap().total_raw.is_none());
    assert_eq!(
        at(&result.total, hour(2)).unwrap().reason,
        Some(ProductionSpendingUnknown::MissedPublication)
    );
}

#[test]
fn production_spending_earlier_unread_precedes_a_later_explicit_gap() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay::spool::Record;
    use replay_frame::branches::Gap;
    let mut frames = day(0, vec![line(1, 1, 100_000)], Some(vec![output(1, 1, 2)]));
    frames.extend(day(1, vec![line(1, 1, 100_000)], None));
    frames.extend(day(
        2,
        vec![line(1, 1, 100_000)],
        Some(vec![output(1, 1, 3)]),
    ));
    let mut recorded = records(1, frames);
    recorded.push(Record::Gap(Gap {
        branch: 1,
        first_hour: hour(3),
        last_hour: hour(3) + 3,
        reason: "later explicit gap".into(),
    }));
    let replay = Replay::from_records(CAMPAIGN.into(), &[recorded]).unwrap();
    let scanned = production::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let result = render_result(&scanned, false);
    assert_eq!(
        at(&result.total, hour(2)).unwrap().reason,
        Some(ProductionSpendingUnknown::MissedPublication)
    );
}

#[test]
fn production_spending_deleted_line_keeps_its_final_applied_input_without_a_cost_census() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut frames = day(
        0,
        vec![line(1, 1, 200_000)],
        Some(vec![applied(1, 1, 2, 400_000)]),
    );
    let mut ended = applied(1, 1, 2, 400_000);
    ended.ended = true;
    frames.extend(day(1, vec![], Some(vec![ended.clone()])));
    frames.extend(day(2, vec![], Some(vec![])));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(2)).unwrap().total_raw.as_deref(),
        Some("400000")
    );
    let mut frames = day(
        0,
        vec![line(1, 1, 200_000)],
        Some(vec![applied(1, 1, 2, 400_000)]),
    );
    ended.units = 3;
    ended.spent_raw = Recorded::Value(Some(500_123));
    frames.extend(day(1, vec![], Some(vec![ended])));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(1)).unwrap().total_raw.as_deref(),
        Some("500123")
    );
}

#[test]
fn production_spending_unresolved_type_keeps_exact_token_leaf_and_total() {
    let scanned = scan(day(
        0,
        vec![line(1, 1, 100_000)],
        Some(vec![output(1, 1, 3)]),
    ));
    let result = reply(
        &scanned,
        &params(false),
        Named::default(),
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.leaves[0].type_key, ":mod_rifle");
    assert_eq!(result.leaves[0].domain, None);
    assert_eq!(result.total[0].total_raw.as_deref(), Some("300000"));
}

#[test]
fn production_spending_reports_invalid_variant_text_without_content() {
    let mut frames = day(0, vec![line(1, 1, 100_000)], Some(vec![output(1, 1, 3)]));
    for frame in &mut frames {
        if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
            &mut *frame.tables[TableId::VariantDefinitions.index()]
        {
            rows[0].display_name = vec![0xff];
        }
    }
    let scanned = scan(frames);
    let result = reply(
        &scanned,
        &params(false),
        Named::default(),
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.total[0].total_raw.as_deref(), Some("300000"));
    assert_eq!(result.leaves[0].variant, "1");
    assert!(
        result
            .warnings
            .iter()
            .any(|why| why.contains("equipment display name")
                && why.contains("not UTF-8")
                && why.ends_with("; 1"))
    );
}

#[test]
fn production_spending_mod_content_domains_and_refit_naval_railway_rules_are_shared() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut lines = vec![
        line(1, 1, 100_000),
        line(2, 2, 200_000),
        line(3, 3, 300_000),
        line(4, 4, 400_000),
        line(5, 3, 500_000),
    ];
    lines[2].kind = engine_native::cequipment_production_line::KIND_NAVAL;
    lines[3].kind = engine_native::cequipment_production_line::KIND_RAILWAY_GUN;
    lines[4].kind = engine_native::cequipment_production_line::KIND_SHIP_REFIT;
    let outputs = lines
        .iter()
        .map(|line| applied(line.handle, line.variant, 1, line.cost_raw))
        .collect();
    let scanned = scan(day(0, lines, Some(outputs)));
    let result = render_result(&scanned, false);
    assert_eq!(
        result
            .leaves
            .iter()
            .map(|leaf| (&*leaf.type_key, leaf.domain))
            .collect::<Vec<_>>(),
        [
            ("mod_land", Some(Domain::Land)),
            ("mod_air", Some(Domain::Air)),
            ("mod_naval", Some(Domain::Naval)),
            ("mod_rail_type", Some(Domain::Land))
        ]
    );
    assert_eq!(result.total[0].total_raw.as_deref(), Some("1000000"));
    assert_eq!(
        render_result(&scanned, true).total[0].total_raw.as_deref(),
        Some("1500000")
    );
}

#[test]
fn production_spending_is_independent_of_later_census_design_mio_and_invalid_cost() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (variant, mio, cost) in [(2, 0, 100_000), (1, 99, 0), (1, 0, -1)] {
        let mut census = line(1, variant, cost);
        census.mio = mio;
        let result = render_result(
            &scan(day(0, vec![census], Some(vec![output(1, 1, 1)]))),
            false,
        );
        assert_eq!(result.total[0].total_raw.as_deref(), Some("100000"));
    }
}

#[test]
fn production_spending_widens_independent_native_totals_instead_of_multiplying_them_by_cost() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let lines = (1..=3).map(|id| line(id, 1, i64::MAX)).collect();
    let outputs = (1..=3)
        .map(|id| applied(id, 1, i64::MAX, i64::MAX))
        .collect();
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, day(0, lines, Some(outputs)))]).unwrap();
    let scanned = production::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        render_result(&scanned, false).total[0].total_raw,
        Some((i128::from(i64::MAX) * 3).to_string())
    );
}

#[test]
fn production_spending_signed_changes_are_independent_of_units() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let first = applied(1, 1, 2, i64::MAX);
    let mut last = applied(1, 1, 3, i64::MIN);
    last.exact = false;
    let mut frames = day(0, vec![line(1, 1, 7)], Some(vec![first]));
    frames.extend(day(1, vec![line(1, 1, -1)], Some(vec![last])));
    let scanned = scan(frames);
    assert_eq!(scanned.tags["opaque|tag"].current[&(1, false)].inexact, 1);
    let result = render_result(&scanned, false);
    assert_eq!(
        at(&result.total, hour(0)).unwrap().total_raw,
        Some(i64::MAX.to_string())
    );
    assert_eq!(
        at(&result.total, hour(1)).unwrap().total_raw,
        Some(i64::MIN.to_string())
    );
}

#[test]
fn production_spending_refusal_preserves_known_deliveries() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut row = output(1, 1, 3);
    row.spent_raw = Recorded::Value(None);
    let scanned = scan(day(0, vec![line(1, 1, 100_000)], Some(vec![row])));
    assert_eq!(scanned.tags["opaque|tag"].current[&(1, false)].units, 3);
    assert_eq!(scanned.tags["opaque|tag"].current[&(1, false)].inexact, 0);
    assert_eq!(
        render_result(&scanned, false).total[0].reason,
        Some(ProductionSpendingUnknown::SpendingUnavailable)
    );
}

#[test]
fn production_spending_unknown_owner_with_zero_units_does_not_taint_known_units() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut row = applied(1, 1, 0, 123_456);
    row.country = 999;
    let scanned = scan(day(0, vec![], Some(vec![row])));
    assert!(scanned.tainted.is_empty());
    assert_eq!(
        render_result(&scanned, false).total[0].reason,
        Some(ProductionSpendingUnknown::SpendingUnavailable)
    );
}

#[test]
fn production_spending_lost_inexact_unit_row_still_withholds_its_independent_spending() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut row = applied(1, 1, 0, 123_456);
    row.exact = false;
    let mut frames = day(0, vec![line(1, 1, 1)], Some(vec![row]));
    frames.extend(day(1, vec![], Some(vec![])));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(0)).unwrap().total_raw.as_deref(),
        Some("123456")
    );
    assert_eq!(
        at(&result.total, hour(1)).unwrap().reason,
        Some(ProductionSpendingUnknown::LostOutput)
    );
}

#[test]
fn production_spending_every_country_5000_country_builder() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let country_count = 5_000u32;
    let countries: Vec<_> = (1..=country_count)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: format!("opaque|{id}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links: Vec<_> = (1..=country_count)
        .map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        })
        .collect();
    let mut frames = Vec::new();
    for index in 0..2 {
        let lines: Vec<_> = (1..=country_count)
            .map(|id| ProductionLineRow {
                country: id,
                ..line(u64::from(id), 1, 100_000)
            })
            .collect();
        let outputs: Vec<_> = (1..=country_count)
            .map(|id| ProductionLineOutputRow {
                country: id,
                ..output(u64::from(id), 1, i64::from(index + 1))
            })
            .collect();
        for mut frame in day(index, lines, Some(outputs)) {
            *frame.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(countries.clone()));
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(links.clone()));
            frames.push(frame);
        }
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = production::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.tags.len(), country_count as usize);
    let mut selected = params(false);
    selected.tag = "opaque|5000".into();
    let result = reply(
        &scanned,
        &selected,
        Named {
            units: Some(&units()),
            texts: None,
        },
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        at(&result.total, hour(1)).unwrap().total_raw.as_deref(),
        Some("200000")
    );
}

fn with_zero_rocket(track: TagHistory) -> ScannedProduced {
    let first = track.hours[0];
    let rocket = TagHistory {
        hours: vec![first],
        last_hours: vec![*track.last_hours.last().unwrap()],
        identities: vec![track.identities[0]],
        presence: vec![Some(false)],
        ..Default::default()
    };
    let mut scanned = ScannedProduced::default();
    scanned.observations.push(first);
    scanned.rocket.observations.push(first);
    scanned.rocket.reads.push(first);
    scanned.rocket.tags.insert("opaque|tag".into(), rocket);
    scanned.tags.insert("opaque|tag".into(), track);
    scanned
}

#[test]
fn production_spending_many_retired_designs_have_sparse_runs_over_long_history() {
    use production::{TagHistory, Total};
    let mut track = TagHistory::default();
    for day in 0..10_000 {
        track.hours.push(hour(day));
        track.last_hours.push(hour(day) + 23);
        track.starts.push(day == 0);
        track.identities.push((1, 1, 0));
    }
    for variant in 1..=1_000 {
        track.changes.insert(
            (variant, false),
            vec![(
                0,
                Total {
                    units: 1,
                    inexact: 0,
                },
            )],
        );
        track.cost_changes.insert(
            (variant, false),
            vec![(
                0,
                CostTotal {
                    raw: 100_000,
                    reason: None,
                },
            )],
        );
    }
    let scanned = with_zero_rocket(track);
    let result = reply(
        &scanned,
        &params(false),
        Named::default(),
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.leaves.len(), 1_000);
    assert!(result.leaves.iter().all(|leaf| leaf.runs.len() == 1));
    assert_eq!(result.total.len(), 1);
    assert_eq!(result.total[0].total_raw.as_deref(), Some("100000000"));
}

#[test]
fn production_spending_keeps_more_than_300000_changes_and_runs_within_reply_budget() {
    let samples = 150_001u32;
    let mut track = TagHistory::default();
    for sample in 0..samples {
        track.hours.push(sample as i32);
        track.last_hours.push(sample as i32);
        track.identities.push((1, 1, 0));
    }
    for refit in [false, true] {
        track.cost_changes.insert(
            (1, refit),
            (0..samples)
                .map(|sample| {
                    (
                        sample,
                        CostTotal {
                            raw: i128::from(sample + 1),
                            reason: None,
                        },
                    )
                })
                .collect(),
        );
    }
    let scanned = with_zero_rocket(track);
    let result = reply(
        &scanned,
        &params(true),
        Named::default(),
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(result.leaves[0].runs.len(), samples as usize);
    assert_eq!(result.total.len(), samples as usize);
    assert_eq!(
        result.total.last().unwrap().total_raw.as_deref(),
        Some("300002")
    );
    assert!(crate::transport::json_len(&result, MAX_REPLY_BYTES).is_some());
}

#[test]
fn production_spending_refuses_complete_reply_past_byte_budget() {
    let error = reply(
        &ScannedProduced::default(),
        &params(false),
        Named::default(),
        vec!["x".repeat(MAX_REPLY_BYTES)],
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("reply exceeds its memory budget")
    );
}
