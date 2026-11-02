//! The native emit held to the row walk's casualty observer on the same hours, written as the
//! native recorder writes them: after every hour by what its publications read, and page for
//! page once both publish, across segment starts, flushes, a resume, a gap and refused tables.

use super::*;
use crate::products::war_casualties::{
    facts::Projector,
    test_fixtures::{casualty_frame, research_replay},
};
use dashboard_base::prepared::{frame_changes::FrameChangeTracker, series::tests::Fixture};
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryAppearanceRow, CountryLinkRow, CountryRow, RowCodec, WarRelationRow},
    scope::CaptureMode,
};
use std::{collections::BTreeMap, io::Read};

const FORMAT: FormatVersion = FormatVersion::CURRENT;
const REFUSAL: &str = "synthetic refusal";

/// The replay blocks the scenario publishes pair windows at.
const BLOCKS: [(i32, i32); 4] = [(100, 109), (110, 124), (125, 139), (140, 150)];

/// One hour as the recorder writes it; None where WarRelations is unavailable.
struct Tables {
    hour: i32,
    countries: Vec<CountryRow>,
    links: Vec<CountryLinkRow>,
    appearance: Vec<CountryAppearanceRow>,
    relations: Option<Vec<WarRelationRow>>,
}

/// The hour the counters of the first two wars have reached: they hold from 134 to 137.
fn clock(hour: i32) -> i64 {
    i64::from(match hour {
        134..=137 => 133,
        138.. => hour - 4,
        _ => hour,
    })
}

/// Hours 100 to 150 without 125 and 126. Countries 1 and 2 fight from before 100 to 144, their
/// unknown losses agreeing with those of the war of 1 and 3 from 105 until 118, after which they
/// part; 2's unknown losses move against 1 from 110 to 114 while its war with 3 holds still.
/// That war ends at 129 and a new one between them starts at 130 on the same row. Every
/// counter holds from 134 to 137. WarRelations is unavailable at 122. Country 4 and its link
/// join at 140 and fight 1 from 141. Country 2 takes a cosmetic tag from 120 to 132 and 3 a
/// name tag from 138.
fn scenario() -> Vec<Tables> {
    (100..=150)
        .filter(|hour| !(125..=126).contains(hour))
        .map(|hour: i32| {
            let mut countries = vec![(100, 1, "AAA"), (200, 2, "BBB"), (300, 3, "CCC")];
            let mut links = vec![(1, 1), (2, 2), (3, 3), (100, 1), (200, 2), (300, 3)];
            if hour >= 140 {
                countries.push((400, 4, "DDD"));
                links.extend([(4, 4), (400, 4)]);
            }
            let t = clock(hour);
            let mut relations = Vec::new();
            if hour < 145 {
                relations.push(WarRelationRow {
                    first_country: 1,
                    second_country: 2,
                    start_hour: 90,
                    first_casualties: 3 * t,
                    second_casualties: t.min(115),
                    first_unknown_casualties: 2 * (t - 100),
                    second_unknown_casualties: (t - 109).clamp(0, 5),
                    ..Default::default()
                });
            }
            if hour >= 105 {
                relations.push(WarRelationRow {
                    first_country: 1,
                    second_country: 3,
                    start_hour: 105,
                    first_casualties: if t < 120 { 7 } else { t },
                    second_casualties: 2,
                    first_unknown_casualties: 2 * (t - 105) + 3 * (t - 117).max(0),
                    ..Default::default()
                });
            }
            relations.push(if hour < 130 {
                WarRelationRow {
                    first_country: 2,
                    second_country: 3,
                    start_hour: 90,
                    first_casualties: 4,
                    second_casualties: 4,
                    ..Default::default()
                }
            } else {
                WarRelationRow {
                    first_country: 2,
                    second_country: 3,
                    start_hour: 130,
                    first_casualties: (t - 130).max(0),
                    ..Default::default()
                }
            });
            if hour >= 141 {
                relations.push(WarRelationRow {
                    first_country: 4,
                    second_country: 1,
                    start_hour: 141,
                    first_casualties: t - 137,
                    second_casualties: 2 * (t - 137),
                    second_unknown_casualties: t - 137,
                    ..Default::default()
                });
            }
            relations.sort_unstable_by_key(|row| row.key());
            let appearance = countries
                .iter()
                .map(|&(id, _, _)| CountryAppearanceRow {
                    country: id,
                    cosmetic_tag: if id == 200 && (120..=132).contains(&hour) {
                        b"BBB_ALT".to_vec()
                    } else {
                        Vec::new()
                    },
                    name_tag: if id == 300 && hour >= 138 {
                        b"AAA".to_vec()
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                })
                .collect();
            let mut links: Vec<_> = links
                .into_iter()
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .collect();
            links.sort_unstable_by_key(|link| link.id);
            Tables {
                hour,
                countries: countries
                    .into_iter()
                    .map(|(id, canonical_slot, tag): (u32, u32, &str)| CountryRow {
                        id,
                        canonical_slot,
                        tag: tag.as_bytes().to_vec(),
                        ..Default::default()
                    })
                    .collect(),
                links,
                appearance,
                relations: (hour != 122).then_some(relations),
            }
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

/// Every page `writer` holds, by section, range by range.
fn pages(writer: &mut store::Writer) -> BTreeMap<String, Vec<Vec<u8>>> {
    let names: Vec<String> = writer.section_names().map(str::to_owned).collect();
    names
        .into_iter()
        .map(|name| {
            let count = writer.range_count(&name).unwrap().unwrap_or(0);
            let ranges = (0..count)
                .map(|range| {
                    let mut bytes = Vec::new();
                    if let Some(mut page) = writer.read_at(&name, range).unwrap() {
                        page.read_to_end(&mut bytes).unwrap();
                    }
                    bytes
                })
                .collect();
            (name, ranges)
        })
        .collect()
}

/// A branch's emit as [`Emit::new`] makes it, publishing at [`BLOCKS`].
fn emit(replay: &Replay) -> Emit {
    let mut emit = Emit::new(replay, 1, &Ok(None)).unwrap();
    emit.blocks = BLOCKS.to_vec();
    emit
}

/// The scenario's hours kept by the row observer and, projected by an operator starting cold at
/// each of `starts` and after the gap, by the emit: both hold the same after every hour, both
/// publish at each of `flushes` and the end, and the emit is saved and taken up again after
/// the flush at `resume`. Both publish the same pages. The emit alone also publishes at each of
/// `alone`, partway through windows, after which only the pair windows are compared.
fn compare(starts: &[i32], flushes: &[i32], resume: Option<i32>, alone: &[i32]) {
    let replay = research_replay(vec![casualty_frame(100, None, 0)]);
    let flag = AtomicBool::new(false);
    let mut oracle = Prepared::new(&replay, 1);
    oracle.configure(&Ok(None));
    let mut tracker = FrameChangeTracker::new();
    let mut next = 0;
    let mut native = emit(&replay);
    let (oracle_pages, native_pages) = (Fixture::new(), Fixture::new());
    let (mut oracle_writer, mut native_writer) = (oracle_pages.writer(), native_pages.writer());
    let mut projector = Projector::new(&replay).unwrap();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut last: Option<i32> = None;
    let mut parted = false;
    for (ordinal, tables) in (0..).zip(scenario()) {
        let gap = last.is_some_and(|last| last + 1 != tables.hour);
        last = Some(tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, TableId::Countries, Some(&tables.countries[..]));
        write(walk, TableId::CountryLinks, Some(&tables.links[..]));
        write(
            walk,
            TableId::CountryAppearance,
            Some(&tables.appearance[..]),
        );
        write(walk, TableId::WarRelations, tables.relations.as_deref());
        let cold = starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(&replay).unwrap();
        }
        let mut out = Vec::new();
        walk.end(|state, manifest, decoded, first| {
            while let Some(&(first, last)) = BLOCKS.get(next)
                && last < decoded.hour
            {
                oracle
                    .finish_block(&mut oracle_writer, 1, first, last)
                    .unwrap();
                next += 1;
            }
            let changes = tracker.observe(decoded, FORMAT);
            oracle
                .observe_changes(decoded, &replay, &flag, &changes)
                .unwrap();
            if let Some(&(first, last)) = BLOCKS.get(next)
                && last == decoded.hour
            {
                oracle
                    .finish_block(&mut oracle_writer, 1, first, last)
                    .unwrap();
                next += 1;
            }
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs().unwrap(),
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
        for hour in out {
            native
                .observe(&mut native_writer, &replay, hour, &flag)
                .unwrap();
        }
        if !parted {
            assert_eq!(
                native.prepared.parity(),
                oracle.parity(),
                "after hour {}",
                tables.hour
            );
        }
        if alone.contains(&tables.hour) {
            parted = true;
            native.flush(&mut native_writer, None, &flag).unwrap();
            if resume == Some(tables.hour) {
                native = Emit::load(&replay, 1, &Ok(None), &native.save().unwrap()).unwrap();
                native.blocks = BLOCKS.to_vec();
                projector = Projector::new(&replay).unwrap();
            }
        }
        if flushes.contains(&tables.hour) {
            oracle.flush(&mut oracle_writer, 1, &flag).unwrap();
            native.flush(&mut native_writer, None, &flag).unwrap();
            if !parted {
                assert_eq!(
                    pages(&mut native_writer),
                    pages(&mut oracle_writer),
                    "flushed at hour {}",
                    tables.hour
                );
            }
            if resume == Some(tables.hour) {
                native = Emit::load(&replay, 1, &Ok(None), &native.save().unwrap()).unwrap();
                native.blocks = BLOCKS.to_vec();
                projector = Projector::new(&replay).unwrap();
            }
        }
    }
    while let Some(&(first, last)) = BLOCKS.get(next) {
        oracle
            .finish_block(&mut oracle_writer, 1, first, last)
            .unwrap();
        next += 1;
    }
    oracle.flush(&mut oracle_writer, 1, &flag).unwrap();
    native.flush(&mut native_writer, Some(150), &flag).unwrap();
    let (mut native, mut oracle) = (pages(&mut native_writer), pages(&mut oracle_writer));
    if parted {
        native.retain(|section, _| section == "casualty-pairs-1");
        oracle.retain(|section, _| section == "casualty-pairs-1");
        assert!(!oracle.is_empty(), "the pair windows are published");
    }
    assert_eq!(native, oracle);
}

#[test]
fn native_casualties_publish_the_row_walk_pages_read_whole_by_segment() {
    compare(&[], &[], None, &[]);
}

#[test]
fn native_casualties_publish_the_row_walk_pages_across_starts_flushes_and_a_resume() {
    compare(
        &[105, 106, 118, 123, 130, 135, 141, 146],
        &[108, 113, 131, 142],
        Some(113),
        &[],
    );
}

#[test]
fn native_casualties_publish_the_row_walk_pages_starting_every_hour() {
    let starts: Vec<i32> = (100..=150).collect();
    compare(&starts, &[120, 140], Some(140), &[]);
}

#[test]
fn native_casualties_keep_a_window_s_reads_through_flushes_partway_through_it() {
    compare(&[123, 131], &[], Some(123), &[104, 112, 123, 131, 145]);
}
