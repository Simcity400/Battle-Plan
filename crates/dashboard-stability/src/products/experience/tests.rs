//! The family's columnar preparation held to the row scan ([`Scan`]) on the same hours, written
//! as the native recorder writes them, across segment starts, a gap, cuts and a resume; the
//! regressions the row preparation kept; and what a quiet hour hands on.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    query::query,
};
use dashboard_base::products::country_experience::{
    ExperienceRead, RawRead, Scan, UNITS,
    test_fixtures::{renamed_frame, slot_base},
};
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
    rows::{
        CountryExperienceGainRow, CountryExperienceRow, CountryLinkRow, CountryRow, RowCodec,
        TableId, tables,
    },
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

const REFUSAL: &str = "synthetic refusal";

/// One hour's tables as the recorder writes them; None where a table is unavailable.
#[derive(Clone, Default)]
struct Tables {
    hour: i32,
    countries: Option<Vec<CountryRow>>,
    links: Option<Vec<CountryLinkRow>>,
    banked: Option<Vec<CountryExperienceRow>>,
    gains: Option<Vec<CountryExperienceGainRow>>,
}

impl Tables {
    /// A row fixture's hour.
    fn of(frame: &DecodedFrame) -> Self {
        let mut countries = frame
            .complete::<tables::Countries>()
            .ok()
            .map(<[_]>::to_vec);
        let mut links = frame
            .complete::<tables::CountryLinks>()
            .ok()
            .map(<[_]>::to_vec);
        let mut banked = frame
            .complete::<tables::CountryExperience>()
            .ok()
            .map(<[_]>::to_vec);
        let mut gains = frame
            .complete::<tables::CountryExperienceGains>()
            .ok()
            .map(<[_]>::to_vec);
        countries
            .iter_mut()
            .for_each(|rows| rows.sort_by_key(|row| row.id));
        links
            .iter_mut()
            .for_each(|rows| rows.sort_by_key(|row| row.id));
        banked
            .iter_mut()
            .for_each(|rows| rows.sort_by_key(|row| row.country));
        gains
            .iter_mut()
            .for_each(|rows| rows.sort_by_key(|row| (row.country, row.instance)));
        Self {
            hour: frame.hour,
            countries,
            links,
            banked,
            gains,
        }
    }
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>) {
    let (read, sample) = (false, Sample::FromReadBit);
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
/// walk's first hour (after a gap, the walk starts again), and the row decoder's frames of the
/// same hours.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
    let mut last: Option<i32> = None;
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = last.is_some_and(|last| last + 1 != tables.hour);
        last = Some(tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, TableId::Countries, tables.countries.as_deref());
        write(walk, TableId::CountryLinks, tables.links.as_deref());
        write(walk, TableId::CountryExperience, tables.banked.as_deref());
        write(
            walk,
            TableId::CountryExperienceGains,
            tables.gains.as_deref(),
        );
        let cold = starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT).unwrap();
        }
        let frame = walk
            .end(|state, manifest, decoded, first| {
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
                decoded.clone()
            })
            .unwrap();
        frames.push(frame);
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
            emit = Emit::resume(&directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// `product`'s reply to `params` from `built`.
fn ask(built: &Built, product: &str, params: Value) -> Value {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    query(
        product,
        params,
        &mut snapshot,
        &directory,
        &AtomicBool::new(false),
    )
    .unwrap()
}

/// Each tag's reads as the row product shows them: the row scan stepped over every frame.
fn oracle(frames: &[DecodedFrame]) -> BTreeMap<String, Vec<ExperienceRead>> {
    let mut scan = Scan::default();
    let mut reads: BTreeMap<String, Vec<RawRead>> = BTreeMap::new();
    for frame in frames {
        scan.step(frame, FormatVersion::CURRENT, None, |tag, _, read| {
            let held = reads.entry(tag.to_owned()).or_default();
            match held.last_mut() {
                Some(previous) if previous.first_hour == read.first_hour => *previous = read,
                _ => held.push(read),
            }
            Ok(())
        })
        .unwrap();
    }
    reads
        .into_iter()
        .map(|(tag, reads)| (tag, reads.iter().map(ExperienceRead::from).collect()))
        .collect()
}

/// The renamed fixture's hours 0 to 96 without 16 and 54: renames, an alias's reuse of a tag, a
/// gain day and gaps.
fn renamed() -> Vec<Tables> {
    (0..=96)
        .filter(|hour| ![16, 54].contains(hour))
        .map(|hour| Tables::of(&renamed_frame(hour)))
        .collect()
}

/// Every placement of segment starts, cuts and a resume answers each tag's banked, gain and
/// change at every hour, and the read supplying it, as the row scan reads them.
#[test]
fn replies_hold_the_row_scans_reads_across_segments_gaps_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let base = slot_base();
    let hours = renamed();
    let at = |offsets: &[i32]| {
        offsets
            .iter()
            .map(|offset| base + offset)
            .collect::<Vec<_>>()
    };
    for (starts, cuts, resume) in [
        (at(&[]), at(&[96]), None),
        (at(&[5, 25, 49, 73]), at(&[24, 48, 96]), Some(base + 48)),
        (
            at(&[12, 13, 30, 50, 90]),
            at(&[15, 47, 60, 96]),
            Some(base + 15),
        ),
        (
            at(&(1..=96).collect::<Vec<_>>()),
            at(&(0..=96)
                .filter(|hour| ![16, 54].contains(hour))
                .collect::<Vec<_>>()),
            Some(base + 72),
        ),
    ] {
        let label = format!("starts {starts:?}");
        let (projected, frames) = projected(&hours, &starts);
        let expected = oracle(&frames);
        let built = build(projected, &cuts, resume, &budget);
        for tag in ["opaque!tag", "renamed", "third"] {
            for offset in 0..=96 {
                let hour = base + offset;
                let read = expected
                    .get(tag)
                    .into_iter()
                    .flatten()
                    .find(|read| read.first_hour <= hour && hour <= read.last_hour);
                let country = frames
                    .iter()
                    .find(|frame| frame.hour == hour)
                    .and_then(|frame| {
                        frame
                            .complete::<tables::Countries>()
                            .unwrap()
                            .iter()
                            .find(|country| country.tag == tag.as_bytes())
                    });
                let key = serde_json::to_string(&(
                    tag,
                    country.map_or(0, |row| row.canonical_slot),
                    country.map_or(0, |row| row.id),
                ))
                .unwrap();
                let modes = [
                    (
                        "banked",
                        read.map(|read| [&read.army_raw, &read.navy_raw, &read.air_raw]),
                    ),
                    (
                        "gain",
                        read.and_then(|read| read.gain.as_ref())
                            .map(|gain| [&gain.army_raw, &gain.navy_raw, &gain.air_raw]),
                    ),
                    (
                        "change",
                        read.and_then(|read| read.change.as_ref())
                            .map(|change| [&change.army_raw, &change.navy_raw, &change.air_raw]),
                    ),
                ];
                for (mode, values) in modes {
                    for (index, branch) in ["army", "navy", "air"].into_iter().enumerate() {
                        let measure = format!("{branch}-{mode}");
                        let reply = ask(
                            &built,
                            "series-state",
                            json!({"branchId": "1", "family": {"kind": "country-experience",
                                "measure": measure}, "reads": [{"tag": key, "hour": hour}]}),
                        );
                        assert_eq!(reply["scale"], UNITS, "{label}");
                        assert_eq!(reply["dayHours"], 24, "{label}");
                        assert_eq!(
                            reply["states"][0]["run"]["valueRaw"].as_str(),
                            values.map(|values| values[index].as_str()),
                            "{label}: {tag} {offset} {measure}"
                        );
                        assert_eq!(
                            reply["states"][0]["experience"],
                            serde_json::to_value(read).unwrap(),
                            "{label}: {tag} {offset}"
                        );
                        assert_eq!(
                            reply["states"][0]["readHour"],
                            json!(read.map(|read| read.read_hour)),
                            "{label}: {tag} {offset}"
                        );
                    }
                }
            }
        }
        let key = serde_json::to_string(&("third", 7, 9001)).unwrap();
        let summary = ask(
            &built,
            "series-summary",
            json!({"branchId": "1", "family": {"kind": "country-experience",
                "measure": "army-banked"}, "tags": [key], "from": base, "to": base + 96,
                "width": 100, "withMarks": true}),
        );
        assert!(
            summary["series"][0]["summary"]["runs"].is_array(),
            "{label}"
        );
        assert!(
            summary["series"][0]["marks"]["clusters"].is_array(),
            "{label}"
        );
        let missing = ask(
            &built,
            "series-state",
            json!({"branchId": "1", "family": {"kind": "country-experience",
                "measure": "army-banked"}, "reads": [{"tag": "[\"missing\",null,null]",
                "hour": base}]}),
        );
        assert_eq!(missing["states"][0]["recorded"], false, "{label}");
        assert_eq!(
            missing["warnings"],
            json!(["missing: no recorded experience in this branch"]),
            "{label}"
        );
    }
}

/// Banked experience that never moves keeps a run for each daily read, and a gain day of the
/// widest awards sums past 64 bits, across cuts and a resume.
#[test]
fn equal_reads_keep_their_boundaries_and_wide_gains_sum_exactly() {
    const COUNTRIES: u32 = 3;
    let budget = Budget::new(256 << 20);
    let base = slot_base();
    let countries: Vec<_> = (1..=COUNTRIES)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: format!("country / {id}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let hours: Vec<_> = (0..100)
        .map(|hour| Tables {
            hour: base + hour,
            links: Some(
                countries
                    .iter()
                    .map(|row| CountryLinkRow {
                        id: row.id,
                        canonical_slot: row.canonical_slot,
                    })
                    .collect(),
            ),
            banked: Some(
                countries
                    .iter()
                    .map(|row| CountryExperienceRow {
                        country: row.id,
                        ..Default::default()
                    })
                    .collect(),
            ),
            gains: Some(
                (1..=COUNTRIES)
                    .map(|id| CountryExperienceGainRow {
                        country: id,
                        instance: 77,
                        complete: u8::from(hour > 0),
                        army_raw: i64::MAX,
                        navy_raw: 0,
                        air_raw: 0,
                    })
                    .collect(),
            ),
            countries: Some(countries.clone()),
        })
        .collect();
    let (projected, _) = projected(&hours, &[base + 40, base + 66]);
    let built = build(
        projected,
        &[base + 32, base + 65, base + 99],
        Some(base + 65),
        &budget,
    );
    let key =
        serde_json::to_string(&(format!("country / {COUNTRIES}"), COUNTRIES, COUNTRIES)).unwrap();
    let gained = (i128::from(i64::MAX) * 24).to_string();
    let reply = ask(
        &built,
        "series-state",
        json!({"branchId": "1", "family": {"kind": "country-experience",
            "measure": "army-gain"}, "reads": [{"tag": key, "hour": base + 99}]}),
    );
    assert_eq!(reply["states"][0]["run"]["valueRaw"], gained);
    assert_eq!(reply["states"][0]["experience"]["gain"]["armyRaw"], gained);
    assert_eq!(reply["states"][0]["available"], true);
    let summary = ask(
        &built,
        "series-summary",
        json!({"branchId": "1", "family": {"kind": "country-experience",
            "measure": "army-banked"}, "tags": [key], "from": base, "to": base + 99,
            "width": 100}),
    );
    assert_eq!(
        summary["series"][0]["summary"]["runs"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    let dense = ask(
        &built,
        "series-summary",
        json!({"branchId": "1", "family": {"kind": "country-experience",
            "measure": "army-gain"}, "tags": [key], "from": base, "to": base + 99,
            "width": 1}),
    );
    let dense = serde_json::from_value::<dashboard_base::prepared_chart::ChartSummary>(
        dense["series"][0]["summary"].clone(),
    )
    .unwrap();
    assert_eq!(
        dense.buckets[0].map(|bucket| bucket.min_value),
        Some((i128::from(i64::MAX) * 24) as f64)
    );
    let navy = ask(
        &built,
        "series-state",
        json!({"branchId": "1", "family": {"kind": "country-experience",
            "measure": "navy-change"}, "reads": [{"tag": key, "hour": base + 10}]}),
    );
    // No read a day before the first: its change is unknown, never zero.
    assert!(navy["states"][0]["run"].is_null());
    assert_eq!(
        navy["states"][0]["experience"]["changeUnknown"],
        "noMatchingRead"
    );
    assert_eq!(navy["states"][0]["available"], true);
}

/// Of 1,500 countries, an hour hands on only the slots whose banked experience or gains moved:
/// nothing when only the daily accumulators move, and nothing when nothing changes.
#[test]
fn a_quiet_hour_hands_on_only_the_slots_that_moved() {
    const COUNTRIES: u32 = 1_500;
    let base = slot_base();
    let countries: Vec<_> = (1..=COUNTRIES)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: format!("T{id:04}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links: Vec<_> = countries
        .iter()
        .map(|row| CountryLinkRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
        })
        .collect();
    let banked = |hour: i32| -> Vec<CountryExperienceRow> {
        countries
            .iter()
            .map(|row| CountryExperienceRow {
                country: row.id,
                army_raw: if hour >= 3 && row.id == 700 { 5 } else { 1 },
                army_daily_raw: i64::from(hour),
                ..Default::default()
            })
            .collect()
    };
    let gains = |hour: i32| -> Vec<CountryExperienceGainRow> {
        countries
            .iter()
            .map(|row| CountryExperienceGainRow {
                country: row.id,
                instance: 1,
                complete: 1,
                army_raw: if hour >= 4 && row.id == 9 { 3 } else { 2 },
                ..Default::default()
            })
            .collect()
    };
    let hours: Vec<_> = (1..=4)
        .map(|hour| Tables {
            hour: base + hour,
            countries: Some(countries.clone()),
            links: Some(links.clone()),
            banked: Some(banked(hour)),
            gains: Some(gains(hour)),
        })
        .collect();
    let (projected, _) = projected(&hours, &[]);
    assert!(projected[0].whole && projected[0].holders.as_ref().unwrap().len() == 1_500);
    assert_eq!(projected[0].banked.len(), 1_500);
    for hour in &projected[1..] {
        assert!(!hour.whole && hour.holders.is_none(), "hour {}", hour.hour);
        assert!(hour.named.is_empty(), "hour {}", hour.hour);
    }
    assert!(projected[1].banked.is_empty() && projected[1].gained.is_empty());
    assert_eq!(projected[2].banked, [(700, Some([5, 0, 0]))]);
    assert!(projected[2].gained.is_empty());
    assert!(projected[3].banked.is_empty());
    assert_eq!(projected[3].gained, [(9, Some([3, 0, 0]))]);
}
