//! The family's columnar preparation held to its row census ([`super::scan`]) on the same
//! hours, written as the native recorder writes them: every tag's runs with their compositions,
//! the four totals, the ranks and the census's counted hours, across segment starts, a gap,
//! cuts and a resume.

use super::{
    emit::{Emit, FAMILY, MIX, POSITIONS, SCHEMA, VALUES},
    facts::{self, Projector},
    *,
};
use dashboard_base::prepared::engine;
use dashboard_base::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag};
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
    rows::{
        AirWingRow, CountryLinkRow, CountryRow, LexicalTokenRow, RowCodec, TableId, TableRows,
        VariantChangeRow, VariantDefinitionRow, WingEquipmentRow,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::collections::BTreeSet;

const REFUSAL: &str = "synthetic unavailable table";
const COUNTRIES: u32 = 4;

/// One hour as the recorder writes it: whether the countries, the wings and the variant
/// definitions are complete; the wings (id, owner, override, planes) and their banks (wing,
/// variant, raw amount).
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: bool,
    definitions: bool,
    wings: Option<Vec<(u64, u32, i32, i32)>>,
    banks: Vec<(u64, u64, i64)>,
}

const UNIT: i64 = 100_000;

/// Hours 100 to 175 without 145 and 146. Country 0's wing grows at 105 and 120; country 1's
/// wing counts for country 0 as effective owner and splits inexactly from 110 to 112; country
/// 2's wing mixes two variants and loses one at 118; country 3 gains a wing at 115 and loses it
/// at 140; the wings are unavailable at 125; the variant definitions at 130 to 132; the
/// countries at 150; a wing no link resolves from 155 to 156; an override no link resolves at
/// 160 and 161; a wing of negative planes at 165.
fn scenario() -> Vec<Tables> {
    (100..=175)
        .filter(|hour| !(145..=146).contains(hour))
        .map(|hour| {
            let a = match hour {
                ..=104 => 10,
                105..=119 => 12,
                _ => 9,
            };
            let mut wings = vec![
                (1, 100_000, 0, a),
                (2, 100_001, 100_000, 8),
                (3, 100_002, 0, 20),
            ];
            let mut banks = vec![(1, 71, i64::from(a) * UNIT)];
            banks.push((
                2,
                72,
                if (110..=112).contains(&hour) {
                    8 * UNIT - 1
                } else {
                    8 * UNIT
                },
            ));
            banks.push((3, 71, 12 * UNIT));
            if hour < 118 {
                banks.push((3, 72, 8 * UNIT));
            }
            if (115..140).contains(&hour) {
                wings.push((4, 100_003, 0, 5));
                banks.push((4, 72, 5 * UNIT));
            }
            if (155..=156).contains(&hour) {
                wings.push((5, 999, 0, 3));
            }
            if (160..=161).contains(&hour) {
                wings.push((6, 100_003, 999, 4));
            }
            if hour == 165 {
                wings.push((7, 100_002, 0, -1));
            }
            wings.sort_unstable();
            banks.sort_unstable();
            Tables {
                hour,
                countries: hour != 150,
                definitions: !(130..=132).contains(&hour),
                wings: (hour != 125).then_some(wings),
                banks,
            }
        })
        .collect()
}

fn country_rows() -> Vec<CountryRow> {
    (0..COUNTRIES)
        .map(|index| CountryRow {
            id: 100_000 + index,
            canonical_slot: index,
            tag: scale_tag(index),
            ..CountryRow::default()
        })
        .collect()
}

fn link_rows() -> Vec<CountryLinkRow> {
    (0..COUNTRIES)
        .map(|index| CountryLinkRow {
            id: 100_000 + index,
            canonical_slot: index,
        })
        .collect()
}

fn wing_rows(wings: &[(u64, u32, i32, i32)]) -> Vec<AirWingRow> {
    wings
        .iter()
        .map(|&(id, owner, country_override, planes)| AirWingRow {
            id,
            owner,
            country_override,
            planes,
            ..Default::default()
        })
        .collect()
}

fn bank_rows(banks: &[(u64, u64, i64)]) -> Vec<WingEquipmentRow> {
    let mut ordinals = BTreeMap::new();
    banks
        .iter()
        .map(|&(wing, variant, amount_raw)| {
            let ordinal = ordinals.entry(wing).or_insert(0);
            let row = WingEquipmentRow {
                wing,
                ordinal: *ordinal,
                variant,
                amount_raw,
            };
            *ordinal += 1;
            row
        })
        .collect()
}

fn token_rows() -> Vec<LexicalTokenRow> {
    vec![
        LexicalTokenRow {
            token: 1,
            literal: b"fighter_equipment_0".to_vec(),
        },
        LexicalTokenRow {
            token: 2,
            literal: b"cas_equipment_1".to_vec(),
        },
    ]
}

fn definition_rows() -> Vec<VariantDefinitionRow> {
    [(71, 1), (72, 2)]
        .into_iter()
        .map(|(id, equipment_token)| VariantDefinitionRow {
            id,
            equipment_token,
            creator_link: 100_000,
            ..Default::default()
        })
        .collect()
}

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = scale_frame(1, tables.hour, COUNTRIES);
    let mut put = |table: TableId, view: TableView| {
        *frame.tables[table.index()] = view;
    };
    let unavailable = || TableView::Unavailable {
        reason: REFUSAL.into(),
    };
    if !tables.countries {
        put(TableId::Countries, unavailable());
    }
    put(
        TableId::AirWings,
        match &tables.wings {
            Some(wings) => TableView::Complete(TableRows::AirWings(wing_rows(wings))),
            None => unavailable(),
        },
    );
    put(
        TableId::WingEquipment,
        TableView::Complete(TableRows::WingEquipment(bank_rows(&tables.banks))),
    );
    put(
        TableId::VariantDefinitions,
        if tables.definitions {
            TableView::Complete(TableRows::VariantDefinitions(definition_rows()))
        } else {
            unavailable()
        },
    );
    put(
        TableId::LexicalTokens,
        TableView::Complete(TableRows::LexicalTokens(token_rows())),
    );
    put(
        TableId::VariantChanges,
        TableView::Complete(TableRows::VariantChanges(Vec::new())),
    );
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
/// walk's first hour (after a gap, the walk starts again), and the row frames of the same hours.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::with(FormatVersion::CURRENT).unwrap();
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = frames
            .last()
            .is_some_and(|last: &DecodedFrame| last.hour + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        let countries = tables.countries.then(country_rows);
        write(walk, TableId::Countries, countries.as_deref());
        write(walk, TableId::CountryLinks, Some(&link_rows()[..]));
        let wings = tables.wings.as_deref().map(wing_rows);
        write(walk, TableId::AirWings, wings.as_deref());
        write(
            walk,
            TableId::WingEquipment,
            Some(&bank_rows(&tables.banks)[..]),
        );
        let definitions = tables.definitions.then(definition_rows);
        write(walk, TableId::VariantDefinitions, definitions.as_deref());
        write(walk, TableId::LexicalTokens, Some(&token_rows()[..]));
        write::<VariantChangeRow>(walk, TableId::VariantChanges, Some(&[]));
        let cold = starts.contains(&tables.hour);
        if cold {
            projector = Projector::with(FormatVersion::CURRENT).unwrap();
        }
        let metrics = walk
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
                decoded.metrics.clone()
            })
            .unwrap();
        // The row oracle reads the same read evidence the native walk wrote.
        let mut observed = frame(tables);
        observed.metrics = metrics;
        frames.push(observed);
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
struct Built {
    pages: Memory,
    root: u64,
}

fn build(hours: Vec<facts::Hour>, cuts: &[i32], resume: Option<i32>) -> Built {
    let mut pages = Memory::default();
    let first = hours[0].hour;
    let mut writer = Writer::new(identity(), first).unwrap();
    let mut emit = Emit::new(None);
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
            emit = Emit::load(None, &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// A run as a key holds it: first and last hour, break, value.
type Raw = (i32, i32, bool, i128);

/// The runs a key holds of `value` of each census run: equal values going on without a break
/// are one, and a value after an hour of none breaks.
fn expected(runs: &[RawRun], value: impl Fn(&RawRun) -> Option<i128>) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    for run in runs {
        let Some(value) = value(run) else {
            continue;
        };
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

/// `runs` in hour order, each holder's adjacent runs at a rank one, without breaks: a rank is
/// held by its tag whatever starts its runs.
fn merged(mut runs: Vec<Raw>) -> Vec<Raw> {
    runs.sort_unstable();
    let mut out: Vec<Raw> = Vec::new();
    for (first, last, _, value) in runs {
        match out.last_mut() {
            Some(held) if held.1 + 1 == first && held.3 == value => held.1 = last,
            _ => out.push((first, last, false, value)),
        }
    }
    out
}

/// Holds the publication `built` to the row census of `frames`.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let scanned = scan::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let tags: BTreeSet<String> = directory
        .actors
        .iter()
        .map(|actor| String::from_utf8(actor.name.to_vec()).unwrap())
        .chain(scanned.series.keys().cloned())
        .collect();
    let accepted = snapshot.publication().accepted;
    for tag in &tags {
        let runs = scanned.series.get(tag).cloned().unwrap_or_default();
        let actor = directory.actor(tag.as_bytes());
        let key = |measure| actor.and_then(|actor| directory.key(actor, measure, None));
        // A tag no country ever held holds no run.
        let Some(mix) = key(MIX) else {
            assert!(runs.is_empty(), "{label}: {tag} has runs but no key");
            continue;
        };
        let mut spelled = Default::default();
        let published: Vec<RawRun> =
            engine::runs(&mut snapshot, mix, accepted.first, accepted.last)
                .unwrap()
                .iter()
                .map(|run| {
                    super::query::raw_run(&mut snapshot, &directory, &mut spelled, mix, run)
                        .unwrap()
                })
                .collect();
        assert_eq!(published, runs, "{label}: {tag}'s runs");
        for ownership in [AirForceOwnership::Roster, AirForceOwnership::Effective] {
            for measure in [AirForceMeasure::Planes, AirForceMeasure::Wings] {
                let channel = emit::channel(ownership.index(), measure == AirForceMeasure::Wings);
                let params = AirForceParams {
                    branch_id: "1".into(),
                    tags: Vec::new(),
                    top: None,
                    measure,
                    ownership,
                };
                assert_eq!(
                    raw(&mut snapshot, key(VALUES + channel as u64)),
                    expected(&runs, |run| run.total(&params).map(i128::from)),
                    "{label}: {tag}'s {measure:?} under {ownership:?}"
                );
            }
        }
    }
    for channel in 0..4 {
        let params = AirForceParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            top: None,
            measure: if channel % 2 == 0 {
                AirForceMeasure::Planes
            } else {
                AirForceMeasure::Wings
            },
            ownership: if channel < 2 {
                AirForceOwnership::Roster
            } else {
                AirForceOwnership::Effective
            },
        };
        let mut ranks: BTreeMap<usize, Vec<Raw>> = BTreeMap::new();
        dashboard_base::ranking::rank_runs(
            &scanned.series,
            |run| (run.first_hour, run.last_hour),
            |run| run.total(&params).map(i128::from),
            &AtomicBool::new(false),
            |position, tag, from, to| {
                let actor = i128::from(directory.actor(tag.as_bytes()).unwrap());
                ranks
                    .entry(position)
                    .or_default()
                    .push((from, to, false, actor));
                Ok(())
            },
        )
        .unwrap();
        for spans in ranks.values_mut() {
            *spans = merged(std::mem::take(spans));
        }
        let positions: Vec<u64> = directory
            .dimensions(FAMILY, POSITIONS + channel as u64)
            .iter()
            .map(|&(_, key)| key)
            .collect();
        assert_eq!(
            positions.len(),
            ranks.len(),
            "{label}: channel {channel}'s ranks"
        );
        for (position, key) in positions.into_iter().enumerate() {
            let held = merged(raw(&mut snapshot, Some(key)));
            assert_eq!(
                held, ranks[&position],
                "{label}: channel {channel}'s rank {position}"
            );
        }
    }
    let published = super::query::scanned(&directory);
    assert_eq!(
        published.unreadable_owners, scanned.unreadable_owners,
        "{label}: owners"
    );
    assert_eq!(
        (
            published.equipment_missing,
            published.equipment_refused,
            published.equipment_remainders
        ),
        (
            scanned.equipment_missing,
            scanned.equipment_refused,
            scanned.equipment_remainders
        ),
        "{label}: banks"
    );
    assert_eq!(
        published.undisplayable_tags, scanned.undisplayable_tags,
        "{label}: tags"
    );
}

/// Every placement of segment starts, cuts and a resume publishes what the row census reads.
#[test]
fn the_publication_holds_what_the_row_census_reads_across_segments_cuts_and_a_resume() {
    let hours = scenario();
    for (starts, cuts, resume) in [
        (&[][..], &[175][..], None),
        (&[110, 131, 150][..], &[109, 129, 149, 175][..], Some(129)),
        (
            &[103, 115, 125, 136, 155, 161][..],
            &[114, 133, 147, 175][..],
            Some(147),
        ),
        (
            &(101..=175).collect::<Vec<_>>()[..],
            &(100..=175)
                .filter(|hour| !(145..=146).contains(hour))
                .collect::<Vec<_>>()[..],
            Some(120),
        ),
    ] {
        let mut starts = starts.to_vec();
        if let Some(hour) = resume.and_then(|cut| hours.iter().find(|hour| hour.hour > cut)) {
            starts.push(hour.hour);
        }
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(&hours, &starts);
        let built = build(projected, cuts, resume);
        check(&built, &frames, &label);
    }
}

/// The scenario reads as the census counts it: unknown hours stay unknown, the inexact split
/// unknown by part, the variant names forgotten while the definitions are not complete.
#[test]
fn the_scenario_reads_as_the_census_counts_it() {
    let (_, frames) = projected(&scenario(), &[]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    let at = |hour: i32| {
        scanned.series[&tag]
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .cloned()
    };
    assert_eq!(at(125).unwrap().roster, None);
    assert_eq!(at(155).unwrap().roster, None);
    assert!(at(145).is_none());
    assert!(
        at(130)
            .unwrap()
            .roster
            .unwrap()
            .planes_by_token
            .contains_key("71/")
    );
    assert!(
        scanned.unreadable_owners[0] > 0
            && scanned.unreadable_owners[1] > scanned.unreadable_owners[0]
    );
    assert!(scanned.equipment_refused > 0);
}

/// Of 1,500 countries, an hour hands the emit only the slots whose counts changed, and nothing
/// when nothing changed.
#[test]
fn an_hour_hands_only_the_counts_that_changed() {
    let count = 1500;
    let countries: Vec<CountryRow> = (0..count)
        .map(|index| CountryRow {
            id: 100_000 + index,
            canonical_slot: index,
            tag: scale_tag(index),
            ..CountryRow::default()
        })
        .collect();
    let links: Vec<CountryLinkRow> = (0..count)
        .map(|index| CountryLinkRow {
            id: 100_000 + index,
            canonical_slot: index,
        })
        .collect();
    let wings = |changed: i32| -> Vec<(u64, u32, i32, i32)> {
        (0..count)
            .map(|index| {
                (
                    u64::from(index) + 1,
                    100_000 + index,
                    0,
                    if index == 7 { changed } else { 10 },
                )
            })
            .collect()
    };
    let banks = |wings: &[(u64, u32, i32, i32)]| -> Vec<(u64, u64, i64)> {
        wings
            .iter()
            .map(|&(id, _, _, planes)| (id, 71, i64::from(planes) * UNIT))
            .collect()
    };
    let mut walk = Synthetic::new(facts::TABLES, 1, 100).unwrap();
    let mut world = None;
    let mut projector = Projector::with(FormatVersion::CURRENT).unwrap();
    let mut out = Vec::new();
    for (ordinal, (hour, planes)) in [(100, 10), (101, 10), (102, 11)].into_iter().enumerate() {
        let wings = wings(planes);
        write(&mut walk, TableId::Countries, Some(&countries[..]));
        write(&mut walk, TableId::CountryLinks, Some(&links[..]));
        write(&mut walk, TableId::AirWings, Some(&wing_rows(&wings)[..]));
        write(
            &mut walk,
            TableId::WingEquipment,
            Some(&bank_rows(&banks(&wings))[..]),
        );
        write(
            &mut walk,
            TableId::VariantDefinitions,
            Some(&definition_rows()[..]),
        );
        write(&mut walk, TableId::LexicalTokens, Some(&token_rows()[..]));
        write::<VariantChangeRow>(&mut walk, TableId::VariantChanges, Some(&[]));
        walk.end(|state, manifest, _, first| {
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
                        hour,
                        run: 0,
                        ordinal: ordinal as u32,
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
    assert!(out[0].whole && out[0].counts.len() == 2 * count as usize);
    assert!(!out[1].whole && out[1].counts.is_empty() && out[1].live.is_none());
    let changed: Vec<_> = out[2]
        .counts
        .iter()
        .map(|&(channel, slot, _)| (channel, slot))
        .collect();
    assert_eq!(changed, [(0, 7), (1, 7)]);
}

/// A complete variant read names a variant from that hour, whether a scheduled read or a design
/// edit's witness supplied it: a wing counted before any read is renamed there, in a run that
/// goes on without a segment start.
#[test]
fn a_witness_read_names_a_variant_from_its_hour() {
    let planes = |part| {
        Arc::new(facts::Counts {
            planes: 5,
            wings: 1,
            parts: vec![(part, Some(5))],
            signatures: vec![(Arc::from(vec![part]), 1)],
        })
    };
    let variant = facts::Part::Variant(71);
    let hour = |at: i32, first: bool, names: facts::Names| facts::Hour {
        branch: 1,
        hour: at,
        named: if first {
            vec![(0, Arc::from("AAA"))]
        } else {
            Vec::new()
        },
        live: first.then(|| vec![(0, (0, 100_000))]),
        complete: [true; 2],
        whole: first,
        counts: if first {
            vec![(0, 0, planes(variant)), (1, 0, planes(variant))]
        } else {
            Vec::new()
        },
        flags: 0,
        undisplayable: first.then(Vec::new),
        names,
    };
    let hours = vec![
        hour(19, true, facts::Names::Carried),
        hour(
            20,
            false,
            facts::Names::Read {
                whole: true,
                tokens: vec![(71, Arc::from("fighter_equipment_0"))],
            },
        ),
        hour(21, false, facts::Names::Carried),
    ];
    let built = build(hours, &[21], None);
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let actor = directory.actor(b"AAA").unwrap();
    let mix = directory.key(actor, MIX, None).unwrap();
    let mut spelled = Default::default();
    let runs: Vec<RawRun> = engine::runs(&mut snapshot, mix, 19, 21)
        .unwrap()
        .iter()
        .map(|run| {
            super::query::raw_run(&mut snapshot, &directory, &mut spelled, mix, run).unwrap()
        })
        .collect();
    let named: Vec<_> = runs
        .iter()
        .map(|run| {
            let counts = run.roster.as_ref().unwrap();
            (
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                counts.planes_by_token.keys().cloned().collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        named,
        [
            (19, 19, true, vec!["71/".to_owned()]),
            (20, 21, false, vec!["71/fighter_equipment_0".to_owned()]),
        ]
    );
    let planes = directory.key(actor, VALUES, None).unwrap();
    assert_eq!(raw(&mut snapshot, Some(planes)), [(19, 21, true, 5)]);
}
