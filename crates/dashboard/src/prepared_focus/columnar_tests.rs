//! The family's columnar preparation held to the row preparation ([`super::row`]) on the same
//! hours, written as the native recorder writes them: every country's focus at every hour with
//! every page of its order, its dated completions, the tables' refusals at every hour and the
//! event feed's completions and counts, across segment starts, a gap, cuts and a resume.

use super::{
    emit::{Emit, SCHEMA},
    facts::{self, Projector},
    *,
};
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame, scale_tag};
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
    rows::{CompletedFocusRow, FocusRow, RowCodec, TableRows},
    scope::CaptureMode,
};
use std::sync::Arc;

const REFUSAL: &str = "Synthetic focus read unavailable";

/// Countries A to E (`scale_tag(0..5)`, slots 0 to 4) whose balances move every hour, with
/// their focus at `hour`:
/// - A completes `a0`, `a1`, ... every ten hours from 100; its progress rises every five hours
///   and holds while paused from 160 to 175 (longer than a block); at 145 and 146 its count
///   names one completion more than its rows hold; from 185 `a1` is uncompleted, a removal a
///   reset can hide behind, so its dates are unknown from then on; at 165 its tree is
///   replaced with its list unchanged, a certain reset.
/// - B completes `b0`, `b1`, ...; its list resets to `b_reset` at 140 (a shrink and a prefix
///   break) and grows again; from 185 its active focus alternates between two byte strings
///   that render alike.
/// - C is country 2 until 129; from 130 country 4 holds the tag (an identity replacement) and
///   country 2 is renamed; country 2's link is missing from 112 to 114.
/// - D has no focus row from 120 to 125, its tag is empty at 170 and 171, and from 180 to 183
///   it holds B's tag too (the higher country id is B's).
/// - Focus is not read at 150 and 151; hours 136 and 137 are not recorded.
fn focus_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = scale_frame(branch, hour, 5);
    let tag = |index: u32| -> Vec<u8> {
        match (index, hour >= 130) {
            (2, true) => b"OLD".to_vec(),
            (4, true) => scale_tag(2),
            (4, false) => b"NEW".to_vec(),
            (3, _) if (170..=171).contains(&hour) => Vec::new(),
            (3, _) if (180..=183).contains(&hour) => scale_tag(1),
            _ => scale_tag(index),
        }
    };
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for (index, row) in rows.iter_mut().enumerate() {
            row.tag = tag(index as u32);
            row.political_power_raw = Some(i64::from(hour) * 1000);
            row.command_power_raw = i64::from(hour) * 7;
        }
    }
    if (112..=114).contains(&hour)
        && let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.retain(|link| link.id != 100_002);
    }
    let step = i64::from(hour - 100);
    let mut focus = Vec::new();
    let mut completed = Vec::new();
    for index in 0..5_u32 {
        if index == 3 && (120..=125).contains(&hour) {
            continue;
        }
        let country = 100_000 + index;
        let (tokens, current, progress, paused): (Vec<String>, Option<Vec<u8>>, i64, bool) =
            match index {
                0 => {
                    let paused = (160..=175).contains(&hour);
                    let done = (step / 10) as usize;
                    let held = if paused { 155 } else { hour };
                    (
                        (0..done)
                            .filter(|&n| hour < 185 || n != 1)
                            .map(|n| format!("a{n}"))
                            .collect(),
                        Some(format!("a{done}").into_bytes()),
                        i64::from((held - 100) % 10 / 5) * 3,
                        paused,
                    )
                }
                1 => {
                    let tokens = if hour < 140 {
                        (0..(step / 7) as usize).map(|n| format!("b{n}")).collect()
                    } else {
                        std::iter::once("b_reset".to_owned())
                            .chain((0..((hour - 140) / 9) as usize).map(|n| format!("r{n}")))
                            .collect()
                    };
                    let current = match hour {
                        185.. if hour % 2 == 0 => b"\xffnext".to_vec(),
                        185.. => b"\xfenext".to_vec(),
                        _ => b"b_next".to_vec(),
                    };
                    (tokens, Some(current), step / 4, false)
                }
                _ => (
                    vec![format!("c{index}")],
                    (index != 4).then(|| b"shared".to_vec()),
                    step / 6,
                    false,
                ),
            };
        let extra = i32::from(index == 0 && (145..=146).contains(&hour));
        focus.push(FocusRow {
            country,
            tree: Some(if index == 0 && hour >= 165 { b"tree_b".to_vec() } else { b"tree_a".to_vec() }),
            current,
            progress,
            paused: u8::from(paused),
            completed_count: tokens.len() as i32 + extra,
            ..Default::default()
        });
        completed.extend(tokens.into_iter().enumerate().map(|(ordinal, token)| {
            CompletedFocusRow {
                country,
                ordinal: ordinal as u32,
                token: token.into_bytes(),
            }
        }));
    }
    *frame.tables[TableId::Focus.index()] = if (150..=151).contains(&hour) {
        TableView::Unavailable {
            reason: REFUSAL.into(),
        }
    } else {
        TableView::Complete(TableRows::Focus(focus))
    };
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(completed));
    frame
}

/// Branch 1's recorded hours: 100 through 190 without 136 and 137.
fn hours() -> Vec<i32> {
    (100..=190)
        .filter(|hour| !(136..=137).contains(hour))
        .collect()
}

/// The recording of `frames`, in blocks of 12.
fn recording(frames: &[DecodedFrame]) -> Replay {
    let mut blocks = frames.chunks(12);
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, blocks.next().unwrap().to_vec())])
            .unwrap();
    for block in blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, block.to_vec())])
            .unwrap();
    }
    replay
}

/// A store holding the row preparation's focus pages of `replay`, prepared hour by hour as the
/// hourly walk prepared them, and the source reading them.
struct Prepared {
    source: Source,
    store: store::Store,
    key: store::SourceKey,
    recording: std::path::PathBuf,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        let _ = self.store.clear();
        let _ = std::fs::remove_file(&self.recording);
    }
}

fn prepared(replay: Replay) -> Prepared {
    let cancelled = AtomicBool::new(false);
    let id = uuid::Uuid::new_v4();
    let recording = std::env::temp_dir().join(format!("prepared-focus-{id}.battleplan"));
    std::fs::write(&recording, b"synthetic recording").unwrap();
    let key = store::SourceKey::capture(&recording, &[]).unwrap();
    let store = store::Store::local(id).unwrap();
    let mut index = row::FocusIndex::default();
    let mut writer = store.replace(key.clone()).unwrap();
    for block in &replay.index {
        replay
            .read_block_alone(
                block,
                row::TABLES,
                |_, frame| index.observe_frame(&replay, frame, &cancelled),
                || Ok(()),
            )
            .unwrap();
    }
    index.flush(&mut writer, &cancelled).unwrap();
    writer.commit().unwrap();
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: id.to_string(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &cancelled,
    )
    .unwrap();
    Prepared {
        source,
        store,
        key,
        recording,
    }
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, view: &TableView, rows: Option<&[R]>) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match (view, rows) {
        (_, Some(rows)) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        (TableView::Unavailable { reason }, None) => Input::Unavailable {
            reason,
            read,
            sample,
        },
        _ => panic!("the fixture holds {table:?} whole or unavailable"),
    };
    walk.table(table, input).unwrap();
}

/// Writes `frame`'s four focus tables into `walk`.
fn write_frame(walk: &mut Synthetic, frame: &DecodedFrame) {
    for &table in facts::TABLES {
        let view = &*frame.tables[table.index()];
        match view {
            TableView::Complete(TableRows::Countries(rows)) => write(walk, table, view, Some(rows)),
            TableView::Complete(TableRows::CountryLinks(rows)) => {
                write(walk, table, view, Some(rows))
            }
            TableView::Complete(TableRows::Focus(rows)) => write(walk, table, view, Some(rows)),
            TableView::Complete(TableRows::CompletedFocuses(rows)) => {
                write(walk, table, view, Some(rows))
            }
            _ => write::<FocusRow>(walk, table, view, None),
        }
    }
}

/// The projected hours of `frames`, a projector starting cold at each of `starts` and at each
/// walk's first hour (after a gap, the walk starts again).
fn projected(frames: &[DecodedFrame], starts: &[i32]) -> Vec<facts::Hour> {
    let mut out = Vec::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
    let mut last: Option<i32> = None;
    for (ordinal, frame) in (0..).zip(frames) {
        let gap = last.is_some_and(|last| last + 1 != frame.hour);
        last = Some(frame.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, frame.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write_frame(walk, frame);
        let cold = starts.contains(&frame.hour);
        if cold {
            projector = Projector::new(FormatVersion::CURRENT, CaptureMode::Full).unwrap();
        }
        walk.end(|state, manifest, decoded, first| {
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(false),
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
    }
    // As its emit is handed them: written to the spool and read back.
    replay_spool::respool(out).unwrap()
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
            emit.hour(&mut writer, hour, None).unwrap();
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

/// Holds the publication `built` to the row preparation `prepared` of `frames`.
fn check(built: &Built, prepared: &Prepared, frames: &[DecodedFrame], label: &str) {
    let budget = Budget::new(64 << 20);
    let cache = Cache::new(&budget);
    let mut snapshot = Snapshot::open(built.pages.clone(), &cache, built.root, identity()).unwrap();
    let directory = Directory::read(&mut snapshot, SCHEMA).unwrap();
    let cancelled = AtomicBool::new(false);
    let tags: BTreeSet<String> = frames
        .iter()
        .flat_map(|frame| match &*frame.tables[TableId::Countries.index()] {
            TableView::Complete(TableRows::Countries(rows)) => rows.clone(),
            _ => Vec::new(),
        })
        .map(|country| String::from_utf8_lossy(&country.tag).into_owned())
        .filter(|tag| !tag.is_empty())
        .chain(["ZZZ".to_owned()])
        .collect();
    let asked: Vec<i32> = (98..=193).collect();
    let mut observed = 0;
    let mut answered_without = [0; 2];
    for hour in asked.iter().copied() {
        let row = row::query::query_unavailable(&prepared.store, &prepared.key, 1, hour, &cancelled)
            .unwrap();
        let native = query::unavailable(&mut snapshot, &directory, hour).unwrap();
        // A prepared hour without refusals is answered: a null reply is one still pending.
        let reply = serde_json::to_value(&native).unwrap();
        assert!(reply.is_object(), "{label}: refusals at {hour}: {reply}");
        answered_without[0] += usize::from(native.refused.is_none());
        assert_eq!(
            serde_json::to_value(&native.refused).unwrap(),
            serde_json::to_value(&row).unwrap(),
            "{label}: refusals at {hour}"
        );
    }
    for tag in &tags {
        for hour in asked.iter().copied() {
            let row = row::query::query_hour(
                &prepared.source,
                &prepared.store,
                &prepared.key,
                1,
                tag,
                hour,
                &cancelled,
            )
            .unwrap();
            let native = query::hour(&mut snapshot, &directory, tag, hour).unwrap();
            // A country not observed at a prepared hour is answered: a null reply is one pending.
            let reply = serde_json::to_value(&native).unwrap();
            assert!(reply.is_object(), "{label}: {tag} at {hour}: {reply}");
            answered_without[1] += usize::from(native.focus.is_none());
            assert_eq!(
                serde_json::to_value(&native.focus).unwrap(),
                serde_json::to_value(&row).unwrap(),
                "{label}: {tag} at {hour}"
            );
            let Some(row) = row else { continue };
            observed += 1;
            for page in 0..=row.completed.count {
                let held = row::query::order_page(
                    &prepared.store,
                    &prepared.key,
                    1,
                    tag,
                    hour,
                    &row.completed,
                    page,
                    &cancelled,
                )
                .unwrap();
                let native =
                    query::order_page(&mut snapshot, &directory, tag, hour, &row.completed, page)
                        .unwrap();
                assert_eq!(native, held, "{label}: {tag} at {hour}, page {page}");
            }
            let mut stale = row.completed.clone();
            stale.rows += 1;
            assert!(
                query::order_page(&mut snapshot, &directory, tag, hour, &stale, 0).is_err(),
                "{label}: {tag} at {hour}: a page of another order"
            );
        }
        assert_eq!(
            query::completions(&mut snapshot, &directory, tag).unwrap(),
            row::query::completions(&prepared.store, &prepared.key, 1, tag, &cancelled).unwrap(),
            "{label}: {tag}'s completions"
        );
    }
    assert!(observed > 300, "{label}: {observed} observed country-hours");
    assert!(
        answered_without.iter().all(|&count| count > 0),
        "{label}: hours without refusals and unobserved country-hours, each answered: {answered_without:?}"
    );
    assert_eq!(
        query::feed(&mut snapshot, &directory).unwrap(),
        row::feed(prepared.source.replay_ref(), 1, &cancelled).unwrap(),
        "{label}: what the event feed reads"
    );
}

/// Every placement of segment starts, cuts and a resume publishes what the row preparation
/// answers, through completions, a reset, an identity replacement, a shared tag, an empty tag,
/// an unresolved country, a missing focus row, an incomplete order, unread tables, a gap,
/// renderings alike and a pause longer than a block.
#[test]
fn the_publication_answers_what_the_row_preparation_answers_across_segments_cuts_and_a_resume()
{
    let budget = Budget::new(256 << 20);
    let frames: Vec<DecodedFrame> = hours().into_iter().map(|hour| focus_frame(1, hour)).collect();
    let prepared = prepared(recording(&frames));
    let every: Vec<i32> = hours();
    for (starts, cuts, resume) in [
        (&[][..], &[190][..], None),
        (&[110, 140, 150][..], &[109, 135, 149, 190][..], Some(135)),
        (
            &[103, 115, 125, 138, 151, 181][..],
            &[114, 133, 146, 170, 190][..],
            Some(146),
        ),
        (&every[1..], &every[..], Some(160)),
    ] {
        let label = format!("starts {starts:?}, cuts {cuts:?}, resume {resume:?}");
        let built = build(projected(&frames, starts), cuts, resume, &budget);
        check(&built, &prepared, &frames, &label);
    }
    // The fixture reaches every case it names.
    let feed = row::feed(prepared.source.replay_ref(), 1, &AtomicBool::new(false)).unwrap();
    for reason in [UNRESOLVED, NO_FOCUS_ROW, NO_TAG, INCOMPLETE_ORDER, INCOMPLETE_TABLES, RESET, UNDATED] {
        assert!(feed.unavailable.contains_key(reason), "{reason}");
    }
    assert!(feed.completions.len() >= 8);
}

/// An hour hands the emit only the observations that changed: of 1,500 countries, three whose
/// progress moved, none where only balances moved, and every one at a segment's first hour.
#[test]
fn an_hour_projects_only_the_observations_that_changed() {
    const COUNT: u32 = 1_500;
    let frame = |hour: i32| {
        let mut frame = scale_frame(1, hour, COUNT);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            for row in rows.iter_mut() {
                row.political_power_raw = Some(i64::from(hour));
            }
        }
        let moved = |index: u32| hour >= 102 && index < 3;
        *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
            (0..COUNT)
                .map(|index| FocusRow {
                    country: 100_000 + index,
                    current: Some(b"focus".to_vec()),
                    progress: i64::from(moved(index)) * i64::from(hour),
                    completed_count: 1,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(
                (0..COUNT)
                    .map(|index| CompletedFocusRow {
                        country: 100_000 + index,
                        ordinal: 0,
                        token: format!("done{index}").into_bytes(),
                    })
                    .collect(),
            ));
        frame
    };
    let frames: Vec<DecodedFrame> = (100..=103).map(frame).collect();
    let projected = projected(&frames, &[]);
    let changed = |at: usize| {
        projected[at]
            .facts
            .as_ref()
            .map_or(0, |facts| facts.tags.len())
    };
    assert!(projected[0].facts.as_ref().unwrap().whole);
    assert_eq!(changed(0), COUNT as usize);
    assert!(projected[1].facts.is_none(), "only balances moved");
    assert_eq!(changed(2), 3);
    assert_eq!(changed(3), 3);
    assert!(!projected[3].facts.as_ref().unwrap().whole);
}
