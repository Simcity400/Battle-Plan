use super::*;

fn date(year: i32, days: i32, hour: i32) -> i32 {
    EPOCH + year * HOURS_PER_YEAR + days * 24 + hour
}

#[test]
fn period_bounds_follow_the_native_date_setters() {
    let year_one = PERIOD_DEFAULT_DATE;
    assert_eq!(normalize_to_year_one(year_one), year_one);
    assert_eq!(apply_bound(year_one, 0, 0), Ok(year_one));
    assert_eq!(apply_bound(year_one, 30, 11), Ok(year_one + 364 * 24));
    // The zero-based day setter does not clamp: 30 February is 3 March.
    assert_eq!(apply_bound(year_one, 30, 1), Ok(year_one + 61 * 24));
    // A bound already set moves within year one; the month keeps the day offset.
    let june = apply_bound(year_one, 14, 5).unwrap();
    assert_eq!(june, year_one + (151 + 14) * 24);
    assert_eq!(set_month_zero_based(june, 0), Ok(year_one + 14 * 24));
    assert_eq!(
        set_month_zero_based(june, 12),
        Ok(year_one + (365 + 14) * 24)
    );
    assert!(set_month_zero_based(june, 13).is_err());
}

#[test]
fn the_period_date_is_the_day_of_year_in_year_one() {
    let year_one = PERIOD_DEFAULT_DATE;
    assert_eq!(period_date(date(1936, 0, 0)), year_one);
    assert_eq!(period_date(date(1936, 0, 23)), year_one);
    assert_eq!(period_date(date(1943, 59, 5)), year_one + 59 * 24);
    assert_eq!(period_date(date(1940, 364, 12)), year_one + 364 * 24);
    let periods = [
        Period {
            start: year_one,
            end: year_one + 30 * 24,
            minimum: 1,
            maximum: 2,
        },
        Period {
            start: year_one + 20 * 24,
            end: year_one + 364 * 24,
            minimum: 3,
            maximum: 4,
        },
    ];
    // Inclusive bounds; the first period in template order wins.
    assert_eq!(find_period(&periods, date(1936, 30, 6)).unwrap().minimum, 1);
    assert_eq!(find_period(&periods, date(1936, 31, 0)).unwrap().minimum, 3);
    assert!(find_period(&periods[..1], date(1936, 31, 0)).is_none());
}
