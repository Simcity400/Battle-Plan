
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::construction as saved;
use replay::campaign::Replay;
use replay_data::{
    Hour, rows::ConstructionLineOutputRow, rows::CountryRow, rows::TableId, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use super::{Report, complete, country_tag, daily_read, invalid, required_country_tag};

macro_rules! production_fields {
    ($report:expr, $subject:expr, $row:expr, $saved:expr, $links:expr) => {{
        let (report, subject, row, saved) = ($report, $subject, $row, $saved);
        value(
            report,
            &format!("{subject}.country"),
            required_country_tag(row.country, $links)?,
            saved.country.as_slice(),
        );
        value(
            report,
            &format!("{subject}.queue_ordinal"),
            row.queue_ordinal,
            saved.queue_ordinal,
        );
        value(
            report,
            &format!("{subject}.active_factories"),
            row.active_factories,
            saved.active_factories,
        );
        value(
            report,
            &format!("{subject}.priority"),
            row.priority,
            saved.priority,
        );
        value(
            report,
            &format!("{subject}.amount"),
            row.amount,
            saved.amount,
        );
        report.scalar(subject, "cost", Some(row.cost), Some(&saved.cost));
        report.scalar(subject, "speed", Some(row.speed), Some(&saved.speed));
        report.scalar(
            subject,
            "produced",
            Some(row.produced),
            Some(&saved.produced),
        );
        value(
            report,
            &format!("{subject}.created_hour"),
            row.created_hour,
            saved.created_date.engine_hour()?,
        );
    }};
}

pub fn naval_bases(
    rows: &[replay_data::rows::NavalBaseRow],
    saved: &BTreeMap<saved::BuildingKey, saved::Building>,
    port_types: &BTreeSet<Vec<u8>>,
    country_bases: &[content_reader_save::naval_launch::CountryNavalLaunchOccurrence],
    raw_tags: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut expected_bases = BTreeMap::<u32, Vec<_>>::new();
    for base in country_bases
        .iter()
        .filter_map(|country| country.naval_bases.as_ref())
        .flatten()
    {
        expected_bases
            .entry(base.province as u32)
            .or_default()
            .push(base);
    }
    let mut ports: BTreeMap<u32, (i32, i32)> = BTreeMap::new();
    for ((province, kind), building) in saved {
        if port_types.contains(kind) {
            let sums = ports.entry(*province).or_default();
            sums.0 += i32::from(building.level);
            sums.1 += i32::from(building.healthy_level);
        }
    }
    let mut seen = BTreeSet::new();
    let mut count = 0;
    let mut countries_count = 0;
    for row in rows {
        if !seen.insert(row.province) {
            return Err(invalid("duplicate naval base province"));
        }
        let label = format!("NavalBase {}", row.province);
        if row.countries_unreadable {
            report.difference(
                format!("{label}.countries"),
                "unreadable: null list with a positive count",
                "a list the game can read",
            );
        } else {
            let countries = match &row.country_tags {
                replay_data::rows::Recorded::Value(Some(tags))
                    if tags.len() == row.countries.len() =>
                {
                    Some(
                        tags.iter()
                            .map(|tag| super::serialized_text(report, &label, tag))
                            .collect::<Vec<_>>(),
                    )
                }
                replay_data::rows::Recorded::NotRecorded => row
                    .countries
                    .iter()
                    .map(|raw| {
                        raw_tags
                            .get(raw)
                            .copied()
                            .map(content_reader_parser::parser::c_string)
                    })
                    .collect::<Option<Vec<_>>>(),
                _ => None,
            };
            for (ordinal, base) in expected_bases
                .get(&row.province)
                .into_iter()
                .flatten()
                .enumerate()
            {
                match &countries {
                    Some(countries) => {
                        value(report, &format!("{label}.countries[{ordinal}]"),
                            countries.clone(), base.countries.iter().map(Vec::as_slice).collect());
                        countries_count += 1;
                    }
                    None => report.difference(format!("{label}.countries[{ordinal}]"),
                        format!("{} raw countries with missing, unreadable or inconsistent exact tag bytes", row.countries.len()),
                        &base.countries),
                }
            }
        }
        let Some(&(level, healthy_level)) = ports.get(&row.province) else {
            if row.level > 0 {
                report.difference(
                    format!("NavalBase {} membership", row.province),
                    "replay row with a positive level",
                    "no saved port building",
                );
            }
            continue;
        };
        for (name, actual, expected) in [
            ("level", row.level, level),
            ("healthy_level", row.healthy_level, healthy_level),
        ] {
            if actual != expected {
                report.difference(format!("{label}.{name}"), actual, expected);
            }
        }
        count += 1;
    }
    for province in expected_bases.keys() {
        if !seen.contains(province) {
            report.difference(
                format!("NavalBase {province} membership"),
                "absent from replay",
                "saved naval base roster entry",
            );
        }
    }
    report.compared("NavalBases", count);
    report.compared("NavalBases serialized country lists", countries_count);
    report.limitation(
        "the naval base holder's raw country is not serialized. Membership and ordered country lists are checked against every saved navy roster occurrence and levels against port buildings. The game omits physical country slot zero, so global bases absent from those rosters have no saved membership witness",
        "NavalBases",
    );
    Ok(())
}

pub fn buildings(
    rows: &[replay_data::rows::BuildingRow],
    saved: &BTreeMap<saved::BuildingKey, saved::Building>,
    province_scope: bool,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let table = if province_scope {
        "ProvinceBuildings"
    } else {
        "StateBuildings"
    };
    let mut seen = BTreeSet::new();
    let mut raw_keys = BTreeSet::new();
    let mut captured_counts = BTreeMap::<u32, usize>::new();
    let mut unresolved = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        if !raw_keys.insert((row.location, row.building_type)) {
            return Err(invalid("duplicate native scoped building type"));
        }
        *captured_counts.entry(row.location).or_default() += 1;
        let subject = format!("{table}.{}.token({})", row.location, row.building_type);
        let name = match token(names, row.building_type) {
            Ok(name) => name,
            Err(reason) => {
                unresolved.insert(row.location);
                report.limitation(
                    &format!("building {reason}; named identity, ordinal, levels and partial health cannot be matched; location and row-count checks continue"),
                    subject,
                );
                continue;
            }
        };
        let key = (row.location, name.to_vec());
        if !seen.insert(key.clone()) {
            return Err(invalid("duplicate resolved scoped building type"));
        }
        let subject = format!("{table}.{}.{}", row.location, String::from_utf8_lossy(name));
        let Some(saved) = saved.get(&key) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        value(
            report,
            &format!("{subject}.ordinal"),
            row.ordinal,
            saved.ordinal,
        );
        value(report, &format!("{subject}.level"), row.level, saved.level);
        value(
            report,
            &format!("{subject}.healthy_level"),
            row.healthy_level,
            saved.healthy_level,
        );
        report.scalar(
            &subject,
            "partial_health",
            Some(row.partial_health),
            Some(&saved.partial_health),
        );
        value(
            report,
            &format!("{subject}.active"),
            row.active,
            saved.level > 0,
        );
    }
    let mut saved_counts = BTreeMap::<u32, usize>::new();
    for key in saved.keys() {
        *saved_counts.entry(key.0).or_default() += 1;
        if !seen.contains(key) && !unresolved.contains(&key.0) {
            report.difference(format!("{table}.{key:?}"), "missing row", "present");
        }
    }
    for location in unresolved {
        let subject = format!("{table}.{location}");
        value(
            report,
            &format!("{subject}.row_count"),
            captured_counts[&location],
            saved_counts.get(&location).copied().unwrap_or(0),
        );
        report.limitation(
            "unresolved building type tokens prevent a named membership census at this location; total row count is compared without assigning names from ordinals",
            subject,
        );
    }
    report.compared(table, matched);
    report.limitation("building serializers preserve list order, levels and partial health; raw CBuildingStatus flags, country field and active vector are not serialized, so the active vector is compared with the positive saved levels it is rebuilt from", table);
    Ok(())
}

pub fn base_resources(
    rows: &[replay_data::rows::StateBaseResourceRow],
    content: &saved::BaseResources,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut raw_keys = BTreeSet::new();
    let mut captured_counts = BTreeMap::<u32, usize>::new();
    let mut unresolved = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        if !raw_keys.insert((row.state, row.resource)) {
            return Err(invalid("duplicate native state/base resource"));
        }
        *captured_counts.entry(row.state).or_default() += 1;
        let name = match token(names, row.resource) {
            Ok(name) => name,
            Err(reason) => {
                unresolved.insert(row.state);
                report.limitation(
                    &format!("base resource {reason}; named identity, ordinal and amount cannot be matched to verified content"),
                    format!("state base resource.{}.token({})", row.state, row.resource),
                );
                continue;
            }
        };
        let key = (row.state, name.to_vec());
        if !seen.insert(key.clone()) {
            return Err(invalid("duplicate resolved state/base resource"));
        }
        let subject = format!(
            "state base resource.{}.{}",
            row.state,
            String::from_utf8_lossy(name)
        );
        let Some(&(ordinal, amount)) = content.rows.get(&key) else {
            report.difference(subject, "present", "missing content declaration");
            continue;
        };
        matched += 1;
        value(report, &format!("{subject}.ordinal"), row.ordinal, ordinal);
        value(report, &format!("{subject}.amount"), row.amount, amount);
    }
    let mut content_counts = BTreeMap::<u32, usize>::new();
    for key in content.rows.keys() {
        *content_counts.entry(key.0).or_default() += 1;
        if !seen.contains(key) && !unresolved.contains(&key.0) {
            report.difference(
                format!("state base resource.{key:?}"),
                "missing row",
                "present",
            );
        }
    }
    for state in unresolved {
        let subject = format!("state base resource.{state}");
        value(
            report,
            &format!("{subject}.row_count"),
            captured_counts[&state],
            content_counts.get(&state).copied().unwrap_or(0),
        );
        report.limitation(
            "unresolved resource type tokens prevent a named membership census in this state; total row count is compared without assigning names from ordinals",
            subject,
        );
    }
    report.compared("StateBaseResources (verified content)", matched);
    report.limitation("template base resources are compared to verified state-history declarations; CState's save resources and temporary-resource entries are separate runtime pools, which this table does not capture", "StateBaseResources");
    Ok(())
}

pub fn partial_base_resources(
    rows: &[replay_data::rows::StateBaseResourceRow],
    content: &saved::PartialBaseResources,
    membership: &content_reader_save::PartialMapMembership,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut affected = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for row in rows {
        if !keys.insert((row.state, row.resource)) {
            return Err(invalid("duplicate native state/base resource"));
        }
    }
    let comparable: Vec<_> = rows
        .iter()
        .filter(|row| {
            if membership.state_is_excluded(row.state) {
                affected.insert(row.state);
                false
            } else {
                true
            }
        })
        .cloned()
        .collect();
    report.membership_skip(
        "StateBaseResources",
        rows.len() - comparable.len(),
        membership,
        &affected,
    );
    base_resources(&comparable, &content.known, names, report)
}

pub fn building_construction(
    rows: &[replay_data::rows::BuildingConstructionRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        if !seen.insert(row.id) {
            return Err(invalid("duplicate full building construction handle"));
        }
        let subject = format!("building construction.{}", row.id);
        let Some(saved) = saved.building_construction.get(&row.id) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        production_fields!(
            &mut *report,
            subject.as_str(),
            row,
            &saved.production,
            links
        );
        match token(names, row.building_type) {
            Ok(name) => value(
                report,
                &format!("{subject}.building_type"),
                name,
                saved.building_type.as_slice(),
            ),
            Err(reason) => report.limitation(
                &format!("construction building {reason}; full-handle membership, numeric fields and target projection are still compared"),
                format!("{subject}.building_type token({})", row.building_type),
            ),
        };
        let (location, country) = if row.target_flags & 1 != 0 {
            (row.target_location as i32, None)
        } else if (row.target_country as i32) > 0 {
            (-1, Some(required_country_tag(row.target_country, links)?))
        } else {
            (row.target_state as i32, None)
        };
        value(
            report,
            &format!("{subject}.target_location"),
            location,
            saved.target_location,
        );
        value(
            report,
            &format!("{subject}.target_country"),
            country,
            saved.target_country.as_deref(),
        );
        repair_levels(
            report,
            &format!("{subject}.repair_levels"),
            row.repair_levels,
            saved.repair_levels,
        );
    }
    missing(
        report,
        "building construction",
        &seen,
        saved.building_construction.keys().copied(),
    );
    report.compared("BuildingConstruction", matched);
    report.limitation("construction target comparison uses the serializer's CBuildingReference projection; discarded status flag bits and unused location/country operands are not individually serialized", "BuildingConstruction");
    report_railway_gun_repairs(saved, report);
    Ok(())
}

const RAILWAY_GUN_REPAIR_LIMITATION: &str =
    "railway_gun repair queue rows are not recorded; they are omitted from comparison while preserving subsequent construction queue ordinals";

fn report_railway_gun_repairs(saved: &saved::Pack, report: &mut Report) {
    for entry in &saved.railway_gun_repairs {
        let mut subject = format!(
            "{}.general_lines[{}].railway_gun",
            String::from_utf8_lossy(&entry.country),
            entry.queue_ordinal
        );
        if let Some(id) = entry.id {
            subject.push_str(&format!(" id={id}"));
        }
        if let Some(gun) = entry.railway_gun {
            subject.push_str(&format!(" target={gun}"));
        }
        let already_reported = report
            .limitations
            .get(RAILWAY_GUN_REPAIR_LIMITATION)
            .is_some_and(|subjects| subjects.contains(&subject));
        if !already_reported {
            report.limitation(RAILWAY_GUN_REPAIR_LIMITATION, subject);
        }
    }
}

pub fn railway_construction(
    rows: &[replay_data::rows::RailwayConstructionRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        if !seen.insert(row.id) {
            return Err(invalid("duplicate full railway construction handle"));
        }
        let subject = format!("railway construction.{}", row.id);
        let Some(saved) = saved.railway_construction.get(&row.id) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        production_fields!(
            &mut *report,
            subject.as_str(),
            row,
            &saved.production,
            links
        );
        value(
            report,
            &format!("{subject}.base_level"),
            row.base_level,
            saved.base_level,
        );
        value(
            report,
            &format!("{subject}.target_level"),
            row.target_level,
            saved.target_level,
        );
        value(
            report,
            &format!("{subject}.cancelled"),
            row.cancelled != 0,
            saved.cancelled,
        );
        value(
            report,
            &format!("{subject}.current_segment"),
            row.current_segment,
            saved.current_segment,
        );
        value(
            report,
            &format!("{subject}.route"),
            row.route.as_slice(),
            saved.route.as_slice(),
        );
        report.scalar(
            &subject,
            "remaining_cost",
            Some(row.remaining_cost),
            Some(&saved.remaining_cost),
        );
    }
    missing(
        report,
        "railway construction",
        &seen,
        saved.railway_construction.keys().copied(),
    );
    report.compared("RailwayConstruction", matched);
    report_railway_gun_repairs(saved, report);
    Ok(())
}

mod occupation;
pub use occupation::{
    assigned_manpower, collaboration, garrison_equipment, garrisons, occupation_laws,
    occupation_settings, reserve_manpower, resistance, resistance_hover, resistance_map,
};

mod states;
pub use states::{
    building_overrides, resource_inputs, state_cores, state_modifiers, state_pools,
    state_strategic_locations,
};
mod resource_map;
pub use resource_map::{ResourceMapRows, resource_map};

fn value<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    subject: &str,
    captured: T,
    saved: T,
) {
    if captured != saved {
        report.difference(subject, captured, saved);
    }
}

fn missing<K: std::fmt::Debug + Ord>(
    report: &mut Report,
    table: &str,
    captured: &BTreeSet<K>,
    saved: impl Iterator<Item = K>,
) {
    for key in saved {
        if !captured.contains(&key) {
            report.difference(format!("{table}.{key:?}"), "missing row", "present");
        }
    }
}

fn token<'a>(names: &'a BTreeMap<u32, &[u8]>, id: u32) -> Result<&'a [u8], &'static str> {
    let literal = names
        .get(&id)
        .copied()
        .ok_or("type is absent from the recorded lexical dictionary")?;
    if literal.contains(&0) {
        return Err("type literal contains NUL and its mapping to text is unproved");
    }
    Ok(literal)
}

fn repair_levels(report: &mut Report, subject: &str, captured: i32, saved: Option<i32>) {
    match saved {
        Some(saved) => value(report, subject, captured, saved),
        None if captured > 0 => report.difference(subject, captured, "omitted: nonpositive"),
        None => report.limitation(
            "building construction serializer omits every nonpositive repair count; its exact omitted raw value is not compared",
            subject,
        ),
    }
}

fn manpower(
    report: &mut Report,
    subject: &str,
    captured: impl IntoIterator<Item = (u32, u32, i32)>,
    saved: &[saved::Manpower],
    links: &BTreeMap<u32, &CountryRow>,
) -> io::Result<()> {
    let mut captured = captured.into_iter().collect::<Vec<_>>();
    captured.sort_unstable_by_key(|entry| entry.0);
    if captured.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid(format!(
            "{subject} has duplicate manpower ordinals"
        )));
    }
    let mut serialized = Vec::new();
    for (ordinal, source, amount) in captured {
        if amount > 0 {
            serialized.push((required_country_tag(source, links)?, amount));
        } else {
            report.limitation(
                "manpower serializer omits nonpositive entries; their source, amount and native ordinal cannot be compared to the save",
                format!("{subject}[{ordinal}]"),
            );
        }
    }
    let saved = saved
        .iter()
        .map(|row| (row.source_country.as_slice(), row.amount))
        .collect::<Vec<_>>();
    value(report, subject, serialized, saved);
    Ok(())
}

#[cfg(test)]
mod tests;

mod output;
pub use output::compare_output;
#[cfg(test)]
use output::{ENDED_ADDITION, OUTPUT_TABLES, OutputCheck};

#[cfg(test)]
mod output_tests;
