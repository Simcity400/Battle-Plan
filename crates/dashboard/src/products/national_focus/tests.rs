use super::ordered;
use crate::test_fixtures::{CAMPAIGN, records, scale_frame, scale_tag, source};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CompletedFocusRow, CountryLinkRow, CountryRow, TableId, TableRows},
};
use std::sync::atomic::AtomicBool;

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

fn focus_countries_frame(
    branch: u64,
    hour: i32,
    germany: (u32, u32, &[&str]),
    france: &[&str],
) -> DecodedFrame {
    use replay::rows::{CompletedFocusRow, FocusRow};
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
    use replay::rows::{CompletedFocusRow, FocusRow};
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
    use replay::rows::{CompletedFocusRow, FocusRow};
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
    *frame.tables[TableId::CompletedFocuses.index()] = TableView::Unavailable {
        reason: "not captured".into(),
    };
    assert!(crate::focus_order(&read(vec![frame]), 1, 100, "GER", &cancel).is_err());
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
fn focus_budget_is_per_requested_country_not_per_recording() {
    use replay::rows::{CompletedFocusRow, FocusRow};
    const COUNTRIES: u32 = 5_000;
    // An hourly change for every country: 105,000 segments across the
    // recording, more than one scan may hold, but 21 for any one country.
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
    for index in [0, 2_777, COUNTRIES - 1] {
        let tag = String::from_utf8(scale_tag(index)).unwrap();
        let (history, progress_stopped) = cache.get(&replay, 1, &tag, &cancel).unwrap();
        assert_eq!(progress_stopped, None);
        assert_eq!(history.segments.len(), 21);
        assert_eq!(history.at(120).unwrap().completed[0].token, "focus_20");
    }
    // The same recording scanned for every country at once exceeds the scan
    // budget, which is why a request never scans other countries.
    assert!(crate::focus_branch(&replay, 1, None, &cancel).is_err());
}
