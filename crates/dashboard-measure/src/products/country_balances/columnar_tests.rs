//! The family's columnar preparation held to the row history (`country_history`) on the same
//! hours, written as the native recorder writes them: every tag's political and command power
//! runs, their holders and segment starts, and their net drops, across segment starts, a gap,
//! cuts and a resume; the replies read from it; and the work an hour hands the emit.

use super::{
    emit::{Emit, MEASURES, SCHEMA},
    facts::{self, Projector},
    query,
};
use dashboard_base::{
    prepared::engine,
    products::{country_history::HistoryScan, country_measure::Measure},
    test_fixtures::source,
};
use dashboard_series::{
    Blob, PageRead, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryLinkRow, CountryRow, RowCodec, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::atomic::AtomicBool};

/// A Countries row: raw id, canonical slot, tag, political power, command power.
type Row = (u32, u32, &'static str, Option<i64>, i64);

/// One hour as the recorder writes it: the countries, none where the table is unavailable, and
/// the links (raw id, slot).
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: Option<Vec<Row>>,
    links: Vec<(u32, u32)>,
}

const REFUSAL: &str = "synthetic refusal";

/// Hours 100 to 160 without 130 and 131. AAA's political power rises and falls (a drop each
/// fall) and is unknown from 112 to 114; its command power falls every ten hours. BBB's country
/// changes id at 135, its power drops at 140, and no country holds it at 150 and 151. CCC is
/// renamed CCX at 155. DDD never holds political power. EEE's second holder resolves except from
/// 120 to 124, and its first never does. The countries are unavailable at 145.
fn scenario() -> Vec<Tables> {
    (100..=160)
        .filter(|hour| !(130..=131).contains(hour))
        .map(|hour| {
            let b = if hour >= 135 { 20 } else { 2 };
            let c = if hour >= 155 { "CCX" } else { "CCC" };
            let a = (!(112..=114).contains(&hour)).then(|| 1_000 * i64::from(hour % 7));
            let mut rows = vec![
                (1, 1, "AAA", a, 50 - i64::from(hour / 10)),
                (3, 3, c, Some(100 + i64::from(hour)), 0),
                (4, 4, "DDD", None, 10),
                (5, 5, "EEE", Some(-1), 1),
                (6, 6, "EEE", Some(i64::from(hour % 3)), 2),
            ];
            if !(150..=151).contains(&hour) {
                rows.push((b, 2, "BBB", Some(if hour >= 140 { 400 } else { 500 }), 7));
            }
            rows.sort_unstable_by_key(|row| row.0);
            let mut links = vec![(1, 1), (11, 1), (b, 2), (3, 3), (4, 4)];
            if !(120..=124).contains(&hour) {
                links.push((6, 6));
            }
            links.sort_unstable();
            Tables {
                hour,
                countries: (hour != 145).then_some(rows),
                links,
            }
        })
        .collect()
}

fn country_rows(rows: &[Row]) -> Vec<CountryRow> {
    rows.iter()
        .map(|&(id, canonical_slot, tag, political, command)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            political_power_raw: political,
            command_power_raw: command,
            ..Default::default()
        })
        .collect()
}

fn link_rows(links: &[(u32, u32)]) -> Vec<CountryLinkRow> {
    links
        .iter()
        .map(|&(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .collect()
}

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = source(1, tables.hour, None);
    *frame.tables[TableId::Countries.index()] = match &tables.countries {
        Some(rows) => TableView::Complete(TableRows::Countries(country_rows(rows))),
        None => TableView::Unavailable {
            reason: REFUSAL.into(),
        },
    };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(link_rows(&tables.links)));
    frame
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

/// The projected hours of `hours`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again), as each segment's own projector does,
/// and the row frames of the same hours.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let format = FormatVersion::CURRENT;
    let mut out = Vec::new();
    let mut frames = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(format.version()).unwrap();
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last: &DecodedFrame| last.hour + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        let countries = tables.countries.as_deref().map(country_rows);
        write(walk, TableId::Countries, countries.as_deref());
        write(walk, TableId::CountryLinks, Some(&link_rows(&tables.links)[..]));
        let cold = starts.contains(&tables.hour);
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            if starts {
                projector = Projector::new(format.version()).unwrap();
            }
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
        frames.push(frame(tables));
    }
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(out).unwrap(), frames)
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
/// names one of them, the emit is saved there and goes on from the publication. A resume goes on
/// at a segment's first hour, as the driver resumes.
struct Built {
    pages: Memory,
    root: u64,
}

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
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

fn reply<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    product: &str,
    params: Value,
) -> Value {
    query::query(product, params, snapshot, directory, &AtomicBool::new(false)).unwrap()
}

const NAMES: [&str; 2] = ["politicalPower", "commandPower"];

fn family(measure: &str) -> Value {
    json!({ "kind": "country-measure", "measure": measure })
}

/// Holds the publication `built` to the row history of `frames`: each balance's runs, the holder
/// and segment start of each, and its drops.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let mut scan = HistoryScan::new(false, &[Measure::PoliticalPower, Measure::CommandPower]);
    for frame in frames {
        scan.observe(frame);
    }
    let history = scan.finish();
    let accepted = snapshot.publication().accepted;
    let tags: BTreeSet<String> = directory
        .actors
        .iter()
        .map(|actor| String::from_utf8(actor.name.to_vec()).unwrap())
        .chain(history.countries.keys().cloned())
        .collect();
    for tag in &tags {
        let country = history.countries.get(tag).cloned().unwrap_or_default();
        for (at, balance) in [&country.political_power, &country.command_power]
            .into_iter()
            .enumerate()
        {
            let label = format!("{label}: {tag}'s {}", NAMES[at]);
            let key = directory
                .actor(tag.as_bytes())
                .and_then(|actor| directory.key(actor, MEASURES[at], None));
            let runs: Vec<_> = match key {
                Some(key) => engine::runs(&mut snapshot, key, accepted.first, accepted.last)
                    .unwrap()
                    .into_iter()
                    .map(|run| (run.span.first, run.span.last, run.break_before, run.value))
                    .collect(),
                None => Vec::new(),
            };
            let expected: Vec<_> = balance
                .runs
                .iter()
                .map(|run| {
                    (
                        run.first_hour,
                        run.last_hour,
                        run.starts_segment,
                        i128::from(run.value),
                    )
                })
                .collect();
            assert_eq!(runs, expected, "{label}: runs");
            // Each run's holder is the identity holding the tag then, its segment the latest start.
            let mut segment = None;
            let mut reads = Vec::new();
            let mut holders = Vec::new();
            for run in &balance.runs {
                if run.starts_segment {
                    segment = Some(run.first_hour);
                }
                let identity = country
                    .identities
                    .iter()
                    .find(|identity| {
                        identity.first_hour <= run.first_hour && run.first_hour <= identity.last_hour
                    })
                    .unwrap();
                reads.push(json!({ "tag": tag, "hour": run.first_hour }));
                holders.push(json!([identity.slot, identity.country_id, segment.unwrap()]));
            }
            let state = reply(
                &mut snapshot,
                &directory,
                "series-state",
                json!({ "branchId": "1", "family": family(NAMES[at]), "reads": reads }),
            );
            let published: Vec<Value> = state["states"]
                .as_array()
                .unwrap()
                .iter()
                .map(|state| {
                    let run = &state["run"];
                    json!([run["slot"], run["countryId"], run["segmentFirstHour"]])
                })
                .collect();
            assert_eq!(published, holders, "{label}: holders");
            let page = reply(
                &mut snapshot,
                &directory,
                "events-page",
                json!({
                    "branchId": "1", "family": family(NAMES[at]), "tags": [tag],
                    "cursor": "0", "pageSize": 1000,
                }),
            );
            let drops: Vec<Value> = page["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["row"].clone())
                .collect();
            let expected: Vec<Value> = balance
                .drops
                .iter()
                .map(|drop| {
                    json!({
                        "hour": drop.hour, "beforeRaw": drop.before.to_string(),
                        "afterRaw": drop.after.to_string(),
                    })
                })
                .collect();
            assert_eq!(drops, expected, "{label}: drops");
            assert_eq!(page["total"], balance.drops.len().to_string(), "{label}: drop total");
        }
    }
}

/// Every placement of segment starts, cuts and a resume publishes what the row history reads.
#[test]
fn the_publication_holds_what_the_row_history_reads_across_segments_cuts_and_a_resume() {
    let hours = scenario();
    for (starts, cuts, resume) in [
        (&[][..], &[160][..], None),
        (&[110, 140, 150][..], &[109, 129, 149, 160][..], Some(129)),
        (&[103, 113, 121, 136, 147, 155][..], &[114, 133, 146, 160][..], Some(146)),
        (
            &(101..=160).collect::<Vec<_>>()[..],
            &(100..=160).filter(|hour| !(130..=131).contains(hour)).collect::<Vec<_>>()[..],
            Some(120),
        ),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(&hours, starts);
        let built = build(projected, cuts, resume);
        check(&built, &frames, &label);
    }
}

/// The scenario reads as the top bar shows it: falls are drops within a segment only, unknown
/// balances and unresolved holders break runs, and a balance never held is never zero.
#[test]
fn the_scenario_reads_as_the_top_bar_shows_it() {
    let (projected, _) = projected(&scenario(), &[]);
    let built = build(projected, &[160], None);
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let drops = |snapshot: &mut Snapshot<'_, Memory>, tag: &str, measure: &str| -> Vec<i64> {
        reply(
            snapshot,
            &directory,
            "events-page",
            json!({
                "branchId": "1", "family": family(measure), "tags": [tag], "pageSize": 1000,
            }),
        )["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["hour"].as_i64().unwrap())
            .collect()
    };
    // BBB's fall at 140 is a drop; its new country at 135 and its return at 152 are not.
    assert_eq!(drops(&mut snapshot, "BBB", "politicalPower"), [140]);
    // AAA falls from 6000 to 0 every seventh hour, but not into its unknown hours at 112.
    assert_eq!(
        drops(&mut snapshot, "AAA", "politicalPower"),
        [105, 119, 126, 133, 140, 147, 154]
    );
    let state = reply(
        &mut snapshot,
        &directory,
        "series-state",
        json!({
            "branchId": "1", "family": family("politicalPower"),
            "reads": [
                {"tag": "AAA", "hour": 113}, {"tag": "DDD", "hour": 120},
                {"tag": "EEE", "hour": 122}, {"tag": "EEE", "hour": 125},
                {"tag": "BBB", "hour": 140}, {"tag": "ZZZ", "hour": 140},
            ],
        }),
    );
    let states = state["states"].as_array().unwrap();
    for at in [0, 1, 2, 5] {
        assert!(states[at]["run"].is_null(), "{}", states[at]);
    }
    assert_eq!(states[3]["run"]["countryId"], 6);
    assert_eq!(states[3]["run"]["segmentFirstHour"], 125);
    assert_eq!(states[3]["run"]["startsSegment"], true);
    assert_eq!(states[4]["drop"]["beforeRaw"], "500");
    assert_eq!(states[4]["drop"]["afterRaw"], "400");
    assert_eq!(
        state["warnings"],
        json!([
            "DDD: no recorded political power in this branch",
            "ZZZ: no recorded political power in this branch"
        ])
    );
    // DDD's command power is recorded, so it warns of nothing.
    let command = reply(
        &mut snapshot,
        &directory,
        "series-state",
        json!({
            "branchId": "1", "family": family("commandPower"),
            "reads": [{"tag": "DDD", "hour": 120}],
        }),
    );
    assert_eq!(command["states"][0]["run"]["valueRaw"], "10");
    assert_eq!(command["warnings"], json!([]));
}

/// A page goes on from its merged ordinal, in hour, tag and per-tag order, and a window starting
/// later counts its first ordinal from the branch's first drop.
#[test]
fn drop_pages_resume_by_merged_ordinal() {
    let (projected, _) = projected(&scenario(), &[120]);
    let built = build(projected, &[129, 160], Some(129));
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let page = |snapshot: &mut Snapshot<'_, Memory>, cursor: &str, size: u32, from: Option<i32>| {
        reply(
            snapshot,
            &directory,
            "events-page",
            json!({
                "branchId": "1", "family": family("politicalPower"), "tags": ["EEE", "AAA"],
                "cursor": cursor, "pageSize": size, "from": from, "to": 160,
            }),
        )
    };
    let whole = page(&mut snapshot, "0", 1000, None);
    let rows = whole["rows"].as_array().unwrap().clone();
    assert!(rows.len() > 4);
    let order: Vec<_> = rows
        .iter()
        .map(|row| (row["hour"].as_i64().unwrap(), row["tag"].as_str().unwrap().to_owned()))
        .collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted, "hour, then tag order");
    let mut cursor = "0".to_owned();
    let mut paged = Vec::new();
    loop {
        let next = page(&mut snapshot, &cursor, 3, None);
        assert_eq!(next["firstOrdinal"], cursor);
        paged.extend(next["rows"].as_array().unwrap().iter().cloned());
        match next["cursor"].as_str() {
            Some(next) => cursor = next.to_owned(),
            None => break,
        }
    }
    assert_eq!(paged, rows);
    for tag in ["AAA", "EEE"] {
        let ordinals: Vec<_> = rows
            .iter()
            .filter(|row| row["tag"] == tag)
            .map(|row| row["ordinal"].as_str().unwrap().parse::<u64>().unwrap())
            .collect();
        assert_eq!(ordinals, (0..ordinals.len() as u64).collect::<Vec<_>>(), "{tag}");
    }
    let later = page(&mut snapshot, "0", 1000, Some(140));
    let before = rows
        .iter()
        .filter(|row| row["hour"].as_i64().unwrap() < 140)
        .count();
    assert_eq!(later["firstOrdinal"], before.to_string());
    assert_eq!(later["rows"].as_array().unwrap()[..], rows[before..]);
    assert_eq!(later["total"], (rows.len() - before).to_string());
}

/// Of 1,500 countries, an hour hands the emit only the tags whose balances it wrote, and none
/// when it wrote none.
#[test]
fn an_hour_hands_the_emit_only_the_balances_it_changed() {
    let rows = |hour: i32| -> Vec<CountryRow> {
        (1..=1_500_u32)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("T{id:04}").into_bytes(),
                political_power_raw: Some(if id <= 3 && hour == 2 { 1 } else { 0 }),
                command_power_raw: 5,
                ..Default::default()
            })
            .collect()
    };
    let links: Vec<_> = (1..=1_500_u32)
        .map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        })
        .collect();
    let format = FormatVersion::CURRENT;
    let mut walk = Synthetic::new(facts::TABLES, 1, 0).unwrap();
    let mut world = None;
    let mut projector = Projector::new(format.version()).unwrap();
    let mut out: Vec<facts::Hour> = Vec::new();
    for (ordinal, hour) in (0..).zip(0..4) {
        let countries = rows(hour);
        write(&mut walk, TableId::Countries, Some(&countries[..]));
        write(&mut walk, TableId::CountryLinks, Some(&links[..]));
        walk.end(|state, manifest, decoded, first| {
            let world = dashboard_world::World::at(
                &mut world,
                first,
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
    let handed: Vec<_> = out
        .iter()
        .map(|hour| (hour.whole, hour.countries.len()))
        .collect();
    assert_eq!(handed, [(true, 1_500), (false, 0), (false, 3), (false, 3)]);
}
