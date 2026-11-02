//! The family's columnar preparation held to its row scan ([`super::row`]) on the same hours,
//! written as the native recorder writes them: every subject's score, bounds, ratio, place and
//! band runs, the recorded and partly known hours, the levels, overlords and shared subjects,
//! and the replies at each hour, across segment starts, a gap, cuts and a resume.

use super::{
    AutonomyParams, AutonomyRun,
    emit::{BAND, Emit, LOWER, RATIO, RELATIVE, SCHEMA, SCORE, UPPER},
    facts::{self, Projector},
    row,
};
use dashboard_base::{
    prepared::engine,
    products::LoadContext,
    source::{Source, SourceKey},
    test_fixtures::{CAMPAIGN, records, source},
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
    rows::{AutonomyRow, CountryLinkRow, CountryRow, RowCodec, SubjectRow, TableId, TableRows},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::json;
use std::sync::{Arc, atomic::AtomicBool};

/// One hour as the recorder writes it: the countries (id, slot, tag), none where the table is
/// unavailable, the links (raw id, slot), the Autonomy rows and the Subjects rows, none where the
/// table is unavailable.
#[derive(Clone)]
struct Tables {
    hour: i32,
    countries: Option<Vec<(u32, u32, String)>>,
    links: Vec<(u32, u32)>,
    levels: Vec<AutonomyRow>,
    relations: Option<Vec<SubjectRow>>,
}

fn level(
    subject: u32,
    state: &str,
    (score, lower, upper): (i64, i64, i64),
    lowest: bool,
    highest: bool,
) -> AutonomyRow {
    AutonomyRow {
        subject,
        score,
        lower_score: lower,
        upper_score: upper,
        last_modified_bits: 0,
        state: state.as_bytes().to_vec(),
        previous: (!lowest).then(|| b"below".to_vec()),
        next: (!highest).then(|| b"above".to_vec()),
    }
}

fn relation(overlord: u32, subject: u32, end_hour: i32, cancel: u8, ratio: i64) -> SubjectRow {
    SubjectRow {
        overlord,
        subject,
        start_hour: 0,
        end_hour,
        cancel,
        autonomy_ratio: ratio,
        state: b"autonomy_puppet".to_vec(),
    }
}

/// Hours 100 to 140, without 130 and 131. GER (also raw id 11) holds AUS, whose score rises
/// every third hour, whose level moves up at 110 and whose ratio changes at 115; another country
/// id holds AUS's slot from 135. GER's alias holds HUN until its relation's recorded end at 120,
/// and ITA holds HUN too from 108 to 111 (HUN then has two overlords); HUN's bounds meet at 112
/// and 113, and an overlord no link names holds it at 128 and 129. ITA is GER's subject from 116
/// to 127 at a level of its own; a subject no link names is recorded from 122 to 124; a
/// cancelled relation never holds. Subjects is unavailable at 125 and 126, and Countries at 138.
fn scenario() -> Vec<Tables> {
    (100..=140)
        .filter(|hour| !(130..=131).contains(hour))
        .map(|hour| {
            let aus = if hour >= 135 { 30 } else { 3 };
            let countries = (hour != 138).then(|| {
                vec![
                    (1, 1, "GER".to_owned()),
                    (2, 2, "ITA".to_owned()),
                    (aus, 3, "AUS".to_owned()),
                    (4, 4, "HUN".to_owned()),
                ]
            });
            let mut links = vec![(1, 1), (2, 2), (aus, 3), (4, 4), (11, 1)];
            links.sort_unstable();
            let score = i64::from(hour / 3) * 1_000;
            let mut levels = vec![if hour < 110 {
                level(aus, "autonomy_puppet", (score, 0, 50_000), true, false)
            } else {
                level(aus, "autonomy_dominion", (score, 50_000, 90_000), false, false)
            }];
            let upper = if (112..=113).contains(&hour) { 0 } else { 50_000 };
            levels.push(level(4, "autonomy_puppet", (5_000, 0, upper), true, false));
            if (116..=127).contains(&hour) {
                levels.push(level(2, "autonomy_colony", (10, 0, 100), true, true));
            }
            if (122..=124).contains(&hour) {
                levels.push(level(99, "autonomy_puppet", (1, 0, 2), true, false));
            }
            levels.sort_unstable_by_key(|row| row.subject);
            let ratio = if hour >= 115 { 25_000 } else { 20_000 };
            let mut relations = vec![
                relation(1, aus, 0, 0, ratio),
                relation(11, 4, 120, 0, 30_000),
                relation(2, aus, 0, 1, 0),
            ];
            if (108..=111).contains(&hour) {
                relations.push(relation(2, 4, 0, 0, 10_000));
            }
            if hour >= 116 {
                relations.push(relation(1, 2, 0, 0, 5_000));
            }
            if (128..=129).contains(&hour) {
                relations.push(relation(77, 4, 0, 0, 0));
            }
            relations.sort_unstable_by_key(|row| (row.overlord, row.subject));
            Tables {
                hour,
                countries,
                links,
                levels,
                relations: (!(125..=126).contains(&hour)).then_some(relations),
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

const REFUSAL: &str = "synthetic refusal";

/// The hour as the row recording holds it.
fn frame(tables: &Tables) -> DecodedFrame {
    let mut frame = source(1, tables.hour, Some(1));
    let mut put = |table: TableId, view: TableView| {
        *frame.tables[table.index()] = view;
    };
    let refused = || TableView::Unavailable {
        reason: REFUSAL.into(),
    };
    put(
        TableId::Countries,
        match &tables.countries {
            Some(countries) => TableView::Complete(TableRows::Countries(country_rows(countries))),
            None => refused(),
        },
    );
    put(
        TableId::CountryLinks,
        TableView::Complete(TableRows::CountryLinks(link_rows(&tables.links))),
    );
    put(
        TableId::Autonomy,
        TableView::Complete(TableRows::Autonomy(tables.levels.clone())),
    );
    put(
        TableId::Subjects,
        match &tables.relations {
            Some(relations) => TableView::Complete(TableRows::Subjects(relations.clone())),
            None => refused(),
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
        write(walk, TableId::Autonomy, Some(&tables.levels[..]));
        write(walk, TableId::Subjects, tables.relations.as_deref());
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

/// What a band holds besides its hours.
fn band(run: &AutonomyRun) -> (Option<&str>, u32, bool, bool, bool) {
    (
        run.overlord.as_deref(),
        run.level,
        run.lowest,
        run.highest,
        run.ratio_raw.is_some(),
    )
}

/// The runs a key holds of `value` of each of a subject's runs: equal values going on from the
/// hour before without a break are one; a run starts anew after an hour of none, where its
/// subject's runs start anew, and where `splits` says.
fn expected(
    runs: &[AutonomyRun],
    value: impl Fn(&AutonomyRun) -> Option<i128>,
    splits: impl Fn(&AutonomyRun, &AutonomyRun) -> bool,
) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    let mut previous: Option<&AutonomyRun> = None;
    for run in runs {
        let goes_on = previous.is_some_and(|held| {
            held.last_hour + 1 == run.first_hour && !run.starts_segment && !splits(held, run)
        });
        if let Some(value) = value(run) {
            match out.last_mut() {
                Some(last) if goes_on && last.1 + 1 == run.first_hour && last.3 == value => {
                    last.1 = run.last_hour;
                }
                last => {
                    let joined = goes_on && last.is_some_and(|last| last.1 + 1 == run.first_hour);
                    out.push((run.first_hour, run.last_hour, !joined, value));
                }
            }
        }
        previous = Some(run);
    }
    out
}

/// Each band's runs: its ordinal, one more for each band before it.
fn expected_bands(runs: &[AutonomyRun]) -> Vec<Raw> {
    let mut out: Vec<Raw> = Vec::new();
    let mut previous: Option<&AutonomyRun> = None;
    for run in runs {
        let goes_on = previous.is_some_and(|held| {
            held.last_hour + 1 == run.first_hour && !run.starts_segment && band(held) == band(run)
        });
        match out.last_mut() {
            Some(last) if goes_on => last.1 = run.last_hour,
            _ => {
                let ordinal = out.len() as i128 + 1;
                out.push((run.first_hour, run.last_hour, run.starts_segment, ordinal));
            }
        }
        previous = Some(run);
    }
    out
}

fn number(text: &str) -> i128 {
    text.parse().unwrap()
}

fn spans(spans: &[dashboard_base::products::country_groups::HourSpan]) -> Vec<(i32, i32)> {
    spans
        .iter()
        .map(|span| (span.first_hour, span.last_hour))
        .collect()
}

/// The series-state reply of every subject at `hours`.
fn state<P: dashboard_series::PageRead>(
    source: &Source,
    snapshot: &mut Snapshot<'_, P>,
    directory: &Directory,
    hours: &[i32],
) -> serde_json::Value {
    let reads: Vec<_> = hours.iter().map(|hour| json!({ "hour": hour })).collect();
    super::query::answer(
        source,
        "series-state",
        &json!({
            "branchId": "1", "family": { "kind": "autonomy" }, "overlords": [],
            "subjects": [], "reads": reads,
        }),
        snapshot,
        directory,
        &LoadContext { content: None },
    )
    .unwrap()
}

/// Holds the publication `built` to the row scan of `frames`, and its replies at each hour to
/// the row replies.
fn check(built: &Built, frames: &[DecodedFrame], label: &str) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let mut observer = row::Observer::default();
    for frame in frames {
        observer.observe(frame).unwrap();
    }
    let scanned = observer.scanned;
    let accepted = snapshot.publication().accepted;
    let published = super::query::scanned(&mut snapshot, &directory, accepted).unwrap();
    assert_eq!(
        published
            .subjects
            .iter()
            .map(|(tag, _)| tag.as_str())
            .collect::<Vec<_>>(),
        scanned
            .subjects
            .iter()
            .map(|(tag, _)| tag.as_str())
            .collect::<Vec<_>>(),
        "{label}: subjects in the order first seen"
    );
    assert_eq!(published.levels, scanned.levels, "{label}: levels");
    assert_eq!(
        published.lowest_lower, scanned.lowest_lower,
        "{label}: lowest move-down scores"
    );
    assert_eq!(
        spans(&published.recorded),
        spans(&scanned.recorded),
        "{label}: recorded hours"
    );
    assert_eq!(
        spans(&published.unidentified),
        spans(&scanned.unidentified),
        "{label}: partly known hours"
    );
    assert_eq!(
        published.shared_subjects, scanned.shared_subjects,
        "{label}: subject-hours with two overlords"
    );
    assert_eq!(
        published.overlord_subjects, scanned.overlord_subjects,
        "{label}: each overlord's subjects"
    );
    for (actor, (tag, runs)) in scanned.subjects.iter().enumerate() {
        let key = |measure| directory.key(actor as u64, measure, None);
        let never = |_: &AutonomyRun, _: &AutonomyRun| false;
        let values = [
            (SCORE, "score"),
            (LOWER, "lower"),
            (UPPER, "upper"),
            (RATIO, "ratio"),
            (RELATIVE, "relative"),
        ];
        for (measure, name) in values {
            let value = |run: &AutonomyRun| -> Option<i128> {
                let (score, lower, upper) = (
                    number(&run.score_raw),
                    number(&run.lower_raw),
                    number(&run.upper_raw),
                );
                match measure {
                    SCORE => Some(score),
                    LOWER => Some(lower),
                    UPPER => Some(upper),
                    RATIO => run.ratio_raw.as_deref().map(number),
                    _ => (upper != lower).then(|| (score - lower) * 1_000_000_000 / (upper - lower)),
                }
            };
            // A score starts a run with each band.
            let wanted = if measure == SCORE {
                expected(runs, value, |held, run| band(held) != band(run))
            } else {
                expected(runs, value, never)
            };
            assert_eq!(
                raw(&mut snapshot, key(measure)),
                wanted,
                "{label}: {tag}'s {name}"
            );
        }
        assert_eq!(
            raw(&mut snapshot, key(BAND)),
            expected_bands(runs),
            "{label}: {tag}'s bands"
        );
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let source = Source::from_replay(
        SourceKey {
            source_id: format!("autonomy-columnar-{label}"),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let context = LoadContext { content: None };
    let oracle = row::load(
        &source,
        &AutonomyParams {
            branch_id: "1".into(),
            overlords: Vec::new(),
            subjects: Vec::new(),
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    let hours: Vec<i32> = frames.iter().map(|frame| frame.hour).collect();
    let whole = state(&source, &mut snapshot, &directory, &hours);
    assert_eq!(
        whole["subjects"],
        serde_json::to_value(&oracle.subjects).unwrap(),
        "{label}: every hour's values"
    );
    for key in ["levels", "recorded", "unidentified", "warnings"] {
        assert_eq!(
            whole[key],
            serde_json::to_value(&oracle).unwrap()[key],
            "{label}: the reply's {key}"
        );
    }
    for &hour in &hours {
        let reply = state(&source, &mut snapshot, &directory, &[hour]);
        let mut wanted = oracle.subjects.clone();
        for subject in &mut wanted {
            subject
                .runs
                .retain(|run| run.first_hour <= hour && hour <= run.last_hour);
        }
        assert_eq!(
            reply["subjects"],
            serde_json::to_value(&wanted).unwrap(),
            "{label}: the values at {hour}"
        );
    }
}

/// Every placement of segment starts, cuts and a resume publishes what the row scan reads, and
/// answers each hour as the row reply does.
#[test]
fn the_publication_holds_what_the_row_scan_reads_across_segments_cuts_and_a_resume() {
    let budget = Budget::new(256 << 20);
    let hours = scenario();
    for (starts, cuts, resume) in [
        (&[][..], &[140][..], None),
        (&[110, 121, 135][..], &[109, 129, 137, 140][..], Some(129)),
        (&[103, 113, 125, 127, 139][..], &[112, 126, 134, 140][..], Some(126)),
        (
            &(101..=140).collect::<Vec<_>>()[..],
            &(100..=140)
                .filter(|hour| !(130..=131).contains(hour))
                .collect::<Vec<_>>()[..],
            Some(120),
        ),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let (projected, frames) = projected(&hours, starts);
        let built = build(projected, cuts, resume, &budget);
        check(&built, &frames, &label);
    }
}

/// Of 1,500 countries and 300 subjects, an hour hands the emit only the subject whose score
/// changed, and nothing where nothing changed.
#[test]
fn an_hour_hands_the_emit_only_the_subjects_that_changed() {
    const COUNTRIES: u32 = 1_500;
    const SUBJECTS: u32 = 300;
    let tables = |hour: i32, moved: bool| {
        let countries = (1..=COUNTRIES)
            .map(|id| (id, id, format!("C{id:04}")))
            .collect();
        let links = (1..=COUNTRIES).map(|id| (id, id)).collect();
        let levels = (2..=SUBJECTS + 1)
            .map(|subject| {
                let score = if moved && subject == 7 { 2_000 } else { 1_000 };
                level(subject, "autonomy_puppet", (score, 0, 50_000), true, false)
            })
            .collect();
        let relations = (2..=SUBJECTS + 1)
            .map(|subject| relation(1, subject, 0, 0, 20_000))
            .collect();
        Tables {
            hour,
            countries: Some(countries),
            links,
            levels,
            relations: Some(relations),
        }
    };
    let (projected, _) = projected(
        &[tables(100, false), tables(101, true), tables(102, true)],
        &[],
    );
    let first = projected[0].facts.as_ref().expect("the first hour is read whole");
    assert!(first.whole);
    assert_eq!(first.changes.len(), SUBJECTS as usize);
    let moved = projected[1].facts.as_ref().expect("a score changed");
    assert!(!moved.whole);
    assert_eq!(
        moved
            .changes
            .iter()
            .map(|change| change.tag.as_ref())
            .collect::<Vec<_>>(),
        ["C0007"]
    );
    assert_eq!(moved.changes[0].sample.as_ref().unwrap().score, 2_000);
    assert!(projected[2].facts.is_none(), "an hour changing nothing carries nothing");
}

/// Rows of two tags interleaved by raw subject id count their levels in id order across tags, as
/// the row scan observes them: ids 1 and 3 hold tag AAA, id 2 tag BBB.
#[test]
fn interleaved_rows_count_their_levels_in_id_order_across_tags() {
    let sample = |slot: u32, level: &str, lower: i64| {
        Arc::new(facts::Sample {
            holder: (slot, slot),
            overlord: None,
            level: Arc::from(level.as_bytes()),
            score: 10,
            lower,
            upper: 100,
            lowest: true,
            highest: false,
            ratio: None,
        })
    };
    let change = |tag: &str, rows: Vec<(u32, Arc<facts::Sample>)>| facts::Change {
        tag: Arc::from(tag),
        order: rows[0].0,
        sample: rows.last().map(|(_, sample)| Arc::clone(sample)),
        rows,
    };
    let hour = facts::Hour {
        branch: 1,
        hour: 100,
        facts: Some(Box::new(facts::Facts {
            recorded: Some(true),
            whole: true,
            changes: vec![
                change(
                    "AAA",
                    vec![(1, sample(1, "first", 0)), (3, sample(1, "third", 0))],
                ),
                change("BBB", vec![(2, sample(2, "second", 0))]),
            ],
            unidentified: Some(false),
            shared: Some(0),
        })),
    };
    let budget = Budget::new(64 << 20);
    let built = build(vec![hour], &[100], None, &budget);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let accepted = snapshot.publication().accepted;
    let published = super::query::scanned(&mut snapshot, &directory, accepted).unwrap();
    assert_eq!(published.levels, ["first", "second", "third"]);
    assert_eq!(
        published
            .subjects
            .iter()
            .map(|(tag, _)| tag.as_str())
            .collect::<Vec<_>>(),
        ["AAA", "BBB"]
    );
}
