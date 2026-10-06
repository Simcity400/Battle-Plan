use super::*;
use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{
        CountryLinkRow, CountryRow, LexicalTokenRow, ProductionLineOutputRow, ProductionLineRow,
        TableId, TableRows, VariantDefinitionRow,
    },
};

fn hour(day: i32) -> i32 {
    let replay::cadence::Cadence::Daily(slot) =
        replay::cadence::cadence(TableId::ProductionLineOutput)
    else {
        panic!("output is daily");
    };
    day * replay::cadence::STAGGER_PERIOD + i32::try_from(slot).unwrap()
}

fn line(handle: u64, variant: u64, cost_raw: i64) -> ProductionLineRow {
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
    ProductionLineOutputRow {
        line,
        variant,
        country: 1,
        units,
        since_hour: hour(0),
        exact: true,
        ..Default::default()
    }
}

fn day(
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

fn units() -> installed_content::equipment::Database {
    installed_content::equipment::fixture::load(&[(
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
    reply(
        scan,
        &params(refits),
        Named {
            units: Some(&units()),
            texts: None,
        },
        Vec::new(),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn at(runs: &[ProductionSpendingRun], hour: i32) -> Option<&ProductionSpendingRun> {
    runs.iter()
        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
}

#[test]
fn production_spending_exact_units_are_priced_once_at_each_read() {
    let mut frames = day(0, vec![line(1, 1, 100_001)], Some(vec![output(1, 1, 2)]));
    frames.extend(day(
        1,
        vec![line(1, 1, 250_003)],
        Some(vec![output(1, 1, 3)]),
    ));
    frames.extend(day(
        2,
        vec![line(1, 1, 900_000)],
        Some(vec![output(1, 1, 3)]),
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
    use replay::{branches::Gap, spool::Record};
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
    let mut frames = day(0, vec![line(1, 1, 100_000)], Some(vec![output(1, 1, 2)]));
    frames.extend(day(1, vec![line(1, 1, 100_000)], None));
    frames.extend(day(
        2,
        vec![line(1, 1, 100_000)],
        Some(vec![output(1, 1, 3)]),
    ));
    let result = render_result(&scan(frames), false);
    assert!(at(&result.total, hour(1)).is_none());
    assert_eq!(
        at(&result.total, hour(2)).unwrap().reason,
        Some(ProductionSpendingUnknown::MissedPublication)
    );
}

#[test]
fn production_spending_earlier_unread_precedes_a_later_explicit_gap() {
    use replay::{branches::Gap, spool::Record};
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
fn production_spending_deleted_line_keeps_deliveries_but_new_unpriced_delivery_is_unknown() {
    let mut frames = day(0, vec![line(1, 1, 200_000)], Some(vec![output(1, 1, 2)]));
    let mut ended = output(1, 1, 2);
    ended.ended = true;
    frames.extend(day(1, vec![], Some(vec![ended.clone()])));
    frames.extend(day(2, vec![], Some(vec![])));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(2)).unwrap().total_raw.as_deref(),
        Some("400000")
    );
    let mut frames = day(0, vec![line(1, 1, 200_000)], Some(vec![output(1, 1, 2)]));
    ended.units = 3;
    frames.extend(day(1, vec![], Some(vec![ended])));
    let result = render_result(&scan(frames), false);
    assert_eq!(
        at(&result.total, hour(1)).unwrap().reason,
        Some(ProductionSpendingUnknown::MissingCost)
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
fn production_spending_mod_content_domains_and_refit_naval_railway_rules_are_shared() {
    let mut lines = vec![
        line(1, 1, 100_000),
        line(2, 2, 200_000),
        line(3, 3, 300_000),
        line(4, 4, 400_000),
        line(5, 3, 500_000),
    ];
    lines[2].kind = engine::cequipment_production_line::KIND_NAVAL;
    lines[3].kind = engine::cequipment_production_line::KIND_RAILWAY_GUN;
    lines[4].kind = engine::cequipment_production_line::KIND_SHIP_REFIT;
    let outputs = lines
        .iter()
        .map(|line| output(line.handle, line.variant, 1))
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
fn production_spending_cost_requires_matching_design_and_mio_at_this_read() {
    for (variant, mio) in [(2, 0), (1, 99)] {
        let mut census = line(1, variant, 100_000);
        census.mio = mio;
        let result = render_result(
            &scan(day(0, vec![census], Some(vec![output(1, 1, 1)]))),
            false,
        );
        assert_eq!(
            result.total[0].reason,
            Some(ProductionSpendingUnknown::MissingCost)
        );
    }
    for (cost, expected) in [
        (0, None),
        (-1, Some(ProductionSpendingUnknown::InvalidCost)),
    ] {
        let result = render_result(
            &scan(day(0, vec![line(1, 1, cost)], Some(vec![output(1, 1, 1)]))),
            false,
        );
        assert_eq!(result.total[0].reason, expected);
        if cost == 0 {
            assert_eq!(result.total[0].total_raw.as_deref(), Some("0"));
        }
    }
}

#[test]
fn production_spending_refuses_integer_overflow_instead_of_saturating() {
    let lines = (1..=3).map(|id| line(id, 1, i64::MAX)).collect();
    let outputs = (1..=3).map(|id| output(id, 1, i64::MAX)).collect();
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, day(0, lines, Some(outputs)))]).unwrap();
    assert!(
        production::scan(&replay, 1, &AtomicBool::new(false))
            .unwrap_err()
            .to_string()
            .contains("exact integer range")
    );
}

#[test]
fn production_spending_every_country_5000_country_builder() {
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

#[test]
fn production_spending_many_retired_designs_have_sparse_runs_over_long_history() {
    use production::{TagHistory, Total};
    let mut scanned = ScannedProduced::default();
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
    scanned.tags.insert("opaque|tag".into(), track);
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
