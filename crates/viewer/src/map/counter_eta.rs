//! The arrival line a moving unit shows, from its fresh counter query.
//!
//! Map-counter wording uses the moving-hours or moving-days-and-hours localisation keys.
//! Foreign units use the nonplayer pair and the next province. Fresh inputs must match
//! the displayed hour, with independent transport, normal, railway and naval refusals.
//! First-edge progress is subtracted only when the searched edge matches the stored
//! ordinary first edge.
//!
//! The viewer uses its explicit default route policy. A game's retained duration cache
//! can differ from a fresh query during its five-unit-update lifetime.
//! [`engine::cunit::eta_hours`] is narrowed to i32. The legacy `arrival` helper omits
//! nonpositive hours; [`map_arrival`] formats every reached query result. Signed days
//! below one use total hours; other results use days and the hours remainder.

use engine::cunit::eta_hours;

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

crate::combat::text::tooltip_keys! {
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
