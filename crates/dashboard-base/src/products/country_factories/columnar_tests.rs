//! The family's columnar preparation held to its row scan ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every tag's value runs and supplying reads, each
//! ranked count, each part and each rank, the counts and each hour's refused reads, across
//! segment starts, a gap, cuts and a resume; the replies, as the row reply answers them; and an
//! hour of 1,500 countries handing the emit only what changed.

use super::{
    emit::{Emit, FAMILY, MIX, PARTS, RANKS, READS, REFUSED, SCHEMA, VALUES},
    facts::{self, Projector},
    row::Scan,
    *,
};
use crate::{
    prepared::{coverage::Coverage, engine},
    products::country_history::scan_histories,
    test_fixtures::{CAMPAIGN, continuation_records, file_source, records, source},
};
use dashboard_series::{
    Blob, Budget, Cache, Publication, Snapshot, Span, Writer,
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
    rows::{CountryLinkRow, CountryRow, RowCodec, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};

/// The fixed5 unit of one whole factory.
const UNIT: i64 = 100_000;

/// A factory row's recorded fields: military and civilian totals (controlled and owned alike),
/// military damage, consumer goods and civilian `unavailable`.
#[derive(Clone, Copy)]
struct Factory {
    military: i64,
    civilian: i64,
    damaged: i64,
    consumer: i32,
    unavailable: i32,
}

/// One hour as the recorder writes it: the countries (id, slot, tag), none where Countries is
/// unavailable; the links (raw id, slot); the factory rows by raw country, none where the table
/// is unavailable.
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: Option<Vec<(u32, u32, String)>>,
    links: Vec<(u32, u32)>,
    factories: Option<Vec<(u32, Factory)>>,
}

/// Hours 100 to 160 of four countries, without 130 and 131. A's military factories grow every
/// five hours, and its civilian `unavailable` is not the sum of its parts from 110 to 112; a
/// second raw id of A's slot holds a row from 115 to 117 (two rows: unknown); B's military total
/// is past what the game can show at 118 and 119; a row no link resolves from 122 to 124; the
/// table is unavailable at 125 and 126; B's country changes id at 135; C's civilian factories
/// pass A's at 140; the countries are unavailable at 145; C is renamed at 155. D never holds a
/// row. A's damage moves by fractions of a factory every hour.
fn scenario() -> Vec<Tables> {
    (100..=160)
        .filter(|hour| !(130..=131).contains(hour))
        .map(|hour| {
            let b = if hour >= 135 { 20 } else { 2 };
            let c = if hour >= 155 { "CCX" } else { "CCC" };
            let countries = (hour != 145).then(|| {
                vec![
                    (1, 1, "AAA".to_owned()),
                    (b, 2, "BBB".to_owned()),
                    (3, 3, c.to_owned()),
                    (4, 4, "DDD".to_owned()),
                ]
            });
            let mut links = vec![(1, 1), (b, 2), (3, 3), (4, 4), (11, 1)];
            links.sort_unstable();
            let a = Factory {
                military: (10 + i64::from(hour - 100) / 5) * UNIT,
                civilian: 20 * UNIT,
                damaged: i64::from(hour % 3) * 1_000,
                consumer: 2,
                unavailable: if (110..=112).contains(&hour) { 5 } else { 2 },
            };
            let b_row = Factory {
                military: if (118..=119).contains(&hour) {
                    i64::MAX
                } else {
                    15 * UNIT
                },
                civilian: 0,
                damaged: 0,
                consumer: 0,
                unavailable: 0,
            };
            let c_row = Factory {
                military: 0,
                civilian: if hour >= 140 { 40 * UNIT } else { 8 * UNIT },
                damaged: 0,
                consumer: 0,
                unavailable: 0,
            };
            let mut factories = vec![(1, a), (b, b_row), (3, c_row)];
            if (115..=117).contains(&hour) {
                factories.push((11, a));
            }
            if (122..=124).contains(&hour) {
                factories.push((99, c_row));
            }
            factories.sort_unstable_by_key(|&(country, _)| country);
            Tables {
                hour,
                countries,
                links,
                factories: (!(125..=126).contains(&hour)).then_some(factories),
            }
        })
        .collect()
}

fn country_rows(countries: &[(u32, u32, String)]) -> Vec<CountryRow> {
    let mut rows: Vec<_> = countries
        .iter()
        .map(|(id, canonical_slot, tag)| CountryRow {
            id: *id,
            canonical_slot: *canonical_slot,
            tag: tag.as_bytes().to_vec(),
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

fn factory_rows(factories: &[(u32, Factory)]) -> Vec<CountryFactoriesRow> {
    factories
        .iter()
        .map(|&(country, factory)| CountryFactoriesRow {
            country,
            military_total_raw: factory.military,
            military_controlled_raw: factory.military,
            military_owned_raw: factory.military,
            military_damaged_raw: factory.damaged,
            civilian_total_raw: factory.civilian,
            civilian_controlled_raw: factory.civilian,
            civilian_owned_raw: factory.civilian,
            consumer_goods: factory.consumer,
            civilian_unavailable: factory.unavailable,
            ..Default::default()
        })
        .collect()
}

const REFUSAL: &str = "synthetic refusal";

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = source(1, tables.hour, Some(1));
    let mut put = |table: TableId, view: TableView| {
        *frame.tables[table.index()] = view;
    };
    put(
        TableId::Countries,
        match &tables.countries {
            Some(countries) => TableView::Complete(TableRows::Countries(country_rows(countries))),
            None => TableView::Unavailable {
                reason: REFUSAL.into(),
            },
        },
    );
    put(
        TableId::CountryLinks,
        TableView::Complete(TableRows::CountryLinks(link_rows(&tables.links))),
    );
    put(
        TableId::CountryFactories,
        match &tables.factories {
            Some(rows) => TableView::Complete(TableRows::CountryFactories(factory_rows(rows))),
            None => TableView::Unavailable {
                reason: REFUSAL.into(),
            },
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

/// The projected hours of `hours`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again), and the row frames of the same hours.
fn projected(hours: &[Tables], starts: &[i32]) -> (Vec<facts::Hour>, Vec<DecodedFrame>) {
    let mut out = Vec::new();
    let mut frames = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT).unwrap();
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
        let factories = tables.factories.as_deref().map(factory_rows);
        write(walk, TableId::CountryFactories, factories.as_deref());
        let cold = starts.contains(&tables.hour);
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
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
        frames.push(frame(tables));
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

/// A run as a key holds it: first and last hour, break, value.
type Raw = (i32, i32, bool, i128);

/// The runs a key holds of `value` of each factory run: equal values going on without a break
/// are one, and a value after an hour of none breaks.
fn expected(runs: &[FactoryRun], value: impl Fn(&FactoryRun) -> i128) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    for run in runs {
        let value = value(run);
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

/// `tag`'s value runs as the publication holds them: each composition run with its event.
fn published_runs<P: dashboard_series::PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
) -> Vec<FactoryRun> {
    let Some(key) = directory
        .actor(tag.as_bytes())
        .and_then(|actor| directory.key(actor, MIX, None))
    else {
        return Vec::new();
    };
    raw(snapshot, Some(key))
        .into_iter()
        .map(|(first_hour, last_hour, starts_segment, _)| {
            let event = snapshot
                .latest_event(
                    dashboard_series::Selection {
                        key,
                        membership: None,
                    },
                    first_hour,
                    None,
                )
                .unwrap()
                .unwrap();
            FactoryRun {
                first_hour,
                last_hour,
                starts_segment,
                values: super::query::mix_of(snapshot, event).unwrap(),
            }
        })
        .collect()
}

/// The row scan of `frames`, and each hour's refused reads.
fn scanned(frames: &[DecodedFrame]) -> (ScannedFactories, Vec<(i32, usize)>) {
    let mut scan = Scan::default();
    let mut refused = Vec::new();
    for frame in frames {
        let before = scan.refused();
        scan.observe(frame, FormatVersion::CURRENT);
        refused.push((frame.hour, scan.refused() - before));
    }
    (scan.finish(), refused)
}

/// Holds the publication `built` to the row scan of `frames`: the series of each tag `sampled`
/// names, and the ranks of each of `channels`.
fn check(
    built: &Built,
    frames: &[DecodedFrame],
    sampled: impl Fn(&str) -> bool,
    channels: &[usize],
    label: &str,
) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let (scanned, refused) = scanned(frames);
    let tags: BTreeSet<String> = directory
        .actors
        .iter()
        .map(|actor| String::from_utf8(actor.name.to_vec()).unwrap())
        .chain(scanned.series.keys().cloned())
        .collect();
    for tag in tags.iter().filter(|tag| sampled(tag)) {
        let runs = scanned.series.get(tag).cloned().unwrap_or_default();
        assert_eq!(
            published_runs(&mut snapshot, &directory, tag),
            runs,
            "{label}: {tag}'s value runs"
        );
        let actor = directory.actor(tag.as_bytes());
        let key = |measure, dimension| actor.and_then(|actor| directory.key(actor, measure, dimension));
        let reads: Vec<FactoryReadRun> = raw(&mut snapshot, key(READS, None))
            .into_iter()
            .map(|(first_hour, last_hour, _, value)| {
                let hourly = value == super::emit::HOURLY;
                FactoryReadRun {
                    first_hour,
                    last_hour,
                    read_hour: if hourly { first_hour } else { value as i32 },
                    hourly,
                }
            })
            .collect();
        assert_eq!(
            reads,
            scanned.reads.get(tag).cloned().unwrap_or_default(),
            "{label}: {tag}'s supplying reads"
        );
        for channel in 0..CHANNELS {
            assert_eq!(
                raw(&mut snapshot, key(VALUES, Some(channel as u64))),
                expected(&runs, |run| i128::from(channel_counts(&run.values)[channel])),
                "{label}: {tag}'s channel {channel}"
            );
        }
        for ordinal in 0..FIELD_COUNT {
            assert_eq!(
                raw(&mut snapshot, key(PARTS, Some(ordinal as u64))),
                expected(&runs, |run| i128::from(run.values[ordinal])),
                "{label}: {tag}'s part {ordinal}"
            );
        }
    }
    for &channel in channels {
        let top = rank_selection(channel);
        let mut spans: BTreeMap<usize, Vec<Raw>> = BTreeMap::new();
        crate::ranking::rank_runs(
            &scanned.series,
            |run| (run.first_hour, run.last_hour),
            |run| Some(i128::from(count_for(&run.values, &top))),
            &AtomicBool::new(false),
            |position, tag, from, to| {
                let actor = i128::from(directory.actor(tag.as_bytes()).unwrap());
                spans.entry(position).or_default().push((from, to, false, actor));
                Ok(())
            },
        )
        .unwrap();
        // A position's spans in hour order, one tag's adjacent spans as one run.
        let mut ranks: BTreeMap<usize, Vec<Raw>> = BTreeMap::new();
        for (position, mut held) in spans {
            held.sort_unstable();
            let merged = ranks.entry(position).or_default();
            for span in held {
                match merged.last_mut() {
                    Some(last) if last.1 + 1 == span.0 && last.3 == span.3 => last.1 = span.1,
                    _ => merged.push(span),
                }
            }
        }
        let positions: Vec<u64> = directory
            .dimensions(FAMILY, RANKS + channel as u64)
            .iter()
            .map(|&(_, key)| key)
            .collect();
        assert_eq!(positions.len(), ranks.len(), "{label}: channel {channel}'s ranks");
        for (position, key) in positions.into_iter().enumerate() {
            assert_eq!(
                raw(&mut snapshot, Some(key)),
                ranks[&position],
                "{label}: channel {channel}'s rank {position}"
            );
        }
    }
    let mut hourly: Vec<Raw> = Vec::new();
    for &(hour, count) in &refused {
        let follows = hourly.last().is_some_and(|last| last.1 + 1 == hour);
        match hourly.last_mut() {
            Some(last) if follows && last.3 == count as i128 => last.1 = hour,
            _ => hourly.push((hour, hour, !follows, count as i128)),
        }
    }
    assert_eq!(
        raw(&mut snapshot, directory.key(FAMILY, REFUSED, None)),
        hourly,
        "{label}: each hour's refused reads"
    );
    let published = super::query::scanned(&directory);
    assert_eq!(published.unexplained, scanned.unexplained, "{label}: unexplained");
    assert_eq!(published.unreadable, scanned.unreadable, "{label}: unreadable");
    assert_eq!(published.unnamed, scanned.unnamed, "{label}: unnamed");
    assert_eq!(
        published.recorded,
        scanned.series.keys().cloned().collect::<BTreeSet<_>>(),
        "{label}: recorded"
    );
}

/// Every placement of segment starts, cuts and a resume publishes what the row scan reads, and
/// the same publication whatever the placement.
#[test]
fn the_publication_holds_what_the_row_scan_reads_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    let channels: Vec<usize> = (0..CHANNELS).collect();
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
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(&hours, starts);
        let built = build(projected, cuts, resume, &budget);
        check(&built, &frames, |_| true, &channels, &label);
    }
}

/// The scenario's counts as the row scan reads them: two rows of one holder, a total the game
/// cannot show and a row no link resolves are unreadable, never zero; an `unavailable` other
/// than its parts is counted.
#[test]
fn the_scenario_counts_what_the_game_could_not_show() {
    let (_, frames) = projected(&scenario(), &[]);
    let (scanned, _) = scanned(&frames);
    assert_eq!(scanned.unreadable, 3 + 2 + 3);
    assert_eq!(
        scanned.unexplained,
        BTreeMap::from([(FactoryGroupKey::Civilian, 3)])
    );
    assert!(
        !scanned.series["AAA"]
            .iter()
            .any(|run| run.first_hour <= 117 && 115 <= run.last_hour),
        "two rows of one holder are unknown"
    );
    assert!(
        !scanned.series["BBB"]
            .iter()
            .any(|run| run.first_hour <= 119 && 118 <= run.last_hour),
        "a total the game cannot show is unknown"
    );
}

/// A publication's replies are the row reply's: ranked and listed countries, and the values at
/// hours.
#[test]
fn replies_from_the_publication_are_the_row_replies() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    let (projected, frames) = projected(&hours, &[110, 140]);
    let built = build(projected, &[129, 160], Some(129), &budget);
    let (first, second): (Vec<_>, Vec<_>) = frames.into_iter().partition(|frame| frame.hour < 130);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, first)]).unwrap();
    let replay = replay
        .append(&[continuation_records(&replay, 1, second)])
        .unwrap();
    let cancelled = AtomicBool::new(false);
    let history = scan_histories(&replay, 1, false, &[], &cancelled).unwrap();
    let (source, _root) = file_source(replay);
    let context = LoadContext { content: None };
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let accepted = Coverage {
        schema: crate::prepared::coverage::SCHEMA,
        first_hour: 100,
        last_hour: 160,
        complete: true,
    };
    // Each series' runs, reads and membership.
    let series = |reply: &serde_json::Value| -> Vec<serde_json::Value> {
        reply["series"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                serde_json::json!([item["tag"], item["runs"], item["reads"], item["membership"]])
            })
            .collect()
    };
    for (tags, top) in [
        (vec![], Some(("built", vec![]))),
        (vec![], Some(("free", vec!["civilian"]))),
        (vec!["AAA".to_owned()], None),
        (vec!["BBB".to_owned(), "CCC".to_owned()], None),
        (vec!["CCX".to_owned()], None),
    ] {
        let top = top.map(|(mode, groups)| serde_json::json!({"count": 2, "mode": mode, "groups": groups}));
        let params = serde_json::json!({
            "branchId": "1", "tags": tags,
            "family": {"kind": "country-factories", "top": top},
            "from": 100, "to": 160, "width": 100,
        });
        let reply = super::query::series(
            &source,
            "series-summary",
            &params,
            &context,
            &mut snapshot,
            &directory,
            &history,
            Some(accepted.clone()),
            &cancelled,
        )
        .unwrap();
        let selection: FactoriesParams = serde_json::from_value(serde_json::json!({
            "branchId": "1", "tags": tags, "top": top,
        }))
        .unwrap();
        let expected = super::row::load(&source, &selection, &context, &cancelled).unwrap();
        let expected = serde_json::to_value(expected).unwrap();
        assert_eq!(series(&reply), series(&expected), "{params}");
        assert_eq!(reply["warnings"], expected["warnings"], "{params}");
    }
    let reads: Vec<_> = [100, 116, 125, 133, 150, 160]
        .into_iter()
        .map(|hour| serde_json::json!({"tag": "AAA", "hour": hour}))
        .collect();
    let state = super::query::series(
        &source,
        "series-state",
        &serde_json::json!({
            "branchId": "1",
            "family": {"kind": "country-factories", "top": null, "mode": "built", "groups": []},
            "reads": reads,
        }),
        &context,
        &mut snapshot,
        &directory,
        &history,
        Some(accepted.clone()),
        &cancelled,
    )
    .unwrap();
    let held: Vec<_> = state["series"][0]["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| run["firstHour"].as_i64().unwrap())
        .collect();
    // Unknown at 116 (two rows of one holder) and 125 (the table unavailable): no value, never
    // zero.
    assert_eq!(held, [100, 133, 150, 160]);
}

/// Of 1,500 countries, an hour hands the emit only the holders whose values changed, nothing
/// when only fractions of a factory move, and nothing when nothing changes; and the ranks of
/// every channel follow the row scan's through a segment start, a cut and a resume while counts
/// tie, rise and fall.
#[test]
fn an_hour_of_1500_countries_hands_on_only_what_changed() {
    let countries: Vec<(u32, u32, String)> = (0..1_500)
        .map(|index| (index + 1, index + 1, format!("T{index:04}")))
        .collect();
    let links: Vec<(u32, u32)> = countries.iter().map(|&(id, slot, _)| (id, slot)).collect();
    let hours: Vec<Tables> = (0..12)
        .map(|offset| {
            let hour = 200 + offset;
            let factories = countries
                .iter()
                .map(|&(id, _, _)| {
                    let base = i64::from(id % 7) * UNIT;
                    let military = match offset {
                        // Only fractions move at 201.
                        1 => base + 500,
                        // One country rises at 202 and falls at 205; many tie from 207.
                        2..=4 if id == 9 => base + 50 * UNIT,
                        7.. if id % 3 == 0 => 3 * UNIT,
                        _ => base,
                    };
                    (
                        id,
                        Factory {
                            military,
                            civilian: i64::from(id % 11) * UNIT,
                            damaged: 0,
                            consumer: 0,
                            unavailable: 0,
                        },
                    )
                })
                .collect();
            Tables {
                hour,
                countries: Some(countries.clone()),
                links: links.clone(),
                factories: Some(factories),
            }
        })
        .collect();
    let (projected, frames) = projected(&hours, &[206]);
    let handed = |hour: i32| {
        let item = projected.iter().find(|item| item.hour == hour).unwrap();
        item.facts.as_ref().map(|facts| {
            let stocks = facts.stocks.as_ref();
            (
                stocks.is_some_and(|stocks| stocks.whole),
                stocks.map_or(0, |stocks| stocks.slots.len()),
            )
        })
    };
    assert_eq!(handed(200), Some((true, 1_500)), "a segment's first hour reads every holder");
    assert_eq!(handed(201), None, "fractions of a factory change nothing");
    assert_eq!(handed(202), Some((false, 1)), "one holder changed");
    assert_eq!(handed(203), None, "nothing changed");
    let budget = Budget::new(512 << 20);
    let built = build(projected, &[204, 211], Some(204), &budget);
    // Every 50th country's series, and the one that rises and falls.
    let sampled = |tag: &str| {
        tag == "T0008" || tag[1..].parse::<u32>().is_ok_and(|index| index % 50 == 0)
    };
    check(&built, &frames, sampled, &[0, 6, 13, CHANNELS - 1], "1,500 countries");
}
