//! The family's columnar preparation held to its row census ([`super::scan`]) on the same
//! hours, written as the native recorder writes them: every tag's runs under each channel with
//! their compositions, the totals, maximum hull HP, the ranks and the census's counts, across
//! segment starts, a gap, cuts and a resume.

use super::{
    emit::{self, Emit, FAMILY, MAXIMUM, MIX, POSITIONS, SCHEMA, VALUES},
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
    campaign::Replay,
    segments::{self, Operator},
};
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryLinkRow, CountryRow, LexicalTokenRow, RowCodec, ShipCounterInputs, ShipModelRow,
        ShipRow, TableId, TableRows, TaskForceRow,
    },
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};

const REFUSAL: &str = "synthetic unavailable table";
const COUNTRIES: u32 = 4;

/// One hour as the recorder writes it: which tables are complete, whether the second country is
/// away, whether the second class token is spelled anew, and the task forces (id, roster owner,
/// logical owner), ships (id, task force, class token, hull HP) and models (ship, maximum).
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: bool,
    forces: bool,
    ships: bool,
    lexical: bool,
    away: bool,
    respelled: bool,
    force_rows: Vec<(u64, u32, u32)>,
    ship_rows: Vec<(u64, u64, i32, i64)>,
    models: Vec<(u64, Option<i64>)>,
}

/// Hours 100 to 175 without 145 and 146. Country 0's ship is damaged at 105 and repaired at
/// 120; country 1's task force counts for country 0 as logical owner; country 2 loses a ship at
/// 118; country 3 gains a task force at 115, which moves to country 2's roster at 130 and goes
/// at 140; the second class is spelled anew from 120 to 124; a ship loses its model counter
/// from 110 to 112 and another's maximum grows at 108; the ships are unavailable at 125, the
/// tokens from 130 to 132, the task forces at 135 and the countries at 150; a task force no
/// link resolves holds a ship at 155 and 156, beside a ship of negative hull HP, and the ships
/// are unavailable at 157 after them; country 1 is away from 160 to 162; a ship of negative hull
/// HP at 165.
fn scenario() -> Vec<Tables> {
    (100..=175)
        .filter(|hour| !(145..=146).contains(hour))
        .map(|hour| {
            let damaged = match hour {
                ..=104 => 100_000,
                105..=119 => 90_000,
                _ => 95_000,
            };
            let mut force_rows = vec![
                (1, 100_000, 100_000),
                (2, 100_001, 100_000),
                (3, 100_002, 100_002),
            ];
            let mut ship_rows = vec![(1, 1, 41, damaged), (2, 2, 42, 80_000), (3, 3, 41, 70_000)];
            if hour < 118 {
                ship_rows.push((4, 3, 42, 60_000));
            }
            if (115..140).contains(&hour) {
                force_rows.push((4, if hour < 130 { 100_003 } else { 100_002 }, 100_003));
                ship_rows.push((5, 4, 42, 50_000));
            }
            if (155..=156).contains(&hour) {
                force_rows.push((5, 999, 999));
                ship_rows.push((6, 5, 41, 40_000));
            }
            if hour == 165 || (155..=156).contains(&hour) {
                ship_rows.push((7, 3, 41, -1));
            }
            ship_rows.sort_unstable();
            let models = ship_rows
                .iter()
                .map(|&(ship, ..)| {
                    let maximum = match ship {
                        4 if (110..=112).contains(&hour) => None,
                        2 if hour >= 108 => Some(100_500),
                        _ => Some(100_000),
                    };
                    (ship, maximum)
                })
                .collect();
            Tables {
                hour,
                countries: hour != 150,
                forces: hour != 135,
                ships: hour != 125 && hour != 157,
                lexical: !(130..=132).contains(&hour),
                away: (160..=162).contains(&hour),
                respelled: (120..=124).contains(&hour),
                force_rows,
                ship_rows,
                models,
            }
        })
        .collect()
}

fn country_rows(away: bool) -> Vec<CountryRow> {
    (0..COUNTRIES)
        .filter(|&index| !(away && index == 1))
        .map(|index| CountryRow {
            id: 100_000 + index,
            canonical_slot: index,
            tag: scale_tag(index),
            ..CountryRow::default()
        })
        .collect()
}

fn link_rows(away: bool) -> Vec<CountryLinkRow> {
    (0..COUNTRIES)
        .filter(|&index| !(away && index == 1))
        .map(|index| CountryLinkRow {
            id: 100_000 + index,
            canonical_slot: index,
        })
        .collect()
}

fn force_rows(forces: &[(u64, u32, u32)]) -> Vec<TaskForceRow> {
    forces
        .iter()
        .map(|&(id, roster_owner, logical_owner)| TaskForceRow {
            id,
            roster_owner,
            logical_owner,
            ..Default::default()
        })
        .collect()
}

fn ship_rows(ships: &[(u64, u64, i32, i64)]) -> Vec<ShipRow> {
    ships
        .iter()
        .map(|&(id, task_force, class_id, strength)| ShipRow {
            id,
            task_force,
            class_id,
            strength,
            ..Default::default()
        })
        .collect()
}

fn model_rows(models: &[(u64, Option<i64>)]) -> Vec<ShipModelRow> {
    models
        .iter()
        .map(|&(ship, maximum)| ShipModelRow {
            ship,
            counter: maximum.map(|maximum_strength| ShipCounterInputs {
                maximum_strength,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect()
}

fn token_rows(respelled: bool) -> Vec<LexicalTokenRow> {
    vec![
        LexicalTokenRow {
            token: 41,
            literal: b"mod_cruiser".to_vec(),
        },
        LexicalTokenRow {
            token: 42,
            literal: if respelled {
                b"mod_destroyer_refit".to_vec()
            } else {
                b"mod_destroyer".to_vec()
            },
        },
    ]
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
    put(
        TableId::Countries,
        if tables.countries {
            TableView::Complete(TableRows::Countries(country_rows(tables.away)))
        } else {
            unavailable()
        },
    );
    put(
        TableId::CountryLinks,
        TableView::Complete(TableRows::CountryLinks(link_rows(tables.away))),
    );
    put(
        TableId::TaskForces,
        if tables.forces {
            TableView::Complete(TableRows::TaskForces(force_rows(&tables.force_rows)))
        } else {
            unavailable()
        },
    );
    put(
        TableId::Ships,
        if tables.ships {
            TableView::Complete(TableRows::Ships(ship_rows(&tables.ship_rows)))
        } else {
            unavailable()
        },
    );
    put(
        TableId::ShipModels,
        TableView::Complete(TableRows::ShipModels(model_rows(&tables.models))),
    );
    put(
        TableId::LexicalTokens,
        if tables.lexical {
            TableView::Complete(TableRows::LexicalTokens(token_rows(tables.respelled)))
        } else {
            unavailable()
        },
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

/// Writes every table `tables` holds into `walk`.
fn write_hour(walk: &mut Synthetic, tables: &Tables) {
    let countries = tables.countries.then(|| country_rows(tables.away));
    write(walk, TableId::Countries, countries.as_deref());
    write(walk, TableId::CountryLinks, Some(&link_rows(tables.away)[..]));
    let forces = tables.forces.then(|| force_rows(&tables.force_rows));
    write(walk, TableId::TaskForces, forces.as_deref());
    let ships = tables.ships.then(|| ship_rows(&tables.ship_rows));
    write(walk, TableId::Ships, ships.as_deref());
    write(walk, TableId::ShipModels, Some(&model_rows(&tables.models)[..]));
    let tokens = tables.lexical.then(|| token_rows(tables.respelled));
    write(walk, TableId::LexicalTokens, tokens.as_deref());
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
        write_hour(walk, tables);
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
fn expected(runs: &[RawNavyRun], value: impl Fn(&RawNavyRun) -> Option<i64>) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    for run in runs {
        let Some(value) = value(run).map(i128::from) else {
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

/// A run with its classes named by token and spelling, which each census numbers its own way.
type Named = (
    i32,
    i32,
    bool,
    Option<i64>,
    Option<i64>,
    BTreeMap<ClassKey, i64>,
);

fn named(run: &RawNavyRun, classes: &[ClassKey]) -> Named {
    (
        run.first_hour,
        run.last_hour,
        run.starts_segment,
        run.total,
        run.reference_total,
        run.classes
            .iter()
            .map(|(&class, &amount)| (classes[class as usize].clone(), amount))
            .collect(),
    )
}

/// Holds the publication `built` to the row census of `frames`.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let scanned = scan::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let classes = super::query::classes(&directory).unwrap();
    // The reply's class warnings come in class number order.
    assert_eq!(*classes, *scanned.classes, "{label}: class numbers");
    let empty = Arc::new(BTreeMap::new());
    let accepted = snapshot.publication().accepted;
    for channel in 0..4 {
        let ownership = channel / 2;
        let strength = channel % 2 == 1;
        let series = if strength {
            &scanned.strength_series[ownership]
        } else {
            &scanned.series[ownership]
        };
        let tags: BTreeSet<String> = directory
            .actors
            .iter()
            .map(|actor| String::from_utf8(actor.name.to_vec()).unwrap())
            .chain(series.keys().cloned())
            .collect();
        for tag in &tags {
            let runs = series.get(tag).cloned().unwrap_or_default();
            let actor = directory.actor(tag.as_bytes());
            let key = |measure| actor.and_then(|actor| directory.key(actor, measure, None));
            // A tag no country ever held holds no run.
            let Some(mix) = key(MIX + channel as u64) else {
                assert!(runs.is_empty(), "{label}: {tag} has runs but no key");
                continue;
            };
            let published: Vec<Named> =
                engine::runs(&mut snapshot, mix, accepted.first, accepted.last)
                    .unwrap()
                    .iter()
                    .map(|run| {
                        let raw = super::query::raw_run(&mut snapshot, mix, run, &empty).unwrap();
                        named(&raw, &classes)
                    })
                    .collect();
            let census: Vec<Named> = runs
                .iter()
                .map(|run| named(run, &scanned.classes))
                .collect();
            assert_eq!(published, census, "{label}: {tag}'s runs of channel {channel}");
            assert_eq!(
                raw(&mut snapshot, key(VALUES + channel as u64)),
                expected(&runs, |run| run.total),
                "{label}: {tag}'s totals of channel {channel}"
            );
            if strength {
                assert_eq!(
                    raw(&mut snapshot, key(MAXIMUM + ownership as u64)),
                    expected(&runs, |run| run.reference_total),
                    "{label}: {tag}'s maximum hull HP of channel {channel}"
                );
            }
        }
        let mut ranks: BTreeMap<usize, Vec<Raw>> = BTreeMap::new();
        dashboard_base::ranking::rank_runs(
            series,
            |run| (run.first_hour, run.last_hour),
            |run| run.total.map(i128::from),
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
    assert_eq!(published.unread, scanned.unread, "{label}: unread");
    assert_eq!(
        published.unattributed, scanned.unattributed,
        "{label}: unattributed"
    );
    assert_eq!(
        (published.refused_strength, published.maximum_overflows),
        (scanned.refused_strength, scanned.maximum_overflows),
        "{label}: notes"
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
            Some(121),
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

/// The scenario reads as the census counts it: unknown hours stay unknown, a respelled class is
/// another class, a refused hull HP keeps its maximum, and the counts it notes are counted.
#[test]
fn the_scenario_reads_as_the_census_counts_it() {
    let (_, frames) = projected(&scenario(), &[]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan::scan(&replay, 1, &AtomicBool::new(false)).unwrap();
    let at = |series: &BTreeMap<String, Vec<RawNavyRun>>, index: u32, hour: i32| {
        series[&String::from_utf8(scale_tag(index)).unwrap()]
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .cloned()
    };
    assert_eq!(at(&scanned.series[0], 0, 125).unwrap().total, None);
    assert_eq!(at(&scanned.series[0], 0, 155).unwrap().total, None);
    assert!(at(&scanned.series[0], 0, 145).is_none());
    assert!(at(&scanned.series[0], 1, 161).is_none());
    let refused = at(&scanned.strength_series[0], 2, 165).unwrap();
    assert_eq!(refused.total, None);
    assert!(refused.reference_total.is_some());
    let spelled = |hour| {
        at(&scanned.series[0], 1, hour)
            .unwrap()
            .classes
            .keys()
            .map(|&class| scanned.classes[class as usize].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(spelled(121), [(42, Some("mod_destroyer_refit".to_owned()))]);
    assert_eq!(spelled(131), [(42, None)]);
    assert_eq!(scanned.unread, 4);
    assert!(scanned.unattributed[0] > 0 && scanned.unattributed[1] > 0);
    assert!(scanned.refused_strength[0] > 0);
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
    let forces: Vec<TaskForceRow> = (0..count)
        .map(|index| TaskForceRow {
            id: u64::from(index) + 1,
            roster_owner: 100_000 + index,
            logical_owner: 100_000 + index,
            ..Default::default()
        })
        .collect();
    let ships = |damaged: i64| -> Vec<ShipRow> {
        (0..count)
            .map(|index| ShipRow {
                id: u64::from(index) + 1,
                task_force: u64::from(index) + 1,
                class_id: 41,
                strength: if index == 7 { damaged } else { 100_000 },
                ..Default::default()
            })
            .collect()
    };
    let models: Vec<ShipModelRow> = (0..count)
        .map(|index| ShipModelRow {
            ship: u64::from(index) + 1,
            counter: Some(ShipCounterInputs {
                maximum_strength: 100_000,
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect();
    let mut walk = Synthetic::new(facts::TABLES, 1, 100).unwrap();
    let mut world = None;
    let mut projector = Projector::with(FormatVersion::CURRENT).unwrap();
    let mut out = Vec::new();
    for (ordinal, (hour, damaged)) in [(100, 100_000), (101, 100_000), (102, 90_000)]
        .into_iter()
        .enumerate()
    {
        write(&mut walk, TableId::Countries, Some(&countries[..]));
        write(&mut walk, TableId::CountryLinks, Some(&links[..]));
        write(&mut walk, TableId::TaskForces, Some(&forces[..]));
        write(&mut walk, TableId::Ships, Some(&ships(damaged)[..]));
        write(&mut walk, TableId::ShipModels, Some(&models[..]));
        write(&mut walk, TableId::LexicalTokens, Some(&token_rows(false)[..]));
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
    assert!(out[0].spelled_whole && out[0].spellings.len() == 1);
    assert!(!out[1].whole && out[1].counts.is_empty() && out[1].live.is_none());
    assert!(!out[1].spelled_whole && out[1].spellings.is_empty());
    let changed: Vec<_> = out[2]
        .counts
        .iter()
        .map(|&(channel, slot, _)| (channel, slot))
        .collect();
    assert_eq!(changed, [(0, 7), (1, 7)]);
    assert_eq!(out[2].counts[0].2.hp, Some(90_000));
}

/// A respelled class starts a new run that goes on without a segment start, and its parts are
/// the respelled class's from that hour.
#[test]
fn a_respelled_class_is_another_class_from_its_hour() {
    let counts = Arc::new(facts::Counts {
        ships: 2,
        hp: Some(150_000),
        maximum: Some(200_000),
        overflow: false,
        classes: vec![(42, 2, 150_000)],
    });
    let hour = |at: i32, first: bool, spellings: Vec<(u32, Option<Arc<str>>)>| facts::Hour {
        branch: 1,
        hour: at,
        named: if first {
            vec![(0, Arc::from("AAA"))]
        } else {
            Vec::new()
        },
        live: first.then(|| vec![(0, (0, 100_000))]),
        undisplayable: first.then(Vec::new),
        census: true,
        join: first.then_some(1),
        read: true,
        unattributed: [0; 2],
        complete: [true; 2],
        whole: first,
        counts: if first {
            vec![(0, 0, Arc::clone(&counts)), (1, 0, Arc::clone(&counts))]
        } else {
            Vec::new()
        },
        spelled_whole: first,
        seen: spellings.iter().map(|&(token, _)| token).collect(),
        spellings,
    };
    let hours = vec![
        hour(19, true, vec![(42, Some(Arc::from("mod_destroyer")))]),
        hour(20, false, vec![(42, Some(Arc::from("mod_destroyer_refit")))]),
        hour(21, false, Vec::new()),
    ];
    let built = build(hours, &[21], None);
    let mut snapshot = Snapshot::open(built.pages.clone(), built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let classes = super::query::classes(&directory).unwrap();
    let actor = directory.actor(b"AAA").unwrap();
    let empty = Arc::new(BTreeMap::new());
    for channel in 0..4 {
        let mix = directory.key(actor, MIX + channel, None).unwrap();
        let runs: Vec<Named> = engine::runs(&mut snapshot, mix, 19, 21)
            .unwrap()
            .iter()
            .map(|run| named(&super::query::raw_run(&mut snapshot, mix, run, &empty).unwrap(), &classes))
            .collect();
        let amount = if channel % 2 == 0 { 2 } else { 150_000 };
        let reference = (channel % 2 == 1).then_some(200_000);
        let total = if channel % 2 == 0 { 2 } else { 150_000 };
        assert_eq!(
            runs,
            [
                (
                    19,
                    19,
                    true,
                    Some(total),
                    reference,
                    BTreeMap::from([((42, Some("mod_destroyer".to_owned())), amount)])
                ),
                (
                    20,
                    21,
                    false,
                    Some(total),
                    reference,
                    BTreeMap::from([((42, Some("mod_destroyer_refit".to_owned())), amount)])
                ),
            ],
            "channel {channel}"
        );
        let totals = directory.key(actor, VALUES + channel, None).unwrap();
        assert_eq!(
            raw(&mut snapshot, Some(totals)),
            [(19, 21, true, i128::from(total))]
        );
        // Without content each class is its own unresolved part.
        let parts = directory.dimensions(actor, emit::parts(channel as usize, 0));
        assert_eq!(parts.len(), 2, "channel {channel}'s parts");
    }
}
