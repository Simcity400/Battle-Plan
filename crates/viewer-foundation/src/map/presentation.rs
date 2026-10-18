//! Viewer-owned animation history, independent of hour-builder cadence.
//!
//! A forward playback skip keeps the live runtimes, including when a paused
//! playhead's final hour finishes building. Seeks are explicit: hour distance
//! cannot distinguish a seek from playback that outran the builders. Keep this
//! contract when supply trains become entirely viewer-side animation.

use replay_data::BranchId;
use replay_frame::branches::Gap;

/// Whether the animation runtimes on screen carry into the next hour shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Continuity {
    Reset,
    Continue,
}

impl Continuity {
    /// Whether a layer may hand the runtimes it built for `previous` to
    /// `next`: only forward on one branch, and only when the viewer continues.
    pub fn keeps(self, previous: (BranchId, i32), next: (BranchId, i32)) -> bool {
        self == Self::Continue && previous.0 == next.0 && previous.1 <= next.1
    }
}

/// A host clock target. Dashboard and full-map targets carry the clock's seek
/// counter through coalesced updates. Pinned targets have no running clock, so
/// moving them is always a seek.
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

/// Which unit entities a new hour recreates, and so which animation
/// histories (`ModelPass` players) start again: the game's, not the
/// presentation cycle's. [`continuity`] still decides the raid cycle, the
/// trains and the weather.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnitRestart {
    /// Playback, a step forward, a skip over built hours and a recorded
    /// gap: the running game destroys no unit entity for them, so each
    /// keeps its history and a changed state blends as usual.
    Keep,
    /// A viewing-country change recreates unit entities only in the new country's owned provinces.
    /// Entities elsewhere retain animation histories.
    ViewingCountry,
    /// Initial load, seeks, step backs and branch switches clear all province unit slots and recreate
    /// entities.
    All,
}

/// [`UnitRestart`] from the same inputs as [`continuity`].
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

/// Whether the recording misses an hour between `from` and `to` on `branch`.
/// Recorded hours are never inside a gap, so the ends need no care.
pub fn crosses_gap(gaps: &[Gap], branch: BranchId, from: i32, to: i32) -> bool {
    gaps.iter()
        .any(|gap| gap.branch == branch && gap.last_hour > from && gap.first_hour < to)
}

/// The history from the hour on screen to `next`. The resolved viewing
/// country, not the fog switch, belongs to the history.
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
        // Load-equivalent: every entity is created afresh.
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
        // A tag switch recreates the new country's provinces' units only.
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
        // The game kept running through a gap: nothing is recreated, though
        // the presentation cycle still starts again.
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
        assert!(!skipped.seeks_from(skipped)); // Pause/camera/fog updates.
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
            // A same-hour or adjacent seek is still a seek.
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
