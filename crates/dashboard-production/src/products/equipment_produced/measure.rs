
use super::{ProducedMeasure, ProducedUnknown, value::Amount};

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

pub fn back(measure: ProducedMeasure, hour: i32) -> Option<i32> {
    match measure {
        ProducedMeasure::Total => None,
        ProducedMeasure::Day => hour.checked_sub(24),
        ProducedMeasure::Week => hour.checked_sub(7 * 24),
        ProducedMeasure::Month => month_before(hour),
    }
}

pub type Counted = (Amount, (Option<i32>, Option<i32>));

pub fn difference(at: Counted, before: Option<Counted>) -> Amount {
    match before {
        None => Amount::Unknown(ProducedUnknown::WindowIncomplete),
        Some((_, observation)) if observation != at.1 => Amount::Unknown(ProducedUnknown::ObservationRestarted),
        Some((before, _)) => at.0.minus(before),
    }
}

pub fn read_at(reads: &[i32], hour: i32) -> Option<i32> {
    reads.partition_point(|&read| read <= hour).checked_sub(1).map(|at| reads[at])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_month_before_is_the_same_day_or_the_shorter_months_last() {
        use game_model::calendar::HOUR_EPOCH;
        let day = |days: i32, hour: i32| HOUR_EPOCH + days * 24 + hour;
        assert_eq!(month_before(day(365 + 31 + 14, 6)), Some(day(365 + 14, 6)));
        assert_eq!(month_before(day(365 + 31 + 28 + 30, 0)), Some(day(365 + 31 + 27, 0)));
        assert_eq!(month_before(day(365 + 3, 1)), Some(day(334 + 3, 1)));
        assert_eq!(month_before(day(3, 1)), None);
        assert_eq!(month_before(HOUR_EPOCH - 1), None);
    }
}
