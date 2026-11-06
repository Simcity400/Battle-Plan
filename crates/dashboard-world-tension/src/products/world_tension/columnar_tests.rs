//! The family's columnar preparation held to its row scan ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every read, every contributor's runs, the
//! threat events and the counts, across segment starts, a gap, cuts and a resume.

use super::{
    emit::{Emit, SCHEMA, VALUE, contributor},
    facts::{self, Projector},
    query, row,
};
use dashboard_base::prepared::engine;
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
        CountryLinkRow, CountryRow, RowCodec, TableId, ThreatCountryAggregateRow, ThreatSourceRow,
        WorldTensionRow,
    },
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;

const REFUSAL: &str = "synthetic refusal";

/// One hour as the recorder writes it; none where a table is unavailable, and the sources'
/// missing rows where their walk was partial.
struct Tables {
    hour: i32,
    countries: Option<Vec<CountryRow>>,
    links: Vec<CountryLinkRow>,
    tension: Option<Vec<WorldTensionRow>>,
    aggregates: Vec<ThreatCountryAggregateRow>,
    sources: Option<Vec<ThreatSourceRow>>,
    missing: Option<u32>,
}

fn source(country: u32, date: i32, reason: &[u8], current_raw: i64, final_raw: i64) -> ThreatSourceRow {
    ThreatSourceRow {
        ordinal: 0,
        country,
        second_country: 0,
        current_raw,
        final_raw,
        daily_raw: 0,
        date,
        reason: reason.to_vec(),
    }
}

/// Hours 100 to 160 without 130 and 131. A's contribution and its war's final contribution grow
/// every hour; B's claim holds 105 to 125; C's two equal sources hold from 103 and C is renamed
/// at 150; slot 4 contributes from 140 to 146, unnamed until its country is recorded at 144.
/// The aggregates disagree with the sources at 115, the tension with the sum at 112, the source
/// count with the rows at 118; WorldTension is unavailable at 120 and 121, the sources at 127 and
/// partial at 133, and the countries at 135.
fn scenario() -> Vec<Tables> {
    (100..=160)
        .filter(|hour| !(130..=131).contains(hour))
        .map(|hour| {
            let c = if hour >= 150 { "CCX" } else { "CCC" };
            let mut countries = vec![(1, 1, "AAA"), (2, 2, "BBB"), (3, 3, c)];
            if hour >= 144 {
                countries.push((4, 4, "DDD"));
            }
            let countries: Vec<CountryRow> = countries
                .into_iter()
                .map(|(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect();
            let links = [(1, 1), (2, 2), (3, 3), (4, 4), (11, 1)]
                .into_iter()
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .collect();
            let a = 100_000 * i64::from(hour - 99);
            let b = if (105..=125).contains(&hour) { 500_000 } else { 0 };
            let c_amount = if hour >= 103 { 250_000 } else { 0 };
            let d = if (140..=146).contains(&hour) { 7_000 } else { 0 };
            let mut sources = vec![source(11, 90, b"War on \x11YPoland\x11!", a, a + 100)];
            if b != 0 {
                sources.push(source(2, 104, b"Claims", b, b));
            }
            if c_amount != 0 {
                sources.push(source(3, 102, b"Same", c_amount / 2, c_amount / 2));
                sources.push(source(3, 102, b"Same", c_amount / 2, c_amount / 2));
            }
            if d != 0 {
                sources.push(source(4, 139, b"Nobody yet", d, d));
            }
            for (ordinal, row) in sources.iter_mut().enumerate() {
                row.ordinal = ordinal as u32;
            }
            let shown = c_amount + if hour == 115 { 1 } else { 0 };
            let aggregates = (0..6)
                .map(|slot| ThreatCountryAggregateRow {
                    country_slot: slot,
                    total_raw: [0, a, b, shown, d, 0][slot as usize],
                })
                .collect();
            let sum: i64 = a + b + shown + d;
            let total = sum.clamp(0, engine_stability::cworld_threat::CLAMP_CEILING)
                / engine_stability::cworld_threat::SCALAR_DIVISOR
                + i64::from(hour == 112);
            let count = sources.len() as i32 + i32::from(hour == 118);
            let missing = (hour == 133).then_some(1);
            if missing.is_some() {
                sources.truncate(sources.len() - 1);
            }
            Tables {
                hour,
                countries: (hour != 135).then_some(countries),
                links,
                tension: (!(120..=121).contains(&hour)).then(|| {
                    vec![WorldTensionRow {
                        total_raw: total,
                        source_count: count,
                        country_aggregate_count: 6,
                    }]
                }),
                aggregates,
                sources: (hour != 127).then_some(sources),
                missing,
            }
        })
        .collect()
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>, partial: Option<u32>) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial,
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

/// The projected hours, a projector starting cold at each of `starts` and at each walk's first
/// hour, and the row frames of the same hours.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames: Vec<DecodedFrame> = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last| last.hour + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, TableId::Countries, tables.countries.as_deref(), None);
        write(walk, TableId::CountryLinks, Some(&tables.links[..]), None);
        write(walk, TableId::WorldTension, tables.tension.as_deref(), None);
        write(walk, TableId::ThreatCountryAggregates, Some(&tables.aggregates[..]), None);
        write(walk, TableId::ThreatSources, tables.sources.as_deref(), tables.missing);
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
/// names one of them, the emit is saved there and goes on from the publication.
fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>, budget: &Budget) -> (Memory, u64) {
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
    (pages, root)
}

/// Holds the publication to the row scan of `frames`.
fn check(pages: &Memory, root: u64, frames: &[DecodedFrame], label: &str) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(pages.clone(), &cache, root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let mut scan = row::Scan::default();
    for frame in frames {
        scan.observe(FormatVersion::CURRENT, frame).unwrap();
    }
    let expected = scan.finish();
    for frame in frames {
        let held = expected
            .reads
            .iter()
            .find(|read| read.first_hour <= frame.hour && frame.hour <= read.last_hour)
            .cloned();
        assert_eq!(
            query::read_at(&mut snapshot, &directory, frame.hour).unwrap(),
            held,
            "{label}: the read at {}",
            frame.hour
        );
    }
    let mut contributors: Vec<(String, u32, i32, u64)> = directory
        .actors
        .iter()
        .enumerate()
        .map(|(actor, held)| {
            let (slot, tag) = contributor(&held.name).unwrap();
            (tag.to_owned(), slot, held.first_hour, actor as u64)
        })
        .collect();
    contributors.sort();
    assert_eq!(
        contributors
            .iter()
            .map(|(tag, slot, _, _)| (tag.clone(), *slot))
            .collect::<Vec<_>>(),
        expected
            .countries
            .iter()
            .map(|country| (country.tag.clone(), country.slot))
            .collect::<Vec<_>>(),
        "{label}: contributors"
    );
    assert_eq!(
        contributors.iter().map(|held| held.2).collect::<Vec<_>>(),
        expected.first_contributions,
        "{label}: first contributions"
    );
    let accepted = snapshot.publication().accepted;
    for ((tag, _, first, actor), country) in contributors.iter().zip(&expected.countries) {
        let key = directory.key(*actor, VALUE, None).unwrap();
        let published: Vec<_> = engine::runs(&mut snapshot, key, accepted.first, accepted.last)
            .unwrap()
            .into_iter()
            .filter(|run| run.value != 0)
            .map(|run| (run.span.first, run.span.last, run.value.to_string(), run.break_before))
            .collect();
        // A contributor's first value starts its line.
        let wanted: Vec<_> = country
            .runs
            .iter()
            .map(|run| {
                (
                    run.first_hour,
                    run.last_hour,
                    run.value_raw.clone(),
                    run.starts_segment || run.first_hour == *first,
                )
            })
            .collect();
        assert_eq!(published, wanted, "{label}: {tag}'s contributions");
    }
    let read = query::published(&mut snapshot, &directory).unwrap();
    assert_eq!(read.events, expected.events, "{label}: events");
    assert_eq!(
        read.counts,
        [
            expected.read_count,
            expected.unnamed_reads as u64,
            expected.partial_sources as u64,
            expected.aggregate_mismatches as u64,
            expected.tension_mismatches as u64,
        ],
        "{label}: counts"
    );
    assert!(expected.aggregate_mismatches > 0 && expected.tension_mismatches > 0);
    assert!(expected.partial_sources > 0 && expected.unnamed_reads > 0);
}

/// Every placement of segment starts, cuts and a resume publishes what the row scan reads.
#[test]
fn the_publication_holds_what_the_row_scan_reads_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    for (starts, cuts, resume) in [
        (&[][..], &[160][..], None),
        (&[110, 140, 150][..], &[109, 129, 149, 160][..], Some(129)),
        (&[103, 115, 125, 136, 155][..], &[114, 133, 146, 160][..], Some(146)),
        (
            &(101..=160).collect::<Vec<_>>()[..],
            &(100..=160).filter(|hour| !(130..=131).contains(hour)).collect::<Vec<_>>()[..],
            Some(120),
        ),
    ] {
        let (projected, frames) = projected(&hours, starts);
        let (pages, root) = build(projected, cuts, resume, &budget);
        check(&pages, root, &frames, &format!("starts {starts:?}, cuts {cuts:?}"));
    }
}

/// Of 1,500 countries, an hour that changes no tension table carries nothing, and one that
/// changes one country's contribution carries the nonzero contributions alone.
#[test]
fn quiet_hours_carry_nothing_and_a_change_carries_the_nonzero_contributions() {
    let countries: Vec<CountryRow> = (0..1500)
        .map(|at| CountryRow {
            id: at + 1,
            canonical_slot: at,
            tag: format!("T{at:04}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links: Vec<CountryLinkRow> = (0..1500)
        .map(|at| CountryLinkRow {
            id: at + 1,
            canonical_slot: at,
        })
        .collect();
    let hours: Vec<Tables> = (0..4)
        .map(|offset| {
            let moved = if offset >= 2 { 9_000 } else { 8_000 };
            Tables {
                hour: 100 + offset,
                countries: Some(countries.clone()),
                links: links.clone(),
                tension: Some(vec![WorldTensionRow {
                    total_raw: 0,
                    source_count: 0,
                    country_aggregate_count: 1500,
                }]),
                aggregates: (0..1500)
                    .map(|slot| ThreatCountryAggregateRow {
                        country_slot: slot,
                        total_raw: match slot {
                            7 => moved,
                            9 => 1_000,
                            _ => 0,
                        },
                    })
                    .collect(),
                sources: Some(Vec::new()),
                missing: None,
            }
        })
        .collect();
    let (projected, _) = projected(&hours, &[]);
    assert!(projected[1].facts.is_none(), "an hour changing nothing carries nothing");
    let facts = projected[2].facts.as_ref().unwrap();
    assert!(facts.identity.is_none() && facts.tension.is_none() && facts.sources.is_none());
    assert_eq!(
        facts.aggregates.clone().flatten().as_deref(),
        Some(&[(7, 9_000), (9, 1_000)][..])
    );
    assert!(projected[3].facts.is_none());
}
