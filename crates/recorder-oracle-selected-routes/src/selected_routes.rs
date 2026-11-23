
use std::collections::{BTreeMap, BTreeSet};

use replay_data::{rows::DivisionStatusRow, rows::Recorded};

use super::*;

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    geometry: ContentRead<'_, content_reader_map::province_geometry::ProvinceGeometry>,
    report: &mut Report,
) {
    report.attempt("Provinces selected route areas", |r| {
        areas(frame, geometry, r)
    });
    report.attempt("DivisionStatus selected route exile", |r| {
        exile(frame, save, r)
    });
    report.attempt("CountryWars selected route consistency", |r| {
        enemies(frame, r)
    });
}

fn areas(
    frame: &DecodedFrame,
    geometry: ContentRead<'_, content_reader_map::province_geometry::ProvinceGeometry>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = frame.complete::<tables::Provinces>()?;
    let mut labels = BTreeMap::<u32, (u32, u32)>::new();
    for row in rows {
        let label = format!("Provinces {}", row.id);
        let (Some(controller), Some(owner)) = (
            report.recorded(&row.controller_area, "Provinces.controller_area", &label),
            report.recorded(&row.owner_area, "Provinces.owner_area", &label),
        ) else {
            continue;
        };
        let (Some(controller), Some(owner)) = (*controller, *owner) else {
            report.difference(
                format!("{label}.areas"),
                "capture refused",
                "controller and owner area labels",
            );
            continue;
        };
        if labels.insert(row.id, (controller, owner)).is_some() {
            return Err(invalid("duplicate selected-route province identity"));
        }
        for (name, area) in [("controller_area", controller), ("owner_area", owner)] {
            if area > row.id {
                report.difference(
                    format!("{label}.{name}"),
                    area,
                    "null or the area's smallest member province",
                );
            }
        }
    }
    if labels.is_empty() {
        return Ok(());
    }
    let mut members = BTreeMap::<u32, u32>::new();
    for (&province, &(controller, owner)) in &labels {
        let label = format!("Provinces {province}");
        *members.entry(controller).or_default() += 1;
        for (name, area, index) in [("controller_area", controller, 0), ("owner_area", owner, 1)] {
            if area == 0 {
                continue;
            }
            let root = labels.get(&area).map(|labels| [labels.0, labels.1][index]);
            if root != Some(area) {
                report.difference(format!("{label}.{name} root"), root, Some(area));
            }
        }
    }
    let content = geometry.as_ref().ok().copied();
    let mut nodes = 0;
    for row in rows {
        let Some(&(controller, _)) = labels.get(&row.id) else {
            continue;
        };
        let label = format!("Provinces {}.area_node", row.id);
        let Some(node) = report.recorded(&row.area_node, "Provinces.area_node", &label) else {
            continue;
        };
        let root = controller != 0 && controller == row.id;
        let Some(node) = node else {
            if root {
                report.difference(&label, "capture refused", "the area's node");
            }
            continue;
        };
        if !root {
            report.difference(
                &label,
                "present",
                "only on its nonnull controller area's label province",
            );
            continue;
        }
        nodes += 1;
        match node.first_land {
            None => report.difference(
                format!("{label}.first_land"),
                "capture refused",
                "the representative province",
            ),
            Some(first) => {
                if labels.get(&first).map(|labels| labels.0) != Some(controller) {
                    report.difference(
                        format!("{label}.first_land"),
                        first,
                        "a member province of the area",
                    );
                }
                if let Some(geometry) = content
                    && !geometry.nonsea_indices.contains_key(&first)
                {
                    report.difference(
                        format!("{label}.first_land nonsea index"),
                        first,
                        "a province of the content's nonsea domain",
                    );
                }
            }
        }
        match &node.neighbours {
            None => report.difference(
                format!("{label}.neighbours"),
                "capture refused",
                "the ordered contacts",
            ),
            Some(neighbours) => {
                for (ordinal, &target) in neighbours.iter().enumerate() {
                    let root = labels.get(&target).map(|labels| labels.0);
                    if target == 0 || target == controller || root != Some(target) {
                        report.difference(
                            format!("{label}.neighbours[{ordinal}]"),
                            target,
                            "another controller area's label",
                        );
                    }
                }
            }
        }
    }
    let roots = members.keys().filter(|&&area| area != 0).count();
    if nodes != roots {
        report.limitation(
            "controller areas whose node was refused or absent",
            format!("{} of {roots} areas", roots - nodes.min(roots)),
        );
    }
    if content.is_none() {
        required_context("verified province geometry", &geometry)?;
    }
    report.compared("Provinces selected route areas", labels.len());
    report.limitation("Controller and owner area labels, representatives and contact order are runtime state without a text-save counterpart; these checks prove the recording consistent, not the native graph. An independent matched-hour native witness remains required", "Provinces selected route areas internal consistency only");
    Ok(())
}

fn exile(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::DivisionStatus>()?;
    let mut compared = 0;
    for row in rows {
        let label = format!("DivisionStatus {:#x}.move_flags exile", row.division);
        let Some(saved) = save.divisions.get(&row.division) else {
            report.difference(&label, "present", "saved division");
            continue;
        };
        report.scalar_checks += 1;
        compared += 1;
        let recorded = row.move_flags & DivisionStatusRow::MOVE_EXILED != 0;
        if recorded != saved.presentation.exile {
            report.difference(&label, recorded, saved.presentation.exile);
        }
    }
    if compared != 0 {
        report.compared("DivisionStatus exile saved truth", compared);
    }
    Ok(())
}

fn current<'a, T>(
    report: &mut Report,
    field: &'a Recorded<Option<T>>,
    name: &str,
    label: &str,
    hour: i32,
    sample_hour: impl FnOnce(&T) -> replay_data::rows::PacketHour,
) -> Option<&'a T> {
    let value = report.recorded(field, name, label)?;
    let Some(value) = value else {
        report.difference(
            label,
            "capture refused",
            "required current-hour native inputs",
        );
        return None;
    };
    let captured_hour = sample_hour(value);
    if !captured_hour.fresh_at(hour) {
        report.difference(format!("{label}.sample_hour"), captured_hour, hour);
        return None;
    }
    Some(value)
}

fn enemies(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    for row in frame.complete::<tables::CountryWars>()? {
        if !seen.insert(row.country) {
            return Err(invalid("duplicate selected-route diplomacy receiver"));
        }
        let label = format!("CountryWars {}.ordered", row.country);
        let Some(packet) = current(
            report,
            &row.ordered,
            "CountryWars.ordered",
            &label,
            frame.hour,
            |packet| packet.sample_hour,
        ) else {
            continue;
        };
        if row.enemies_read != 0 && row.enemies != packet.entries {
            report.difference(
                format!("{label}.legacy shared vector"),
                &packet.entries,
                &row.enemies,
            );
        }
        report.limitation("The diplomacy status's native enemy-vector occurrence order, repeats and raw zero entries are not saved. WarRelations and saved war sides certify separate sets, not this vector. The current suffix is checked for freshness and agreement with an exceptional prefix that shares its read only; an independent matched-hour native witness remains required", label);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
