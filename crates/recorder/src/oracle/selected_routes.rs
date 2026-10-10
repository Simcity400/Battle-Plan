//! Selected-route suffix checks at the exact compared hour.
//!
//! Only Army exile truth has an independent text-save counterpart. Pass-local
//! area labels, graph order, effective commander modifiers and enemy occurrence
//! order receive internal checks, never a saved-field acceptance count.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use replay::rows::{ProvinceAreaInputs, Recorded, SelectedUnitInputs};

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    geometry: ContentRead<'_, installed_content::province_geometry::ProvinceGeometry>,
    report: &mut Report,
) {
    report.attempt("Provinces selected route consistency", |r| {
        areas(frame, geometry, r)
    });
    report.attempt("DivisionMovement selected route", |r| {
        units(frame, saved, r)
    });
    report.attempt("CountryWars selected route consistency", |r| {
        enemies(frame, r)
    });
}

fn current<'a, T>(
    report: &mut Report,
    field: &'a Recorded<Option<T>>,
    name: &str,
    label: &str,
    hour: i32,
    sample_hour: impl FnOnce(&T) -> i32,
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
    if captured_hour != hour {
        report.difference(format!("{label}.sample_hour"), captured_hour, hour);
        return None;
    }
    Some(value)
}

fn required<T>(report: &mut Report, label: &str, value: &Option<T>) {
    if value.is_none() {
        report.difference(label, "capture refused", "required native operand");
    }
}

fn areas(
    frame: &DecodedFrame,
    geometry: ContentRead<'_, installed_content::province_geometry::ProvinceGeometry>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = frame.complete::<tables::Provinces>()?;
    let content = geometry.as_ref().ok().copied();
    let mut content_demanded = false;
    let mut packets = BTreeMap::<u32, &ProvinceAreaInputs>::new();
    for row in rows {
        let label = format!("Provinces {}.selected_areas", row.id);
        let Some(packet) = current(
            report,
            &row.selected_areas,
            "Provinces.selected_areas",
            &label,
            frame.hour,
            |packet| packet.sample_hour,
        ) else {
            continue;
        };
        if packets.insert(row.id, packet).is_some() {
            return Err(invalid("duplicate selected-route province identity"));
        }
        required(report, &format!("{label}.owner"), &packet.owner);
        for (name, area) in [
            ("controller", Some(packet.controller)),
            ("owner", packet.owner),
        ] {
            if area.is_some_and(|area| area > row.id) {
                report.difference(
                    format!("{label}.{name} label"),
                    area,
                    "null or minimum province ID of the pointer group",
                );
            }
        }
        if packet.node.is_some() && (packet.controller == 0 || packet.controller != row.id) {
            report.difference(
                format!("{label}.node placement"),
                packet.controller,
                "one payload at its nonnull controller-area label province",
            );
        }
    }

    let mut nonsea_counts = BTreeSet::new();
    for (&province, packet) in &packets {
        let label = format!("Provinces {province}.selected_areas");
        for (name, area) in [
            ("controller", Some(packet.controller)),
            ("owner", packet.owner),
        ] {
            let Some(area) = area.filter(|area| *area != 0) else {
                continue;
            };
            match packets.get(&area) {
                Some(root) => {
                    let root_label = if name == "controller" {
                        Some(root.controller)
                    } else {
                        root.owner
                    };
                    if root_label != Some(area) {
                        report.difference(format!("{label}.{name} root"), root_label, Some(area));
                    }
                }
                None if rows.binary_search_by_key(&area, |row| row.id).is_ok() => {
                    report.limitation("selected area label joins an unavailable or historical province suffix; the group cannot be certified", format!("{label}.{name} root {area}"));
                }
                None => report.difference(
                    format!("{label}.{name} root"),
                    area,
                    "present province in the authenticated recorded domain",
                ),
            }
        }
        if packet.controller != province || packet.controller == 0 {
            continue;
        }
        required(report, &format!("{label}.node"), &packet.node);
        let Some(node) = &packet.node else {
            continue;
        };
        required(
            report,
            &format!("{label}.node.first_land"),
            &node.first_land,
        );
        required(
            report,
            &format!("{label}.node.nonsea_index"),
            &node.nonsea_index,
        );
        required(
            report,
            &format!("{label}.node.nonsea_count"),
            &node.nonsea_count,
        );
        required(
            report,
            &format!("{label}.node.land_count"),
            &node.land_count,
        );
        required(
            report,
            &format!("{label}.node.neighbours"),
            &node.neighbours,
        );
        if let Some(first) = node.first_land
            && rows.binary_search_by_key(&first, |row| row.id).is_err()
        {
            report.difference(
                format!("{label}.node.first_land"),
                first,
                "present authenticated province identity",
            );
        }
        if let Some(count) = node.nonsea_count {
            nonsea_counts.insert(count);
            content_demanded = true;
        }
        if node.nonsea_index.is_some_and(|index| index < 0) {
            report.limitation("CDF3F0 accepts a known negative start nonsea index as native false before demanding neighbours; the same recorded node reached as a neighbour remains unsupported. The oracle has no selected start/target and cannot certify that reach or route result", format!("{label}.node.nonsea_index negative native state"));
        }
        if let Some(index) = node.nonsea_index.filter(|index| *index >= 0) {
            content_demanded = true;
            // CDF3F0 sizes its visited bitset from CMap+234, not the land-only
            // GameState+6B8 (1401CD490) used for its temporary area records.
            if let Some(geometry) = content {
                let count = geometry.nonsea_indices.len();
                if index as usize >= count {
                    report.difference(
                        format!("{label}.node.nonsea_index"),
                        index,
                        format!("nonnegative native nonsea index below {count}"),
                    );
                } else if let Some(first) = node.first_land
                    && rows.binary_search_by_key(&first, |row| row.id).is_ok()
                {
                    let expected = geometry.nonsea_indices.get(&first).copied();
                    if expected != Some(index) {
                        report.difference(
                            format!("{label}.node.nonsea_index for province {first}"),
                            Some(index),
                            expected,
                        );
                    }
                }
            }
        }
        if let Some(neighbours) = &node.neighbours {
            // Iterate occurrences. Do not sort, deduplicate or derive a graph
            // from saved province/state ownership; each is a distinct domain.
            for (ordinal, &target) in neighbours.iter().enumerate() {
                if target == 0
                    || packets
                        .get(&target)
                        .is_some_and(|root| root.controller != target)
                {
                    report.difference(
                        format!("{label}.node.neighbours[{ordinal}]"),
                        target,
                        "nonnull authenticated controller-area root label",
                    );
                } else if !packets.contains_key(&target) {
                    if rows.binary_search_by_key(&target, |row| row.id).is_ok() {
                        report.limitation("selected area neighbour joins an unavailable or historical province suffix; its graph payload cannot be certified", format!("{label}.node.neighbours[{ordinal}] {target}"));
                    } else {
                        report.difference(
                            format!("{label}.node.neighbours[{ordinal}]"),
                            target,
                            "present authenticated controller-area root label",
                        );
                    }
                }
            }
        }
    }
    if nonsea_counts.len() > 1 {
        report.difference(
            "Provinces.selected_areas nonsea_count consistency",
            nonsea_counts,
            "one GameState+6B8 value in the same capture",
        );
    } else if let Some(&count) = nonsea_counts.first()
        && let Some(geometry) = content
        && count as usize != geometry.land.len()
    {
        report.difference(
            "Provinces.selected_areas GameState+6B8 land count",
            count as usize,
            geometry.land.len(),
        );
    }
    if !packets.is_empty() {
        report.limitation("D0 controller-area and D8 owner-area labels are pointer identities confined to one capture, not saved controller/owner tags. CDF3F0 first-land/nonsea indices, signed land counts and ordered repeated neighbours have no text-save counterpart. Recording joins do not prove the original pointer association, list lifetime, root census completeness or native graph order. An independent matched-hour native witness remains required", "Provinces.selected_areas internal consistency only");
    }
    if content_demanded {
        required_context("verified province geometry", &geometry)?;
    }
    Ok(())
}

fn units(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::DivisionMovement>()?;
    let mut seen = BTreeSet::new();
    let mut compared = 0;
    for row in rows {
        if row.division == 0 || !seen.insert(row.division) {
            return Err(invalid(
                "null or duplicate selected-route division identity",
            ));
        }
        let label = format!("DivisionMovement {}.selected", row.division);
        let Some(packet) = current(
            report,
            &row.selected,
            "DivisionMovement.selected",
            &label,
            frame.hour,
            |packet| packet.sample_hour,
        ) else {
            continue;
        };
        required(report, &format!("{label}.exiled"), &packet.exiled);
        if let Some(raw) = packet.exiled {
            if let Some(saved) = saved.divisions.get(&row.division) {
                report.scalar_checks += 1;
                if (raw != 0) != saved.presentation.exile {
                    report.difference(
                        format!("{label}.exiled saved truth"),
                        raw != 0,
                        saved.presentation.exile,
                    );
                }
                compared += 1;
            } else {
                report.difference(format!("{label}.saved division"), "present", "absent");
            }
        }
        // BEAA60 exits on a nonpositive signed combat count before selecting
        // a commander, so absence is legitimate for that branch.
        if row.combat_count as i32 > 0 {
            retreat(frame, report, &label, packet);
        }
        report.limitation("CArmy::Serialize@140C80E00 calls secondary CUnit::Serialize@140BF7010; BF7459..BF746E saves primary+2AE as exile (token2D0D) only when nonzero, through Boolean writer1424AE850/1424AFDC0. Only zero/nonzero truth is independently compared. DE2730 XORs the raw byte with one; a truth match cannot certify raw nonzero magnitude", format!("{label}.exiled raw byte"));
        report.limitation("BEAA60's C5F0E0/C0AB20 commander selection and independently effective keys297/298 are current runtime accumulators without text-save counterparts. Saved groups, traits or daily retreat flags cannot certify these values; an independent matched-hour native witness remains required", format!("{label}.commander/retreat internal checks only"));
    }
    if compared != 0 {
        report.compared("DivisionMovement.selected exile saved truth", compared);
    }
    Ok(())
}

fn retreat(frame: &DecodedFrame, report: &mut Report, label: &str, packet: &SelectedUnitInputs) {
    required(report, &format!("{label}.commander"), &packet.commander);
    match packet.commander {
        None => {}
        Some(false) => {
            if packet.retreat != [Some(0); 2] {
                report.difference(
                    format!("{label}.retreat null commander"),
                    packet.retreat,
                    [Some(0); 2],
                );
            }
        }
        Some(true) => {
            if let Err(reason) =
                replay::fixed_layout::validate_recording(frame, replay::fixed_layout::RETREAT_BANS)
            {
                report.difference(
                    format!("{label}.retreat fixed layout"),
                    reason,
                    "authenticated recorded modifier names",
                );
            }
            for (ordinal, value) in packet.retreat.iter().enumerate() {
                required(report, &format!("{label}.retreat[{ordinal}]"), value);
            }
        }
    }
}

fn enemies(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    for row in frame.complete::<tables::CountryWars>()? {
        // Reserved physical receiver zero is an authentic row, not absence.
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
        // Exceptional legacy rows and this suffix share the exact original
        // +98 read. This only checks writer consistency, never save order.
        if row.enemies_read != 0 && row.enemies != packet.entries {
            report.difference(
                format!("{label}.legacy shared vector"),
                &packet.entries,
                &row.enemies,
            );
        }
        report.limitation("Diplomacy+98 native enemy occurrence order, repeats and raw zero entries are not saved. WarRelations and saved war sides certify separate sets, not this vector. The current suffix is checked for freshness and agreement with an exceptional prefix that shares its read only; an independent matched-hour native witness remains required", label);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
