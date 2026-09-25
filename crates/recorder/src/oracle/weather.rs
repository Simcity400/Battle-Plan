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
        "WeatherRegions.light_rain, heavy_rain, snow, blizzard, sandstorm, arctic_water",
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
mod tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{TableId, TableRows, WeatherProvinceRow, WeatherRegionRow},
    };

    #[test]
    fn typed_weather_comparison_keeps_exact_words_and_refuses_unavailable_rows() {
        let saved = installed_content::save::parse(
            br#"HOI4txt
date=1936.1.1.1 game_unique_id=campaign game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 version=build
countries={} states={} provinces={}
weather={
    provinces={0={province=0 snow=90071992547.40993 temperature=1
        temperature_offset=-0.00001 mud=yes custom_modifiers={mud=-1 mud=7}}}
    regions={0={region=0 temperature=-90071992547.40993
        next_weather_change=1936.1.2.1 rain_light=yes}}
}"#,
        )
        .unwrap();
        let hour = saved.date.engine_hour().unwrap();
        let mut frame = DecodedFrame {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "outside weather comparison".into(),
            }),
        };
        frame.tables[TableId::WeatherProvinces.index()] =
            TableView::Complete(TableRows::WeatherProvinces(vec![WeatherProvinceRow {
                id: 0,
                snow: 9_007_199_254_740_993,
                permanent_snow: 4_000_000,
                display_snow: 128,
                display_mud: 255,
            }]));
        frame.tables[TableId::WeatherRegions.index()] =
            TableView::Complete(TableRows::WeatherRegions(vec![WeatherRegionRow {
                id: 0,
                light_rain: 255,
                ..Default::default()
            }]));
        let mut report = Report::default();
        compare(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 1);
        assert_eq!(report.limitations.len(), 3);

        for (table, name, scalar_checks) in [
            (TableId::WeatherProvinces, "WeatherProvinces", 0),
            (TableId::WeatherRegions, "WeatherRegions", 1),
        ] {
            let original = frame.tables[table.index()].clone();
            frame.tables[table.index()] = if table == TableId::WeatherProvinces {
                TableView::Truncated {
                    rows: TableRows::WeatherProvinces(Vec::new()),
                    missing: 1,
                }
            } else {
                TableView::Unavailable {
                    reason: "weather read failed".into(),
                }
            };
            let mut report = Report::default();
            assert!(compare(&frame, &saved, &mut report).is_err());
            assert!(report.differences.is_empty());
            assert_eq!(report.scalar_checks, scalar_checks);
            assert_eq!(report.failed_comparisons.len(), 1);
            assert!(report.failed_comparisons.contains_key(name));
            assert_eq!(report.compared_tables.len(), 1);
            assert!(!report.compared_tables.contains_key(name));
            frame.tables[table.index()] = original;
        }

        let TableView::Complete(TableRows::WeatherProvinces(rows)) =
            &mut frame.tables[TableId::WeatherProvinces.index()]
        else {
            unreachable!();
        };
        rows[0].snow -= 1;
        let mut report = Report::default();
        compare(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);

        frame.tables[TableId::WeatherRegions.index()] = TableView::Unavailable {
            reason: "region read failed".into(),
        };
        assert!(compare(&frame, &saved, &mut Report::default()).is_err());
    }

    #[test]
    fn weather_sparse_zero_and_nonzero_flags_follow_serializer_information() {
        let mut report = Report::default();
        sparse_scalar(&mut report, "province 0", "water", 0, None);
        truth(&mut report, "province 0.mud", 0, None);
        truth(&mut report, "region 0.snow", 255, Some(true));
        assert!(report.differences.is_empty());
        sparse_scalar(&mut report, "province 0", "snow", -1, None);
        truth(&mut report, "region 0.snow", 255, None);
        truth(&mut report, "region 1.snow", 0, Some(true));
        assert_eq!(report.differences.len(), 3);
        assert_eq!(report.scalar_checks, 2);
    }

    #[test]
    fn weather_dense_census_includes_zero_and_rejects_duplicates_or_holes() {
        assert!(dense_slot(0, 0, "weather").is_ok());
        assert!(dense_slot(1, 1, "weather").is_ok());
        assert!(dense_slot(0, 1, "weather").is_err());
        assert!(dense_slot(1, 0, "weather").is_err());
        assert!(dense_slot(1, 2, "weather").is_err());
    }
}
