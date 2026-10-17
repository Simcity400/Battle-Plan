//! A production line scan a request read whole is kept by the source's next
//! preparation, never by the request, and answers the next source of the
//! recording exactly, reading none of it.
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
fn a_scan_a_request_read_whole_is_kept_by_preparation_and_answers_the_next_open() {
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
    let read = exact(&source);
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    assert!(production_lines::has_unkept(&source));
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
    assert!(!production_lines::has_unkept(&source));
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
    assert!(
        !production_lines::has_unkept(&again),
        "a kept scan held is not kept again"
    );
}

/// Forward completion must not hide unfinished scans, including non-player
/// requests whose queue belonged to the previous Source.
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
    let expected: Vec<_> = ["GER", "ITA"]
        .into_iter()
        .map(|tag| exact(&source, tag))
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
    assert!(
        production_lines::read_whole(&reopened).is_empty(),
        "new Source has no runtime requests"
    );
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
