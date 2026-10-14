//! Weather comparisons follow the weather manager writer, the weather region writer and the weather writer. The serializers
//! preserve fixed5 words but encode byte flags only as zero/nonzero truth.

use std::{collections::BTreeMap, io};

use installed_content_save::{Decimal, Save};
use replay::{
    frame::DecodedFrame,
    rows::{ClockState, TemperatureRecord, tables},
};

use super::{Report, complete, invalid};

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    temperature: Result<&replay::temperature::Content, String>,
    report: &mut Report,
) -> io::Result<()> {
    let provinces = report.attempt("WeatherProvinces", |report| {
        compare_provinces(frame, saved, temperature, report)
    });
    let regions = report.attempt("WeatherRegions", |report| {
        compare_regions(frame, saved, report)
    });
    report.limitation(
        "the save holds neither the snow ceiling nor permanent snow; a load recomputes both from content",
        "WeatherRegions.max_snow, WeatherProvinces.permanent_snow: census only",
    );
    report.limitation(
        "the weather boolean writer preserves zero/nonzero truth only; nonzero native byte values are indistinguishable in text saves",
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

fn compare_provinces(
    frame: &DecodedFrame,
    saved: &Save,
    temperature: Result<&replay::temperature::Content, String>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::WeatherProvinces>(frame)?;
    // Derive temperatures at the frame's hour from its anchor and pass.
    // `recorder derived` checks each hour against the read checksum.
    let unrecorded = rows
        .iter()
        .all(|row| matches!(row.temperature.record(), Ok(TemperatureRecord::NotRecorded)));
    let temperatures = replay::temperature::Memo::default()
        .states(
            frame.branch,
            frame.hour,
            rows,
            temperature
                .as_ref()
                .map(|content| *content)
                .map_err(String::as_str),
        )
        .map(|states| {
            states
                .iter()
                .map(|state| state.temperature)
                .collect::<Vec<_>>()
        });
    if let (false, Err(reason)) = (unrecorded, &temperatures) {
        report.limitation(
            &format!("WeatherProvinces.temperature refused: {reason}"),
            "WeatherProvinces.temperature: not compared",
        );
    }
    let mut provinces = BTreeMap::new();
    for row in rows {
        if provinces.insert(row.id, row).is_some() {
            return Err(invalid("duplicate native weather province id"));
        }
        // Updated clocks are the hour's derivation and held ones the
        // record's words; neither has a save counterpart.
        if matches!(row.clock.state(row.id), Ok(ClockState::NotRecorded)) {
            report.recorded(
                &replay::rows::Recorded::<()>::NotRecorded,
                "WeatherProvinces.clock",
                format!("weather province {}.clock", row.id),
            );
        }
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
            // The weather manager writer emits every slot from zero to count, in array order.
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
            // The game always emits temperature, even
            // when zero. Absence is malformed/missing evidence, not zero.
            let recorded = match &temperatures {
                _ if unrecorded => Some(replay::rows::Recorded::NotRecorded),
                Ok(values) => values.get(actual.id as usize).copied().map(Into::into),
                // Refused once above; nothing to compare.
                Err(_) => None,
            };
            if let Some(&temperature) = recorded.as_ref().and_then(|recorded| {
                report.recorded(
                    recorded,
                    "WeatherProvinces.temperature",
                    format!("{label}.temperature"),
                )
            }) {
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
        "the weather manager writer omits the display byte arrays; a load rebuilds them from the passes",
        "WeatherProvinces.display_snow, display_mud: census only",
    );
    report.limitation(
        "the weather manager writer omits the stored day/night array; held clocks require a matched native read witness, and updated clocks are the hour's derivation from content (`recorder derived` compares it with recorded clocks)",
        "WeatherProvinces.clock: census only",
    );
    report.limitation(
        "the manager serializer omits the graphics grid, refresh interval and refresh identity; retained boundary and local fade require matched native graphics witnesses",
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
    // The weather region writer tests each complete i64 before emission, omitting exactly zero.
    let expected = saved.map_or(0, |value| value.raw);
    report.scalar_checks += 1;
    if captured != expected {
        report.difference(format!("{label}.{field}"), captured, expected);
    }
}

fn truth(report: &mut Report, label: &str, captured: u8, saved: Option<bool>) {
    // The region and weather writers omit exactly zero; boolean writing spells every nonzero byte as yes.
    let expected = saved.unwrap_or(false);
    if (captured != 0) != expected {
        report.difference(label, captured, expected);
    }
}

#[cfg(test)]
mod tests;
