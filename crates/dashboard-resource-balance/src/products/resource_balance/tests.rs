use dashboard_core::test_fixtures::{CAMPAIGN, records, resource_frame};
use replay::campaign::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::{Arc, atomic::AtomicBool};

// regression: 0e43acf21d Remove shared and remaining economy data caps
#[test]
fn resource_history_keeps_every_run_from_the_recording() {
    use crate::products::resource_balance::scan;
    // 405,000 trade runs: past the former 400,000-run refusal.
    const DEALS: usize = 5_000;
    const READS: i32 = 81;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            (0..READS)
                .map(|hour| {
                    let deals: Vec<_> = (0..DEALS)
                        .map(|id| (id as u64, 21, 9001, i64::from(hour)))
                        .collect();
                    resource_frame(100 + hour, i64::from(hour), 9001, &deals)
                })
                .collect(),
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.trades.len(), DEALS * READS as usize);
    assert_eq!(scanned.series[&1].len(), READS as usize);
    for (read, trades) in scanned.trades.chunks(DEALS).enumerate() {
        assert!(trades.iter().all(|trade| {
            trade.first_hour == 100 + read as i32
                && trade.last_hour == trade.first_hour
                && trade.delivered_raw == read.to_string()
        }));
    }
}

#[test]
fn resource_values_run_over_recorded_hours_and_never_cross_gaps() {
    use crate::products::resource_balance::{TradeDirection, scan};
    let import = [(0x33, 21, 9001, 800_000)];
    let both = [(0x33, 21, 9001, 800_000), (0x44, 11, 31, 1_700_000)];
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, 30_100_000, 9001, &import),
                resource_frame(101, 30_100_000, 9001, &import),
                // Every table is read every hour, so a new value shows at once.
                resource_frame(102, 1, 9001, &import),
                resource_frame(103, 30_100_000, 9001, &both),
                resource_frame(104, 31_000_000, 9001, &both),
                // Hour 105 is missing: 106 starts anew.
                resource_frame(106, 31_000_000, 9001, &both),
                resource_frame(107, 31_000_000, 9001, &both),
                // Another country took the tag: its pools and deals start anew.
                // Italy's deal names Germany's old link, so it is not the new
                // Germany's.
                resource_frame(108, 31_000_000, 9002, &both),
                resource_frame(109, 31_000_000, 9002, &[]),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.resources,
        [(1, "steel".to_owned()), (2, "oil".to_owned())]
    );
    let steel: Vec<_> = scanned.series[&1]
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                run.produced_raw.as_str(),
            )
        })
        .collect();
    assert_eq!(
        steel,
        [
            (100, 101, true, "30100000"),
            (102, 102, false, "1"),
            (103, 103, false, "30100000"),
            (104, 104, false, "31000000"),
            (106, 107, true, "31000000"),
            (108, 109, true, "31000000"),
        ]
    );
    let run = &scanned.series[&1][0];
    assert_eq!(
        (
            run.available_raw.as_str(),
            run.after_projects_raw.as_str(),
            run.to_export_raw.as_str(),
            run.production_raw.as_str()
        ),
        ("28400000", "28360000", "2500000", "-3000000")
    );
    // Italy's rows are not Germany's.
    assert_eq!(scanned.series.len(), 2);
    let trades: Vec<_> = scanned
        .trades
        .iter()
        .map(|trade| {
            (
                trade.direction,
                trade.partner.as_str(),
                trade.first_hour,
                trade.last_hour,
                trade.starts_segment,
            )
        })
        .collect();
    assert_eq!(
        trades,
        [
            (TradeDirection::Import, "ITA", 100, 104, true),
            (TradeDirection::Export, "SPR", 103, 104, true),
            (TradeDirection::Import, "ITA", 106, 107, true),
            (TradeDirection::Export, "SPR", 106, 107, true),
            (TradeDirection::Export, "SPR", 108, 108, true),
        ]
    );
    assert_eq!(
        scanned
            .deal_hours
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 104), (106, 109)]
    );
    assert_eq!(scanned.trades[0].delivered_raw, "800000");
    assert_eq!(scanned.unnamed + scanned.unresolved, 0);
}

#[test]
fn resource_balance_replies_without_content_and_answers_again_from_the_cache() {
    use crate::products::resource_balance::{RESOURCE_DECIMALS, ResourceBalance};
    use dashboard_base::products::{CountryParams, Product};
    use dashboard_core::config::LoadContext;
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    resource_frame(100, 30_100_000, 9001, &[]),
                    resource_frame(101, 30_100_000, 9001, &[]),
                ],
            )],
        )
        .unwrap(),
    );
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "resources".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "GER".into(),
    };
    let load = |params: &CountryParams| {
        ResourceBalance::load(
            &source,
            params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reply = load(&params);
    assert_eq!(reply.decimals, RESOURCE_DECIMALS);
    // Without content, resources keep their tokens and have no names or icons.
    assert_eq!(
        reply
            .resources
            .iter()
            .map(|resource| (
                resource.token.as_str(),
                resource.name.is_none(),
                resource.icon.is_none()
            ))
            .collect::<Vec<_>>(),
        [("steel", true, true), ("oil", true, true)]
    );
    assert_eq!(reply.series[0].runs[0].last_hour, 101);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Resource names and icons are unavailable"))
    );
    assert_eq!(load(&params).series, reply.series);
    let other = load(&CountryParams {
        tag: "XXX".into(),
        ..params
    });
    assert!(other.series.is_empty());
    assert!(
        other
            .warnings
            .iter()
            .any(|warning| warning.starts_with("XXX:"))
    );
}

#[test]
fn resource_pools_and_deals_keep_their_own_values_when_the_other_table_is_missing() {
    use crate::products::resource_balance::scan;
    let deal = [(0x33, 21, 9001, 800_000)];
    let missing = |mut frame: DecodedFrame, table: TableId| {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        };
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, 30_100_000, 9001, &deal),
                // The deals were not captured at this hour: the pools still count,
                // and the deals are unknown, not none.
                missing(
                    resource_frame(101, 30_100_000, 9001, &deal),
                    TableId::ResourceExchanges,
                ),
                resource_frame(102, 30_100_000, 9001, &deal),
                // The pools were not captured at this hour: the deals still count.
                missing(
                    resource_frame(103, 30_100_000, 9001, &deal),
                    TableId::CountryResources,
                ),
                resource_frame(104, 30_100_000, 9001, &deal),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let spans = |spans: &[dashboard_base::products::country_groups::HourSpan]| {
        spans
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>()
    };
    assert_eq!(spans(&scanned.deal_hours), [(100, 100), (102, 104)]);
    let steel: Vec<_> = scanned.series[&1]
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
        .collect();
    assert_eq!(steel, [(100, 102, true), (104, 104, true)]);
    let deals: Vec<_> = scanned
        .trades
        .iter()
        .map(|trade| (trade.first_hour, trade.last_hour, trade.starts_segment))
        .collect();
    assert_eq!(deals, [(100, 100, true), (102, 104, true)]);
}

#[test]
fn a_repeated_resource_token_is_its_own_resource_and_never_blanks_the_others() {
    use crate::products::resource_balance::{ResourceBalance, ordinals};
    use dashboard_base::products::{CountryParams, Product};
    use dashboard_core::config::LoadContext;
    use replay_data::{rows::CountryResourceRow, rows::ResourceDefinitionRow};
    // Resource 3 repeats resource 1's token, as a second declaration in content would.
    let repeated = |hour| {
        let mut frame = resource_frame(hour, 30_100_000, 9001, &[]);
        if let TableView::Complete(TableRows::ResourceDefinitions(rows)) =
            &mut *frame.tables[TableId::ResourceDefinitions.index()]
        {
            rows.push(ResourceDefinitionRow {
                id: 3,
                token: 501,
                active: 1,
                factory_cost: 12_500,
            });
        }
        if let TableView::Complete(TableRows::CountryResources(rows)) =
            &mut *frame.tables[TableId::CountryResources.index()]
        {
            rows.insert(
                2,
                CountryResourceRow {
                    country: 11,
                    resource: 3,
                    produced: 700_000,
                    ..Default::default()
                },
            );
        }
        frame
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![repeated(100), repeated(101)])],
        )
        .unwrap(),
    );
    let source = dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "repeated".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ResourceBalance::load(
        &source,
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply
            .resources
            .iter()
            .map(|resource| (resource.id, resource.token.as_str(), resource.ordinal))
            .collect::<Vec<_>>(),
        [(1, "steel", 1), (2, "oil", 1), (3, "steel", 2)]
    );
    // Each keeps its own history; none overlaps another's.
    assert_eq!(
        reply
            .series
            .iter()
            .map(|series| (
                series.id,
                series.runs.len(),
                series.runs[0].produced_raw.as_str()
            ))
            .collect::<Vec<_>>(),
        [(1, 1, "30100000"), (2, 1, "0"), (3, 1, "700000")]
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("steel is declared 2 times"))
    );
    assert_eq!(
        ordinals(&[
            (1, "a".into()),
            (2, "b".into()),
            (5, "a".into()),
            (9, "a".into())
        ]),
        [1, 1, 2, 3]
    );
}
