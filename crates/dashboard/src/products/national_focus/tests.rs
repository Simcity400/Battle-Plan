use super::{COMPLETED_OWNER_INSPECTIONS, HeldFocusRead, completed_for_owner, ordered};
use crate::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag, source};
use replay::container::Replay;
use replay_data::{
    rows::CompletedFocusRow, rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::sync::atomic::AtomicBool;

const PREPARATION_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(30);

struct PreparationFinished<'a>(&'a AtomicBool);

impl Drop for PreparationFinished<'_> {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

#[test]
fn cold_preparation_runs_while_the_history_scan_is_still_active() {
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, resumed) = std::sync::mpsc::channel();
    let prepared = AtomicBool::new(false);
    let prepared = &prepared;
    let result = super::prepare_while_scanning(
        &AtomicBool::new(false),
        || {
            // The serial scan-before-preparation path cannot reach this signal.
            waiting.recv_timeout(PREPARATION_HANG_GUARD).unwrap();
            assert!(!prepared.load(std::sync::atomic::Ordering::Acquire));
            release.send(()).unwrap();
            Ok(42)
        },
        move || {
            let _finished = PreparationFinished(prepared);
            entered.send(()).unwrap();
            resumed.recv_timeout(PREPARATION_HANG_GUARD).unwrap();
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(result, 42);
    assert!(prepared.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn cold_preparation_joins_and_preserves_history_error_over_content_error_or_panic() {
    for preparation_panics in [false, true] {
        let (entered, waiting) = std::sync::mpsc::channel();
        let (release, resumed) = std::sync::mpsc::channel::<()>();
        let prepared = AtomicBool::new(false);
        let prepared = &prepared;
        let error = super::prepare_while_scanning(
            &AtomicBool::new(false),
            || -> std::io::Result<()> {
                waiting.recv_timeout(PREPARATION_HANG_GUARD).unwrap();
                drop(release);
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "synthetic history refusal",
                ))
            },
            move || {
                let _finished = PreparationFinished(prepared);
                entered.send(()).unwrap();
                assert_eq!(
                    resumed.recv_timeout(PREPARATION_HANG_GUARD),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                );
                if preparation_panics {
                    panic!("synthetic content panic");
                }
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "synthetic content refusal",
                ))
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "synthetic history refusal");
        assert!(
            prepared.load(std::sync::atomic::Ordering::Acquire),
            "content preparation must be fully joined before returning a scan refusal"
        );
    }
}

#[test]
fn cold_preparation_preserves_content_error_or_panic_after_a_successful_scan() {
    let error = super::prepare_while_scanning(
        &AtomicBool::new(false),
        || Ok(42),
        || {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "synthetic content refusal",
            ))
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(error.to_string(), "synthetic content refusal");
    let panic = std::panic::catch_unwind(|| {
        super::prepare_while_scanning(
            &AtomicBool::new(false),
            || Ok(42),
            || {
                panic!("synthetic content panic");
            },
        )
    })
    .unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"synthetic content panic")
    );
}

#[test]
fn cold_preparation_joins_before_unwinding_a_history_panic() {
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, resumed) = std::sync::mpsc::channel::<()>();
    let prepared = AtomicBool::new(false);
    let prepared = &prepared;
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        super::prepare_while_scanning(
            &AtomicBool::new(false),
            || -> std::io::Result<()> {
                waiting.recv_timeout(PREPARATION_HANG_GUARD).unwrap();
                drop(release);
                panic!("synthetic history panic");
            },
            move || {
                let _finished = PreparationFinished(prepared);
                entered.send(()).unwrap();
                assert_eq!(
                    resumed.recv_timeout(PREPARATION_HANG_GUARD),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                );
                Ok(())
            },
        )
    }))
    .unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"synthetic history panic")
    );
    assert!(prepared.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn cold_preparation_refuses_pre_cancelled_work_and_joins_mid_scan_cancellation() {
    let error = super::prepare_while_scanning::<()>(
        &AtomicBool::new(true),
        || panic!("pre-cancelled history must not scan"),
        || panic!("pre-cancelled content must not prepare"),
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(error.to_string(), "Focus request cancelled");

    let cancelled = AtomicBool::new(false);
    let (entered, waiting) = std::sync::mpsc::channel();
    let (release, resumed) = std::sync::mpsc::channel::<()>();
    let prepared = AtomicBool::new(false);
    let prepared = &prepared;
    let error = super::prepare_while_scanning(
        &cancelled,
        || -> std::io::Result<()> {
            waiting.recv_timeout(PREPARATION_HANG_GUARD).unwrap();
            cancelled.store(true, std::sync::atomic::Ordering::Release);
            drop(release);
            Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Focus request cancelled",
            ))
        },
        move || {
            let _finished = PreparationFinished(prepared);
            entered.send(()).unwrap();
            assert_eq!(
                resumed.recv_timeout(PREPARATION_HANG_GUARD),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
            );
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(error.to_string(), "Focus request cancelled");
    assert!(prepared.load(std::sync::atomic::Ordering::Acquire));
}

#[test]
fn preserves_recorded_order_and_repeats() {
    let rows = [
        CompletedFocusRow {
            country: 42,
            ordinal: 2,
            token: b"a".to_vec(),
        },
        CompletedFocusRow {
            country: 42,
            ordinal: 0,
            token: b"a".to_vec(),
        },
        CompletedFocusRow {
            country: 42,
            ordinal: 1,
            token: b"z".to_vec(),
        },
    ];
    let result = ordered(&rows.iter().collect::<Vec<_>>(), 3).unwrap();
    assert_eq!(
        result.iter().map(|r| r.token.as_str()).collect::<Vec<_>>(),
        ["a", "z", "a"]
    );
    assert!(ordered(&[&rows[0]], 1).is_err());
    assert!(ordered(&[], 1).is_err());
    assert!(ordered(&[], -1).is_err());
    assert!(ordered(&[], 0).unwrap().is_empty());
}

#[test]
fn completed_owner_range_preserves_boundaries_and_absent_owners() {
    let rows: Vec<_> = [
        (1, "first"),
        (1, "repeat"),
        (4, "middle"),
        (u32::MAX, "last"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (country, token))| CompletedFocusRow {
        country,
        ordinal: if index == 1 { 1 } else { 0 },
        token: token.as_bytes().to_vec(),
    })
    .collect();
    assert_eq!(completed_for_owner(&rows, 1), &rows[..2]);
    assert_eq!(completed_for_owner(&rows, 4), &rows[2..3]);
    assert_eq!(completed_for_owner(&rows, u32::MAX), &rows[3..]);
    for owner in [0, 2, 3, 5, u32::MAX - 1] {
        assert!(completed_for_owner(&rows, owner).is_empty());
    }
    assert!(completed_for_owner(&[], 1).is_empty());
}

#[test]
fn selected_focus_history_inspects_logarithmic_completed_owner_keys() {
    use replay_data::rows::FocusRow;
    const COUNTRIES: u32 = 200;
    const COMPLETIONS: u32 = 40;
    let selected = COUNTRIES / 2;
    let mut frame = scale_frame(1, 100, COUNTRIES);
    *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
        (0..COUNTRIES)
            .map(|index| FocusRow {
                country: 100_000 + index,
                completed_count: COMPLETIONS as i32,
                ..FocusRow::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(
            (0..COUNTRIES)
                .flat_map(|index| {
                    (0..COMPLETIONS).map(move |ordinal| CompletedFocusRow {
                        country: 100_000 + index,
                        ordinal,
                        token: format!("focus_{index}_{ordinal}").into_bytes(),
                    })
                })
                .collect(),
        ));
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    let tag = String::from_utf8(scale_tag(selected)).unwrap();
    COMPLETED_OWNER_INSPECTIONS.with(|count| count.set(0));
    let history = crate::focus_history(&replay, 1, &tag, &AtomicBool::new(false)).unwrap();
    let inspections = COMPLETED_OWNER_INSPECTIONS.with(|count| count.replace(0));
    // Count the real history path's owner-key reads rather than elapsed time.
    // Inspecting every recorded completion would visit 8,000 owner keys.
    assert!(
        (1..128).contains(&inspections),
        "selected owner extraction inspected {inspections} keys"
    );
    let order = history.at(100).unwrap();
    assert_eq!(order.completed.len(), COMPLETIONS as usize);
    assert_eq!(order.completed[0].token, format!("focus_{selected}_0"));
    assert_eq!(
        order.completed.last().unwrap().token,
        format!("focus_{selected}_{}", COMPLETIONS - 1)
    );
}

#[test]
fn held_focus_hours_validate_once_across_hourly_balances() {
    use replay_data::{rows::FocusRow, rows::tables};
    const COUNTRIES: u32 = 24;
    const HOURS: i32 = 24;
    let selected = COUNTRIES / 2;
    let mut first = scale_frame(1, 100, COUNTRIES);
    *first.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
        (0..COUNTRIES)
            .map(|index| FocusRow {
                country: 100_000 + index,
                progress: 12_345,
                completed_count: if index == selected { 1 } else { 0 },
                ..FocusRow::default()
            })
            .collect(),
    ));
    *first.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
            country: 100_000 + selected,
            ordinal: 0,
            token: b"held_completion".to_vec(),
        }]));
    COMPLETED_OWNER_INSPECTIONS.with(|count| count.set(0));
    completed_for_owner(
        first.complete::<tables::CompletedFocuses>().unwrap(),
        100_000 + selected,
    );
    let one_read = COMPLETED_OWNER_INSPECTIONS.with(|count| count.replace(0));
    let frames = (0..HOURS)
        .map(|step| {
            let mut frame = first.clone();
            frame.hour += step;
            if let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                countries[selected as usize].political_power_raw = Some(i64::from(step));
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let tag = String::from_utf8(scale_tag(selected)).unwrap();
    let history = crate::focus_history(&replay, 1, &tag, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        COMPLETED_OWNER_INSPECTIONS.with(|count| count.replace(0)),
        one_read,
        "held hourly tables must not rebuild or revalidate completed orders"
    );
    assert_eq!(history.segments.len(), 1);
    assert_eq!(history.segments[0].first_hour, 100);
    assert_eq!(history.segments[0].last_hour, 100 + HOURS - 1);
    assert_eq!(history.progress.len(), 1);
    assert_eq!(history.progress[0].first_hour, 100);
    assert_eq!(history.progress[0].last_hour, 100 + HOURS - 1);
    assert_eq!(history.progress[0].progress_raw, "12345");
    assert_eq!(
        history.at(100 + HOURS - 1).unwrap().completed[0].token,
        "held_completion"
    );
    assert!(history.at(99).is_err());
    assert!(history.at(100 + HOURS).is_err());
}

#[test]
fn held_focus_read_invalidates_dependencies_identity_gaps_and_duplicate_tags() {
    use replay_data::rows::tables;
    let first = focus_countries_frame(1, 100, (9001, 7, &["a"]), &[]);
    let mut read = HeldFocusRead::default();
    read.begin(&first, first.complete::<tables::Countries>().unwrap());
    read.accept("GER");
    let mut next = first.clone();
    next.hour += 1;
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *next.tables[TableId::Countries.index()]
    {
        countries[0].political_power_raw = Some(123);
    }
    assert!(read.can_extend(&next, next.complete::<tables::Countries>().unwrap()));
    for table in [
        TableId::CountryLinks,
        TableId::Focus,
        TableId::CompletedFocuses,
    ] {
        let mut changed = next.clone();
        changed.tables[table.index()] = (*changed.tables[table.index()]).clone().into();
        assert!(!read.can_extend(&changed, changed.complete::<tables::Countries>().unwrap()));
    }
    for key in 0..3 {
        let mut changed = next.clone();
        if let TableView::Complete(TableRows::Countries(countries)) =
            &mut *changed.tables[TableId::Countries.index()]
        {
            match key {
                0 => countries[0].id += 1,
                1 => countries[0].canonical_slot += 1,
                _ => countries[0].tag = b"renamed".to_vec(),
            }
        }
        assert!(!read.can_extend(&changed, changed.complete::<tables::Countries>().unwrap()));
    }
    let mut gap = next.clone();
    gap.hour += 1;
    assert!(!read.can_extend(&gap, gap.complete::<tables::Countries>().unwrap()));
    read.accept("GER");
    assert!(!read.can_extend(&next, next.complete::<tables::Countries>().unwrap()));
}

#[test]
fn held_focus_invalid_utf8_keeps_the_original_hourly_segments() {
    for field in 0..3 {
        let mut first = focus_countries_frame(1, 100, (9001, 7, &["completion"]), &[]);
        match field {
            0 | 1 => {
                if let TableView::Complete(TableRows::Focus(focuses)) =
                    &mut *first.tables[TableId::Focus.index()]
                {
                    if field == 0 {
                        focuses[0].current = Some(vec![0xff]);
                    } else {
                        focuses[0].continuous = Some(vec![0xff]);
                    }
                }
            }
            _ => {
                if let TableView::Complete(TableRows::CompletedFocuses(completed)) =
                    &mut *first.tables[TableId::CompletedFocuses.index()]
                {
                    completed[0].token = vec![0xff];
                }
            }
        }
        let frames = (100..=102)
            .map(|hour| {
                let mut frame = first.clone();
                frame.hour = hour;
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let history = crate::focus_history(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
        assert_eq!(
            history
                .segments
                .iter()
                .map(|segment| (segment.first_hour, segment.last_hour))
                .collect::<Vec<_>>(),
            [(100, 100), (101, 101), (102, 102)],
            "invalid raw UTF-8 in field {field} must not acquire held-hour merging"
        );
        assert_eq!(history.progress.len(), 1);
        assert_eq!(history.progress[0].last_hour, 102);
        let order = history.at(102).unwrap();
        let lossy = String::from_utf8_lossy(&[0xff]);
        match field {
            0 => assert_eq!(order.current.as_deref(), Some(lossy.as_ref())),
            1 => assert_eq!(order.continuous.as_deref(), Some(lossy.as_ref())),
            _ => assert_eq!(order.completed[0].token, lossy),
        }
        assert_eq!(order.completed[0].completed_by_hour, None);
    }
}

fn focus_countries_frame(
    branch: u64,
    hour: i32,
    germany: (u32, u32, &[&str]),
    france: &[&str],
) -> DecodedFrame {
    use replay_data::{rows::CompletedFocusRow, rows::FocusRow};
    let mut frame = source(branch, hour, Some(1));
    let (german_id, german_slot, german_tokens) = germany;
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: german_id,
            canonical_slot: german_slot,
            tag: b"GER".to_vec(),
            ..CountryRow::default()
        },
        CountryRow {
            id: 10001,
            canonical_slot: 9,
            tag: b"FRA".to_vec(),
            ..CountryRow::default()
        },
    ]));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        [
            (11, german_slot),
            (44, 9),
            (german_id, german_slot),
            (10001, 9),
        ]
        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
        .to_vec(),
    ));
    *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
        [(11, german_tokens), (44, france)]
            .map(|(country, tokens)| FocusRow {
                country,
                completed_count: tokens.len() as i32,
                ..FocusRow::default()
            })
            .to_vec(),
    ));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(
            [(11, german_tokens), (44, france)]
                .into_iter()
                .flat_map(|(country, tokens)| {
                    tokens
                        .iter()
                        .enumerate()
                        .map(move |(ordinal, token)| CompletedFocusRow {
                            country,
                            ordinal: ordinal as u32,
                            token: token.as_bytes().to_vec(),
                        })
                })
                .collect(),
        ));
    frame
}

#[test]
fn selected_focus_history_uses_raw_owner_alias_without_changing_identity() {
    use replay_data::rows::FocusRow;
    let frames = [(100, 11, &['a', 'b'][..]), (101, 30, &['a', 'b', 'c'][..])]
        .into_iter()
        .map(|(hour, owner, tokens)| {
            let mut frame = focus_countries_frame(1, hour, (9001, 7, &[]), &[]);
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(
                    [(7, 7), (11, 7), (30, 7), (44, 9), (9001, 7), (10001, 9)]
                        .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                        .to_vec(),
                ));
            *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(vec![
                FocusRow {
                    country: owner,
                    current: Some(b"current".to_vec()),
                    continuous: Some(b"continuous".to_vec()),
                    progress: hour as i64,
                    paused: 1,
                    completed_count: tokens.len() as i32,
                    ..FocusRow::default()
                },
                FocusRow {
                    country: 44,
                    ..FocusRow::default()
                },
            ]));
            let mut rows = vec![
                CompletedFocusRow {
                    country: 7,
                    ordinal: 0,
                    token: b"canonical_slot_decoy".to_vec(),
                },
                CompletedFocusRow {
                    country: 9001,
                    ordinal: 0,
                    token: b"country_id_decoy".to_vec(),
                },
            ];
            rows.extend(
                tokens
                    .iter()
                    .enumerate()
                    .map(|(ordinal, token)| CompletedFocusRow {
                        country: owner,
                        ordinal: ordinal as u32,
                        token: token.to_string().into_bytes(),
                    }),
            );
            rows.sort_by_key(|row| (row.country, row.ordinal));
            *frame.tables[TableId::CompletedFocuses.index()] =
                TableView::Complete(TableRows::CompletedFocuses(rows));
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let history = crate::focus_history(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let order = history.at(101).unwrap();
    assert_eq!(order.current.as_deref(), Some("current"));
    assert_eq!(order.continuous.as_deref(), Some("continuous"));
    assert!(order.paused);
    assert_eq!(
        order
            .completed
            .iter()
            .map(|row| (row.token.as_str(), row.completed_by_hour))
            .collect::<Vec<_>>(),
        [("a", None), ("b", None), ("c", Some(101))]
    );
    assert_eq!(
        history
            .progress
            .iter()
            .map(|row| (row.first_hour, row.progress_raw.as_str()))
            .collect::<Vec<_>>(),
        [(100, "100"), (101, "101")]
    );
}

#[test]
fn selected_segment_observer_preserves_history_through_gaps_resets_and_identity_changes() {
    let mut missing = focus_countries_frame(1, 102, (9001, 7, &["old"]), &[]);
    *missing.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable {
        reason: "synthetic missing completed order".into(),
    };
    let mut invalid = focus_countries_frame(1, 108, (9002, 7, &["reset"]), &[]);
    if let TableView::Complete(TableRows::Focus(rows)) =
        &mut *invalid.tables[TableId::Focus.index()]
    {
        rows[0].completed_count = 2;
    }
    let frames = vec![
        focus_countries_frame(1, 100, (9001, 7, &["old"]), &[]),
        focus_countries_frame(1, 101, (9001, 7, &["old"]), &[]),
        missing,
        focus_countries_frame(1, 103, (9001, 7, &["old"]), &[]),
        focus_countries_frame(1, 104, (9001, 7, &["old", "new"]), &[]),
        focus_countries_frame(1, 105, (9001, 7, &["reset"]), &[]),
        focus_countries_frame(1, 106, (9001, 7, &["reset"]), &[]),
        focus_countries_frame(1, 107, (9002, 7, &["reset"]), &[]),
        invalid,
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let cancelled = AtomicBool::new(false);
    let baseline = super::focus_branch(&replay, 1, Some("GER"), &cancelled).unwrap();
    let mut observed = Vec::new();
    let with_observer = super::focus_branch_observing(
        &replay,
        1,
        Some("GER"),
        &cancelled,
        super::PROGRESS_BUDGET_BYTES,
        &mut |frame| {
            observed.push(frame.hour);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(observed, [100, 103, 104, 105, 107]);
    assert_eq!(with_observer.countries, baseline.countries);
    assert_eq!(with_observer.retained_bytes, baseline.retained_bytes);
    assert_eq!(with_observer.progress_stopped, baseline.progress_stopped);
    let history = &with_observer.countries["GER"];
    assert_eq!(
        history.at(104).unwrap().completed[1].completed_by_hour,
        Some(104)
    );
    assert_eq!(
        history.at(105).unwrap().completed[0].completed_by_hour,
        None
    );
    assert_eq!(
        history.at(107).unwrap().completed[0].completed_by_hour,
        None
    );
    assert!(history.at(102).is_err());
    assert!(history.at(108).is_err());
    observed.clear();
    super::focus_branch_observing(
        &replay,
        1,
        None,
        &cancelled,
        super::PROGRESS_BUDGET_BYTES,
        &mut |frame| {
            observed.push(frame.hour);
            Ok(())
        },
    )
    .unwrap();
    assert!(
        observed.is_empty(),
        "whole-country scans do not seed the selected-country presentation cache"
    );
}

#[test]
fn cancelled_segment_observation_does_not_publish_a_partial_history() {
    use std::sync::atomic::Ordering;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                focus_countries_frame(1, 100, (9001, 7, &["old"]), &[]),
                focus_countries_frame(1, 101, (9001, 7, &["old", "new"]), &[]),
            ],
        )],
    )
    .unwrap();
    let cancelled = AtomicBool::new(false);
    let mut cache = super::FocusCache::default();
    let mut observed = Vec::new();
    let error = cache
        .get_observing(&replay, 1, "GER", &cancelled, &mut |frame| {
            observed.push(frame.hour);
            cancelled.store(true, Ordering::Release);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::Interrupted);
    assert_eq!(observed, [100]);
    assert!(cache.countries.is_empty());
    cancelled.store(false, Ordering::Release);
    let (history, _) = cache.get(&replay, 1, "GER", &cancelled).unwrap();
    assert_eq!(
        history.at(101).unwrap().completed[1].completed_by_hour,
        Some(101)
    );
}

#[test]
fn selected_focus_history_requires_matching_completion_count_and_ordinals() {
    let row = |ordinal, token: &[u8]| CompletedFocusRow {
        country: 11,
        ordinal,
        token: token.to_vec(),
    };
    for (count, rows) in [
        (-1, vec![]),
        (1, vec![]),
        (0, vec![row(0, b"a")]),
        (1, vec![row(1, b"a")]),
        (2, vec![row(0, b"a"), row(2, b"b")]),
        (1, vec![row(0, b"")]),
        (1, vec![row(0, &[b'a'; 513])]),
    ] {
        let mut frame = focus_countries_frame(1, 100, (9001, 7, &[]), &["unrelated"]);
        if let TableView::Complete(TableRows::Focus(focuses)) =
            &mut *frame.tables[TableId::Focus.index()]
        {
            focuses[0].completed_count = count;
        }
        *frame.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(rows));
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        assert!(crate::focus_history(&replay, 1, "GER", &AtomicBool::new(false)).is_err());
    }
    // A complete zero count stays known even when other owners have rows.
    let frame = focus_countries_frame(1, 100, (9001, 7, &[]), &["unrelated"]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
    assert!(
        crate::focus_history(&replay, 1, "GER", &AtomicBool::new(false))
            .unwrap()
            .at(100)
            .unwrap()
            .completed
            .is_empty()
    );
}

#[test]
fn focus_branch_cache_keeps_countries_branches_and_gaps_independent() {
    let first = focus_countries_frame(1, 100, (9001, 7, &["a"]), &["f"]);
    let next = focus_countries_frame(1, 101, (9001, 7, &["a", "b"]), &["f"]);
    let mut unavailable = focus_countries_frame(1, 103, (9001, 7, &["a", "b"]), &["f"]);
    *unavailable.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable {
        reason: "synthetic missing focus sample".into(),
    };
    let last = focus_countries_frame(1, 104, (9001, 7, &["a", "b"]), &["f", "g"]);
    let alternate = focus_countries_frame(2, 100, (9001, 7, &["other_branch"]), &[]);
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[
            records(1, vec![first, next, unavailable, last]),
            records(2, vec![alternate]),
        ],
    )
    .unwrap();
    let cancel = AtomicBool::new(false);
    let projection = crate::focus_branch(&replay, 1, None, &cancel).unwrap();
    assert_eq!(projection.countries.len(), 2);
    let mut cache = crate::FocusCache::default();
    for tag in ["GER", "FRA", "GER", "FRA"] {
        let (history, progress_stopped) = cache.get(&replay, 1, tag, &cancel).unwrap();
        assert_eq!(progress_stopped, None);
        assert_eq!(history, projection.countries[tag]);
        assert!(history.at(102).is_err());
        assert!(history.at(103).is_err());
        assert_eq!(history.at(100).unwrap().completed.len(), 1);
        assert_eq!(history.at(104).unwrap().completed.len(), 2);
    }
    assert_eq!(
        cache
            .get(&replay, 1, "FRA", &cancel)
            .unwrap()
            .0
            .at(104)
            .unwrap()
            .completed[1]
            .completed_by_hour,
        Some(104)
    );
    assert_eq!(
        cache
            .get(&replay, 2, "GER", &cancel)
            .unwrap()
            .0
            .at(100)
            .unwrap()
            .completed[0]
            .token,
        "other_branch"
    );
    assert!(
        cache
            .get(&replay, 1, "GER", &AtomicBool::new(true))
            .is_err()
    );
    assert_eq!(
        cache.get(&replay, 1, "GER", &cancel).unwrap().0,
        projection.countries["GER"]
    );
}

#[test]
fn focus_dates_reset_when_list_shrinks_or_country_identity_changes() {
    let snapshots = [
        (100, 9001, 7, &[][..]),
        (101, 9001, 7, &["a"][..]),
        (102, 9001, 7, &["a", "b"][..]),
        (103, 9001, 7, &["a"][..]),
        (104, 9001, 7, &["a", "b"][..]),
        (105, 9002, 7, &["a", "b"][..]),
        (106, 9002, 8, &["a", "b", "c"][..]),
        (107, 9002, 8, &["a", "b", "c", "d"][..]),
    ];
    let frames = snapshots
        .into_iter()
        .map(|(hour, id, slot, tokens)| focus_countries_frame(1, hour, (id, slot, tokens), &[]))
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let history = crate::focus_history(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let dates = |hour| {
        history
            .at(hour)
            .unwrap()
            .completed
            .into_iter()
            .map(|entry| entry.completed_by_hour)
            .collect::<Vec<_>>()
    };
    assert_eq!(dates(102), [Some(101), Some(102)]);
    assert_eq!(dates(103), [None]);
    assert_eq!(dates(104), [None, Some(104)]);
    assert_eq!(dates(105), [None, None]);
    assert_eq!(dates(106), [None, None, None]);
    assert_eq!(dates(107), [None, None, None, Some(107)]);
}

#[test]
fn focus_dates_preserve_unknown_start_repeats_gaps_and_as_of_history() {
    use replay_data::{rows::CompletedFocusRow, rows::FocusRow};
    let snapshot = |hour, tokens: &[&str]| {
        let mut frame = source(1, hour, Some(1));
        *frame.tables[TableId::Focus.index()] =
            TableView::Complete(TableRows::Focus(vec![FocusRow {
                country: 11,
                completed_count: tokens.len() as i32,
                ..FocusRow::default()
            }]));
        *frame.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(
                tokens
                    .iter()
                    .enumerate()
                    .map(|(ordinal, token)| CompletedFocusRow {
                        country: 11,
                        ordinal: ordinal as u32,
                        token: token.as_bytes().to_vec(),
                    })
                    .collect(),
            ));
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                snapshot(100, &["old"]),
                snapshot(101, &["old", "new"]),
                snapshot(104, &["old", "new", "new"]),
                snapshot(105, &["reset"]),
            ],
        )],
    )
    .unwrap();
    let history = crate::focus_history(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(history.at(100).unwrap().completed.len(), 1);
    assert_eq!(
        history.at(100).unwrap().completed[0].completed_by_hour,
        None
    );
    assert_eq!(
        history.at(101).unwrap().completed[1].completed_by_hour,
        Some(101)
    );
    assert!(history.at(102).is_err());
    let later = history.at(104).unwrap();
    assert_eq!(
        later
            .completed
            .iter()
            .map(|r| r.completed_by_hour)
            .collect::<Vec<_>>(),
        [None, Some(101), Some(104)]
    );
    assert_eq!(
        history.at(105).unwrap().completed[0].completed_by_hour,
        None
    );
    assert!(history.at(106).is_err());
}

#[test]
fn focus_order_uses_exact_branch_country_links_and_complete_tables() {
    use replay_data::{rows::CompletedFocusRow, rows::FocusRow};
    let mut frame = source(1, 100, Some(1));
    *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(vec![FocusRow {
        country: 11,
        current: Some(b"next".to_vec()),
        completed_count: 1,
        ..FocusRow::default()
    }]));
    *frame.tables[TableId::CompletedFocuses.index()] =
        TableView::Complete(TableRows::CompletedFocuses(vec![CompletedFocusRow {
            country: 11,
            ordinal: 0,
            token: b"first".to_vec(),
        }]));
    let read = |frames| Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let replay = read(vec![frame.clone()]);
    let cancel = AtomicBool::new(false);
    let result = crate::focus_order(&replay, 1, 100, "GER", &cancel).unwrap();
    assert_eq!(result.current.as_deref(), Some("next"));
    assert_eq!(result.completed[0].token, "first");
    assert!(crate::focus_order(&replay, 2, 100, "GER", &cancel).is_err());
    assert!(crate::focus_order(&replay, 1, 101, "GER", &cancel).is_err());
    assert!(crate::focus_order(&replay, 1, 100, "ABSENT", &cancel).is_err());
    assert!(crate::focus_order(&replay, 1, 100, "GER", &AtomicBool::new(true)).is_err());
    for incomplete in [
        TableView::Truncated {
            rows: TableRows::CompletedFocuses(vec![CompletedFocusRow {
                country: 11,
                ordinal: 0,
                token: b"first".to_vec(),
            }]),
            missing: 1,
        },
        TableView::Unavailable {
            reason: "not captured".into(),
        },
    ] {
        *frame.tables[TableId::CompletedFocuses.index()] = incomplete;
        COMPLETED_OWNER_INSPECTIONS.with(|count| count.set(0));
        assert!(crate::focus_order(&read(vec![frame.clone()]), 1, 100, "GER", &cancel).is_err());
        assert_eq!(
            COMPLETED_OWNER_INSPECTIONS.with(|count| count.replace(0)),
            0,
            "an incomplete table must never reach owner-range lookup"
        );
    }
}

#[test]
fn focus_progress_runs_compress_equal_hours_and_never_bridge_gaps() {
    let with_progress = |hour, progress| {
        let mut frame = focus_countries_frame(1, hour, (9001, 7, &["a"]), &["f"]);
        if let TableView::Complete(TableRows::Focus(rows)) =
            &mut *frame.tables[TableId::Focus.index()]
        {
            for row in rows.iter_mut() {
                row.progress = progress;
            }
        }
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                with_progress(100, 0),
                with_progress(101, 0),
                with_progress(102, 100_000),
                with_progress(104, 100_000),
            ],
        )],
    )
    .unwrap();
    let history = crate::focus_branch(&replay, 1, None, &AtomicBool::new(false))
        .unwrap()
        .countries["GER"]
        .clone();
    let runs: Vec<_> = history
        .progress
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.progress_raw.as_str()))
        .collect();
    // Hour 103 is missing, so the equal value at 104 starts a new run.
    assert_eq!(
        runs,
        [(100, 101, "0"), (102, 102, "100000"), (104, 104, "100000")]
    );
    // Two countries, so the budget holds two hours of runs. Past it, recording
    // stops rather than failing the product: the order history is complete and
    // the last kept run is never extended.
    let run = std::mem::size_of::<crate::FocusProgress>() + 20;
    let limited = crate::products::national_focus::focus_branch_within(
        &replay,
        1,
        None,
        &AtomicBool::new(false),
        4 * run,
    )
    .unwrap();
    assert_eq!(limited.progress_stopped, Some(104));
    let country = &limited.countries["GER"];
    assert_eq!(country.segments, history.segments);
    assert_eq!(
        country
            .progress
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(100, 101), (102, 102)]
    );
}

#[test]
fn focus_topic_cache_keeps_selected_countries_independent() {
    use replay_data::{rows::CompletedFocusRow, rows::FocusRow};
    const COUNTRIES: u32 = 3;
    // Hourly changes stay in each selected country's independent cache entry.
    let frames = (0..21)
        .map(|step| {
            let mut frame = scale_frame(1, 100 + step, COUNTRIES);
            *frame.tables[TableId::Focus.index()] = TableView::Complete(TableRows::Focus(
                (0..COUNTRIES)
                    .map(|index| FocusRow {
                        country: 100_000 + index,
                        completed_count: 1,
                        ..FocusRow::default()
                    })
                    .collect(),
            ));
            *frame.tables[TableId::CompletedFocuses.index()] =
                TableView::Complete(TableRows::CompletedFocuses(
                    (0..COUNTRIES)
                        .map(|index| CompletedFocusRow {
                            country: 100_000 + index,
                            ordinal: 0,
                            token: format!("focus_{step}").into_bytes(),
                        })
                        .collect(),
                ));
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let cancel = AtomicBool::new(false);
    let mut cache = crate::FocusCache::default();
    for index in [0, 1, COUNTRIES - 1] {
        let tag = String::from_utf8(scale_tag(index)).unwrap();
        let (history, progress_stopped) = cache.get(&replay, 1, &tag, &cancel).unwrap();
        assert_eq!(progress_stopped, None);
        assert_eq!(history.segments.len(), 21);
        assert_eq!(history.at(120).unwrap().completed[0].token, "focus_20");
    }
    let all = crate::focus_branch(&replay, 1, None, &cancel).unwrap();
    assert_eq!(all.countries.len(), COUNTRIES as usize);
    assert!(
        all.countries
            .values()
            .all(|history| history.segments.len() == 21)
    );
}
