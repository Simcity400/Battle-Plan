//! Actual-owner-zero intel is admitted only by its paired sample certificate.
//! Sparse holes are exact zero only inside that complete native vector.

use super::*;
use installed_content::save::ActualCountry;
use replay::rows::{CountryIntelRow, ReservedCountryNamingRow};

const LABEL: &str = "ReservedCountryNaming.country_intel_sample";

struct Admitted<'a> {
    naming: &'a ReservedCountryNamingRow,
    sample_hour: i32,
    count: u32,
    records: BTreeMap<u32, &'a CountryIntelRow>,
}

fn admitted<'a>(frame: &'a DecodedFrame, report: &mut Report) -> io::Result<Option<Admitted<'a>>> {
    let rows = frame.complete::<tables::CountryIntel>()?;
    let [naming] = frame.complete::<tables::ReservedCountryNaming>()? else {
        return Err(invalid(
            "reserved country intel requires one complete naming row",
        ));
    };
    let Some(sample) = report.recorded(&naming.country_intel_sample, LABEL, LABEL) else {
        if rows.iter().any(|row| row.owner_country == 0) {
            return Err(invalid(
                "actual-owner-zero intel rows have no recorded admission certificate",
            ));
        }
        return Ok(None);
    };
    let Some(sample) = sample else {
        return Err(invalid(
            "actual-owner-zero country intel sample was refused at capture",
        ));
    };
    if sample.vector_count > i32::MAX as u32 {
        return Err(invalid(
            "actual-owner-zero intel count exceeds the native signed domain",
        ));
    }
    if sample.sample_hour > frame.hour {
        return Err(invalid(format!(
            "{LABEL}.sample_hour {} is later than oracle hour {}",
            sample.sample_hour, frame.hour,
        )));
    }
    let mut records = BTreeMap::new();
    for row in rows.iter().filter(|row| row.owner_country == 0) {
        if row.target_index >= sample.vector_count {
            return Err(invalid(
                "actual-owner-zero intel row lies outside its admitted vector",
            ));
        }
        if records.insert(row.target_index, row).is_some() {
            return Err(invalid(
                "actual-owner-zero intel repeats a native record index",
            ));
        }
    }
    if sample.vector_count > 0 && !records.contains_key(&(sample.vector_count - 1)) {
        return Err(invalid(
            "actual-owner-zero intel is missing its admitted final-index sentinel",
        ));
    }
    Ok(Some(Admitted {
        naming,
        sample_hour: sample.sample_hour,
        count: sample.vector_count,
        records,
    }))
}

fn compare_records(
    actual: &Admitted<'_>,
    saved: Option<&[installed_content::save::CountryIntelRecord]>,
    label: &str,
    report: &mut Report,
) -> io::Result<()> {
    // The nested native list writer omits the whole list when count is not
    // positive. The Save reader refuses an explicitly present empty list.
    let saved_count = saved.map_or(0, |records| records.len());
    let saved_count = u32::try_from(saved_count)
        .map_err(|_| invalid("saved country intel count exceeds its wire width"))?;
    if actual.count != saved_count {
        report.difference(format!("{label}.count"), actual.count, saved_count);
    }
    let zero = CountryIntelRow::default();
    for (index, expected) in saved.unwrap_or_default().iter().enumerate() {
        let index = index as u32;
        let label = format!("{label}[{index}]");
        if index >= actual.count {
            report.difference(&label, "absent", "present");
            continue;
        }
        let captured = actual.records.get(&index).copied().unwrap_or(&zero);
        report.compared("CountryIntel actual owner0", 1);
        for (field, captured, expected) in [
            ("army", captured.army, expected.army),
            ("airforce", captured.air, expected.air),
            ("civilian", captured.civilian, expected.civilian),
            ("navy", captured.navy, expected.navy),
        ] {
            if captured != expected {
                report.difference(format!("{label}.{field}"), captured, expected);
            }
        }
    }
    if actual.count > saved_count {
        report.difference(
            format!("{label}[{saved_count}..{}]", actual.count),
            "present",
            "absent",
        );
    }
    Ok(())
}

pub(super) fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let Some(actual) = admitted(frame, report)? else {
        return Ok(());
    };
    if actual.sample_hour != frame.hour {
        report.carried.insert(
            "actual owner0 country intel".into(),
            Some(actual.sample_hour),
        );
        report.limitation(
            "actual-owner-zero country intel is carried from its paired acquisition hour and is not compared with the current save; hourly reverse relations do not make this a fresh intel sample",
            format!("{LABEL}: carried sample hour {}", actual.sample_hour),
        );
        return Ok(());
    }
    let physical = actual.naming.country_array.require("ReservedCountryNaming.country_array")
        .map_err(invalid)?
        .as_deref()
        .filter(|physical| physical.first() == Some(&0))
        .ok_or_else(|| invalid("reserved country intel save join requires the authenticated physical country array"))?;
    let emissions = save.country_emissions.as_deref().ok_or_else(|| invalid(
        "reserved country intel save join requires the validated original country emission projection",
    ))?;
    let expected_emissions = physical.len() - 1;
    if emissions.len() != expected_emissions {
        report.difference(
            "reserved country intel physical country emission count",
            expected_emissions,
            emissions.len(),
        );
    }
    let expected_zero = physical.iter().skip(1).filter(|&&raw| raw == 0).count();
    let saved_zero = emissions
        .iter()
        .filter(|emission| emission.identity.actual == ActualCountry::Reserved)
        .count();
    if saved_zero != expected_zero {
        report.difference(
            "reserved country intel actual0 country emission count",
            expected_zero,
            saved_zero,
        );
    }
    for (ordinal, &raw) in physical.iter().skip(1).enumerate() {
        let Some(emission) = emissions.get(ordinal) else {
            continue;
        };
        let identity = &emission.identity;
        let slot = u32::try_from(ordinal + 1)
            .map_err(|_| invalid("physical country emission exceeds its recorded wire width"))?;
        if identity.physical_slot != slot || identity.own_id != raw {
            report.difference(
                format!("reserved country intel emission {ordinal}.physical identity"),
                (slot, raw),
                (identity.physical_slot, identity.own_id),
            );
        }
        if raw != 0 {
            if identity.actual == ActualCountry::Reserved || identity.tag == actual.naming.tag {
                report.difference(
                    format!("reserved country intel emission {ordinal}.actual identity"),
                    "positive own-ID country",
                    "reserved country occurrence",
                );
            }
            continue;
        }
        let label = format!("actual owner0 country emission {ordinal}.intel");
        if identity.actual != ActualCountry::Reserved {
            report.difference(&label, "certified actual Country0", identity.actual);
            continue;
        }
        if identity.tag != actual.naming.tag {
            report.difference(
                format!("{label}.tag"),
                actual.naming.tag.as_slice(),
                identity.tag.as_slice(),
            );
            continue;
        }
        if !emission.intel_present {
            report.difference(
                &label,
                "admitted nonnull CountryIntel object",
                "absent outer serialized intel object",
            );
            continue;
        }
        // CountryProjection validates every ORIGINAL Item in physical order
        // and requires exact whole-Item equality of repeated actual objects.
        // The typed payload is therefore this occurrence's payload, including
        // each repeated emission; every one receives its own count/lane checks.
        compare_records(
            &actual,
            save.country_intel
                .get(identity.tag.as_slice())
                .map(Vec::as_slice),
            &label,
            report,
        )?;
    }
    // Failure of an unrelated positive link remains a failed join, but does
    // not conceal the reserved occurrences' exact payload comparisons.
    report.attempt("reserved country intel positive emission joins", |report| {
        if !physical.iter().skip(1).any(|&raw| raw != 0) {
            return Ok(());
        }
        let context = super::super::country_context(frame)?;
        for (ordinal, &raw) in physical
            .iter()
            .skip(1)
            .enumerate()
            .filter(|(_, raw)| **raw != 0)
        {
            let tag = required_country_tag(raw, &context.links)?;
            if tag == actual.naming.tag {
                return Err(invalid(
                    "positive country tag collides with the recorded reserved tag",
                ));
            }
            if let Some(emission) = emissions.get(ordinal) {
                if tag != emission.identity.tag.as_slice() {
                    report.difference(
                        format!("reserved country intel emission {ordinal}.tag"),
                        tag,
                        emission.identity.tag.as_slice(),
                    );
                }
                if emission.identity.actual
                    != ActualCountry::Canonical(context.links[&raw].canonical_slot)
                {
                    report.difference(
                        format!("reserved country intel emission {ordinal}.actual identity"),
                        context.links[&raw].canonical_slot,
                        emission.identity.actual,
                    );
                }
            }
        }
        Ok(())
    });
    if expected_zero == 0 {
        report.limitation(
            "the game state save writer skips only physical slot0 and writes every other pointer; this authenticated physical array has no actual Country0 occurrence after slot0, so its admitted intel vector count and all four lanes have no country-save counterpart and require a matched same-hour native witness",
            format!("{LABEL}: count {} at hour {}", actual.count, actual.sample_hour),
        );
    }
    report.limitation(
        "serialized country intel lanes do not certify the actual reserved object's nonnull pointer, owner identity, signed native header or paired acquisition certificate; these native facts require a matched same-hour witness even when every saved positional lane agrees",
        LABEL,
    );
    Ok(())
}

pub(super) fn is_serialized_reserved_tag(save: &Save, tag: &[u8]) -> bool {
    // This metadata exists only after the approved original-tree projection;
    // raw link aliases to canonical0 and tag spelling alone never qualify.
    save.country_emissions.as_deref().is_some_and(|emissions| {
        emissions.iter().any(|emission| {
            emission.identity.actual == ActualCountry::Reserved
                && emission.identity.own_id == 0
                && emission.identity.physical_slot > 0
                && emission.identity.tag.as_slice() == tag
        })
    })
}

#[cfg(test)]
#[path = "reserved_intel_tests.rs"]
mod tests;
