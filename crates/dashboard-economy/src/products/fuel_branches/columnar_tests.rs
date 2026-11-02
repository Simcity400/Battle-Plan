//! The family's columnar preparation held to its row census ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every reply, across segment starts, a gap, cuts
//! and a resume.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    *,
};
use crate::products::test_support;
use dashboard_base::prepared::frame_changes::FrameChangeTracker;
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryFuelConsumerRow, CountryLinkRow, CountryRow, RowCodec, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::atomic::AtomicBool;

const REFUSAL: &str = "synthetic refusal";

/// One hour as the recorder writes it: the countries (id, slot, tag), none where unavailable;
/// the links (raw id, slot); the consumers (country, index, requested, received, priority),
/// none where unavailable; and whether the consumers were read.
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: Option<Vec<(u32, u32, &'static str)>>,
    links: Vec<(u32, u32)>,
    consumers: Option<Vec<(u32, u32, i64, i64, i32)>>,
    read: bool,
}

/// Hours 100 to 175 without 130 and 131: consumers read every sixth hour, countries moving every
/// third, so some hours change nothing. A holds three consumers, an alias of its slot one of them
/// at 115-117, and none at 160-162; B changes id at 140; C gains a fourth index at 150 and is
/// renamed at 155; D holds none; E swaps its only consumer for another at unread 141. A row no
/// link resolves is held at 122-124; the consumers are unavailable at 125-126, the countries
/// at 145.
fn scenario() -> Vec<Tables> {
    (100..=175)
        .filter(|hour| !(130..=131).contains(hour))
        .map(|hour| {
            let day = i64::from(hour / 6);
            let b = if hour >= 140 { 20 } else { 2 };
            let c = if hour >= 155 { "CCX" } else { "CCC" };
            let countries = (hour != 145)
                .then(|| vec![(1, 1, "AAA"), (b, 2, "BBB"), (3, 3, c), (4, 4, "DDD"), (5, 5, "EEE")]);
            let mut links = vec![(1, 1), (b, 2), (3, 3), (4, 4), (5, 5), (11, 1)];
            links.sort_unstable();
            let mut consumers = Vec::new();
            if !(160..=162).contains(&hour) {
                consumers.push((1, 0, 1000 * day, 900 * day, 0));
                consumers.push((1, 1, 0, 0, 1));
                consumers.push((1, 2, 50 * day, 50 * day, 2));
            }
            consumers.push((b, 0, 7, 3, 0));
            consumers.push((3, 0, 20 * day, 10 * day, 1));
            consumers.push((3, 1, 5, 5, i32::from(day % 2 == 0)));
            if hour >= 150 {
                consumers.push((3, 3, 9, 9, 3));
            }
            consumers.push((5, u32::from(hour >= 141), 4, 4, 0));
            if (115..=117).contains(&hour) {
                consumers.push((11, 0, 77, 66, 5));
            }
            if (122..=124).contains(&hour) {
                consumers.push((99, 0, 1, 1, 0));
            }
            consumers.sort_unstable_by_key(|&(country, index, ..)| (country, index));
            Tables {
                hour,
                countries,
                links,
                consumers: (!(125..=126).contains(&hour)).then_some(consumers),
                read: hour % 6 == 0,
            }
        })
        .collect()
}

/// The countries as the recorder writes them, their political power moving every third hour.
fn country_rows(countries: &[(u32, u32, &str)], hour: i32) -> Vec<CountryRow> {
    let mut rows: Vec<_> = countries
        .iter()
        .map(|&(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            political_power_raw: Some(i64::from(hour / 3)),
            ..Default::default()
        })
        .collect();
    rows.sort_unstable_by_key(|row| row.id);
    rows
}

fn link_rows(links: &[(u32, u32)]) -> Vec<CountryLinkRow> {
    links
        .iter()
        .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .collect()
}

fn consumer_rows(consumers: &[(u32, u32, i64, i64, i32)]) -> Vec<CountryFuelConsumerRow> {
    consumers
        .iter()
        .map(|&(country, index, requested_raw, received_raw, priority)| CountryFuelConsumerRow {
            country,
            index,
            priority,
            requested_raw,
            received_raw,
        })
        .collect()
}

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = test_support::frame(tables.hour);
    let mut put = |table: TableId, view: TableView| {
        *frame.tables[table.index()] = view;
    };
    let refused = || TableView::Unavailable {
        reason: REFUSAL.into(),
    };
    put(
        TableId::Countries,
        match &tables.countries {
            Some(countries) => {
                TableView::Complete(TableRows::Countries(country_rows(countries, tables.hour)))
            }
            None => refused(),
        },
    );
    put(
        TableId::CountryLinks,
        TableView::Complete(TableRows::CountryLinks(link_rows(&tables.links))),
    );
    put(
        TableId::CountryFuelConsumers,
        match &tables.consumers {
            Some(consumers) => {
                TableView::Complete(TableRows::CountryFuelConsumers(consumer_rows(consumers)))
            }
            None => refused(),
        },
    );
    if tables.read {
        frame.metrics.table_ns[TableId::CountryFuelConsumers.index()] = 1;
    }
    frame
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>, read: bool) {
    let sample = Sample::FromReadBit;
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

/// The projected hours of `hours`, a projector starting cold at each of `starts` from the
/// hour before's state and at each walk's first hour (after a gap, the walk starts again), and
/// the row frames of the same hours, a table the hour left as it was keeping its view.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full);
    let mut before: Option<&Tables> = None;
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last: &DecodedFrame| last.hour + 1 != tables.hour);
        let fresh = walk.is_none() || gap;
        if fresh {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        let cold = fresh || starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full);
            let start = if fresh {
                segments::Start::Run { continues: false }
            } else {
                segments::Start::Restart(walk.state())
            };
            projector.start(start, 1, tables.hour).unwrap();
        }
        let countries = tables
            .countries
            .as_deref()
            .map(|countries| country_rows(countries, tables.hour));
        write(walk, TableId::Countries, countries.as_deref(), false);
        write(walk, TableId::CountryLinks, Some(&link_rows(&tables.links)[..]), false);
        let consumers = tables.consumers.as_deref().map(consumer_rows);
        write(walk, TableId::CountryFuelConsumers, consumers.as_deref(), tables.read);
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
        let mut now = frame(tables);
        if let (Some(before), Some(last)) = (before.filter(|_| !gap), frames.last()) {
            for (table, same) in [
                (
                    TableId::Countries,
                    before.countries == tables.countries && before.hour / 3 == tables.hour / 3,
                ),
                (TableId::CountryLinks, before.links == tables.links),
                (TableId::CountryFuelConsumers, before.consumers == tables.consumers),
            ] {
                if same {
                    now.tables[table.index()] = last.tables[table.index()].clone();
                }
            }
        }
        frames.push(now);
        before = Some(tables);
    }
    (out, frames)
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

fn labels() -> [Option<String>; 3] {
    [Some("Land".into()), Some("Air".into()), Some("Sea".into())]
}

fn params(tags: &[&str]) -> FuelBranchesParams {
    FuelBranchesParams {
        branch_id: "1".into(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
    }
}

const SELECTIONS: &[&[&str]] = &[
    &[],
    &["AAA"],
    &["BBB"],
    &["CCC"],
    &["CCX"],
    &["DDD"],
    &["AAA", "CCX"],
];

/// The row census's replies to every selection through `through`, from the row frames observed
/// as one block with their change facts.
fn row_replies(frames: &[DecodedFrame], through: i32) -> Vec<serde_json::Value> {
    let mut prepared = row::Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in frames {
        let changes = tracker.observe(frame, FormatVersion::CURRENT);
        prepared.observe_changes(frame, &changes).unwrap();
    }
    let fixture = test_support::fixture();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    SELECTIONS
        .iter()
        .map(|tags| {
            let reply = row::reply(
                &fixture.store,
                &fixture.key,
                &params(tags),
                Some(through),
                labels(),
                Vec::new(),
                &flag,
            )
            .unwrap();
            serde_json::to_value(reply).unwrap()
        })
        .collect()
}

/// The publication's replies to every selection through `through`.
fn replies(built: &Built, through: i32) -> Vec<serde_json::Value> {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let flag = AtomicBool::new(false);
    SELECTIONS
        .iter()
        .map(|tags| {
            let reply = query::reply(
                &mut snapshot,
                &directory,
                &params(tags),
                through,
                labels(),
                Vec::new(),
                &flag,
            )
            .unwrap();
            serde_json::to_value(reply).unwrap()
        })
        .collect()
}

/// Every placement of segment starts, cuts and a resume publishes what the row census answers,
/// through the last hour and through one before it.
#[test]
fn the_publication_answers_what_the_row_census_answers_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    for (starts, cuts, resume) in [
        (&[][..], &[175][..], None),
        (&[110, 140, 141, 150][..], &[109, 129, 149, 175][..], Some(129)),
        (&[103, 116, 124, 125, 136, 155, 161][..], &[115, 133, 146, 175][..], Some(146)),
        (
            &(101..=175).collect::<Vec<_>>()[..],
            &(100..=175).filter(|hour| !(130..=131).contains(hour)).collect::<Vec<_>>()[..],
            Some(120),
        ),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(&hours, starts);
        let built = build(projected, cuts, resume, &budget);
        for through in [175, 150] {
            let expected = row_replies(&frames, through);
            let published = replies(&built, through);
            for ((tags, expected), published) in SELECTIONS.iter().zip(&expected).zip(&published) {
                assert_eq!(published, expected, "{label}, through {through}, tags {tags:?}");
            }
        }
    }
}

/// The scenario reads as the fuel panel shows it: the alias's cell, the unidentified row's
/// unknown hours, the refused hours and the renamed holder.
#[test]
fn the_scenario_reads_as_the_fuel_panel_shows_it() {
    let (_, frames) = projected(&scenario(), &[]);
    let replies = row_replies(&frames, 175);
    let every = &replies[0];
    let countries = every["countries"].as_array().unwrap();
    let holders: Vec<&str> = countries
        .iter()
        .map(|country| country["holder"].as_str().unwrap())
        .collect();
    assert_eq!(holders, ["1", "2", "3", "5"]);
    let known = every["known"].as_array().unwrap();
    assert!(known.iter().all(|run| {
        let (first, last) = (run[0].as_i64().unwrap(), run[1].as_i64().unwrap());
        !(122..=126).contains(&first) && !(122..=126).contains(&last) && !(first..=last).contains(&145)
    }));
    let warnings: Vec<&str> = every["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect();
    assert!(warnings.iter().any(|line| line.contains("beyond the fuel panel")));
    // 124 changes nothing and reads nothing: the census does not observe it.
    assert!(warnings.iter().any(|line| line.contains("(2 observed hours)") && line.contains("no identity")));
    assert!(warnings.iter().any(|line| line.contains("fuel consumers unavailable: hour 125")));
    assert!(warnings.iter().any(|line| line.contains("country identities unavailable: hour 145")));
    let renamed = &countries[2]["tags"];
    assert_eq!(renamed[1]["tag"], "CCX");
    assert_eq!(renamed[1]["hour"], 155);
    // E's swap at 141 keeps the read that supplied its consumers.
    let read = countries[3]["read"].as_array().unwrap();
    assert!(read.iter().any(|run| run[0] == 138 && run[1].as_i64().unwrap() >= 141 && run[2] == "138"));
}
