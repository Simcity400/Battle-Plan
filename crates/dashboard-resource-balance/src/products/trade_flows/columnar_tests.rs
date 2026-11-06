//! The family's columnar preparation held to its row oracle ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every flow's runs, terms and aliases, the reads
//! known and in part, the warnings, each flow's integrals and active hours at every hour asked,
//! and the delivered integral's chart, across segment starts, gaps, cuts and a resume.

use super::{
    TradeFlowParams, TradeFlowsReply,
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    query::Read,
    scan,
};
use dashboard_base::{
    prepared::series,
    prepared_chart::{ChartBucket, ChartSummary, merge},
    products::{country_history::scan_histories, resources::ResourceNaming},
    source::{Source, SourceKey},
};
use dashboard_core::{
    config::LoadContext,
    test_fixtures::{CAMPAIGN, records, resource_frame},
};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    container::Replay,
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryRow, RowCodec, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};

const REFUSAL: &str = "synthetic refusal";

/// Italy (link 21) sells Germany (links 11 and 9001) steel and Germany sells Spain (link 31)
/// steel, over hours 0 to 80 without 15 and 61 to 63: a deal changes at 5 and a second joins
/// the first's flow at 8; the deal table is missing at 12 and 13; a seller no link resolves and
/// a resource no definition names are listed from 18 to 20; Germany is renamed at 24 and its
/// tag taken by a new country at 30; the links are unreadable at 35; nothing is traded from 40
/// to 44; a deal delivers a negative amount from 50, making its flow's sum negative; the
/// countries are missing at 70. Spain's convoys change every ten hours.
fn scenario() -> Vec<DecodedFrame> {
    (0..=80)
        .filter(|hour| *hour != 15 && !(61..=63).contains(hour))
        .map(|hour| {
            let germany = if hour >= 30 { 9002 } else { 9001 };
            let first = match hour {
                ..=4 => 100_000,
                5..=49 => 120_000,
                _ => -50_000,
            };
            let mut deals = vec![(1, 21, germany, first), (2, 11, 31, 50_000)];
            if hour >= 8 {
                deals.push((3, 21, germany, 30_000));
            }
            if (18..=20).contains(&hour) {
                deals.push((4, 777, germany, 5));
                deals.push((5, 21, 31, 7));
            }
            if (40..=44).contains(&hour) {
                deals.clear();
            }
            let mut frame = resource_frame(hour, 0, germany, &deals);
            if let TableView::Complete(TableRows::ResourceExchanges(rows)) =
                &mut *frame.tables[TableId::ResourceExchanges.index()]
            {
                for row in rows.iter_mut().filter(|row| row.id == 5) {
                    row.resource = 999;
                }
                for row in rows.iter_mut().filter(|row| row.id == 2) {
                    row.convoys_assigned = hour / 10 % 3;
                    row.convoys_required = 2;
                }
            }
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
                && hour >= 24
            {
                for row in rows.iter_mut().filter(|row| row.canonical_slot == 7) {
                    row.tag = if hour >= 30 { b"GER".to_vec() } else { b"GRR".to_vec() };
                }
            }
            let missing = |frame: &mut DecodedFrame, table: TableId| {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: REFUSAL.into(),
                };
            };
            if (12..=13).contains(&hour) {
                missing(&mut frame, TableId::ResourceExchanges);
            }
            if hour == 35 {
                missing(&mut frame, TableId::CountryLinks);
            }
            if hour == 70 {
                missing(&mut frame, TableId::Countries);
            }
            frame
        })
        .collect()
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Input::Unavailable {
            reason: REFUSAL,
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// `frame`'s tables the family reads, as the native recorder writes them.
fn feed(walk: &mut Synthetic, frame: &DecodedFrame) {
    for &table in facts::TABLES {
        match &*frame.tables[table.index()] {
            TableView::Complete(TableRows::Countries(rows)) => write(walk, table, Some(&rows[..])),
            TableView::Complete(TableRows::CountryLinks(rows)) => {
                write(walk, table, Some(&rows[..]))
            }
            TableView::Complete(TableRows::LexicalTokens(rows)) => {
                write(walk, table, Some(&rows[..]))
            }
            TableView::Complete(TableRows::ResourceDefinitions(rows)) => {
                write(walk, table, Some(&rows[..]))
            }
            TableView::Complete(TableRows::ResourceExchanges(rows)) => {
                write(walk, table, Some(&rows[..]))
            }
            TableView::Unavailable { .. } => write::<CountryRow>(walk, table, None),
            _ => panic!("{table:?} is not written by this fixture"),
        }
    }
}

/// The projected hours of `frames`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again).
fn projected(frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut last: Option<i32> = None;
    let format = FormatVersion::CURRENT;
    let mut projector = Projector::new(format.version()).unwrap();
    for (ordinal, frame) in (0..).zip(frames) {
        if walk.is_none() || last.is_some_and(|last| last + 1 != frame.hour) {
            walk = Some(Synthetic::new(facts::TABLES, 1, frame.hour).unwrap());
        }
        last = Some(frame.hour);
        let walk = walk.as_mut().unwrap();
        feed(walk, frame);
        let cold = starts.contains(&frame.hour);
        if cold {
            projector = Projector::new(format.version()).unwrap();
        }
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(),
                (format, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
                        run: 0,
                        ordinal,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
    }
    // As its emit is handed them: written to the spool and read back.
    replay_spool::respool(out).unwrap()
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
struct Built {
    pages: Memory,
    root: u64,
}

fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>, budget: &Budget) -> Built {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = Emit::new();
    let mut root = 0;
    let mut generation = 0;
    let mut hours = hours.into_iter().peekable();
    for &cut in cuts {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        generation += 1;
        root = publish(
            &mut writer,
            &mut pages,
            &cache,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(first, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if resume == Some(cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, &cache, budget, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// The row recording of `frames`, and a source reading it.
fn recorded(frames: &[DecodedFrame], flag: &AtomicBool) -> (Arc<Replay>, Source) {
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap());
    let source = Source::from_replay(
        SourceKey {
            source_id: "trade-columnar".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::clone(&replay),
        flag,
    )
    .unwrap();
    (replay, source)
}

/// Every flow's integrals through `hour` by the oracle's runs: each run's terms for each hour
/// it held through `hour`, and the hours.
fn integrals(reply: &TradeFlowsReply, token: &str, hour: i32) -> [String; 6] {
    let flow = reply
        .flows
        .iter()
        .find(|flow| {
            serde_json::to_string(&(&flow.seller_identity, &flow.buyer_identity, flow.resource))
                .unwrap()
                == token
        })
        .unwrap();
    let mut totals = [0_i128; 6];
    for run in 0..flow.first_hours.len() {
        let through = flow.last_hours[run].min(hour);
        if flow.first_hours[run] > through {
            continue;
        }
        let hours = i128::from(through - flow.first_hours[run] + 1);
        let rates = [
            flow.delivered_raw[run].parse::<i128>().unwrap(),
            i128::from(flow.factories[run]),
            i128::from(flow.convoys_assigned[run]),
            i128::from(flow.convoys_required[run]),
            i128::from(flow.deals[run]),
            1,
        ];
        for (total, rate) in totals.iter_mut().zip(rates) {
            *total += rate * hours;
        }
    }
    totals.map(|total| total.to_string())
}

/// Holds the publication `built` to the row oracle of `frames`.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let flag = AtomicBool::new(false);
    let (replay, source) = recorded(frames, &flag);
    let context = LoadContext { content: None };
    let history = scan_histories(&replay, 1, false, &[], &flag).unwrap();
    let expected = super::reply(
        &source,
        &TradeFlowParams {
            branch_id: "1".into(),
            tags: None,
        },
        &context,
        &scan(&replay, 1, None, &flag).unwrap(),
        &flag,
    )
    .unwrap();
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let (first, last) = (frames[0].hour, frames.last().unwrap().hour);
    let naming = ResourceNaming::stored(&source, &context);
    let read = Read {
        branch: 1,
        history: &history,
        last_hour: last,
        complete: true,
        naming: &naming,
        cancelled: &flag,
    };
    let params = json!({"branchId": "1", "tags": null, "from": first, "to": last, "basis": first});
    let actual = read
        .reply(&mut snapshot, &directory, params)
        .unwrap()
        .unwrap();
    let reply: TradeFlowsReply = serde_json::from_value(actual.clone()).unwrap();
    assert_eq!(reply.flows, expected.flows, "{label}: flows");
    assert_eq!(reply.countries, expected.countries, "{label}: aliases");
    assert_eq!(reply.known, expected.known, "{label}: known reads");
    assert_eq!(reply.reads, expected.reads, "{label}: reads");
    assert_eq!(reply.partial, expected.partial, "{label}: reads in part");
    assert_eq!(reply.warnings, expected.warnings, "{label}: warnings");
    assert_eq!(
        serde_json::to_value(&reply.resources).unwrap(),
        serde_json::to_value(&expected.resources).unwrap(),
        "{label}: resources"
    );
    let prefixes = actual["prefixes"].as_object().unwrap();
    assert_eq!(prefixes.len(), expected.flows.len(), "{label}: flows integrated");
    for (token, hours) in prefixes {
        for (hour, totals) in hours.as_object().unwrap() {
            let hour: i32 = hour.parse().unwrap();
            assert_eq!(
                totals,
                &json!(integrals(&expected, token, hour)),
                "{label}: {token}'s integrals through {hour}"
            );
        }
    }
    for (token, hours) in actual["bounds"].as_object().unwrap() {
        let flow = expected
            .flows
            .iter()
            .find(|flow| {
                serde_json::to_string(&(&flow.seller_identity, &flow.buyer_identity, flow.resource))
                    .unwrap()
                    == *token
            })
            .unwrap();
        for (hour, held) in hours.as_object().unwrap() {
            let hour: i32 = hour.parse().unwrap();
            let active: Vec<(i32, i32)> = (0..flow.first_hours.len())
                .map(|run| (flow.first_hours[run].max(first), flow.last_hours[run].min(hour)))
                .filter(|(from, to)| from <= to)
                .collect();
            let bounds = [
                active.first().map(|run| run.0),
                active.last().map(|run| run.1),
            ];
            assert_eq!(held, &json!(bounds), "{label}: {token}'s active hours by {hour}");
        }
    }
}

/// The publication holds what the row oracle reads, whatever the placement of segment starts,
/// cuts and a resume.
#[test]
fn the_publication_holds_what_the_row_oracle_reads_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let frames = scenario();
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    for (starts, cuts, resume) in [
        (&[][..], &[80][..], None),
        (&[10, 24, 31, 52][..], &[9, 23, 47, 80][..], Some(23)),
        (&[6, 19, 41, 64][..], &[12, 30, 60, 80][..], Some(60)),
        (&hours[1..], &hours[..], Some(36)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let built = build(projected(&frames, starts), cuts, resume, &budget);
        check(&built, &frames, &label);
    }
}

/// The scenario's flows as the oracle reads them: a renamed seller and a new holder of its tag
/// start new runs, unknown reads and the hours not walked end them, and the deals left out make
/// their reads known in part.
#[test]
fn the_scenario_reads_as_the_deals_were_recorded() {
    let flag = AtomicBool::new(false);
    let (replay, _) = recorded(&scenario(), &flag);
    let scanned = scan(&replay, 1, None, &flag).unwrap();
    let spans = |spans: &[dashboard_base::products::country_groups::HourSpan]| {
        spans
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        spans(&scanned.known),
        [(0, 11), (14, 14), (16, 34), (36, 60), (64, 69), (71, 80)]
    );
    assert_eq!(
        scanned
            .partial
            .iter()
            .map(|span| (span.first_hour, span.last_hour, span.deals))
            .collect::<Vec<_>>(),
        [(18, 20, 2)]
    );
    assert_eq!((scanned.unnamed, scanned.unresolved), (3, 3));
    let (countries, flows) = scanned.reply_flows();
    assert!(countries.contains(&"GRR".to_owned()));
    let italy = flows
        .iter()
        .find(|flow| flow.seller_identity == "[8,21]" && flow.buyer_identity == "[7,9001]")
        .unwrap();
    // Two deals of one seller, buyer and resource are one flow; the rename starts a run.
    assert_eq!(italy.first_hours[..4], [0, 5, 8, 14]);
    assert!(italy.first_hours.contains(&24));
    assert_eq!(italy.deals[2], 2);
}

/// Each bin's integral of what a flow delivered, folded hour by hour as a stored series of it
/// folds: its hours from the flow's first through the last published, ties at their first hour.
fn folded(
    reply: &TradeFlowsReply,
    flow: usize,
    first: i32,
    published: i32,
    bins: &[(i32, i32)],
) -> Vec<Option<ChartBucket>> {
    let flow = &reply.flows[flow];
    let started = flow.first_hours[0];
    let mut total = 0_i128;
    let mut daily = std::collections::BTreeMap::new();
    for run in 0..flow.first_hours.len() {
        for hour in flow.first_hours[run]..=flow.last_hours[run] {
            daily.insert(hour, flow.delivered_raw[run].parse::<i128>().unwrap());
        }
    }
    let mut values = std::collections::BTreeMap::new();
    for hour in started..=published {
        total += daily.get(&hour).copied().unwrap_or(0);
        values.insert(hour, total);
    }
    let shown = |raw: i128| raw as f64 / 100_000.0;
    bins.iter()
        .map(|&(low, high)| {
            let mut bucket = None;
            let mut seen = false;
            for hour in low.max(started).max(first)..=high.min(published) {
                let raw = values[&hour];
                let at = Some(ChartBucket {
                    absolute_total: raw.unsigned_abs(),
                    first_hour: hour,
                    last_hour: hour,
                    first_value: shown(raw),
                    last_value: shown(raw),
                    last_value_hour: hour,
                    min_hour: hour,
                    min_value: shown(raw),
                    max_hour: hour,
                    max_value: shown(raw),
                    first_break: hour == started,
                    has_break: false,
                });
                bucket = if seen { merge(bucket, at) } else { at };
                seen = true;
            }
            bucket.map(|mut bucket: ChartBucket| {
                bucket.has_break = bucket.first_hour > low || bucket.last_hour < high;
                bucket
            })
        })
        .collect()
}

/// The delivered integral's chart folds as the hourly integrals would, on dense and hourly bins,
/// through the run that started before the window and across the hours a flow held no run.
#[test]
fn the_integral_chart_folds_as_the_hourly_integrals_would() {
    let budget = Budget::new(256 << 20);
    let frames = scenario();
    let flag = AtomicBool::new(false);
    let (replay, source) = recorded(&frames, &flag);
    let context = LoadContext { content: None };
    let history = scan_histories(&replay, 1, false, &[], &flag).unwrap();
    let expected = super::reply(
        &source,
        &TradeFlowParams {
            branch_id: "1".into(),
            tags: None,
        },
        &context,
        &scan(&replay, 1, None, &flag).unwrap(),
        &flag,
    )
    .unwrap();
    let built = build(projected(&frames, &[24, 52]), &[30, 80], Some(30), &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let naming = ResourceNaming::stored(&source, &context);
    let read = Read {
        branch: 1,
        history: &history,
        last_hour: 80,
        complete: true,
        naming: &naming,
        cancelled: &flag,
    };
    for (index, flow) in expected.flows.iter().enumerate() {
        let token = serde_json::to_string(&(&flow.seller_identity, &flow.buyer_identity, flow.resource))
            .unwrap();
        let actor = directory.actor(token.as_bytes()).unwrap();
        for (first, last, width) in [(0, 80, 7), (3, 57, 5), (20, 33, 14), (45, 80, 64), (0, 80, 81)] {
            let view = (f64::from(first), f64::from(last), width);
            let (_, bins) = series::summary_bins((first, last), view).unwrap();
            let chart: ChartSummary = read
                .integral_chart(&mut snapshot, &directory, actor, (first, last), view)
                .unwrap();
            let want = folded(&expected, index, first, 80, &bins);
            let strip = |bucket: Option<ChartBucket>| {
                bucket.map(|bucket| {
                    (
                        bucket.absolute_total,
                        bucket.first_hour,
                        bucket.last_hour,
                        bucket.min_hour,
                        bucket.min_value,
                        bucket.max_hour,
                        bucket.max_value,
                        bucket.first_break,
                        bucket.has_break,
                    )
                })
            };
            assert_eq!(
                chart.buckets.into_iter().map(strip).collect::<Vec<_>>(),
                want.into_iter().map(strip).collect::<Vec<_>>(),
                "{token} over {first}..={last} in {width}"
            );
        }
    }
}

/// Navigation and the dense charts answer from the publication: every flow's daily amounts,
/// totals and averages in one bin per hour or fewer, and the next change from each hour.
#[test]
fn dense_charts_and_navigation_answer_from_the_publication() {
    let budget = Budget::new(256 << 20);
    let frames = scenario();
    let flag = AtomicBool::new(false);
    let (replay, source) = recorded(&frames, &flag);
    let context = LoadContext { content: None };
    let history = scan_histories(&replay, 1, false, &[], &flag).unwrap();
    let built = build(projected(&frames, &[40]), &[80], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let naming = ResourceNaming::stored(&source, &context);
    let read = Read {
        branch: 1,
        history: &history,
        last_hour: 80,
        complete: true,
        naming: &naming,
        cancelled: &flag,
    };
    for measure in ["current", "total", "average"] {
        let params = json!({
            "branchId": "1", "tags": null, "from": 0, "to": 80, "width": 8, "basis": 0,
            "measure": measure, "hours": [80],
        });
        let reply = read
            .reply(&mut snapshot, &directory, params)
            .unwrap()
            .unwrap();
        let summaries = reply["summaries"].as_object().unwrap();
        assert_eq!(summaries.len(), directory.actors.len(), "{measure}");
        for summary in summaries.values() {
            let summary: ChartSummary = serde_json::from_value(summary.clone()).unwrap();
            // The deal table's missing hours are never charted as no trade.
            assert!(summary.buckets.iter().flatten().all(|bucket| {
                !(bucket.first_hour <= 12 && 13 <= bucket.last_hour && !bucket.has_break)
            }));
        }
    }
    let mut stops = Vec::new();
    let mut from: Option<f64> = None;
    loop {
        let params = json!({
            "branchId": "1", "tags": null, "between": false, "op": "navigate",
            "key": "ArrowRight", "from": from, "firstHour": 0, "lastHour": 80,
        });
        let reply = read
            .reply(&mut snapshot, &directory, params)
            .unwrap()
            .unwrap();
        let Some(hour) = reply["hour"].as_f64() else {
            break;
        };
        if from == Some(hour) {
            break;
        }
        stops.push(hour as i32);
        from = Some(hour);
    }
    // Every deal change, unknown read and hour not walked is a stop.
    for hour in [5, 8, 12, 14, 16, 18, 21, 24, 30, 35, 36, 40, 45, 50, 64, 70, 71] {
        assert!(stops.contains(&hour), "stop at {hour} in {stops:?}");
    }
    assert!(stops.windows(2).all(|pair| pair[0] < pair[1]));
}

/// A flow's terms refuse when their sum overflows, never wrapping, at the hour they do.
#[test]
fn summed_terms_refuse_where_they_overflow() {
    let frames = [
        resource_frame(0, 0, 9001, &[(1, 21, 9001, i64::MAX), (2, 21, 9001, -1)]),
        resource_frame(1, 0, 9001, &[(1, 21, 9001, i64::MAX), (3, 21, 9001, 1)]),
    ];
    let mut walk = Synthetic::new(facts::TABLES, 1, 0).unwrap();
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT.version()).unwrap();
    let mut out: Vec<facts::Hour> = Vec::new();
    let mut results = Vec::new();
    for (ordinal, frame) in (0..).zip(&frames) {
        feed(&mut walk, frame);
        results.push(
            walk.end(|state, manifest, decoded, first| {
                let world = dashboard_world::World::at(
                    &mut world,
                    first,
                    &facts::needs(),
                    (FormatVersion::CURRENT, CaptureMode::Full),
                    state,
                    manifest,
                )
                .unwrap();
                projector.hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
                        run: 0,
                        ordinal,
                        starts: first,
                        opens: first,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
            })
            .unwrap()
            .map_err(|error| error.to_string()),
        );
    }
    assert_eq!(
        results,
        [
            Ok(()),
            Err("recorded trade terms overflow their sum".to_owned())
        ]
    );
}
