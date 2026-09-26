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

/// The game's own sentence in the viewer's language: `UNIT_MOVING_HOURS` or
/// `UNIT_MOVING_DAYS_HOURS`, with the `_NONPLAYER` pair for a stack the viewing
/// country does not own, `$DAYS$`/`$HOURS$` filled and the colour codes
/// stripped. `at` is [`Arrival::hour`] already written as a date, appended as
/// the viewer's projected arrival. `None` when the content lacks the key.
pub fn sentence(
    localisation: &installed_content::localisation::Localisation,
    arrival: &Arrival,
    ours: bool,
    at: &str,
) -> Option<String> {
    let key = match (arrival.days == 0, ours) {
        (true, true) => "UNIT_MOVING_HOURS",
        (false, true) => "UNIT_MOVING_DAYS_HOURS",
        (true, false) => "UNIT_MOVING_HOURS_NONPLAYER",
        (false, false) => "UNIT_MOVING_DAYS_HOURS_NONPLAYER",
    };
    let text = substitute(
        localisation.get(key)?,
        &[
            ("DAYS", arrival.days.to_string()),
            ("HOURS", arrival.hours.to_string()),
        ],
    );
    Some(format!("{} ({at})", super::text::strip_color_codes(&text)))
}

/// Replaces each `$NAME$` or `$NAME|format$` placeholder with its value; any
/// other placeholder is kept as written.
fn substitute(text: &str, values: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('$') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let token = &after[..end];
        let name = token.split('|').next().unwrap_or(token);
        match values.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                out.push('$');
                out.push_str(token);
                out.push('$');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
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
        let one = Arrival {
            days: 0,
            hours: 5,
            hour: 7,
        };
        assert_eq!(
            sentence(&localisation, &one, true, "1936-09-30  01:00").as_deref(),
            Some("Arrives at its destination in 5 hour(s). (1936-09-30  01:00)")
        );
        let two = Arrival {
            days: 1,
            hours: 5,
            hour: 7,
        };
        assert_eq!(
            sentence(&localisation, &two, false, "d").as_deref(),
            Some("Next province in 1 day(s) and 5 h, $OTHER$. (d)")
        );
        // A key the content lacks draws no line rather than a vanilla one.
        assert_eq!(sentence(&localisation, &one, false, "d"), None);
    }
}
