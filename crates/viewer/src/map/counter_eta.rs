//! The arrival line a moving division shows, from its derived remaining durations.
//!
//! The game words it from one ceiling at two sites. `CArmyDivisionView::idea_tooltip`
//! (`armiesview.cpp`, `141691B40`) takes `eta_remaining` (`140BECF90`) over the stored route —
//! what `DivisionEtaInputs` reconstructs — and writes `UNIT_VIEW_MOVE_HOURS` or
//! `UNIT_VIEW_MOVE_DAYS_HOURS`. `CUnitCounterItem::tooltip_builder` (`1418C00D0`) writes the same
//! sentence on the **map counter** as `UNIT_MOVING_HOURS` / `UNIT_MOVING_DAYS_HOURS`, and switches
//! to the `_NONPLAYER` pair over the **next** province for a unit the viewing player does not own
//! (`ccountry_tag::same_country` against the idler's country). The viewer has no division view, so
//! its counter hover carries the line, in the counter tooltip's own wording.
//!
//! **UNVERIFIED**: the native counter tooltip reads `ForwardCachedPathCost` (`140BED280`), the
//! pathfinder's cached estimate, where the division view reads `eta_remaining`. Only the latter is
//! recorded, so a stale cache is the one case where the game's counter and this one could differ.
//!
//! Both sites do the same arithmetic: [`engine::cunit::eta_hours`] (`14047BDE0`), narrowed to i32,
//! shown only when positive, split into `hours / 24` days and `hours % 24` hours.

use engine::cunit::eta_hours;

/// One counter's arrival, in the parts the game's sentence names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arrival {
    /// The game's `hours / 24`.
    pub days: i32,
    /// The game's `hours % 24`.
    pub hours: i32,
    /// The recorded hour plus the whole hours remaining, a projected arrival.
    pub hour: i32,
}

/// What `remaining` fixed5 hours arrive at, or `None` where the game writes no line at all: a
/// route already run out, and any negative or sentinel duration.
pub fn arrival(recorded_hour: i32, remaining: i64) -> Option<Arrival> {
    let total = eta_hours(remaining) as i32;
    if total < 1 {
        return None;
    }
    Some(Arrival {
        days: total / 24,
        hours: total % 24,
        hour: recorded_hour.saturating_add(total),
    })
}

/// The game's own sentence. `ours` picks the destination wording over the next-province one, and
/// `at` is [`Arrival::hour`] already written as a date.
pub fn sentence(arrival: &Arrival, ours: bool, at: &str) -> String {
    let place = if ours {
        "its destination"
    } else {
        "the next province"
    };
    let when = if arrival.days == 0 {
        format!("{} hour(s)", arrival.hours)
    } else {
        format!("{} day(s) and {} hour(s)", arrival.days, arrival.hours)
    };
    format!("The Unit will arrive in {place} in {when}. ({at})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_ceilings_to_whole_hours_and_lands_on_an_hour() {
        // 4.43806 h, the RENDERING.md 7.3 worked example, ceilings to 5.
        assert_eq!(
            arrival(60_832_632, 443_806),
            Some(Arrival {
                days: 0,
                hours: 5,
                hour: 60_832_637
            })
        );
        // Exact hours do not ceiling past themselves.
        assert_eq!(arrival(100, 100_000).unwrap().hours, 1);
        assert_eq!(arrival(100, 100_001).unwrap().hours, 2);
    }

    #[test]
    fn days_and_hours_split_as_the_game_splits_them() {
        let arrival = arrival(0, 2_900_000).expect("29 h remaining");
        assert_eq!((arrival.days, arrival.hours, arrival.hour), (1, 5, 29));
    }

    #[test]
    fn nothing_is_shown_for_an_arrived_or_negative_duration() {
        // 0 is arrived or no route; a negative remaining is the progress subtraction.
        assert_eq!(arrival(100, 0), None);
        assert_eq!(arrival(100, -1), None);
        assert_eq!(arrival(100, -100_000), None);
        // Any part of an hour still ceilings to one, so it is shown.
        assert_eq!(arrival(100, 1).unwrap().hours, 1);
        // 4294967295 is the engine's zero-denominator sentinel: a duration like any
        // other here, and the game words it the same way.
        assert_eq!(arrival(100, 4_294_967_295).unwrap().days, 1789);
    }

    #[test]
    fn the_wording_follows_the_viewing_country() {
        let one = Arrival {
            days: 0,
            hours: 5,
            hour: 7,
        };
        assert_eq!(
            sentence(&one, true, "1936-09-30  01:00"),
            "The Unit will arrive in its destination in 5 hour(s). (1936-09-30  01:00)"
        );
        let two = Arrival {
            days: 1,
            hours: 5,
            hour: 7,
        };
        assert_eq!(
            sentence(&two, false, "1936-09-30  01:00"),
            "The Unit will arrive in the next province in 1 day(s) and 5 hour(s). (1936-09-30  01:00)"
        );
    }
}
