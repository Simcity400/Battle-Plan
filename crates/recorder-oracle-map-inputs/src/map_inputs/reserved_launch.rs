//! The saved signed launch-base list is ordered, including physical aliases.
//! This comparison never authorizes duplicate countries in the full Save parser.

use super::*;
use installed_content_save::naval_launch::CountryNavalLaunchOccurrence;
use replay::rows::{LaunchNavalBases, Recorded, ReservedCountryNamingRow};

const LABEL: &str = "ReservedCountryNaming.launch_sources";

pub(super) fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let [row] = frame.complete::<tables::ReservedCountryNaming>()? else {
        return Err(invalid(
            "reserved launch comparison requires one reserved row",
        ));
    };
    let Some(packet) = report.recorded(&row.launch_sources, LABEL, LABEL) else {
        return Ok(());
    };
    let Some(packet) = packet else {
        report.difference(LABEL, "unavailable", "required current-hour launch packet");
        return Ok(());
    };
    if !report.packet_fresh(LABEL, packet.sample_hour, frame.hour) {
        return Ok(());
    }
    match &packet.naval_bases {
        LaunchNavalBases::Bases(_) | LaunchNavalBases::NativeNull => {
            report.attempt("reserved launch saved naval bases", |report| {
                saved_bases(
                    frame,
                    row,
                    &packet.naval_bases,
                    &save.country_naval_launch,
                    save.country_emissions.is_some(),
                    report,
                )
            });
            if let LaunchNavalBases::Bases(bases) = &packet.naval_bases {
                report.limitation(
                    "the naval base save writer does not serialize the holder's raw country word; exact holder bits and object membership require matched native/write/consumer witnesses",
                    format!("{LABEL}.naval_bases: {} recorded occurrences", bases.len()),
                );
            } else {
                report.limitation(
                    "native-null launch provider: the save absence check cannot certify the runtime pointer or the consuming calculations",
                    format!("{LABEL}.naval_bases: native null; saved actual0 base-absence comparison required"),
                );
            }
        }
        LaunchNavalBases::Unavailable => report.difference(
            format!("{LABEL}.naval_bases"),
            "unavailable",
            "required actual0 ordered naval list",
        ),
    }
    match &packet.owned_states {
        Some(states) => {
            match &row.owned_state_count {
                Recorded::Value(Some(count)) if *count >= 0 => {
                    eq(
                        report,
                        &format!("{LABEL}.owned_states.count"),
                        states.len(),
                        *count as usize,
                    );
                }
                Recorded::Value(Some(count)) => report.difference(
                    format!("{LABEL}.owned_states.count"),
                    states.len(),
                    format!("refused negative native count {count}"),
                ),
                _ => report.difference(
                    format!("{LABEL}.owned_states.count"),
                    "unavailable",
                    "required same-read native count",
                ),
            }
            report.limitation(
                "the country save writer does not serialize the owned-state array; saved State owners cannot certify its exact IDs, count, order or repeats",
                format!("{LABEL}.owned_states: {} occurrences require matched native/write/consumer witnesses", states.len()),
            );
        }
        None => report.difference(
            format!("{LABEL}.owned_states"),
            "unavailable",
            "required actual0 ordered state list",
        ),
    }
    report.compared(LABEL, 1);
    Ok(())
}

fn saved_bases(
    frame: &DecodedFrame,
    row: &ReservedCountryNamingRow,
    provider: &LaunchNavalBases,
    saved: &[CountryNavalLaunchOccurrence],
    full_alias_admitted: bool,
    report: &mut Report,
) -> io::Result<()> {
    let physical = match &row.country_array {
        Recorded::Value(Some(physical)) if physical.first() == Some(&0) => physical,
        _ => {
            return Err(invalid(
                "reserved launch save join requires the authenticated physical country array",
            ));
        }
    };
    if saved
        .iter()
        .enumerate()
        .any(|(ordinal, country)| country.ordinal != ordinal)
    {
        return Err(invalid(
            "saved country launch occurrence ordinals are not contiguous",
        ));
    }
    let expected_count = physical.len() - 1;
    eq(
        report,
        "reserved launch physical country emission count",
        expected_count,
        saved.len(),
    );
    let expected_aliases = physical.iter().skip(1).filter(|&&id| id == 0).count();
    let saved_aliases = saved
        .iter()
        .filter(|country| country.tag == row.tag)
        .count();
    eq(
        report,
        "reserved launch actual0 country emission count",
        expected_aliases,
        saved_aliases,
    );
    let words = match provider {
        LaunchNavalBases::Bases(bases) => Some(
            bases
                .iter()
                .map(|base| base.base_province as i32)
                .collect::<Vec<_>>(),
        ),
        LaunchNavalBases::NativeNull => None,
        LaunchNavalBases::Unavailable => {
            return Err(invalid("reserved launch provider unavailable"));
        }
    };
    for (ordinal, &raw) in physical.iter().skip(1).enumerate() {
        let Some(country) = saved.get(ordinal) else {
            // The exact physical census difference above already records absence.
            continue;
        };
        if raw != 0 {
            if country.tag == row.tag {
                report.difference(
                    format!("reserved launch country emission {ordinal}"),
                    "positive own-ID occurrence",
                    "reserved tag occurrence",
                );
            }
            continue;
        }
        eq(
            report,
            &format!("reserved launch country emission {ordinal}.tag"),
            row.tag.as_slice(),
            country.tag.as_slice(),
        );
        match (&words, &country.naval_bases) {
            // A successfully observed null is accepted only for a saved slot
            // with no bases, whether its navy block is omitted or empty.
            (None, Some(saved)) if !saved.is_empty() => report.difference(
                format!("reserved launch country emission {ordinal}.naval_base"),
                "native-null provider",
                saved.as_slice(),
            ),
            (None, _) => {}
            (Some(words), Some(saved)) => eq(
                report,
                &format!("reserved launch country emission {ordinal}.naval_base"),
                words.as_slice(),
                &saved.iter().map(|base| base.province).collect::<Vec<_>>(),
            ),
            (Some(_), None) => report.difference(
                format!("reserved launch country emission {ordinal}.strategic_navy"),
                "observed navy list",
                "absent serialized navy",
            ),
        }
    }
    // Verify the other emission tags separately. An unrelated positive-link
    // failure cannot suppress exact comparisons of the reserved occurrences.
    report.attempt(
        "reserved launch positive country emission joins",
        |report| {
            if !physical.iter().skip(1).any(|&raw| raw != 0) {
                return Ok(());
            }
            let context = country_context(frame)?;
            for (ordinal, &raw) in physical
                .iter()
                .skip(1)
                .enumerate()
                .filter(|(_, raw)| **raw != 0)
            {
                let tag = required_country_tag(raw, &context.links)?;
                if tag == row.tag {
                    return Err(invalid(
                        "positive country tag collides with the recorded reserved tag",
                    ));
                }
                if let Some(country) = saved.get(ordinal) {
                    eq(
                        report,
                        &format!("reserved launch country emission {ordinal}.tag"),
                        tag,
                        country.tag.as_slice(),
                    );
                }
            }
            Ok(())
        },
    );
    if matches!(provider, LaunchNavalBases::NativeNull) {
        report.compared("reserved launch native-null saved absence", 1);
    } else if expected_aliases == 0 {
        report.limitation(
            "the game state save writer starts at physical slot one; an authenticated census with no positive physical actual0 occurrence provides no saved reserved-navy payload",
            format!("{LABEL}.naval_bases: matched native/write/consumer witness required"),
        );
    }
    if expected_aliases > 1 && !full_alias_admitted {
        report.limitation(
            "this ordered projection checks each repeated physical occurrence but does not relax Save::parse duplicate-country/handle refusals; repeated-alias full-save acceptance remains failed until separately supported",
            format!("{LABEL}: {expected_aliases} physical actual0 emissions"),
        );
    }
    report.compared("reserved launch saved naval bases", expected_aliases);
    Ok(())
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod tests;
