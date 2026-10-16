//! The arrival line a moving unit shows on its map counter.
//!
//! The game words it from one ceiling at two sites. The division-view tooltip
//! integrates remaining time over the stored route and writes
//! `UNIT_VIEW_MOVE_HOURS` or `UNIT_VIEW_MOVE_DAYS_HOURS`.
//! The counter tooltip builder writes the same sentence on the **map
//! counter** as `UNIT_MOVING_HOURS` / `UNIT_MOVING_DAYS_HOURS`, and switches to the
//! `_NONPLAYER` pair over the **next** province for a unit the viewing player does not own
//! (`ccountry_tag::same_country` against the idler's country). The viewer has no division view,
//! so its counter hover carries the line, in the counter tooltip's own wording.
//!
//! On a counter cache miss the game searches a new route, then integrates its
//! duration. The viewer words a division's
//! arrival estimate instead ([`super::division_eta`]): the walk over its stored route with
//! each edge at its recorded base speed and the recorded infrastructure and organisation
//! terms, which drifts from the game wherever the unrecorded speed terms apply. A railway gun keeps the fresh native query with its current-hour operands, and
//! a task force walks its stored route at its native per-region speeds. The native interface's
//! retained duration cache and route option are local presentation state the viewer does not
//! read or capture.
//!
//! Both sites use [`engine::cunit::eta_hours`], narrowed to i32.
//! The division view's legacy `arrival` test helper omits nonpositive hours. The map
//! counter's [`map_arrival`] formats every reached duration: signed days<1
//! uses total hours, otherwise days and the hours remainder.

use engine::cunit::eta_hours;
use viewer_foundation::tooltip_text::tooltip_keys;

/// One counter's arrival, in the parts the game's sentence names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arrival {
    /// The `hours / 24`.
    pub days: i32,
    /// The `hours % 24`.
    pub hours: i32,
}

/// What `remaining` fixed5 hours arrive at, or `None` where the game writes no line at all: a
/// route already run out or any duration whose narrowed total hours is nonpositive.
#[cfg(test)]
pub fn arrival(remaining: i64) -> Option<Arrival> {
    let total = eta_hours(remaining) as i32;
    if total < 1 {
        return None;
    }
    Some(Arrival {
        days: total / 24,
        hours: total % 24,
    })
}

/// Format every reached query result. When the whole-day count is below one,
/// pass the total signed hours, including zero and negative values.
pub(crate) fn map_arrival(remaining: i64) -> Arrival {
    let total = eta_hours(remaining) as i32;
    let days = total / 24;
    if days < 1 {
        Arrival {
            days: 0,
            hours: total,
        }
    } else {
        Arrival {
            days,
            hours: total % 24,
        }
    }
}

/// Format counter arrival with the content's moving-hours or moving-days-and-hours
/// keys and their nonplayer variants. Integer arguments are `HOURS` and, where
/// needed, `DAYS`. Missing keys retain their bytes. The shared text reader validates
/// colour controls and inline artwork.
pub fn sentence(
    localisation: &installed_content::localisation::Localisation,
    arrival: &Arrival,
    ours: bool,
) -> Result<String, String> {
    use installed_content::localisation::Argument;
    let key = match (arrival.days == 0, ours) {
        (true, true) => UNIT_MOVING_HOURS,
        (false, true) => UNIT_MOVING_DAYS_HOURS,
        (true, false) => UNIT_MOVING_HOURS_NONPLAYER,
        (false, false) => UNIT_MOVING_DAYS_HOURS_NONPLAYER,
    };
    let hours = ("HOURS", Argument::Int32(arrival.hours));
    let days = ("DAYS", Argument::Int32(arrival.days));
    let arguments: &[_] = if arrival.days == 0 {
        &[hours]
    } else {
        &[days, hours]
    };
    localisation
        .localise_with(key.as_bytes(), arguments)
        .map_err(|refusal| refusal.to_string())
}

tooltip_keys! {
    UNIT_MOVING_HOURS = "UNIT_MOVING_HOURS",
    UNIT_MOVING_DAYS_HOURS = "UNIT_MOVING_DAYS_HOURS",
    UNIT_MOVING_HOURS_NONPLAYER = "UNIT_MOVING_HOURS_NONPLAYER",
    UNIT_MOVING_DAYS_HOURS_NONPLAYER = "UNIT_MOVING_DAYS_HOURS_NONPLAYER",
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_ceilings_to_whole_hours() {
        // 4.43806 h, the RENDERING.md 7.3 worked example, ceilings to 5.
        assert_eq!(arrival(443_806), Some(Arrival { days: 0, hours: 5 }));
        // Exact hours do not ceiling past themselves.
        assert_eq!(arrival(100_000).unwrap().hours, 1);
        assert_eq!(arrival(100_001).unwrap().hours, 2);
    }

    #[test]
    fn days_and_hours_split_as_the_game_splits_them() {
        let arrival = arrival(2_900_000).expect("29 h remaining");
        assert_eq!((arrival.days, arrival.hours), (1, 5));
    }

    #[test]
    fn nothing_is_shown_for_an_arrived_or_negative_duration() {
        // 0 is arrived or no route; a negative remaining is the progress subtraction.
        assert_eq!(arrival(0), None);
        assert_eq!(arrival(-1), None);
        assert_eq!(arrival(-100_000), None);
        // Any part of an hour still ceilings to one, so it is shown.
        assert_eq!(arrival(1).unwrap().hours, 1);
        // 4294967295 is the engine's zero-denominator sentinel: a duration like any
        // other here, and the game words it the same way.
        assert_eq!(arrival(4_294_967_295).unwrap().days, 1789);
    }

    #[test]
    fn map_counter_formats_zero_negative_and_narrowed_total_hours() {
        assert_eq!(map_arrival(0), Arrival { days: 0, hours: 0 });
        assert_eq!(
            map_arrival(-2_900_000),
            Arrival {
                days: 0,
                hours: -29
            }
        );
        assert_eq!(map_arrival(2_900_000), Arrival { days: 1, hours: 5 });
        assert_eq!(
            map_arrival((i64::from(i32::MAX) + 1) * 100_000),
            Arrival {
                days: 0,
                hours: i32::MIN
            }
        );
    }

    #[test]
    fn the_wording_follows_the_viewing_country_and_the_content_text() {
        let localisation: installed_content::localisation::Localisation = [
            (
                "UNIT_MOVING_HOURS",
                "Arrives at its destination in $HOURS|Y$ §Yhour(s)§!.",
            ),
            (
                "UNIT_MOVING_DAYS_HOURS_NONPLAYER",
                "Next province in $DAYS|Y$ §Yday(s)§! and $HOURS$ h, $OTHER$.",
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
        let one = Arrival { days: 0, hours: 5 };
        assert_eq!(
            sentence(&localisation, &one, true).as_deref(),
            Ok("Arrives at its destination in \u{11}Y5\u{11}! \u{11}Yhour(s)\u{11}!.")
        );
        let two = Arrival { days: 1, hours: 5 };
        // `$OTHER$` is no argument and no key: the localiser writes nothing.
        assert_eq!(
            sentence(&localisation, &two, false).as_deref(),
            Ok("Next province in \u{11}Y1\u{11}! \u{11}Yday(s)\u{11}! and 5 h, .")
        );
        // A key the content lacks is drawn as its bytes.
        assert_eq!(
            sentence(&localisation, &one, false).as_deref(),
            Ok("UNIT_MOVING_HOURS_NONPLAYER")
        );
        let icon: installed_content::localisation::Localisation = [(
            "UNIT_MOVING_DAYS_HOURS".to_owned(),
            "£time $DAYS$".to_owned(),
        )]
        .into_iter()
        .collect();
        assert_eq!(sentence(&icon, &two, true), Ok("\u{13}time 1".into()));
    }
}
