//! The family's columnar preparation held to the row oracle ([`super::super::row`]) on the same
//! hours, written as the native recorder writes them: the projector's hours scan as the
//! oracle's do across cold starts and a gap, and the publication built from them, cut and
//! resumed, answers every window and every country's leaves as the row scan's log does.

use super::super::{
    Cut, Scan, ScannedLog,
    facts::{self, Projector},
    observed, query,
    row::Rows,
    tests::{CIVILIAN, building_line, railway_line},
};
use super::{Emit, SCHEMA};
use crate::products::construction_spending;
use dashboard_base::products::{battles::BattleWindow, country_groups::HourSpan};
use dashboard_series::{
    Blob, Publication, Snapshot, Span, Writer,
    family::Directory,
    memory::{Memory, publish},
};
use engine_native::cbuilding_database::RAILWAY_TEMPLATE_TOKEN;
use replay::{
    columnar_frames::{
        metrics,
        synthetic::{Input, Sample, Synthetic},
    },
    segments::{self, Operator},
};
use replay_data::{
    cadence::{self, Cadence},
    format::FormatVersion,
    rows::{
        BuildingRow, ConstructionLineOutputRow, CountryLinkRow, CountryRow, LexicalTokenRow,
        ProvinceRow, Recorded, RowCodec, TableId,
    },
    scope::CaptureMode,
};
use std::{collections::BTreeMap, sync::Arc};

const FIRST: i32 = 96;
const LAST: i32 = 175;
/// Hours the recording lacks.
const GAP: [i32; 2] = [140, 141];
/// Hours a segment starts at, its projector cold.
const STARTS: [i32; 2] = [121, 160];

/// Whether `table` is read at `hour` by its daily slot.
fn scheduled(table: TableId, hour: i32) -> bool {
    match cadence::cadence(table) {
        Cadence::Daily(slot) => hour.rem_euclid(cadence::STAGGER_PERIOD) == slot as i32,
        Cadence::Hourly | Cadence::Sliced => true,
        Cadence::Static => false,
    }
}

/// `table`'s reads from the first hour through `hour`.
fn reads(table: TableId, hour: i32) -> usize {
    (FIRST..=hour)
        .filter(|&at| !GAP.contains(&at) && scheduled(table, at))
        .count()
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, hour: i32, rows: Option<&[R]>) {
    let (read, sample) = (scheduled(table, hour), Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Input::Unavailable {
            reason: "synthetic refusal",
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

fn building(location: u32, building_type: u32, (level, healthy_level): (i16, i16)) -> BuildingRow {
    BuildingRow {
        location,
        building_type,
        level,
        healthy_level,
        ..Default::default()
    }
}

/// Writes `hour` as the recorder writes it. GER (raw 11) and ITA (raw 12); a civilian factory
/// at state 40 rises at 120, is damaged from 150 and repaired at 168, and the state table is
/// unavailable at 130 and 131; the second route province's railway rises at 144. GER's line
/// 900 builds there, its count falls at its second read and it leaves at its fourth, its output
/// read on; line 901's country no link resolves; ITA's line 902 appears at the queue's second
/// read; GER's railway 950 leaves at the railway queue's third read; output no census owns.
fn hour_tables(walk: &mut Synthetic, hour: i32) {
    let mut countries = vec![
        CountryRow {
            id: 11,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 12,
            canonical_slot: 2,
            tag: b"ITA".to_vec(),
            ..Default::default()
        },
    ];
    countries.sort_unstable_by_key(|row| row.id);
    write(walk, TableId::Countries, hour, Some(&countries[..]));
    let links =
        [(11, 1), (12, 2)].map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot });
    write(walk, TableId::CountryLinks, hour, Some(&links[..]));
    let provinces = [(1001, 40), (1002, 41)].map(|(id, state)| ProvinceRow {
        id,
        state,
        ..Default::default()
    });
    write(walk, TableId::Provinces, hour, Some(&provinces[..]));
    let factory = match hour {
        ..120 => (1, 1),
        150..168 => (2, 1),
        _ => (2, 2),
    };
    let states = [building(40, CIVILIAN, factory)];
    write(
        walk,
        TableId::StateBuildings,
        hour,
        (!(130..=131).contains(&hour)).then_some(&states[..]),
    );
    let rails = [
        building(1001, RAILWAY_TEMPLATE_TOKEN, (1, 1)),
        building(
            1002,
            RAILWAY_TEMPLATE_TOKEN,
            if hour < 144 { (1, 1) } else { (2, 2) },
        ),
    ];
    write(walk, TableId::ProvinceBuildings, hour, Some(&rails[..]));
    let queued = reads(TableId::BuildingConstruction, hour);
    let mut lines = Vec::new();
    if queued < 4 {
        lines.push(building_line(900, 40, if queued < 2 { 2 } else { 1 }, 0, 3));
    }
    lines.push(replay_data::rows::BuildingConstructionRow {
        country: 99,
        ..building_line(901, 40, 1, 0, 1)
    });
    if queued >= 2 {
        lines.push(replay_data::rows::BuildingConstructionRow {
            country: 12,
            created_hour: 110,
            ..building_line(902, 41, 1, 0, 2)
        });
    }
    if queued >= 3 {
        for id in [903, 904] {
            lines.push(replay_data::rows::BuildingConstructionRow {
                country: 12,
                ..building_line(id, 41, 1, 0, 2)
            });
        }
    }
    lines.sort_unstable_by_key(|line| line.id);
    write(walk, TableId::BuildingConstruction, hour, Some(&lines[..]));
    let railways = if reads(TableId::RailwayConstruction, hour) < 3 {
        vec![railway_line(950, &[1001, 1002], 1)]
    } else {
        Vec::new()
    };
    write(
        walk,
        TableId::RailwayConstruction,
        hour,
        Some(&railways[..]),
    );
    let read = reads(TableId::ConstructionLineOutput, hour) as i64;
    let mut output = vec![ConstructionLineOutputRow {
        line: 900,
        since_hour: FIRST,
        construction_raw: 100_000 * read,
        observed_since_creation: true,
        exact: true,
        completion: Recorded::Value(None),
        ..Default::default()
    }];
    if read < 3 {
        output.push(ConstructionLineOutputRow {
            line: 777,
            country: Recorded::Value(Some(11)),
            building_type: Recorded::Value(Some(CIVILIAN)),
            target_flags: Recorded::Value(Some(2)),
            target_location: Recorded::Value(Some(41)),
            target_state: Recorded::Value(Some(41)),
            target_country: Recorded::Value(Some(0)),
            since_hour: FIRST,
            construction_raw: 50_000 * read,
            exact: true,
            ended: read == 2,
            completion: Recorded::Value(None),
            ..Default::default()
        });
        output.push(ConstructionLineOutputRow {
            line: 778,
            ..output.last().unwrap().clone()
        });
        output.last_mut().unwrap().country = Recorded::Value(None);
    }
    output.push(ConstructionLineOutputRow {
        line: 902,
        country: Recorded::Value(Some(12)),
        building_type: Recorded::Value(Some(CIVILIAN)),
        target_flags: Recorded::Value(Some(2)),
        target_location: Recorded::Value(Some(41)),
        target_state: Recorded::Value(Some(41)),
        target_country: Recorded::Value(Some(0)),
        since_hour: FIRST,
        exact: true,
        construction_raw: 7_000 * read,
        ..Default::default()
    });
    for id in [903, 904] {
        let known = id == 903 && read < 2;
        output.push(ConstructionLineOutputRow {
            line: id,
            country: Recorded::Value(known.then_some(11)),
            building_type: Recorded::Value(known.then_some(CIVILIAN)),
            target_flags: Recorded::Value(known.then_some(2)),
            target_location: Recorded::Value(known.then_some(40)),
            target_state: Recorded::Value(known.then_some(40)),
            target_country: Recorded::Value(known.then_some(0)),
            since_hour: FIRST,
            exact: true,
            construction_raw: 11_000 * read,
            ..Default::default()
        });
    }
    output.sort_unstable_by_key(|row| row.line);
    write(
        walk,
        TableId::ConstructionLineOutput,
        hour,
        Some(&output[..]),
    );
    let mut tokens = vec![
        LexicalTokenRow {
            token: CIVILIAN,
            literal: b"industrial_complex".to_vec(),
        },
        LexicalTokenRow {
            token: RAILWAY_TEMPLATE_TOKEN,
            literal: b"rail_way".to_vec(),
        },
    ];
    tokens.sort_unstable_by_key(|row| row.token);
    write(walk, TableId::LexicalTokens, hour, Some(&tokens[..]));
}

/// The projector's hours and the oracle's of the same walk: a walk starts again after the
/// gap, and the projector starts cold at each of [`STARTS`].
fn projected() -> (Vec<observed::Hour>, Vec<observed::Hour>) {
    let (mut projector_hours, mut oracle_hours) = (Vec::new(), Vec::new());
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
    let mut oracle = Rows::default();
    let mut last: Option<i32> = None;
    for (ordinal, hour) in (0..).zip((FIRST..=LAST).filter(|hour| !GAP.contains(hour))) {
        if walk.is_none() || last.is_some_and(|last| last + 1 != hour) {
            walk = Some(Synthetic::new(facts::TABLES, 1, hour).unwrap());
        }
        last = Some(hour);
        let walk = walk.as_mut().unwrap();
        hour_tables(walk, hour);
        let cold = STARTS.contains(&hour);
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
                    &mut projector_hours,
                )
                .unwrap();
            let flags = metrics(manifest);
            let read = [
                TableId::BuildingConstruction,
                TableId::RailwayConstruction,
                TableId::ConstructionLineOutput,
            ]
            .map(|table| {
                dashboard_text::metadata::slot_read(
                    FormatVersion::CURRENT,
                    &flags,
                    table,
                    decoded.hour,
                )
            });
            oracle_hours.push(oracle.hour(decoded, read));
        })
        .unwrap();
    }
    // As its emit is handed them: written to the spool and read back.
    (replay_spool::respool(projector_hours).unwrap(), oracle_hours)
}

/// What the scan of `hours` logs over the branch.
fn scanned(hours: &[observed::Hour]) -> ScannedLog {
    let mut scan = Scan::default();
    for hour in hours {
        scan.observe_reads(hour, true);
    }
    scan.finish(Some(LAST));
    scan.log
}

#[test]
fn the_projector_s_hours_scan_as_the_row_oracle_s_across_cold_starts_and_a_gap() {
    let (projector, oracle) = projected();
    assert_eq!(projector.len(), oracle.len());
    // A segment's first hour holds each building table whole, every later hour only changes.
    assert!(
        projector
            .iter()
            .any(|hour| matches!(hour.buildings[0], observed::Buildings::Changed(_)))
    );
    assert!(
        projector
            .iter()
            .filter(|hour| STARTS.contains(&hour.hour))
            .all(|hour| matches!(hour.buildings[1], observed::Buildings::Whole(_)))
    );
    assert!(
        projector
            .iter()
            .any(|hour| matches!(hour.buildings[1], observed::Buildings::Unchanged))
    );
    let (actual, expected) = (scanned(&projector), scanned(&oracle));
    assert!(!expected.projects.is_empty());
    assert!(expected.unresolved > 0);
    assert!(expected.unowned_lines > 0);
    let output_only = expected
        .projects
        .iter()
        .find(|project| project.id == "777")
        .unwrap();
    assert_eq!(output_only.source, super::super::ProjectSource::OutputAlone);
    assert!(output_only.runs.is_empty());
    let enriched = expected
        .projects
        .iter()
        .find(|project| project.id == "902")
        .unwrap();
    assert_eq!(enriched.source, super::super::ProjectSource::Census);
    assert!(
        enriched
            .census_read
            .is_some_and(|read| read > enriched.first_read)
    );
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
}

fn identity() -> dashboard_series::Identity {
    dashboard_series::Identity {
        source: [7; 32],
        content: [8; 32],
        branch: [9; 32],
        schema: SCHEMA,
    }
}

/// The replay blocks the log's windows are.
fn windows() -> Vec<HourSpan> {
    vec![
        HourSpan {
            first_hour: FIRST,
            last_hour: 135,
        },
        HourSpan {
            first_hour: 136,
            last_hour: LAST,
        },
    ]
}

/// The publication of `hours`, cut after each of `cuts`' hours, the emit saved at `resume` and
/// going on from the publication.
fn build(hours: Vec<observed::Hour>, cuts: &[i32], resume: i32) -> (Memory, u64) {
    let mut pages = Memory::default();
    let mut writer = Writer::new(identity(), FIRST).unwrap();
    let mut emit = Emit::new(windows());
    let mut root = 0;
    let mut hours = hours.into_iter().peekable();
    for (generation, &cut) in (1..).zip(cuts) {
        let mut last = None;
        while let Some(hour) = hours.next_if(|hour| hour.hour <= cut) {
            last = Some(hour.hour);
            emit.hour(&mut writer, hour).unwrap();
        }
        if cut == LAST
            && let Some(last) = last
        {
            emit.end(&mut writer, last).unwrap();
        }
        emit.cut(&mut writer, cut).unwrap();
        root = publish(
            &mut writer,
            &mut pages,
            Publication {
                generation,
                segment: generation - 1,
                accepted: Span::new(FIRST, cut).unwrap(),
                reached: cut,
                refusal: None,
            },
            Blob::EMPTY,
        )
        .unwrap();
        if cut == resume {
            let mut saved = Vec::new();
            emit.save(&mut saved).unwrap();
            writer = Writer::resume(&mut pages, root, identity()).unwrap();
            let mut snapshot = Snapshot::open(pages.clone(), root, identity()).unwrap();
            let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
            emit = Emit::load(windows(), &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    (pages, root)
}

#[test]
fn the_publication_answers_every_window_and_leaf_as_the_row_scan_does_across_cuts_and_a_resume() {
    let (_, oracle) = projected();
    let expected = scanned(&oracle);
    let leaves = construction_spending::all_leaves(&expected);
    assert!(leaves.contains_key("GER"));
    let battle: Vec<BattleWindow> = windows()
        .iter()
        .map(|window| BattleWindow {
            first_hour: window.first_hour,
            last_hour: window.last_hour,
        })
        .collect();
    let cut = Cut::new(Arc::new(expected), battle.clone());
    for (cuts, resume) in [
        (&[119, 151, LAST][..], 151),
        (&[100, 135, 142, LAST][..], 135),
    ] {
        let (pages, root) = build(projected().0, cuts, resume);
        let mut snapshot = Snapshot::open(pages, root, identity()).unwrap();
        let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
        for window in 0..battle.len() {
            let reply =
                query::window_reply(&mut snapshot, &directory, battle.clone(), window).unwrap();
            assert_eq!(
                serde_json::to_value(reply).unwrap(),
                serde_json::to_value(cut.reply(window)).unwrap(),
                "cuts {cuts:?}: window {window}"
            );
        }
        for (tag, (expected, _)) in &leaves {
            let published = query::leaves(&mut snapshot, &directory, tag).unwrap();
            assert_eq!(published.len(), expected.len(), "{tag}");
            let totals: BTreeMap<_, _> = published
                .iter()
                .map(|(leaf, key)| {
                    let total = match snapshot.value_at(*key, LAST).unwrap() {
                        dashboard_series::Observation::Value { run, .. } => run.value,
                        other => panic!("{tag}: no total at the last hour: {other:?}"),
                    };
                    ((leaf.repair, leaf.building, leaf.state), total)
                })
                .collect();
            for ((leaf, _), expected) in published.iter().zip(expected) {
                let mut want = expected.clone();
                let total = want
                    .spent
                    .last()
                    .map_or(0, |step| step.total_raw.parse::<i128>().unwrap());
                want.spent.clear();
                assert_eq!(leaf, &want, "cuts {cuts:?}: {tag}");
                assert_eq!(
                    totals[&(leaf.repair, leaf.building, leaf.state)],
                    total,
                    "cuts {cuts:?}: {tag}'s total"
                );
            }
        }
    }
}

/// A spending gap of `project` known at `known_at`, open where `until` is none.
fn gap(project: &str, known_at: i32, until: Option<i32>) -> super::SpendingUnknown {
    super::SpendingUnknown {
        known_at,
        reason: super::SpendingGap::FinalOutputMissing,
        project: project.to_owned(),
        until,
    }
}

fn leaf_of(unknown: Vec<super::SpendingUnknown>) -> super::SpendingLeaf {
    super::SpendingLeaf {
        repair: false,
        kind: super::ProjectKind::Building,
        building: 0,
        state: Some(40),
        to_state: None,
        spent: Vec::new(),
        unknown,
        projects: Vec::new(),
    }
}

/// A final project's open gap, published, is merged and resolved by a later line of its handle
/// as one metadata holding both lines merges and resolves it.
#[test]
fn a_reused_handle_closes_the_gap_its_final_line_left_open() {
    let mut whole = super::LeafMetadata::default();
    super::merge_leaf(&mut whole, leaf_of(vec![gap("5", 100, None)]));
    let mut held = super::Leaf {
        key: 0,
        total: 0,
        observed: true,
        live: super::LeafMetadata::default(),
        finals: BTreeMap::new(),
        dirty: false,
    };
    super::merge_leaf(&mut held.live, leaf_of(vec![gap("5", 100, None)]));
    let finals = held.live.take("5");
    for gap in finals.unknown.values().filter(|gap| gap.until.is_none()) {
        held.finals.insert((5, gap.known_at), gap.reason);
    }
    assert!(held.live.is_empty());
    // The handle's next line resolves at 130, with a gap of the same reason at 150 before.
    for (merge, resolve) in [(Some(gap("5", 150, None)), None), (None, Some(130))] {
        if let Some(gap) = merge {
            super::merge_leaf(&mut whole, leaf_of(vec![gap.clone()]));
            held.revive(&gap.project);
            super::merge_leaf(&mut held.live, leaf_of(vec![gap]));
        }
        if let Some(hour) = resolve {
            whole.resolve("5", hour);
            held.revive("5");
            held.live.resolve("5", hour);
        }
    }
    // The reader takes in the final entries, then the live ones over them.
    let mut read = finals;
    read.extend(held.live.clone());
    assert_eq!(read, whole);
    assert!(held.finals.is_empty());
}
