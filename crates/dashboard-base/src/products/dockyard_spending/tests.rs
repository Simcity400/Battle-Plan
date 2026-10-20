use super::*;
use crate::prepared::series::Run;
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

fn run(first_hour: i32, last_hour: i32, value: i128, break_before: bool) -> Run {
    Run::new(first_hour, last_hour, value, break_before)
}

fn design(new: Vec<Run>, combined: Vec<Run>) -> prepared::Design {
    prepared::Design {
        unresolved: false,
        new,
        combined,
        unproven: [Vec::new(), Vec::new()],
    }
}

#[test]
fn a_month_counts_new_and_refit_input_from_its_baseline_and_refuses_what_cannot_be_told() {
    use ProductionSpendingUnknown::{CountryChanged, SpendingUnavailable};
    let origins = [0];
    // From the observation's start: 10 new, then 4 refit and 6 more new.
    let ship = design(
        vec![run(0, 99, 10, true), run(100, 299, 16, false)],
        vec![
            run(0, 49, 10, true),
            run(50, 99, 14, false),
            run(100, 299, 20, false),
        ],
    );
    // The first month starts at the observation's zero.
    assert_eq!(prepared::design_change(&ship, 0, 99, &origins), Ok((10, 4)));
    assert_eq!(
        prepared::design_change(&ship, 100, 299, &origins),
        Ok((6, 0))
    );
    assert_eq!(
        prepared::design_change(&ship, 40, 120, &origins),
        Ok((6, 4))
    );
    // A refit-only design has no new series at all.
    let refitted = design(vec![], vec![run(150, 299, 7, false)]);
    assert_eq!(
        prepared::design_change(&refitted, 100, 199, &origins),
        Ok((0, 7))
    );
    assert_eq!(
        prepared::design_change(&refitted, 200, 299, &origins),
        Ok((0, 0))
    );
    // Before a design's first input nothing is spent on it.
    assert_eq!(
        prepared::design_change(&refitted, 0, 99, &origins),
        Ok((0, 0))
    );
    // A series that ended is not known after it, nor across a gap.
    let ended = design(vec![], vec![run(0, 99, 5, true), run(150, 299, 9, false)]);
    assert_eq!(
        prepared::design_change(&ended, 100, 120, &origins),
        Err(SpendingUnavailable)
    );
    assert_eq!(
        prepared::design_change(&ended, 50, 200, &origins),
        Err(SpendingUnavailable)
    );
    // Its first value after the gap holds input from the gap too.
    assert_eq!(
        prepared::design_change(&ended, 150, 200, &origins),
        Err(SpendingUnavailable)
    );
    assert_eq!(
        prepared::design_change(&ended, 151, 200, &origins),
        Ok((0, 0))
    );
    // A new segment that is not the observation's start is another holder.
    let replaced = design(vec![], vec![run(0, 99, 5, true), run(100, 299, 2, true)]);
    assert_eq!(
        prepared::design_change(&replaced, 50, 150, &origins),
        Err(CountryChanged)
    );
    assert_eq!(
        prepared::design_change(&replaced, 150, 200, &origins),
        Ok((0, 0))
    );
    // Signed corrections stay exact.
    let corrected = design(
        vec![run(0, 99, 8, true), run(100, 199, 5, false)],
        vec![run(0, 99, 8, true), run(100, 199, 5, false)],
    );
    assert_eq!(
        prepared::design_change(&corrected, 100, 199, &origins),
        Ok((-3, 0))
    );
}

#[test]
fn a_month_is_known_only_where_the_equipment_source_was_read_and_untainted() {
    use crate::products::equipment::production::ScannedProduced;
    use ProductionSpendingUnknown::{NotRecorded, ObservationRestarted, RecordingGap};
    let mut scanned = ScannedProduced {
        reads: vec![100, 124, 148],
        observations: vec![100],
        ..Default::default()
    };
    let coverage = prepared::SourceCoverage::of(&scanned);
    // Before any read nothing is known, not zero: no series is no proof.
    assert_eq!(coverage.month(99), Err(NotRecorded));
    // The month the recording starts in counts from its first read.
    assert_eq!(coverage.month(100), Ok(()));
    assert_eq!(coverage.month(10_000), Ok(()));
    scanned.spending_tainted = std::collections::BTreeMap::from([(0, (500, RecordingGap))]);
    scanned.observations = vec![100, 900];
    let coverage = prepared::SourceCoverage::of(&scanned);
    assert_eq!(coverage.month(499), Ok(()));
    assert_eq!(coverage.month(500), Err(RecordingGap));
    scanned.spending_tainted.clear();
    let coverage = prepared::SourceCoverage::of(&scanned);
    assert_eq!(coverage.month(950), Err(ObservationRestarted));
    let none = prepared::SourceCoverage::of(&ScannedProduced::default());
    assert_eq!(none.month(1_000), Err(NotRecorded));
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

#[test]
fn a_month_is_purpose_uncertain_only_where_unproven_input_joined_it() {
    let origins = [0];
    // Unproven new input in the first month only; later months add proved
    // input of the same design.
    let mut ship = design(
        vec![run(0, 99, 10, true), run(100, 299, 16, false)],
        vec![run(0, 99, 10, true), run(100, 299, 16, false)],
    );
    ship.unproven = [vec![run(0, 299, 4, true)], Vec::new()];
    assert_eq!(prepared::unproven_in(&ship, 0, 99, &origins), Ok(true));
    assert_eq!(prepared::unproven_in(&ship, 100, 299, &origins), Ok(false));
    // Unproven refit input counts the same, from its own baseline.
    let mut refitted = design(vec![], vec![run(0, 99, 5, true), run(100, 299, 9, false)]);
    refitted.unproven = [Vec::new(), vec![run(150, 299, 3, false)]];
    assert_eq!(prepared::unproven_in(&refitted, 0, 99, &origins), Ok(false));
    assert_eq!(
        prepared::unproven_in(&refitted, 100, 199, &origins),
        Ok(true)
    );
    assert_eq!(
        prepared::unproven_in(&refitted, 200, 299, &origins),
        Ok(false)
    );
}
