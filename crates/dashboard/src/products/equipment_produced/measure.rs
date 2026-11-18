//! Running totals made into what a window delivered: the total at an hour less the total a day,
//! seven days or a calendar month before, inside one holder's stretch and one observation. No
//! recording is read here.

use super::{ProducedMeasure, ProducedUnknown, value::Amount};
use crate::products::production_spending::runs::Piece;

/// The hour one calendar month before `hour`: the same day and hour of the month before, or that
/// month's last day when it is shorter (engine calendar: 365-day years from
/// `game_model::calendar::HOUR_EPOCH`). None before the calendar begins.
pub fn month_before(hour: i32) -> Option<i32> {
    use game_model::calendar::{HOUR_EPOCH, MONTH_LENGTHS};
    let elapsed = i64::from(hour) - i64::from(HOUR_EPOCH);
    if elapsed < 0 {
        return None;
    }
    let (days, hour_of_day) = (elapsed / 24, elapsed % 24);
    let (year, mut day) = (days / 365, days % 365);
    let mut month = 0;
    while day >= i64::from(MONTH_LENGTHS[month]) {
        day -= i64::from(MONTH_LENGTHS[month]);
        month += 1;
    }
    let (year, month) = if month == 0 { (year - 1, 11) } else { (year, month - 1) };
    if year < 0 {
        return None;
    }
    let day = day.min(i64::from(MONTH_LENGTHS[month]) - 1);
    let before: i64 = MONTH_LENGTHS[..month].iter().map(|days| i64::from(*days)).sum();
    i32::try_from(i64::from(HOUR_EPOCH) + (year * 365 + before + day) * 24 + hour_of_day).ok()
}

/// The hour a window ending at `hour` is measured from; none for a running total, or before the
/// calendar begins.
pub fn back(measure: ProducedMeasure, hour: i32) -> Option<i32> {
    match measure {
        ProducedMeasure::Total => None,
        ProducedMeasure::Day => hour.checked_sub(24),
        ProducedMeasure::Week => hour.checked_sub(7 * 24),
        ProducedMeasure::Month => month_before(hour),
    }
}

/// A total and the observation it counts in (the lines' and Rocket output's).
pub type Counted = (Amount, (Option<i32>, Option<i32>));

/// What `total` (covering `hour` and `before` in the same stretch of one holder) makes of a window:
/// the difference where both are known in the same observation.
pub fn difference(at: Counted, before: Option<Counted>) -> Amount {
    match before {
        None => Amount::Unknown(ProducedUnknown::WindowIncomplete),
        Some((_, observation)) if observation != at.1 => Amount::Unknown(ProducedUnknown::ObservationRestarted),
        Some((before, _)) => at.0.minus(before),
    }
}

fn find<T: Copy>(pieces: &[Piece<T>], hour: i32) -> Option<T> {
    let at = pieces.partition_point(|piece| piece.1 < hour);
    pieces.get(at).filter(|piece| piece.0 <= hour).map(|piece| piece.2)
}

/// The latest of the sorted `reads` at or before `hour`.
pub fn read_at(reads: &[i32], hour: i32) -> Option<i32> {
    reads.partition_point(|&read| read <= hour).checked_sub(1).map(|at| reads[at])
}

/// What a window ending at `hour` delivered, from `totals` and production's `reads` (both of one holder's stretch,
/// ascending): the total as of the latest read less the total as of the read exactly that long before it; none
/// where no read is that long before inside the stretch.
pub fn window_at(totals: &[Piece<Counted>], reads: &[i32], measure: ProducedMeasure, hour: i32) -> Amount {
    let first = totals.first().map_or(i32::MAX, |piece| piece.0);
    let Some(read) = read_at(reads, hour).filter(|&read| read >= first) else {
        return Amount::Unknown(ProducedUnknown::WindowIncomplete);
    };
    let Some(at) = find(totals, read) else {
        return Amount::Unknown(ProducedUnknown::NotRecorded);
    };
    let before = back(measure, read).filter(|&start| start >= first && reads.binary_search(&start).is_ok()).and_then(|start| find(totals, start));
    difference(at, before)
}

/// `totals` (pieces covering one holder's stretch, ascending) as `measure`'s values over the same hours: a running
/// total as it is; a window as of each read of production (`reads`, ascending) until the next, known where a read
/// exactly that long before lies in the stretch and counts in the same observation. Equal neighbours are joined.
pub fn measured(totals: &[Piece<Counted>], reads: &[i32], measure: ProducedMeasure) -> Vec<Piece<Amount>> {
    let (Some(first), Some(last)) = (totals.first().map(|piece| piece.0), totals.last().map(|piece| piece.1)) else {
        return Vec::new();
    };
    let mut out: Vec<Piece<Amount>> = Vec::with_capacity(totals.len());
    let mut push = |from: i32, to: i32, value: Amount| match out.last_mut() {
        Some(open) if open.2 == value && open.1.checked_add(1) == Some(from) => open.1 = to,
        _ => out.push((from, to, value)),
    };
    if measure == ProducedMeasure::Total {
        for &(from, to, (value, _)) in totals {
            push(from, to, value);
        }
        return out;
    }
    let shown: Vec<i32> = reads.iter().copied().filter(|&read| first <= read && read <= last).collect();
    if shown.first().is_none_or(|&read| read > first) {
        push(first, shown.first().map_or(last, |read| read - 1), Amount::Unknown(ProducedUnknown::WindowIncomplete));
    }
    for (index, &read) in shown.iter().enumerate() {
        let to = shown.get(index + 1).map_or(last, |next| next - 1);
        push(read, to, window_at(totals, &shown, measure, read));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProducedUnknown as Why;

    const OBSERVED: (Option<i32>, Option<i32>) = (Some(0), Some(0));

    fn known(value: i128) -> Counted {
        (Amount::Known(value), OBSERVED)
    }

    #[test]
    fn a_month_before_is_the_same_day_or_the_shorter_months_last() {
        use game_model::calendar::HOUR_EPOCH;
        let day = |days: i32, hour: i32| HOUR_EPOCH + days * 24 + hour;
        // 15 February of year 1, 06:00, is 15 January's.
        assert_eq!(month_before(day(365 + 31 + 14, 6)), Some(day(365 + 14, 6)));
        // 31 March is 28 February's, the shorter month's last day.
        assert_eq!(month_before(day(365 + 31 + 28 + 30, 0)), Some(day(365 + 31 + 27, 0)));
        // January reaches the year before; the calendar's first month has none.
        assert_eq!(month_before(day(365 + 3, 1)), Some(day(334 + 3, 1)));
        assert_eq!(month_before(day(3, 1)), None);
        assert_eq!(month_before(HOUR_EPOCH - 1), None);
    }

    #[test]
    fn a_day_is_the_total_at_a_read_less_the_total_read_a_day_before_inside_the_stretch() {
        // Reads at 0, 12, 36, 60 and 84 deliver 5, 7 and nothing; the read at 12 has none a day before it.
        let totals = [(0, 11, known(0)), (12, 35, known(5)), (36, 59, known(12)), (60, 100, known(12))];
        let reads = [0, 12, 36, 60, 84];
        assert_eq!(measured(&totals, &reads, ProducedMeasure::Day), vec![
            (0, 35, Amount::Unknown(Why::WindowIncomplete)),
            (36, 59, Amount::Known(7)),
            (60, 100, Amount::Known(0)),
        ]);
        assert_eq!(measured(&totals, &reads, ProducedMeasure::Total), vec![(0, 11, Amount::Known(0)), (12, 35, Amount::Known(5)), (36, 100, Amount::Known(12))]);
        assert_eq!(measured(&totals, &reads, ProducedMeasure::Week), vec![(0, 100, Amount::Unknown(Why::WindowIncomplete))]);
        assert_eq!(window_at(&totals, &reads, ProducedMeasure::Day, 50), Amount::Known(7));
        assert_eq!(window_at(&totals, &reads, ProducedMeasure::Day, 20), Amount::Unknown(Why::WindowIncomplete));
        assert!(measured(&[], &reads, ProducedMeasure::Day).is_empty());
        // Before the stretch's first read nothing is known.
        assert_eq!(measured(&totals, &[12, 36], ProducedMeasure::Day)[0], (0, 35, Amount::Unknown(Why::WindowIncomplete)));
    }

    #[test]
    fn an_unknown_end_or_another_observation_leaves_the_window_unknown() {
        let totals = [
            (0, 29, known(3)),
            (30, 53, (Amount::Unknown(Why::LostOutput), OBSERVED)),
            (54, 77, known(9)),
            (78, 120, (Amount::Known(1), (Some(78), Some(0)))),
        ];
        let reads = [6, 30, 54, 78, 102];
        assert_eq!(measured(&totals, &reads, ProducedMeasure::Day), vec![
            (0, 29, Amount::Unknown(Why::WindowIncomplete)),
            (30, 77, Amount::Unknown(Why::LostOutput)),
            (78, 101, Amount::Unknown(Why::ObservationRestarted)),
            (102, 120, Amount::Known(0)),
        ]);
    }
}
