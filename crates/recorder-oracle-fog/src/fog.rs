
use std::{collections::BTreeMap, io};

use content_reader_save::{IntelEntry, Save};
use replay_data::{rows::CountryRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{Report, complete, country_tag, invalid};

type HostPositions<'a> = Vec<(Option<&'a [u8]>, i32)>;

pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete_comparison = report.attempt("fog UnitIntel", |report| {
        compare_unit_intel(frame, saved, report)
    });
    complete_comparison &= report.attempt("fog VolunteerHosts", |report| {
        compare_volunteer_hosts(frame, saved, links, report)
    });
    complete_comparison &= report.attempt("fog StationedUnits", |report| {
        compare_stationed(frame, saved, report)
    });
    complete_comparison &= report.attempt("fog TheatreAreas", |report| {
        compare_areas(frame, saved, report)
    });
    complete_comparison &=
        report.attempt("fog RadarCoverage", |report| record_radar(frame, report));
    complete_comparison &= report.attempt("fog AirBaseRegistrations", |report| {
        record_registrations(frame, report)
    });
    complete_comparison &= report.attempt("state Air container fuel", |report| {
        compare_state_container_fuel(frame, saved, links, report)
    });
    complete_comparison &= report.attempt("fog CarrierAir (recording self-check)", |report| {
        record_carrier(frame, links, report)
    });
    if complete_comparison {
        Ok(())
    } else {
        Err(invalid(
            "fog derivation input comparison incomplete; see per-table errors",
        ))
    }
}

fn ordered<'a, K: Ord + Copy + std::fmt::Debug, R>(
    rows: impl Iterator<Item = (K, u32, &'a R)>,
    label: &str,
) -> io::Result<BTreeMap<K, Vec<&'a R>>> {
    let mut grouped: BTreeMap<K, Vec<&R>> = BTreeMap::new();
    for (parent, ordinal, row) in rows {
        let entries = grouped.entry(parent).or_default();
        if ordinal as usize != entries.len() {
            return Err(invalid(format!(
                "captured {label} {parent:?} has a missing or repeated native ordinal {ordinal}"
            )));
        }
        entries.push(row);
    }
    Ok(grouped)
}

fn compare_unit_intel(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let captured = ordered(
        complete::<tables::UnitIntel>(frame)?
            .iter()
            .map(|row| (row.unit, row.ordinal, row)),
        "unit viewer entries",
    )?;
    let mut expected: BTreeMap<u64, &[IntelEntry]> = BTreeMap::new();
    for (id, division) in &saved.divisions {
        expected.insert(*id, &division.country_intel);
    }
    for (id, task_force) in &saved.task_forces {
        if expected.insert(*id, &task_force.country_intel).is_some() {
            return Err(invalid(
                "saved division and task force share one unit handle",
            ));
        }
    }
    for (id, gun) in &saved.railway_guns {
        if expected.insert(*id, &gun.country_intel).is_some() {
            return Err(invalid(
                "saved railway gun shares a unit handle with another unit",
            ));
        }
    }
    let mut compared = 0;
    for (unit, entries) in &expected {
        let label = format!("unit {unit} country_intel");
        let actual = captured.get(unit).map(Vec::as_slice).unwrap_or(&[]);
        if actual.len() != entries.len() {
            report.difference(format!("{label}.count"), actual.len(), entries.len());
            continue;
        }
        for (ordinal, (actual, expected)) in actual.iter().zip(entries.iter()).enumerate() {
            compared += 1;
            if actual.viewer != expected.viewer as u32 {
                report.difference(
                    format!("{label}[{ordinal}].viewer"),
                    actual.viewer,
                    expected.viewer,
                );
            }
            if i32::from(actual.level) != expected.level {
                report.difference(
                    format!("{label}[{ordinal}].level"),
                    actual.level,
                    expected.level,
                );
            }
        }
    }
    for unit in captured.keys() {
        if !expected.contains_key(unit) {
            report.difference(
                format!("unit {unit} country_intel"),
                "present",
                "no saved unit",
            );
        }
    }
    report.compared("UnitIntel", compared);
    report.limitation(
        "the viewer entry's native decay accumulator is not a recorded field; the save's second triple word is parsed but has no captured counterpart",
        "UnitIntel: viewer and level compare exactly, timer does not",
    );
    report.limitation(
        "a stored viewer key is a country ordinal in the engine's country array, which no table maps back to a tag; ordinals compare raw against the save's own raw word",
        "UnitIntel.viewer",
    );
    Ok(())
}

fn compare_volunteer_hosts(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let captured = ordered(
        complete::<tables::VolunteerHosts>(frame)?
            .iter()
            .map(|row| (row.country, row.position, row)),
        "volunteer host positions",
    )?;
    let mut by_tag: BTreeMap<&[u8], HostPositions<'_>> = BTreeMap::new();
    for (country, rows) in &captured {
        let tag = country_tag(*country, links)?
            .ok_or_else(|| invalid("captured volunteer host row has a zero country"))?;
        let mut positions = Vec::with_capacity(rows.len());
        for row in rows {
            positions.push((country_tag(row.host, links)?, row.deployed_divisions));
        }
        if by_tag.insert(tag, positions).is_some() {
            return Err(invalid(
                "two captured volunteer host countries resolve to one tag",
            ));
        }
    }
    let mut compared = 0;
    for (tag, country) in report.canonical_countries(&saved.countries) {
        let label = format!("{} received_air_volunteer_permission", show(tag));
        let actual = by_tag.remove(tag.as_slice()).unwrap_or_default();
        if actual.len() != country.volunteer_permissions.len() {
            report.difference(
                format!("{label}.count"),
                actual.len(),
                country.volunteer_permissions.len(),
            );
            continue;
        }
        for (position, (actual, expected)) in actual
            .iter()
            .zip(country.volunteer_permissions.iter())
            .enumerate()
        {
            compared += 1;
            if actual.0 != Some(expected.tag.as_slice()) {
                report.difference(
                    format!("{label}[{position}].tag"),
                    actual.0.map(show),
                    show(&expected.tag),
                );
            }
            if actual.1 != expected.count {
                report.difference(
                    format!("{label}[{position}].count"),
                    actual.1,
                    expected.count,
                );
            }
        }
    }
    for tag in by_tag.keys() {
        report.difference(
            format!("{} received_air_volunteer_permission", show(tag)),
            "present",
            "no saved country",
        );
    }
    report.compared("VolunteerHosts", compared);
    report.limitation("the directed effective Air volunteer-capacity modifier is a runtime accumulator result without an established saved scalar", "VolunteerHosts.air_capacity_modifier");
    report.limitation(
        "the host vector stores raw country references and the save writes resolved tags, so two aliases of one country cannot be told apart in the comparison",
        "VolunteerHosts.host",
    );
    Ok(())
}

fn volunteer_theatres(
    saved: &Save,
) -> io::Result<BTreeMap<u64, &content_reader_save::naval_history::LandTheatre>> {
    let Some(countries) = &saved.naval_history.theatre_countries else {
        return Err(invalid(
            "save has no theatre roster; volunteer theatre scope cannot be established",
        ));
    };
    let mut theatres = BTreeMap::new();
    for country in countries.values() {
        for theatre in &country.land {
            if theatre.volunteers_country.is_none() {
                continue;
            }
            if theatres.insert(theatre.id, theatre).is_some() {
                return Err(invalid("saved land theatres share one handle"));
            }
        }
    }
    Ok(theatres)
}

fn compare_stationed(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let captured = ordered(
        complete::<tables::StationedUnits>(frame)?
            .iter()
            .map(|row| (row.theatre, row.ordinal, row)),
        "stationed unit positions",
    )?;
    let expected = volunteer_theatres(saved)?;
    let mut compared = 0;
    for (handle, theatre) in &expected {
        let label = format!("theatre {handle} stationed units");
        let actual = captured.get(handle).map(Vec::as_slice).unwrap_or(&[]);
        if actual.len() != theatre.units.len() {
            report.difference(format!("{label}.count"), actual.len(), theatre.units.len());
            continue;
        }
        for (ordinal, (actual, expected)) in actual.iter().zip(theatre.units.iter()).enumerate() {
            compared += 1;
            if actual.unit != *expected {
                report.difference(format!("{label}[{ordinal}]"), actual.unit, expected);
            }
        }
    }
    for handle in captured.keys() {
        if !expected.contains_key(handle) {
            report.difference(
                format!("theatre {handle} stationed units"),
                "present",
                "no saved volunteer theatre",
            );
        }
    }
    report.compared("StationedUnits", compared);
    Ok(())
}

fn compare_areas(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let captured = ordered(
        complete::<tables::TheatreAreas>(frame)?
            .iter()
            .map(|row| (row.theatre, row.ordinal, row)),
        "controller area positions",
    )?;
    let expected = volunteer_theatres(saved)?;
    let mut compared = 0;
    for (handle, theatre) in &expected {
        let label = format!("theatre {handle} controller areas");
        let actual = captured.get(handle).map(Vec::as_slice).unwrap_or(&[]);
        if actual.len() != theatre.area_representatives.len() {
            report.difference(
                format!("{label}.count"),
                actual.len(),
                theatre.area_representatives.len(),
            );
            continue;
        }
        for (ordinal, (actual, expected)) in actual
            .iter()
            .zip(theatre.area_representatives.iter())
            .enumerate()
        {
            compared += 1;
            if actual.representative != Some(*expected) {
                report.difference(
                    format!("{label}[{ordinal}].representative"),
                    actual.representative,
                    expected,
                );
            }
        }
    }
    for handle in captured.keys() {
        if !expected.contains_key(handle) {
            report.difference(
                format!("theatre {handle} controller areas"),
                "present",
                "no saved volunteer theatre",
            );
        }
    }
    report.compared("TheatreAreas", compared);
    report.limitation(
        "The theatre writer emits one representative province per controller area, never the area's whole membership; the captured province list has no save representation",
        "TheatreAreas.provinces",
    );
    Ok(())
}

fn record_radar(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::RadarCoverage>(frame)?;
    report.compared("RadarCoverage", 0);
    report.limitation(
        "no inspected country serializer path writes the radar coverage vectors, so this table has no save comparison; the recorded rows decode but are not certified against a save",
        format!("RadarCoverage: {} decoded rows, zero save comparisons", rows.len()),
    );
    Ok(())
}

fn record_registrations(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::AirBaseRegistrations>(frame)?;
    report.compared("AirBaseRegistrations", 0);
    report.limitation(
        "The saved country containers do not serialize pool registration order or indexed country slots; state dense-country order and fuel scalars compare separately by the saved base handle and resolved country",
        format!(
            "AirBaseRegistrations: {} decoded rows, zero save comparisons",
            rows.len()
        ),
    );
    Ok(())
}

fn compare_state_container_fuel(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let bases = complete::<tables::AirBases>(frame)?;
    let rows = complete::<tables::AirBaseRegistrations>(frame)?;
    let expected: BTreeMap<_, _> = saved
        .air
        .bases
        .iter()
        .filter(|base| base.state.is_some())
        .map(|base| (base.id, base))
        .collect();
    if expected.len()
        != saved
            .air
            .bases
            .iter()
            .filter(|base| base.state.is_some())
            .count()
    {
        return Err(invalid("saved state air bases repeat one handle"));
    }
    let mut actual = BTreeMap::new();
    for row in rows {
        if !expected.contains_key(&row.base) {
            continue;
        }
        let Some(packet) = report.recorded(
            &row.carrier,
            "AirBaseRegistrations.carrier",
            "state fuel country",
        ) else {
            continue;
        };
        let Some(packet) = packet else {
            return Err(invalid("state fuel container country unavailable"));
        };
        if row.slot == 0 {
            report.limitation(
                "a reserved or unlinked country reference has no authenticated saved tag join",
                format!("state air base {} slot0", row.base),
            );
            continue;
        }
        let tag = country_tag(packet.country, links)?
            .ok_or_else(|| invalid("state fuel country resolves to zero"))?;
        if actual.insert((row.base, tag), row).is_some() {
            return Err(invalid(
                "state air-base registrations share one saved country identity",
            ));
        }
    }
    let mut compared = 0;
    for (&id, base) in &expected {
        if let Some(current) = bases.iter().find(|row| row.id == id)
            && let Some(Some(packet)) = report.recorded(
                &current.carrier,
                "AirBases.carrier",
                "state dense country order",
            )
            && let Some(dense) = packet.registered_countries.as_ref()
        {
            let actual_tags = dense
                .iter()
                .map(|row| country_tag(row.country, links))
                .collect::<io::Result<Vec<_>>>()?
                .into_iter()
                .collect::<Option<Vec<_>>>();
            if let Some(actual_tags) = actual_tags {
                let expected_tags: Vec<_> = base
                    .containers
                    .iter()
                    .map(|row| row.country.as_slice())
                    .collect();
                if actual_tags != expected_tags {
                    report.difference(
                        format!("state air base {id}.countries"),
                        actual_tags,
                        expected_tags,
                    );
                }
                report.compared("AirBases.registered_countries (save)", dense.len());
            }
        }
        let mut visited = std::collections::BTreeSet::new();
        for container in &base.containers {
            let tag = container.country.as_slice();
            let key = (id, tag);
            visited.insert(tag);
            let Some(&row) = actual.get(&key) else {
                report.difference(
                    format!(
                        "state air base {id} country {}",
                        String::from_utf8_lossy(tag)
                    ),
                    "absent",
                    "saved container",
                );
                continue;
            };
            let label = format!(
                "state air base {id} country {}",
                String::from_utf8_lossy(tag)
            );
            for (field, captured, expected) in [
                (
                    "fuel_demand_raw",
                    &row.fuel_demand_raw,
                    &container.fuel_demand,
                ),
                (
                    "fuel_allocated_raw",
                    &row.fuel_allocated_raw,
                    &container.fuel_allocated,
                ),
            ] {
                let Some(captured) = report.recorded(captured, field, &label) else {
                    continue;
                };
                report.scalar(&label, field, *captured, Some(expected));
                compared += 1;
            }
        }
        for tag in visited {
            actual.remove(&(id, tag));
        }
    }
    for ((base, tag), _) in actual {
        report.difference(
            format!(
                "state air base {base} country {}",
                String::from_utf8_lossy(tag)
            ),
            "recorded container",
            "absent from save",
        );
    }
    report.compared("AirBaseRegistrations.fuel (save)", compared);
    Ok(())
}

fn record_carrier(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    report.compared("AirBases.carrier", 0);
    report.compared("AirBaseRegistrations.carrier", 0);
    report.limitation(
        "carrier recording self-check only: dense/indexed registration joins, raw name availability, fuel availability and ordered wing partitions; zero fresh-save or native comparisons",
        "AirBases.carrier, AirBaseRegistrations.carrier",
    );
    report.limitation(
        "Retained host-name bytes and passive current/max query results have no established text-save comparator and require a matched native witness. State-container demand/allocation and dense country occurrences compare separately with the serialized container list",
        "registered_countries, host_name, fuel_demand_raw, fuel_allocated_raw, current_fuel_raw, maximum_fuel_raw",
    );
    report.limitation(
        "carrier pool order, empty pools and signed grouping keys are not certified by the save here; member sub-unit index joins below are recording self-checks. military::air_transport separately compares saved wing-pool identity through recorded content; it supplies no invented save join for empty carrier pools",
        "AirBaseRegistrations.carrier.pools",
    );
    let bases = complete::<tables::AirBases>(frame)?;
    let registrations = complete::<tables::AirBaseRegistrations>(frame)?;
    let by_base: BTreeMap<_, _> = bases.iter().map(|row| (row.id, row)).collect();
    let by_registration: BTreeMap<_, _> = registrations
        .iter()
        .map(|row| ((row.base, row.slot), row))
        .collect();
    if by_base.len() != bases.len() || by_registration.len() != registrations.len() {
        return Err(invalid(
            "duplicate carrier base or indexed registration key",
        ));
    }
    let mut wings = None;
    let mut checks_complete = true;
    for base in bases {
        let label = format!("carrier base {}", base.id);
        let Some(packet) = report.recorded(&base.carrier, "AirBases.carrier", &label) else {
            continue;
        };
        let Some(packet) = packet else {
            checks_complete &= report.attempt(&label, |_| {
                Err(invalid("current carrier base packet acquisition refused"))
            });
            continue;
        };
        if base.state.is_none() {
            checks_complete &= report.attempt(&format!("{label}.host_name"), |report| {
                carrier_input(&packet.host_name, "host_name", report)?;
                Ok(())
            });
        }
        checks_complete &= report.attempt(&format!("{label}.registered_countries"), |report| {
            let dense =
                carrier_input(&packet.registered_countries, "registered_countries", report)?;
            let mut joined = true;
            for (ordinal, entry) in dense.iter().enumerate() {
                joined &= report.attempt(
                    &format!("{label}.registered_countries[{ordinal}]"),
                    |report| {
                        carrier_country(entry.country, entry.slot, links, report)?;
                        let indexed = by_registration
                            .get(&(base.id, entry.slot))
                            .ok_or_else(|| invalid("dense country has no indexed registration"))?;
                        let Some(packet) = report.recorded(
                            &indexed.carrier,
                            "AirBaseRegistrations.carrier",
                            &label,
                        ) else {
                            return Ok(());
                        };
                        let packet = packet.as_ref().ok_or_else(|| {
                            invalid("dense country indexed packet acquisition refused")
                        })?;
                        if packet.country != entry.country {
                            return Err(invalid(
                                "dense country differs from indexed packet raw country",
                            ));
                        }
                        report.compared("CarrierAir.dense_join (recording self-check)", 1);
                        Ok(())
                    },
                );
            }
            if joined {
                Ok(())
            } else {
                Err(invalid("carrier dense joins incomplete"))
            }
        });
    }
    for registration in registrations {
        let label = format!(
            "carrier base {} slot {}",
            registration.base, registration.slot
        );
        let Some(base) = by_base.get(&registration.base) else {
            checks_complete &= report.attempt(&label, |_| {
                Err(invalid("indexed registration has no recorded air base"))
            });
            continue;
        };
        let Some(packet) = report.recorded(
            &registration.carrier,
            "AirBaseRegistrations.carrier",
            &label,
        ) else {
            continue;
        };
        let Some(packet) = packet else {
            checks_complete &= report.attempt(&label, |_| {
                Err(invalid(
                    "current carrier registration packet acquisition refused",
                ))
            });
            continue;
        };
        checks_complete &= report.attempt(&format!("{label}.country"), |report| {
            carrier_country(packet.country, registration.slot, links, report)
        });
        for (field, host, state) in [
            (
                "fuel_demand_raw",
                &packet.fuel_demand_raw,
                &registration.fuel_demand_raw,
            ),
            (
                "fuel_allocated_raw",
                &packet.fuel_allocated_raw,
                &registration.fuel_allocated_raw,
            ),
        ] {
            let value = if base.state.is_some() {
                let Some(value) = report.recorded(state, field, &label) else {
                    continue;
                };
                value
            } else {
                host
            };
            checks_complete &= report.attempt(&format!("{label}.{field}"), |report| {
                carrier_input(value, field, report)?;
                Ok(())
            });
        }
        checks_complete &= report.attempt(&format!("{label}.pools"), |report| {
            let pools = carrier_input(&packet.pools, "pools", report)?;
            let mut pool_checks = true;
            let mut consumed = Some(0_u32);
            for (ordinal, pool) in pools.iter().enumerate() {
                let pool_label = format!("{label}.pools[{ordinal}]");
                let end = pool.wing_start.checked_add(pool.wing_count);
                pool_checks &= report.attempt(&format!("{pool_label}.partition"), |report| {
                    if consumed != Some(pool.wing_start)
                        || end.is_none_or(|end| end as usize > registration.wings.len())
                    {
                        return Err(invalid("carrier pool range does not continue the ordered wing partition"));
                    }
                    report.compared("CarrierAir.partition (recording self-check)", 1);
                    Ok(())
                });
                consumed = end;
                for (field, value) in [
                    ("current_fuel_raw", &pool.current_fuel_raw),
                    ("maximum_fuel_raw", &pool.maximum_fuel_raw),
                ].into_iter().filter(|_| base.state.is_none()) {
                    pool_checks &= report.attempt(&format!("{pool_label}.{field}"), |report| {
                        carrier_input(value, field, report)?;
                        Ok(())
                    });
                }
                pool_checks &= report.attempt(&format!("{pool_label}.members"), |report| {
                    let end = end.ok_or_else(|| invalid("carrier pool range overflow"))? as usize;
                    let members = registration.wings.get(pool.wing_start as usize..end)
                        .ok_or_else(|| invalid("carrier pool range outside recorded wings"))?;
                    if members.is_empty() { return Ok(()); }
                    let wings = wings.get_or_insert_with(|| {
                        complete::<tables::AirWings>(frame).and_then(|rows| {
                            let mut wings = BTreeMap::new();
                            for row in rows {
                                if row.id == 0 || wings.insert(row.id, row).is_some() {
                                    return Err(invalid("null or duplicate AirWings handle in carrier member join"));
                                }
                            }
                            Ok(wings)
                        })
                    });
                    let wings = wings.as_ref().map_err(|error| invalid(error.to_string()))?;
                    for id in members {
                        let wing = wings.get(id)
                            .ok_or_else(|| invalid(format!("carrier member wing {id} absent from AirWings")))?;
                        let Some(index) = report.recorded(
                            &wing.pool_subunit, "AirWings.pool_subunit", format!("air wing {id}"),
                        ) else { continue; };
                        let index = index.ok_or_else(|| invalid(format!("carrier member wing {id} pool identity acquisition refused")))?;
                        if index != pool.subunit_index {
                            return Err(invalid(format!("carrier member wing {id} pool identity differs from its recorded pool")));
                        }
                        report.compared("CarrierAir.member_identity (recording self-check)", 1);
                    }
                    Ok(())
                });
            }
            pool_checks &= report.attempt(&format!("{label}.pools.final_partition"), |_| {
                if consumed.map(|length| length as usize) != Some(registration.wings.len()) {
                    return Err(invalid("carrier pools do not consume every recorded wing"));
                }
                Ok(())
            });
            if pool_checks { Ok(()) } else { Err(invalid("carrier pool self-check incomplete")) }
        });
    }
    if checks_complete {
        Ok(())
    } else {
        Err(invalid("carrier recording self-check incomplete"))
    }
}

fn carrier_input<'a, T>(
    value: &'a Option<T>,
    field: &str,
    report: &mut Report,
) -> io::Result<&'a T> {
    let value = value
        .as_ref()
        .ok_or_else(|| invalid(format!("carrier {field}: current acquisition refused")))?;
    report.compared(&format!("CarrierAir.{field} (recording self-check)"), 1);
    Ok(value)
}

fn carrier_country(
    raw: u32,
    slot: u32,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    match (raw, slot, links.get(&raw)) {
        (0, 0, _) => {}
        (0, _, _) => return Err(invalid("carrier raw zero country must use slot zero")),
        (_, _, Some(country)) if country.canonical_slot == slot => {}
        (_, 0, None) => {
            report.limitation(
                "carrier slot-zero nonzero raw country has no recorded link; readable native unlinked-reference resolution cannot be certified by this self-check",
                format!("country {raw}, slot {slot}"),
            );
            return Ok(());
        }
        _ => {
            return Err(invalid(
                "carrier raw country does not resolve to its indexed slot",
            ));
        }
    }
    report.compared("CarrierAir.country (recording self-check)", 1);
    Ok(())
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::frame;
    use content_reader_save::parse;
    use replay_data::{
        rows::AirBaseCarrierInputs, rows::AirBaseCountryOccurrence,
        rows::AirBaseRegistrationCarrierInputs, rows::AirBaseRegistrationRow, rows::AirBaseRow,
        rows::AirWingRow, rows::CarrierAirPool, rows::RadarCoverageRow, rows::Recorded,
        rows::StationedUnitRow, rows::TableId, rows::TableRows, rows::TheatreAreaRow,
        rows::UnitIntelRow, rows::VolunteerHostRow,
    };
    use replay_frame::frame::TableView;

    const DIVISION: u64 = (9_u64 << 32) | 4919;
    const RAILWAY_GUN: u64 = (7_u64 << 32) | 74;
    const THEATRE: u64 = (5_u64 << 32) | 67;

    fn saved() -> content_reader_save::Save {
        parse(
            br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={
 GER={
  received_air_volunteer_permission={ tag=SOV count=3 }
  received_air_volunteer_permission={ tag=SOV count=-1 }
  units={ division={ id={id=9 type=4919} logical_country=GER strength=0 organisation=0
    country_intel={ 2 0 4 -1 0 -1 } } }
  theatres={ theatre={ id={id=5 type=67} volunteers_theatre=SOV
    unit={id=9 type=4919} area={ 21 22 } } }
 }
 SOV={}
}
states={} provinces={}"#,
        )
        .unwrap()
    }

    fn railway_saved() -> content_reader_save::Save {
        parse(
            br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={ GER={ units={ railway_gun={ id={id=7 type=74} logical_country=GER strength=0
  country_intel={ 2 0 4 -1 0 -1 } } } } }
states={} provinces={}"#,
        )
        .unwrap()
    }

    fn railway_frame() -> DecodedFrame {
        frame([TableRows::UnitIntel(vec![
            UnitIntelRow {
                unit: RAILWAY_GUN,
                ordinal: 0,
                viewer: 2,
                level: 4,
            },
            UnitIntelRow {
                unit: RAILWAY_GUN,
                ordinal: 1,
                viewer: u32::MAX,
                level: -1,
            },
        ])])
    }

    fn railway_rows(frame: &mut DecodedFrame) -> &mut Vec<UnitIntelRow> {
        let TableView::Complete(TableRows::UnitIntel(rows)) =
            &mut *frame.tables[TableId::UnitIntel.index()]
        else {
            unreachable!()
        };
        rows
    }

    fn links() -> (CountryRow, CountryRow) {
        (
            CountryRow {
                id: 12,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 34,
                canonical_slot: 2,
                tag: b"SOV".to_vec(),
                ..Default::default()
            },
        )
    }

    fn complete_frame() -> DecodedFrame {
        frame([
            TableRows::UnitIntel(vec![
                UnitIntelRow {
                    unit: DIVISION,
                    ordinal: 0,
                    viewer: 2,
                    level: 4,
                },
                UnitIntelRow {
                    unit: DIVISION,
                    ordinal: 1,
                    viewer: u32::MAX,
                    level: -1,
                },
            ]),
            TableRows::VolunteerHosts(vec![
                VolunteerHostRow {
                    country: 12,
                    position: 0,
                    host: 34,
                    deployed_divisions: 3,
                    air_capacity_modifier: (None).into(),
                },
                VolunteerHostRow {
                    country: 12,
                    position: 1,
                    host: 34,
                    deployed_divisions: -1,
                    air_capacity_modifier: (None).into(),
                },
            ]),
            TableRows::StationedUnits(vec![StationedUnitRow {
                theatre: THEATRE,
                ordinal: 0,
                unit: DIVISION,
            }]),
            TableRows::TheatreAreas(vec![
                TheatreAreaRow {
                    theatre: THEATRE,
                    ordinal: 0,
                    representative: Some(21),
                    provinces: vec![21, 99],
                },
                TheatreAreaRow {
                    theatre: THEATRE,
                    ordinal: 1,
                    representative: Some(22),
                    provinces: Vec::new(),
                },
            ]),
            TableRows::RadarCoverage(vec![RadarCoverageRow {
                country: 12,
                ordinal: 0,
                state: Some(4),
                provinces: vec![7],
                radius: None,
            }]),
            TableRows::AirBases(vec![AirBaseRow {
                id: 65,
                state: Some(4),
                carrier: Recorded::Value(Some(AirBaseCarrierInputs {
                    registered_countries: Some(vec![AirBaseCountryOccurrence {
                        country: 12,
                        slot: 1,
                    }]),
                    host_name: None,
                })),
                ..Default::default()
            }]),
            TableRows::AirBaseRegistrations(vec![AirBaseRegistrationRow {
                base: 65,
                slot: 1,
                capacity_raw: 100_000,
                wings: vec![69],
                carrier: Recorded::Value(Some(AirBaseRegistrationCarrierInputs {
                    country: 12,
                    fuel_demand_raw: None,
                    fuel_allocated_raw: None,
                    pools: Some(vec![CarrierAirPool {
                        subunit_index: 42,
                        wing_count: 1,
                        ..Default::default()
                    }]),
                })),
                transport_planes: Default::default(),
                fuel_demand_raw: Recorded::Value(Some(0)),
                fuel_allocated_raw: Recorded::Value(Some(0)),
            }]),
            TableRows::AirWings(vec![AirWingRow {
                id: 69,
                owner: 12,
                planes: 1,
                air_base: 65,
                pool_subunit: Recorded::Value(Some(42)),
                ..Default::default()
            }]),
        ])
    }

    const CARRIER_BASE: u64 = (23_u64 << 32) | 65;
    const CARRIER_WING: u64 = (29_u64 << 32) | 69;

    fn carrier_frame() -> DecodedFrame {
        frame([
            TableRows::AirBases(vec![AirBaseRow {
                id: CARRIER_BASE,
                host: (31_u64 << 32) | 70,
                carrier: Recorded::Value(Some(AirBaseCarrierInputs {
                    registered_countries: Some(vec![
                        AirBaseCountryOccurrence {
                            country: 12,
                            slot: 1,
                        },
                        AirBaseCountryOccurrence {
                            country: 12,
                            slot: 1,
                        },
                    ]),
                    host_name: Some(vec![0xff, 0]),
                })),
                ..Default::default()
            }]),
            TableRows::AirBaseRegistrations(vec![AirBaseRegistrationRow {
                base: CARRIER_BASE,
                slot: 1,
                capacity_raw: 0,
                wings: vec![CARRIER_WING, CARRIER_WING],
                transport_planes: Default::default(),
                carrier: Recorded::Value(Some(AirBaseRegistrationCarrierInputs {
                    country: 12,
                    fuel_demand_raw: Some(i64::MIN),
                    fuel_allocated_raw: Some(i64::MAX),
                    pools: Some(vec![
                        CarrierAirPool {
                            type_index: i32::MIN,
                            subunit_index: 42,
                            current_fuel_raw: Some(i64::MIN),
                            maximum_fuel_raw: Some(i64::MAX),
                            ..Default::default()
                        },
                        CarrierAirPool {
                            type_index: i32::MAX,
                            subunit_index: 42,
                            wing_count: 2,
                            current_fuel_raw: Some(i64::MAX),
                            maximum_fuel_raw: Some(i64::MIN),
                            ..Default::default()
                        },
                        CarrierAirPool {
                            type_index: -1,
                            subunit_index: 0,
                            wing_start: 2,
                            current_fuel_raw: Some(0),
                            maximum_fuel_raw: Some(0),
                            ..Default::default()
                        },
                    ]),
                })),
                fuel_demand_raw: Default::default(),
                fuel_allocated_raw: Default::default(),
            }]),
            TableRows::AirWings(vec![AirWingRow {
                id: CARRIER_WING,
                owner: 12,
                planes: 3,
                air_base: CARRIER_BASE,
                pool_subunit: Recorded::Value(Some(42)),
                ..Default::default()
            }]),
        ])
    }

    fn carrier_base(frame: &mut DecodedFrame) -> &mut AirBaseRow {
        let TableView::Complete(TableRows::AirBases(rows)) =
            &mut *frame.tables[TableId::AirBases.index()]
        else {
            unreachable!()
        };
        &mut rows[0]
    }

    fn carrier_registration(frame: &mut DecodedFrame) -> &mut AirBaseRegistrationRow {
        let TableView::Complete(TableRows::AirBaseRegistrations(rows)) =
            &mut *frame.tables[TableId::AirBaseRegistrations.index()]
        else {
            unreachable!()
        };
        &mut rows[0]
    }

    fn carrier_packet(frame: &mut DecodedFrame) -> &mut AirBaseRegistrationCarrierInputs {
        let Recorded::Value(Some(packet)) = &mut carrier_registration(frame).carrier else {
            unreachable!()
        };
        packet
    }

    fn carrier_count(report: &Report, field: &str) -> usize {
        report
            .compared_tables
            .get(&format!("CarrierAir.{field} (recording self-check)"))
            .copied()
            .unwrap_or(0)
    }

    #[test]
    fn carrier_self_check_preserves_empty_pools_repeats_raw_bytes_and_signed_extrema() {
        let (country, _) = links();
        let links = BTreeMap::from([(12, &country)]);
        let frame = carrier_frame();
        let mut report = Report::default();
        record_carrier(&frame, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(carrier_count(&report, "dense_join"), 2);
        assert_eq!(carrier_count(&report, "partition"), 3);
        assert_eq!(carrier_count(&report, "member_identity"), 2);
        assert_eq!(carrier_count(&report, "host_name"), 1);
        assert_eq!(carrier_count(&report, "current_fuel_raw"), 3);
        assert_eq!(carrier_count(&report, "maximum_fuel_raw"), 3);
        assert_eq!(carrier_count(&report, "empty_pool_planes"), 0);
        assert_eq!(carrier_count(&report, "pool_planes"), 0);
        assert_eq!(report.compared_tables["AirBases.carrier"], 0);
        assert_eq!(report.compared_tables["AirBaseRegistrations.carrier"], 0);
        assert!(
            !report
                .compared_tables
                .contains_key("AirBaseRegistrations.transport_planes")
        );
        for reason in [
            "zero fresh-save or native comparisons",
            "matched native witness",
            "empty carrier pools",
        ] {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|value| value.contains(reason)),
                "{reason}"
            );
        }
    }

    #[test]
    fn state_registration_checks_raw_scalars_and_metadata_without_pool_getter_results() {
        let (country, _) = links();
        let links = BTreeMap::from([(12, &country)]);
        let mut frame = carrier_frame();
        carrier_base(&mut frame).state = Some(4);
        carrier_registration(&mut frame).fuel_demand_raw = Recorded::Value(Some(-11));
        carrier_registration(&mut frame).fuel_allocated_raw = Recorded::Value(Some(5));
        carrier_packet(&mut frame).fuel_demand_raw = None;
        carrier_packet(&mut frame).fuel_allocated_raw = None;
        if let Recorded::Value(Some(base)) = &mut carrier_base(&mut frame).carrier {
            base.host_name = None;
        }
        for pool in carrier_packet(&mut frame).pools.as_mut().unwrap() {
            pool.current_fuel_raw = None;
            pool.maximum_fuel_raw = None;
        }
        let mut report = Report::default();
        record_carrier(&frame, &links, &mut report).unwrap();
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(carrier_count(&report, "fuel_demand_raw"), 1);
        assert_eq!(carrier_count(&report, "fuel_allocated_raw"), 1);
        assert_eq!(carrier_count(&report, "partition"), 3);
        assert_eq!(carrier_count(&report, "member_identity"), 2);
        assert_eq!(carrier_count(&report, "current_fuel_raw"), 0);
        assert_eq!(carrier_count(&report, "maximum_fuel_raw"), 0);
    }

    #[test]
    fn state_save_compares_carried_country_order_and_exact_signed_fuel_with_zero_omission() {
        use collector_air::extended::air::registration_store_tests::carried_state_orders;

        let (mut country, _) = links();
        country.id = 1;
        let links = BTreeMap::from([(country.id, &country)]);
        let mut frame = carrier_frame();
        let (_, carried) = carried_state_orders(true);
        carrier_base(&mut frame).carrier = carried;
        carrier_base(&mut frame).state = Some(4);
        carrier_packet(&mut frame).country = country.id;
        carrier_registration(&mut frame).fuel_demand_raw = Recorded::Value(Some(-3));
        carrier_registration(&mut frame).fuel_allocated_raw = Recorded::Value(Some(123456));
        let mut save = saved();
        save.air.bases.push(content_reader_save::air::AirBase {
            id: carrier_base(&mut frame).id,
            state: Some(4),
            host: None,
            capacity: 0,
            kind: 0,
            containers: vec![
                content_reader_save::air::AirBaseCountry {
                    country: b"GER".to_vec(),
                    fuel_demand: content_reader_save::Decimal::parse(b"-0.00003").unwrap(),
                    fuel_allocated: content_reader_save::Decimal::parse(b"1.23456").unwrap(),
                };
                2
            ],
        });
        let mut report = Report::default();
        compare_state_container_fuel(&frame, &save, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report
                .compared_tables
                .get("AirBases.registered_countries (save)"),
            Some(&2)
        );
        assert_eq!(
            report.compared_tables["AirBaseRegistrations.fuel (save)"],
            4
        );
        carrier_registration(&mut frame).fuel_allocated_raw = Recorded::Value(Some(123455));
        let mut report = Report::default();
        compare_state_container_fuel(&frame, &save, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
    }

    #[test]
    fn carrier_optional_refusals_preserve_independent_fuel_name_and_pool_checks() {
        let (country, _) = links();
        let links = BTreeMap::from([(12, &country)]);
        let mut frame = carrier_frame();
        let Recorded::Value(Some(base)) = &mut carrier_base(&mut frame).carrier else {
            unreachable!()
        };
        base.registered_countries = None;
        let packet = carrier_packet(&mut frame);
        packet.fuel_demand_raw = None;
        packet.pools.as_mut().unwrap()[1].current_fuel_raw = None;
        let mut report = Report::default();
        assert!(record_carrier(&frame, &links, &mut report).is_err());
        for field in [
            "registered_countries",
            "fuel_demand_raw",
            "current_fuel_raw",
        ] {
            assert!(
                report
                    .failed_comparisons
                    .keys()
                    .any(|key| key.ends_with(field)),
                "{field}"
            );
        }
        assert_eq!(carrier_count(&report, "host_name"), 1);
        assert_eq!(carrier_count(&report, "fuel_allocated_raw"), 1);
        assert_eq!(carrier_count(&report, "current_fuel_raw"), 2);
        assert_eq!(carrier_count(&report, "maximum_fuel_raw"), 3);
        assert_eq!(carrier_count(&report, "member_identity"), 2);

        carrier_packet(&mut frame).pools = None;
        let mut report = Report::default();
        assert!(record_carrier(&frame, &links, &mut report).is_err());
        assert_eq!(carrier_count(&report, "fuel_allocated_raw"), 1);
        assert_eq!(carrier_count(&report, "pools"), 0);
    }

    #[test]
    fn carrier_historical_absence_current_refusal_and_state_scope_stay_distinct() {
        let (country, _) = links();
        let links = BTreeMap::from([(12, &country)]);
        let mut frame = carrier_frame();
        carrier_base(&mut frame).carrier = Recorded::NotRecorded;
        carrier_registration(&mut frame).carrier = Recorded::NotRecorded;
        let mut report = Report::default();
        record_carrier(&frame, &links, &mut report).unwrap();
        assert!(
            report
                .limitations
                .contains_key("AirBases.carrier not recorded")
        );
        assert!(
            report
                .limitations
                .contains_key("AirBaseRegistrations.carrier not recorded")
        );

        carrier_base(&mut frame).carrier = Recorded::Value(None);
        carrier_registration(&mut frame).carrier = Recorded::Value(None);
        let mut report = Report::default();
        assert!(record_carrier(&frame, &links, &mut report).is_err());
        assert_eq!(report.failed_comparisons.len(), 2);
        carrier_base(&mut frame).state = Some(4);
        let mut report = Report::default();
        assert!(record_carrier(&frame, &links, &mut report).is_err());
        assert_eq!(report.failed_comparisons.len(), 2);

        let mut frame = carrier_frame();
        let base = carrier_base(&mut frame);
        let Recorded::Value(Some(packet)) = &mut base.carrier else {
            unreachable!()
        };
        packet.host_name = Some(Vec::new());
        record_carrier(&frame, &links, &mut Report::default()).unwrap();
        let Recorded::Value(Some(packet)) = &mut carrier_base(&mut frame).carrier else {
            unreachable!()
        };
        packet.host_name = None;
        let mut report = Report::default();
        assert!(record_carrier(&frame, &links, &mut report).is_err());
        assert_eq!(carrier_count(&report, "host_name"), 0);
        assert_eq!(carrier_count(&report, "dense_join"), 2);
    }

    #[test]
    fn carrier_pool_gaps_overlap_overflow_and_unconsumed_wings_refuse_only_their_checks() {
        let (country, _) = links();
        let links = BTreeMap::from([(12, &country)]);
        for mutate in [
            |pools: &mut Vec<CarrierAirPool>| pools[1].wing_start = 1,
            |pools: &mut Vec<CarrierAirPool>| pools[2].wing_start = 1,
            |pools: &mut Vec<CarrierAirPool>| {
                pools[1].wing_start = u32::MAX;
                pools[1].wing_count = 2;
            },
            |pools: &mut Vec<CarrierAirPool>| {
                pools[1].wing_count = 1;
                pools[2].wing_start = 1;
            },
        ] {
            let mut frame = carrier_frame();
            mutate(carrier_packet(&mut frame).pools.as_mut().unwrap());
            let mut report = Report::default();
            assert!(record_carrier(&frame, &links, &mut report).is_err());
            assert!(
                report
                    .failed_comparisons
                    .keys()
                    .any(|key| key.contains("partition"))
            );
            assert_eq!(carrier_count(&report, "current_fuel_raw"), 3);
            assert_eq!(carrier_count(&report, "maximum_fuel_raw"), 3);
            assert_eq!(carrier_count(&report, "fuel_allocated_raw"), 1);
        }
    }

    #[test]
    fn carrier_member_missing_or_refused_identity_never_certifies_the_pool() {
        let (country, _) = links();
        let links = BTreeMap::from([(12, &country)]);
        for identity in [Recorded::Value(Some(43)), Recorded::Value(None)] {
            let mut frame = carrier_frame();
            let TableView::Complete(TableRows::AirWings(rows)) =
                &mut *frame.tables[TableId::AirWings.index()]
            else {
                unreachable!()
            };
            rows[0].pool_subunit = identity;
            let mut report = Report::default();
            assert!(record_carrier(&frame, &links, &mut report).is_err());
            assert_eq!(carrier_count(&report, "member_identity"), 0);
            assert_eq!(carrier_count(&report, "maximum_fuel_raw"), 3);
        }
        let mut historical = carrier_frame();
        let TableView::Complete(TableRows::AirWings(rows)) =
            &mut *historical.tables[TableId::AirWings.index()]
        else {
            unreachable!()
        };
        rows[0].pool_subunit = Recorded::NotRecorded;
        let mut report = Report::default();
        record_carrier(&historical, &links, &mut report).unwrap();
        assert!(
            report
                .limitations
                .contains_key("AirWings.pool_subunit not recorded")
        );
        assert_eq!(carrier_count(&report, "member_identity"), 0);
        assert_eq!(carrier_count(&report, "maximum_fuel_raw"), 3);
        let mut frame = carrier_frame();
        *frame.tables[TableId::AirWings.index()] = TableView::Unavailable {
            reason: "guarded wing read".into(),
        };
        let mut report = Report::default();
        assert!(record_carrier(&frame, &links, &mut report).is_err());
        assert_eq!(carrier_count(&report, "current_fuel_raw"), 3);
        assert_eq!(carrier_count(&report, "fuel_demand_raw"), 1);
        *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![]));
        assert!(record_carrier(&frame, &links, &mut Report::default()).is_err());
    }

    #[test]
    fn carrier_dense_country_requires_matching_indexed_raw_identity_and_slot() {
        let (country, _) = links();
        let links = BTreeMap::from([(12, &country)]);
        for mutate in [
            |row: &mut AirBaseRegistrationRow| row.slot = 2,
            |row: &mut AirBaseRegistrationRow| {
                let Recorded::Value(Some(packet)) = &mut row.carrier else {
                    unreachable!()
                };
                packet.country = 0;
            },
        ] {
            let mut frame = carrier_frame();
            mutate(carrier_registration(&mut frame));
            let mut report = Report::default();
            assert!(record_carrier(&frame, &links, &mut report).is_err());
            assert_eq!(carrier_count(&report, "dense_join"), 0);
            assert_eq!(carrier_count(&report, "fuel_demand_raw"), 1);
        }
        for raw in [0, 4999] {
            let mut frame = carrier_frame();
            carrier_registration(&mut frame).slot = 0;
            carrier_packet(&mut frame).country = raw;
            let Recorded::Value(Some(base)) = &mut carrier_base(&mut frame).carrier else {
                unreachable!()
            };
            for entry in base.registered_countries.as_mut().unwrap() {
                entry.country = raw;
                entry.slot = 0;
            }
            let mut report = Report::default();
            record_carrier(&frame, &links, &mut report).unwrap();
            assert_eq!(carrier_count(&report, "dense_join"), 2);
            assert_eq!(
                carrier_count(&report, "country"),
                if raw == 0 { 3 } else { 0 }
            );
            assert_eq!(
                report
                    .limitations
                    .keys()
                    .any(|key| key.contains("slot-zero nonzero")),
                raw != 0
            );
        }
    }

    #[test]
    fn carrier_raw_zero_slot_is_not_overridden_by_a_recorded_country_link() {
        let (country, _) = links();
        let links = BTreeMap::from([(0, &country), (12, &country)]);
        let mut frame = carrier_frame();
        carrier_packet(&mut frame).country = 0;
        let Recorded::Value(Some(base)) = &mut carrier_base(&mut frame).carrier else {
            unreachable!()
        };
        for entry in base.registered_countries.as_mut().unwrap() {
            entry.country = 0;
        }
        let mut report = Report::default();
        assert!(record_carrier(&frame, &links, &mut report).is_err());
        assert_eq!(carrier_count(&report, "country"), 0);
        assert_eq!(carrier_count(&report, "dense_join"), 0);
        assert_eq!(carrier_count(&report, "fuel_allocated_raw"), 1);
        carrier_registration(&mut frame).slot = 0;
        let Recorded::Value(Some(base)) = &mut carrier_base(&mut frame).carrier else {
            unreachable!()
        };
        for entry in base.registered_countries.as_mut().unwrap() {
            entry.slot = 0;
        }
        let mut report = Report::default();
        record_carrier(&frame, &links, &mut report).unwrap();
        assert_eq!(carrier_count(&report, "country"), 3);
        assert_eq!(carrier_count(&report, "dense_join"), 2);
    }

    #[test]
    fn carrier_refusal_does_not_replace_the_existing_registration_save_limitation() {
        let (country, soviet) = links();
        let links = BTreeMap::from([(12, &country), (34, &soviet)]);
        let mut frame = complete_frame();
        let carrier = carrier_frame();
        for table in [
            TableId::AirBases,
            TableId::AirBaseRegistrations,
            TableId::AirWings,
        ] {
            frame.tables[table.index()] = carrier.tables[table.index()].clone();
        }
        carrier_packet(&mut frame).fuel_allocated_raw = None;
        let mut report = Report::default();
        assert!(compare(&frame, &saved(), &links, &mut report).is_err());
        assert_eq!(report.compared_tables["AirBaseRegistrations"], 0);
        assert!(report.limitations.keys().any(|key| {
            key.contains("do not serialize pool registration order or indexed country slots")
        }));
        assert!(
            report
                .failed_comparisons
                .contains_key("fog CarrierAir (recording self-check)")
        );
    }

    #[test]
    fn railway_gun_intel_matches_every_saved_entry_in_native_order() {
        let mut report = Report::default();
        compare_unit_intel(&railway_frame(), &railway_saved(), &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["UnitIntel"], 2);
    }

    #[test]
    fn railway_gun_intel_missing_extra_and_changed_entries_are_differences() {
        let save = railway_saved();

        let mut missing = railway_frame();
        railway_rows(&mut missing).pop();
        let mut report = Report::default();
        compare_unit_intel(&missing, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("country_intel.count"));

        let mut extra = railway_frame();
        railway_rows(&mut extra).push(UnitIntelRow {
            unit: (8_u64 << 32) | 74,
            ordinal: 0,
            viewer: 2,
            level: 4,
        });
        let mut report = Report::default();
        compare_unit_intel(&extra, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("no saved unit"));

        for change in [
            |row: &mut UnitIntelRow| row.viewer = 3,
            |row: &mut UnitIntelRow| row.level = 3,
        ] {
            let mut changed = railway_frame();
            change(&mut railway_rows(&mut changed)[0]);
            let mut report = Report::default();
            compare_unit_intel(&changed, &save, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        }
    }

    #[test]
    fn railway_gun_intel_refuses_a_gap_in_the_native_ordinals() {
        let mut frame = railway_frame();
        railway_rows(&mut frame)[1].ordinal = 2;
        let error =
            compare_unit_intel(&frame, &railway_saved(), &mut Report::default()).unwrap_err();
        assert!(error.to_string().contains("native ordinal"), "{error}");
    }

    #[test]
    fn every_saved_family_matches_and_the_unsaved_ones_are_named_limitations() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let mut report = Report::default();
        compare(&complete_frame(), &saved(), &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["UnitIntel"], 2);
        assert_eq!(report.compared_tables["VolunteerHosts"], 2);
        assert_eq!(report.compared_tables["StationedUnits"], 1);
        assert_eq!(report.compared_tables["TheatreAreas"], 2);
        assert_eq!(report.compared_tables["RadarCoverage"], 0);
        assert_eq!(report.compared_tables["AirBaseRegistrations"], 0);
        for expected in [
            "decay accumulator",
            "representative province per controller area",
            "radar coverage vectors",
            "do not serialize pool registration order or indexed country slots",
        ] {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains(expected)),
                "{expected}"
            );
        }
    }

    #[test]
    fn changed_levels_hosts_units_and_representatives_are_differences() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let save = saved();
        for mutate in [
            |frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::UnitIntel(rows)) =
                    &mut *frame.tables[TableId::UnitIntel.index()]
                else {
                    unreachable!()
                };
                rows[1].level = 0;
            },
            |frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::VolunteerHosts(rows)) =
                    &mut *frame.tables[TableId::VolunteerHosts.index()]
                else {
                    unreachable!()
                };
                rows[0].deployed_divisions = 4;
            },
            |frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::StationedUnits(rows)) =
                    &mut *frame.tables[TableId::StationedUnits.index()]
                else {
                    unreachable!()
                };
                rows[0].unit = 1;
            },
            |frame: &mut DecodedFrame| {
                let TableView::Complete(TableRows::TheatreAreas(rows)) =
                    &mut *frame.tables[TableId::TheatreAreas.index()]
                else {
                    unreachable!()
                };
                rows[1].representative = None;
            },
        ] {
            let mut frame = complete_frame();
            mutate(&mut frame);
            let mut report = Report::default();
            compare(&frame, &save, &links, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        }
    }

    #[test]
    fn missing_rows_extra_parents_and_unavailable_tables_never_pass_silently() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let save = saved();
        let mut frame = complete_frame();
        *frame.tables[TableId::UnitIntel.index()] =
            TableView::Complete(TableRows::UnitIntel(vec![]));
        let mut report = Report::default();
        compare(&frame, &save, &links, &mut report).unwrap();
        assert!(report.differences.iter().any(|line| line.contains("count")));

        let mut frame = complete_frame();
        let TableView::Complete(TableRows::StationedUnits(rows)) =
            &mut *frame.tables[TableId::StationedUnits.index()]
        else {
            unreachable!()
        };
        rows[0].theatre = (6_u64 << 32) | 67;
        let mut report = Report::default();
        compare(&frame, &save, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);

        let mut frame = complete_frame();
        *frame.tables[TableId::RadarCoverage.index()] = TableView::Unavailable {
            reason: "guarded copy failed".into(),
        };
        let mut report = Report::default();
        assert!(compare(&frame, &save, &links, &mut report).is_err());
        assert!(report.failed_comparisons.contains_key("fog RadarCoverage"));
    }

    #[test]
    fn a_gap_in_the_native_ordinals_refuses_the_comparison() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let mut frame = complete_frame();
        let TableView::Complete(TableRows::UnitIntel(rows)) =
            &mut *frame.tables[TableId::UnitIntel.index()]
        else {
            unreachable!()
        };
        rows[1].ordinal = 3;
        let mut report = Report::default();
        assert!(compare(&frame, &saved(), &links, &mut report).is_err());
        assert!(report.failed_comparisons["fog UnitIntel"].contains("native ordinal"));
    }

    #[test]
    fn a_save_without_a_theatre_roster_refuses_both_stationed_comparisons() {
        let (germany, soviet) = links();
        let links = BTreeMap::from([(12, &germany), (34, &soviet)]);
        let mut save = saved();
        save.naval_history.theatre_countries = None;
        let mut report = Report::default();
        assert!(compare(&complete_frame(), &save, &links, &mut report).is_err());
        assert!(
            report.failed_comparisons["fog StationedUnits"].contains("no theatre roster")
                && report.failed_comparisons["fog TheatreAreas"].contains("no theatre roster")
        );
    }
}
