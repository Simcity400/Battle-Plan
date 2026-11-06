//! The family's columnar preparation held to its row scan ([`super::observe`]) on the same
//! hours, written as the native recorder writes them: every tag's composite runs, measure
//! runs, presence and ranks, the world's runs, the native owned-state counter and the regional
//! series, across segment starts, a gap, an hour without countries, cuts and a resume.

use super::{
    Builder, Scan, TerritoryRun, columns,
    emit::{
        self, Content, Emit, FACTS, FAMILY, NATIVE_OWNED_STATES, POSITIONS, PRESENT, REGION_KEYS,
        REGIONS, RegionalMeta, SCHEMA, VALUE, VALUE_CHANNELS, WORLD,
    },
    facts::{self, Projector},
    observe, query,
};
use crate::{
    prepared::engine,
    products::{
        country_history::HistoryScan,
        territory_breakdown::{self, Regions, TerritoryRegionRun},
    },
};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use installed_content_province_area::province_area::ProvinceAreas;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryCounterInputRow, CountryLinkRow, CountryRow, ProvinceRow, StateRow, TableId,
        TableRows, TableSchema, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};

const FORMAT: FormatVersion = FormatVersion::CURRENT;
const REFUSAL: &str = "synthetic refusal";

/// The hours of the scenario: 100 to 140 without 120 and 121. Countries are unavailable at
/// 115. AAA (id 1, slot 1, alias link 11), BBB (id 2, then 20 from 125, slot 2), CCC (slot 3)
/// and DDD (slot 4, holding nothing) own two states each, two provinces a state. AAA occupies
/// BBB's state 2 from 105 to 112; CCC controls province 7 from 108 to 116; AAA's alias
/// controls BBB's state 5 from 130; an unknown raw holder owns the unassigned state 6 from 135
/// to 137; province 3's victory points change at 118; province 12 has no map area; CCC is
/// renamed CCX at 133.
fn hours() -> Vec<i32> {
    (100..=140)
        .filter(|hour| !(120..=121).contains(hour))
        .collect()
}

fn frame(hour: i32) -> DecodedFrame {
    let b = if hour >= 125 { 20 } else { 2 };
    let c = if hour >= 133 { "CCX" } else { "CCC" };
    let mut frame = crate::test_fixtures::source(1, hour, None);
    let mut put = |table: TableId, view: TableView| {
        *frame.tables[table.index()] = view;
    };
    let mut countries: Vec<CountryRow> = [(1, 1, "AAA"), (b, 2, "BBB"), (3, 3, c), (4, 4, "DDD")]
        .into_iter()
        .map(|(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        })
        .collect();
    countries.sort_unstable_by_key(|row| row.id);
    put(
        TableId::Countries,
        if hour == 115 {
            TableView::Unavailable {
                reason: REFUSAL.into(),
            }
        } else {
            TableView::Complete(TableRows::Countries(countries))
        },
    );
    let mut links: Vec<CountryLinkRow> = [(1, 1), (b, 2), (3, 3), (4, 4), (11, 1)]
        .into_iter()
        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .collect();
    links.sort_unstable_by_key(|link| link.id);
    put(
        TableId::CountryLinks,
        TableView::Complete(TableRows::CountryLinks(links)),
    );
    let holder = |state: u32| -> (u32, u32) {
        let owner = match state {
            1 | 4 => 1,
            2 | 5 => b,
            6 if (135..=137).contains(&hour) => 99,
            6 => 0,
            _ => 3,
        };
        let controller = match state {
            2 if (105..=112).contains(&hour) => 1,
            5 if hour >= 130 => 11,
            _ => owner,
        };
        (controller, owner)
    };
    put(
        TableId::States,
        TableView::Complete(TableRows::States(
            (1..=6)
                .map(|id| {
                    let (controller, owner) = holder(id);
                    StateRow {
                        id,
                        controller,
                        owner,
                        ..Default::default()
                    }
                })
                .collect(),
        )),
    );
    put(
        TableId::Provinces,
        TableView::Complete(TableRows::Provinces(
            (1..=12)
                .map(|id| {
                    let state = (id - 1) / 2 + 1;
                    let (mut controller, owner) = holder(state);
                    if id == 7 && (108..=116).contains(&hour) {
                        controller = 3;
                    }
                    let victory_points = match id {
                        3 if hour >= 118 => 9,
                        3 => 4,
                        8 => -2,
                        _ => i32::try_from(id % 3).unwrap(),
                    };
                    ProvinceRow {
                        id,
                        state,
                        controller,
                        owner,
                        victory_points,
                        ..Default::default()
                    }
                })
                .collect(),
        )),
    );
    let owned = |slot: u32| (1..=6).filter(|&state| holder(state).1 == slot).count() as i32;
    put(
        TableId::CountryCounterInputs,
        TableView::Complete(TableRows::CountryCounterInputs(
            [1, b, 3]
                .into_iter()
                .map(|country| CountryCounterInputRow {
                    country,
                    owned_state_count: owned(country),
                    ..Default::default()
                })
                .collect(),
        )),
    );
    frame
}

/// Areas of provinces 0 to 11: province 12 has none.
fn areas() -> ProvinceAreas {
    ProvinceAreas {
        land: (0..12).map(|id| 10 + id).collect(),
    }
}

/// Provinces 1 to 10 in regions by their id modulo 3, 11 and 12 in none; states 1 to 4 split
/// over two regions, 5 whole, 6 without a roster.
fn regions() -> Regions {
    let provinces: HashMap<u32, Option<u32>> = (1..=10).map(|id| (id, Some(id % 3))).collect();
    let states = (1..=5)
        .map(|id| {
            let parts = if id == 5 {
                vec![(Some(2), 1, 1)]
            } else {
                vec![(Some(0), 1, 2), (Some(1), 1, 2)]
            };
            (id, parts)
        })
        .collect();
    Regions::synthetic(provinces, states)
}

fn write<T: TableSchema>(walk: &mut Synthetic, input: &DecodedFrame) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match &*input.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        _ => Input::Unavailable {
            reason: REFUSAL,
            read,
            sample,
        },
    };
    walk.table(T::ID, input).unwrap();
}

/// The projected hours of `frames`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again).
fn projected(frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let areas = Some(Arc::new(areas()));
    let regions = Some(Arc::new(regions()));
    let fresh = || Projector::new(FORMAT, areas.clone(), regions.clone()).unwrap();
    let mut projector = fresh();
    let mut last: Option<i32> = None;
    for (ordinal, input) in (0..).zip(frames) {
        let gap = last.is_some_and(|last| last + 1 != input.hour);
        last = Some(input.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, input.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write::<tables::Countries>(walk, input);
        write::<tables::CountryLinks>(walk, input);
        write::<tables::States>(walk, input);
        write::<tables::Provinces>(walk, input);
        write::<tables::CountryCounterInputs>(walk, input);
        let cold = starts.contains(&input.hour);
        if cold {
            projector = fresh();
        }
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(),
                (FORMAT, CaptureMode::Full),
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

fn content() -> Arc<Content> {
    Arc::new(Content {
        areas: true,
        regional: RegionalMeta::default(),
        ..Content::default()
    })
}

struct Built {
    pages: Memory,
    root: u64,
}

/// A publication built from projected hours, cut after each of `cuts`' hours; when `resume`
/// names one of them, the emit is saved there and goes on from the publication.
fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>, budget: &Budget) -> Built {
    let cache = Cache::new(budget);
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(budget, identity(), first).unwrap();
    let mut emit = Emit::new(content());
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
            emit = Emit::load(content(), &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// A run as a key holds it: first and last hour, break, value.
type Raw = (i32, i32, bool, i128);

fn raw<P: dashboard_series::PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    key: Option<u64>,
) -> Vec<Raw> {
    let Some(key) = key else {
        return Vec::new();
    };
    let accepted = snapshot.publication().accepted;
    engine::runs(snapshot, key, accepted.first, accepted.last)
        .unwrap()
        .into_iter()
        .map(|run| (run.span.first, run.span.last, run.break_before, run.value))
        .collect()
}

/// The runs a key holds of `value` of each composite run: equal values going on without a
/// break are one, and a value after an hour of none breaks.
fn expected(runs: &[TerritoryRun], value: impl Fn(&TerritoryRun) -> Option<i64>) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    for run in runs {
        let Some(value) = value(run) else {
            continue;
        };
        let value = i128::from(value);
        let follows = out
            .last()
            .is_some_and(|last| last.1.checked_add(1) == Some(run.first_hour));
        let starts = run.starts_segment || !follows;
        match out.last_mut() {
            Some(last) if !starts && last.3 == value => last.1 = run.last_hour,
            _ => out.push((run.first_hour, run.last_hour, starts, value)),
        }
    }
    out
}

/// The row scan of `frames`, sealed.
fn scanned(frames: &[DecodedFrame]) -> Scan {
    let areas = areas();
    let mut scan = Scan {
        format: FORMAT,
        ..Scan::default()
    };
    let mut builder = Builder::default();
    for frame in frames {
        observe(&mut scan, frame, Some(&areas), &mut builder, None).unwrap();
    }
    builder.seal(&mut scan);
    scan
}

/// Each tag's native owned-state count by hour, as the walk's accumulator carries what the
/// identity history hands it.
fn owned(frames: &[DecodedFrame]) -> BTreeMap<(String, i32), Option<i128>> {
    let mut history = HistoryScan::new(false, &[]);
    let mut held: BTreeMap<String, Option<i128>> = BTreeMap::new();
    let mut out = BTreeMap::new();
    let mut last: Option<i32> = None;
    for frame in frames {
        if last.is_some_and(|last| last + 1 != frame.hour) {
            held.clear();
        }
        last = Some(frame.hour);
        history.observe_owned_states(frame, None, |tag, amount, _| {
            held.insert(tag.to_owned(), amount.map(i128::from));
        });
        for (tag, amount) in &held {
            out.insert((tag.clone(), frame.hour), *amount);
        }
    }
    out
}

/// Holds the publication `built` to the row scan of `frames`.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let scan = scanned(frames);
    let actors = query::actors(&directory);
    let tags: BTreeSet<String> = actors
        .keys()
        .cloned()
        .chain(scan.series.keys().map(|tag| tag.to_string()))
        .collect();
    for tag in &tags {
        let empty = super::Track::default();
        let track = scan.series.get(tag.as_str()).unwrap_or(&empty);
        let actor = actors.get(tag).copied();
        let key =
            |measure, dimension| actor.and_then(|actor| directory.key(actor, measure, dimension));
        let accepted = snapshot.publication().accepted;
        let runs = query::runs(
            &mut snapshot,
            key(FACTS, None),
            accepted.first,
            accepted.last,
        )
        .unwrap();
        assert_eq!(runs, track.runs, "{label}: {tag}'s composite runs");
        for (column, channel) in VALUE_CHANNELS.into_iter().enumerate() {
            assert_eq!(
                raw(&mut snapshot, key(VALUE, Some(channel as u64))),
                expected(&track.runs, |run| run.values[column]),
                "{label}: {tag}'s column {column}"
            );
        }
        let present: Vec<(i32, i32)> = raw(&mut snapshot, key(PRESENT, None))
            .into_iter()
            .map(|(first, last, _, _)| (first, last))
            .collect();
        let held: Vec<(i32, i32)> = track
            .present
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect();
        assert_eq!(present, held, "{label}: {tag}'s presence");
    }
    let accepted = snapshot.publication().accepted;
    let world = query::runs(
        &mut snapshot,
        directory.key(FAMILY, WORLD, None),
        accepted.first,
        accepted.last,
    )
    .unwrap();
    assert_eq!(world, scan.world_totals, "{label}: the world's runs");
    // Each column's positions at every hour, ranked by value descending, ties by tag.
    for frame in frames {
        let hour = frame.hour;
        for column in 0..columns::COUNT {
            let mut ranked: Vec<(i64, &str)> = scan
                .series
                .iter()
                .filter_map(|(tag, track)| {
                    let run = track
                        .runs
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour)?;
                    Some((run.values[column]?, tag.as_ref()))
                })
                .collect();
            ranked.sort_unstable_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(right.1)));
            for position in 0..=ranked.len() {
                let key = directory.key(FAMILY, POSITIONS + column as u64, Some(position as u64));
                let held = key
                    .and_then(|key| engine::run_at(&mut snapshot, key, hour).unwrap())
                    .map(|(run, _)| run.value);
                let expected = ranked
                    .get(position)
                    .map(|(_, tag)| i128::from(actors[*tag]));
                assert_eq!(
                    held, expected,
                    "{label}: column {column} position {position} at {hour}"
                );
            }
        }
    }
    for ((tag, hour), amount) in owned(frames) {
        let key = actors
            .get(&tag)
            .and_then(|&actor| directory.key(actor, VALUE, Some(NATIVE_OWNED_STATES as u64)));
        let held = key
            .and_then(|key| engine::run_at(&mut snapshot, key, hour).unwrap())
            .map(|(run, _)| run.value);
        assert_eq!(held, amount, "{label}: {tag}'s owned states at {hour}");
    }
    let areas = areas();
    let (series, _, _) =
        territory_breakdown::oracle(frames, &regions(), Some(&areas), FORMAT).unwrap();
    let mut published = Vec::new();
    let mut index = 0;
    while let Some(scalar) = directory.scalar(REGION_KEYS + index) {
        let (tag, region, missing, _): (Option<String>, Option<u32>, Option<String>, bool) =
            serde_json::from_slice(scalar.text().unwrap()).unwrap();
        let key = directory.key(FAMILY, REGIONS, Some(index)).unwrap();
        let mut runs: Vec<TerritoryRegionRun> = Vec::new();
        let accepted = snapshot.publication().accepted;
        for marker in engine::runs(&mut snapshot, key, accepted.first, accepted.last).unwrap() {
            let fields =
                query::latest(&mut snapshot, key, marker.span.first, emit::kind::REGION).unwrap();
            runs.push(query::region_run(&mut snapshot, fields, &marker).unwrap());
        }
        published.push((tag, region, missing, runs));
        index += 1;
    }
    published.sort_unstable_by(|a, b| (&a.0, a.1, &a.2).cmp(&(&b.0, b.1, &b.2)));
    let rows: Vec<_> = series
        .into_iter()
        .map(|series| (series.tag, series.region, series.missing, series.runs))
        .collect();
    assert_eq!(
        serde_json::to_value(&published).unwrap(),
        serde_json::to_value(&rows).unwrap(),
        "{label}: the regional series"
    );
}

#[test]
fn the_publication_holds_what_the_row_scan_reads_across_starts_cuts_and_a_resume() {
    let frames: Vec<DecodedFrame> = hours().into_iter().map(frame).collect();
    let budget = Budget::new(64 << 20);
    for (label, starts, cuts, resume) in [
        ("one segment", &[][..], &[140][..], None),
        ("segments", &[106, 113, 126, 134][..], &[140][..], None),
        ("cuts", &[110][..], &[104, 115, 119, 122, 140][..], None),
        ("resume", &[118, 130][..], &[109, 125, 140][..], Some(125)),
    ] {
        let built = build(projected(&frames, starts), cuts, resume, &budget);
        check(&built, &frames, label);
    }
}

/// Of 1,500 countries, an hour moving one province hands the emit only its two holders'
/// values, and an hour moving nothing territory reads hands it nothing.
#[test]
fn an_hour_hands_the_emit_only_the_holders_its_changed_rows_moved() {
    let countries = 1_500u32;
    let at = |hour: i32, moved: bool| {
        let mut frame = crate::test_fixtures::source(1, hour, None);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=countries)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("T{id:04}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=countries)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
            (1..=countries)
                .map(|id| StateRow {
                    id,
                    controller: id,
                    owner: id,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
            (1..=countries)
                .map(|id| ProvinceRow {
                    id,
                    state: id,
                    controller: if moved && id == 7 { 8 } else { id },
                    owner: id,
                    victory_points: 1,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(Vec::new()));
        frame
    };
    let frames = [at(100, false), at(101, true), at(102, true)];
    let hours = projected(&frames, &[]);
    let facts = |at: usize| hours[at].facts.as_deref();
    assert!(facts(0).is_some_and(|facts| facts.values.len() == countries as usize));
    let moved: Vec<u32> = facts(1)
        .unwrap()
        .values
        .iter()
        .map(|(index, _)| *index)
        .collect();
    assert_eq!(moved, [6, 7], "the dense holders of countries 7 and 8");
    assert!(facts(2).is_none(), "an unchanged hour carries nothing");
}
