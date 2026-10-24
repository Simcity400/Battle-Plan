//! All-country publication, cancellation and source reopening.
use super::*;
use crate::{
    products::{
        LoadContext, Product,
        production_lines::{self, ProductionLines, ProductionLinesParams},
    },
    test_fixtures::{
        CAMPAIGN, PRODUCTION_HOUR as FIRST, continuation_records, production_frame, records,
    },
};

/// Eight one-hour blocks, each a read of Germany's two lines.
fn recording() -> Replay {
    let frame = |hour: i32| {
        let units = i64::from(hour - FIRST);
        production_frame(
            hour,
            true,
            true,
            9001,
            &[(5, 70, 8, 0), (6, 71, 2, 0)],
            Some(&[(5, 70, units, true, false), (6, 71, units, true, false)]),
        )
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(FIRST)])]).unwrap();
    for hour in FIRST + 1..FIRST + 8 {
        replay = replay
            .append(&[continuation_records(&replay, 1, vec![frame(hour)])])
            .unwrap();
    }
    replay
}

#[test]
fn all_recorded_countries_are_prepared_before_requests_and_answer_the_next_open() {
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("production-kept");
    let flag = AtomicBool::new(false);
    let exact = |source: &Source| {
        ProductionLines::load(
            source,
            &ProductionLinesParams {
                branch_id: "1".into(),
                tag: "GER".into(),
                exact: Some(true),
            },
            &LoadContext { content: None },
            &flag,
        )
        .unwrap()
    };
    let reference = fixture.source("production-reference");
    production_lines::prepared::fixtures::prepare(&reference, &LoadContext { content: None })
        .unwrap();
    let read = exact(&reference);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    assert!(
        !production_lines::saved_current(
            &store,
            &key,
            &source,
            &LoadContext { content: None },
            1,
            "GER"
        )
        .unwrap(),
        "the request keeps nothing"
    );
    run(&source, None, &flag, |_| {}).unwrap();
    assert!(
        production_lines::saved_current(
            &store,
            &key,
            &source,
            &LoadContext { content: None },
            1,
            "GER"
        )
        .unwrap()
    );
    // The next open answers from the kept scan without reading the recording.
    let again = fixture.source("production-kept-again");
    crate::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
    let reply = exact(&again);
    assert!(
        crate::blocks::SCANNED_BLOCKS.with_borrow(Vec::is_empty),
        "the kept scan answers"
    );
    assert_eq!(reply.lines, read.lines);
    assert_eq!(reply.reads, read.reads);
}

/// Forward completion must not hide unfinished all-country preparation.
#[test]
fn cancelled_side_scans_resume_after_every_forward_publisher_finished() {
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;
    let replay = recording();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("production-outstanding");
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let exact = |source: &Source, tag: &str| {
        ProductionLines::load(
            source,
            &ProductionLinesParams {
                branch_id: "1".into(),
                tag: tag.into(),
                exact: Some(true),
            },
            &context,
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reference = fixture.source("production-cancel-reference");
    production_lines::prepared::fixtures::prepare(&reference, &context).unwrap();
    let expected: Vec<_> = ["GER", "ITA"]
        .into_iter()
        .map(|tag| exact(&reference, tag))
        .collect();
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let finished = Arc::new((Mutex::new(false), Condvar::new()));
    let signal = Arc::clone(&finished);
    let waited = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&waited);
    let result = super::super::production_scans::with_test_stall(
        move || {
            let (done, changed) = &*signal;
            let (guard, _) = changed
                .wait_timeout_while(done.lock().unwrap(), Duration::from_secs(10), |done| !*done)
                .unwrap();
            observed.store(*guard, Ordering::Release);
        },
        || {
            run(&source, None, &flag, |_| {
                let Some((saved, _)) = checkpoint(&store, &key, None).unwrap() else {
                    return;
                };
                if saved.forward_complete(replay.index.len()) {
                    flag.store(true, Ordering::Release);
                    let (done, changed) = &*finished;
                    *done.lock().unwrap() = true;
                    changed.notify_all();
                }
            })
        },
    );
    assert_eq!(
        result.err().map(|error| error.kind()),
        Some(io::ErrorKind::Interrupted)
    );
    assert!(
        waited.load(Ordering::Acquire),
        "forward publishers finished with scans outstanding"
    );
    let (saved, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    assert!(saved.forward_complete(replay.index.len()));
    flag.store(false, Ordering::Release);
    for tag in ["GER", "ITA"] {
        assert!(!production_lines::saved_current(&store, &key, &source, &context, 1, tag).unwrap());
    }
    assert!(restore(&source, None, &flag).unwrap().is_none());
    let reopened = fixture.source("production-resumed");
    assert!(restore(&reopened, None, &flag).unwrap().is_none());
    let mut walked = Vec::new();
    run_checked(
        &reopened,
        None,
        &flag,
        |_| {},
        |next| {
            walked.push(next);
            Ok(())
        },
    )
    .unwrap();
    assert!(walked.is_empty(), "only unfinished side work resumes");
    assert!(restore(&reopened, None, &flag).unwrap().is_some());
    for tag in ["GER", "ITA"] {
        assert!(
            production_lines::saved_current(&store, &key, &reopened, &context, 1, tag).unwrap(),
            "{tag}: outstanding scan was scheduled and persisted"
        );
    }
    let again = fixture.source("production-resumed-again");
    crate::blocks::SCANNED_BLOCKS.with_borrow_mut(Vec::clear);
    for (tag, expected) in ["GER", "ITA"].into_iter().zip(expected) {
        let reply = exact(&again, tag);
        assert_eq!(reply.lines, expected.lines);
        assert_eq!(reply.reads, expected.reads);
    }
    assert!(
        crate::blocks::SCANNED_BLOCKS.with_borrow(Vec::is_empty),
        "kept pages answer the next open"
    );
}

#[test]
fn new_branch_seed_tags_reschedule_completed_and_resumed_production_branches() {
    use replay_data::rows::{CountryLinkRow, CountryRow};
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let scans = |source: &Source| -> BTreeMap<_, _> {
        super::family_lanes::payloads(source, None)
            .into_iter()
            .filter(|((section, _), _)| section.starts_with("production-lines-scan-"))
            .collect()
    };
    for extend_old in [false, true] {
        let prefix = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![production_frame(
                    FIRST,
                    true,
                    true,
                    9001,
                    &[(5, 70, 8, 0)],
                    Some(&[]),
                )],
            )],
        )
        .unwrap();
        let mut fixture = Fixture::new(&prefix);
        let first = fixture.source("seed-prefix");
        run(&first, None, &flag, |_| {}).unwrap();
        let (store, old_key) = crate::prepared::files(&first, None).unwrap();
        assert!(
            production_lines::prepared::current(&store, &old_key, &first, &context, 1).unwrap()
        );
        let accepted = production_lines::prepared::directory(&store, &old_key, 1)
            .unwrap()
            .unwrap();
        let mut added = production_frame(FIRST + 2, true, true, 9001, &[(5, 70, 8, 0)], Some(&[]));
        added.branch = 2;
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *added.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        rows.push(CountryRow {
            id: 10000,
            canonical_slot: 8,
            tag: b"NEW".to_vec(),
            ..Default::default()
        });
        let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *added.tables[TableId::CountryLinks.index()]
        else {
            unreachable!()
        };
        rows.push(CountryLinkRow {
            id: 10000,
            canonical_slot: 8,
        });
        let mut grown = prefix.append(&[records(2, vec![added])]).unwrap();
        if extend_old {
            grown = grown
                .append(&[continuation_records(
                    &grown,
                    1,
                    vec![production_frame(
                        FIRST + 1,
                        true,
                        true,
                        9001,
                        &[(5, 70, 8, 0)],
                        Some(&[(5, 70, 1, true, false)]),
                    )],
                )])
                .unwrap();
        }
        fixture.replace(&grown);
        let reopened = fixture.source("seed-grown");
        assert!(reopened.is_append_of(&first));
        assert!(
            reopened
                .metadata()
                .countries
                .iter()
                .any(|country| country.tag == "NEW")
        );
        assert!(
            !production_lines::prepared::current(&store, &old_key, &reopened, &context, 1).unwrap()
        );
        assert!(
            !production_lines::saved_current(&store, &old_key, &reopened, &context, 1, "NEW")
                .unwrap()
        );
        assert!(
            super::super::production_scans::missing(&reopened, &store, &old_key, &context)
                .unwrap()
                .contains(&1)
        );
        run(&reopened, None, &flag, |_| {}).unwrap();
        let (_, key) = crate::prepared::files(&reopened, None).unwrap();
        assert!(
            super::super::production_scans::missing(&reopened, &store, &key, &context)
                .unwrap()
                .is_empty()
        );
        for branch in [1, 2] {
            assert!(
                production_lines::prepared::current(&store, &key, &reopened, &context, branch)
                    .unwrap()
            );
            for tag in ["GER", "ITA", "NEW"] {
                assert!(
                    production_lines::saved_current(&store, &key, &reopened, &context, branch, tag)
                        .unwrap()
                );
            }
        }
        if !extend_old {
            let reconciled = production_lines::prepared::directory(&store, &key, 1)
                .unwrap()
                .unwrap();
            assert_eq!(reconciled.pages, accepted.pages);
            assert_eq!(reconciled.last_block, accepted.last_block);
            assert_eq!(
                serde_json::to_value(reconciled.coverage).unwrap(),
                serde_json::to_value(accepted.coverage).unwrap()
            );
        }
        let mut fresh = Fixture::new(&grown);
        let full = fresh.source("seed-fresh");
        run(&full, None, &flag, |_| {}).unwrap();
        let expected = scans(&full);
        assert_eq!(
            expected.len(),
            12,
            "three countries, two branches, scan plus meta"
        );
        assert_eq!(scans(&reopened), expected, "extend_old={extend_old}");
        run(&reopened, None, &flag, |_| {}).unwrap();
        assert_eq!(scans(&reopened), expected);
    }
}
