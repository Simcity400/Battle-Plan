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
