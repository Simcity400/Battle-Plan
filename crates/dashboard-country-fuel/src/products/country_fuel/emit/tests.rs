//! The publication of synthetic walks against the whole-branch row scan
//! ([`super::super::Scan`]): at every cut, every tag's read, current consumption, consumption
//! segment, levels and days at every walked hour are what the scan of the recording through the
//! cut gives, however the walk is cut into segments, published and resumed; and the charts of
//! the days, dated late with the last day read from the reads, are those of the days kept on
//! the hours they describe. The detail products' windows, consumption segments, applied days
//! and demand packs are the scan's too.

use super::super::tests::{award_row, fuel_frame, fuel_slot};
use super::super::{
    FuelAwardsParams, FuelRead, Scan, ScannedBranch, demand,
    facts::{Projector, TABLES, needs},
    fuel_by_slot, query, resolved_slots, tagged_identities,
};
use super::*;
use dashboard_base::{prepared::engine, products::country_groups::HourSpan};
use dashboard_series::{
    Identity, Publication, Snapshot,
    memory::{Memory, publish},
};
use replay::{
    columnar_frames::synthetic::{Input, Sample as Read, Synthetic},
    segments::{self, Operator, Start},
};
use replay_data::{
    rows::{
        CountryFuelRow, CountryLinkRow, CountryRow, FuelDemandFleet, FuelDemandGroup,
        FuelDemandRead, FuelDemandSite, FuelDemandUnit, FuelDemandWing, Recorded, StateRow,
        TableRows, TableSchema, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::AtomicBool,
};

/// A recorded hour and whether it reads the fuel table.
type Hourly = (DecodedFrame, bool);

fn write<T: TableSchema>(walk: &mut Synthetic, frame: &DecodedFrame, read: bool) {
    let read = read && T::ID == TableId::CountryFuel;
    let sample = if read {
        Read::FromReadBit
    } else {
        Read::Carried(frame.hour - 1)
    };
    let input = match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: None,
            read,
            sample,
        },
        TableView::Truncated { rows, missing } => Input::Snapshot {
            rows: rows.rows::<T>().unwrap(),
            partial: Some(*missing),
            read,
            sample,
        },
        TableView::Unavailable { reason } => Input::Unavailable {
            reason,
            read,
            sample,
        },
        TableView::NotRecorded { .. } => Input::NotRecorded { read, sample },
    };
    walk.table(T::ID, input).unwrap();
}

fn write_hour(walk: &mut Synthetic, frame: &DecodedFrame, read: bool) {
    write::<tables::Campaign>(walk, frame, read);
    write::<tables::Countries>(walk, frame, read);
    write::<tables::CountryLinks>(walk, frame, read);
    write::<tables::States>(walk, frame, read);
    write::<tables::CountryFuel>(walk, frame, read);
    write::<tables::FuelDemandParts>(walk, frame, read);
}

/// GER (9001, slot 7, its fuel row named by link 11, instance 9, one division's demand), ITA
/// and a tag of a country with no fuel row.
fn frame(hour: i32) -> DecodedFrame {
    let mut frame = fuel_frame(hour, false, 9001, (100, 1000, 37, 23));
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 31,
            canonical_slot: 9,
            tag: b"empty".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 31,
            canonical_slot: 9,
        });
    }
    let (pack, parts) = FuelDemandRead {
        country_instance: Some(9),
        divisions: vec![FuelDemandUnit {
            reference: 17,
            rate_raw: Some(i64::from(hour) * 7 - 777),
            ..Default::default()
        }],
        ..Default::default()
    }
    .split(11)
    .unwrap();
    if let TableView::Complete(TableRows::CountryFuel(rows)) =
        &mut *frame.tables[TableId::CountryFuel.index()]
    {
        rows[0].country_instance = Recorded::Value(Some(9));
        rows[0].demand = Recorded::Value(pack);
        rows[0].current_raw = 100 + i64::from(hour % 5);
        rows[0].ring_gain_raw = 37 + i64::from(hour % 3);
    }
    *frame.tables[TableId::FuelDemandParts.index()] =
        TableView::Complete(TableRows::FuelDemandParts(parts));
    frame
}

fn rows<'a>(frame: &'a mut DecodedFrame, table: TableId) -> &'a mut TableRows {
    match &mut *frame.tables[table.index()] {
        TableView::Complete(rows) => rows,
        _ => unreachable!("the fixture's table is complete"),
    }
}

/// Each run's frames normalized as its walk records them, and each run's projected hours with
/// a projector starting a segment at each ordinal of `restarts`.
fn walk(
    runs: &[(u64, Vec<Hourly>)],
    restarts: &BTreeSet<usize>,
) -> (Vec<DecodedFrame>, Vec<facts::Hour>) {
    let format = FormatVersion::CURRENT;
    let mut normalized = Vec::new();
    let mut hours = Vec::new();
    let mut ordinal = 0;
    for (run, (branch, inputs)) in runs.iter().enumerate() {
        let branch = *branch;
        let continues = runs[..run].iter().any(|(earlier, _)| *earlier == branch);
        let mut synthetic = Synthetic::new(TABLES, branch, inputs[0].0.hour).unwrap();
        let mut held = None;
        let mut projector = Projector::new(format);
        projector
            .start(Start::Run { continues }, branch, inputs[0].0.hour)
            .unwrap();
        let mut first = ordinal;
        for (at, (input, read)) in inputs.iter().enumerate() {
            if at > 0 && restarts.contains(&ordinal) {
                projector.end(&mut hours).unwrap();
                projector = Projector::new(format);
                projector
                    .start(Start::Restart(synthetic.state()), branch, input.hour)
                    .unwrap();
                first = ordinal;
            }
            write_hour(&mut synthetic, input, *read);
            synthetic
                .end(|state, manifest, frame, walk_starts| {
                    normalized.push(frame.clone());
                    let world = dashboard_world::World::at(
                        &mut held,
                        walk_starts,
                        &needs(),
                        (format, CaptureMode::Full),
                        state,
                        manifest,
                    )
                    .unwrap();
                    projector
                        .hour(
                            &segments::Hour {
                                branch,
                                hour: frame.hour,
                                run: run as u32,
                                ordinal: at as u32,
                                starts: ordinal == first,
                                opens: ordinal == first,
                                state,
                                manifest,
                                world,
                            },
                            &mut hours,
                        )
                        .unwrap();
                })
                .unwrap();
            ordinal += 1;
        }
        projector.end(&mut hours).unwrap();
    }
    (normalized, hours)
}

fn identity() -> Identity {
    Identity {
        source: [1; 32],
        content: [2; 32],
        branch: [3; 32],
        schema: SCHEMA,
    }
}

/// The publication of `hours` cut after each of `cuts`' hours, with each cut's root; at each of
/// `resume`'s, the emit is saved and goes on from the publication.
fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: &[i32]) -> (Memory, Vec<(i32, u64)>) {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = Emit::new(FormatVersion::CURRENT);
    let mut roots = Vec::new();
    let mut hours = hours.into_iter().peekable();
    for (generation, &cut) in (1..).zip(cuts) {
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            emit.hour(&mut writer, hour).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        let publication = Publication {
            generation,
            segment: generation - 1,
            accepted: Span::new(first, cut).unwrap(),
            reached: cut,
            refusal: None,
        };
        let root = publish(&mut writer, &mut pages, publication, Blob::EMPTY).unwrap();
        roots.push((cut, root));
        if resume.contains(&cut) {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(FormatVersion::CURRENT, &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none(), "every hour is cut");
    (pages, roots)
}

/// What a tag shows at an hour.
#[derive(Clone, Debug, PartialEq)]
struct State {
    fuel: Option<Value>,
    consumption: Option<Value>,
    segment: Option<(i32, i32)>,
    levels: [Option<i128>; 2],
    rates: [Option<i128>; 3],
    days: [Option<i128>; 15],
}

/// What a branch shows: each tag's state at each walked hour, its meta (read, days left out
/// by reason), the countries whose tags are not UTF-8, each native country's tags, whether two
/// held it at once and its tags' reads, the consumption reads the detail products join with
/// each read of a native country, and each read's applied day and demand pack.
#[derive(Debug, PartialEq)]
struct Shown {
    states: BTreeMap<(String, i32), State>,
    meta: BTreeMap<String, (bool, [usize; 3])>,
    unnamed: BTreeSet<u32>,
    natives: Natives,
    joined: Joined,
    details: BTreeMap<(String, i32), String>,
}

/// Each native country's tags, whether two held it at once and its tags' reads.
type Natives = BTreeMap<u32, (BTreeSet<String>, bool, Vec<String>)>;
/// The consumption reads joined with each read of a native country, by country and hour.
type Joined = BTreeMap<(u32, i32), Vec<(String, Value)>>;

fn joined_reads(reads: Vec<(String, demand::FuelConsumptionRead)>) -> Vec<(String, Value)> {
    reads
        .into_iter()
        .map(|(tag, read)| (tag, serde_json::to_value(read).unwrap()))
        .collect()
}

/// What the detail products read of each native country in the whole-branch scan.
fn scanned_natives(scanned: &ScannedBranch) -> (Natives, Joined) {
    let mut natives = BTreeMap::new();
    let mut joined = BTreeMap::new();
    for (country, tags) in &scanned.actor_aliases {
        let number: u32 = country.parse().unwrap();
        let mut reads: Vec<String> = tags
            .iter()
            .flat_map(|tag| {
                scanned.countries[tag].reads.iter().map(move |read| {
                    format!(
                        "{tag} {}",
                        serde_json::to_string(&FuelRead::from(read)).unwrap()
                    )
                })
            })
            .collect();
        reads.sort();
        let mut spans: Vec<(i32, i32)> = tags
            .iter()
            .flat_map(|tag| &scanned.countries[tag].reads)
            .filter(|read| read.country == number)
            .map(|read| (read.first_hour, read.last_hour))
            .collect();
        spans.sort();
        let conflict = spans.windows(2).any(|pair| pair[0].1 >= pair[1].0);
        natives.insert(number, (tags.clone(), conflict, reads));
        for tag in tags {
            for read in &scanned.countries[tag].consumption {
                if read.country != number {
                    continue;
                }
                let basis = FuelAwardsParams {
                    branch_id: "1".into(),
                    tag: tag.clone(),
                    hour: read.first_hour,
                    country: country.clone(),
                    country_instance: 0,
                };
                let reads = demand::actor_consumption(scanned, &basis)
                    .into_iter()
                    .map(|(tag, read)| (tag.to_owned(), demand::FuelConsumptionRead::from(read)))
                    .collect();
                joined.insert((number, read.first_hour), joined_reads(reads));
            }
        }
    }
    (natives, joined)
}

/// The fuel row of `tag`'s country at `frame`, as the scan joins it.
fn level(frame: &DecodedFrame, tag: &str) -> Option<(i64, i64)> {
    let countries = frame.complete::<tables::Countries>().ok()?;
    let links = frame.complete::<tables::CountryLinks>().ok()?;
    let rows = frame.complete::<tables::CountryFuel>().ok()?;
    let slots = resolved_slots(links);
    let keys = dashboard_text::metadata::CountryKeys::read(countries);
    let tagged = tagged_identities(keys.iter(), &slots);
    let (slot, _) = (*tagged.get(tag.as_bytes())?)?;
    let by_slot = fuel_by_slot(rows, |row| row.country, |raw| slots.get(&raw).copied());
    by_slot
        .get(&slot)
        .map(|row| (row.current_raw, row.maximum_raw))
}

/// What the whole-branch scan of `frames` (one branch's, normalized) gives.
fn expected(frames: &[DecodedFrame]) -> Shown {
    let cancelled = AtomicBool::new(false);
    let mut scan = Scan::default();
    for frame in frames {
        scan.observe(frame, FormatVersion::CURRENT, &cancelled)
            .unwrap();
    }
    let scanned = scan.finish(FormatVersion::CURRENT);
    // Each native country's consumption segments, unless two tags held it at once.
    let mut natives: BTreeMap<u32, Vec<(String, &demand::RawConsumption)>> = BTreeMap::new();
    for (tag, country) in &scanned.countries {
        for read in &country.consumption {
            natives
                .entry(read.country)
                .or_default()
                .push((tag.clone(), read));
        }
    }
    let mut segments: BTreeMap<(String, i32), (i32, i32)> = BTreeMap::new();
    for reads in natives.values_mut() {
        reads.sort_by_key(|(_, read)| read.first_hour);
        if reads
            .windows(2)
            .any(|pair| pair[0].1.last_hour >= pair[1].1.first_hour)
        {
            continue;
        }
        let mut start = 0;
        for at in 0..reads.len() {
            let joins = at + 1 < reads.len()
                && reads[at].1.last_hour.checked_add(1) == Some(reads[at + 1].1.first_hour)
                && (reads[at].0 != reads[at + 1].0 || !reads[at + 1].1.starts_segment);
            if !joins {
                let span = (reads[start].1.first_hour, reads[at].1.last_hour);
                for (tag, read) in &reads[start..=at] {
                    for hour in read.first_hour..=read.last_hour {
                        segments.insert((tag.clone(), hour), span);
                    }
                }
                start = at + 1;
            }
        }
    }
    let at: BTreeMap<i32, &DecodedFrame> = frames.iter().map(|frame| (frame.hour, frame)).collect();
    let mut states = BTreeMap::new();
    let mut meta = BTreeMap::new();
    for (tag, country) in &scanned.countries {
        meta.insert(tag.clone(), (!country.reads.is_empty(), country.left_out()));
        for (read, consumption) in country.reads.iter().zip(&country.consumption) {
            let days = read.day.as_ref().map(day_values);
            for hour in read.first_hour..=read.last_hour {
                let Some(frame) = at.get(&hour) else {
                    continue;
                };
                let held = level(frame, tag);
                let mut shown = [None; 15];
                if let Some(days) = days {
                    for (measure, value) in days.into_iter().enumerate() {
                        if measure >= STRIPS || hour == read.first_hour {
                            shown[measure] = value;
                        }
                    }
                }
                states.insert(
                    (tag.clone(), hour),
                    State {
                        fuel: Some(serde_json::to_value(FuelRead::from(read)).unwrap()),
                        consumption: Some(
                            serde_json::to_value(demand::FuelConsumptionRead::from(consumption))
                                .unwrap(),
                        ),
                        segment: segments.get(&(tag.clone(), hour)).copied(),
                        levels: [
                            held.map(|level| i128::from(level.0)),
                            held.map(|level| i128::from(level.1)),
                        ],
                        rates: std::array::from_fn(|family| {
                            consumption.native_daily[family]
                                .is_ok()
                                .then_some(consumption.positive_daily[family])
                        }),
                        days: shown,
                    },
                );
            }
        }
    }
    let (natives, joined) = scanned_natives(&scanned);
    let mut details = BTreeMap::new();
    for (tag, country) in &scanned.countries {
        for (read, detail) in country.reads.iter().zip(&country.details) {
            details.insert((tag.clone(), read.first_hour), format!("{detail:?}"));
        }
    }
    Shown {
        states,
        meta,
        unnamed: scanned.unnamed,
        natives,
        joined,
        details,
    }
}

fn value_at(snapshot: &mut Snapshot<'_, Memory>, key: Option<u64>, hour: i32) -> Option<i128> {
    let key = key?;
    engine::run_at(snapshot, key, hour)
        .unwrap()
        .map(|(run, _)| run.value)
}

/// What the publication at `root` shows at `hours`.
fn shown(pages: &Memory, root: u64, hours: &[i32]) -> Shown {
    let cancelled = AtomicBool::new(false);
    let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let last = snapshot.publication().accepted.last;
    let mut states = BTreeMap::new();
    let mut meta = BTreeMap::new();
    let mut details = BTreeMap::new();
    for (actor, described) in directory.actors.iter().enumerate() {
        let actor = actor as u64;
        if native_of(&described.name).is_some() {
            continue;
        }
        let tag = String::from_utf8(described.name.to_vec()).unwrap();
        meta.insert(tag.clone(), query::tag_meta(&directory, Some(actor)));
        let key = |measure: u64| directory.key(actor, measure, None);
        let mut tails: Vec<Vec<dashboard_base::prepared::series::Run>> = Vec::new();
        for measure in 0..DAY_MEASURES.len() {
            tails.push(match key(DAY + measure as u64) {
                Some(_) => {
                    query::day_tail(&mut snapshot, &directory, actor, measure, &cancelled).unwrap()
                }
                None => Vec::new(),
            });
        }
        let mut firsts = BTreeSet::new();
        for &hour in hours.iter().filter(|&&hour| hour <= last) {
            let Some((read, consumption)) =
                query::read_at(&mut snapshot, &directory, actor, hour).unwrap()
            else {
                continue;
            };
            firsts.insert(read.first_hour);
            let segment = query::segment_at(&mut snapshot, &directory, consumption.country, hour)
                .unwrap()
                .map(|span| (span.first_hour, span.last_hour));
            let mut days = [None; 15];
            for (measure, day) in days.iter_mut().enumerate() {
                *day = if hour + LAG <= last {
                    value_at(&mut snapshot, key(DAY + measure as u64), hour + LAG)
                } else {
                    tails[measure]
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                        .map(|run| run.value)
                };
            }
            states.insert(
                (tag.clone(), hour),
                State {
                    fuel: Some(serde_json::to_value(FuelRead::from(&read)).unwrap()),
                    consumption: Some(
                        serde_json::to_value(demand::FuelConsumptionRead::from(&consumption))
                            .unwrap(),
                    ),
                    segment,
                    levels: [
                        value_at(&mut snapshot, key(FUEL), hour),
                        value_at(&mut snapshot, key(CAPACITY), hour),
                    ],
                    rates: std::array::from_fn(|family| {
                        value_at(&mut snapshot, key(CONSUMPTION + family as u64), hour)
                    }),
                    days,
                },
            );
        }
        let firsts: Vec<i32> = firsts.into_iter().collect();
        let mut packs: BTreeMap<i32, pack::Demand> =
            query::demand_over(&mut snapshot, &directory, actor, &firsts, &cancelled)
                .unwrap()
                .into_iter()
                .collect();
        for &first in &firsts {
            let (_, awards) = query::recorded_at(&mut snapshot, &directory, actor, first)
                .unwrap()
                .unwrap();
            let demand = packs.remove(&first).expect("every read holds a pack");
            details.insert((tag.clone(), first), format!("{:?}", (awards, demand)));
        }
    }
    meta.retain(|_, (read, _)| *read);
    let window = (*hours.iter().min().unwrap(), *hours.iter().max().unwrap());
    let mut natives = BTreeMap::new();
    let mut joined = BTreeMap::new();
    for described in &directory.actors {
        let Some(country) = native_of(&described.name) else {
            continue;
        };
        let Some((_, aliases)) = directory
            .actor(&native_name(country))
            .and_then(|actor| query::native_meta(&directory, actor))
            .filter(|(_, aliases)| !aliases.is_empty())
        else {
            continue;
        };
        let held =
            query::actor_window_in(&mut snapshot, &directory, country, window, &cancelled).unwrap();
        let mut reads: Vec<String> = held
            .reads
            .iter()
            .map(|(tag, read)| {
                format!(
                    "{tag} {}",
                    serde_json::to_string(&FuelRead::from(read)).unwrap()
                )
            })
            .collect();
        reads.sort();
        for (_, read) in held
            .reads
            .iter()
            .filter(|(_, read)| read.country == country)
        {
            let reads = query::actor_consumption_in(
                &mut snapshot,
                &directory,
                country,
                read.first_hour,
                window,
                &cancelled,
            )
            .unwrap();
            joined.insert((country, read.first_hour), joined_reads(reads));
        }
        natives.insert(
            country,
            (aliases.into_iter().collect(), held.conflict, reads),
        );
    }
    Shown {
        states,
        meta,
        unnamed: query::unnamed(&directory),
        natives,
        joined,
        details,
    }
}

/// Where a walk starts segments, and every how many hours it cuts, resuming or not at each cut
/// that ends a segment (as the driver resumes: a fresh projector names its tags anew).
struct Placement {
    label: &'static str,
    restarts: BTreeSet<usize>,
    every: usize,
    resume: bool,
}

fn placements(hours: usize) -> Vec<Placement> {
    vec![
        Placement {
            label: "one segment, one cut",
            restarts: BTreeSet::new(),
            every: usize::MAX,
            resume: false,
        },
        Placement {
            label: "segments every sixth hour, cuts every eighth",
            restarts: (1..hours).filter(|ordinal| ordinal % 6 == 0).collect(),
            every: 8,
            resume: false,
        },
        Placement {
            label: "a segment at every hour, cuts every fifth, resumed at each",
            restarts: (1..hours).collect(),
            every: 5,
            resume: true,
        },
        Placement {
            label: "segments at odd places, cuts every hour, resumed at each segment's end",
            restarts: [1, 5, 7, 13, 22, 23, 30, 31, 47].into(),
            every: 1,
            resume: true,
        },
    ]
}

/// Checks the publication of `runs` (one branch's) against the scan at every cut of every
/// placement.
fn check(label: &str, runs: &[(u64, Vec<Hourly>)]) {
    let total = runs.iter().map(|(_, inputs)| inputs.len()).sum();
    let mut finals: Vec<Shown> = Vec::new();
    for placement in placements(total) {
        let (normalized, projected) = walk(runs, &placement.restarts);
        let walked: Vec<i32> = normalized.iter().map(|frame| frame.hour).collect();
        let mut cuts: Vec<i32> = walked
            .iter()
            .copied()
            .enumerate()
            .filter(|&(at, _)| {
                placement.every != usize::MAX && at % placement.every == placement.every - 1
            })
            .map(|(_, hour)| hour)
            .collect();
        if cuts.last() != walked.last() {
            cuts.push(*walked.last().unwrap());
        }
        let starts: BTreeSet<usize> = runs
            .iter()
            .scan(0, |at, (_, inputs)| {
                let first = *at;
                *at += inputs.len();
                Some(first)
            })
            .chain(placement.restarts.iter().copied())
            .collect();
        let resume: Vec<i32> = walked
            .iter()
            .enumerate()
            .filter(|&(at, hour)| {
                placement.resume && cuts.contains(hour) && starts.contains(&(at + 1))
            })
            .map(|(_, &hour)| hour)
            .collect();
        let (pages, roots) = build(projected, &cuts, &resume);
        for &(cut, root) in &roots {
            let through: Vec<DecodedFrame> = normalized
                .iter()
                .filter(|frame| frame.hour <= cut)
                .cloned()
                .collect();
            let mut expected = expected(&through);
            expected.meta.retain(|_, (read, _)| *read);
            let actual = shown(&pages, root, &walked);
            let at = format!("{label}, {}, cut at {cut}", placement.label);
            for (place, state) in &expected.states {
                assert_eq!(actual.states.get(place), Some(state), "{at}: {place:?}");
            }
            assert_eq!(actual.states.len(), expected.states.len(), "{at}");
            assert_eq!(actual.meta, expected.meta, "{at}");
            assert_eq!(actual.unnamed, expected.unnamed, "{at}");
            assert_eq!(actual.natives, expected.natives, "{at}");
            assert_eq!(actual.joined, expected.joined, "{at}");
            for (read, detail) in &expected.details {
                assert_eq!(actual.details.get(read), Some(detail), "{at}: {read:?}");
            }
            assert_eq!(actual.details.len(), expected.details.len(), "{at}");
        }
        finals.push(shown(&pages, roots.last().unwrap().1, &walked));
    }
    for other in &finals[1..] {
        assert_eq!(*other, finals[0], "{label}: the same however it is walked");
    }
}

/// One run of `inputs` on branch 1, a new run at each gap.
fn runs_of(inputs: Vec<Hourly>) -> Vec<(u64, Vec<Hourly>)> {
    let mut runs: Vec<(u64, Vec<Hourly>)> = Vec::new();
    for input in inputs {
        match runs.last_mut() {
            Some((_, run)) if run.last().unwrap().0.hour + 1 == input.0.hour => run.push(input),
            _ => runs.push((1, vec![input])),
        }
    }
    runs
}

#[test]
fn days_crossing_cuts_keep_ownership_and_overlaps_as_the_scan_decides_them() {
    let slot = fuel_slot(100);
    let inputs: Vec<Hourly> = (slot - 30..=slot + 75)
        .map(|hour| {
            let mut input = frame(hour);
            if (slot + 30..slot + 35).contains(&hour)
                && let TableRows::States(states) = rows(&mut input, TableId::States)
            {
                states.retain(|row| row.owner != 11);
            }
            let read = (hour - slot).rem_euclid(24) == 0
                || [slot - 2, slot + 2, slot + 25, slot + 47].contains(&hour);
            (input, read)
        })
        .collect();
    check("boundary days", &runs_of(inputs));
}

#[test]
fn a_refresh_before_the_slot_is_overlapped_by_the_slot_read_after_it() {
    let slot = fuel_slot(1000);
    // The read a day before the slot lacks its preceding day; the refresh an hour before the
    // next slot has one, which the slot's own read then overlaps.
    let inputs: Vec<Hourly> = (slot - 25..=slot + 30)
        .map(|hour| {
            let mut input = frame(hour);
            if let TableRows::CountryFuel(fuel) = rows(&mut input, TableId::CountryFuel) {
                fuel[0].ring_consumed_raw = 29 + i64::from(hour % 7);
            }
            (
                input,
                [slot - 24, slot - 1, slot, slot + 24].contains(&hour),
            )
        })
        .collect();
    check("refresh before the slot", &runs_of(inputs));
}

#[test]
fn every_refused_table_is_unknown_and_a_walk_can_start_at_it() {
    let start = fuel_slot(100);
    for table in TABLES
        .iter()
        .copied()
        .filter(|table| *table != TableId::Campaign)
    {
        for partial in [false, true] {
            let mut refused = frame(start + 2);
            let TableView::Complete(held) = &*refused.tables[table.index()] else {
                unreachable!()
            };
            refused.tables[table.index()] = if partial {
                TableView::Truncated {
                    rows: held.clone(),
                    missing: 1,
                }
            } else {
                TableView::Unavailable {
                    reason: "synthetic refusal".into(),
                }
            }
            .into();
            let mut inputs: Vec<Hourly> = (start..=start + 50)
                .map(|hour| {
                    (
                        frame(hour),
                        hour == start || hour == start + 24 || hour == start + 48,
                    )
                })
                .collect();
            inputs[2] = (refused, true);
            check(
                &format!("{table:?} refused, partial {partial}"),
                &runs_of(inputs),
            );
        }
    }
}

#[test]
fn aliases_identity_changes_unresolved_owners_and_removals_follow_the_scan() {
    let start = fuel_slot(100);
    let inputs: Vec<Hourly> = (start..=start + 58)
        .map(|hour| {
            let mut input = frame(hour);
            if (start + 4..start + 9).contains(&hour)
                && let TableRows::CountryLinks(links) = rows(&mut input, TableId::CountryLinks)
            {
                links.retain(|row| row.id != 11);
            }
            if (start + 10..start + 15).contains(&hour)
                && let TableRows::Countries(countries) = rows(&mut input, TableId::Countries)
            {
                countries.retain(|row| row.tag != b"GER");
            }
            if (start + 16..start + 20).contains(&hour)
                && let TableRows::States(states) = rows(&mut input, TableId::States)
            {
                states.push(StateRow {
                    id: 4,
                    owner: 123456,
                    ..Default::default()
                });
            }
            if hour >= start + 21 {
                if let TableRows::Countries(countries) = rows(&mut input, TableId::Countries) {
                    countries
                        .iter_mut()
                        .find(|row| row.tag == b"GER")
                        .unwrap()
                        .id = 9002;
                    countries.push(CountryRow {
                        id: 44,
                        canonical_slot: 7,
                        tag: b"alias".to_vec(),
                        ..Default::default()
                    });
                }
                if let TableRows::CountryLinks(links) = rows(&mut input, TableId::CountryLinks) {
                    links.push(CountryLinkRow {
                        id: 9002,
                        canonical_slot: 7,
                    });
                    links.push(CountryLinkRow {
                        id: 44,
                        canonical_slot: 7,
                    });
                }
            }
            if hour >= start + 30 {
                if let TableRows::CountryLinks(links) = rows(&mut input, TableId::CountryLinks) {
                    links.push(CountryLinkRow {
                        id: 12,
                        canonical_slot: 7,
                    });
                }
                if let TableRows::CountryFuel(fuel) = rows(&mut input, TableId::CountryFuel) {
                    fuel.push(CountryFuelRow {
                        country: 12,
                        current_raw: 888,
                        maximum_raw: 999,
                        ..Default::default()
                    });
                    if hour >= start + 35 {
                        fuel.retain(|row| row.country != 11);
                    }
                }
            }
            (input, (hour - start).rem_euclid(6) == 0)
        })
        .collect();
    check("identities and owners", &runs_of(inputs));
}

#[test]
fn awarded_days_and_their_reasons_carry_across_gaps_and_runs() {
    let start = fuel_slot(100);
    let inputs: Vec<Hourly> = (start..=start + 76)
        .filter(|hour| !(start + 27..start + 32).contains(hour))
        .map(|hour| {
            let mut input = frame(hour);
            if let TableRows::CountryFuel(fuel) = rows(&mut input, TableId::CountryFuel) {
                let demand = fuel[0].demand.clone();
                fuel[0] = award_row(hour);
                fuel[0].demand = demand;
                if hour >= start + 40
                    && let Recorded::Value(day) = &mut fuel[0].award_day
                {
                    day.continuity_start_hour = Some(start + 17);
                }
            }
            (
                input,
                (hour - start).rem_euclid(12) == 0 || hour == start + 32,
            )
        })
        .collect();
    check("awards and runs", &runs_of(inputs));
}

#[test]
fn an_off_slot_read_a_cession_and_a_gap_settle_as_the_scan_does() {
    // Reads at the slot every day, and off it at +30 (a day after the first, inside a day of
    // the second); the second country's state passes to the first at +47; +90 and +91 are a gap.
    let base = fuel_slot(1000) - 17;
    let inputs: Vec<Hourly> = (0..=121)
        .filter(|offset| ![90, 91].contains(offset))
        .map(|offset| {
            let hour = base + offset;
            let mut input = frame(hour);
            if hour >= base + 47
                && let TableRows::States(states) = rows(&mut input, TableId::States)
            {
                for state in states.iter_mut() {
                    state.owner = 11;
                }
            }
            let read = (offset - 17).rem_euclid(24) == 0 || offset == 30;
            (input, read)
        })
        .collect();
    check("off-slot read and cession", &runs_of(inputs));
}

#[test]
fn changed_parts_and_full_status_changes_keep_their_reads() {
    let start = fuel_slot(100);
    let inputs: Vec<Hourly> = (start..=start + 55)
        .map(|hour| {
            let mut input = frame(hour);
            if hour >= start + 5
                && let TableRows::FuelDemandParts(parts) =
                    rows(&mut input, TableId::FuelDemandParts)
                && let replay_data::rows::FuelDemandPart::Division(unit) = &mut parts[0].part
            {
                unit.rate_raw = Some(1234);
            }
            if (start + 7..start + 11).contains(&hour) {
                let TableView::Complete(held) = &*input.tables[TableId::FuelDemandParts.index()]
                else {
                    unreachable!()
                };
                input.tables[TableId::FuelDemandParts.index()] = TableView::Truncated {
                    rows: held.clone(),
                    missing: if hour < start + 9 { 1 } else { 2 },
                }
                .into();
            }
            if hour >= start + 12 {
                if let TableRows::Countries(countries) = rows(&mut input, TableId::Countries) {
                    countries.push(CountryRow {
                        id: 99,
                        canonical_slot: 10,
                        tag: vec![0xff],
                        ..Default::default()
                    });
                }
                if let TableRows::CountryLinks(links) = rows(&mut input, TableId::CountryLinks) {
                    links.push(CountryLinkRow {
                        id: 99,
                        canonical_slot: 10,
                    });
                }
            }
            let read = [
                start,
                start + 6,
                start + 8,
                start + 10,
                start + 11,
                start + 24,
                start + 48,
            ]
            .contains(&hour);
            (input, read)
        })
        .collect();
    check("parts and equal reads", &runs_of(inputs));
}

#[test]
fn later_runs_opening_with_refused_joins_start_cold() {
    let start = fuel_slot(100);
    for refused in [TableId::Countries, TableId::CountryLinks] {
        let inputs: Vec<Hourly> = [
            start,
            start + 1,
            start + 8,
            start + 9,
            start + 10,
            start + 35,
        ]
        .into_iter()
        .map(|hour| {
            let mut input = frame(hour);
            if hour >= start + 8
                && let TableRows::CountryFuel(fuel) = rows(&mut input, TableId::CountryFuel)
            {
                fuel[0].current_raw = 12345;
            }
            if hour == start + 8 {
                *input.tables[refused.index()] = TableView::Unavailable {
                    reason: "synthetic run opening refusal".into(),
                };
            }
            (input, true)
        })
        .collect();
        check(&format!("later run, {refused:?} refused"), &runs_of(inputs));
    }
}

/// A plain publication of `runs`, each written on the hours it describes, cut once at `last`.
/// GER's demand pack at walked hour `t` of a pack test: divisions sliding in and out, a rate
/// moving, task forces coming and going, air sites gaining wings, commands and a fleet
/// renamed; every fourth hour the pack before it again, and the one at hour 45 refused.
fn varied(t: i32) -> FuelDemandRead {
    let t = (if t % 4 == 2 { t - 1 } else { t }) as u64;
    let unit = |reference: u64, rate: Option<i64>| FuelDemandUnit {
        reference,
        parent: reference / 7,
        ordinal: (reference % 5) as u32,
        owners: (reference % 2 == 0).then_some([11, 9, 11]),
        rate_raw: rate,
        reason: if rate.is_none() {
            b"no rate".to_vec()
        } else {
            Vec::new()
        },
        name: (reference % 3 != 0).then(|| format!("Division {reference}").into_bytes()),
        group: Some(70),
        ..Default::default()
    };
    FuelDemandRead {
        country_instance: Some(9),
        divisions: (t / 3..t / 3 + 4 + t % 3)
            .map(|reference| {
                unit(
                    reference,
                    Some(reference as i64 * 5 + (reference == t) as i64),
                )
            })
            .collect(),
        task_forces: if t % 5 == 0 {
            Vec::new()
        } else {
            vec![unit(900, None)]
        },
        sites: (0..2)
            .map(|site| FuelDemandSite {
                list: site,
                ordinal: u32::from(site),
                base: 5000 + u64::from(site),
                container_owner: Some(11),
                capacity_raw: Some(100),
                wings: (0..1 + (t + u64::from(site)) % 3)
                    .map(|index| FuelDemandWing {
                        reference: 60 + index,
                        owner: Some(11),
                        pool_ordinal: 1,
                        wing_ordinal: index as u32,
                        name: Some(b"Wing".to_vec()),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .collect(),
        groups: vec![FuelDemandGroup {
            reference: 70,
            country: 11,
            members: (0..t % 4).collect(),
            ..Default::default()
        }],
        fleets: vec![FuelDemandFleet {
            reference: 80,
            name: (t % 10 != 0).then(|| b"Fleet".to_vec()),
            ..Default::default()
        }],
        // One refusal, late enough that a chain first runs past its keyframes.
        reason: if t == 45 {
            b"fuel demand pack refused".to_vec()
        } else {
            Vec::new()
        },
        ..Default::default()
    }
}

#[test]
fn demand_chains_rebuild_every_reads_pack_across_cuts_and_resumes() {
    let start = fuel_slot(100);
    let inputs: Vec<Hourly> = (0..60)
        .filter(|t| *t != 41)
        .map(|t| {
            let mut input = frame(start + t);
            let (pack, parts) = varied(t).split(11).unwrap();
            if let TableRows::CountryFuel(rows) = rows(&mut input, TableId::CountryFuel) {
                rows[0].demand = Recorded::Value(pack);
            }
            *input.tables[TableId::FuelDemandParts.index()] =
                TableView::Complete(TableRows::FuelDemandParts(parts));
            (input, true)
        })
        .collect();
    check("demand chains", &runs_of(inputs));
}

#[test]
fn a_tag_no_longer_read_leaves_its_pack_out_of_the_saved_emit() {
    let start = fuel_slot(100);
    let inputs: Vec<Hourly> = (0..=48)
        .map(|t| {
            let mut input = frame(start + t);
            let (pack, parts) = varied(1).split(11).unwrap();
            if let TableRows::CountryFuel(rows) = rows(&mut input, TableId::CountryFuel) {
                rows[0].demand = Recorded::Value(pack);
                if t >= 30 {
                    rows.clear();
                }
            }
            *input.tables[TableId::FuelDemandParts.index()] =
                TableView::Complete(TableRows::FuelDemandParts(parts));
            (input, t % 24 == 0)
        })
        .collect();
    let (_, projected) = walk(&runs_of(inputs), &BTreeSet::new());
    let mut writer = Writer::new(identity(), start).unwrap();
    let mut emit = Emit::new(FormatVersion::CURRENT);
    let mut saved = BTreeMap::new();
    for hour in projected {
        let at = hour.hour - start;
        emit.hour(&mut writer, hour).unwrap();
        let mut bytes = Vec::new();
        emit.save(&mut bytes).unwrap();
        let actor = emit.actors.find(b"GER").unwrap();
        let Entity::Tag(tag) = &emit.entities[actor as usize] else {
            panic!("GER is a tag");
        };
        saved.insert(at, (bytes, tag.stream.is_some()));
    }
    let mut keyframe = Vec::new();
    pack::keyframe(
        &pack::Encoded::of(&Ok(pack::Pack::of(&varied(1)))).unwrap(),
        &mut keyframe,
    );
    let holds = |bytes: &[u8]| bytes.windows(keyframe.len()).any(|at| at == keyframe);
    assert!(saved[&24].1, "a tag still read keeps its chain");
    assert!(holds(&saved[&24].0), "the saved emit holds its pack");
    assert!(!saved[&48].1, "a tag no read supplied leaves it");
    assert!(!holds(&saved[&48].0), "the saved emit holds no pack of it");
}

fn plain(runs: &[(u64, Vec<(i32, i32, i128, bool)>)], first: i32, last: i32) -> (Memory, u64) {
    let mut pages = Memory::default();
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut actors = Actors::new(SCHEMA);
    for (key, pieces) in runs {
        let actor = actors
            .actor(&mut writer, format!("K{key}").as_bytes(), first)
            .unwrap();
        let key = actors
            .key(
                &mut writer,
                Place {
                    actor,
                    measure: 0,
                    dimension: None,
                },
                (RunMode::Exact, 0),
                Some(WALKED),
            )
            .unwrap();
        for &(from, to, value, broken) in pieces {
            writer
                .run(key, Span::new(from, to).unwrap(), value, broken, false)
                .unwrap();
        }
    }
    let known = Evidence {
        span: Span::new(first, last).unwrap(),
        known: Knownness::Known,
        supplying_read: None,
        holder: None,
        identity_first: None,
    };
    writer.domain(WALKED, known).unwrap();
    let publication = Publication {
        generation: 1,
        segment: 0,
        accepted: Span::new(first, last).unwrap(),
        reached: last,
        refusal: None,
    };
    let root = publish(&mut writer, &mut pages, publication, Blob::EMPTY).unwrap();
    (pages, root)
}

/// Every day measure's chart of each tag, dated late with the last day read from its reads, is
/// the chart of the days kept on the hours they describe, in every view and membership.
#[test]
fn day_charts_are_the_charts_of_the_kept_days_on_their_hours() {
    let base = fuel_slot(1000) - 17;
    let inputs: Vec<Hourly> = (0..=121)
        .filter(|offset| ![60, 61].contains(offset))
        .map(|offset| {
            let hour = base + offset;
            let mut input = frame(hour);
            if (40..44).contains(&offset)
                && let TableRows::States(states) = rows(&mut input, TableId::States)
            {
                states.retain(|row| row.owner != 11);
            }
            let read = (offset - 17).rem_euclid(24) == 0 || [30, 100].contains(&offset);
            (input, read)
        })
        .collect();
    let runs = runs_of(inputs);
    let total: usize = runs.iter().map(|(_, inputs)| inputs.len()).sum();
    // Each cut ends a segment, so the emit resumes there with a fresh projector.
    let ends: BTreeSet<usize> = (0..total).step_by(7).chain([total - 1]).collect();
    let restarts = ends.iter().map(|at| at + 1).collect();
    let (normalized, projected) = walk(&runs, &restarts);
    let walked: Vec<i32> = normalized.iter().map(|frame| frame.hour).collect();
    let first = walked[0];
    let cuts: Vec<i32> = ends.iter().map(|&at| walked[at]).collect();
    let (pages, roots) = build(projected, &cuts, &cuts);
    let cancelled = AtomicBool::new(false);
    for &(cut, root) in &roots {
        let through: Vec<DecodedFrame> = normalized
            .iter()
            .filter(|frame| frame.hour <= cut)
            .cloned()
            .collect();
        let cancel = AtomicBool::new(false);
        let mut scan = Scan::default();
        for frame in &through {
            scan.observe(frame, FormatVersion::CURRENT, &cancel)
                .unwrap();
        }
        let scanned = scan.finish(FormatVersion::CURRENT);
        let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        for (tag, country) in &scanned.countries {
            let actor = directory.actor(tag.as_bytes());
            for measure in 0..DAY_MEASURES.len() {
                let mut pieces = Vec::new();
                for read in &country.reads {
                    let Some(value) = read.day.as_ref().and_then(|day| day_values(day)[measure])
                    else {
                        continue;
                    };
                    let to = if measure < STRIPS {
                        read.first_hour
                    } else {
                        read.last_hour
                    };
                    pieces.push((read.first_hour, to, value, read.starts_segment));
                }
                let (reference, reference_root) = plain(&[(0, pieces)], first, cut);
                let mut referenced =
                    Snapshot::open(reference, reference_root, identity()).unwrap();
                let reference_directory = Directory::read(&mut referenced, SCHEMA).unwrap();
                let key = reference_directory
                    .actor(b"K0")
                    .and_then(|actor| reference_directory.key(actor, 0, None))
                    .unwrap();
                for (from, to, width) in [
                    (f64::from(first), f64::from(cut), 2000),
                    (f64::from(first) + 0.4, f64::from(cut) + 0.7, 7),
                    (f64::from(cut - 30), f64::from(cut), 31),
                    (f64::from(cut - 3), f64::from(cut + 5), 9),
                ] {
                    for within in [
                        None,
                        Some(vec![HourSpan {
                            first_hour: cut - 40,
                            last_hour: cut - 2,
                        }]),
                    ] {
                        let at = format!(
                            "{tag} {} at {cut}, {from}..{to}/{width}, {within:?}",
                            DAY_MEASURES[measure]
                        );
                        let expected = engine::summary(
                            &mut referenced,
                            key,
                            (first, cut),
                            (from, to, width),
                            0,
                            within.as_deref(),
                        )
                        .unwrap();
                        let actual = query::day_summary(
                            &mut snapshot,
                            &directory,
                            actor,
                            measure,
                            (first, cut),
                            (from, to, width),
                            within.as_deref(),
                            &cancelled,
                        )
                        .unwrap();
                        assert_eq!(
                            serde_json::to_value(&actual).unwrap(),
                            serde_json::to_value(&expected).unwrap(),
                            "{at}"
                        );
                    }
                }
            }
        }
    }
}

/// Fifteen hundred countries read at once write one read each, and an hour no row or owner
/// changes writes nothing.
#[test]
fn fifteen_hundred_countries_write_reads_only_where_they_are_read() {
    let start = fuel_slot(100);
    let mut first = dashboard_base::test_fixtures::scale_frame(1, start, 1500);
    *first.tables[TableId::FuelDemandParts.index()] =
        TableView::Complete(TableRows::FuelDemandParts(Vec::new()));
    *first.tables[TableId::CountryFuel.index()] = TableView::Complete(TableRows::CountryFuel(
        (0..1500)
            .map(|index| CountryFuelRow {
                country: 100_000 + index,
                current_raw: i64::from(index),
                maximum_raw: 9000,
                ..Default::default()
            })
            .collect(),
    ));
    let inputs: Vec<Hourly> = (0..5)
        .map(|ordinal| {
            let mut input = first.clone();
            input.hour += ordinal;
            if ordinal >= 3
                && let TableRows::CountryFuel(fuel) = rows(&mut input, TableId::CountryFuel)
            {
                fuel[777].current_raw += 1;
            }
            (input, ordinal == 0)
        })
        .collect();
    let (_, projected) = walk(&runs_of(inputs), &BTreeSet::new());
    assert_eq!(
        projected
            .iter()
            .map(facts::Hour::changed_countries)
            .collect::<Vec<_>>(),
        [1500, 0, 0, 1, 0]
    );
    let mut writer = Writer::new(identity(), start).unwrap();
    let mut emit = Emit::new(FormatVersion::CURRENT);
    let mut held = Vec::new();
    for hour in projected {
        emit.hour(&mut writer, hour).unwrap();
        held.push(writer.held_bytes());
    }
    assert!(held[0] > 1500 * 21 * 17, "every country's read: {held:?}");
    assert_eq!(held[1], held[0], "a quiet hour writes nothing");
    assert_eq!(held[2], held[0], "a quiet hour writes nothing");
    assert!(
        held[3] > held[2] && held[3] - held[2] < 1024,
        "one level moves: {held:?}"
    );
    assert_eq!(held[4], held[3], "a quiet hour writes nothing");
}

#[test]
fn a_damaged_saved_emit_is_refused() {
    let start = fuel_slot(100);
    let inputs: Vec<Hourly> = (start..=start + 3)
        .map(|hour| (frame(hour), hour == start))
        .collect();
    let (_, projected) = walk(&runs_of(inputs), &BTreeSet::new());
    let (pages, roots) = build(projected, &[start + 3], &[]);
    let mut snapshot = Snapshot::open(pages, roots[0].1, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let mut emit = Emit::new(FormatVersion::CURRENT);
    let mut writer = Writer::new(identity(), start).unwrap();
    let inputs: Vec<Hourly> = (start..=start + 3)
        .map(|hour| (frame(hour), hour == start))
        .collect();
    for hour in walk(&runs_of(inputs), &BTreeSet::new()).1 {
        emit.hour(&mut writer, hour).unwrap();
    }
    let mut saved = Vec::new();
    emit.save(&mut saved).unwrap();
    assert!(Emit::load(FormatVersion::CURRENT, &directory, &saved).is_ok());
    for cut in [1, 9, saved.len() / 2, saved.len() - 1] {
        assert!(
            Emit::load(FormatVersion::CURRENT, &directory, &saved[..cut]).is_err(),
            "cut at {cut}"
        );
    }
}
