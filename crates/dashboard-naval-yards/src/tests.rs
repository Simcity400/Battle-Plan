use crate::products::{
    naval_refits::{self, RefitsObserver},
    naval_repairs::{self, RepairsObserver},
    yards,
};
use dashboard_base::{
    prepared::{coverage::Coverage, prefix::Prefix, series::tests::Fixture},
    products::Product,
    test_fixtures,
};
use dashboard_keyed::{
    Observer, Scan, Values,
    query::{self, KeyedWindow, Query},
};
use replay_data::{format::FormatVersion, rows::*};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::AtomicBool},
};
const COUNTRY: &str = "MOD@7";
const SHIP: u64 = (9_u64 << 32) | 61;
const TASK: u64 = (3_u64 << 32) | 61;
const LINE: u64 = (5_u64 << 32) | 71;

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = test_fixtures::source(1, hour, Some(1));
    for &table in naval_repairs::TABLES.iter().chain(naval_refits::TABLES) {
        if !matches!(
            table,
            TableId::Countries | TableId::CountryLinks | TableId::ReservedCountryNaming
        ) {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
    }
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 11,
            canonical_slot: 7,
            tag: b"MOD".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id: 11,
            canonical_slot: 7,
        }]));
    *frame.tables[TableId::TaskForces.index()] =
        TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
            id: TASK,
            roster_owner: 11,
            ..Default::default()
        }]));
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![ShipRow {
        id: SHIP,
        task_force: TASK,
        strength: 123_000,
        ..Default::default()
    }]));
    *frame.tables[TableId::ShipModels.index()] =
        TableView::Complete(TableRows::ShipModels(vec![ShipModelRow {
            ship: SHIP,
            counter: Some(ShipCounterInputs {
                maximum_strength: 200_000,
                ..Default::default()
            }),
            ..Default::default()
        }]));
    *frame.tables[TableId::NavalBases.index()] =
        TableView::Complete(TableRows::NavalBases(vec![NavalBaseRow {
            province: 9,
            country: 11,
            ..Default::default()
        }]));
    *frame.tables[TableId::NavalRepairBases.index()] =
        TableView::Complete(TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
            province: 9,
            priority: -3,
            allocated_dockyards: 4,
        }]));
    *frame.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![ProvinceRow {
            id: 9,
            state: 1,
            ..Default::default()
        }]));
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner: 11,
            ..Default::default()
        }]));
    frame
}
fn repair_frame(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour);
    *frame.tables[TableId::NavalRepairQueue.index()] =
        TableView::Complete(TableRows::NavalRepairQueue(vec![
            NavalRepairQueueRow {
                province: 9,
                ordinal: 0,
                ship: SHIP,
            },
            NavalRepairQueueRow {
                province: 9,
                ordinal: 1,
                ship: SHIP,
            },
        ]));
    frame
}
fn refit_frame(hour: i32) -> DecodedFrame {
    let mut frame = frame(hour);
    *frame.tables[TableId::NavalRefitShips.index()] =
        TableView::Complete(TableRows::NavalRefitShips(vec![NavalRefitShipRow {
            ship: SHIP,
            line: LINE,
        }]));
    *frame.tables[TableId::NavalRefitProgress.index()] =
        TableView::Complete(TableRows::NavalRefitProgress(vec![NavalRefitProgressRow {
            line: LINE,
            progress_raw: 123_456,
        }]));
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(vec![ProductionLineRow {
            handle: LINE,
            country: 11,
            cost_raw: 1_000_000,
            active_factories: 3,
            ..Default::default()
        }]));
    frame
}
fn missing(frame: &mut DecodedFrame, table: TableId) {
    *frame.tables[table.index()] = TableView::Unavailable {
        reason: format!("Synthetic {table:?} refusal"),
    };
}
fn values<O: Observer>(frame: &DecodedFrame) -> BTreeMap<String, i128> {
    let mut observer = O::default();
    let mut values = Values::default();
    assert!(
        observer
            .observe(FormatVersion::CURRENT, frame, &mut values)
            .unwrap()
    );
    values
        .iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}
fn flush<O: Observer>(fixture: &Fixture, scan: &mut Scan<O>) {
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    scan.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
}
fn window<O: Observer>(fixture: &Fixture, first: i32, last: i32) -> KeyedWindow {
    let flag = AtomicBool::new(false);
    query::open::<O>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap()
        .window(first, last, None, |_| true)
        .unwrap()
}
fn runs(window: &KeyedWindow, name: &str) -> Vec<query::KeyedRun> {
    window
        .series
        .iter()
        .find(|series| series.name == name)
        .map(|series| series.runs.clone())
        .unwrap_or_default()
}
fn source(frames: Vec<DecodedFrame>) -> dashboard_base::Source {
    let replay = replay::container::Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(1, frames)],
    )
    .unwrap();
    dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "yards-synthetic".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn prefix(first: i32, last: i32, complete: bool) -> Prefix {
    Prefix {
        coverage: Coverage {
            schema: 1,
            first_hour: first,
            last_hour: last,
            complete,
        },
        pending: vec![],
    }
}

#[test]
fn repairs_keep_building_limit_separate_native_queue_repeats_and_independent_hull() {
    let mut frame = repair_frame(100);
    *frame.tables[TableId::NavalRepairCountries.index()] =
        TableView::Complete(TableRows::NavalRepairCountries(vec![
            NavalRepairCountryRow {
                country: 11,
                max_factories_for_repair: -1,
                max_allowed_dockyards_to_repair: 8,
                num_used_dockyards: 4,
            },
        ]));
    *frame.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(vec![CountryFactoriesRow {
            country: 11,
            naval_total_raw: 6_499_999,
            ..Default::default()
        }]));
    let read = values::<RepairsObserver>(&frame);
    assert_eq!(read[&format!("{COUNTRY}|total|buildingRepairLimit")], -1);
    assert_eq!(read[&format!("{COUNTRY}|total|dockyards")], 64);
    assert_eq!(read[&format!("{COUNTRY}|total|queueEntries")], 2);
    assert_eq!(read[&format!("{COUNTRY}|total|distinctQueuedShips")], 1);
    for ordinal in [0, 1] {
        assert_eq!(
            read[&format!("{COUNTRY}|9|queue|{ordinal}")],
            i128::from(SHIP)
        );
    }
    let ship = format!("{COUNTRY}|9|ship|{SHIP}");
    assert_eq!(read[&format!("{ship}|hull")], 123_000);
    assert_eq!(read[&format!("{ship}|maximum")], 200_000);
    missing(&mut frame, TableId::ShipModels);
    missing(&mut frame, TableId::VariantDefinitions);
    let read = values::<RepairsObserver>(&frame);
    assert_eq!(read[&format!("{ship}|hull")], 123_000);
    assert!(!read.contains_key(&format!("{ship}|maximum")));
    assert_eq!(read[&format!("{COUNTRY}|total|dockyards")], 64);
}

#[test]
fn repairs_known_empty_missing_ports_and_queues_never_become_the_same_state() {
    let fixture = Fixture::new();
    let mut scan = Scan::<RepairsObserver>::default();
    let a = repair_frame(100);
    let mut b = frame(101);
    let c = frame(102);
    missing(&mut b, TableId::NavalRepairQueue);
    let mut d = frame(103);
    missing(&mut d, TableId::NavalBases);
    for frame in [a, b, c, d] {
        scan.prepare(FormatVersion::CURRENT, &frame, true).unwrap();
    }
    flush(&fixture, &mut scan);
    let read = window::<RepairsObserver>(&fixture, 100, 103);
    assert_eq!(
        runs(&read, &format!("{COUNTRY}|total|queueEntries")),
        vec![(100, 100, "2".into(), true), (102, 102, "0".into(), true)]
    );
    assert_eq!(
        runs(&read, &format!("{COUNTRY}|known|NavalRepairQueue")),
        vec![
            (100, 100, "1".into(), true),
            (101, 101, "0".into(), false),
            (102, 103, "1".into(), false)
        ]
    );
    assert_eq!(read.known.len(), 1);
}

#[test]
fn rejoin_and_ordered_children_resolve_only_against_complete_full_task_references() {
    let destination = TASK + 1;
    let mut frame = repair_frame(1);
    *frame.tables[TableId::NavalRepairTaskForces.index()] =
        TableView::Complete(TableRows::NavalRepairTaskForces(vec![
            NavalRepairTaskForceRow {
                task_force: TASK,
                merge_after_repair: destination,
                refit_after_repair: u64::MAX,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::NavalRepairChildren.index()] =
        TableView::Complete(TableRows::NavalRepairChildren(vec![
            NavalRepairChildRow {
                task_force: TASK,
                ordinal: 0,
                child: destination,
            },
            NavalRepairChildRow {
                task_force: TASK,
                ordinal: 1,
                child: destination,
            },
        ]));
    let key = format!("{COUNTRY}|9|ship|{SHIP}");
    let read = values::<RepairsObserver>(&frame);
    assert_eq!(read[&format!("{key}|rejoin")], i128::from(destination));
    assert_eq!(read[&format!("{key}|rejoinResolves")], 0);
    assert_eq!(read[&format!("{key}|child:0")], i128::from(destination));
    assert_eq!(read[&format!("{key}|child:1")], i128::from(destination));
    if let TableView::Complete(TableRows::TaskForces(rows)) =
        &mut *frame.tables[TableId::TaskForces.index()]
    {
        rows.push(TaskForceRow {
            id: destination,
            ..Default::default()
        });
    }
    let read = values::<RepairsObserver>(&frame);
    assert_eq!(read[&format!("{key}|rejoinResolves")], 1);
    assert_eq!(read[&format!("{key}|childResolves:0")], 1);
    missing(&mut frame, TableId::TaskForces);
    let read = values::<RepairsObserver>(&frame);
    assert!(read.contains_key(&format!("{key}|rejoin")));
    assert!(!read.contains_key(&format!("{key}|rejoinResolves")));
}

#[test]
fn refit_liveness_is_hourly_and_full_reference_not_stale_daily_metadata() {
    let mut frame = refit_frame(7);
    let key = format!("{COUNTRY}|{LINE}");
    let read = values::<RefitsObserver>(&frame);
    assert_eq!(read[&format!("{key}|progress")], 123_456);
    for replacement in [
        vec![],
        vec![NavalRefitProgressRow {
            line: LINE + 1,
            progress_raw: 900,
        }],
    ] {
        *frame.tables[TableId::NavalRefitProgress.index()] =
            TableView::Complete(TableRows::NavalRefitProgress(replacement));
        let read = values::<RefitsObserver>(&frame);
        assert_eq!(read[&format!("{COUNTRY}|total|ships")], 0);
        assert!(!read.contains_key(&format!("{key}|progress")));
        assert!(!read.contains_key(&format!("{key}|cost")));
    }
    missing(&mut frame, TableId::NavalRefitProgress);
    let read = values::<RefitsObserver>(&frame);
    assert!(!read.contains_key(&format!("{COUNTRY}|total|ships")));
    assert_eq!(read[&format!("{key}|cost")], 1_000_000);
    assert!(!read.contains_key(&format!("{key}|ship|{SHIP}|present")));
}

#[test]
fn refits_count_shared_lines_once_keep_signed_progress_and_refuse_invalid_costs() {
    let mut frame = refit_frame(8);
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *frame.tables[TableId::Ships.index()]
    {
        rows.push(ShipRow {
            id: SHIP + 1,
            task_force: TASK,
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::NavalRefitShips(rows)) =
        &mut *frame.tables[TableId::NavalRefitShips.index()]
    {
        rows.push(NavalRefitShipRow {
            ship: SHIP + 1,
            line: LINE,
        });
    }
    if let TableView::Complete(TableRows::NavalRefitProgress(rows)) =
        &mut *frame.tables[TableId::NavalRefitProgress.index()]
    {
        rows[0].progress_raw = -75;
    }
    let key = format!("{COUNTRY}|{LINE}");
    let read = values::<RefitsObserver>(&frame);
    assert_eq!(read[&format!("{COUNTRY}|total|ships")], 2);
    assert_eq!(read[&format!("{COUNTRY}|total|lines")], 1);
    assert_eq!(read[&format!("{COUNTRY}|total|dockyards")], 3);
    assert_eq!(read[&format!("{key}|progress")], -75);
    assert_eq!(
        read[&format!("{COUNTRY}|total|remainingCostRaw")],
        1_000_075
    );
    for cost in [0, -1, i64::MAX, i64::MIN] {
        if let TableView::Complete(TableRows::ProductionLines(rows)) =
            &mut *frame.tables[TableId::ProductionLines.index()]
        {
            rows[0].cost_raw = cost;
        }
        let read = values::<RefitsObserver>(&frame);
        assert_eq!(read[&format!("{key}|cost")], i128::from(cost));
        assert!(!read.contains_key(&format!("{key}|remainingCostRaw")));
        assert!(!read.contains_key(&format!("{COUNTRY}|total|remainingCostRaw")));
    }
    missing(&mut frame, TableId::ProductionLines);
    let read = values::<RefitsObserver>(&frame);
    assert_eq!(read[&format!("{key}|progress")], -75);
    assert!(!read.contains_key(&format!("{COUNTRY}|total|dockyards")));
}

#[test]
fn independent_daily_read_dates_advance_on_equal_reads_and_reset_across_gaps() {
    let fixture = Fixture::new();
    let mut scan = Scan::<RefitsObserver>::default();
    for hour in 7..=33 {
        let mut frame = refit_frame(hour);
        if hour == 7 {
            frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
        }
        let changes = dashboard_base::prepared::frame_changes::FrameChanges::alone(
            &frame,
            FormatVersion::CURRENT,
        );
        scan.prepare_changed(FormatVersion::CURRENT, &frame, &changes)
            .unwrap();
    }
    flush(&fixture, &mut scan);
    let read = window::<RefitsObserver>(&fixture, 7, 33);
    assert_eq!(
        runs(&read, &format!("{COUNTRY}|read|ProductionLines")),
        vec![
            (7, 7, "7".into(), true),
            (8, 31, "8".into(), false),
            (32, 33, "32".into(), false)
        ]
    );
    assert_eq!(
        runs(&read, &format!("{COUNTRY}|read|VariantDefinitions")),
        vec![(22, 33, "22".into(), true)]
    );
    scan.prepare(FormatVersion::CURRENT, &refit_frame(35), true)
        .unwrap();
    flush(&fixture, &mut scan);
    assert!(
        runs(
            &window::<RefitsObserver>(&fixture, 35, 35),
            &format!("{COUNTRY}|read|ProductionLines")
        )
        .is_empty()
    );
}

#[test]
fn scans_resume_append_keep_absence_unknown_gaps_and_tag_reuse_separate() {
    let mut fixture = Fixture::new();
    let mut scan = Scan::<RepairsObserver>::default();
    scan.prepare(FormatVersion::CURRENT, &repair_frame(100), true)
        .unwrap();
    flush(&fixture, &mut scan);
    let serial = serde_json::to_vec(&scan).unwrap();
    let mut scan: Scan<RepairsObserver> = serde_json::from_slice(&serial).unwrap();
    let mut unavailable = repair_frame(101);
    missing(&mut unavailable, TableId::Countries);
    scan.prepare(FormatVersion::CURRENT, &unavailable, true)
        .unwrap();
    let mut reused = repair_frame(103);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *reused.tables[TableId::Countries.index()]
    {
        rows[0].canonical_slot = 8;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *reused.tables[TableId::CountryLinks.index()]
    {
        rows[0].canonical_slot = 8;
    }
    scan.prepare(FormatVersion::CURRENT, &reused, true).unwrap();
    let mut writer = fixture.append_writer();
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    let flag = AtomicBool::new(false);
    let mut open = query::open::<RepairsObserver>(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let read = open.window(100, 103, Some(100), |_| true).unwrap();
    assert_eq!(read.known.len(), 1);
    assert!(
        read.series
            .iter()
            .all(|series| !series.name.starts_with("MOD@8"))
    );
    let read = open.window(100, 103, None, |_| true).unwrap();
    assert_eq!(read.known.len(), 2);
    assert_eq!(
        runs(&read, &format!("{COUNTRY}|9|ship|{SHIP}|hull")),
        vec![(100, 100, "123000".into(), true)]
    );
    assert_eq!(
        runs(&read, &format!("MOD@8|9|ship|{SHIP}|hull")),
        vec![(103, 103, "123000".into(), true)]
    );
}

#[test]
fn queries_are_cold_empty_prefix_clipped_country_filtered_and_cancelled() {
    let source = source(vec![repair_frame(100), repair_frame(101)]);
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let params = serde_json::json!({"branchId":"1","tag":"MOD"});
    let prefix = prefix(100, 100, false);
    let query = Query {
        source: &source,
        params: &params,
        content: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        prefix: &prefix,
        cancelled: &flag,
    };
    let cold = naval_repairs::query(&query).unwrap();
    assert_eq!(cold["window"]["series"], serde_json::json!([]));
    assert_eq!(cold["ports"], serde_json::json!([]));
    let mut scan = Scan::<RepairsObserver>::default();
    for hour in [100, 101] {
        scan.prepare(FormatVersion::CURRENT, &repair_frame(hour), true)
            .unwrap();
    }
    flush(&fixture, &mut scan);
    let reply = naval_repairs::query(&query).unwrap();
    assert_eq!(reply["ports"].as_array().unwrap().len(), 1);
    assert_eq!(reply["ports"][0]["ships"].as_array().unwrap().len(), 1);
    assert_eq!(reply["ports"][0]["ships"][0]["id"], SHIP.to_string());
    assert!(
        reply["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .all(|series| series["runs"]
                .as_array()
                .unwrap()
                .iter()
                .all(|run| run[1].as_i64().unwrap() == 100))
    );
    flag.store(true, std::sync::atomic::Ordering::Release);
    assert!(naval_repairs::query(&query).is_err());
    assert!(naval_refits::query(&query).is_err());
}

#[test]
fn five_thousand_countries_and_large_shared_line_queues_remain_sparse() {
    let mut frame = refit_frame(8);
    let mut countries = Vec::new();
    let mut links = Vec::new();
    let mut tasks = Vec::new();
    let mut ships = Vec::new();
    let mut refits = Vec::new();
    let mut bases = Vec::new();
    let mut queue = Vec::new();
    for index in 1..=5000_u32 {
        countries.push(CountryRow {
            id: index,
            canonical_slot: index,
            tag: format!("C{index}").into_bytes(),
            ..Default::default()
        });
        links.push(CountryLinkRow {
            id: index,
            canonical_slot: index,
        });
        let task = (u64::from(index) << 32) | 61;
        let ship = (u64::from(index) << 32) | 62;
        tasks.push(TaskForceRow {
            id: task,
            roster_owner: index,
            ..Default::default()
        });
        ships.push(ShipRow {
            id: ship,
            task_force: task,
            strength: i64::from(index),
            name: Some(
                format!(
                    "Synthetic ship {index} {}",
                    "x".repeat(index as usize % 257)
                )
                .into_bytes(),
            )
            .into(),
            ..Default::default()
        });
        refits.push(NavalRefitShipRow { ship, line: LINE });
        bases.push(NavalBaseRow {
            province: index,
            country: index,
            ..Default::default()
        });
        queue.push(NavalRepairQueueRow {
            province: index,
            ordinal: 0,
            ship,
        });
    }
    for (table, rows) in [
        (TableId::Countries, TableRows::Countries(countries)),
        (TableId::CountryLinks, TableRows::CountryLinks(links)),
        (TableId::TaskForces, TableRows::TaskForces(tasks)),
        (TableId::Ships, TableRows::Ships(ships)),
        (TableId::NavalRefitShips, TableRows::NavalRefitShips(refits)),
        (TableId::NavalBases, TableRows::NavalBases(bases)),
        (
            TableId::NavalRepairQueue,
            TableRows::NavalRepairQueue(queue),
        ),
    ] {
        *frame.tables[table.index()] = TableView::Complete(rows);
    }
    let refits = values::<RefitsObserver>(&frame);
    let repairs = values::<RepairsObserver>(&frame);
    assert_eq!(refits["C5000@5000|total|ships"], 1);
    assert_eq!(repairs["C5000@5000|total|queueEntries"], 1);
    assert!(refits.len() < 5000 * 100);
    assert!(repairs.len() < 5000 * 100);
    // One country's many ships still allocate/cost the shared line once.
    if let TableView::Complete(TableRows::TaskForces(tasks)) =
        &mut *frame.tables[TableId::TaskForces.index()]
    {
        for task in tasks {
            task.roster_owner = 1;
        }
    }
    let read = values::<RefitsObserver>(&frame);
    assert_eq!(read["C1@1|total|ships"], 5000);
    assert_eq!(read["C1@1|total|dockyards"], 3);
}

#[test]
fn product_contracts_export_every_nested_type_validate_and_refuse_direct_loads() {
    for types in [
        naval_repairs::NavalRepairs::types(),
        naval_refits::NavalRefits::types(),
    ] {
        assert!(types.iter().any(|decl| decl.contains("YardGroup")));
        assert!(types.iter().any(|decl| decl.contains("KeyedSeries")));
    }
    let params = yards::NavalYardsParams {
        branch_id: "1".into(),
        tag: "MOD".into(),
        first_hour: Some(10),
        last_hour: Some(9),
    };
    assert!(params.validate().is_err());
    let source = source(vec![frame(1)]);
    let valid = yards::NavalYardsParams {
        first_hour: None,
        last_hour: None,
        ..params
    };
    let context = dashboard_base::products::LoadContext { content: None };
    let flag = AtomicBool::new(false);
    assert_eq!(
        naval_repairs::NavalRepairs::load(&source, &valid, &context, &flag)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Unsupported
    );
    assert_eq!(
        naval_refits::NavalRefits::load(&source, &valid, &context, &flag)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Unsupported
    );
    assert!(
        yards::checked(&"x".repeat(dashboard_base::products::MAX_PRODUCT_REPLY_BYTES)).is_err()
    );
}

#[test]
fn changing_design_suppliers_and_content_binding_change_names_not_hourly_progress() {
    let mut a = refit_frame(8);
    if let TableView::Complete(TableRows::ProductionLines(rows)) =
        &mut *a.tables[TableId::ProductionLines.index()]
    {
        rows[0].variant = 77;
    }
    *a.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: 77,
            equipment_token: 99,
            creator_link: 11,
            ..Default::default()
        }]));
    *a.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 99,
            literal: b"mod_design".to_vec(),
        }]));
    let mut b = a.clone();
    b.hour = 9;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *b.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(42);
    }
    let mut c = b.clone();
    c.hour = 10;
    if let TableView::Complete(TableRows::VariantDefinitions(rows)) =
        &mut *c.tables[TableId::VariantDefinitions.index()]
    {
        rows[0].display_name = b"Changed design".to_vec();
    }
    let source = source(vec![a.clone(), b.clone(), c.clone()]);
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut scan = Scan::<RefitsObserver>::default();
    for frame in [&a, &b, &c] {
        scan.prepare(FormatVersion::CURRENT, frame, true).unwrap();
    }
    flush(&fixture, &mut scan);
    let read = window::<RefitsObserver>(&fixture, 8, 10);
    let name = runs(&read, &format!("{COUNTRY}|{LINE}|targetName"));
    assert_eq!(name.len(), 2);
    assert_eq!((name[0].0, name[0].1), (8, 9));
    let params = serde_json::json!({"branchId":"1","tag":"MOD","firstHour":8,"lastHour":9});
    let prefix = prefix(8, 10, true);
    let (one, one_folder) = dashboard_base::catalogue::tests::folder_catalogue(&[(
        "localisation/base_l_english.yml",
        "\u{feff}l_english:\n mod_design:0 \"First design\"\n",
    )]);
    let (two, two_folder) = dashboard_base::catalogue::tests::folder_catalogue(&[(
        "localisation/base_l_english.yml",
        "\u{feff}l_english:\n mod_design:0 \"Second design\"\n",
    )]);
    let mut names = Vec::new();
    for (cat, folder) in [(one, one_folder), (two, two_folder)] {
        let options = dashboard_core::config::ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: folder.0.clone(),
                user_dir: folder.0.join("user"),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        dashboard_base::catalogue::set_test_catalogue(&source, &options, Arc::new(cat));
        let query = Query {
            source: &source,
            params: &params,
            content: Some(&options),
            store: &fixture.store,
            key: &fixture.key,
            branch: 1,
            prefix: &prefix,
            cancelled: &flag,
        };
        let (reply, decoded) =
            dashboard_base::prepared::query::progressive::without_block_decodes(|| {
                naval_refits::query(&query)
            });
        assert!(!decoded);
        let reply = reply.unwrap();
        names.push(reply["lines"][0]["name"].as_str().unwrap().to_owned());
        let progress = reply["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|series| series["name"] == format!("{COUNTRY}|{LINE}|progress"))
            .unwrap();
        assert_eq!(progress["runs"][0][2], "123456");
    }
    assert_eq!(names, ["First design", "Second design"]);
}

#[test]
fn keyed_prepass_fold_matches_hourly_refit_walk_including_daily_siblings() {
    let frames: Vec<_> = (7..=10).map(refit_frame).collect();
    let source = source(frames.clone());
    let replay = source.replay();
    let format = FormatVersion::CURRENT;
    let mut extract = (naval_refits::FAMILY.start)(format);
    let mut direct = Scan::<RefitsObserver>::default();
    for frame in &frames {
        let changes = dashboard_base::prepared::frame_changes::FrameChanges::alone(frame, format);
        extract.observe(&replay, frame, &changes).unwrap();
        direct.prepare_changed(format, frame, &changes).unwrap();
    }
    let mut folded = Scan::<RefitsObserver>::default();
    let (_, through) = dashboard_keyed::block::fold(
        &mut folded,
        extract.finish().unwrap(),
        &replay,
        &replay.index[0],
        &mut |_, result| result.map(|_| true),
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(through, Some(10));
    let first = Fixture::new();
    let second = Fixture::new();
    flush(&first, &mut direct);
    flush(&second, &mut folded);
    let a = serde_json::to_value(window::<RefitsObserver>(&first, 7, 10)).unwrap();
    let b = serde_json::to_value(window::<RefitsObserver>(&second, 7, 10)).unwrap();
    assert_eq!(a, b);
}

#[test]
fn ship_names_observers_prefer_cache_and_keep_rebuild_provenance_and_numbers() {
    let mut repair = repair_frame(100);
    let mut refit = refit_frame(100);
    for frame in [&mut repair, &mut refit] {
        if let TableView::Complete(TableRows::Ships(rows)) =
            &mut *frame.tables[TableId::Ships.index()]
        {
            rows[0].name = Some(b"Recorded cache".to_vec()).into();
            rows[0].naming = Some(ShipNamingOperands {
                kind: 1,
                order: 2,
                ordered: 1,
                equipment: Some(70),
                ..Default::default()
            })
            .into();
        }
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![
                VariantDefinitionRow {
                    id: 70,
                    design: VariantDesignState {
                        division_names_group: Some(b"NAMING_GROUP".to_vec()),
                        ..Default::default()
                    }
                    .into(),
                    ..Default::default()
                },
                VariantDefinitionRow {
                    id: 80,
                    design: VariantDesignState {
                        division_names_group: Some(b"HULL_GROUP".to_vec()),
                        ..Default::default()
                    }
                    .into(),
                    ..Default::default()
                },
            ]));
        if let TableView::Complete(TableRows::ShipModels(rows)) =
            &mut *frame.tables[TableId::ShipModels.index()]
        {
            rows[0].variant = 80;
        }
    }
    let repair_key = format!("{COUNTRY}|9|ship|{SHIP}");
    let refit_key = format!("{COUNTRY}|{LINE}|ship|{SHIP}");
    for (frame, key) in [(&repair, &repair_key), (&refit, &refit_key)] {
        let mut state = yards::State::default();
        let mut read = Values::default();
        state.ship(frame, &mut read, COUNTRY, key, SHIP);
        let read: BTreeMap<_, _> = read
            .iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        assert!(read.contains_key(&format!("{key}|name")));
        assert!(!read.contains_key(&format!("{key}|nameRebuilt")));
        assert!(
            matches!(&state.tokens[read[&format!("{key}|name")] as usize], yards::Operand::ShipCache { bytes, .. } if bytes == b"Recorded cache")
        );
    }
    for frame in [&mut repair, &mut refit] {
        if let TableView::Complete(TableRows::Ships(rows)) =
            &mut *frame.tables[TableId::Ships.index()]
        {
            rows[0].name = None.into();
        }
    }
    for (frame, key) in [(&repair, &repair_key), (&refit, &refit_key)] {
        let mut state = yards::State::default();
        let mut read = Values::default();
        state.ship(frame, &mut read, COUNTRY, key, SHIP);
        let read: BTreeMap<_, _> = read
            .iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect();
        assert!(!read.contains_key(&format!("{key}|name")));
        let id = read[&format!("{key}|nameRebuilt")] as usize;
        assert!(
            matches!(&state.tokens[id], yards::Operand::ShipRebuilt { group: Ok(Some(key)), .. } if key == b"NAMING_GROUP")
        );
        assert_eq!(read[&format!("{key}|hull")], 123_000);
        let restored: yards::State =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert!(matches!(
            &restored.tokens[id],
            yards::Operand::ShipRebuilt { .. }
        ));
    }
    let repairs = values::<RepairsObserver>(&repair);
    let refits = values::<RefitsObserver>(&refit);
    assert!(repairs.contains_key(&format!("{repair_key}|nameRebuilt")));
    assert!(refits.contains_key(&format!("{refit_key}|nameRebuilt")));
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *repair.tables[TableId::Ships.index()]
    {
        rows[0].name = Some(Vec::new()).into();
    }
    let read = values::<RepairsObserver>(&repair);
    assert!(read.contains_key(&format!("{repair_key}|name")));
    assert!(!read.contains_key(&format!("{repair_key}|nameRebuilt")));
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *repair.tables[TableId::Ships.index()]
    {
        rows[0].name = None.into();
        rows[0]
            .naming
            .require_mut("naming")
            .unwrap()
            .as_mut()
            .unwrap()
            .equipment = None;
    }
    let mut state = yards::State::default();
    let mut values = Values::default();
    state.ship(&repair, &mut values, COUNTRY, &repair_key, SHIP);
    assert!(state.tokens.iter().any(|operand| matches!(operand, yards::Operand::ShipRebuilt { group: Ok(Some(key)), .. } if key == b"HULL_GROUP")));
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *repair.tables[TableId::Ships.index()]
    {
        rows[0]
            .naming
            .require_mut("naming")
            .unwrap()
            .as_mut()
            .unwrap()
            .equipment = Some(90);
    }
    let mut state = yards::State::default();
    state.ship(&repair, &mut Values::default(), COUNTRY, &repair_key, SHIP);
    assert!(state.tokens.iter().any(|operand| matches!(operand, yards::Operand::ShipRebuilt { group: Err(why), .. } if why.contains("90"))));
}

#[test]
fn ship_names_history_dates_renames_refusals_full_references_and_gaps() {
    let fixture = Fixture::new();
    let mut scan = Scan::<RepairsObserver>::default();
    let key = format!("{COUNTRY}|9|ship|{SHIP}|name");
    for hour in [100, 101, 102, 104, 105] {
        let mut frame = repair_frame(hour);
        if let TableView::Complete(TableRows::Ships(rows)) =
            &mut *frame.tables[TableId::Ships.index()]
        {
            rows[0].name = match hour {
                102 => None,
                100 => Some(b"Before".to_vec()),
                _ => Some(b"After".to_vec()),
            }
            .into();
            if hour == 105 {
                rows[0].id = SHIP + (1_u64 << 32);
            }
        }
        scan.prepare(FormatVersion::CURRENT, &frame, true).unwrap();
    }
    flush(&fixture, &mut scan);
    let window = window::<RepairsObserver>(&fixture, 100, 105);
    let names = window
        .series
        .iter()
        .find(|series| series.name == key)
        .unwrap();
    assert_eq!(
        names
            .runs
            .iter()
            .map(|(first, last, _, _)| (*first, *last))
            .collect::<Vec<_>>(),
        [(100, 100), (101, 101), (104, 104)]
    );
    assert!(names.runs[2].3);
    assert_ne!(names.runs[0].2, names.runs[1].2);
    assert_eq!(names.runs[1].2, names.runs[2].2);
    let numeric = window
        .series
        .iter()
        .find(|series| series.name == format!("{COUNTRY}|9|ship|{SHIP}|hull"))
        .unwrap();
    assert!(
        numeric
            .runs
            .iter()
            .any(|(first, last, _, _)| *first <= 102 && *last >= 102)
    );
}

#[test]
fn ship_names_query_resolves_only_demanded_cache_and_rebuilt_tokens_without_frames() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let fixture = Fixture::new();
    let mut scan = Scan::<RepairsObserver>::default();
    let mut frames = Vec::new();
    for hour in [100, 101] {
        let mut frame = repair_frame(hour);
        if let TableView::Complete(TableRows::Ships(rows)) =
            &mut *frame.tables[TableId::Ships.index()]
        {
            rows[0].name = if hour == 100 {
                Some(b"Recorded cache".to_vec())
            } else {
                None
            }
            .into();
            rows[0].naming = Some(ShipNamingOperands {
                kind: 1,
                order: 2,
                ordered: 1,
                equipment: Some(70),
                ..Default::default()
            })
            .into();
        }
        *frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: 70,
                design: VariantDesignState {
                    division_names_group: Some(b"MOD_GROUP".to_vec()),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }]));
        scan.prepare(FormatVersion::CURRENT, &frame, true).unwrap();
        frames.push(frame);
    }
    // A valid but undemanded operand must never be returned or resolved.
    let unused = scan.observer.state.intern(yards::Operand::ShipCache {
        handle: 1,
        bytes: vec![0xff],
    });
    flush(&fixture, &mut scan);
    let source = source(frames);
    let (cat, folder) = dashboard_base::catalogue::tests::folder_catalogue(&[(
        "common/units/names_ships/synthetic.txt",
        "MOD_GROUP={type=ship ordered={2={\"Rebuilt %d\"}}}",
    )]);
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folder.0.clone(),
            user_dir: folder.0.join("user"),
            cache_dir: folder.0.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    dashboard_base::catalogue::set_test_catalogue(&source, &options, Arc::new(cat));
    let prefix = prefix(100, 101, true);
    let cancelled = AtomicBool::new(false);
    for (first, last, field, expected) in [
        (100, 100, "name", "Recorded cache"),
        (101, 101, "nameRebuilt", "Rebuilt 2"),
    ] {
        let params =
            serde_json::json!({"branchId":"1", "tag":"MOD", "firstHour":first, "lastHour":last});
        let query = Query {
            source: &source,
            params: &params,
            content: Some(&options),
            store: &fixture.store,
            key: &fixture.key,
            branch: 1,
            prefix: &prefix,
            cancelled: &cancelled,
        };
        let (reply, decoded) =
            dashboard_base::prepared::query::progressive::without_block_decodes(|| {
                naval_repairs::query(&query)
            });
        assert!(!decoded);
        let reply = reply.unwrap();
        assert_eq!(reply["ports"][0]["ships"][0]["name"], expected);
        let series = reply["window"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .find(|series| series["name"] == format!("{COUNTRY}|9|ship|{SHIP}|{field}"))
            .unwrap();
        let token = series["runs"][0][2].as_str().unwrap();
        assert_eq!(reply["tokens"][token]["name"], expected);
        assert!(reply["tokens"].get(unused.to_string()).is_none());
        assert!(
            !reply["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| warning.as_str().unwrap().contains("ship cached name"))
        );
    }
}

fn s5_management_frames(first: i32, last: i32) -> Vec<DecodedFrame> {
    let mut base = refit_frame(first);
    base.tables[TableId::NavalRepairQueue.index()] =
        repair_frame(first).tables[TableId::NavalRepairQueue.index()].clone();
    for &table in naval_repairs::TABLES {
        if !matches!(*base.tables[table.index()], TableView::Complete(_)) {
            let rows = match table {
                TableId::ReservedCountryNaming => {
                    TableRows::ReservedCountryNaming(vec![Default::default()])
                }
                _ => TableRows::empty(table),
            };
            *base.tables[table.index()] = TableView::Complete(rows);
        }
    }
    (first..=last)
        .map(|hour| {
            let mut frame = base.clone();
            frame.hour = hour;
            frame.metrics = Default::default();
            frame
        })
        .collect()
}

fn s5_fold<O: Observer>(source: &dashboard_base::Source, scan: &mut Scan<O>) -> (usize, usize) {
    use dashboard_base::prepared::frame_changes::FrameChanges;
    use replay::container::BlockChange;
    let replay = source.replay();
    scan.observer.bind(source, None, &mut || Ok(())).unwrap();
    let mut changed = 0;
    let mut quiet = 0;
    for block in &replay.index {
        let mut extract = (O::family().start)(replay.format);
        let fields = (O::family().fields.expect("field-driven yards"))().unwrap();
        replay
            .read_block_changes(
                block,
                &fields,
                |step| {
                    match step {
                        BlockChange::Changed { frame, rebuilt, .. } => {
                            changed += 1;
                            let changes =
                                FrameChanges::rebuilt(frame, replay.format, changed > 1, |table| {
                                    rebuilt.contains(&table)
                                });
                            extract.observe(&replay, frame, &changes)?;
                        }
                        BlockChange::Quiet(hours) => {
                            quiet += hours.hours().len();
                            extract.quiet(&replay, hours, 0..hours.hours().len())?;
                        }
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        let (folded, reached) = dashboard_keyed::block::fold(
            scan,
            extract.finish().unwrap(),
            &replay,
            block,
            &mut |_, result| result.map(|_| true),
            &mut || Ok(()),
        )
        .unwrap();
        assert_eq!(folded, dashboard_base::prepared::prepass::Folded::Fast);
        assert_eq!(reached, Some(block.last_hour));
    }
    (changed, quiet)
}

#[test]
fn s5_repairs_and_refits_share_the_management_projection_and_each_ship_join() {
    use dashboard_base::prepared::{prepass, query::observations};
    let mut frames = s5_management_frames(8, 8);
    if let TableView::Complete(TableRows::ShipModels(rows)) =
        &mut *frames[0].tables[TableId::ShipModels.index()]
    {
        rows[0].counter = None;
    }
    let mut independent = frames[0].clone();
    independent.tables = independent.tables.map(|view| (*view).clone().into());
    let source = source(frames.clone());
    let mut repairs = RepairsObserver::default();
    let mut refits = RefitsObserver::default();
    for observer in [&mut repairs.state, &mut refits.state] {
        observer.bind(&source);
    }
    let projects = prepass::noted("naval-yards.project");
    let ships = prepass::noted("naval-yards.ship");
    let mut repair_values = Values::default();
    let mut refit_values = Values::default();
    repairs
        .observe(FormatVersion::CURRENT, &frames[0], &mut repair_values)
        .unwrap();
    refits
        .observe(FormatVersion::CURRENT, &independent, &mut refit_values)
        .unwrap();
    assert_eq!(prepass::noted("naval-yards.project") - projects, 1);
    assert_eq!(prepass::noted("naval-yards.ship") - ships, 1);
    let (retained, budget, computations) = observations::usage(&source, yards::management::CACHE);
    assert_eq!(computations, 1);
    assert!(retained > 0 && retained <= budget);
    for (values, state) in [
        (&repair_values, &repairs.state),
        (&refit_values, &refits.state),
    ] {
        let refusals: BTreeMap<_, _> = values
            .iter()
            .filter_map(|(key, amount)| {
                let (_, id) = key.split_once("|refusal|")?;
                match &state.tokens[id.parse::<usize>().unwrap()] {
                    yards::Operand::Refusal(why) => Some((why.as_str(), amount)),
                    _ => None,
                }
            })
            .collect();
        assert_eq!(refusals["Maximum ship hull not recorded"], 1);
    }
    let repairs: BTreeMap<_, _> = repair_values.iter().collect();
    let refits: BTreeMap<_, _> = refit_values.iter().collect();
    assert_eq!(
        repairs[format!("{COUNTRY}|9|ship|{SHIP}|hull").as_str()],
        123_000
    );
    assert_eq!(repairs[format!("{COUNTRY}|total|queueEntries").as_str()], 2);
    assert_eq!(
        refits[format!("{COUNTRY}|{LINE}|ship|{SHIP}|hull").as_str()],
        123_000
    );
    assert_eq!(
        refits[format!("{COUNTRY}|{LINE}|progress").as_str()],
        123_456
    );
}

#[test]
fn s5_field_driven_quiet_spans_and_equal_sibling_dates_match_hourly_walks() {
    fn check<O: Observer>() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, prepass};
        let mut frames = s5_management_frames(7, 33);
        for frame in &mut frames {
            // Sailing and organisation changes are not repair/refit operands.
            if let TableView::Complete(TableRows::TaskForces(rows)) =
                &mut *frame.tables[TableId::TaskForces.index()]
            {
                rows[0].province = frame.hour as u32;
            }
            if let TableView::Complete(TableRows::Ships(rows)) =
                &mut *frame.tables[TableId::Ships.index()]
            {
                rows[0].organisation = i64::from(frame.hour);
            }
            if let TableView::Complete(TableRows::ReservedCountryNaming(rows)) =
                &mut *frame.tables[TableId::ReservedCountryNaming.index()]
            {
                rows[0].built = (frame.hour % 8) as u8;
            }
            if frame.hour == 7 {
                frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
            }
            // These timed siblings are not even among the selected tables.
            if frame.hour == 10 {
                frame.metrics.table_ns[TableId::CountryProductionModifiers.index()] = 1;
            }
            if frame.hour == 16 {
                frame.metrics.table_ns[TableId::VariantModules.index()] = 1;
            }
        }
        let source = source(frames.clone());
        let mut direct = Scan::<O>::default();
        direct.observer.bind(&source, None, &mut || Ok(())).unwrap();
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames {
            let changes = tracker.observe(frame, FormatVersion::CURRENT);
            direct
                .prepare_changed(FormatVersion::CURRENT, frame, &changes)
                .unwrap();
        }
        let fixture = Fixture::new();
        flush(&fixture, &mut direct);
        let expected = window::<O>(&fixture, 7, 33);
        let mut folded = Scan::<O>::default();
        let before = prepass::noted("naval-yards.project");
        let (changed, quiet) = s5_fold(&source, &mut folded);
        assert_eq!(changed, 1);
        assert_eq!(quiet, 26);
        // The cached numeric projection is reused even at equal supplying reads.
        assert!(prepass::noted("naval-yards.project") - before <= 1);
        let fixture = Fixture::new();
        flush(&fixture, &mut folded);
        let actual = window::<O>(&fixture, 7, 33);
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(actual.reads.len(), 27);
        assert_eq!(
            runs(&actual, &format!("{COUNTRY}|read|VariantDefinitions")),
            [(16, 21, "16".into(), true), (22, 33, "22".into(), false),]
        );
        let dates = runs(&actual, &format!("{COUNTRY}|read|ProductionLines"));
        if O::NAME == RefitsObserver::NAME {
            assert_eq!(
                dates,
                [
                    (7, 7, "7".into(), true),
                    (8, 9, "8".into(), false),
                    (10, 31, "10".into(), false),
                    (32, 33, "32".into(), false),
                ]
            );
        } else {
            assert!(dates.is_empty());
        }
    }
    check::<RepairsObserver>();
    check::<RefitsObserver>();
}

#[test]
fn s5_sibling_dates_survive_quiet_flush_restore_and_reset_after_a_recorded_gap() {
    let mut frames = s5_management_frames(7, 24);
    frames.retain(|frame| frame.hour != 23);
    let mut captured = test_fixtures::records(1, frames.clone());
    captured.insert(
        captured.len() - 1,
        replay::spool::Record::Gap(replay_frame::branches::Gap {
            branch: 1,
            first_hour: 23,
            last_hour: 23,
            reason: "Synthetic missing yard hour".into(),
        }),
    );
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "yards-gap-synthetic".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(
            replay::container::Replay::from_records(test_fixtures::CAMPAIGN.into(), &[captured])
                .unwrap(),
        ),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut scan = Scan::<RefitsObserver>::default();
    scan.observer.bind(&source, None, &mut || Ok(())).unwrap();
    scan.prepare(FormatVersion::CURRENT, &frames[1], true)
        .unwrap();
    scan.advance_quiet(9, true).unwrap();
    let mut fixture = Fixture::new();
    flush(&fixture, &mut scan);
    let mut scan: Scan<RefitsObserver> =
        serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
    scan.observer.bind(&source, None, &mut || Ok(())).unwrap();
    for hour in 10..22 {
        scan.advance_quiet(hour, true).unwrap();
    }
    scan.prepare(FormatVersion::CURRENT, &frames[15], true)
        .unwrap();
    let mut writer = fixture.append_writer();
    scan.flush(&mut writer, 1, &AtomicBool::new(false)).unwrap();
    writer.commit().unwrap();
    assert_eq!(
        runs(
            &window::<RefitsObserver>(&fixture, 8, 22),
            &format!("{COUNTRY}|read|ProductionLines")
        ),
        [(8, 22, "8".into(), true),]
    );
    // Rebinding a restored scan must not carry a read across the recorded gap.
    let mut after_gap: Scan<RefitsObserver> =
        serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
    after_gap
        .observer
        .bind(&source, None, &mut || Ok(()))
        .unwrap();
    after_gap
        .prepare(FormatVersion::CURRENT, frames.last().unwrap(), true)
        .unwrap();
    let fixture = Fixture::new();
    flush(&fixture, &mut after_gap);
    assert!(
        runs(
            &window::<RefitsObserver>(&fixture, 24, 24),
            &format!("{COUNTRY}|read|ProductionLines")
        )
        .is_empty()
    );
}

#[test]
fn s5_projection_invalidates_changed_operands_and_preserves_supplier_local_unknowns() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, query::observations};
    let mut frames = s5_management_frames(8, 12);
    for frame in &mut frames[2..] {
        if let TableView::Complete(TableRows::Ships(rows)) =
            &mut *frame.tables[TableId::Ships.index()]
        {
            rows[0].strength = 42_000;
        }
    }
    for frame in &mut frames[3..4] {
        missing(frame, TableId::ProductionLines);
    }
    let source = source(frames.clone());
    let mut repairs = Scan::<RepairsObserver>::default();
    let mut refits = Scan::<RefitsObserver>::default();
    repairs
        .observer
        .bind(&source, None, &mut || Ok(()))
        .unwrap();
    refits.observer.bind(&source, None, &mut || Ok(())).unwrap();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        let changes = tracker.observe(frame, FormatVersion::CURRENT);
        repairs
            .prepare_changed(FormatVersion::CURRENT, frame, &changes)
            .unwrap();
        refits
            .prepare_changed(FormatVersion::CURRENT, frame, &changes)
            .unwrap();
    }
    // Four shared snapshots and one completed block observation, not one
    // projection for each view.
    assert_eq!(observations::usage(&source, yards::management::CACHE).2, 5);
    let repair_store = Fixture::new();
    let refit_store = Fixture::new();
    flush(&repair_store, &mut repairs);
    flush(&refit_store, &mut refits);
    let repair = window::<RepairsObserver>(&repair_store, 8, 12);
    let refit = window::<RefitsObserver>(&refit_store, 8, 12);
    assert_eq!(
        runs(&repair, &format!("{COUNTRY}|9|ship|{SHIP}|hull")),
        [
            (8, 9, "123000".into(), true),
            (10, 12, "42000".into(), false),
        ]
    );
    assert_eq!(
        runs(&refit, &format!("{COUNTRY}|{LINE}|progress")),
        [(8, 12, "123456".into(), true),]
    );
    assert_eq!(
        runs(&refit, &format!("{COUNTRY}|known|ProductionLines")),
        [
            (8, 10, "1".into(), true),
            (11, 11, "0".into(), false),
            (12, 12, "1".into(), false),
        ]
    );
    assert_eq!(
        runs(&refit, &format!("{COUNTRY}|read|ProductionLines")),
        [(8, 10, "8".into(), true),]
    );
    assert_eq!(
        runs(&refit, &format!("{COUNTRY}|{LINE}|cost")),
        [
            (8, 10, "1000000".into(), true),
            (12, 12, "1000000".into(), true),
        ]
    );
}

#[test]
fn s5_unreadable_block_does_not_carry_supplier_dates_through_recorded_but_unobserved_hours() {
    let frames = s5_management_frames(8, 13);
    let mut replay = replay::container::Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(1, frames[..2].to_vec())],
    )
    .unwrap();
    for batch in [&frames[2..4], &frames[4..]] {
        replay = replay
            .append(&[test_fixtures::continuation_records(
                &replay,
                1,
                batch.to_vec(),
            )])
            .unwrap();
    }
    assert_eq!(replay.index.len(), 3);
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "yards-unreadable-synthetic".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut scan = Scan::<RefitsObserver>::default();
    scan.observer.bind(&source, None, &mut || Ok(())).unwrap();
    scan.prepare(FormatVersion::CURRENT, &frames[0], true)
        .unwrap();
    scan.prepare(FormatVersion::CURRENT, &frames[1], true)
        .unwrap();
    scan.unreadable_block();
    scan.prepare(FormatVersion::CURRENT, &frames[4], true)
        .unwrap();
    let fixture = Fixture::new();
    flush(&fixture, &mut scan);
    let observed = window::<RefitsObserver>(&fixture, 8, 12);
    assert_eq!(
        runs(&observed, &format!("{COUNTRY}|read|ProductionLines")),
        [(8, 9, "8".into(), true),]
    );
    assert!(
        runs(&observed, &format!("{COUNTRY}|{LINE}|progress"))
            .iter()
            .any(|run| run.0 == 12)
    );
}

#[test]
fn s5_sequential_fallback_preserves_sibling_dates_across_block_boundaries() {
    let mut frames = s5_management_frames(7, 13);
    frames[3].metrics.table_ns[TableId::CountryProductionModifiers.index()] = 1;
    let mut replay = replay::container::Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(1, frames[..3].to_vec())],
    )
    .unwrap();
    for batch in [&frames[3..5], &frames[5..]] {
        replay = replay
            .append(&[test_fixtures::continuation_records(
                &replay,
                1,
                batch.to_vec(),
            )])
            .unwrap();
    }
    let source = dashboard_base::Source::from_replay(
        dashboard_base::SourceKey {
            source_id: "yards-fallback-synthetic".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut scan = Scan::<RefitsObserver>::default();
    scan.observer.bind(&source, None, &mut || Ok(())).unwrap();
    let replay = source.replay();
    for block in &replay.index {
        assert_eq!(
            dashboard_keyed::block::observe_block(
                &mut scan,
                &replay,
                block,
                None,
                &mut |_, result| result.map(|_| true),
                &mut || Ok(()),
            )
            .unwrap(),
            Some(block.last_hour)
        );
    }
    let fixture = Fixture::new();
    flush(&fixture, &mut scan);
    let observed = window::<RefitsObserver>(&fixture, 7, 13);
    assert_eq!(
        runs(&observed, &format!("{COUNTRY}|read|ProductionLines")),
        [(8, 9, "8".into(), true), (10, 13, "10".into(), false),]
    );
}

#[test]
fn s5_separate_family_folds_share_every_changed_projection_in_a_block() {
    use dashboard_base::prepared::prepass;
    let mut frames = s5_management_frames(8, 12);
    for frame in &mut frames[2..] {
        if let TableView::Complete(TableRows::Ships(rows)) =
            &mut *frame.tables[TableId::Ships.index()]
        {
            rows[0].strength = 42_000;
        }
    }
    if let TableView::Complete(TableRows::NavalRefitProgress(rows)) =
        &mut *frames[4].tables[TableId::NavalRefitProgress.index()]
    {
        rows[0].progress_raw += 1;
    }
    let source = source(frames);
    assert_eq!(source.replay().index.len(), 1);
    let before = prepass::noted("naval-yards.project");
    let mut repairs = Scan::<RepairsObserver>::default();
    let mut refits = Scan::<RefitsObserver>::default();
    assert_eq!(s5_fold(&source, &mut repairs), (3, 2));
    assert_eq!(prepass::noted("naval-yards.project") - before, 3);
    assert_eq!(s5_fold(&source, &mut refits), (3, 2));
    assert_eq!(prepass::noted("naval-yards.project") - before, 3);
}
