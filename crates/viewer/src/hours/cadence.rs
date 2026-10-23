use dashboard_base::prepared_map::pages;
use replay_data::Hour;

pub(super) fn stride_among(speed: f64, strides: &[u32]) -> Option<u32> {
    if !speed.is_finite() || speed <= 0.0 {
        return None;
    }
    strides
        .iter()
        .copied()
        .filter(|&stride| stride != 0 && f64::from(stride) * f64::from(pages::CADENCE) <= speed)
        .max()
}

pub(super) fn stride(speed: f64) -> Option<u32> {
    stride_among(speed, &pages::provisional_strides())
}

pub(super) fn targets(
    stride: u32,
    count: u32,
    hour: Hour,
    anchor: Option<Hour>,
    range: (Hour, Hour),
) -> Vec<Hour> {
    let (first, last) = (i64::from(range.0), i64::from(range.1));
    let mut targets = Vec::new();
    let mut push = |target: i64| {
        let target = Hour::try_from(target).expect("an hour within the range");
        if !targets.contains(&target) {
            targets.push(target);
        }
    };
    if let Some(anchor) = anchor.filter(|anchor| range.0 <= *anchor && *anchor <= range.1) {
        push(i64::from(anchor));
    }
    if stride == 0 || count == 0 {
        return targets;
    }
    let stride = i64::from(stride);
    let after = i64::from(hour).max(first - 1);
    let mut next = (after.div_euclid(stride) + 1) * stride;
    for _ in 0..count {
        if next > last {
            break;
        }
        push(next);
        next += stride;
    }
    let horizon = (after.div_euclid(stride) + i64::from(count)) * stride;
    if last > i64::from(hour) && last <= horizon {
        push(last);
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stride_keeps_the_cadence_target_from_the_finest_preset_up() {
        assert_eq!(pages::provisional_strides(), [20, 40]);
        for (speed, expected) in [
            (0.25, None),
            (24.0, None),
            (719.0, None),
            (719.999, None),
            (720.0, Some(20)),
            (1_000.0, Some(20)),
            (1_439.0, Some(20)),
            (1_440.0, Some(40)),
            (1_440.25, Some(40)),
            (100_000.0, Some(40)),
            (0.0, None),
            (-1_440.0, None),
            (f64::NAN, None),
            (f64::INFINITY, None),
        ] {
            assert_eq!(stride(speed), expected, "speed {speed}");
        }
        assert_eq!(stride_among(1_440.0, &[40, 0, 20]), Some(40));
        assert_eq!(stride_among(1_440.0, &[]), None);
        assert_eq!(stride_among(1_440.0, &[0]), None);
    }

    #[test]
    fn targets_are_absolute_grid_hours_after_the_playhead() {
        let range = (0, 10_000);
        assert_eq!(targets(40, 4, 101, None, range), [120, 160, 200, 240]);
        assert_eq!(targets(40, 2, 120, None, range), [160, 200]);
        assert_eq!(targets(20, 3, 119, None, range), [120, 140, 160]);
        assert_eq!(targets(20, 3, -45, None, (-1_000, 1_000)), [-40, -20, 0]);
        assert_eq!(targets(40, 2, -40, None, (-1_000, 1_000)), [0, 40]);
        assert_eq!(targets(40, 2, -500, None, (90, 1_000)), [120, 160]);
        assert_eq!(targets(0, 4, 101, Some(101), range), [101]);
        assert_eq!(targets(40, 0, 101, Some(101), range), [101]);
        assert!(targets(40, 0, 101, None, range).is_empty());
    }

    #[test]
    fn the_anchor_comes_first_once_and_only_within_the_range() {
        let range = (100, 10_000);
        assert_eq!(targets(40, 3, 101, Some(101), range), [101, 120, 160, 200]);
        assert_eq!(targets(40, 3, 101, Some(160), range), [160, 120, 200]);
        assert_eq!(targets(40, 2, 230, Some(101), range), [101, 240, 280]);
        assert_eq!(targets(40, 2, 230, Some(50), range), [240, 280]);
        assert_eq!(targets(40, 2, 230, Some(20_000), range), [240, 280]);
    }

    #[test]
    fn the_branch_end_is_the_last_target_only_once_the_horizon_reaches_it() {
        let range = (0, 1_500);
        assert_eq!(targets(40, 4, 1_401, None, range), [1_440, 1_480, 1_500]);
        assert_eq!(
            targets(40, 4, 1_000, None, range),
            [1_040, 1_080, 1_120, 1_160]
        );
        assert_eq!(targets(20, 4, 1_455, None, (0, 1_480)), [1_460, 1_480]);
        assert_eq!(targets(20, 4, 5, None, (0, 15)), [15]);
        assert!(targets(40, 4, 1_500, None, range).is_empty());
        assert_eq!(targets(40, 4, 1_500, Some(1_500), range), [1_500]);
    }

    #[test]
    fn targets_never_overflow_at_the_ends_of_the_hour_range() {
        let range = (Hour::MAX - 100, Hour::MAX);
        let wanted = targets(40, 8, Hour::MAX - 50, None, range);
        assert!(!wanted.is_empty());
        assert!(wanted.iter().all(|hour| *hour > Hour::MAX - 50));
        assert_eq!(wanted.last(), Some(&Hour::MAX));
        let range = (Hour::MIN, Hour::MIN + 100);
        let wanted = targets(40, 3, Hour::MIN, None, range);
        assert!(
            wanted
                .iter()
                .all(|hour| range.0 <= *hour && *hour <= range.1)
        );
        assert!(targets(40, u32::MAX, Hour::MAX, None, (0, Hour::MAX)).is_empty());
    }

    #[test]
    fn targets_keep_their_laws_over_every_small_case() {
        for stride in [1, 3, 20, 40] {
            for count in 0..5 {
                for hour in -90..90 {
                    for (first, last) in [(-100, 100), (-7, 13), (10, 10), (0, 79)] {
                        for anchor in [None, Some(hour), Some(first - 1), Some(last)] {
                            let wanted = targets(stride, count, hour, anchor, (first, last));
                            let label =
                                format!("{stride} {count} {hour} {anchor:?} {first}..={last}");
                            let anchored = anchor.filter(|a| first <= *a && *a <= last);
                            let rest = &wanted[usize::from(anchored.is_some())..];
                            assert_eq!(wanted.first().copied(), anchored.or(rest.first().copied()));
                            assert!(wanted.iter().all(|h| first <= *h && *h <= last), "{label}");
                            assert!(rest.windows(2).all(|pair| pair[0] < pair[1]), "{label}");
                            assert!(!rest.iter().any(|h| Some(*h) == anchored), "{label}");
                            assert!(rest.iter().all(|h| *h > hour), "{label}");
                            let grid = rest.iter().filter(|h| h.rem_euclid(stride as i32) == 0);
                            assert!(grid.count() <= count as usize, "{label}");
                            assert!(
                                rest.iter()
                                    .all(|h| h.rem_euclid(stride as i32) == 0 || *h == last),
                                "{label}"
                            );
                        }
                    }
                }
            }
        }
    }
}
