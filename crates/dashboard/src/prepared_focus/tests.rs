use super::*;
use crate::products::Product;
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame, scale_tag};
use replay_data::rows::{FocusRow, TableRows};
use std::sync::Arc;

/// Countries A to E (`scale_tag(0..5)`, slots 0 to 4) whose balances move
/// every hour, with their focus at `hour`:
/// - A completes `a0`, `a1`, ... every ten hours from 100; its progress
///   rises every five hours and holds while paused from 160 to 175 (longer
///   than a block).
/// - B completes `b0`, `b1`, ...; its list resets to `b_reset` at 140 (a
///   shrink and a prefix break) and grows again.
/// - C is country 2 until 129; from 130 country 4 holds the tag (an identity
///   replacement) and country 2 is renamed.
/// - D has no focus row from 120 to 125.
/// - Focus is not read at 150 and 151; hours 136 and 137 are not recorded.
fn focus_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = scale_frame(branch, hour, 5);
    let tag = |index: u32| -> Vec<u8> {
        match (index, hour >= 130) {
            (2, true) => b"OLD".to_vec(),
            (4, true) => scale_tag(2),
            (4, false) => b"NEW".to_vec(),
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
    let step = i64::from(hour - 100);
    let mut focus = Vec::new();
    let mut completed = Vec::new();
    for index in 0..5_u32 {
        if index == 3 && (120..=125).contains(&hour) {
            continue;
        }
        let country = 100_000 + index;
        let (tokens, current, progress, paused): (Vec<String>, Option<String>, i64, bool) =
            match index {
                0 => {
                    let paused = (160..=175).contains(&hour);
                    let done = (step / 10) as usize;
                    let held = if paused { 155 } else { hour };
                    (
                        (0..done).map(|n| format!("a{n}")).collect(),
                        Some(format!("a{done}")),
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
                    (tokens, Some("b_next".into()), step / 4, false)
                }
                _ => (
                    vec![format!("c{index}")],
                    (index != 4).then(|| "shared".to_owned()),
                    step / 6,
                    false,
                ),
            };
        focus.push(FocusRow {
            country,
            tree: Some(format!("tree{}", hour % 3).into_bytes()),
            current: current.map(String::into_bytes),
            progress,
            paused: u8::from(paused),
            completed_count: tokens.len() as i32,
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
            reason: "Synthetic focus read unavailable".into(),
        }
    } else {
        TableView::Complete(TableRows::Focus(focus))
    };
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(completed));
    frame
}

/// Every tag the fixture actually records, including both sides of its
/// identity change. The fifth slot never holds `scale_tag(4)`.
fn focus_tags() -> impl Iterator<Item = String> {
    (0..4)
        .map(|index| String::from_utf8(scale_tag(index)).unwrap())
        .chain(["NEW".to_owned(), "OLD".to_owned()])
}

/// Branch 1's hours 100 through 190 without 136 and 137, in blocks of 12.
fn scenario() -> Replay {
    let hours: Vec<i32> = (100..=190)
        .filter(|hour| !(136..=137).contains(hour))
        .collect();
    let mut blocks = hours.chunks(12);
    let frames = |hours: &[i32]| hours.iter().map(|&hour| focus_frame(1, hour)).collect();
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, frames(blocks.next().unwrap()))],
    )
    .unwrap();
    for block in blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(block))])
            .unwrap();
    }
    replay
}

/// A store holding `replay`'s focus pages, prepared hour by hour as the
/// hourly walk does, and the source reading them.
struct Prepared {
    source: Source,
    store: store::Store,
    key: store::SourceKey,
    recording: std::path::PathBuf,
    frames: Vec<DecodedFrame>,
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
    let mut frames = Vec::new();
    for block in &replay.index {
        replay
            .read_block_alone(
                block,
                TABLES,
                |_, frame| {
                    frames.push(frame.clone());
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    let mut index = FocusIndex::default();
    let mut writer = store.replace(key.clone()).unwrap();
    for frame in &frames {
        index.observe_frame(&replay, frame, &cancelled).unwrap();
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
        frames,
    }
}

impl Prepared {
    fn hour(&self, tag: &str, hour: i32) -> Option<Hour> {
        query_hour(
            &self.source,
            &self.store,
            &self.key,
            1,
            tag,
            hour,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    /// Every completed focus the hour's pages list.
    fn order(&self, tag: &str, hour: &Hour) -> Vec<FocusEntry> {
        let mut entries = Vec::new();
        for page in 0..hour.completed.count {
            let rows = order_page(
                &self.store,
                &self.key,
                1,
                tag,
                hour.at,
                &hour.completed,
                page,
                &AtomicBool::new(false),
            )
            .unwrap();
            entries.extend(serde_json::from_value::<Vec<FocusEntry>>(rows).unwrap());
        }
        entries
    }
}

/// The progress and daily gain of `tag` at each hour it is observed, as
/// hourly preparation carried them hour by hour: a run of equal progress
/// breaks at a gap, an hour unobserved or another identity; its gain is the
/// rise since the observation the hour before it began, while the same focus
/// is active.
fn hourly_progress(frames: &[DecodedFrame], tag: &str) -> BTreeMap<i32, (i64, Option<i128>)> {
    struct Seen {
        hour: i32,
        identity: (u32, u32),
        raw: i64,
        current: Option<Recorded>,
    }
    let mut last: Option<Seen> = None;
    type Run = (i32, i64, Option<(i64, i32, Option<Recorded>)>);
    let mut run: Option<Run> = None;
    let mut result = BTreeMap::new();
    for frame in frames {
        let Some(observed) = read_hour(frame, Some(tag), &HourRead::default(), false)
            .tags
            .remove(tag)
        else {
            continue;
        };
        let identity = (observed.slot, observed.country_id);
        let current = observed.order.current();
        let same = last.as_ref().filter(|last| last.identity == identity);
        let consecutive = same.is_some_and(|last| last.hour + 1 == frame.hour);
        if !(consecutive && run.as_ref().is_some_and(|run| run.1 == observed.progress)) {
            run = Some((
                frame.hour,
                observed.progress,
                same.map(|last| (last.raw, last.hour, last.current.clone())),
            ));
        }
        let (first, raw, previous) = run.as_ref().unwrap();
        let gain = match previous {
            Some((before, hour, active))
                if hour + 1 == *first && current.is_some() && current == *active =>
            {
                Some(i128::from(*raw) - i128::from(*before)).filter(|gain| *gain > 0)
            }
            _ => None,
        };
        result.insert(frame.hour, (observed.progress, gain));
        last = Some(Seen {
            hour: frame.hour,
            identity,
            raw: observed.progress,
            current,
        });
    }
    result
}

/// At every recorded hour and for every country, the prepared order (its
/// hours, the run it began, the active focus, pause and every dated
/// completion) is the whole-branch focus history's, through completions, a
/// reset, an identity replacement, a missing focus row, unread tables, a gap
/// and a pause longer than a block. Progress and its daily gain are hourly
/// preparation's, read from the hour's block alone.
#[test]
fn chain_dates_order_segments_and_gain_match_focus_history() {
    let replay = scenario();
    let prepared = prepared(replay);
    let cancelled = AtomicBool::new(false);
    let replay = prepared.source.replay();
    let hours: Vec<i32> = prepared.frames.iter().map(|frame| frame.hour).collect();
    let mut checked = 0;
    // Country 4 is NEW then takes country 2's tag, while country 2 becomes
    // OLD. Exercise every recorded tag instead of the never-recorded tag 004.
    let tags: std::collections::BTreeSet<_> = prepared
        .frames
        .iter()
        .flat_map(|frame| {
            frame
                .complete::<replay_data::rows::tables::Countries>()
                .unwrap()
        })
        .map(|country| String::from_utf8(country.tag.clone()).unwrap())
        .collect();
    for tag in tags {
        let history =
            crate::products::national_focus::focus_history(&replay, 1, &tag, &cancelled).unwrap();
        let progress = hourly_progress(&prepared.frames, &tag);
        for &hour in &hours {
            let segment = history
                .segments
                .iter()
                .find(|segment| segment.first_hour <= hour && hour <= segment.last_hour);
            let reads = query::BLOCK_READS.with(|reads| reads.borrow().len());
            let actual = prepared.hour(&tag, hour);
            let Some(segment) = segment else {
                assert!(actual.is_none(), "{tag} at {hour}");
                continue;
            };
            let actual = actual.unwrap_or_else(|| panic!("{tag} at {hour}"));
            let block = replay
                .index
                .iter()
                .find(|block| block.first_hour <= hour && hour <= block.last_hour)
                .unwrap();
            assert_eq!(
                query::BLOCK_READS.with(|reads| reads.borrow()[reads.borrow().len() - 1]),
                block.first_hour,
                "{tag} at {hour}: progress reads the hour's block"
            );
            assert_eq!(
                query::BLOCK_READS.with(|reads| reads.borrow().len()) - reads,
                1,
                "{tag} at {hour}: one block read"
            );
            assert_eq!(
                (actual.hour, actual.last_hour, actual.order_first_hour),
                (
                    segment.first_hour,
                    segment.last_hour,
                    Some(segment.first_hour)
                ),
                "{tag} at {hour}"
            );
            assert_eq!(
                (&actual.current, &actual.continuous, actual.paused),
                (
                    &segment.order.current,
                    &segment.order.continuous,
                    segment.order.paused
                ),
                "{tag} at {hour}"
            );
            assert_eq!(
                prepared.order(&tag, &actual),
                segment.order.completed,
                "{tag} at {hour}"
            );
            let (raw, gain) = progress[&hour];
            assert_eq!(actual.progress_raw, raw.to_string(), "{tag} at {hour}");
            assert_eq!(
                actual.daily_gain_raw,
                gain.map(|gain| gain.to_string()),
                "{tag} at {hour}"
            );
            assert_eq!(actual.at, hour);
            checked += 1;
        }
    }
    assert!(checked > 300, "{checked} observed country-hours checked");
    // The fixture reaches every case: dates, a reset, an identity change and
    // gains inside and across blocks.
    let a = String::from_utf8(scale_tag(0)).unwrap();
    let b = String::from_utf8(scale_tag(1)).unwrap();
    let c = String::from_utf8(scale_tag(2)).unwrap();
    let late = prepared.hour(&a, 190).unwrap();
    assert!(
        prepared
            .order(&a, &late)
            .iter()
            .filter(|entry| entry.completed_by_hour.is_some())
            .count()
            >= 8
    );
    let reset = prepared.hour(&b, 140).unwrap();
    assert!(
        prepared
            .order(&b, &reset)
            .iter()
            .all(|entry| entry.completed_by_hour.is_none())
    );
    assert_eq!(
        (
            prepared.hour(&c, 129).unwrap().country_id,
            prepared.hour(&c, 130).unwrap().country_id
        ),
        (100_002, 100_004)
    );
    assert!(prepared.hour(&a, 150).is_none());
    assert!(hours.iter().any(|&hour| {
        prepared
            .hour(&a, hour)
            .is_some_and(|row| row.daily_gain_raw.is_some())
    }));
}

/// A country's prepared completions are the dated completions of every order its whole-branch
/// history holds, each once, through completions, a reset and an identity replacement.
#[test]
fn completions_are_the_dated_completions_of_every_order_the_history_holds() {
    let replay = scenario();
    let prepared = prepared(replay);
    let cancelled = AtomicBool::new(false);
    let replay = prepared.source.replay();
    let tags: std::collections::BTreeSet<_> = prepared
        .frames
        .iter()
        .flat_map(|frame| {
            frame
                .complete::<replay_data::rows::tables::Countries>()
                .unwrap()
        })
        .map(|country| String::from_utf8(country.tag.clone()).unwrap())
        .collect();
    let key = |entry: &FocusEntry| (entry.completed_by_hour, entry.ordinal, entry.token.clone());
    let mut dated = 0;
    for tag in &tags {
        let history =
            crate::products::national_focus::focus_history(&replay, 1, tag, &cancelled).unwrap();
        let expected: Vec<FocusEntry> = history
            .segments
            .iter()
            .flat_map(|segment| &segment.order.completed)
            .filter(|entry| entry.completed_by_hour.is_some())
            .map(|entry| (key(entry), entry.clone()))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .collect();
        let mut actual =
            completions(&prepared.store, &prepared.key, 1, tag, &cancelled).unwrap();
        actual.sort_by_key(key);
        assert_eq!(actual, expected, "{tag}");
        dated += actual.len();
    }
    assert!(dated >= 8, "{dated} dated completions checked");
}

#[test]
fn sampled_focus_has_the_prepared_order_and_values_without_invented_dates() {
    let prepared = prepared(scenario());
    let flag = AtomicBool::new(false);
    for hour in [100, 119, 130, 138, 149, 159, 176, 190] {
        for tag in focus_tags() {
            let exact = prepared.hour(&tag, hour);
            let sampled = sampled_hour(&prepared.source, 1, &tag, hour, &flag).unwrap();
            assert_eq!(sampled.is_some(), exact.is_some(), "{tag} at {hour}");
            let (Some(sampled), Some(exact)) = (sampled, exact) else {
                continue;
            };
            assert_eq!(
                (
                    &sampled.current,
                    &sampled.continuous,
                    sampled.paused,
                    &sampled.progress_raw
                ),
                (
                    &exact.current,
                    &exact.continuous,
                    exact.paused,
                    &exact.progress_raw
                )
            );
            if sampled.daily_gain_raw.is_some() {
                assert_eq!(sampled.daily_gain_raw, exact.daily_gain_raw);
            }
            assert!(sampled.order_first_hour.is_none());
            assert!(
                sampled
                    .preparing
                    .iter()
                    .any(|field| field == "completedByHour")
            );
            let mut entries = Vec::new();
            for page in 0..sampled.completed.count {
                let data = sampled_page(
                    &prepared.source,
                    1,
                    &tag,
                    hour,
                    &sampled.completed,
                    page,
                    &flag,
                )
                .unwrap();
                entries.extend(serde_json::from_value::<Vec<FocusEntry>>(data).unwrap());
            }
            assert!(
                entries
                    .iter()
                    .all(|entry| entry.completed_by_hour.is_none())
            );
            let expected = prepared.order(&tag, &exact);
            assert_eq!(
                entries
                    .iter()
                    .map(|entry| (entry.ordinal, &entry.token))
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|entry| (entry.ordinal, &entry.token))
                    .collect::<Vec<_>>()
            );
        }
    }
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    sampled_hour(&prepared.source, 1, &tag, 190, &flag).unwrap();
    let reads = dashboard_base::prepared::query::frames::reads(&prepared.source);
    sampled_hour(&prepared.source, 1, &tag, 190, &flag).unwrap();
    assert_eq!(
        dashboard_base::prepared::query::frames::reads(&prepared.source),
        reads,
        "a re-ask shares decoded hours"
    );
}

/// Completions are the order's newly dated entries at hours whose country
/// was observed the hour before; those first seen after a gap, an unread
/// table or another identity are counted without an event date, and a
/// reset is counted once.
#[test]
fn completions_are_dated_only_when_observed_the_hour_before() {
    let prepared = prepared(scenario());
    let cancelled = AtomicBool::new(false);
    let mut index = FocusIndex::default();
    let replay = prepared.source.replay();
    for frame in &prepared.frames {
        index.observe_frame(&replay, frame, &cancelled).unwrap();
    }
    index.fold_held(&cancelled).unwrap();
    let a = String::from_utf8(scale_tag(0)).unwrap();
    let b = String::from_utf8(scale_tag(1)).unwrap();
    let completions = |tag: &str| -> Vec<(String, i32)> {
        index
            .seen
            .iter()
            .filter(|event| event.tag == tag)
            .map(|event| (event.token.clone(), event.hour))
            .collect()
    };
    // a{n} completes at 100 + 10 (n + 1): a2 at 130 and a3 at 140 (after
    // the gap), each observed the hour before; a4 completes at 150, unread,
    // and is first seen at 152; a5 completes at 160, inside the pause.
    let ours = completions(&a);
    assert!(ours.contains(&("a2".into(), 130)));
    assert!(ours.contains(&("a3".into(), 140)));
    assert!(ours.contains(&("a5".into(), 160)));
    assert!(
        ours.iter().all(|(token, _)| token != "a4"),
        "a4 first seen after unread hours has no event date: {ours:?}"
    );
    assert!(
        completions(&b).iter().all(|(token, _)| token != "b_reset"),
        "a reset lists nothing"
    );
    let (observed, unavailable) = index.completion_metadata();
    assert!(observed);
    assert_eq!(unavailable.get(RESET), Some(&1));
    assert!(unavailable.get(UNDATED).is_some_and(|count| *count >= 1));
    // Two hours with Focus unread.
    assert_eq!(unavailable.get(INCOMPLETE_TABLES), Some(&2));
    // D has no focus row for six hours.
    assert_eq!(unavailable.get(NO_FOCUS_ROW), Some(&6));
}

/// Hours at which tables are not complete keep their named reasons, bounded
/// by the hours they hold, beside the numeric focus of the hours between.
#[test]
fn unavailable_table_intervals_keep_named_reasons() {
    let frames: Vec<DecodedFrame> = [100, 101, 102, 105]
        .into_iter()
        .map(|hour| {
            let mut frame = focus_frame(1, hour);
            if hour != 102 {
                *frame.tables[TableId::Focus.index()] = TableView::Unavailable {
                    reason: "exact synthetic supplying-read reason".into(),
                };
            }
            if hour == 105 {
                *frame.tables[TableId::CompletedFocuses.index()] = TableView::Truncated {
                    rows: TableRows::CompletedFocuses(Vec::new()),
                    missing: u32::MAX,
                };
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let prepared = prepared(replay);
    let cancelled = AtomicBool::new(false);
    let missing = query_unavailable(&prepared.store, &prepared.key, 1, 101, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!((missing.first_hour, missing.last_hour), (100, 101));
    assert_eq!(
        missing.reasons,
        [TableRefusal {
            table: "Focus".into(),
            reason: Some("exact synthetic supplying-read reason".into()),
            missing_rows: None
        }]
    );
    assert!(
        query_unavailable(&prepared.store, &prepared.key, 1, 102, &cancelled)
            .unwrap()
            .is_none()
    );
    assert!(
        query_unavailable(&prepared.store, &prepared.key, 1, 104, &cancelled)
            .unwrap()
            .is_none()
    );
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    assert!(prepared.hour(&tag, 102).is_some());
    assert!(prepared.hour(&tag, 101).is_none());
    let later = query_unavailable(&prepared.store, &prepared.key, 1, 105, &cancelled)
        .unwrap()
        .unwrap();
    assert_eq!(later.reasons[1].missing_rows.as_deref(), Some("4294967295"));
}

/// Two recorded tokens that are not UTF-8 but show the same replacement text
/// are different focuses: the order changes between them (the active focus
/// at 102, a completed focus at 103, which starts an undated list). An
/// unchanged recorded order holds its run hour after hour (104).
#[test]
fn distinct_recorded_bytes_with_one_rendering_change_the_order() {
    let at = |hour: i32| {
        let mut frame = scale_frame(1, hour, 1);
        let current: &[u8] = if hour < 102 {
            &b"x\xff"[..]
        } else {
            &b"x\xfe"[..]
        };
        let completed: Vec<&[u8]> = match hour {
            100 => vec![&b"ok"[..]],
            101 | 102 => vec![&b"ok"[..], &b"c\xff"[..]],
            _ => vec![&b"ok"[..], &b"c\xfe"[..]],
        };
        *frame.tables[TableId::Focus.index()] =
            TableView::Complete(TableRows::Focus(vec![FocusRow {
                country: 100_000,
                current: Some(current.to_vec()),
                completed_count: completed.len() as i32,
                ..Default::default()
            }]));
        *frame.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(
                completed
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, token)| CompletedFocusRow {
                        country: 100_000,
                        ordinal: ordinal as u32,
                        token: token.to_vec(),
                    })
                    .collect(),
            ));
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (100..=104).map(at).collect())],
    )
    .unwrap();
    let prepared = prepared(replay);
    let tag = String::from_utf8(scale_tag(0)).unwrap();
    let hour = |hour| prepared.hour(&tag, hour).unwrap();
    // Both active focuses show one replacement text, yet the order changes.
    assert_eq!(hour(101).current, hour(102).current);
    assert_eq!(
        (hour(101).order_first_hour, hour(101).last_hour),
        (Some(101), 101)
    );
    assert_eq!(hour(102).order_first_hour, Some(102));
    let dated = prepared.order(&tag, &hour(102));
    assert_eq!(dated[1].completed_by_hour, Some(101));
    // The completed focus's bytes changed: a new, undated list.
    let reset = prepared.order(&tag, &hour(103));
    assert_eq!(reset[1].token, dated[1].token, "one rendering");
    assert!(reset.iter().all(|entry| entry.completed_by_hour.is_none()));
    assert_eq!(hour(103).order_first_hour, Some(103));
    // Equal recorded bytes hold the run.
    assert_eq!(
        (hour(104).order_first_hour, hour(104).hour),
        (Some(103), 103)
    );
}

/// Completed-focus tokens a block's facts retain: each list part once,
/// however many orders share it.
fn retained_tokens(facts: &Facts) -> usize {
    let mut parts = std::collections::HashSet::new();
    let mut tokens = 0;
    for stretches in facts.tags.values() {
        for stretch in stretches {
            for (_, order) in &stretch.orders {
                let mut part = Some(&order.completed);
                while let Some(at) = part {
                    if parts.insert(Arc::as_ptr(&at.added) as *const Recorded as usize) {
                        tokens += at.added.len();
                    }
                    part = at.before.as_deref();
                }
            }
        }
    }
    tokens
}

/// Completions staggered over a block, one country an hour, each table a
/// new allocation: the block's facts keep each country's completions once
/// and each completion once (not a table per change), and the lists read
/// back whole and in order.
#[test]
fn a_block_retains_each_country_s_completions_once_and_each_change() {
    const COUNTRIES: u32 = 40;
    const BASE: usize = 50;
    let tokens = |country: u32, hour: i32| -> Vec<String> {
        (0..BASE)
            .map(|n| format!("b{n}"))
            .chain(
                (101..=hour)
                    .filter(|at| (at - 101).rem_euclid(COUNTRIES as i32) as u32 == country)
                    .map(|at| format!("x{at}")),
            )
            .collect()
    };
    let at = |hour: i32| {
        let mut frame = scale_frame(1, hour, COUNTRIES);
        *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
            (0..COUNTRIES)
                .map(|country| FocusRow {
                    country: 100_000 + country,
                    completed_count: tokens(country, hour).len() as i32,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(
                (0..COUNTRIES)
                    .flat_map(|country| {
                        tokens(country, hour).into_iter().enumerate().map(
                            move |(ordinal, token)| CompletedFocusRow {
                                country: 100_000 + country,
                                ordinal: ordinal as u32,
                                token: token.into_bytes(),
                            },
                        )
                    })
                    .collect(),
            ));
        frame
    };
    let mut builder = Builder::new(None);
    let changes = 120;
    for hour in 100..=100 + changes {
        builder.hour(&at(hour)).unwrap();
    }
    let facts = builder.finish();
    assert_eq!(
        retained_tokens(&facts),
        COUNTRIES as usize * BASE + changes as usize
    );
    for country in 0..COUNTRIES {
        let tag = String::from_utf8(scale_tag(country)).unwrap();
        let (_, last) = facts.tags[tag.as_str()][0].orders.last().unwrap();
        let read: Vec<String> = last
            .completed
            .tokens()
            .iter()
            .map(|token| token.text())
            .collect();
        assert_eq!(read, tokens(country, 100 + changes), "{tag}");
    }
}

/// Kept art stays within its byte budget: the least recently used goes
/// first, art larger than the budget is not kept, and a replaced key counts
/// once.
#[test]
fn kept_art_stays_within_its_budget_evicting_the_least_recently_used() {
    let art = |token: &str, icon: usize| -> crate::catalogue::Drawn<artwork::ResolvedArt> {
        crate::catalogue::Drawn::new(|| {
            (
                Artwork {
                    token: token.into(),
                    name: Some("Name".into()),
                    description: None,
                    base_duration_days: None,
                    icon: Some("i".repeat(icon)),
                },
                None,
            )
        })
    };
    let key = |token: &str| {
        (
            crate::FocusDatabase::National,
            Presentation::Display,
            token.to_owned(),
            String::new(),
        )
    };
    let mut kept = art::Kept::with_budget(100_000);
    kept.insert(key("a"), art("a", 40_000));
    kept.insert(key("b"), art("b", 40_000));
    // Using a makes b the least recently used.
    assert!(kept.get(&key("a")).is_some());
    kept.insert(key("c"), art("c", 40_000));
    assert!(kept.bytes() <= 100_000);
    assert!(
        kept.get(&key("b")).is_none(),
        "the least recently used is evicted"
    );
    assert!(kept.get(&key("a")).is_some() && kept.get(&key("c")).is_some());
    kept.insert(key("c"), art("c", 40_000));
    assert_eq!(kept.count(), 2);
    assert!(kept.bytes() <= 100_000);
    kept.insert(key("huge"), art("huge", 200_000));
    assert!(kept.get(&key("huge")).is_none(), "larger than the budget");
    assert_eq!(kept.count(), 2);
}

/// A tag held by two countries is the one with the higher country id, as
/// hourly preparation published it, whichever row comes first.
#[test]
fn a_shared_tag_is_the_higher_country_id() {
    let frames: Vec<DecodedFrame> = (100..=102)
        .map(|hour| {
            let mut frame = focus_frame(1, hour);
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[3].tag = scale_tag(1);
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let prepared = prepared(replay);
    let tag = String::from_utf8(scale_tag(1)).unwrap();
    for hour in 100..=102 {
        assert_eq!(prepared.hour(&tag, hour).unwrap().country_id, 100_003);
    }
}

/// `focus-art` evaluates any recorded hour as hourly preparation presented
/// it there, row for row and with its warnings: titles reading the country,
/// Clock-seeded random text and effect sessions, conditional icons, a
/// refused sprite and static titles, for a token subset or the whole order.
/// Static art is kept per content binding: another country asking first
/// leaves the next country's rows unchanged. A request of static tokens
/// reads no recording; one with a dynamic token reads its hour once.
#[test]
fn focus_art_matches_hourly_presentation_and_reads_frames_only_for_dynamic_tokens() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use super::art::art_at;
    use crate::catalogue::texture_tests::{png_bytes, texture_catalogue};
    use installed_content_image::Rgba8Image;
    use replay_data::rows::{CountryLinkRow, CountryRow};
    let definitions = format!(
        "focus_tree={{{} focus={{id=a_static text=static_title icon=GFX_mod_true}} focus={{id=b_refused text=static_title icon=GFX_missing}}}}",
        (0..12)
            .map(|index| format!(
                "focus={{id=t{index:02} text=title{} icon={{GFX_mod_true={{tag=GER}} GFX_mod_other=yes}}}}",
                index % 3
            ))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let image = |red| {
        png_bytes(&Rgba8Image {
            width: 1,
            height: 1,
            pixels: vec![red, 0, 0, 255],
        })
    };
    let (catalogue, folders) = texture_catalogue(
        &[
            ("common/national_focus/subsets.txt", &definitions),
            (
                "interface/subsets.gfx",
                "spriteTypes={spriteType={name=GFX_mod_true textureFile=gfx/mod/true.png} spriteType={name=GFX_mod_other textureFile=gfx/mod/other.png}}",
            ),
            (
                "common/defines/subsets.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NFocus={FOCUS_POINT_DAYS=7}}",
            ),
            (
                "common/scripted_localisation/subsets.txt",
                "entry={name=Choice text={random_list={1={localization_key=A} 1={localization_key=B}}}}",
            ),
            (
                "common/bop/subsets.txt",
                "balance={range={id=active on_activate={set_temp_variable={x=7} if={limit={check_variable={temp_var:x=7}} custom_effect_tooltip=SESSION_SEVEN} else={custom_effect_tooltip=SESSION_FRESH}}}}",
            ),
            (
                "localisation/subsets_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n",
                    " title0:0 \"[ROOT.GetTag] [FROM.GetTag]\"\n title0_desc:0 \"[ROOT.GetTag] description\"\n",
                    " title1:0 \"[Choice] [ROOT.Choice]\"\n title1_desc:0 \"[Choice] description\"\n",
                    " title2:0 \"[ROOT.GetActiveRangeActivationEffect]\"\n title2_desc:0 \"[ROOT.GetActiveRangeActivationEffect]\"\n",
                    " A:0 \"A\"\n B:0 \"B\"\n SESSION_SEVEN:0 \"Session seven\"\n SESSION_FRESH:0 \"Fresh session\"\n",
                    " static_title:0 \"Static title\"\n static_title_desc:0 \"Static details\"\n",
                    " PROGRESS_SECTION_ACTIVATION_EFFECT:0 \"Activate:\\n$DESC$\"\n"
                ),
            ),
        ],
        &[
            ("gfx/mod/true.png", image(17)),
            ("gfx/mod/other.png", image(34)),
        ],
    );
    let tokens: Vec<String> = (0..12)
        .map(|index| format!("t{index:02}"))
        .chain(["a_static".into(), "b_refused".into()])
        .collect();
    let frames: Vec<DecodedFrame> = [100, 101, 102]
        .into_iter()
        .map(|hour| {
            let mut decoded = crate::test_fixtures::source(1, hour, Some(1));
            for table in TableId::ALL {
                // Unused singleton inputs keep their named unavailable state.
                if table.key_bits() == 0 {
                    continue;
                }
                if matches!(
                    &*decoded.tables[table.index()],
                    TableView::Unavailable { .. }
                ) {
                    *decoded.tables[table.index()] = TableView::Complete(TableRows::empty(table));
                }
            }
            // ENG (9002, slot 8) completes the static focus too.
            *decoded.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(vec![
                    CountryRow {
                        id: 9001,
                        canonical_slot: 7,
                        tag: b"GER".to_vec(),
                        ..Default::default()
                    },
                    CountryRow {
                        id: 9002,
                        canonical_slot: 8,
                        tag: b"ENG".to_vec(),
                        ..Default::default()
                    },
                ]));
            *decoded.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow {
                        id: 9001,
                        canonical_slot: 7,
                    },
                    CountryLinkRow {
                        id: 9002,
                        canonical_slot: 8,
                    },
                ]));
            *decoded.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(vec![
                FocusRow {
                    country: 9001,
                    completed_count: tokens.len() as i32,
                    current: Some(b"t05".to_vec()),
                    ..Default::default()
                },
                FocusRow {
                    country: 9002,
                    completed_count: 1,
                    ..Default::default()
                },
            ]));
            *decoded.tables[TableId::CompletedFocuses.index()] =
                TableView::Complete(TableRows::CompletedFocuses(
                    tokens
                        .iter()
                        .enumerate()
                        .map(|(ordinal, token)| CompletedFocusRow {
                            country: 9001,
                            ordinal: ordinal as u32,
                            token: token.as_bytes().to_vec(),
                        })
                        .chain(std::iter::once(CompletedFocusRow {
                            country: 9002,
                            ordinal: 0,
                            token: b"a_static".to_vec(),
                        }))
                        .collect(),
                ));
            *decoded.tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![replay_data::rows::ClockRow {
                    hour,
                    seed: 900 + hour,
                    ..Default::default()
                }]));
            *decoded.tables[TableId::PowerBalances.index()] =
                TableView::Complete(TableRows::PowerBalances(vec![
                    replay_data::rows::PowerBalanceRow {
                        balance: b"balance".to_vec(),
                        active_range: b"active".to_vec(),
                        ..Default::default()
                    },
                ]));
            *decoded.tables[TableId::PowerBalanceMembers.index()] =
                TableView::Complete(TableRows::PowerBalanceMembers(vec![
                    replay_data::rows::PowerBalanceMemberRow {
                        country: 9001,
                        ..Default::default()
                    },
                ]));
            decoded
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: uuid::Uuid::new_v4().to_string(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &cancelled,
    )
    .unwrap();
    let content = ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: folders[0].0.clone(),
            user_dir: folders[0].0.join("user"),
            cache_dir: folders[0].0.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    let catalogue = Arc::new(catalogue);
    crate::catalogue::set_test_catalogue(&source, &content, Arc::clone(&catalogue));
    let reads = || art::FRAME_READS.with(std::cell::Cell::get);
    let national = artwork::Kind::Focus(crate::FocusDatabase::National);
    // Hourly preparation's rows: every token of the country's order
    // evaluated on the hour's whole frame by a context of its own.
    let oracle = |frame: &DecodedFrame, tag: &str, tokens: &[String], mode: Presentation| {
        let mut context = artwork::Context::load(Arc::clone(&catalogue), national).unwrap();
        let inputs = artwork::FocusFrame::new(frame, &catalogue);
        let requests: Vec<_> = tokens.iter().map(|token| (token.as_str(), tag)).collect();
        let rows: Vec<artwork::ResolvedArt> =
            context.focuses_in(&requests, &inputs, true, mode, |items| items.collect());
        let mut warnings = context.standing_warnings();
        warnings.sort();
        warnings.dedup();
        let rows: BTreeMap<String, serde_json::Value> = rows
            .into_iter()
            .map(|(artwork, reason)| {
                (
                    artwork.token.clone(),
                    serde_json::to_value(PresentedFocus { artwork, reason }).unwrap(),
                )
            })
            .collect();
        (rows, warnings)
    };
    let mut sorted = tokens.clone();
    sorted.sort();
    for frame in &frames {
        for mode in [Presentation::Display, Presentation::Text] {
            // ENG asks for the shared static focus first.
            let before = reads();
            let eng = art_at(
                &source,
                &content,
                1,
                "ENG",
                frame.hour,
                crate::FocusDatabase::National,
                &["a_static".into()],
                mode,
                &cancelled,
            )
            .unwrap();
            assert_eq!(reads(), before, "a static request reads no recording");
            let (expected, warnings) = oracle(frame, "ENG", &["a_static".into()], mode);
            assert_eq!(
                serde_json::to_value(&eng.art[0]).unwrap(),
                expected["a_static"]
            );
            assert_eq!(eng.warnings, warnings);
            let (expected, warnings) = oracle(frame, "GER", &sorted, mode);
            for subset in [
                sorted.clone(),
                vec!["a_static".into(), "b_refused".into()],
                sorted[..5].to_vec(),
            ] {
                let before = reads();
                let reply = art_at(
                    &source,
                    &content,
                    1,
                    "GER",
                    frame.hour,
                    crate::FocusDatabase::National,
                    &subset,
                    mode,
                    &cancelled,
                )
                .unwrap();
                let dynamic = subset.iter().any(|token| token.starts_with('t'));
                assert_eq!(
                    reads() - before,
                    usize::from(dynamic),
                    "{subset:?}: a frame only for a dynamic token"
                );
                assert!(reply.failures.is_empty());
                assert_eq!(reply.warnings, warnings);
                assert_eq!(reply.art.len(), subset.len());
                for (token, row) in subset.iter().zip(&reply.art) {
                    assert_eq!(
                        serde_json::to_value(row).unwrap(),
                        expected[token],
                        "{token} at {} in {mode:?}",
                        frame.hour
                    );
                }
            }
        }
    }
    // The refused sprite is named on its row.
    let refused = art_at(
        &source,
        &content,
        1,
        "GER",
        100,
        crate::FocusDatabase::National,
        &["b_refused".into()],
        Presentation::Display,
        &cancelled,
    )
    .unwrap();
    assert!(refused.art[0].reason.is_some());
    // Static text is the content's, in request order with repeats.
    let wanted = vec!["t00".to_owned(), "a_static".into(), "t00".into()];
    let text = art::static_text(
        &source,
        Some(&content),
        crate::FocusDatabase::National,
        &wanted,
        &cancelled,
    )
    .unwrap();
    let context = artwork::Context::load(Arc::clone(&catalogue), national).unwrap();
    for (token, item) in wanted.iter().zip(&text) {
        let (mut expected, _) = context
            .text_catalogue(BTreeSet::from([token.as_str()]), &cancelled)
            .unwrap();
        assert_eq!(
            serde_json::to_value(item).unwrap(),
            serde_json::to_value(expected.pop().unwrap()).unwrap()
        );
    }
    assert!(
        art::static_text(
            &source,
            None,
            crate::FocusDatabase::National,
            &wanted,
            &cancelled
        )
        .is_err()
    );
    // The event feed's art at each completion's hour is the event
    // presentation of that hour's frame.
    let requests = [
        (101, "GER", "t00"),
        (102, "GER", "a_static"),
        (101, "ENG", "a_static"),
    ];
    let before = reads();
    art::EVENT_ART_FREE.with(|free| free.set(true));
    let (art, _) = art::event_art(&source, &content, 1, &requests, &cancelled)
        .unwrap()
        .unwrap();
    // Its frames are read with the art context free, so other requests present meanwhile.
    assert!(reads() > before, "the dynamic completion reads its hour's frame");
    assert!(art::EVENT_ART_FREE.with(std::cell::Cell::get));
    let mut events = artwork::Context::load(Arc::clone(&catalogue), national).unwrap();
    for ((hour, tag, token), actual) in requests.iter().zip(&art) {
        let expected = events
            .focuses_at(&[(*token, *tag)], &frames[(*hour - 100) as usize])
            .pop()
            .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
}

/// `focus-art` asks for distinct tokens in order, at most a page.
#[test]
fn focus_art_params_are_one_ordered_page_of_distinct_tokens() {
    let params = |tokens: Vec<&str>| FocusArtParams {
        branch_id: "1".into(),
        tag: "GER".into(),
        hour: 100,
        database: crate::FocusDatabase::National,
        tokens: tokens.into_iter().map(str::to_owned).collect(),
        mode: FocusArtMode::Display,
    };
    assert!(FocusArt::validate(&params(vec!["a", "b"])).is_ok());
    for tokens in [
        vec![],
        vec!["b", "a"],
        vec!["a", "a"],
        vec![""],
        vec!["a\0"],
    ] {
        assert!(FocusArt::validate(&params(tokens)).is_err());
    }
    let page: Vec<String> = (0..=PAGE_ROWS)
        .map(|index| format!("t{index:04}"))
        .collect();
    let mut over = params(Vec::new());
    over.tokens = page;
    assert!(FocusArt::validate(&over).is_err());
}

/// `focus-art` and `focus-static-text` look every token up in the database
/// the request names. One id names a different focus in each: the art kept
/// for the national focus never answers for the continuous one, and a
/// refused continuous file leaves the national focus answered.
#[test]
fn focus_art_and_static_text_keep_each_database_apart() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::FocusDatabase::{Continuous, National};
    for refused in [false, true] {
        let mut files = vec![
            (
                "common/national_focus/databases.txt",
                "focus_tree={focus={id=shared text=national_title}}",
            ),
            (
                "common/continuous_focus/databases.txt",
                "continuous_focus_palette={focus={id=shared text=continuous_title}}",
            ),
            (
                "localisation/databases_l_english.yml",
                "\u{feff}l_english:\n national_title:0 \"National title\"\n national_title_desc:0 \"National details\"\n continuous_title:0 \"Continuous title\"\n continuous_title_desc:0 \"Continuous details\"\n",
            ),
        ];
        if refused {
            // A later continuous file the reader refuses could redefine any continuous focus.
            files.push((
                "common/continuous_focus/zz_refused.txt",
                "continuous_focus_palette={focus={text=continuous_title}}",
            ));
        }
        let (catalogue, folder) = crate::catalogue::tests::folder_catalogue(&files);
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![crate::test_fixtures::source(1, 100, Some(1))])],
        )
        .unwrap();
        let cancelled = AtomicBool::new(false);
        let source = Source::from_replay(
            crate::SourceKey {
                source_id: uuid::Uuid::new_v4().to_string(),
                revision: "1".into(),
            },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &cancelled,
        )
        .unwrap();
        let content = ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: folder.0.clone(),
                user_dir: folder.0.join("user"),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        crate::catalogue::set_test_catalogue(&source, &content, Arc::new(catalogue));
        let context = LoadContext {
            content: Some(content.clone()),
        };
        let art = |database| {
            let reply = FocusArt::load(
                &source,
                &FocusArtParams {
                    branch_id: "1".into(),
                    tag: "GER".into(),
                    hour: 100,
                    database,
                    tokens: vec!["shared".into()],
                    mode: FocusArtMode::Text,
                },
                &context,
                &cancelled,
            )
            .unwrap();
            assert!(reply.failures.is_empty(), "{:?}", reply.failures);
            reply.art.into_iter().next().unwrap()
        };
        // The national art is kept first, where a key without its database would answer again.
        for _ in 0..2 {
            let national = art(National);
            assert_eq!(national.artwork.name.as_deref(), Some("National title"));
            assert_eq!(
                national.artwork.description.as_deref(),
                Some("National details")
            );
            assert_eq!(national.reason, None);
            let continuous = art(Continuous);
            if refused {
                assert_eq!(continuous.artwork.name, None);
                assert_eq!(continuous.artwork.description, None);
                let reason = continuous.reason.unwrap();
                assert!(
                    reason.contains("common/continuous_focus/zz_refused.txt"),
                    "{reason}"
                );
            } else {
                assert_eq!(continuous.artwork.name.as_deref(), Some("Continuous title"));
                assert_eq!(
                    continuous.artwork.description.as_deref(),
                    Some("Continuous details")
                );
                assert_eq!(continuous.reason, None);
            }
        }
        let text = |database| {
            art::static_text(
                &source,
                Some(&content),
                database,
                &["shared".to_owned()],
                &cancelled,
            )
            .unwrap()
            .remove(0)
        };
        let national = text(National);
        assert_eq!(national.name.as_deref(), Some("National title"));
        assert_eq!(national.description.as_deref(), Some("National details"));
        let continuous = text(Continuous);
        let expected = |text: &str| (!refused).then(|| text.to_owned());
        assert_eq!(continuous.name, expected("Continuous title"));
        assert_eq!(continuous.description, expected("Continuous details"));
        // The event feed's completions are national focuses.
        let (events, _) = art::event_art(&source, &content, 1, &[(100, "GER", "shared")], &cancelled)
            .unwrap()
            .unwrap();
        assert_eq!(events[0].0.name.as_deref(), Some("National title"));
        assert_eq!(
            events[0].0.description.as_deref(),
            Some("National details")
        );
    }
}

/// A continuation of another schema is refused, so preparation rebuilds it.
#[test]
fn only_changed_prepared_focus_schema_is_refused() {
    let saved = serde_json::to_value(FocusIndex::default()).unwrap();
    assert_eq!(saved["schema"], STATE_SCHEMA);
    serde_json::from_value::<FocusIndex>(saved.clone()).unwrap();
    let mut old = saved;
    old["schema"] = 5.into();
    assert!(serde_json::from_value::<FocusIndex>(old).is_err());
}
