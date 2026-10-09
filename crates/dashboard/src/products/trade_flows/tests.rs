use crate::test_fixtures::{CAMPAIGN, records, resource_frame, scale_frame};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{TableId, TableRows},
};
use std::sync::{Arc, atomic::AtomicBool};

/// Each flow as (seller, buyer, resource) tags and its runs as (first, last,
/// delivered, deals).
#[allow(clippy::type_complexity)]
fn flow_runs(
    countries: &[String],
    flows: &[crate::products::trade_flows::TradeFlow],
) -> Vec<((String, String, u32), Vec<(i32, i32, String, u32)>)> {
    flows
        .iter()
        .map(|flow| {
            (
                (
                    countries[flow.seller as usize].clone(),
                    countries[flow.buyer as usize].clone(),
                    flow.resource,
                ),
                (0..flow.first_hours.len())
                    .map(|run| {
                        (
                            flow.first_hours[run],
                            flow.last_hours[run],
                            flow.delivered_raw[run].clone(),
                            flow.deals[run],
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

fn span_pairs(spans: &[crate::products::country_groups::HourSpan]) -> Vec<(i32, i32)> {
    spans
        .iter()
        .map(|span| (span.first_hour, span.last_hour))
        .collect()
}

/// Italy sells Germany steel from hour 100 and Germany sells Spain steel from 103;
/// hour 105 is missing and Italy adds a second deal at 108.
fn trade_replay() -> Replay {
    let import = [(0x33, 21, 9001, 800_000)];
    let both = [(0x33, 21, 9001, 800_000), (0x44, 11, 31, 1_700_000)];
    let later = [
        (0x33, 21, 9001, 800_000),
        (0x44, 11, 31, 2_000_000),
        (0x55, 21, 9001, 100_000),
    ];
    Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, 30_100_000, 9001, &import),
                resource_frame(101, 30_100_000, 9001, &import),
                // Every hour is read: the changed deal shows at once.
                resource_frame(102, 30_100_000, 9001, &[(0x33, 21, 9001, 1)]),
                resource_frame(103, 30_100_000, 9001, &both),
                resource_frame(104, 30_100_000, 9001, &both),
                // Hour 105 is missing: 106 starts anew.
                resource_frame(106, 30_100_000, 9001, &both),
                resource_frame(107, 30_100_000, 9001, &both),
                resource_frame(108, 30_100_000, 9001, &later),
            ],
        )],
    )
    .unwrap()
}

#[test]
fn trade_flows_read_every_hour_and_never_cross_gaps() {
    use crate::products::trade_flows::scan;
    let scanned = scan(&trade_replay(), 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reads, [100, 101, 102, 103, 104, 106, 107, 108]);
    assert_eq!(span_pairs(&scanned.known), [(100, 104), (106, 108)]);
    let (countries, flows) = scanned.reply_flows();
    assert_eq!(countries, ["GER", "ITA", "SPR"]);
    let steel = |first, last, raw: &str, deals| (first, last, raw.to_owned(), deals);
    assert_eq!(
        flow_runs(&countries, &flows),
        [
            (
                ("GER".to_owned(), "SPR".to_owned(), 1),
                vec![
                    steel(103, 104, "1700000", 1),
                    steel(106, 107, "1700000", 1),
                    steel(108, 108, "2000000", 1),
                ]
            ),
            (
                ("ITA".to_owned(), "GER".to_owned(), 1),
                vec![
                    steel(100, 101, "800000", 1),
                    steel(102, 102, "1", 1),
                    steel(103, 104, "800000", 1),
                    steel(106, 107, "800000", 1),
                    // Two deals for the same seller, buyer and resource are one flow.
                    steel(108, 108, "900000", 2),
                ]
            ),
        ]
    );
    assert_eq!(flows[0].factories, [1, 1, 1]);
    assert_eq!(flows[1].factories, [1, 1, 1, 1, 2]);
    assert_eq!(scanned.unnamed + scanned.unresolved, 0);
}

#[test]
fn trade_flows_for_chosen_countries_keep_only_their_deals() {
    use crate::products::trade_flows::scan;
    let tags = ["SPR".to_owned()];
    let scanned = scan(&trade_replay(), 1, Some(&tags), &AtomicBool::new(false)).unwrap();
    let (countries, flows) = scanned.reply_flows();
    assert_eq!(countries, ["GER", "SPR"]);
    assert_eq!(flows.len(), 1);
    // What the deals are known for does not depend on who is asked about.
    assert_eq!(span_pairs(&scanned.known), [(100, 104), (106, 108)]);
}

#[test]
fn trade_flows_are_unknown_after_a_missing_table_and_count_what_they_cannot_name() {
    use crate::products::trade_flows::scan;
    let deal = [(0x33, 21, 9001, 800_000), (0x66, 777, 9001, 5)];
    let mut missing = resource_frame(101, 30_100_000, 9001, &deal);
    *missing.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let mut unlinked = resource_frame(103, 30_100_000, 9001, &deal);
    *unlinked.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, 30_100_000, 9001, &deal),
                // The deal table fails at this hour: the following hour is known.
                missing,
                resource_frame(102, 30_100_000, 9001, &deal),
                // A read whose links are unreadable cannot say who trades: unknown too.
                unlinked,
                resource_frame(104, 30_100_000, 9001, &deal),
                resource_frame(105, 30_100_000, 9001, &deal),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reads, [100, 102, 104, 105]);
    assert_eq!(
        span_pairs(&scanned.known),
        [(100, 100), (102, 102), (104, 105)]
    );
    let (countries, flows) = scanned.reply_flows();
    assert_eq!(
        flow_runs(&countries, &flows)[0].1,
        [
            (100, 100, "800000".to_owned(), 1),
            (102, 102, "800000".to_owned(), 1),
            (104, 105, "800000".to_owned(), 1)
        ]
    );
    // A seller the links do not name is counted at each read, never guessed.
    assert_eq!(scanned.unresolved, 4);
    // Those reads are known only in part.
    assert_eq!(
        scanned
            .partial
            .iter()
            .map(|span| (span.first_hour, span.last_hour, span.deals))
            .collect::<Vec<_>>(),
        [(100, 100, 1), (102, 102, 1), (104, 105, 1)]
    );
}

#[test]
fn trade_runs_need_country_tables_and_end_when_country_identity_changes() {
    use crate::products::trade_flows::scan;
    let deal = [(0x33, 21, 9001, 800_000)];
    // The country that takes the tag GER later is linked as 9002.
    let renamed = [(0x33, 21, 9002, 800_000)];
    let mut no_countries = resource_frame(101, 30_100_000, 9001, &deal);
    *no_countries.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, 30_100_000, 9001, &deal),
                // The deal table is recorded but who holds which tag is not known.
                no_countries,
                resource_frame(102, 30_100_000, 9001, &deal),
                resource_frame(103, 30_100_000, 9001, &deal),
                // Another country took the tag GER: its trade starts a new run.
                resource_frame(104, 30_100_000, 9002, &renamed),
                resource_frame(105, 30_100_000, 9002, &renamed),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(span_pairs(&scanned.known), [(100, 100), (102, 105)]);
    let (countries, flows) = scanned.reply_flows();
    assert_eq!(
        flow_runs(&countries, &flows)[0]
            .1
            .iter()
            .map(|run| (run.0, run.1))
            .collect::<Vec<_>>(),
        [(100, 100), (102, 103)]
    );
    assert_eq!(flows.len(), 2);
    assert_ne!(flows[0].buyer_identity, flows[1].buyer_identity);
    assert_eq!(flows[1].first_hours, [104]);
    assert_eq!(flows[1].last_hours, [105]);
}

#[test]
fn a_reply_longer_than_the_transport_holds_is_refused_with_what_to_do() {
    use crate::products::trade_flows::{TradeFlowsReply, fits_one_reply, scan};
    let scanned = scan(&trade_replay(), 1, None, &AtomicBool::new(false)).unwrap();
    let (countries, flows) = scanned.reply_flows();
    let reply = TradeFlowsReply {
        branch_id: "1".into(),
        decimals: 5,
        resources: Vec::new(),
        countries,
        country_identities: scanned.country_identities.clone(),
        reads: scanned.reads.clone(),
        known: scanned.known.clone(),
        partial: scanned.partial.clone(),
        flows,
        warnings: Vec::new(),
    };
    let bytes = serde_json::to_vec(&reply).unwrap().len();
    assert!(fits_one_reply(&reply, bytes).is_ok());
    // The complete reply is refused, never cut.
    let error = fits_one_reply(&reply, bytes - 1).expect_err("refused");
    assert!(
        error.to_string().contains("choose fewer countries"),
        "{error}"
    );
}

#[test]
fn hours_with_deals_they_cannot_attribute_are_known_only_in_part() {
    use crate::products::trade_flows::scan;
    let clean = [(0x33, 21, 9001, 800_000)];
    let unresolved = [
        (0x33, 21, 9001, 800_000),
        (0x66, 777, 9001, 5),
        (0x77, 21, 778, 5),
    ];
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, 30_100_000, 9001, &unresolved),
                resource_frame(101, 30_100_000, 9001, &unresolved),
                resource_frame(102, 30_100_000, 9001, &clean),
                resource_frame(103, 30_100_000, 9001, &clean),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(span_pairs(&scanned.known), [(100, 103)]);
    // Two deals are left out at hours 100 and 101; hour 102 is whole.
    assert_eq!(
        scanned
            .partial
            .iter()
            .map(|span| (span.first_hour, span.last_hour, span.deals))
            .collect::<Vec<_>>(),
        [(100, 101, 2)]
    );
}

#[test]
fn trade_flows_treat_tags_as_opaque_strings() {
    use crate::products::trade_flows::scan;
    // A mod's tags may hold any non-NUL bytes: spaces, separators, a dynamic tag.
    let retag = |mut frame: DecodedFrame| {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            for row in rows {
                row.tag = match row.tag.as_slice() {
                    b"ITA" => b"It aly".to_vec(),
                    b"SPR" => b"D01".to_vec(),
                    _ => b"G>E,R".to_vec(),
                };
            }
        }
        frame
    };
    let both = [(0x33, 21, 9001, 800_000), (0x44, 11, 31, 1_700_000)];
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![retag(resource_frame(100, 30_100_000, 9001, &both))],
        )],
    )
    .unwrap();
    let tags = ["It aly".to_owned()];
    let scanned = scan(&replay, 1, Some(&tags), &AtomicBool::new(false)).unwrap();
    let (countries, flows) = scanned.reply_flows();
    assert_eq!(countries, ["G>E,R", "It aly"]);
    assert_eq!(
        flow_runs(&countries, &flows)
            .into_iter()
            .map(|(key, _)| key)
            .collect::<Vec<_>>(),
        [("It aly".to_owned(), "G>E,R".to_owned(), 1)]
    );
    let scanned = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(scanned.reply_flows().0, ["D01", "G>E,R", "It aly"]);
}

/// `count` countries each selling the next one steel and oil, the amount changing
/// at every hourly read.
fn world_trade_frame(hour: i32, count: u32) -> DecodedFrame {
    use replay::rows::{LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow};
    let mut frame = scale_frame(1, hour, count);
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 501,
                literal: b"steel".to_vec(),
            },
            LexicalTokenRow {
                token: 502,
                literal: b"oil".to_vec(),
            },
        ]));
    *frame.tables[TableId::ResourceDefinitions.index()] =
        TableView::Complete(TableRows::ResourceDefinitions(
            [(1, 501), (2, 502)]
                .map(|(id, token)| ResourceDefinitionRow {
                    id,
                    token,
                    active: 1,
                    factory_cost: 12_500,
                })
                .to_vec(),
        ));
    *frame.tables[TableId::ResourceExchanges.index()] =
        TableView::Complete(TableRows::ResourceExchanges(
            (0..count)
                .flat_map(|index| {
                    (1..=2).map(move |resource| ResourceExchangeRow {
                        id: u64::from(index * 2 + resource),
                        exporter: 100_000 + index,
                        importer: 100_000 + (index + 1) % count,
                        resource,
                        delivered: 12_345_678 + i64::from(hour) * 100_000 + i64::from(index),
                        assigned_factories: 3,
                        convoys_assigned: 14,
                        convoys_required: 20,
                        ..Default::default()
                    })
                })
                .collect(),
        ));
    frame.metrics.table_ns[TableId::ResourceExchanges.index()] = 1;
    frame
}

#[test]
fn a_whole_world_of_trade_fits_one_reply_with_every_flow() {
    use crate::products::{
        LoadContext, Product,
        trade_flows::{TradeFlowParams, TradeFlows},
    };
    const COUNTRIES: u32 = 5_000;
    const READS: i32 = 40;
    // A real campaign's hours, so the reply is as long as a real one.
    const START: i32 = 60_827_800;
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                (0..READS)
                    .map(|hour| world_trade_frame(START + hour, COUNTRIES))
                    .collect(),
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "world".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = TradeFlowParams {
        branch_id: "1".into(),
        tags: Some(
            (0..COUNTRIES)
                .map(|index| String::from_utf8(crate::test_fixtures::scale_tag(index)).unwrap())
                .collect(),
        ),
    };
    assert!(TradeFlows::validate(&params).is_ok());
    let reply = TradeFlows::load(
        &source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.countries.len(), COUNTRIES as usize);
    assert_eq!(reply.flows.len(), 2 * COUNTRIES as usize);
    assert_eq!(reply.country_identities.len(), COUNTRIES as usize);
    assert_eq!(
        reply
            .flows
            .iter()
            .map(|flow| &flow.seller_identity)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        COUNTRIES as usize,
    );
    // Every read's change is its own run: nothing is dropped or merged.
    assert!(
        reply
            .flows
            .iter()
            .all(|flow| flow.first_hours.len() == READS as usize)
    );
    assert_eq!(span_pairs(&reply.known), [(START, START + READS - 1)]);
    assert!(serde_json::to_vec(&reply).unwrap().len() < crate::products::MAX_PRODUCT_REPLY_BYTES);
    let appended = source
        .replay_ref()
        .append(&[crate::test_fixtures::continuation_records(
            source.replay_ref(),
            1,
            (READS..51)
                .map(|hour| world_trade_frame(START + hour, COUNTRIES))
                .collect(),
        )])
        .unwrap();
    let oversized_source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "world-over-budget".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(appended),
        &AtomicBool::new(false),
    )
    .unwrap();
    let error = TradeFlows::load(
        &oversized_source,
        &params,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.to_string().contains("more than 24 MB"), "{error}");
}

/// Germany's native holder renames to Y; Spain's unrelated holder takes its old
/// tag X. End-tag requests must include Germany's earlier X history, not Spain's.
fn renamed_trade_frame(hour: i32) -> DecodedFrame {
    let mut frame = resource_frame(hour, 0, 9001, &[(1, 9001, 21, 100_000)]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows {
            if row.id == 9001 {
                row.tag = (if hour < 24 { b"X" } else { b"Y" }).to_vec();
            }
            if row.id == 31 {
                row.tag = (if hour < 24 { b"Z" } else { b"X" }).to_vec();
            }
        }
    }
    if let TableView::Complete(TableRows::ResourceExchanges(rows)) =
        &mut *frame.tables[TableId::ResourceExchanges.index()]
    {
        // Census keeps observing the renamed holder after its trade stops.
        if hour >= 48 {
            rows.clear();
        } else if hour >= 24 {
            rows[0].delivered = 200_000;
            let mut replacement = rows[0].clone();
            replacement.id = 2;
            replacement.exporter = 31;
            replacement.delivered = 300_000;
            rows.push(replacement);
        }
    }
    frame
}

#[test]
fn trade_native_histories_survive_rename_and_refuse_tag_reuse_as_identity() {
    use crate::products::trade_flows::scan;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (0..72).map(renamed_trade_frame).collect())],
    )
    .unwrap();
    let scanned = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    let (countries, flows) = scanned.reply_flows();
    assert_eq!(flows.len(), 2);
    let old = flows
        .iter()
        .find(|flow| flow.seller_identity == "[7,9001]")
        .unwrap();
    let replacement = flows
        .iter()
        .find(|flow| flow.seller_identity == "[9,31]")
        .unwrap();
    assert_eq!(old.first_hours, [0, 24]);
    assert_eq!(old.last_hours, [23, 47]);
    assert_eq!(old.delivered_raw, ["100000", "200000"]);
    assert_eq!(
        old.seller_tags
            .iter()
            .map(|tag| countries[*tag as usize].as_str())
            .collect::<Vec<_>>(),
        ["X", "Y"],
    );
    assert_eq!(replacement.first_hours, [24]);
    assert_eq!(replacement.last_hours, [47]);
    assert_eq!(replacement.delivered_raw, ["300000"]);
    assert_ne!(old.seller_identity, replacement.seller_identity);
    let y = scanned
        .country_identities
        .iter()
        .find(|country| country.tag == "Y")
        .unwrap();
    assert_eq!(
        (
            y.runs[0].slot,
            y.runs[0].country_id,
            y.runs[0].first_hour,
            y.runs[0].last_hour
        ),
        (7, 9001, 24, 71),
    );

    let chosen = scan(&replay, 1, Some(&["Y".into()]), &AtomicBool::new(false)).unwrap();
    let (_, selected) = chosen.reply_flows();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].seller_identity, "[7,9001]");
    assert_eq!(selected[0].first_hours, [0, 24]);
    assert_eq!(selected[0].delivered_raw, ["100000", "200000"]);
    let x = chosen
        .country_identities
        .iter()
        .find(|country| country.tag == "X")
        .unwrap();
    assert_eq!(x.runs.len(), 2);
    assert_eq!((x.runs[0].slot, x.runs[0].country_id), (7, 9001));
    assert_eq!((x.runs[1].slot, x.runs[1].country_id), (9, 31));
}

#[test]
fn a_peaceful_former_alias_selects_its_holder_when_trade_starts_after_rename() {
    use crate::products::trade_flows::scan;
    let mut before = renamed_trade_frame(23);
    if let TableView::Complete(TableRows::ResourceExchanges(rows)) =
        &mut *before.tables[TableId::ResourceExchanges.index()]
    {
        rows.clear();
    }
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![before, renamed_trade_frame(24)])],
    )
    .unwrap();
    // Z was peaceful, then Spain takes X and starts trading. Z selects Spain only.
    let scanned = scan(&replay, 1, Some(&["Z".into()]), &AtomicBool::new(false)).unwrap();
    let (_, flows) = scanned.reply_flows();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].seller_identity, "[9,31]");
    assert_eq!(flows[0].first_hours, [24]);
    assert_eq!(flows[0].delivered_raw, ["300000"]);
}

#[test]
fn trade_country_census_stays_complete_when_chosen_deals_are_filtered() {
    use crate::products::trade_flows::scan;
    let replay = trade_replay();
    let all = scan(&replay, 1, None, &AtomicBool::new(false)).unwrap();
    let selected = scan(&replay, 1, Some(&["SPR".into()]), &AtomicBool::new(false)).unwrap();
    assert_eq!(all.reply_flows().1.len(), 2);
    assert_eq!(selected.reply_flows().1.len(), 1);
    assert_eq!(
        serde_json::to_value(&selected.country_identities).unwrap(),
        serde_json::to_value(&all.country_identities).unwrap(),
    );
    // Italy never touches the chosen Spain deals, but still keeps its colour slot.
    assert!(
        selected
            .country_identities
            .iter()
            .any(|country| country.tag == "ITA")
    );
}
