use super::{Report, complete, country_tag, invalid, required_country_tag};
use super::{compare_text, equal};
use content_reader_save::Save;
use replay_data::{
    rows::CountryRow, rows::FuelDemandHour, rows::FuelDemandRead, rows::Recorded, rows::TableId,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let groups: BTreeMap<_, _> = saved
        .naval_history
        .theatre_countries
        .iter()
        .flat_map(|countries| countries.values())
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        .map(|group| (group.id, group))
        .collect();
    let rows = replay_frame::rows::with_demand_parts(
        complete::<tables::CountryFuel>(frame)?,
        &frame.tables[TableId::FuelDemandParts.index()],
        replay_data::format::FormatVersion::CURRENT,
    )?;
    for row in &rows {
        let label = format!("fuel demand country {}", row.country);
        let Recorded::Value(pack) = &row.demand else {
            report.limitation(
                "current individual fuel demand was not recorded in this schema prefix",
                label,
            );
            continue;
        };
        let tag = required_country_tag(row.country, links)?;
        let fresh = if pack.hour == FuelDemandHour::Host {
            report.read_at_hour(TableId::CountryFuel)
        } else {
            pack.hour.at(frame.hour) == frame.hour
        };
        if !fresh {
            return Err(invalid(
                "fuel demand oracle requires its exact supplying hour",
            ));
        }
        if !pack.reason.is_empty() {
            return Err(invalid("fuel demand read has a named acquisition refusal"));
        }
        if !pack.army_reason.is_empty()
            || !pack.navy_reason.is_empty()
            || !pack.air_reason.is_empty()
            || !pack.group_reason.is_empty()
        {
            return Err(invalid("fuel demand supplying census is incomplete"));
        }
        census(
            report,
            &format!("{label}.divisions"),
            pack.divisions.iter().map(|unit| unit.reference),
            saved
                .counter_inputs
                .division_orders
                .get(tag)
                .map_or(&[][..], Vec::as_slice),
        );
        let fleets = saved
            .counter_inputs
            .fleet_orders
            .get(tag)
            .map_or(&[][..], Vec::as_slice);
        census(
            report,
            &format!("{label}.fleets"),
            pack.fleets.iter().map(|fleet| fleet.reference),
            fleets,
        );
        let tasks: Vec<_> = fleets
            .iter()
            .flat_map(|fleet| {
                saved
                    .counter_inputs
                    .fleets
                    .get(fleet)
                    .into_iter()
                    .flat_map(|fleet| fleet.task_forces.iter().copied())
            })
            .collect();
        census(
            report,
            &format!("{label}.task_forces"),
            pack.task_forces.iter().map(|unit| unit.reference),
            &tasks,
        );
        for unit in &pack.divisions {
            let expected = saved
                .divisions
                .get(&unit.reference)
                .ok_or_else(|| invalid("fuel demand division is absent from save"))?;
            equal(
                report,
                format!("{label}.division.{:#x}.requested", unit.reference),
                unit.rate_raw
                    .ok_or_else(|| invalid("fuel demand division cache unavailable"))?,
                expected
                    .presentation
                    .fuel_requested
                    .as_ref()
                    .map_or(0, |value| value.raw),
            );
            owners(
                report,
                links,
                unit.owners,
                [&expected.roster_owner, &expected.logical_owner],
                expected.expeditionary_owner.as_deref(),
                &format!("{label}.division.{:#x}", unit.reference),
            )?;
        }
        for unit in &pack.task_forces {
            let expected = saved
                .task_forces
                .get(&unit.reference)
                .ok_or_else(|| invalid("fuel demand task force is absent from save"))?;
            let demand = saved
                .naval_missions
                .task_forces
                .get(&unit.reference)
                .ok_or_else(|| invalid("saved task-force requested cache absent"))?;
            equal(
                report,
                format!("{label}.task_force.{:#x}.requested", unit.reference),
                unit.rate_raw
                    .ok_or_else(|| invalid("fuel demand task-force cache unavailable"))?,
                demand.requested.as_ref().map_or(0, |value| value.raw),
            );
            equal(
                report,
                format!("{label}.task_force.{:#x}.fleet", unit.reference),
                unit.parent,
                expected.fleet,
            );
            conditional_name(
                report,
                &format!("{label}.task_force.{:#x}.name", unit.reference),
                unit.name.as_deref(),
                expected.name.as_deref(),
            )?;
            owners(
                report,
                links,
                unit.owners,
                [&expected.roster_owner, &expected.logical_owner],
                expected.expeditionary_owner.as_deref(),
                &format!("{label}.task_force.{:#x}", unit.reference),
            )?;
        }
        for fleet in &pack.fleets {
            let expected = saved
                .fleets
                .get(&fleet.reference)
                .ok_or_else(|| invalid("fuel demand fleet absent from save"))?;
            text(
                report,
                &format!("{label}.fleet.{:#x}.name", fleet.reference),
                fleet.name.as_deref(),
                expected
                    .name
                    .as_deref()
                    .ok_or_else(|| invalid("saved fleet name missing"))?,
            )?;
        }
        for group in &pack.groups {
            let expected = saved
                .plans
                .group_names
                .get(&group.reference)
                .ok_or_else(|| invalid("fuel demand command absent from save"))?;
            conditional_name(
                report,
                &format!("{label}.group.{:#x}.name", group.reference),
                group.name.as_deref(),
                expected.as_deref(),
            )?;
            let expected = groups
                .get(&group.reference)
                .ok_or_else(|| invalid("fuel demand command topology absent from save"))?;
            equal(
                report,
                format!("{label}.group.{:#x}.members", group.reference),
                &group.members,
                &expected.members,
            );
            equal(
                report,
                format!("{label}.group.{:#x}.children", group.reference),
                &group.children,
                &expected.children,
            );
            equal(
                report,
                format!("{label}.group.{:#x}.army_group", group.reference),
                group.army_group != 0,
                expected.is_army_group,
            );
        }
        compare_wings(pack, saved, links, &label, report)?;
        report.compared(
            "CountryFuel current demand",
            pack.divisions.len() + pack.task_forces.len(),
        );
        report.limitation("division retained name cache is unsaved; authored generator operands cannot certify these possibly stale language bytes", &label);
        report.limitation("raw surrender-target and country-link alias bits are not serialized; cached unit and roster quantities do not prove fresh hover-getter equality", &label);
        report.limitation("command raw parent pointer and discriminator bits beyond saved boolean truthiness need a same-boundary native witness", &label);
        report.limitation("air-site lists/repeated pool occurrences, selected mission, active count, period, live fuel/modifier/ace/capacity/weather operands and daily wrapping projection are unsaved supplying-read state; recording arithmetic is checked, but exact native current-pool/tooltip agreement requires a same-boundary passive witness", label);
    }
    Ok(())
}

fn census(report: &mut Report, label: &str, rows: impl Iterator<Item = u64>, saved: &[u64]) {
    let actual: Vec<_> = rows.collect();
    equal(report, label, actual.as_slice(), saved);
}
fn text(report: &mut Report, label: &str, actual: Option<&[u8]>, saved: &[u8]) -> io::Result<()> {
    compare_text(
        report,
        label,
        actual.ok_or_else(|| invalid("fuel demand stored-name capture failed"))?,
        saved,
    );
    Ok(())
}

fn conditional_name(
    report: &mut Report,
    label: &str,
    actual: Option<&[u8]>,
    saved: Option<&[u8]>,
) -> io::Result<()> {
    let actual = actual.ok_or_else(|| invalid("fuel demand stored-name capture failed"))?;
    let prefix = content_reader_parser::parser::c_string(actual);
    if prefix.len() < actual.len() {
        report.limitation(
            "native text serialization ends at the first NUL; captured trailing name bytes are not save-comparable",
            label,
        );
    }
    equal(report, label, (!actual.is_empty()).then_some(prefix), saved);
    Ok(())
}
fn owners(
    report: &mut Report,
    links: &BTreeMap<u32, &CountryRow>,
    actual: Option<[u32; 3]>,
    expected: [&Vec<u8>; 2],
    expeditionary: Option<&[u8]>,
    label: &str,
) -> io::Result<()> {
    let owners = actual.ok_or_else(|| invalid("fuel demand owner words unavailable"))?;
    equal(
        report,
        format!("{label}.roster"),
        country_tag(owners[0], links)?,
        Some(expected[0].as_slice()),
    );
    equal(
        report,
        format!("{label}.logical"),
        country_tag(owners[2], links)?,
        Some(expected[1].as_slice()),
    );
    let actual = if (owners[1] as i32) <= 0 {
        None
    } else {
        country_tag(owners[1], links)?
    };
    equal(
        report,
        format!("{label}.expeditionary"),
        actual,
        expeditionary,
    );
    Ok(())
}
fn compare_wings(
    pack: &FuelDemandRead,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    label: &str,
    report: &mut Report,
) -> io::Result<()> {
    for site in &pack.sites {
        if !site.reason.is_empty() {
            return Err(invalid("fuel demand air site has a capture refusal"));
        }
        for wing in &site.wings {
            let expected = saved
                .air_wings
                .get(&wing.reference)
                .ok_or_else(|| invalid("fuel demand wing absent from save"))?;
            if !wing.owner_reason.is_empty() {
                return Err(invalid("fuel demand wing selected owner unavailable"));
            }
            let owner = wing
                .owner
                .ok_or_else(|| invalid("fuel demand wing selected owner unavailable"))?;
            let override_country = wing
                .country_override
                .ok_or_else(|| invalid("fuel demand wing owner selection unavailable"))?;
            let expected_owner = if override_country > 0 {
                equal(
                    report,
                    format!("{label}.wing.{:#x}.override_owner", wing.reference),
                    owner,
                    override_country as u32,
                );
                &expected.country_override
            } else {
                &expected.owner
            };
            if let Some(tag) = country_tag(owner, links)? {
                equal(
                    report,
                    format!("{label}.wing.{:#x}.selected_owner", wing.reference),
                    tag,
                    expected_owner.as_slice(),
                );
            } else {
                report.limitation("the game saves the raw-zero database tag; its literal is not recorded by CountryLinks and is not assumed", format!("{label}.wing.{:#x}.selected_owner", wing.reference));
            }
            text(
                report,
                &format!("{label}.wing.{:#x}.name", wing.reference),
                wing.name.as_deref(),
                expected
                    .name
                    .as_deref()
                    .ok_or_else(|| invalid("saved wing name missing"))?,
            )?;
            let inputs = wing
                .inputs
                .as_ref()
                .ok_or_else(|| invalid("fuel demand wing has a capture refusal"))?;
            if inputs.mission_first == 0 {
                equal(
                    report,
                    format!("{label}.wing.{:#x}.mask", wing.reference),
                    inputs.mask,
                    expected.mission_mask,
                );
                if inputs.reached {
                    equal(
                        report,
                        format!("{label}.wing.{:#x}.planes", wing.reference),
                        inputs.planes,
                        expected.planes,
                    );
                }
            }
            if inputs.reached {
                equal(
                    report,
                    format!("{label}.wing.{:#x}.region", wing.reference),
                    inputs.region,
                    expected.mission_region,
                );
            }
            pack.wing_rate(wing).map_err(invalid)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
