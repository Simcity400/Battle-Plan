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
//! Counter tooltip ETA uses the stored route (the division panel's value); the native counter
//! shows a client-UI path-cache cost filled by hover/selection queries (140BED3A0), which is
//! unrecorded. The seed is five unit updates (140BE9902); 140BF0300 also decays viewer levels,
//! but its tail at 140BF08E0 decrements entry+0x10 in the stride-0x18 path-cache vector at
//! CUnit+0x308/count+0x314 and compacts expired entries. Each shown destination-ETA line counts
//! this limitation. Fresh sample H showed 38 h natively and 30 h here: on a cache miss,
//! 140BED280 → 140BED3A0 searches a route instead of using 140BECF90's stored route.
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
}

/// What `remaining` fixed5 hours arrive at, or `None` where the game writes no line at all: a
/// route already run out, and any negative or sentinel duration.
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

/// The game's own sentence in the viewer's language: `UNIT_MOVING_HOURS` or
/// `UNIT_MOVING_DAYS_HOURS`, with the `_NONPLAYER` pair for a stack the viewing
/// country does not own, localised with the integer arguments `HOURS`, and
/// `DAYS` for the days keys, as 1418C00D0 passes them (Localisation_FormatValue
/// 1402E2E60, LocalizeWithTwoIntegers 1406BABB0). A key the content lacks is
/// its own bytes (142230940). The counter builder validates the resulting
/// native colour controls and inline artwork through its shared text reader.
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
        // A key the content lacks is drawn as its bytes (142230940).
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
