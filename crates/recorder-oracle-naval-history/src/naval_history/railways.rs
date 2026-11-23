use std::{collections::BTreeMap, io};

use content_reader_save::naval_history as saved;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;

use super::{Report, complete, equal, invalid};

pub fn compare_railway_buildings(
    frame: &DecodedFrame,
    saved: &content_reader_save::construction::Pack,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::RailCooldowns>(frame)?;
    for row in rows {
        compare_selected_building(frame.hour, row, saved, names, report)?;
        let label = format!("railway province {}.building", row.province);
        let Some(value) = report.recorded(&row.building, "RailCooldowns.building", &label) else {
            continue;
        };
        let Some(value) = value else {
            if row.cooldown_ticks > 0 {
                report.limitation("positive cooldown proves the selected-building getter was not reached; its absent domain is not an observed null building", &label);
            } else if !complete::<tables::RailEdges>(frame)?
                .iter()
                .any(|edge| edge.from == row.province)
            {
                report.limitation("complete directed railway pairs contain no source edge, so the selected-building getter was not reached; absent domain is not an observed null building", &label);
            } else {
                report.difference(&label, "unavailable", "required selected-building state");
            }
            continue;
        };
        let Some(health) = value.building else {
            report.limitation("the rail-info's observed null selected-building pointer is runtime state; independently saved status buildings do not certify this cache's null-to-absence equivalence", &label);
            continue;
        };
        let name = *names
            .get(&engine_native_parts::cbuilding_template::RAIL_WAY_TOKEN)
            .ok_or_else(|| invalid("railway building token has no recorded spelling"))?;
        let Some(expected) = saved.province_buildings.get(&(row.province, name.to_vec())) else {
            report.difference(&label, health, "absent from saved province buildings");
            continue;
        };
        for (field, actual, expected) in [
            ("level", health.level, expected.level),
            ("healthy", health.healthy, expected.healthy_level),
        ] {
            report.scalar_checks += 1;
            equal(report, format!("{label}.{field}"), actual, expected);
        }
        match health.partial {
            Some(value) => report.scalar(&label, "partial", Some(value), Some(&expected.partial_health)),
            None if health.level != 0 && health.level != health.healthy => {
                report.difference(format!("{label}.partial"), "unavailable on damaged branch", expected.partial_health.raw);
            }
            None => report.limitation("the native health getter does not consume partial health at zero level or equal total/healthy levels; that saved scalar is not compared on this branch", format!("{label}.partial")),
        }
    }
    report.compared("RailCooldowns.building", rows.len());
    Ok(())
}

fn compare_selected_building(
    hour: i32,
    row: &replay_data::rows::RailCooldownRow,
    saved: &content_reader_save::construction::Pack,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let replay_data::rows::Recorded::Value(Some(inputs)) = &row.selection else {
        return Ok(());
    };
    if row.selection_hour != Some(hour).into() {
        return Ok(());
    }
    let label = format!("railway province {}.selection.building_level", row.province);
    let Some(level) = inputs.building_level else {
        if row.cooldown_ticks == 0
            && (inputs.pair_count.is_some_and(|count| count != 0)
                || inputs
                    .levels
                    .as_ref()
                    .is_some_and(|levels| levels.iter().any(|&level| level > 0)))
        {
            report.difference(&label, "unavailable", "required selected-building level");
        } else {
            report.limitation("selected railway building-level read is outside the proved current gates; absence is not a native null building", label);
        }
        return Ok(());
    };
    let Some(level) = level else {
        report.limitation("selected railway null building is a runtime cache pointer, not proof of an absent saved status building", label);
        return Ok(());
    };
    let name = *names
        .get(&engine_native_parts::cbuilding_template::RAIL_WAY_TOKEN)
        .ok_or_else(|| invalid("railway building token has no recorded spelling"))?;
    report.compared("RailCooldowns.selection.building_level", 1);
    if let Some(expected) = saved.province_buildings.get(&(row.province, name.to_vec())) {
        report.scalar_checks += 1;
        equal(report, label, level, expected.level);
    } else {
        report.difference(label, level, "absent from saved province buildings");
    }
    Ok(())
}

pub(super) fn compare_railways(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let expected = saved
        .railways
        .as_ref()
        .ok_or_else(|| invalid("save lacks railway manager"))?;
    let mut rows = BTreeMap::new();
    for row in complete::<tables::RailCooldowns>(frame)? {
        if rows.insert(row.province, row).is_some() {
            return Err(invalid("duplicate recorded railway province"));
        }
    }
    let mut compared = 0;
    let mut levels_compared = 0;
    for (&province, expected) in expected {
        let label = format!("railway province {province}");
        let Some(actual) = rows.remove(&province) else {
            report.difference(label, "absent", "present");
            continue;
        };
        compared += 1;
        let selection_label = format!("{label}.selection");
        if let Some(selection) = report.recorded(
            &actual.selection,
            "RailCooldowns.selection",
            &selection_label,
        ) {
            match selection {
                None => report.difference(
                    &selection_label,
                    "unavailable",
                    "current selected railway inputs",
                ),
                Some(inputs) => {
                    if actual.selection_hour != Some(frame.hour).into() {
                        report.difference(
                            format!("{label}.selection_hour"),
                            actual.selection_hour,
                            frame.hour,
                        );
                    } else {
                        match &inputs.levels {
                            None => report.difference(
                                format!("{selection_label}.levels"),
                                "unavailable",
                                &expected.levels,
                            ),
                            Some(levels) => {
                                report.scalar_checks += levels.len().max(expected.levels.len());
                                equal(
                                    report,
                                    format!("{selection_label}.levels"),
                                    levels,
                                    &expected.levels,
                                );
                                levels_compared += 1;
                            }
                        }
                        match inputs.pair_count {
                            None => report.difference(format!("{selection_label}.pair_count"), "unavailable", "raw native signed pair count"),
                            Some(count) => report.limitation("the railway writer serializes every indexed railway level but no pair count; railway loading rebuilds only positive pairs and railway editing can retain nonpositive pairs, so a save cannot certify the raw count", format!("{selection_label}.pair_count={count}")),
                        }
                    }
                }
            }
        }
        if let Some(cooldown) = expected.cooldown {
            equal(
                report,
                format!("{label}.cooldown"),
                actual.cooldown_ticks,
                cooldown,
            );
        } else if actual.cooldown_ticks > 0 {
            report.difference(
                format!("{label}.cooldown"),
                actual.cooldown_ticks,
                "omitted signed-nonpositive value",
            );
        } else {
            report.limitation("the railway writer omits signed-nonpositive cooldown; absence verifies that predicate but not the exact native integer", label);
        }
    }
    for province in rows.keys() {
        report.difference(format!("railway province {province}"), "present", "absent");
    }
    report.compared("RailCooldowns", compared);
    report.compared("RailCooldowns.selection.levels", levels_compared);
    Ok(())
}

pub fn compare_rail_edges(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    adjacency: &saved::RailwayAdjacency,
    report: &mut Report,
) -> io::Result<()> {
    let expected = saved
        .railways
        .as_ref()
        .ok_or_else(|| invalid("save lacks railway manager"))?;
    let mut rows = BTreeMap::new();
    for row in complete::<tables::RailEdges>(frame)? {
        if rows.insert((row.from, row.to), row.level).is_some() {
            return Err(invalid("duplicate recorded directed railway edge"));
        }
    }
    let mut compared = 0;
    let mut nonpositive = 0;
    for (&source, info) in expected {
        let neighbors = adjacency.neighbors(source).ok_or_else(|| {
            invalid(format!(
                "saved railway province {source} lacks verified content adjacency"
            ))
        })?;
        if neighbors.len() != info.levels.len() {
            return Err(invalid(format!(
                "railway province {source} has {} saved levels but {} verified content neighbors",
                info.levels.len(),
                neighbors.len()
            )));
        }
        for (&target, &level) in neighbors.iter().zip(&info.levels) {
            let label = format!("railway edge {source}->{target}");
            if let Some(actual) = rows.remove(&(source, target)) {
                equal(report, format!("{label}.level"), actual, level);
                compared += 1;
            } else if level > 0 {
                report.difference(label, "absent", level);
            }
            if level <= 0 {
                nonpositive += 1;
            }
        }
    }
    for ((source, target), level) in rows {
        report.difference(
            format!("railway edge {source}->{target}"),
            level,
            "absent from saved railway adjacency",
        );
    }
    report.compared("RailEdges", compared);
    if nonpositive != 0 {
        report.limitation("saved nonpositive railway levels do not uniquely encode sorted-pair presence; present recorded pairs compare their exact signed level, and every positive saved edge is required", format!("RailEdges: {nonpositive} nonpositive saved neighbor positions"));
    }
    Ok(())
}
