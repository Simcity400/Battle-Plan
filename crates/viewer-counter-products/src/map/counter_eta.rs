
use game_model::movement::unit::eta_hours;
use viewer_foundation::tooltip_text::tooltip_keys;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arrival {
    pub days: i32,
    pub hours: i32,
}

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

pub const ESTIMATED_SUFFIX: &str = " (estimated)";

pub fn sentence(
    localisation: &content_reader_localisation::localisation::Localisation,
    arrival: &Arrival,
    ours: bool,
) -> Result<String, String> {
    use content_reader_localisation::localisation::Argument;
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

pub fn estimated_sentence(
    localisation: &content_reader_localisation::localisation::Localisation,
    arrival: &Arrival,
    ours: bool,
) -> Result<String, String> {
    let text = sentence(localisation, arrival, ours)?;
    if text.is_empty() {
        Ok(text)
    } else {
        Ok(format!("{text}{ESTIMATED_SUFFIX}"))
    }
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
    fn map_counter_formats_zero_negative_and_narrowed_total_hours() {
        assert_eq!(map_arrival(443_806), Arrival { days: 0, hours: 5 });
        assert_eq!(map_arrival(100_000).hours, 1);
        assert_eq!(map_arrival(100_001).hours, 2);
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
        let localisation: content_reader_localisation::localisation::Localisation = [
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
        assert_eq!(
            sentence(&localisation, &two, false).as_deref(),
            Ok("Next province in \u{11}Y1\u{11}! \u{11}Yday(s)\u{11}! and 5 h, .")
        );
        assert_eq!(
            sentence(&localisation, &one, false).as_deref(),
            Ok("UNIT_MOVING_HOURS_NONPLAYER")
        );
        let icon: content_reader_localisation::localisation::Localisation = [(
            "UNIT_MOVING_DAYS_HOURS".to_owned(),
            "£time $DAYS$".to_owned(),
        )]
        .into_iter()
        .collect();
        assert_eq!(sentence(&icon, &two, true), Ok("\u{13}time 1".into()));
        assert_eq!(
            estimated_sentence(&localisation, &one, true).as_deref(),
            Ok("Arrives at its destination in \u{11}Y5\u{11}! \u{11}Yhour(s)\u{11}!. (estimated)")
        );
        assert_eq!(
            estimated_sentence(&localisation, &two, false).as_deref(),
            Ok("Next province in \u{11}Y1\u{11}! \u{11}Yday(s)\u{11}! and 5 h, . (estimated)")
        );
        let empty: content_reader_localisation::localisation::Localisation =
            [("UNIT_MOVING_HOURS".to_owned(), String::new())]
                .into_iter()
                .collect();
        assert_eq!(estimated_sentence(&empty, &one, true), Ok(String::new()));
    }
}
