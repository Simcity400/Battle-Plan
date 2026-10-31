use crate::products::{
    naval_refits, naval_repairs,
    yards::{
        self,
        layout::{self, ship},
        project::{Key, Projector, Spelled},
        segment::{Refits, Repairs, View},
    },
};
use dashboard_base::{products::Product, test_fixtures};
use replay_data::rows::*;
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
    for &table in yards::management::TABLES {
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
    // A design creator that is not dynamic is named by its own tag.
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: 11,
            ..Default::default()
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
/// Every value `frame`'s view holds whole, by the observer's name for it: `TAG@slot|...`; a
/// label as the number of its operand among those returned, a refusal's reason too.
fn named(frame: &DecodedFrame, refits: bool) -> (BTreeMap<String, i128>, Vec<yards::Operand>) {
    let (_, rows) = Projector::build(frame, refits).unwrap().unwrap();
    let (mut values, mut operands) = (BTreeMap::new(), Vec::new());
    let integer = |cell: &Spelled| match cell {
        Spelled::Integer(value) => Some(*value),
        _ => None,
    };
    let list = |cell: &Spelled| match cell {
        Spelled::List(cells) => cells.clone(),
        _ => Vec::new(),
    };
    for row in rows.rows {
        let owner = &rows.owners[row.owner as usize];
        let cells = row.cells.unwrap();
        let mut number = |operand: &yards::Operand| {
            operands.push(operand.clone());
            (operands.len() - 1) as i128
        };
        let prefix = match (&row.key, row.stream) {
            (Key::Token(why), _) => {
                let reason = number(&why.operand);
                values.insert(format!("{owner}|refusal|{reason}"), integer(&cells[0]).unwrap());
                continue;
            }
            (Key::Number(_), layout::TOTALS) => format!("{owner}|total"),
            (Key::Number(key), layout::HOLDERS) => format!("{owner}|{key}"),
            (Key::Number(key), _) => {
                let Spelled::Unsigned(holder) = cells[ship::HOLDER - 2] else {
                    panic!("a ship's row names its holder");
                };
                format!("{owner}|{holder}|ship|{key}")
            }
        };
        if row.stream == layout::SHIPS || (row.stream == layout::HOLDERS && !refits) {
            values.insert(format!("{prefix}|present"), 1);
        }
        for &(at, name) in yards::at::measures(row.stream, refits) {
            let value = match &cells[at - 2] {
                Spelled::Null | Spelled::List(_) => continue,
                Spelled::Integer(value) => *value,
                Spelled::Unsigned(value) => i128::from(*value),
                Spelled::Token(label) => number(&label.operand),
            };
            values.insert(format!("{prefix}|{name}"), value);
        }
        if row.stream == layout::HOLDERS && !refits {
            for pair in list(&cells[layout::port::QUEUE - 2]).chunks(2) {
                let ordinal = integer(&pair[0]).unwrap();
                values.insert(format!("{prefix}|queue|{ordinal}"), integer(&pair[1]).unwrap());
            }
        }
        if row.stream == layout::SHIPS {
            for child in list(&cells[ship::CHILDREN - 2]).chunks(3) {
                let ordinal = integer(&child[0]).unwrap();
                if let Some(handle) = integer(&child[1]) {
                    values.insert(format!("{prefix}|child:{ordinal}"), handle);
                }
                if let Some(resolves) = integer(&child[2]) {
                    values.insert(format!("{prefix}|childResolves:{ordinal}"), resolves);
                }
            }
        }
    }
    (values, operands)
}

/// Every name `frame`'s view `V` reads whole, and its value.
fn values<V: View>(frame: &DecodedFrame) -> BTreeMap<String, i128> {
    named(frame, V::REFITS).0
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
    let read = values::<Repairs>(&frame);
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
    // The game floors a model's maximum at 100 (0.001 HP).
    let mut low = frame.clone();
    if let TableView::Complete(TableRows::ShipModels(rows)) =
        &mut *low.tables[TableId::ShipModels.index()]
    {
        rows[0].counter.as_mut().unwrap().maximum_strength = 0;
    }
    assert_eq!(
        values::<Repairs>(&low)[&format!("{ship}|maximum")],
        100
    );
    missing(&mut frame, TableId::ShipModels);
    missing(&mut frame, TableId::VariantDefinitions);
    let read = values::<Repairs>(&frame);
    assert_eq!(read[&format!("{ship}|hull")], 123_000);
    assert!(!read.contains_key(&format!("{ship}|maximum")));
    assert_eq!(read[&format!("{COUNTRY}|total|dockyards")], 64);
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
    let read = values::<Repairs>(&frame);
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
    let read = values::<Repairs>(&frame);
    assert_eq!(read[&format!("{key}|rejoinResolves")], 1);
    assert_eq!(read[&format!("{key}|childResolves:0")], 1);
    missing(&mut frame, TableId::TaskForces);
    let read = values::<Repairs>(&frame);
    assert!(read.contains_key(&format!("{key}|rejoin")));
    assert!(!read.contains_key(&format!("{key}|rejoinResolves")));
}

#[test]
fn refit_liveness_is_hourly_and_full_reference_not_stale_daily_metadata() {
    let mut frame = refit_frame(7);
    let key = format!("{COUNTRY}|{LINE}");
    let read = values::<Refits>(&frame);
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
        let read = values::<Refits>(&frame);
        assert_eq!(read[&format!("{COUNTRY}|total|ships")], 0);
        assert!(!read.contains_key(&format!("{key}|progress")));
        assert!(!read.contains_key(&format!("{key}|cost")));
    }
    missing(&mut frame, TableId::NavalRefitProgress);
    let read = values::<Refits>(&frame);
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
    let read = values::<Refits>(&frame);
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
        let read = values::<Refits>(&frame);
        assert_eq!(read[&format!("{key}|cost")], i128::from(cost));
        assert!(!read.contains_key(&format!("{key}|remainingCostRaw")));
        assert!(!read.contains_key(&format!("{COUNTRY}|total|remainingCostRaw")));
    }
    missing(&mut frame, TableId::ProductionLines);
    let read = values::<Refits>(&frame);
    assert_eq!(read[&format!("{key}|progress")], -75);
    assert!(!read.contains_key(&format!("{COUNTRY}|total|dockyards")));
}

#[test]
fn product_contracts_validate_and_refuse_direct_loads() {
    let params = yards::at::YardsAtParams {
        branch_id: "branch".into(),
        tag: "MOD".into(),
        hour: 10,
    };
    assert!(params.validate().is_err());
    let source = source(vec![frame(1)]);
    let valid = yards::at::YardsAtParams {
        branch_id: "1".into(),
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
    for (frame, key, refits) in [(&repair, &repair_key, false), (&refit, &refit_key, true)] {
        let (read, operands) = named(frame, refits);
        assert!(read.contains_key(&format!("{key}|name")));
        assert!(!read.contains_key(&format!("{key}|nameRebuilt")));
        assert!(
            matches!(&operands[read[&format!("{key}|name")] as usize], yards::Operand::ShipCache { bytes, .. } if bytes == b"Recorded cache")
        );
    }
    for frame in [&mut repair, &mut refit] {
        if let TableView::Complete(TableRows::Ships(rows)) =
            &mut *frame.tables[TableId::Ships.index()]
        {
            rows[0].name = None.into();
        }
    }
    for (frame, key, refits) in [(&repair, &repair_key, false), (&refit, &refit_key, true)] {
        let (read, operands) = named(frame, refits);
        assert!(!read.contains_key(&format!("{key}|name")));
        let id = read[&format!("{key}|nameRebuilt")] as usize;
        assert!(
            matches!(&operands[id], yards::Operand::ShipRebuilt { group: Ok(Some(key)), .. } if key == b"NAMING_GROUP")
        );
        assert_eq!(read[&format!("{key}|hull")], 123_000);
    }
    let repairs = values::<Repairs>(&repair);
    let refits = values::<Refits>(&refit);
    assert!(repairs.contains_key(&format!("{repair_key}|nameRebuilt")));
    assert!(refits.contains_key(&format!("{refit_key}|nameRebuilt")));
    if let TableView::Complete(TableRows::Ships(rows)) = &mut *repair.tables[TableId::Ships.index()]
    {
        rows[0].name = Some(Vec::new()).into();
    }
    let read = values::<Repairs>(&repair);
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
    let (_, operands) = named(&repair, false);
    assert!(operands.iter().any(|operand| matches!(operand, yards::Operand::ShipRebuilt { group: Ok(Some(key)), .. } if key == b"HULL_GROUP")));
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
    let (_, operands) = named(&repair, false);
    assert!(operands.iter().any(|operand| matches!(operand, yards::Operand::ShipRebuilt { group: Err(why), .. } if why.contains("90"))));
}

mod changes;
pub(crate) mod rows;
