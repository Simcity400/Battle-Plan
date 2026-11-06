//! The family's columnar preparation held to its row census ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every country's pool runs, known deal hours,
//! trades and counts, across segment starts, a gap, cuts and a resume; and the operator handing
//! the emit only the holders whose rows changed.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    *,
};
use dashboard_base::{
    prepared::coverage::Coverage,
    test_fixtures::{CAMPAIGN, continuation_records, file_source, records, resource_frame},
};
use dashboard_series::{
    Blob, Publication, Snapshot, Span, Writer,
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
    rows::{CountryResourceRow, RowCodec, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::atomic::AtomicBool;

const REFUSAL: &str = "synthetic missing resource read";

fn pools(frame: &mut DecodedFrame) -> &mut Vec<CountryResourceRow> {
    match &mut *frame.tables[TableId::CountryResources.index()] {
        TableView::Complete(TableRows::CountryResources(rows)) => rows,
        _ => unreachable!("scenario pools are complete"),
    }
}

/// Hours 100 to 135 without 120 and 121. Germany imports steel from Italy and, from 106, from
/// Spain; Italy's terms change at 115 and a deal names an unlinked country through 109. Germany
/// holds no row at 104 and 105 and changes holder at 110, coal is defined from 112, deals are
/// unread at 111 and 118, pools at 116, and a pool names an undefined resource throughout.
fn scenario() -> Vec<DecodedFrame> {
    (100..=135)
        .filter(|hour| !(120..=121).contains(hour))
        .map(|hour| {
            let deals: Vec<(u64, u32, u32, i64)> = match hour {
                ..=105 => vec![(51, 21, 11, 12), (53, 0xdead, 11, 1)],
                106..=109 => vec![(51, 21, 11, 12), (52, 31, 11, 5), (53, 0xdead, 11, 1)],
                110..=114 => vec![(51, 21, 11, 12), (52, 31, 11, 5)],
                _ => vec![(51, 21, 11, 13), (52, 31, 11, 5)],
            };
            let steel = if hour < 102 { -100 } else { 300 };
            let germany = if hour < 110 { 9001 } else { 9002 };
            let mut frame = resource_frame(hour, steel, germany, &deals);
            if (104..106).contains(&hour)
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.retain(|row| row.tag != b"GER");
            }
            let mut unnamed = pools(&mut frame)[0].clone();
            unnamed.resource = 999;
            pools(&mut frame).push(unnamed);
            pools(&mut frame).sort_by_key(|row| (row.country, row.resource));
            if hour >= 112 {
                if let TableView::Complete(TableRows::LexicalTokens(rows)) =
                    &mut *frame.tables[TableId::LexicalTokens.index()]
                {
                    rows.push(replay_data::rows::LexicalTokenRow {
                        token: 503,
                        literal: b"coal".to_vec(),
                    });
                }
                if let TableView::Complete(TableRows::ResourceDefinitions(rows)) =
                    &mut *frame.tables[TableId::ResourceDefinitions.index()]
                {
                    rows.push(replay_data::rows::ResourceDefinitionRow {
                        id: 3,
                        token: 503,
                        active: 1,
                        factory_cost: 12_500,
                    });
                }
                let mut coal = pools(&mut frame)[0].clone();
                coal.resource = 3;
                coal.produced = i64::from(hour) * 1_000;
                pools(&mut frame).push(coal);
                pools(&mut frame).sort_by_key(|row| (row.country, row.resource));
            }
            let unread = match hour {
                111 | 118 => Some(TableId::ResourceExchanges),
                116 => Some(TableId::CountryResources),
                _ => None,
            };
            if let Some(table) = unread {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: REFUSAL.into(),
                };
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

/// Writes `frame`'s tables into `walk` as the recorder writes them.
fn record(walk: &mut Synthetic, frame: &DecodedFrame) {
    for &table in facts::TABLES {
        let view = &*frame.tables[table.index()];
        match view {
            TableView::Complete(TableRows::Countries(rows)) => write(walk, table, Some(rows)),
            TableView::Complete(TableRows::CountryLinks(rows)) => write(walk, table, Some(rows)),
            TableView::Complete(TableRows::LexicalTokens(rows)) => write(walk, table, Some(rows)),
            TableView::Complete(TableRows::ResourceDefinitions(rows)) => {
                write(walk, table, Some(rows))
            }
            TableView::Complete(TableRows::CountryResources(rows)) => {
                write(walk, table, Some(rows))
            }
            TableView::Complete(TableRows::ResourceExchanges(rows)) => {
                write(walk, table, Some(rows))
            }
            _ => write::<CountryResourceRow>(walk, table, None),
        }
    }
}

/// The projected hours of `frames`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again).
fn projected(frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
    let mut last: Option<i32> = None;
    for (ordinal, frame) in (0..).zip(frames) {
        let gap = last.is_some_and(|last| last + 1 != frame.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, frame.hour).unwrap());
        }
        last = Some(frame.hour);
        let walk = walk.as_mut().unwrap();
        record(walk, frame);
        let cold = starts.contains(&frame.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT).unwrap();
        }
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(),
                (FormatVersion::CURRENT, CaptureMode::Full),
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

struct Built {
    pages: Memory,
    root: u64,
}

/// A publication of `hours`, cut after each of `cuts`' hours; when `resume` names one of them,
/// the emit is saved there and goes on from the publication.
fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>) -> Built {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
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
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// The recording of `frames` in two blocks.
fn recording(frames: &[DecodedFrame]) -> Replay {
    let (first, second): (Vec<_>, Vec<_>) =
        frames.iter().cloned().partition(|frame| frame.hour < 120);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, first)]).unwrap();
    replay
        .append(&[continuation_records(&replay, 1, second)])
        .unwrap()
}

fn request(tag: &str, extra: serde_json::Value) -> (query::Params, serde_json::Value) {
    let mut params = serde_json::json!({ "branchId": "1", "tag": tag });
    for (key, value) in extra.as_object().unwrap() {
        params[key] = value.clone();
    }
    (serde_json::from_value(params.clone()).unwrap(), params)
}

/// Every placement of segment starts, cuts and a resume answers each country as the row census
/// reads it: its pool runs, known deal hours, trades and warnings.
#[test]
fn replies_from_the_publication_are_the_row_census_across_segments_cuts_and_a_resume() {
    let frames = scenario();
    let replay = recording(&frames);
    let cancelled = AtomicBool::new(false);
    let mut expected = BTreeMap::new();
    for tag in ["GER", "ITA", "SPR"] {
        expected.insert(tag, row::scan(&replay, 1, tag, &cancelled).unwrap());
    }
    let (source, _root) = file_source(replay);
    let context = LoadContext { content: None };
    let coverage = Coverage {
        schema: dashboard_base::prepared::coverage::SCHEMA,
        first_hour: 100,
        last_hour: 135,
        complete: true,
    };
    for (starts, cuts, resume) in [
        (&[][..], &[135][..], None),
        (&[106, 112, 125][..], &[105, 115, 135][..], Some(115)),
        (&[101, 111, 116, 119, 130][..], &[103, 111, 119, 128, 135][..], Some(111)),
        (
            &(101..=135).collect::<Vec<_>>()[..],
            &(100..=135).filter(|hour| !(120..=121).contains(hour)).collect::<Vec<_>>()[..],
            Some(124),
        ),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        // A resumed preparation projects anew from the hour after its resume.
        let mut starts = starts.to_vec();
        starts.extend(resume.and_then(|resume| {
            frames
                .iter()
                .map(|frame| frame.hour)
                .find(|&hour| hour > resume)
        }));
        let built = build(projected(&frames, &starts), cuts, resume);
        let mut snapshot =
            Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        for (tag, scanned) in &expected {
            let want = super::reply(
                &source,
                &CountryParams {
                    branch_id: "1".into(),
                    tag: (*tag).into(),
                },
                &context,
                scanned,
                &cancelled,
            )
            .unwrap();
            let (input, params) = request(tag, serde_json::json!({ "from": 100, "to": 135 }));
            let mut actual = query::reply(
                &source,
                &context,
                &mut snapshot,
                &directory,
                input,
                &params,
                &coverage,
                &cancelled,
            )
            .unwrap()
            .unwrap();
            for field in ["summaries", "campaignSizes", "exactHours"] {
                actual.as_object_mut().unwrap().remove(field);
            }
            let mut actual: ResourceBalanceReply = serde_json::from_value(actual).unwrap();
            let mut want = want;
            want.trades.sort_by_key(|run| (run.first_hour, run.deal.clone()));
            actual.trades.sort_by_key(|run| (run.first_hour, run.deal.clone()));
            assert_eq!(actual.series, want.series, "{label}: {tag}'s pools");
            assert_eq!(actual.deal_hours, want.deal_hours, "{label}: {tag}'s deal hours");
            assert_eq!(actual.trades, want.trades, "{label}: {tag}'s trades");
            assert_eq!(actual.warnings, want.warnings, "{label}: {tag}'s warnings");
            assert_eq!(actual.resources, want.resources, "{label}: {tag}'s resources");
        }
    }
}

/// The scenario's counts as the row census reads them: an undefined resource's rows and the
/// unlinked deal's every hour they were read, never zero.
#[test]
fn the_scenario_counts_unnamed_and_unresolved_rows_every_hour_read() {
    let replay = recording(&scenario());
    let germany = row::scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert!(germany.unnamed > 0 && germany.unresolved > 0);
    assert_eq!(germany.resources.len(), 3);
    assert!(germany.series[&3].iter().all(|run| run.first_hour >= 112));
}

/// A dense view answers chart summaries of each resource's eleven values, and navigation steps
/// from a pool change to the next.
#[test]
fn dense_views_summarise_and_navigation_steps_between_changes() {
    let frames = scenario();
    let (source, _root) = file_source(recording(&frames));
    let built = build(projected(&frames, &[]), &[135], None);
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let context = LoadContext { content: None };
    let coverage = Coverage {
        schema: dashboard_base::prepared::coverage::SCHEMA,
        first_hour: 100,
        last_hour: 135,
        complete: true,
    };
    let cancelled = AtomicBool::new(false);
    let (input, params) = request(
        "GER",
        serde_json::json!({ "from": 100.25, "to": 135.5, "width": 4, "hours": [106] }),
    );
    let dense = query::reply(
        &source, &context, &mut snapshot, &directory, input, &params, &coverage, &cancelled,
    )
    .unwrap()
    .unwrap();
    let summaries = dense["summaries"].as_array().unwrap();
    assert_eq!(summaries.len(), 3, "steel, oil and coal");
    assert_eq!(summaries[0]["supply"]["from"], 100.25);
    assert_eq!(summaries[0]["parts"].as_array().unwrap().len(), 8);
    assert!(
        dense["series"][0]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"].as_i64().unwrap() <= 106
                && run["lastHour"].as_i64().unwrap() >= 106)
    );
    let (input, params) = request(
        "GER",
        serde_json::json!({
            "op": "navigate", "key": "ArrowRight", "from": 100.5, "firstHour": 100,
            "lastHour": 135, "resources": [1],
        }),
    );
    let hour = query::reply(
        &source, &context, &mut snapshot, &directory, input, &params, &coverage, &cancelled,
    )
    .unwrap()
    .unwrap();
    assert_eq!(hour["hour"], 102.0, "steel's pools change at 102");
}

/// A page halved for its rows' facts keeps, for the lines it gives the next page, only the
/// hours asked for: the reply is the one that never drew those lines.
#[test]
fn a_halved_page_keeps_no_samples_of_the_lines_it_gives_away() {
    let frames = scenario();
    let (source, _root) = file_source(recording(&frames));
    let built = build(projected(&frames, &[]), &[135], None);
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let context = LoadContext { content: None };
    let coverage = Coverage {
        schema: dashboard_base::prepared::coverage::SCHEMA,
        first_hour: 100,
        last_hour: 135,
        complete: true,
    };
    let cancelled = AtomicBool::new(false);
    let view = serde_json::json!({ "from": 100.25, "to": 135.5, "width": 4, "hours": [106] });
    let mut ask = |extra: serde_json::Value| {
        let mut params = view.clone();
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        let (input, params) = request("GER", params);
        query::reply(
            &source, &context, &mut snapshot, &directory, input, &params, &coverage, &cancelled,
        )
        .unwrap()
        .unwrap()
    };
    let whole = ask(serde_json::json!({}));
    assert_eq!(whole["summaries"].as_array().unwrap().len(), 3);
    assert!(whole["cursor"].is_null());
    let first = whole["summaries"][0]["id"].clone();
    let alone = ask(serde_json::json!({ "resources": [first] }));
    assert_ne!(
        whole["series"], alone["series"],
        "the other lines' drawn hours add pool runs"
    );
    // Every line is admitted, but the whole reply is a byte too large: the page keeps one line.
    let bytes = serde_json::to_vec(&whole).unwrap().len();
    let halved = ask(serde_json::json!({ "pageBudget": bytes - 1 }));
    assert_eq!(halved["cursor"], "1");
    assert_eq!(halved["summaries"], serde_json::json!([whole["summaries"][0]]));
    assert_eq!(halved["series"], alone["series"]);
}

/// Of 1,500 countries, an hour hands the emit only the holder whose pool changed, and nothing
/// when nothing the balance reads changes.
#[test]
fn an_hour_hands_the_emit_only_the_holders_whose_rows_changed() {
    let countries: Vec<_> = (0..1_500_u32)
        .map(|at| replay_data::rows::CountryRow {
            id: 10_000 + at,
            canonical_slot: at,
            tag: format!("T{at:04}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links: Vec<_> = countries
        .iter()
        .map(|country| replay_data::rows::CountryLinkRow {
            id: country.id,
            canonical_slot: country.canonical_slot,
        })
        .collect();
    let pools = |hour: i32| -> Vec<CountryResourceRow> {
        countries
            .iter()
            .map(|country| CountryResourceRow {
                country: country.id,
                resource: 1,
                produced: if country.id == 10_700 && hour >= 2 {
                    5
                } else {
                    1
                },
                ..Default::default()
            })
            .collect()
    };
    let definitions = [replay_data::rows::ResourceDefinitionRow {
        id: 1,
        token: 501,
        active: 1,
        factory_cost: 0,
    }];
    let tokens = [replay_data::rows::LexicalTokenRow {
        token: 501,
        literal: b"steel".to_vec(),
    }];
    let mut walk = Synthetic::new(facts::TABLES, 1, 0).unwrap();
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
    let mut out = Vec::new();
    for hour in 0..4 {
        write(&mut walk, TableId::Countries, Some(&countries[..]));
        write(&mut walk, TableId::CountryLinks, Some(&links[..]));
        write(&mut walk, TableId::LexicalTokens, Some(&tokens[..]));
        write(&mut walk, TableId::ResourceDefinitions, Some(&definitions[..]));
        write(&mut walk, TableId::CountryResources, Some(&pools(hour)[..]));
        write::<replay_data::rows::ResourceExchangeRow>(
            &mut walk,
            TableId::ResourceExchanges,
            Some(&[]),
        );
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
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
                        run: 0,
                        ordinal: hour as u32,
                        starts: first,
                        opens: first,
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
    let listed = |at: usize| -> Option<Vec<u32>> {
        out[at].facts.as_ref().and_then(|facts| {
            facts
                .pools
                .as_ref()
                .map(|pools| pools.slots.iter().map(|&(slot, _)| slot).collect())
        })
    };
    assert_eq!(listed(0).map(|slots| slots.len()), Some(1_500), "the first hour is whole");
    assert!(out[1].facts.is_none(), "an unchanged hour carries nothing");
    assert_eq!(listed(2), Some(vec![700]), "only the changed holder");
    assert!(out[3].facts.is_none(), "an unchanged hour carries nothing");
}
