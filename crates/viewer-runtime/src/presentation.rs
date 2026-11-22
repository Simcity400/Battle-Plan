
use replay_data::BranchId;
use replay_frame::branches::Gap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Continuity {
    Reset,
    Continue,
}

impl Continuity {
    pub fn keeps(self, previous: (BranchId, i32), next: (BranchId, i32)) -> bool {
        self == Self::Continue && previous.0 == next.0 && previous.1 <= next.1
    }
}

#[derive(Clone, Copy)]
pub struct Target {
    pub branch: BranchId,
    pub hour: i32,
    pub seek_sequence: Option<u64>,
}

impl Target {
    pub fn seeks_from(self, previous: Self) -> bool {
        self.seek_sequence != previous.seek_sequence
            || self.branch != previous.branch
            || self.hour < previous.hour
            || (self.seek_sequence.is_none() && self.hour != previous.hour)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnitRestart {
    Keep,
    ViewingCountry,
    All,
}

pub fn unit_restart(
    previous: Option<(BranchId, i32, Option<&str>)>,
    next: (BranchId, i32, Option<&str>),
    sought: bool,
) -> UnitRestart {
    match previous {
        Some((branch, hour, viewing)) if !sought && branch == next.0 && hour <= next.1 => {
            if viewing == next.2 {
                UnitRestart::Keep
            } else {
                UnitRestart::ViewingCountry
            }
        }
        _ => UnitRestart::All,
    }
}

pub fn crosses_gap(gaps: &[Gap], branch: BranchId, from: i32, to: i32) -> bool {
    gaps.iter()
        .any(|gap| gap.branch == branch && gap.last_hour > from && gap.first_hour < to)
}

pub fn continuity(
    previous: Option<(BranchId, i32, Option<&str>)>,
    next: (BranchId, i32, Option<&str>),
    sought: bool,
    gaps: &[Gap],
) -> Continuity {
    match previous {
        Some((branch, hour, viewing))
            if !sought
                && branch == next.0
                && hour <= next.1
                && viewing == next.2
                && !crosses_gap(gaps, branch, hour, next.1) =>
        {
            Continuity::Continue
        }
        _ => Continuity::Reset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_entities_restart_only_where_the_game_recreates_them() {
        let gaps = [Gap {
            branch: 1,
            first_hour: 30,
            last_hour: 32,
            reason: "recorder paused".into(),
        }];
        let shown = Some((1, 10, Some("GER")));
        let case = |next: (BranchId, i32, Option<&str>), sought: bool| {
            (
                unit_restart(shown, next, sought),
                continuity(shown, next, sought, &gaps),
            )
        };
        use Continuity::{Continue, Reset};
        use UnitRestart::{All, Keep, ViewingCountry};
        assert_eq!(case((1, 40, Some("GER")), true), (All, Reset), "seek");
        assert_eq!(
            case((1, 10, Some("GER")), true),
            (All, Reset),
            "same-hour seek"
        );
        assert_eq!(case((1, 9, Some("GER")), false), (All, Reset), "step back");
        assert_eq!(case((2, 11, Some("GER")), false), (All, Reset), "branch");
        let first = (1, 10, Some("GER"));
        assert_eq!(
            (
                unit_restart(None, first, false),
                continuity(None, first, false, &gaps)
            ),
            (All, Reset),
            "first hour"
        );
        assert_eq!(
            case((1, 10, Some("ENG")), false),
            (ViewingCountry, Reset),
            "country"
        );
        assert_eq!(
            case((1, 10, None), false),
            (ViewingCountry, Reset),
            "no country"
        );
        assert_eq!(case((1, 40, Some("GER")), false), (Keep, Reset), "gap");
        assert_eq!(
            case((1, 10, Some("GER")), false),
            (Keep, Continue),
            "playback"
        );
        assert_eq!(case((1, 11, Some("GER")), false), (Keep, Continue), "+1");
        assert_eq!(case((1, 25, Some("GER")), false), (Keep, Continue), "skip");
    }

    #[test]
    fn dashboard_seeks_survive_coalescing_and_pinned_targets_have_no_clock() {
        let previous = Target {
            branch: 1,
            hour: 20,
            seek_sequence: Some(7),
        };
        let skipped = Target {
            hour: 200,
            ..previous
        };
        assert!(!skipped.seeks_from(previous));
        assert!(!skipped.seeks_from(skipped));
        for next in [
            Target {
                seek_sequence: Some(8),
                ..skipped
            },
            Target {
                seek_sequence: Some(8),
                ..previous
            },
            Target {
                hour: 19,
                ..previous
            },
            Target {
                branch: 2,
                ..skipped
            },
        ] {
            assert!(next.seeks_from(previous));
        }
        let pinned = Target {
            seek_sequence: None,
            ..previous
        };
        assert!(pinned.seeks_from(previous));
        assert!(!pinned.seeks_from(pinned));
        assert!(Target { hour: 21, ..pinned }.seeks_from(pinned));
        assert!(previous.seeks_from(pinned));
    }

    #[test]
    fn playback_skips_and_paused_catchup_keep_history_but_discontinuities_reset() {
        let previous = Some((1, 20, Some("AAA")));
        for hour in [20, 21, 200] {
            assert_eq!(
                continuity(previous, (1, hour, Some("AAA")), false, &[]),
                Continuity::Continue
            );
            assert_eq!(
                continuity(previous, (1, hour, Some("AAA")), true, &[]),
                Continuity::Reset
            );
        }
        for next in [
            (1, 19, Some("AAA")),
            (2, 21, Some("AAA")),
            (1, 21, Some("BBB")),
            (1, 21, None),
        ] {
            assert_eq!(continuity(previous, next, false, &[]), Continuity::Reset);
        }
        assert_eq!(
            continuity(None, (1, 20, Some("AAA")), false, &[]),
            Continuity::Reset
        );
        let gap = |branch, first_hour, last_hour| Gap {
            branch,
            first_hour,
            last_hour,
            reason: "missing".into(),
        };
        for gap in [gap(1, 21, 21), gap(1, 25, 30), gap(1, 39, 39)] {
            assert_eq!(
                continuity(previous, (1, 40, Some("AAA")), false, &[gap]),
                Continuity::Reset
            );
        }
        assert_eq!(
            continuity(
                previous,
                (1, 40, Some("AAA")),
                false,
                &[gap(2, 25, 30), gap(1, 41, 50), gap(1, 1, 19)]
            ),
            Continuity::Continue
        );
    }
}
