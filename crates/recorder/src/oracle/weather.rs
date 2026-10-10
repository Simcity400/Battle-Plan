//! Weather comparisons follow F0D8E0, F0DBB0 and F0DDC0. The serializers
//! preserve fixed5 words but encode byte flags only as zero/nonzero truth.

use std::{collections::BTreeMap, io};

use installed_content::save::{Decimal, Save};
use replay::{frame::DecodedFrame, rows::tables};

use super::{Report, complete, invalid};

pub(super) fn compare(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let provinces = report.attempt("WeatherProvinces", |report| {
        compare_provinces(frame, saved, report)
    });
    let regions = report.attempt("WeatherRegions", |report| {
        compare_regions(frame, saved, report)
    });
    report.limitation(
        "the save holds neither the snow ceiling +0x1f8 nor permanent snow +0x30; a load recomputes both from content",
        "WeatherRegions.max_snow, WeatherProvinces.permanent_snow: census only",
    );
    report.limitation(
        "weather Boolean serializer AFDC0 preserves zero/nonzero truth only; nonzero native byte values are indistinguishable in text saves",
        "WeatherRegions.light_rain, heavy_rain, snow, blizzard, sandstorm, arctic_water; WeatherProvinces.raw_mud",
    );
    if provinces && regions {
        Ok(())
    } else {
        Err(invalid(
            "weather comparison incomplete; see per-table errors",
        ))
    }
}

fn compare_provinces(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let mut provinces = BTreeMap::new();
    for row in complete::<tables::WeatherProvinces>(frame)? {
        if provinces.insert(row.id, row).is_some() {
            return Err(invalid("duplicate native weather province id"));
        }
        report.recorded(
            &row.local_hour,
            "WeatherProvinces.local_hour",
            format!("weather province {}.local_hour", row.id),
        );
        report.recorded(
            &row.night_factor,
            "WeatherProvinces.night_factor",
            format!("weather province {}.night_factor", row.id),
        );
        if row.id == 0
            && report
                .recorded(
                    &row.display,
                    "WeatherProvinces.display",
                    "weather display boundary",
                )
                .is_some_and(Option::is_none)
        {
            report.limitation(
                "WeatherProvinces.display unavailable",
                "1 live snow/mud boundary acquisition refused",
            );
        }
    }
    for (ordinal, &id) in provinces.keys().enumerate() {
        dense_slot(ordinal, id, "captured weather provinces")?;
    }

    let mut compared_provinces = 0;
    if let Some(expected) = &saved.weather.provinces {
        for (ordinal, expected) in expected.iter().enumerate() {
            // F0D8E0 emits every slot from zero to count, in array order.
            // This also rejects repeated keys without collapsing their data.
            dense_slot(ordinal, expected.slot, "saved weather provinces")?;
            let label = format!("weather province {}", expected.slot);
            let Some(actual) = provinces.remove(&expected.slot) else {
                report.difference(label, "absent", "present");
                continue;
            };
            compared_provinces += 1;
            if Some(actual.id) != expected.province {
                report.difference(
                    format!("{label}.province"),
                    Some(actual.id),
                    expected.province,
                );
            }
            sparse_scalar(report, &label, "snow", actual.snow, expected.snow.as_ref());
            // 140F0DBB0 always emits temperature through 1424AE590, even
            // when zero. Absence is malformed/missing evidence, not zero.
            if let Some(&temperature) = report.recorded(
                &actual.temperature,
                "WeatherProvinces.temperature",
                format!("{label}.temperature"),
            ) {
                report.scalar_checks += 1;
                if expected.temperature.as_ref().map(|value| value.raw) != Some(temperature) {
                    report.difference(
                        format!("{label}.temperature"),
                        Some(temperature),
                        expected.temperature.as_ref().map(|value| value.raw),
                    );
                }
            }
            if let Some(&mud) = report.recorded(
                &actual.raw_mud,
                "WeatherProvinces.raw_mud",
                format!("{label}.mud"),
            ) {
                truth(report, &format!("{label}.mud"), mud, expected.mud);
            }
        }
        for id in provinces.keys() {
            report.difference(format!("weather province {id}"), "present", "absent");
        }
    } else {
        report.difference(
            "weather.provinces",
            format!("{} decoded native rows", provinces.len()),
            "missing serialized dense roster",
        );
    }
    report.compared("WeatherProvinces", compared_provinces);
    report.limitation(
        "the manager serializer F0D8E0 omits the display byte arrays +0x310/+0x328; a load rebuilds them from the passes",
        "WeatherProvinces.display_snow, display_mud: census only",
    );
    report.limitation(
        "the manager serializer F0D8E0 omits the stored day/night array +0x370; these simulation values require a matched native read witness",
        "WeatherProvinces.local_hour, night_factor: census only",
    );
    report.limitation(
        "the manager serializer 140F0D8E0 omits the graphics grid +388/+390, refresh interval +300 and refresh identity +340; retained boundary and local fade require matched native graphics witnesses",
        "WeatherProvinces.display: runtime-only; no text-save equality claim",
    );
    Ok(())
}

fn compare_regions(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let mut regions = BTreeMap::new();
    for row in complete::<tables::WeatherRegions>(frame)? {
        if regions.insert(row.id, row).is_some() {
            return Err(invalid("duplicate native weather region id"));
        }
    }
    for (ordinal, &id) in regions.keys().enumerate() {
        dense_slot(ordinal, id, "captured weather regions")?;
    }
    let mut compared_regions = 0;
    if let Some(expected) = &saved.weather.regions {
        for (ordinal, expected) in expected.iter().enumerate() {
            dense_slot(ordinal, expected.slot, "saved weather regions")?;
            let label = format!("weather region {}", expected.slot);
            let Some(actual) = regions.remove(&expected.slot) else {
                report.difference(label, "absent", "present");
                continue;
            };
            compared_regions += 1;
            if Some(actual.id) != expected.region {
                report.difference(format!("{label}.region"), Some(actual.id), expected.region);
            }
            for (name, captured, saved) in [
                ("rain_light", actual.light_rain, expected.rain_light),
                ("rain_heavy", actual.heavy_rain, expected.rain_heavy),
                ("snow", actual.snow, expected.snow),
                ("blizzard", actual.blizzard, expected.blizzard),
                ("sandstorm", actual.sandstorm, expected.sandstorm),
                ("arctic_water", actual.arctic_water, expected.arctic_water),
            ] {
                truth(report, &format!("{label}.{name}"), captured, saved);
            }
        }
        for id in regions.keys() {
            report.difference(format!("weather region {id}"), "present", "absent");
        }
    } else {
        report.difference(
            "weather.regions",
            format!("{} decoded native rows", regions.len()),
            "missing serialized dense roster",
        );
    }
    report.compared("WeatherRegions", compared_regions);
    Ok(())
}

fn dense_slot(ordinal: usize, slot: u32, label: &str) -> io::Result<()> {
    if u32::try_from(ordinal).ok() != Some(slot) {
        return Err(invalid(format!(
            "{label} must retain every dense slot in order from zero: position {ordinal}, slot {slot}"
        )));
    }
    Ok(())
}

fn sparse_scalar(
    report: &mut Report,
    label: &str,
    field: &str,
    captured: i64,
    saved: Option<&Decimal>,
) {
    // F0DBB0 tests each complete i64 before emission, omitting exactly zero.
    let expected = saved.map_or(0, |value| value.raw);
    report.scalar_checks += 1;
    if captured != expected {
        report.difference(format!("{label}.{field}"), captured, expected);
    }
}

fn truth(report: &mut Report, label: &str, captured: u8, saved: Option<bool>) {
    // F0DBB0/F0DDC0 omit exactly zero; AFDC0 spells every nonzero byte as yes.
    let expected = saved.unwrap_or(false);
    if (captured != 0) != expected {
        report.difference(label, captured, expected);
    }
}

#[cfg(test)]
mod tests;
