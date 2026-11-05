//! The family's columnar preparation held to its row census ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every tag's stockpile runs, totals, counts and
//! ranks, the complete reads and their empty flags, the counts and spellings, and the event
//! feed's increases and refusals, across segment starts, a gap, cuts and a resume; and the
//! replies, as the row reply answers them.

use super::{
    emit::{Emit, FAMILY, MIX, PARTS, POSITIONS, SCHEMA, TOTAL},
    facts::{self, Projector},
    *,
};
use dashboard_base::{
    prepared::{coverage::Coverage, engine},
    products::{LoadContext, country_history::scan_histories},
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
    rows::{CountryLinkRow, CountryRow, LexicalTokenRow, NukeRow, RowCodec, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::atomic::AtomicBool;

/// One hour as the recorder writes it: the countries (id, slot, tag), the links (raw id, slot),
/// the Nukes rows (country, ordinal, raw amount), none where the table is unavailable, and the
/// tokens.
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: Option<Vec<(u32, u32, &'static str)>>,
    links: Vec<(u32, u32)>,
    nukes: Option<Vec<(u32, u32, i64)>>,
    tokens: bool,
}

const UNIT: i64 = 100_000_000;

/// Hours 100 to 160 of four countries, without 130 and 131. Everything is zero through 102
/// (empty reads); A's and C's stockpiles grow at 103. A's grows at 105, gains a second type at
/// 108 and grows at 120;
/// a second raw id of A's slot holds a row of its type from 115 to 117 (unknown); B holds a
/// zero stockpile; C holds types 0 and 2 from 112 to 118 (a hole: its total is unknown, and
/// type 2 is refused); a row no link resolves from 122 to 124; the table is unavailable at 125
/// and 126; B's country changes id at 135; the countries are unavailable at 145; C is renamed at
/// 155. D never holds a row.
fn scenario() -> Vec<Tables> {
    (100..=160)
        .filter(|hour| !(130..=131).contains(hour))
        .map(|hour| {
            let b = if hour >= 135 { 20 } else { 2 };
            let c = if hour >= 155 { "CCX" } else { "CCC" };
            let countries = (hour != 145).then(|| vec![(1, 1, "AAA"), (b, 2, "BBB"), (3, 3, c), (4, 4, "DDD")]);
            let mut links = vec![(1, 1), (b, 2), (3, 3), (4, 4), (11, 1)];
            links.sort_unstable();
            let a = match hour {
                ..=102 => 0,
                103..=104 => 1,
                105..=119 => 2,
                _ => 5,
            };
            let c_amount = if hour <= 102 { 0 } else { 3 * UNIT };
            let mut nukes = vec![(1, 0, a * UNIT), (b, 0, 0), (b, 1, 0), (3, 0, c_amount)];
            if hour >= 108 {
                nukes.push((1, 1, UNIT));
            }
            if (115..=117).contains(&hour) {
                nukes.push((11, 0, UNIT));
            }
            if (112..=118).contains(&hour) {
                nukes.push((3, 2, UNIT));
            }
            if (122..=124).contains(&hour) {
                nukes.push((99, 0, UNIT));
            }
            nukes.sort_unstable_by_key(|&(country, ordinal, _)| (country, ordinal));
            Tables {
                hour,
                countries,
                links,
                nukes: (!(125..=126).contains(&hour)).then_some(nukes),
                tokens: hour >= 101,
            }
        })
        .collect()
}

fn country_rows(countries: &[(u32, u32, &str)]) -> Vec<CountryRow> {
    let mut rows: Vec<_> = countries
        .iter()
        .map(|&(id, canonical_slot, tag)| CountryRow {
            id,
            canonical_slot,
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

fn nuke_rows(nukes: &[(u32, u32, i64)]) -> Vec<NukeRow> {
    nukes
        .iter()
        .map(|&(country, nuke_type, amount_raw)| NukeRow {
            country,
            nuke_type,
            amount_raw,
            ready: 0,
        })
        .collect()
}

fn token_rows(spelled: bool) -> Vec<LexicalTokenRow> {
    if !spelled {
        return Vec::new();
    }
    let mut rows = vec![
        LexicalTokenRow {
            token: NUCLEAR_BOMB_TOKEN,
            literal: b"fission".to_vec(),
        },
        LexicalTokenRow {
            token: THERMONUCLEAR_BOMB_TOKEN,
            literal: b"fusion".to_vec(),
        },
    ];
    rows.sort_unstable_by_key(|row| row.token);
    rows
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
        TableId::Nukes,
        match &tables.nukes {
            Some(nukes) => TableView::Complete(TableRows::Nukes(nuke_rows(nukes))),
            None => TableView::Unavailable {
                reason: REFUSAL.into(),
            },
        },
    );
    put(
        TableId::LexicalTokens,
        TableView::Complete(TableRows::LexicalTokens(token_rows(tables.tokens))),
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
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
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
        let nukes = tables.nukes.as_deref().map(nuke_rows);
        write(walk, TableId::Nukes, nukes.as_deref());
        write(walk, TableId::LexicalTokens, Some(&token_rows(tables.tokens)[..]));
        let cold = starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
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
    let mut emit = Emit::new(FormatVersion::CURRENT);
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
            emit = Emit::load(FormatVersion::CURRENT, &directory, &saved).unwrap();
        }
    }
    assert!(hours.next().is_none());
    Built { pages, root }
}

/// A run as a key holds it: first and last hour, break, value.
type Raw = (i32, i32, bool, i128);

/// The runs a key holds of `value` of each stockpile run: equal values going on without a
/// break are one, and a value after an hour of none breaks.
fn expected(runs: &[NukeRun], value: impl Fn(&NukeRun) -> Option<i128>) -> Vec<Raw> {
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

/// `tag`'s stockpile runs as the publication holds them: each composition run with its event.
fn published_runs<P: dashboard_series::PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    tag: &str,
) -> Vec<NukeRun> {
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
            let (counts, total) = super::query::mix_of(snapshot, event).unwrap();
            NukeRun {
                first_hour,
                last_hour,
                starts_segment,
                counts,
                total,
            }
        })
        .collect()
}

/// Holds the publication `built` to the row census of `frames`.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let mut census = row::Census::default();
    for frame in frames {
        census.observe(frame, FormatVersion::CURRENT);
    }
    let (scanned, feed) = census.finish();
    let tags: BTreeSet<String> = directory
        .actors
        .iter()
        .map(|actor| String::from_utf8(actor.name.to_vec()).unwrap())
        .chain(scanned.series.keys().cloned())
        .collect();
    for tag in &tags {
        let runs = scanned.series.get(tag).cloned().unwrap_or_default();
        assert_eq!(
            published_runs(&mut snapshot, &directory, tag),
            runs,
            "{label}: {tag}'s stockpile runs"
        );
        let actor = directory.actor(tag.as_bytes());
        let key = |measure, dimension| actor.and_then(|actor| directory.key(actor, measure, dimension));
        assert_eq!(
            raw(&mut snapshot, key(TOTAL, None)),
            expected(&runs, |run| run.total.map(i128::from)),
            "{label}: {tag}'s totals"
        );
        for &ordinal in &scanned.types {
            assert_eq!(
                raw(&mut snapshot, key(PARTS, Some(u64::from(ordinal)))),
                expected(&runs, |run| {
                    run.counts
                        .iter()
                        .find(|count| count.ordinal == ordinal)
                        .map(|count| i128::from(count.value))
                }),
                "{label}: {tag}'s type {ordinal}"
            );
        }
    }
    let mut ranks: BTreeMap<usize, Vec<Raw>> = BTreeMap::new();
    dashboard_base::ranking::rank_positions(
        &scanned.series,
        |run| (run.first_hour, run.last_hour),
        |run| run.total.filter(|total| *total != 0).map(i128::from),
        &AtomicBool::new(false),
        |position, tag, from, to| {
            let actor = i128::from(directory.actor(tag.as_bytes()).unwrap());
            let held = ranks.entry(position).or_default();
            match held.last_mut() {
                Some(last) if last.1 + 1 == from && last.3 == actor => last.1 = to,
                _ => held.push((from, to, false, actor)),
            }
            Ok(())
        },
    )
    .unwrap();
    let positions: Vec<u64> = directory
        .dimensions(FAMILY, POSITIONS)
        .iter()
        .map(|&(_, key)| key)
        .collect();
    assert_eq!(positions.len(), ranks.len(), "{label}: ranks");
    for (position, key) in positions.into_iter().enumerate() {
        assert_eq!(
            raw(&mut snapshot, Some(key)),
            ranks[&position],
            "{label}: rank {position}"
        );
    }
    let accepted = snapshot.publication().accepted;
    let reads = super::query::Reads::of(&directory)
        .within(&mut snapshot, accepted.first, accepted.last)
        .unwrap();
    assert_eq!(
        reads.iter().map(|&(hour, _)| hour).collect::<Vec<_>>(),
        scanned.read_hours,
        "{label}: complete reads"
    );
    assert_eq!(
        reads
            .iter()
            .filter(|&&(_, empty)| empty)
            .map(|&(hour, _)| hour)
            .collect::<Vec<_>>(),
        scanned.empty_read_hours,
        "{label}: empty reads"
    );
    let published = super::query::scanned(&directory);
    assert_eq!(published.types, scanned.types, "{label}: ordinals read");
    assert_eq!(published.tokens, scanned.tokens, "{label}: spellings");
    assert_eq!(published.refused, scanned.refused, "{label}: refused rows");
    assert_eq!(published.unreadable, scanned.unreadable, "{label}: unresolved");
    assert_eq!(
        published.unknown_hours, scanned.unknown_hours,
        "{label}: unknown country-hours"
    );
    assert_eq!(
        super::query::feed(&mut snapshot, &directory).unwrap(),
        feed,
        "{label}: what the event feed reads"
    );
}

/// Every placement of segment starts, cuts and a resume publishes what the row census reads,
/// and the same publication whatever the placement.
#[test]
fn the_publication_holds_what_the_row_census_reads_across_segments_cuts_and_a_resume() {
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
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(&hours, starts);
        let built = build(projected, cuts, resume, &budget);
        check(&built, &frames, &label);
    }
}

/// The scenario's stockpiles as its reads give them: increases dated where a total grows within
/// a segment, holes and repeated types unknown, the zero stockpile never ranked.
#[test]
fn the_scenario_reads_as_the_top_bar_shows_it() {
    let (_, frames) = projected(&scenario(), &[]);
    let mut census = row::Census::default();
    for frame in &frames {
        census.observe(frame, FormatVersion::CURRENT);
    }
    let (scanned, feed) = census.finish();
    assert_eq!(scanned.empty_read_hours, [100, 101, 102]);
    assert_eq!(
        feed.increases
            .iter()
            .map(|increase| (increase.tag.as_str(), increase.hour, increase.amount))
            .collect::<Vec<_>>(),
        [
            ("AAA", 103, 1),
            ("CCC", 103, 3),
            ("AAA", 105, 1),
            ("AAA", 108, 1),
            ("AAA", 120, 3)
        ]
    );
    assert_eq!(scanned.refused, BTreeMap::from([(2, 7)]));
    assert_eq!(
        feed.refusals,
        BTreeMap::from([(
            REFUSAL.to_owned(),
            vec![HourSpan {
                first_hour: 125,
                last_hour: 126
            }]
        )])
    );
    // Two rows of one type leave A's stockpile unknown, never zero.
    assert!(
        !scanned.series["AAA"]
            .iter()
            .any(|run| run.first_hour <= 117 && 115 <= run.last_hour)
    );
    assert!(
        scanned.series["CCC"]
            .iter()
            .any(|run| run.first_hour == 112 && run.total.is_none())
    );
    assert!(scanned.unknown_hours > 0 && scanned.unreadable > 0);
}

/// A publication's replies are the row reply's: ranked and listed countries, a drill at its
/// origin hour, and the states at hours.
#[test]
fn replies_from_the_publication_are_the_row_replies() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    let (projected, frames) = projected(&hours, &[110, 140]);
    let built = build(projected, &[129, 160], Some(129), &budget);
    let early: Vec<_> = frames.iter().filter(|frame| frame.hour <= 120).cloned().collect();
    let (first, second): (Vec<_>, Vec<_>) = frames.into_iter().partition(|frame| frame.hour < 130);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, first)]).unwrap();
    let replay = replay
        .append(&[continuation_records(&replay, 1, second)])
        .unwrap();
    let cancelled = AtomicBool::new(false);
    let history = scan_histories(&replay, 1, false, &[], &cancelled).unwrap();
    let (source, root) = file_source(replay);
    let context = LoadContext { content: None };
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let accepted = Coverage {
        schema: dashboard_base::prepared::coverage::SCHEMA,
        first_hour: 100,
        last_hour: 160,
        complete: true,
    };
    for (tags, top, origin) in [
        (vec![], Some(2), None),
        (vec!["AAA".to_owned()], None, None),
        (vec!["BBB".to_owned(), "CCC".to_owned()], None, None),
        (vec!["CCC".to_owned()], None, Some(150)),
        (vec!["CCX".to_owned()], None, Some(158)),
    ] {
        let params = NukesParams {
            branch_id: "1".into(),
            tags: tags.clone(),
            top,
            origin_hour: origin,
        };
        let label = format!("{params:?}");
        let mut reply = super::query::series(
            &source,
            "series-summary",
            &serde_json::json!({
                "branchId": "1", "tags": tags,
                "family": {"kind": "country-nukes", "top": top, "originHour": origin},
                "from": 100, "to": 160, "width": 100,
            }),
            &context,
            &mut snapshot,
            &directory,
            &history,
            Some(accepted.clone()),
            &cancelled,
        )
        .unwrap();
        for field in ["summaries", "coverage"] {
            reply.as_object_mut().unwrap().remove(field);
        }
        let expected = row::load(&source, &params, &context, &cancelled).unwrap();
        assert_eq!(reply, serde_json::to_value(expected).unwrap(), "{label}");
    }
    // A run the window clips starts no segment at the window's first hour; one starting there does.
    for (from, starts) in [(132, true), (135, false), (146, true), (150, false)] {
        let reply = super::query::series(
            &source,
            "series-summary",
            &serde_json::json!({
                "branchId": "1", "tags": ["AAA"],
                "family": {"kind": "country-nukes", "top": null, "originHour": null},
                "from": from, "to": 160, "width": 100,
            }),
            &context,
            &mut snapshot,
            &directory,
            &history,
            Some(accepted.clone()),
            &cancelled,
        )
        .unwrap();
        let run = &reply["series"][0]["runs"][0];
        assert_eq!(
            (run["firstHour"].as_i64(), run["startsSegment"].as_bool()),
            (Some(i64::from(from)), Some(starts)),
            "from {from}"
        );
    }
    let reads: Vec<_> = [100, 104, 116, 125, 133, 150, 160]
        .into_iter()
        .map(|hour| serde_json::json!({"tag": "AAA", "hour": hour}))
        .collect();
    let state = super::query::series(
        &source,
        "series-state",
        &serde_json::json!({
            "branchId": "1",
            "family": {"kind": "country-nukes", "top": null, "originHour": null},
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
    let series = state["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|series| series["tag"] == "AAA")
        .unwrap();
    let totals: Vec<_> = series["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| {
            (
                run["firstHour"].as_i64().unwrap(),
                run["lastHour"].as_i64().unwrap(),
                run["total"].as_i64(),
            )
        })
        .collect();
    // Unknown at 116 (two rows of one type) and 125 (the table unavailable): no value, never zero.
    assert_eq!(
        totals,
        [
            (100, 100, Some(0)),
            (104, 104, Some(1)),
            (133, 133, Some(6)),
            (150, 150, Some(6)),
            (160, 160, Some(6))
        ]
    );
    assert_eq!(state["readHours"], serde_json::json!([100, 104, 133, 150, 160]));
    // Identities through 120 while the stockpiles go on (CCX ranks second from 155): nothing past
    // their shared coverage is read, and a reply waits for any.
    let early = Replay::from_records(CAMPAIGN.into(), &[records(1, early)]).unwrap();
    let trailing = scan_histories(&early, 1, false, &[], &cancelled).unwrap();
    let shared = Coverage {
        last_hour: 120,
        complete: false,
        ..accepted.clone()
    };
    let ask = |snapshot: &mut Snapshot<'_, Memory>, product: &str, params: serde_json::Value, accepted| {
        super::query::series(
            &source, product, &params, &context, snapshot, &directory, &trailing, accepted,
            &cancelled,
        )
        .unwrap()
    };
    let ranked = serde_json::json!({
        "branchId": "1", "tags": [],
        "family": {"kind": "country-nukes", "top": 2, "originHour": null},
        "from": 100, "to": 160, "width": 100,
    });
    let reply = ask(&mut snapshot, "series-summary", ranked.clone(), Some(shared.clone()));
    // The summaries read only the hours the coverage holds.
    for line in reply["summaries"].as_array().unwrap() {
        assert!(
            line["summary"]["buckets"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|bucket| bucket["lastHour"].as_i64())
                .all(|hour| hour <= 120),
            "{line}"
        );
    }
    let series = reply["series"].as_array().unwrap();
    assert_eq!(
        series
            .iter()
            .map(|series| series["tag"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["AAA", "CCC"]
    );
    for series in series {
        for field in ["runs", "membership"] {
            assert!(
                series[field]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|run| run["lastHour"].as_i64().unwrap() <= 120),
                "{field}: {series}"
            );
        }
    }
    let end = ask(
        &mut snapshot,
        "series-next-change",
        serde_json::json!({
            "branchId": "1", "tags": [],
            "family": {"kind": "country-nukes", "top": 2, "originHour": null},
            "key": "End", "from": null, "firstHour": 100, "lastHour": 160,
        }),
        Some(shared.clone()),
    );
    assert!(end["hour"].as_f64().is_some_and(|hour| hour <= 120.0), "{end}");
    // A dense summary's compositions and the reads that supplied them stop there too.
    let mut dense = ranked.clone();
    dense["width"] = serde_json::json!(1);
    let reply = ask(&mut snapshot, "series-summary", dense, Some(shared.clone()));
    for field in ["readHours", "emptyReadHours"] {
        assert!(
            reply[field]
                .as_array()
                .unwrap()
                .iter()
                .all(|hour| hour.as_i64().unwrap() <= 120),
            "{field}: {}",
            reply[field]
        );
    }
    assert!(reply["series"].as_array().unwrap().iter().all(|series| {
        series["runs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|run| run["lastHour"].as_i64().unwrap() <= 120)
    }));
    // A window wholly past the coverage reads no value.
    let beyond = ask(
        &mut snapshot,
        "series-summary",
        serde_json::json!({
            "branchId": "1", "tags": ["AAA"],
            "family": {"kind": "country-nukes", "top": null, "originHour": null},
            "from": 125, "to": 160, "width": 100,
        }),
        Some(shared.clone()),
    );
    for line in beyond["summaries"].as_array().unwrap() {
        assert_eq!(line["summary"]["buckets"], serde_json::json!([]), "{line}");
    }
    assert_eq!(beyond["readHours"], serde_json::json!([]));
    // The publication pinned through the coverage, as the query pins it: its engine reads
    // nothing past 120, and a step forward from past the coverage lands on its last hour.
    let (cut, _) = self::projected(&hours, &[110, 140]);
    let cut = build(cut, &[120, 160], None, &budget);
    let mut pinned = Snapshot::open(cut.pages.clone(), &cache, cut.root, identity())
        .unwrap()
        .through(120)
        .unwrap()
        .unwrap();
    assert_eq!(pinned.publication().accepted.last, 120);
    let pinned_directory = Directory::read(&mut pinned, SCHEMA).unwrap();
    let step = super::query::series(
        &source,
        "series-next-change",
        &serde_json::json!({
            "branchId": "1", "tags": ["AAA"],
            "family": {"kind": "country-nukes", "top": null, "originHour": null},
            "key": "ArrowRight", "from": 150, "firstHour": 100, "lastHour": 160,
        }),
        &context,
        &mut pinned,
        &pinned_directory,
        &trailing,
        Some(shared.clone()),
        &cancelled,
    )
    .unwrap();
    assert_eq!(step["hour"], serde_json::json!(120.0));
    assert!(ask(&mut snapshot, "series-summary", ranked, None).is_null());
    drop(source);
    std::fs::remove_dir_all(root).unwrap();
}

/// A page halved to one country answers it from its own reads alone: the dates the countries it
/// gave away were read on never reach its reply, so it is not refused for them.
#[test]
fn a_halved_page_keeps_no_reads_of_the_countries_it_gives_away() {
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
    let (source, root) = file_source(replay);
    let context = LoadContext { content: None };
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let accepted = Coverage {
        schema: dashboard_base::prepared::coverage::SCHEMA,
        first_hour: 100,
        last_hour: 160,
        complete: true,
    };
    // Dense: each country's compositions are read on its own dates.
    let mut ask = |tags: serde_json::Value, extra: serde_json::Value| {
        let mut params = serde_json::json!({
            "branchId": "1", "tags": tags,
            "family": {"kind": "country-nukes", "top": null, "originHour": null},
            "from": 100, "to": 160, "width": 1,
        });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        super::query::series(
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
        .unwrap()
    };
    let whole = ask(serde_json::json!(["BBB", "CCC"]), serde_json::json!({}));
    let lines = whole["summaries"].as_array().unwrap();
    let tags: Vec<_> = lines.iter().map(|line| line["tag"].clone()).collect();
    assert_eq!(tags, ["BBB", "CCC", "CCX"]);
    let alone = ask(serde_json::json!(["BBB"]), serde_json::json!({}));
    assert_ne!(
        whole["readHours"], alone["readHours"],
        "C's compositions were read on dates B's were not"
    );
    let mut expected = alone.clone();
    expected["cursor"] = serde_json::json!("1");
    let bytes = serde_json::to_vec(&expected).unwrap().len();
    let held: usize = lines
        .iter()
        .map(|line| serde_json::to_vec(line).unwrap().len() + 1)
        .sum();
    assert!(held <= bytes, "every line is admitted, so the page is halved");
    // B's reply alone fits exactly; one holding C's dates would be refused.
    let halved = ask(
        serde_json::json!(["BBB", "CCC"]),
        serde_json::json!({ "pageBudget": bytes }),
    );
    assert_eq!(halved, expected);
    drop(source);
    std::fs::remove_dir_all(root).unwrap();
}

/// Of 1,500 countries, an hour where one country's stockpile changes hands the emit that
/// country's slot alone; an hour where every country's rows change but no count shown does, and
/// an hour where nothing changes, hand nothing but their read.
#[test]
fn an_hour_hands_only_the_stockpiles_that_changed() {
    let countries: Vec<(u32, u32, &'static str)> = (1..=1_500)
        .map(|id| {
            let tag: &'static str = Box::leak(format!("C{id:04}").into_boxed_str());
            (id, id, tag)
        })
        .collect();
    let links: Vec<(u32, u32)> = (1..=1_500).map(|id| (id, id)).collect();
    let hours: Vec<Tables> = (0..6)
        .map(|hour| Tables {
            hour,
            countries: Some(countries.clone()),
            links: links.clone(),
            nukes: Some(
                (1..=1_500)
                    .map(|id| {
                        let whole = if id == 7 && hour >= 3 { UNIT } else { 0 };
                        (id, 0, whole + if hour >= 4 { UNIT / 3 } else { 0 })
                    })
                    .collect(),
            ),
            tokens: true,
        })
        .collect();
    let (projected, _) = projected(&hours, &[]);
    let handed: Vec<Option<(bool, usize)>> = projected
        .iter()
        .map(|hour| {
            hour.facts
                .as_ref()
                .and_then(|facts| facts.stocks.as_ref())
                .map(|stocks| (stocks.whole, stocks.slots.len()))
        })
        .collect();
    assert_eq!(
        handed,
        [Some((true, 1_500)), None, None, Some((false, 1)), None, None]
    );
    assert!(
        [1, 2, 4, 5]
            .iter()
            .all(|&at| projected[at].facts.is_none() && projected[at].read)
    );
}

/// 1,500 countries whose totals tie, rise, fall to zero, go unknown and jump from the bottom to
/// the top in the same hours rank as the row census ranks them, across a segment start, a cut
/// and a resume.
#[test]
fn a_crowded_ranking_moves_as_the_row_ranking_does() {
    let countries: Vec<(u32, u32, &'static str)> = (1..=1_500)
        .map(|id| {
            let tag: &'static str = Box::leak(format!("C{id:04}").into_boxed_str());
            (id, id, tag)
        })
        .collect();
    let links: Vec<(u32, u32)> = (1..=1_500).map(|id| (id, id)).collect();
    let hours: Vec<Tables> = (0..8)
        .map(|hour| Tables {
            hour,
            countries: Some(countries.clone()),
            links: links.clone(),
            nukes: Some(
                (1..=1_500u32)
                    .map(|id| {
                        let at = hour as u32;
                        let amount = if (id, at) == (1_500, 5) {
                            1_000
                        } else {
                            i64::from((id * 7 + at * (id % 5)) % 40)
                        };
                        // A type past a hole in the slot's array leaves its total unknown.
                        (id, u32::from(id % 97 == at), amount * UNIT)
                    })
                    .collect(),
            ),
            tokens: true,
        })
        .collect();
    let budget = Budget::new(256 << 20);
    let (projected, frames) = projected(&hours, &[5]);
    let built = build(projected, &[3, 7], Some(3), &budget);
    check(&built, &frames, "a crowded ranking");
}
