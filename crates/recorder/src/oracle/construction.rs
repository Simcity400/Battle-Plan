//! Construction and occupation comparisons retain native precision and scope.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::save::construction as saved;
use replay::{
    Hour,
    container::Replay,
    frame::{DecodedFrame, FrameDecoder},
    rows::{ConstructionLineOutputRow, CountryRow, TableId, tables},
};

use super::{Report, complete, daily_read, invalid, required_country_tag};

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

/// `NavalBases` against the saved port province buildings.
///
/// province's eligible port buildings, the content's `is_port` types, so they
/// are the sums of those buildings' saved `level` and `healthy_levels`. The
/// base's own country (`(CNavalBase + 8) + 0x10`) and its `+0x40` country list
/// are not serialized at all, so they are stated as a limitation.
pub(super) fn naval_bases(
    rows: &[replay::rows::NavalBaseRow],
    saved: &BTreeMap<saved::BuildingKey, saved::Building>,
    port_types: &BTreeSet<Vec<u8>>,
    report: &mut Report,
) -> io::Result<()> {
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
    for row in rows {
        if !seen.insert(row.province) {
            return Err(invalid("duplicate naval base province"));
        }
        let Some(&(level, healthy_level)) = ports.get(&row.province) else {
            // The hash keeps a base while its level is zero, so only a
            // positive level has to have a saved building behind it.
            if row.level > 0 {
                report.difference(
                    format!("NavalBase {} membership", row.province),
                    "replay row with a positive level",
                    "no saved port building",
                );
            }
            continue;
        };
        let label = format!("NavalBase {}", row.province);
        if row.countries_unreadable {
            report.difference(
                format!("{label}.countries"),
                "unreadable: null list with a positive count",
                "a list the game can read",
            );
        }
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
    for province in ports.keys() {
        if !seen.contains(province) {
            report.difference(
                format!("NavalBase {province} membership"),
                "absent from replay",
                "saved port building",
            );
        }
    }
    report.compared("NavalBases", count);
    report.limitation(
        "the naval base's own country and its country list are not serialized; only its province membership and its port levels can be compared",
        "NavalBases",
    );
    Ok(())
}

pub(super) fn buildings(
    rows: &[replay::rows::BuildingRow],
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
        // The active vector is not serialized. It is rebuilt from the
        // building list (+0x38) as the positive levels in native order
        // (`rebuild_active_buildings@1411641D0`) at the end of every status
        // block and on every zero crossing of `set_level@1410C71A0`. It is
        // recorded because it is the stored value the F61B40 scan reads.
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

pub(super) fn base_resources(
    rows: &[replay::rows::StateBaseResourceRow],
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

pub(super) fn partial_base_resources(
    rows: &[replay::rows::StateBaseResourceRow],
    content: &saved::PartialBaseResources,
    membership: &installed_content::save::PartialMapMembership,
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

pub(super) fn building_construction(
    rows: &[replay::rows::BuildingConstructionRow],
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
        // 1163510 constructs one CBuildingReference, preferring province scope
        // before a signed-positive country and finally the resolved state.
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
    Ok(())
}

pub(super) fn railway_construction(
    rows: &[replay::rows::RailwayConstructionRow],
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
    Ok(())
}

mod occupation;
pub(super) use occupation::{
    assigned_manpower, collaboration, garrisons, occupation_laws, occupation_settings,
    reserve_manpower, resistance,
};

mod states;
pub(super) use states::{
    building_overrides, resource_inputs, state_cores, state_modifiers, state_pools,
};

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
    // The dictionary stores the full native string length. AAC50 forwards its
    // c_str to text-stream virtual+30; that sink's NUL handling is unproved.
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

/// C5B610 emits only positive amounts, in native vector order. Filtering occurs
/// only in this comparison; the replay keeps every raw row and its ordinal.
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
pub(super) use output::compare_output;
#[cfg(test)]
use output::{OUTPUT_TABLES, OutputCheck};

#[cfg(test)]
mod output_tests;
