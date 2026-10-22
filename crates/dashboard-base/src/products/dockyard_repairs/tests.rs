use super::*;
use crate::{
    prepared::{
        frame_changes::FrameChangeTracker,
        series::{Run, tests::Fixture},
    },
    test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag},
};
use engine_executable::modifier_keys;
use replay_data::rows::{
    CountryProductionModifiersRow, ModifierBreakdownNode, NavalRepairCountryRow,
    ProductionModifierTree, TableRows,
};
use replay_frame::frame::{FrameDecoder, FrameEncoder, TableView};
use std::sync::Arc;

fn tag() -> String {
    String::from_utf8(scale_tag(0)).unwrap()
}

fn modifiers(country: u32, power_raw: i64, bonus: i64) -> CountryProductionModifiersRow {
    CountryProductionModifiersRow {
        country,
        power_raw,
        factory_speed: [200_000, 400_000, 600_000, 1_000_000],
        trees: engine_production::production_modifiers::KEYS
            .into_iter()
            .map(|key| ProductionModifierTree {
                key,
                nodes: vec![ModifierBreakdownNode {
                    children: 0,
                    value: if key == modifier_keys::INDUSTRIAL_CAPACITY_DOCKYARD {
                        bonus
                    } else {
                        0
                    },
                    name: b"Synthetic source".to_vec(),
                }],
                failure: Vec::new(),
            })
            .collect(),
        ..Default::default()
    }
}

fn frame(
    hour: i32,
    count: i32,
    output: Option<CountryProductionModifiersRow>,
    read: bool,
) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, 1);
    *frame.tables[TableId::NavalRepairCountries.index()] =
        TableView::Complete(TableRows::NavalRepairCountries(vec![
            NavalRepairCountryRow {
                country: 100_000,
                num_used_dockyards: count,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryProductionModifiers.index()] = TableView::Complete(
        TableRows::CountryProductionModifiers(output.into_iter().collect()),
    );
    frame.metrics.table_ns[TableId::CountryProductionModifiers.index()] = u64::from(read);
    frame
}

fn observe(prepared: &mut Prepared, replay: &Replay, frames: &[DecodedFrame]) {
    let mut encoder = FrameEncoder::default();
    let mut decoder = FrameDecoder::default();
    let mut tracker = FrameChangeTracker::new();
    for (index, input) in frames.iter().enumerate() {
        let raw = Arc::new(encoder.encode(input.clone(), index == 0).unwrap());
        let frame = decoder.advance(&raw).unwrap();
        let changes = tracker.observe_updates(frame, raw, replay.format);
        prepared.observe(frame, replay, &changes).unwrap();
    }
}

fn prepared(frames: &[DecodedFrame]) -> (Fixture, Prepared) {
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let mut prepared = Prepared::default();
    observe(&mut prepared, &replay, frames);
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    (fixture, prepared)
}

#[test]
fn integrates_changed_use_and_output_partial_and_multiple_spans() {
    let frames: Vec<_> = (0..10)
        .map(|hour| {
            frame(
                hour,
                if hour < 3 { 2 } else { 3 },
                Some(modifiers(
                    100_000,
                    if hour < 5 { 100_000 } else { 50_000 },
                    if hour < 5 { 0 } else { 20_000 },
                )),
                hour == 0 || hour == 5,
            )
        })
        .collect();
    let (fixture, _) = prepared(&frames);
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(
        reads
            .integrals(&tag(), &[(0, 9), (2, 6), (6, 6), (7, 9)])
            .unwrap(),
        [Ok(25_200_000), Ok(13_280_000), Ok(2_640_000), Ok(7_920_000)]
    );
    assert_eq!(reads.origin(&tag()).unwrap(), Some(0));
    assert_eq!(reads.origin("unrecorded").unwrap(), None);
    assert_eq!(
        reads.integrals("unrecorded", &[(0, 9)]).unwrap(),
        [Err(ProductionSpendingUnknown::NotRecorded)]
    );
    assert!(reads.integrals(&tag(), &[]).unwrap().is_empty());
    assert!(reads.integrals(&tag(), &[(3, 2)]).is_err());
}

#[test]
fn zero_is_known_but_missing_negative_failed_and_unavailable_inputs_are_not() {
    let mut frames = vec![
        frame(0, 0, None, false),
        frame(1, 2, None, false),
        frame(2, 2, Some(modifiers(100_000, 50_000, 20_000)), true),
    ];
    let mut failed = modifiers(100_000, 100_000, 0);
    let tree = failed
        .trees
        .iter_mut()
        .find(|tree| tree.key == modifier_keys::INDUSTRIAL_CAPACITY_DOCKYARD)
        .unwrap();
    tree.nodes.clear();
    tree.failure = b"Synthetic failed tree".to_vec();
    frames.push(frame(3, 2, Some(failed), true));
    frames.push(frame(4, 2, Some(modifiers(100_000, 100_000, 0)), true));
    frames.push(frame(5, -1, Some(modifiers(100_000, 100_000, 0)), false));
    let mut missing = frame(6, 2, Some(modifiers(100_000, 100_000, 0)), false);
    *missing.tables[TableId::NavalRepairCountries.index()] =
        TableView::Complete(TableRows::NavalRepairCountries(Vec::new()));
    frames.push(missing);
    let mut unavailable = frame(7, 2, Some(modifiers(100_000, 100_000, 0)), false);
    *unavailable.tables[TableId::NavalRepairCountries.index()] = TableView::Unavailable {
        reason: "Synthetic repair refusal".into(),
    };
    frames.push(unavailable);
    let mut unavailable = frame(8, 2, None, true);
    *unavailable.tables[TableId::CountryProductionModifiers.index()] = TableView::Unavailable {
        reason: "Synthetic modifiers refusal".into(),
    };
    frames.push(unavailable);
    frames.push(frame(9, 2, Some(modifiers(100_000, 100_000, 0)), false));
    frames.push(frame(10, 2, Some(modifiers(100_000, 100_000, 0)), true));
    let mut zero = frame(11, 0, None, false);
    *zero.tables[TableId::CountryProductionModifiers.index()] = TableView::Unavailable {
        reason: "Synthetic modifiers refusal".into(),
    };
    frames.push(zero);
    let (fixture, _) = prepared(&frames);
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    let results = reads
        .integrals(
            &tag(),
            &(0..12).map(|hour| (hour, hour)).collect::<Vec<_>>(),
        )
        .unwrap();
    for (hour, expected) in [
        (0, Some(0)),
        (1, None),
        (2, Some(1_760_000)),
        (3, None),
        (4, Some(2_000_000)),
        (5, None),
        (6, None),
        (7, None),
        (8, None),
        (9, None),
        (10, Some(2_000_000)),
        (11, Some(0)),
    ] {
        assert_eq!(
            results[hour],
            expected.ok_or(ProductionSpendingUnknown::SpendingUnavailable),
            "hour {hour}"
        );
    }
    assert_eq!(reads.origin(&tag()).unwrap(), Some(0));
    assert_eq!(
        reads.integrals(&tag(), &[(0, 10)]).unwrap(),
        [Err(ProductionSpendingUnknown::SpendingUnavailable)]
    );
}

#[test]
fn opening_in_use_hours_before_the_first_daily_read_have_no_rate_or_origin() {
    let frames: Vec<_> = (0..10)
        .map(|hour| {
            frame(
                hour,
                2,
                (hour >= 8).then(|| modifiers(100_000, 100_000, 0)),
                hour == 8,
            )
        })
        .collect();
    let (fixture, _) = prepared(&frames);
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(reads.origin(&tag()).unwrap(), Some(8));
    assert_eq!(
        reads.integrals(&tag(), &[(0, 7), (0, 9), (8, 9)]).unwrap(),
        [
            Err(ProductionSpendingUnknown::SpendingUnavailable),
            Err(ProductionSpendingUnknown::SpendingUnavailable),
            Ok(4_000_000)
        ]
    );
    let (fixture, _) = prepared(&frames[..8]);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(reads.origin(&tag()).unwrap(), None);
}

#[test]
fn gaps_withdraw_daily_carry_and_holder_changes_break_equal_rates() {
    let mut frames: Vec<_> = [0, 1, 4, 5, 6]
        .into_iter()
        .map(|hour| {
            frame(
                hour,
                2,
                Some(modifiers(100_000, 100_000, 0)),
                hour == 0 || hour == 5,
            )
        })
        .collect();
    let changed = frames.last_mut().unwrap();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id = 100_001;
    }
    // The new native country and both source rows resolve through current links.
    *changed.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            replay_data::rows::CountryLinkRow {
                id: 100_000,
                canonical_slot: 0,
            },
            replay_data::rows::CountryLinkRow {
                id: 100_001,
                canonical_slot: 0,
            },
        ]));
    let (fixture, state) = prepared(&frames);
    let mut state: Prepared = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
    let mut next = frames.last().unwrap().clone();
    next.hour = 7;
    frames.push(next.clone());
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    observe(&mut state, &replay, &[next]);
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(
        reads
            .integrals(&tag(), &[(0, 5), (4, 4), (5, 6), (6, 6)])
            .unwrap(),
        [
            Err(ProductionSpendingUnknown::RecordingGap),
            Err(ProductionSpendingUnknown::SpendingUnavailable),
            Err(ProductionSpendingUnknown::CountryChanged),
            Ok(2_000_000)
        ]
    );
}

#[test]
fn aliases_join_but_duplicate_slots_and_missing_country_links_are_unknown() {
    let mut first = frame(0, 2, Some(modifiers(77, 100_000, 0)), true);
    *first.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            replay_data::rows::CountryLinkRow {
                id: 77,
                canonical_slot: 0,
            },
            replay_data::rows::CountryLinkRow {
                id: 100_000,
                canonical_slot: 0,
            },
        ]));
    let mut second = first.clone();
    second.hour = 1;
    if let TableView::Complete(TableRows::CountryProductionModifiers(rows)) =
        &mut *second.tables[TableId::CountryProductionModifiers.index()]
    {
        rows.push(modifiers(100_000, 100_000, 0));
    }
    let mut third = first.clone();
    third.hour = 2;
    *third.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "Synthetic missing links".into(),
    };
    let (fixture, _) = prepared(&[first, second, third]);
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(
        reads.integrals(&tag(), &[(0, 0), (1, 1), (2, 2)]).unwrap(),
        [
            Ok(2_000_000),
            Err(ProductionSpendingUnknown::SpendingUnavailable),
            Err(ProductionSpendingUnknown::SpendingUnavailable)
        ]
    );
}

#[test]
fn integral_arithmetic_is_checked_and_unrecorded_tables_have_no_values() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    prepared.keys.insert(tag(), 0);
    prepared.rates.push(0, Run::new(0, 1, i128::MAX, true));
    prepared.catalogue.recorded = true;
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert!(reads.integrals(&tag(), &[(0, 1)]).is_err());
    prepared.catalogue.recorded = false;
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(
        reads.integrals(&tag(), &[(0, 0)]).unwrap(),
        [Err(ProductionSpendingUnknown::NotRecorded)]
    );
}

#[test]
fn balance_and_repair_budget_deltas_do_not_visit_countries_or_publish_rates() {
    let first = frame(0, 2, Some(modifiers(100_000, 100_000, 0)), true);
    let mut second = first.clone();
    second.hour = 1;
    second.metrics.table_ns[TableId::CountryProductionModifiers.index()] = 0;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *second.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(42);
    }
    if let TableView::Complete(TableRows::NavalRepairCountries(rows)) =
        &mut *second.tables[TableId::NavalRepairCountries.index()]
    {
        rows[0].max_factories_for_repair = 10;
        rows[0].max_allowed_dockyards_to_repair = 20;
    }
    let (_, prepared) = prepared(&[first, second]);
    assert_eq!(prepared.country_visits, 1);
    // The residual of use that no readable base assignment reconciles is
    // observed unknown once and never sealed.
    assert_eq!(
        prepared.rates.key_visits, 3,
        "one observation and one flush seal for the total, one for the residual"
    );
}

fn split_put(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}

fn split_frame(hour: i32, count: i32, assigned: i32) -> DecodedFrame {
    use replay_data::rows::*;
    let mut observed = frame(hour, count, Some(modifiers(100_000, 100_000, 0)), hour == 0);
    split_put(
        &mut observed,
        TableRows::NavalBases(vec![NavalBaseRow {
            province: 7,
            country: 100_000,
            level: 10,
            ..Default::default()
        }]),
    );
    split_put(
        &mut observed,
        TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
            province: 7,
            allocated_dockyards: assigned,
            ..Default::default()
        }]),
    );
    split_put(
        &mut observed,
        TableRows::NavalRepairQueue(
            [1, 2, 2, 3]
                .into_iter()
                .enumerate()
                .map(|(ordinal, ship)| NavalRepairQueueRow {
                    province: 7,
                    ordinal: ordinal as u32,
                    ship,
                })
                .collect(),
        ),
    );
    split_put(
        &mut observed,
        TableRows::Ships(
            (1..=3)
                .map(|id| ShipRow {
                    id,
                    task_force: 8,
                    strength: if id == 2 { 100 } else { 50 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    split_put(
        &mut observed,
        TableRows::ShipModels(
            (1..=3)
                .map(|ship| ShipModelRow {
                    ship,
                    variant: if ship == 1 { 20 } else { 30 },
                    counter: Some(ShipCounterInputs {
                        maximum_strength: 100,
                        ..Default::default()
                    }),
                    has_criticals: Some(u32::from(ship == 2)).into(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    split_put(
        &mut observed,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 8,
            province: 7,
            roster_owner: 100_000,
            ..Default::default()
        }]),
    );
    split_put(
        &mut observed,
        TableRows::Provinces(vec![ProvinceRow {
            id: 7,
            controller: 100_000,
            ..Default::default()
        }]),
    );
    split_put(
        &mut observed,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 1,
            literal: b"synthetic_ship".to_vec(),
        }]),
    );
    split_put(
        &mut observed,
        TableRows::VariantDefinitions(
            [20, 30, 99]
                .into_iter()
                .map(|id| VariantDefinitionRow {
                    id,
                    equipment_token: 1,
                    creator_link: 100_000,
                    display_name: format!("Synthetic {id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    split_put(&mut observed, TableRows::VariantModules(vec![]));
    split_put(&mut observed, TableRows::VariantUpgrades(vec![]));
    split_put(&mut observed, TableRows::VariantChanges(vec![]));
    observed.metrics.table_ns[TableId::VariantDefinitions.index()] = u64::from(hour == 0);
    observed
}

#[test]
fn repair_split_parts_repeats_residual_new_design_zero_history_and_exact_span_sums() {
    let frames = [
        split_frame(0, 1, 1),
        split_frame(1, 3, 3),
        split_frame(2, 8, 8),
        split_frame(3, 0, 8),
    ];
    let (fixture, state) = prepared(&frames);
    assert_eq!(state.base_visits, 3, "zero use must not walk a queue");
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    let parts = reads.parts(&tag()).unwrap();
    assert_eq!(
        parts,
        [
            RepairPart::Unattributed,
            RepairPart::Design(20),
            RepairPart::Design(30)
        ]
    );
    let spans = [(0, 3), (0, 0), (1, 1), (1, 2), (3, 3)];
    assert_eq!(
        reads
            .part_integrals(&tag(), RepairPart::Design(30), &spans)
            .unwrap(),
        [Ok(5_000_000), Ok(0), Ok(2_000_000), Ok(5_000_000), Ok(0)]
    );
    let mut sum = vec![0; spans.len()];
    for part in parts {
        for (sum, value) in sum
            .iter_mut()
            .zip(reads.part_integrals(&tag(), part, &spans).unwrap())
        {
            *sum += value.unwrap();
        }
    }
    assert_eq!(
        reads.integrals(&tag(), &spans).unwrap(),
        sum.into_iter().map(Ok).collect::<Vec<_>>()
    );
    assert_eq!(
        reads
            .variants()
            .versions
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [20, 30]
    );
    assert_eq!(reads.variants().token(20), "synthetic_ship");
    assert_eq!(
        reads.variants().versions[&30][0].first_hour,
        0,
        "a later-selected design keeps its actual supplying date"
    );
}

#[test]
fn repair_split_refusals_missing_models_variants_and_prohibited_entries_stay_unattributed() {
    use replay_data::rows::*;
    let mut frames = vec![split_frame(0, 3, 3)];
    for hour in 1..=5 {
        let mut observed = split_frame(hour, 3, 3);
        match hour {
            1 => {
                observed.tables[TableId::NavalRepairQueue.index()] = TableView::Unavailable {
                    reason: "Synthetic refusal".into(),
                }
                .into()
            }
            2 => split_put(&mut observed, TableRows::ShipModels(vec![])),
            3 => {
                if let TableView::Complete(TableRows::ShipModels(rows)) =
                    &mut *observed.tables[TableId::ShipModels.index()]
                {
                    for row in rows {
                        row.variant = 0;
                    }
                }
            }
            4 => {
                if let TableView::Complete(TableRows::NavalBases(rows)) =
                    &mut *observed.tables[TableId::NavalBases.index()]
                {
                    rows[0].countries = vec![100_000];
                }
            }
            5 => {
                observed.tables[TableId::NavalBases.index()] = TableView::Unavailable {
                    reason: "Synthetic bases refusal".into(),
                }
                .into()
            }
            _ => unreachable!(),
        }
        frames.push(observed);
    }
    let (fixture, _) = prepared(&frames);
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(
        reads
            .part_integrals(&tag(), RepairPart::Unattributed, &[(0, 0), (1, 4), (5, 5)])
            .unwrap(),
        [
            Ok(0),
            Ok(12_000_000),
            // Bases that cannot be read cannot reconcile with the use.
            Err(ProductionSpendingUnknown::SpendingUnavailable)
        ]
    );
    for design in [20, 30] {
        assert_eq!(
            reads
                .part_integrals(&tag(), RepairPart::Design(design), &[(1, 4)])
                .unwrap(),
            [Ok(0)]
        );
    }
    assert_eq!(reads.integrals(&tag(), &[(5, 5)]).unwrap(), [Ok(3_000_000)]);
    assert_eq!(
        reads.allocation_mismatches(),
        0,
        "an unread table is no mismatch"
    );
}

#[test]
fn repair_split_overselection_counts_unidentified_variants_and_withdraws_only_parts() {
    use replay_data::rows::*;
    let mut over = split_frame(1, 2, 3);
    if let TableView::Complete(TableRows::ShipModels(rows)) =
        &mut *over.tables[TableId::ShipModels.index()]
    {
        rows[1].variant = 0;
    }
    let frames = [split_frame(0, 3, 3), over, split_frame(2, 2, 1)];
    let (fixture, _) = prepared(&frames);
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(reads.integrals(&tag(), &[(0, 2)]).unwrap(), [Ok(7_000_000)]);
    // Three assigned against two used, then one assigned against two used.
    assert_eq!(reads.allocation_mismatches(), 2);
    for part in reads.parts(&tag()).unwrap() {
        assert_eq!(
            reads
                .part_integrals(&tag(), part, &[(1, 1), (0, 2)])
                .unwrap(),
            [Err(ProductionSpendingUnknown::SpendingUnavailable); 2]
        );
    }
}

#[test]
fn repair_split_allied_recipient_is_charged_to_base_holder_and_output_changes_reuse_selection() {
    use replay_data::rows::*;
    let mut first = split_frame(0, 3, 3);
    let countries = scale_frame(1, 0, 2);
    first.tables[TableId::Countries.index()] = countries.tables[TableId::Countries.index()].clone();
    first.tables[TableId::CountryLinks.index()] =
        countries.tables[TableId::CountryLinks.index()].clone();
    split_put(
        &mut first,
        TableRows::NavalRepairCountries(vec![
            NavalRepairCountryRow {
                country: 100_000,
                num_used_dockyards: 3,
                ..Default::default()
            },
            NavalRepairCountryRow {
                country: 100_001,
                num_used_dockyards: 0,
                ..Default::default()
            },
        ]),
    );
    split_put(
        &mut first,
        TableRows::TaskForces(vec![TaskForceRow {
            id: 8,
            province: 7,
            roster_owner: 100_001,
            ..Default::default()
        }]),
    );
    split_put(
        &mut first,
        TableRows::RelationLinks(vec![RelationLinkRow {
            country: 100_001,
            target_index: 0,
            links: RelationLinkRow::DOCKING_RIGHTS,
            ..Default::default()
        }]),
    );
    split_put(&mut first, TableRows::FactionMembers(vec![]));
    split_put(&mut first, TableRows::WarRelations(vec![]));
    split_put(&mut first, TableRows::Subjects(vec![]));
    let mut second = first.clone();
    second.hour = 1;
    second.metrics.table_ns[TableId::VariantDefinitions.index()] = 0;
    split_put(
        &mut second,
        TableRows::CountryProductionModifiers(vec![modifiers(100_000, 50_000, 20_000)]),
    );
    let (fixture, state) = prepared(&[first, second]);
    assert_eq!(
        state.base_visits, 1,
        "changing output must not reselect queues"
    );
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    assert_eq!(
        reads
            .part_integrals(&tag(), RepairPart::Design(30), &[(0, 1)])
            .unwrap(),
        [Ok(3_760_000)]
    );
    let ally = String::from_utf8(scale_tag(1)).unwrap();
    assert_eq!(reads.parts(&ally).unwrap(), [RepairPart::Unattributed]);
    assert_eq!(reads.integrals(&ally, &[(0, 1)]).unwrap(), [Ok(0)]);
}

#[test]
fn repair_split_unknown_total_gaps_holders_and_late_designs_keep_refusals() {
    use replay_data::rows::*;
    let mut missing = split_frame(1, 3, 1);
    split_put(&mut missing, TableRows::CountryProductionModifiers(vec![]));
    let mut later = split_frame(2, 3, 3);
    later.metrics.table_ns[TableId::CountryProductionModifiers.index()] = 1;
    let mut gap = split_frame(5, 3, 3);
    gap.metrics.table_ns[TableId::CountryProductionModifiers.index()] = 1;
    let mut holder = split_frame(6, 3, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *holder.tables[TableId::Countries.index()]
    {
        rows[0].id = 100_001;
    }
    split_put(
        &mut holder,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 100_000,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: 100_001,
                canonical_slot: 0,
            },
        ]),
    );
    let (fixture, _) = prepared(&[split_frame(0, 3, 1), missing, later, gap, holder]);
    let flag = AtomicBool::new(false);
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    for part in reads.parts(&tag()).unwrap() {
        assert_eq!(
            reads
                .part_integrals(&tag(), part, &[(1, 1), (2, 5), (5, 6)])
                .unwrap(),
            [
                Err(ProductionSpendingUnknown::SpendingUnavailable),
                Err(ProductionSpendingUnknown::RecordingGap),
                Err(ProductionSpendingUnknown::CountryChanged),
            ]
        );
    }
    assert_eq!(
        reads
            .part_integrals(&tag(), RepairPart::Design(30), &[(0, 1)])
            .unwrap(),
        [Err(ProductionSpendingUnknown::SpendingUnavailable)],
        "no zero fill across a pre-design hole"
    );
}

#[test]
fn repair_split_churn_revisits_only_changed_and_using_holders() {
    use replay_data::rows::*;
    let mut first = split_frame(0, 1, 1);
    let population = scale_frame(1, 0, 24);
    first.tables[TableId::Countries.index()] =
        population.tables[TableId::Countries.index()].clone();
    first.tables[TableId::CountryLinks.index()] =
        population.tables[TableId::CountryLinks.index()].clone();
    let use_rows = |late: i32| {
        TableRows::NavalRepairCountries(
            (0..24)
                .map(|index| NavalRepairCountryRow {
                    country: 100_000 + index,
                    num_used_dockyards: match index {
                        0..3 => 1,
                        23 => late,
                        _ => 0,
                    },
                    ..Default::default()
                })
                .collect(),
        )
    };
    split_put(&mut first, use_rows(0));
    split_put(
        &mut first,
        TableRows::CountryProductionModifiers(
            (0..24)
                .map(|index| modifiers(100_000 + index, 100_000, 0))
                .collect(),
        ),
    );
    split_put(
        &mut first,
        TableRows::NavalBases(
            (1..=3)
                .map(|province| NavalBaseRow {
                    province,
                    country: 100_000 + province - 1,
                    level: 10,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    split_put(
        &mut first,
        TableRows::NavalRepairBases(
            (1..=3)
                .map(|province| NavalRepairBaseRow {
                    province,
                    allocated_dockyards: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    split_put(
        &mut first,
        TableRows::NavalRepairQueue(
            (1..=3)
                .map(|province| NavalRepairQueueRow {
                    province,
                    ordinal: 0,
                    ship: u64::from(province),
                })
                .collect(),
        ),
    );
    split_put(
        &mut first,
        TableRows::ShipModels(
            (1..=3_u64)
                .map(|ship| ShipModelRow {
                    ship,
                    variant: 20,
                    counter: Some(ShipCounterInputs {
                        maximum_strength: 100,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    split_put(
        &mut first,
        TableRows::TaskForces(
            (1..=3_u64)
                .map(|id| TaskForceRow {
                    id,
                    province: id as u32,
                    roster_owner: 100_000 + id as u32 - 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    split_put(
        &mut first,
        TableRows::Provinces(
            (1..=3)
                .map(|id| ProvinceRow {
                    id,
                    controller: 100_000 + id - 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    // Every hour a repairing ship's hull changes; from hour 25 a country
    // without bases starts using dockyards.
    let frames: Vec<_> = (0..50)
        .map(|hour| {
            let mut observed = first.clone();
            observed.hour = hour;
            observed.metrics.table_ns[TableId::CountryProductionModifiers.index()] =
                u64::from(hour == 0);
            observed.metrics.table_ns[TableId::VariantDefinitions.index()] = u64::from(hour == 0);
            split_put(
                &mut observed,
                TableRows::Ships(
                    (1..=3_u64)
                        .map(|id| ShipRow {
                            id,
                            task_force: id,
                            strength: if id == 1 {
                                50 + i64::from(hour % 3)
                            } else {
                                50
                            },
                            ..Default::default()
                        })
                        .collect(),
                ),
            );
            if hour >= 25 {
                split_put(&mut observed, use_rows(1));
            }
            observed
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let mut state = Prepared::default();
    observe(&mut state, &replay, &frames);
    // The first hour visits every holder; later hours only the three using
    // dockyards and, from hour 25, the one whose use changed and then uses.
    assert_eq!(state.country_visits, 24 + 24 * 3 + 25 * 4);
    assert_eq!(state.base_visits, 50 * 3);
    // Links, bases and access are indexed once, at the first hour: a
    // changing hour does no country-wide work.
    assert_eq!(state.joins.builds, 3);
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    let flag = AtomicBool::new(false);
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut reads = RepairReads::new(&fixture.store, &fixture.key, 1, &flag).unwrap();
    // Its use has no base assignment behind it: counted, total kept, split refused.
    assert_eq!(reads.allocation_mismatches(), 25);
    let late = String::from_utf8(scale_tag(23)).unwrap();
    assert_eq!(
        reads.integrals(&late, &[(25, 49)]).unwrap(),
        [Ok(25 * 1_000_000)]
    );
    assert_eq!(
        reads
            .part_integrals(&late, RepairPart::Unattributed, &[(25, 49)])
            .unwrap(),
        [Err(ProductionSpendingUnknown::SpendingUnavailable)]
    );
    // An unrelated holder kept its known zero through every partial hour.
    let quiet = String::from_utf8(scale_tag(12)).unwrap();
    assert_eq!(reads.integrals(&quiet, &[(0, 49)]).unwrap(), [Ok(0)]);
    assert_eq!(
        reads
            .part_integrals(&tag(), RepairPart::Design(20), &[(0, 49)])
            .unwrap(),
        [Ok(50 * 1_000_000)]
    );
}
