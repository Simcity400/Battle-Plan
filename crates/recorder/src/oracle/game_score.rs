//! Saved operands of the current endgame getters (1406D8DE0 and its parts).
//! Runtime headers, air-site access and selected stat caches are not saved.
use std::{collections::BTreeMap, io};

use installed_content::save::{PartialMapMembership, ProvinceMembership, Save};
use replay::{
    frame::DecodedFrame,
    rows::{CountryGameScoreInputRow, CountryRow, tables},
};

use super::{Report, complete, invalid};

fn country_inputs<'a>(
    frame: &'a DecodedFrame,
    links: &BTreeMap<u32, &'a CountryRow>,
    report: &mut Report,
) -> io::Result<BTreeMap<&'a [u8], &'a CountryGameScoreInputRow>> {
    let mut rows = BTreeMap::new();
    for row in complete::<tables::CountryGameScoreInputs>(frame)? {
        // Write@1401F4185/1401F4190 starts at physical slot one. The
        // authenticated reserved object has neither a save row nor a link.
        if row.country == 0 {
            report.limitation(
                "CGameState::Write@1401F4185/1401F4190 omits reserved slot zero; Countries and CountryLinks intentionally contain only positive identities",
                "CountryGameScoreInputs reserved object: unsaved roster and land operands",
            );
            continue;
        }
        let tag = links
            .get(&row.country)
            .map(|country| country.tag.as_slice())
            .ok_or_else(|| {
                invalid(format!(
                    "game score country object {} has no recorded identity link",
                    row.country
                ))
            })?;
        if rows.insert(tag, row).is_some() {
            return Err(invalid("duplicate resolved game score country"));
        }
    }
    Ok(rows)
}

pub(super) fn rosters(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = country_inputs(frame, links, report)?;
    let countries = report.canonical_countries(&saved.countries);
    let country_count = countries.len();
    for (tag, _) in countries {
        let label = format!("{}.game_score", String::from_utf8_lossy(tag));
        let Some(row) = actual.remove(tag.as_slice()) else {
            report.difference(label, "absent", "saved country present");
            continue;
        };
        let divisions = saved
            .counter_inputs
            .division_orders
            .get(tag)
            .ok_or_else(|| invalid("save lacks country division roster witness"))?;
        let saved_fleets = saved
            .counter_inputs
            .fleet_orders
            .get(tag)
            .ok_or_else(|| invalid("save lacks country fleet roster witness"))?;
        let army = i32::try_from(divisions.len())
            .map_err(|_| invalid("saved division roster exceeds native i32"))?;
        if row.army_count != Some(army) {
            report.difference(
                format!("{label}.army_count"),
                (&row.army_count, &row.army_error),
                Some(army),
            );
        }
        let fleets = row
            .fleets
            .iter()
            .map(|fleet| fleet.fleet)
            .collect::<Vec<_>>();
        if fleets != *saved_fleets {
            report.difference(format!("{label}.fleet_order"), &fleets, saved_fleets);
        }
        let mut navy = 0_i32;
        for &fleet in saved_fleets {
            let forces = &saved
                .counter_inputs
                .fleets
                .get(&fleet)
                .ok_or_else(|| invalid("save lacks fleet task-force roster witness"))?
                .task_forces;
            let count = i32::try_from(forces.len())
                .map_err(|_| invalid("saved task-force roster exceeds native i32"))?;
            // CalculateNavyScore@1406D93E0 adds the headers in i32 EDI.
            navy = navy.wrapping_add(count);
        }
        if row.navy_count != Some(navy) {
            report.difference(
                format!("{label}.navy_count"),
                (&row.navy_count, &row.navy_error),
                Some(navy),
            );
        }
        for fleet in &row.fleets {
            if let Some(saved_fleet) = saved.counter_inputs.fleets.get(&fleet.fleet) {
                let count = i32::try_from(saved_fleet.task_forces.len())
                    .map_err(|_| invalid("saved task-force roster exceeds native i32"))?;
                if fleet.task_forces != Some(count) {
                    report.difference(
                        format!("{label}.fleet {}.task_forces", fleet.fleet),
                        (&fleet.task_forces, &fleet.error),
                        Some(count),
                    );
                }
            } else {
                report.difference(
                    format!("{label}.fleet {}", fleet.fleet),
                    "present",
                    "absent",
                );
            }
        }
    }
    for tag in actual.keys() {
        report.difference(
            format!("{}.game_score", String::from_utf8_lossy(tag)),
            "present",
            "country absent",
        );
    }
    report.compared("CountryGameScoreInputs saved rosters", country_count);
    report.limitation("CCountry::Serialize@14070C580 and CFleet::Serialize@140D4A390 save roster bodies, not raw header identity/capacity. Division order beyond the first saved handle has no captured witness; existing unit comparisons certify membership, while FleetCounterInputs certifies saved task-force order independently when available", "CountryGameScoreInputs raw headers / division order");
    report.limitation("the save parser refuses duplicate full fleet/task-force definitions rather than discarding them. Normal AddFleet@1406F2920 rejects an already-listed pointer, but no exhaustive runtime uniqueness proof exists; repeated captured fleet occurrences are not certified by this save oracle", "CountryGameScoreInputs repeated fleet occurrences");
    report.limitation("CStrategicAir::Serialize@140C57230 saves ordered pools and CAirWingPool::Serialize@140F53E70 saves ordered wings. AirWings.pool_ordinal compares only the flattened per-owner wing order and each wing's saved base; empty pools and boundaries/order between pools sharing a base have no distinct captured witness. The save does not hold the country strategic-air pointer state, first-site registration order/repeats, or the access predicate used by CalculateAirScore@1406D8560", "CountryGameScoreInputs.air_state / first_sites / access / air pool grouping");
    Ok(())
}

pub(super) fn land(
    frame: &DecodedFrame,
    saved: &Save,
    membership: &PartialMapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let unknown = membership
        .provinces
        .iter()
        .filter_map(|(&id, membership)| {
            matches!(membership, ProvinceMembership::Unknown(_)).then_some(id)
        })
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return Err(invalid(format!(
            "game score land count needs complete province membership; {} unknown provinces: {unknown:?}",
            unknown.len()
        )));
    }
    let mut counts = BTreeMap::<&[u8], i32>::new();
    // CProvince::Serialize@140E6CA90 defaults omission to the state's OWNER,
    // even where its controller differs. Reuse the shared saved-control join.
    let control = saved.province_control(membership)?;
    for province in &control {
        if let Some(controller) = province.controller.as_deref() {
            let count = counts.entry(controller).or_default();
            *count = count.wrapping_add(1);
        }
    }
    let mut actual = country_inputs(frame, links, report)?;
    let countries = report.canonical_countries(&saved.countries);
    let country_count = countries.len();
    for (tag, _) in countries {
        let Some(row) = actual.remove(tag.as_slice()) else {
            report.difference(
                format!("{}.game_score.land_count", String::from_utf8_lossy(tag)),
                "absent",
                "saved country present",
            );
            continue;
        };
        let expected = counts.get(tag.as_slice()).copied().unwrap_or(0);
        if row.land_count != Some(expected) {
            report.difference(
                format!("{}.game_score.land_count", String::from_utf8_lossy(tag)),
                (&row.land_count, &row.land_error),
                Some(expected),
            );
        }
    }
    for tag in actual.keys() {
        report.difference(
            format!("{}.game_score.land_count", String::from_utf8_lossy(tag)),
            "present",
            "country absent",
        );
    }
    report.compared("CountryGameScoreInputs.land_count", country_count);
    report.limitation("CalculateProvinceScore@1406E4B80 reads Country+0x434, which the country writer does not save directly. Saved controller inputs check the maintained-count invariant, including owner-default omissions, not a reset/stale header's exact value; any count difference still fails and needs matched native evidence", "CountryGameScoreInputs.land_count stored-header invariant");
    Ok(())
}

pub(super) fn wings(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let mut actual = BTreeMap::new();
    for row in complete::<tables::AirWingGameScoreStats>(frame)? {
        if row.id == 0 || actual.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate game score air wing"));
        }
    }
    for (&id, expected) in &saved.air_wings {
        let Some(row) = actual.remove(&id) else {
            report.difference(format!("game score air wing {id}"), "absent", "present");
            continue;
        };
        if row.planes != Some(expected.planes) {
            report.difference(
                format!("game score air wing {id}.planes"),
                (&row.planes, &row.planes_error),
                Some(expected.planes),
            );
        }
    }
    for id in actual.keys() {
        report.difference(format!("game score air wing {id}"), "present", "absent");
    }
    report.compared("AirWingGameScoreStats.planes", saved.air_wings.len());
    report.limitation("CAirWing::Serialize@140F539B0 writes count (0x29EA), equipment (0x2F4E) and mission (0x2CBA); existing AirWings and WingEquipment comparisons certify saved count, mission masks and equipment. It omits the selected stat cache read by 140F4B740 -> 140F4B7F0 -> 140F49280 and mission-index result used by CalculateAirScore@1406D8560", "AirWingGameScoreStats.stat_raw / mission_index");
    Ok(())
}

pub(super) fn multipliers(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    report.limitation("CalculateIndustryScore@1406E4760 reads CountryFactories' effective naval/civilian/military totals and civilian/military transfers. CProductionStatus::Serialize@140E5D3B0 omits these runtime totals; RecalculateFactoryTotals@140E57F10 rebuilds them. The existing factories oracle compares only saved building inputs, so a fresh save does not certify Industry's effective factory operands", "Game score Industry / CountryFactories effective totals and transfers");
    let [row] = complete::<tables::GameScoreMultipliers>(frame)? else {
        return Err(invalid("game score multipliers must contain one row"));
    };
    if row.id != 0 {
        return Err(invalid(
            "game score multipliers row has nonzero singleton id",
        ));
    }
    report.limitation("the six NCountry *_SCORE_MULTIPLIER values are runtime NDefines fixed5 words consumed by the endgame getters (1406D8DE0, 1406D8920, 1406D93E0, 1406D8560, 1406E4760, 1406E4B80), with no established save fields; compare with matched native score cells and tooltips, not guessed defaults", "GameScoreMultipliers");
    Ok(())
}

#[cfg(test)]
mod tests;
