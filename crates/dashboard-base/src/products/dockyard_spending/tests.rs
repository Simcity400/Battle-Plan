use super::*;
use engine_native::cdate::HOUR_EPOCH;

const DAY: i32 = 24;

#[test]
fn months_follow_the_engine_calendar_and_clip_to_the_window() {
    // 1 January of a year, then 1 February 31 days later.
    let year = HOUR_EPOCH + 1936 * 365 * DAY;
    assert_eq!(month_of(year), (year, year + 31 * DAY));
    assert_eq!(month_of(year + 31 * DAY - 1), (year, year + 31 * DAY));
    // February has 28 days in every year.
    let february = year + 31 * DAY;
    assert_eq!(month_of(february + 5), (february, february + 28 * DAY));
    // December ends the year.
    let december = year + 334 * DAY;
    assert_eq!(month_of(december + 30 * DAY), (december, year + 365 * DAY));
    assert_eq!(
        months(year + 10, february + 3),
        vec![(year + 10, february - 1), (february, february + 3)]
    );
    assert_eq!(months(year + 5, year + 5), vec![(year + 5, year + 5)]);
    assert!(months(year + 5, year + 4).is_empty());
}

#[test]
fn repairs_before_the_rate_origin_or_without_one_are_unknown_never_zero() {
    use ProductionSpendingUnknown::{NotRecorded, RecordingGap};
    let asked = std::cell::RefCell::new(Vec::new());
    let integrals = |spans: &[(i32, i32)]| {
        asked.borrow_mut().extend_from_slice(spans);
        Ok(spans.iter().map(|_| Ok(48)).collect())
    };
    assert_eq!(
        prepared::window_integrals(Some(100), [90, 120], integrals).unwrap(),
        [Err(NotRecorded), Ok(48)]
    );
    assert_eq!(
        *asked.borrow(),
        [(100, 120)],
        "only the span after the origin is read"
    );
    assert_eq!(
        prepared::window_integrals(None, [90, 120], |_| Ok(vec![Err(RecordingGap)])).unwrap(),
        [Err(NotRecorded); 2]
    );
}
