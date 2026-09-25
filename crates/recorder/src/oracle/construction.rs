//! Construction and occupation comparisons retain native precision and scope.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::save::construction as saved;
use replay::rows::CountryRow;

use super::{Report, invalid, required_country_tag};

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
    report.limitation("building serializers preserve list order, levels and partial health; raw CBuildingStatus flags and country field are not serialized", table);
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

pub(super) fn occupation_settings(
    rows: &[replay::rows::OccupationSettingsRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        let country = required_country_tag(row.country, links)?;
        if !seen.insert(country.to_vec()) {
            return Err(invalid("duplicate resolved occupation settings country"));
        }
        let subject = format!("occupation settings.{}", String::from_utf8_lossy(country));
        let Some(saved) = saved.occupation_settings.get(country) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        value(
            report,
            &format!("{subject}.garrison_template"),
            row.garrison_template,
            saved.garrison_template,
        );
        value(
            report,
            &format!("{subject}.default_law_present"),
            row.default_law_present,
            u8::from(saved.default_law.is_some()),
        );
        if let Some(law) = &saved.default_law {
            value(
                report,
                &format!("{subject}.default_law"),
                row.law.as_slice(),
                law.as_slice(),
            );
        }
    }
    missing(
        report,
        "occupation settings",
        &seen,
        saved.occupation_settings.keys().cloned(),
    );
    report.compared("OccupationSettings", matched);
    Ok(())
}

pub(super) fn occupation_laws(
    rows: &[replay::rows::StateOccupationLawRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        let controller = required_country_tag(row.controller, links)?;
        let occupied = required_country_tag(row.occupied, links)?;
        let key = (controller.to_vec(), occupied.to_vec(), row.state);
        if !seen.insert(key.clone()) {
            return Err(invalid(
                "duplicate resolved controller/occupied/state law key",
            ));
        }
        let subject = format!(
            "occupation law.{}.{}/{:}",
            String::from_utf8_lossy(controller),
            String::from_utf8_lossy(occupied),
            row.state
        );
        let Some(saved) = saved.state_occupation_laws.get(&key) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        value(report, &subject, row.name.as_slice(), saved.law.as_slice());
    }
    missing(
        report,
        "occupation law",
        &seen,
        saved.state_occupation_laws.keys().cloned(),
    );
    report.compared("StateOccupationLaws", matched);
    Ok(())
}

pub(super) fn garrisons(
    rows: &[replay::rows::StateGarrisonRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        let controller = required_country_tag(row.controller, links)?;
        let occupied = required_country_tag(row.occupied, links)?;
        let key = (controller.to_vec(), row.state);
        if !seen.insert(key.clone()) {
            return Err(invalid("duplicate resolved controller/state garrison key"));
        }
        let subject = format!(
            "garrison.{}.{:}",
            String::from_utf8_lossy(controller),
            row.state
        );
        let Some(saved) = saved.state_garrisons.get(&key) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        value(
            report,
            &format!("{subject}.occupied"),
            occupied,
            saved.occupied.as_slice(),
        );
        value(
            report,
            &format!("{subject}.required_manpower"),
            row.required_manpower,
            saved.required_manpower,
        );
        report.scalar(
            &subject,
            "strength_ratio",
            Some(row.strength_ratio),
            Some(&saved.strength_ratio),
        );
        report.scalar(
            &subject,
            "garrison_required",
            Some(row.garrison_required),
            Some(&saved.garrison_required),
        );
        value(
            report,
            &format!("{subject}.serialized_flag"),
            row.serialized_flag != 0,
            saved.serialized_flag,
        );
        if row.serialized_flag > 1 {
            report.limitation("garrison serializer represents its flag as a boolean; nonzero byte magnitude is not serialized", subject);
        }
    }
    missing(
        report,
        "garrison",
        &seen,
        saved.state_garrisons.keys().cloned(),
    );
    report.compared("StateGarrisons", matched);
    Ok(())
}

pub(super) fn assigned_manpower(
    rows: &[replay::rows::StateAssignedManpowerRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut groups = BTreeMap::<saved::GarrisonKey, Vec<(u32, u32, i32)>>::new();
    for row in rows {
        let controller = required_country_tag(row.controller, links)?;
        let key = (controller.to_vec(), row.state);
        let subject = format!(
            "assigned manpower.{}.{:}",
            String::from_utf8_lossy(controller),
            row.state
        );
        let Some(saved) = saved.state_garrisons.get(&key) else {
            report.difference(subject, "present", "missing saved garrison");
            continue;
        };
        value(
            report,
            &format!("{subject}.occupied"),
            required_country_tag(row.occupied, links)?,
            saved.occupied.as_slice(),
        );
        groups
            .entry(key)
            .or_default()
            .push((row.ordinal, row.source_country, row.amount));
    }
    let mut matched = 0;
    for (key, garrison) in &saved.state_garrisons {
        let rows = groups.remove(key).unwrap_or_default();
        matched += rows
            .iter()
            .filter(|row| row.2 > 0)
            .count()
            .min(garrison.manpower.len());
        manpower(
            report,
            &format!(
                "assigned manpower.{}.{:}",
                String::from_utf8_lossy(&key.0),
                key.1
            ),
            rows,
            &garrison.manpower,
            links,
        )?;
    }
    report.compared("StateAssignedManpower", matched);
    Ok(())
}

pub(super) fn reserve_manpower(
    rows: &[replay::rows::CountryOccupationReserveManpowerRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut groups = BTreeMap::<Vec<u8>, Vec<(u32, u32, i32)>>::new();
    for row in rows {
        let country = required_country_tag(row.country, links)?;
        if !saved.country_reserve_manpower.contains_key(country) {
            report.difference(
                format!("occupation reserve.{}", String::from_utf8_lossy(country)),
                "present",
                "missing saved occupation status",
            );
            continue;
        }
        groups.entry(country.to_vec()).or_default().push((
            row.ordinal,
            row.source_country,
            row.amount,
        ));
    }
    let mut matched = 0;
    for (country, saved) in &saved.country_reserve_manpower {
        let rows = groups.remove(country).unwrap_or_default();
        matched += rows.iter().filter(|row| row.2 > 0).count().min(saved.len());
        manpower(
            report,
            &format!("occupation reserve.{}", String::from_utf8_lossy(country)),
            rows,
            saved,
            links,
        )?;
    }
    report.compared("CountryOccupationReserveManpower", matched);
    Ok(())
}

pub(super) fn resistance(
    rows: &[replay::rows::StateResistanceRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        if !seen.insert(row.state) {
            return Err(invalid("duplicate captured state resistance"));
        }
        let subject = format!("resistance.{}", row.state);
        let Some(saved) = saved.state_resistance.get(&row.state) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        report.scalar(&subject, "level", Some(row.level), Some(&saved.level));
        report.scalar(&subject, "speed", Some(row.speed), Some(&saved.speed));
        report.scalar(
            &subject,
            "compliance",
            Some(row.compliance),
            Some(&saved.compliance),
        );
        report.scalar(
            &subject,
            "compliance_speed",
            Some(row.compliance_speed),
            Some(&saved.compliance_speed),
        );
        value(
            report,
            &format!("{subject}.active"),
            row.active != 0,
            saved.active,
        );
        if (row.country as i32) > 0 {
            value(
                report,
                &format!("{subject}.country"),
                Some(required_country_tag(row.country, links)?),
                saved.country.as_deref(),
            );
        } else if saved.country.is_some() {
            report.difference(
                format!("{subject}.country"),
                "omitted: signed-nonpositive",
                &saved.country,
            );
        } else {
            report.limitation("resistance serializer omits signed-nonpositive country links; the exact omitted raw link is not compared", format!("{subject}.country"));
        }
        if row.active > 1 {
            report.limitation("resistance serializer records active as a boolean; nonzero byte magnitude is not serialized", subject);
        }
    }
    missing(
        report,
        "resistance",
        &seen,
        saved.state_resistance.keys().copied(),
    );
    report.compared("StateResistance", matched);
    Ok(())
}

pub(super) fn collaboration(
    rows: &[replay::rows::CountryCollaborationRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows {
        let source = required_country_tag(row.source, links)?;
        let target = required_country_tag(row.target, links)?;
        let key = (source.to_vec(), target.to_vec());
        if !seen.insert(key.clone()) {
            return Err(invalid("duplicate resolved country collaboration pair"));
        }
        let subject = format!(
            "collaboration.{}.{}",
            String::from_utf8_lossy(source),
            String::from_utf8_lossy(target)
        );
        let Some(saved) = saved.country_collaboration.get(&key) else {
            report.difference(subject, "present", "missing row");
            continue;
        };
        matched += 1;
        report.scalar(
            &subject,
            "progress",
            Some(row.progress),
            Some(&saved.progress),
        );
    }
    missing(
        report,
        "collaboration",
        &seen,
        saved.country_collaboration.keys().cloned(),
    );
    report.compared("CountryCollaboration", matched);
    Ok(())
}

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
mod tests {
    use super::*;

    fn saved_pack(text: &[u8]) -> saved::Pack {
        saved::parse(&installed_content::parser::parse(text).unwrap().items).unwrap()
    }

    #[test]
    fn building_and_railway_comparisons_detect_one_native_unit_and_order_changes() {
        let saved = saved_pack(b"countries={GER={production={general_lines={rail_way={id={id=2 type=4713} cost=90071992547.40993 priority=-1 amount=1 created_date=1936.1.1.12 base=-1 target=2 current_province=-1 remaining_hours=-0.00001 rail_way={7 4 7}}}}}} states={1={buildings={factory={level=-32768 healthy_levels=-2 partial_health=0.00001}}}} provinces={}");
        let germany = CountryRow {
            id: 12,
            canonical_slot: 3,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany)]);
        let names = BTreeMap::from([(31, b"factory".as_slice())]);
        let building = replay::rows::BuildingRow {
            location: 1,
            building_type: 31,
            level: i16::MIN,
            healthy_level: -2,
            partial_health: 1,
            ..Default::default()
        };
        let mut rail = replay::rows::RailwayConstructionRow {
            id: (2_u64 << 32) | 4713,
            country: 12,
            cost: 9_007_199_254_740_993,
            priority: -1,
            amount: 1,
            created_hour: installed_content::save::GameDate::parse(b"1936.1.1.12")
                .unwrap()
                .engine_hour()
                .unwrap(),
            base_level: -1,
            target_level: 2,
            current_segment: -1,
            remaining_cost: -1,
            route: vec![7, 4, 7],
            ..Default::default()
        };
        let mut report = Report::default();
        buildings(
            &[building],
            &saved.state_buildings,
            false,
            &names,
            &mut report,
        )
        .unwrap();
        railway_construction(&[rail.clone()], &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        rail.cost -= 1;
        rail.route.swap(0, 1);
        rail.created_hour += 1;
        railway_construction(&[rail], &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3);
    }

    #[test]
    fn occupation_template_id_retains_full_handle_and_null_omission() {
        let saved = saved_pack(b"countries={GER={occupation_status={division_template_id={id=4294967295 type=52} default_law=\"\"}} FRA={occupation_status={}}} states={} provinces={}");
        let germany = CountryRow {
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let france = CountryRow {
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany), (22, &france)]);
        let mut rows = [
            replay::rows::OccupationSettingsRow {
                country: 12,
                garrison_template: Some((u64::from(u32::MAX) << 32) | 52),
                default_law_present: 1,
                law: Vec::new(),
            },
            replay::rows::OccupationSettingsRow {
                country: 22,
                ..Default::default()
            },
        ];
        let mut report = Report::default();
        occupation_settings(&rows, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        rows[0].garrison_template = Some((u64::from(u32::MAX - 1) << 32) | 52);
        occupation_settings(&rows, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("garrison_template"));

        rows[0].garrison_template = Some((u64::from(u32::MAX) << 32) | 52);
        rows[1].garrison_template = Some((1_u64 << 32) | 52);
        let mut report = Report::default();
        occupation_settings(&rows, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("FRA.garrison_template"));
    }

    #[test]
    fn laws_with_zero_state_remain_distinct_in_different_occupied_scopes() {
        let saved = saved_pack(b"countries={GER={occupation_status={occupation={FRA={occupation_law_list={0=a}} ENG={occupation_law_list={0=b}}}}}} states={} provinces={}");
        let germany = CountryRow {
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let france = CountryRow {
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let britain = CountryRow {
            tag: b"ENG".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany), (22, &france), (32, &britain)]);
        let mut rows = [
            replay::rows::StateOccupationLawRow {
                controller: 12,
                occupied: 22,
                state: 0,
                name: b"a".to_vec(),
            },
            replay::rows::StateOccupationLawRow {
                controller: 12,
                occupied: 32,
                state: 0,
                name: b"b".to_vec(),
            },
        ];
        let mut report = Report::default();
        occupation_laws(&rows, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        rows[0].name = b"b".to_vec();
        occupation_laws(&rows, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn resistance_country_omission_does_not_certify_raw_zero() {
        let saved = saved_pack(b"countries={} states={1={resistance={resistance=90071992547.40993 resistance_speed=-0.00001}}} provinces={}");
        let mut row = replay::rows::StateResistanceRow {
            state: 1,
            country: u32::MAX,
            level: 9_007_199_254_740_993,
            speed: -1,
            ..Default::default()
        };
        let mut report = Report::default();
        resistance(&[row.clone()], &saved, &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("signed-nonpositive"))
        );
        row.level -= 1;
        resistance(&[row], &saved, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn repair_omission_is_an_inequality_not_a_zero_value() {
        let mut report = Report::default();
        repair_levels(&mut report, "negative", -9, None);
        repair_levels(&mut report, "zero", 0, None);
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.values().map(Vec::len).sum::<usize>(), 2);
        repair_levels(&mut report, "positive", 1, None);
        repair_levels(&mut report, "present", -1, Some(1));
        assert_eq!(report.differences.len(), 2);
    }

    #[test]
    fn dictionary_missing_entry_is_explicit_and_literal_bytes_are_preserved() {
        let names = BTreeMap::from([(17, b"\xff".as_slice()), (19, b"factory\0tail".as_slice())]);
        assert_eq!(token(&names, 17).unwrap(), b"\xff");
        assert!(token(&names, 18).unwrap_err().contains("absent"));
        assert!(
            token(&names, 19)
                .unwrap_err()
                .contains("mapping to text is unproved")
        );
        assert_eq!(names[&19], b"factory\0tail");
    }

    #[test]
    fn missing_dictionary_preserves_scope_checks_without_inventing_named_rows() {
        let saved = saved_pack(b"countries={} states={} provinces={7={buildings={factory={level=1 healthy_levels=1 partial_health=0}}}}");
        let row = replay::rows::BuildingRow {
            location: 7,
            building_type: 99,
            ordinal: 77,
            level: i16::MIN,
            healthy_level: i16::MIN,
            partial_health: i64::MIN,
            ..Default::default()
        };
        let mut report = Report::default();
        buildings(
            std::slice::from_ref(&row),
            &saved.province_buildings,
            true,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["ProvinceBuildings"], 0);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("levels and partial health cannot be matched"))
        );

        let mut extra = row.clone();
        extra.building_type += 1;
        let mut report = Report::default();
        buildings(
            &[row, extra],
            &saved.province_buildings,
            true,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("row_count"));

        let mut content = saved::BaseResources::default();
        content
            .add_state_history(b"state={id=1 resources={steel=3 oil=7}}")
            .unwrap();
        let resources = [
            replay::rows::StateBaseResourceRow {
                state: 1,
                resource: 91,
                ordinal: 77,
                amount: i64::MIN,
            },
            replay::rows::StateBaseResourceRow {
                state: 1,
                resource: 92,
                ordinal: 1,
                amount: 700_001,
            },
        ];
        let mut report = Report::default();
        base_resources(
            &resources,
            &content,
            &BTreeMap::from([(92, b"oil".as_slice())]),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("oil.amount"));
        assert_eq!(
            report.compared_tables["StateBaseResources (verified content)"],
            1
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("amount cannot be matched"))
        );
    }

    #[test]
    fn nul_token_mapping_limits_keep_independent_scope_count_and_numeric_checks() {
        let saved = saved_pack(b"countries={GER={production={general_lines={building={id={id=4 type=59} cost=1 priority=0 amount=1 created_date=1936.1.1.12 building={template=factory location=3}}}}}} states={} provinces={7={buildings={factory={level=1 healthy_levels=1 partial_health=0}}}}");
        let names = BTreeMap::from([
            (11, b"factory\0tail".as_slice()),
            (12, b"oil\0tail".as_slice()),
        ]);
        let mut report = Report::default();
        buildings(
            &[replay::rows::BuildingRow {
                location: 7,
                building_type: 11,
                level: i16::MIN,
                partial_health: i64::MIN,
                ..Default::default()
            }],
            &saved.province_buildings,
            true,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        let mut content = saved::BaseResources::default();
        content
            .add_state_history(b"state={id=1 resources={oil=3 steel=0}}")
            .unwrap();
        base_resources(
            &[replay::rows::StateBaseResourceRow {
                state: 1,
                resource: 12,
                ordinal: 77,
                amount: i64::MIN,
            }],
            &content,
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("row_count"));

        let germany = CountryRow {
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        building_construction(
            &[replay::rows::BuildingConstructionRow {
                id: (4_u64 << 32) | 59,
                country: 12,
                cost: 100_001,
                amount: 1,
                created_hour: installed_content::save::GameDate::parse(b"1936.1.1.12")
                    .unwrap()
                    .engine_hour()
                    .unwrap(),
                building_type: 11,
                target_state: 4,
                ..Default::default()
            }],
            &saved,
            &BTreeMap::from([(12, &germany)]),
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        assert!(
            report.differences[1..]
                .iter()
                .any(|difference| difference.contains("cost"))
        );
        assert!(
            report.differences[1..]
                .iter()
                .any(|difference| difference.contains("target_location"))
        );
        assert_eq!(report.compared_tables["ProvinceBuildings"], 0);
        assert_eq!(
            report.compared_tables["StateBaseResources (verified content)"],
            0
        );
        assert_eq!(report.compared_tables["BuildingConstruction"], 1);
        assert_eq!(
            report
                .limitations
                .keys()
                .filter(|reason| reason.contains("mapping to text is unproved"))
                .count(),
            3
        );
    }

    #[test]
    fn resolved_province_buildings_compare_signed_zero_values_and_named_membership() {
        let saved = saved_pack(b"countries={} states={7={buildings={port={level=9 healthy_levels=9 partial_health=0}}}} provinces={7={buildings={factory={level=0 healthy_levels=0 partial_health=0} port={level=-32768 healthy_levels=-1 partial_health=-90071992547.40993}}}}");
        let names = BTreeMap::from([(91, b"factory".as_slice()), (92, b"port".as_slice())]);
        let rows = [
            replay::rows::BuildingRow {
                location: 7,
                building_type: 91,
                ..Default::default()
            },
            replay::rows::BuildingRow {
                location: 7,
                building_type: 92,
                ordinal: 1,
                level: i16::MIN,
                healthy_level: -1,
                partial_health: -9_007_199_254_740_993,
                ..Default::default()
            },
        ];
        let mut report = Report::default();
        buildings(&rows, &saved.province_buildings, true, &names, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["ProvinceBuildings"], 2);

        let mut changed = rows.clone();
        changed[0].partial_health = 1;
        changed[1].healthy_level -= 1;
        changed[1].ordinal = 0;
        let mut report = Report::default();
        buildings(
            &changed,
            &saved.province_buildings,
            true,
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);

        let wrong_names = BTreeMap::from([(91, b"factory".as_slice()), (92, b"other".as_slice())]);
        let mut report = Report::default();
        buildings(
            &rows,
            &saved.province_buildings,
            true,
            &wrong_names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert_eq!(report.compared_tables["ProvinceBuildings"], 1);

        assert!(
            buildings(
                &[rows[0].clone(), rows[0].clone()],
                &saved.province_buildings,
                true,
                &names,
                &mut Report::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn naval_bases_sum_the_contents_port_buildings() {
        let saved = saved_pack(b"countries={} states={} provinces={7={buildings={dock={level=2 healthy_levels=1 partial_health=0} pier={level=3 healthy_levels=3 partial_health=0} factory={level=5 healthy_levels=5 partial_health=0}}}}");
        let ports = BTreeSet::from([b"dock".to_vec(), b"pier".to_vec()]);
        let mut row = replay::rows::NavalBaseRow {
            province: 7,
            level: 5,
            healthy_level: 4,
            ..Default::default()
        };
        let mut report = Report::default();
        naval_bases(
            std::slice::from_ref(&row),
            &saved.province_buildings,
            &ports,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        row.level = 2;
        let mut report = Report::default();
        naval_bases(
            &[row.clone()],
            &saved.province_buildings,
            &ports,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        // An unreadable country list is named on its own base, never waived.
        row.level = 5;
        row.countries_unreadable = true;
        let mut report = Report::default();
        naval_bases(&[row], &saved.province_buildings, &ports, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert_eq!(report.compared_tables["NavalBases"], 1);
    }

    #[test]
    fn resolved_base_resources_compare_repeated_initial_blocks_and_native_ordinals() {
        let mut content = saved::BaseResources::default();
        content.add_state_history(b"state={id=949 resources={steel=0 oil=-2147483648.9} history={resources={steel=999 aluminium=888}} resources={} resources={aluminium=10.000}}")
            .unwrap();
        let names = BTreeMap::from([
            (91, b"steel".as_slice()),
            (92, b"oil".as_slice()),
            (93, b"aluminium".as_slice()),
        ]);
        let rows = [
            replay::rows::StateBaseResourceRow {
                state: 949,
                resource: 91,
                ordinal: 0,
                amount: 0,
            },
            replay::rows::StateBaseResourceRow {
                state: 949,
                resource: 92,
                ordinal: 1,
                amount: -214_748_364_800_000,
            },
            replay::rows::StateBaseResourceRow {
                state: 949,
                resource: 93,
                ordinal: 2,
                amount: 1_000_000,
            },
        ];
        let mut report = Report::default();
        base_resources(&rows, &content, &names, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.compared_tables["StateBaseResources (verified content)"],
            3
        );

        let mut changed = rows.clone();
        changed[0].amount = 1;
        changed[1].amount += 1;
        changed[2].ordinal = 0;
        let mut report = Report::default();
        base_resources(&changed, &content, &names, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);

        let mut wrong_names = names.clone();
        wrong_names.insert(92, b"chromium".as_slice());
        let mut report = Report::default();
        base_resources(&rows, &content, &wrong_names, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert_eq!(
            report.compared_tables["StateBaseResources (verified content)"],
            2
        );
        assert!(
            base_resources(
                &[rows[0].clone(), rows[0].clone()],
                &content,
                &names,
                &mut Report::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn resolved_construction_names_retain_exact_numeric_and_queue_checks() {
        let saved = saved_pack(b"countries={GER={production={general_lines={building={id={id=4294967295 type=59} cost=0 speed=-0.00001 produced=-90071992547.40993 active_factories=-2 priority=-1 amount=0 created_date=1936.1.1.12 building={template=factory location=3} to_repair=1}}}}} states={} provinces={}");
        let germany = CountryRow {
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany)]);
        let names = BTreeMap::from([(91, b"factory".as_slice())]);
        let row = replay::rows::BuildingConstructionRow {
            id: (u64::from(u32::MAX) << 32) | 59,
            country: 12,
            active_factories: -2,
            speed: -1,
            produced: -9_007_199_254_740_993,
            priority: -1,
            created_hour: installed_content::save::GameDate::parse(b"1936.1.1.12")
                .unwrap()
                .engine_hour()
                .unwrap(),
            building_type: 91,
            target_flags: 2,
            target_location: u32::MAX,
            target_state: 3,
            target_country: u32::MAX,
            repair_levels: 1,
            ..Default::default()
        };
        let mut report = Report::default();
        building_construction(
            std::slice::from_ref(&row),
            &saved,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["BuildingConstruction"], 1);

        let mut changed = row.clone();
        changed.cost = 1;
        changed.produced += 1;
        changed.queue_ordinal = 1;
        let mut report = Report::default();
        building_construction(&[changed], &saved, &links, &names, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);

        let mut report = Report::default();
        building_construction(
            std::slice::from_ref(&row),
            &saved,
            &links,
            &BTreeMap::from([(91, b"port".as_slice())]),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("building_type"));
        assert!(
            building_construction(
                &[row.clone(), row],
                &saved,
                &links,
                &names,
                &mut Report::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn missing_construction_name_does_not_skip_numeric_or_target_comparison() {
        let saved = saved_pack(b"countries={GER={production={general_lines={building={id={id=4 type=59} cost=90071992547.40993 priority=-2 amount=1 created_date=1936.1.1.12 building={template=factory location=3}}}}}} states={} provinces={}");
        let germany = CountryRow {
            id: 12,
            canonical_slot: 3,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany)]);
        let row = replay::rows::BuildingConstructionRow {
            id: (4_u64 << 32) | 59,
            country: 12,
            cost: 9_007_199_254_740_994,
            priority: -2,
            amount: 1,
            created_hour: installed_content::save::GameDate::parse(b"1936.1.1.12")
                .unwrap()
                .engine_hour()
                .unwrap(),
            building_type: 91,
            target_flags: 2,
            target_location: 3,
            target_state: 4,
            ..Default::default()
        };
        let mut report = Report::default();
        building_construction(&[row], &saved, &links, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("cost"))
        );
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("target_location"))
        );
        assert_eq!(report.compared_tables["BuildingConstruction"], 1);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("construction building type is absent"))
        );
    }

    #[test]
    fn manpower_filter_retains_relative_order_and_resolves_country_aliases() {
        let germany = CountryRow {
            id: 12,
            canonical_slot: 3,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let france = CountryRow {
            id: 22,
            canonical_slot: 4,
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &germany), (112, &germany), (22, &france)]);
        let saved = [
            saved::Manpower {
                source_country: b"GER".to_vec(),
                amount: 9,
            },
            saved::Manpower {
                source_country: b"FRA".to_vec(),
                amount: 7,
            },
        ];
        let mut report = Report::default();
        manpower(
            &mut report,
            "reserve",
            [(9, 22, 7), (1, u32::MAX, -2), (4, 112, 9)],
            &saved,
            &links,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.values().map(Vec::len).sum::<usize>(), 1);
        manpower(
            &mut report,
            "reserve",
            [(0, 22, 7), (1, 12, 9)],
            &saved,
            &links,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(
            manpower(
                &mut report,
                "duplicate",
                [(0, 12, 7), (0, 12, 9)],
                &saved,
                &links
            )
            .is_err()
        );
        assert!(manpower(&mut report, "unresolved", [(0, 999, 7)], &saved, &links).is_err());
    }
}
