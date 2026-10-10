//! Exact-hour checks for the genuine selected-attack input suffixes.
//!
//! Selection, hovered targets and preview arithmetic never enter this oracle.
//! A saved identity comparison is independent of a packet consistency check;
//! neither certifies the runtime-only numeric inputs. This integrated source is
//! uncompiled and unrun. Serializer omission evidence is separately pinned.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::save::Save;
use replay::{
    frame::DecodedFrame,
    rows::{tables, *},
};

use super::{ContentRead, Report, invalid, pass_inputs::eq};

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    buildings: ContentRead<'_, installed_content::buildings::Buildings>,
    report: &mut Report,
) {
    // These are suffix groups, not replacement table comparisons. Existing
    // division/template/group/weather/building comparisons still run.
    report.attempt("DivisionStatus selected attack", |r| {
        divisions(frame, saved, r)
    });
    report.attempt("OrdersGroupCounterInputs selected attack", |r| {
        groups(frame, saved, r)
    });
    report.attempt("WeatherRegions selected attack", |r| {
        regions(frame, saved, r)
    });
    report.attempt("Provinces selected attack", |r| {
        provinces(frame, saved, &buildings, r)
    });
    report.attempt("ReservedCountryNaming exact raw observations", |r| {
        raw_observations(frame, r)
    });
    report.attempt("CountryCounterInputs selected attack", |r| {
        countries(frame, r)
    });
    report.attempt("SimulationFeatures political runtime scalars", |r| {
        scalars(frame, r)
    });
}

fn fresh(report: &mut Report, label: &str, sample_hour: i32, hour: i32) -> bool {
    eq(report, &format!("{label}.sample_hour"), sample_hour, hour);
    sample_hour == hour
}

fn required<T: std::fmt::Debug>(report: &mut Report, label: &str, value: &Option<T>) {
    if value.is_none() {
        report.difference(
            label,
            "capture refused",
            "required current-hour native input",
        );
    }
}

fn terrain(report: &mut Report, label: &str, values: &Option<Vec<AttackTerrain>>) {
    required(report, label, values);
    let Some(values) = values else {
        return;
    };
    let mut previous = 0;
    for (ordinal, entry) in values.iter().enumerate() {
        if entry.terrain <= previous {
            report.difference(
                format!("{label}[{ordinal}].terrain"),
                entry.terrain,
                format!("positive key strictly above {previous}"),
            );
        }
        previous = entry.terrain;
        required(
            report,
            &format!("{label}[{}].attack", entry.terrain),
            &entry.attack,
        );
    }
    report.limitation("positive terrain ordering and individual read presence are packet checks only; the live terrain domain, cache/static adjuster values and completeness require a matched native witness", label);
}

fn divisions(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::DivisionStatus>()?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.division == 0 || !seen.insert(row.division) {
            return Err(invalid(
                "null or duplicate selected-attack division identity",
            ));
        }
        let label = format!("DivisionStatus {}.attack", row.division);
        let expected = saved.divisions.get(&row.division);
        if expected.is_none() {
            report.difference(format!("{label} saved division"), "present", "absent");
        }
        let Some(packet) = report.recorded(&row.attack, "DivisionStatus.attack", &label) else {
            continue;
        };
        if !fresh(report, &label, packet.sample_hour, frame.hour) {
            continue;
        }
        required(report, &format!("{label}.template"), &packet.template);
        if let Some(handle) = packet.template {
            if handle != 0
                && (handle as u32 != engine::creferenced_division_template::REFERENCE_TYPE
                    || handle >> 32 == 0)
            {
                report.difference(
                    format!("{label}.template typed identity"),
                    handle,
                    "null or nonzero full type52 reference",
                );
            }
            if let Some(expected) = expected {
                // The army writer saves the standard wrapper identity only.
                // No fake-intel/alternate/daily name substitution is allowed.
                eq(
                    report,
                    &format!("{label}.template saved standard wrapper"),
                    handle,
                    expected.template.unwrap_or(0),
                );
                if handle != 0 {
                    eq(
                        report,
                        &format!("{label}.template saved registry entry"),
                        saved.division_templates.contains_key(&handle),
                        true,
                    );
                }
            }
        }
        // The null standard wrapper is supported. It does not prove that
        // every cached stat getter is reached, so it makes no leaf demand.
        if packet.template.is_some_and(|handle| handle != 0) {
            for (name, value) in [
                ("stats.river[0]", packet.stats.river[0]),
                ("stats.river[1]", packet.stats.river[1]),
                ("stats.fort[0]", packet.stats.fort[0]),
                ("stats.fort[1]", packet.stats.fort[1]),
                ("stats.snow", packet.stats.snow),
                ("stats.night", packet.stats.night),
                ("acclimatization", packet.acclimatization),
            ] {
                required(report, &format!("{label}.{name}"), &value);
            }
            terrain(report, &format!("{label}.terrain"), &packet.terrain);
        }
        report.limitation("CArmy cached attack stat words and the fresh acclimatization factor are not independently compared with save text here; saved acclimatization type/value is a different operand, and numerical serializer-omission certification requires a matched numeric save comparison", &label);
    }
    eq(
        report,
        "DivisionStatus selected-attack saved identity census",
        seen,
        saved.divisions.keys().copied().collect(),
    );
    report.compared(
        "DivisionStatus.attack saved identities / packet checks",
        rows.len(),
    );
    report.limitation("matching a full saved template handle cannot certify the collector's registry-to-wrapper pointer authentication or a quiescent sample boundary; those require independent native acquisition evidence", "DivisionStatus.attack.template authentication");
    Ok(())
}

fn saved_groups(
    saved: &Save,
) -> io::Result<BTreeMap<u64, &installed_content::save::naval_history::OrdersGroup>> {
    let mut result = BTreeMap::new();
    for country in saved
        .naval_history
        .theatre_countries
        .iter()
        .flat_map(|countries| countries.values())
    {
        for theatre in &country.land {
            for group in &theatre.definitions {
                if result.insert(group.id, group).is_some() {
                    return Err(invalid("duplicate saved orders-group definition"));
                }
            }
        }
    }
    Ok(result)
}

fn group_packet(
    hour: i32,
    label: &str,
    packet: &OrdersGroupAttackInputs,
    saved_leader: Option<Option<u64>>,
    report: &mut Report,
) {
    if !fresh(report, label, packet.sample_hour, hour) {
        return;
    }
    required(report, &format!("{label}.commander"), &packet.commander);
    // Some(None) is genuine null. Outer None is selector/read refusal.
    if let Some(commander) = &packet.commander {
        if let Some(saved_leader) = saved_leader {
            eq(
                report,
                &format!("{label}.commander saved direct leader"),
                commander.as_ref().map(|leader| leader.leader),
                saved_leader,
            );
        }
        if let Some(commander) = commander {
            if commander.leader as u32 != engine::cunit_leader::REFERENCE_TYPE
                || commander.leader >> 32 == 0
            {
                report.difference(
                    format!("{label}.commander typed identity"),
                    commander.leader,
                    "nonzero full CUnitLeader reference",
                );
            }
            for (name, value) in [
                ("fort", commander.fort),
                ("river", commander.river),
                ("snow", commander.snow),
                ("night", commander.night),
            ] {
                required(report, &format!("{label}.commander.{name}"), &value);
            }
            terrain(
                report,
                &format!("{label}.commander.terrain"),
                &commander.terrain,
            );
        }
    }
}

fn groups(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let expected = saved_groups(saved)?;
    let rows = frame.complete::<tables::OrdersGroupCounterInputs>()?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.group == 0 || !seen.insert(row.group) {
            return Err(invalid("null or duplicate selected-attack group identity"));
        }
        let label = format!("OrdersGroupCounterInputs {}.attack", row.group);
        let Some(packet) = report.recorded(&row.attack, "OrdersGroupCounterInputs.attack", &label)
        else {
            continue;
        };
        group_packet(
            frame.hour,
            &label,
            packet,
            expected.get(&row.group).map(|group| group.leader),
            report,
        );
        if !expected.contains_key(&row.group) {
            report.limitation("a group outside serialized land-theatre definitions has no saved direct leader witness; no parent-group commander is substituted", &label);
        }
    }
    for group in expected.keys() {
        if !seen.contains(group) {
            report.difference(
                format!("OrdersGroupCounterInputs {group}.attack identity"),
                "absent",
                "saved group definition",
            );
        }
    }
    report.compared(
        "OrdersGroupCounterInputs.attack saved direct leaders / packet checks",
        rows.len(),
    );
    report.limitation("the saved direct leader reference certifies identity only; leader modifier and terrain caches need an independent native witness, not a recalculation from saved leader traits", "OrdersGroupCounterInputs.attack numeric operands");
    Ok(())
}

fn regions(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::WeatherRegions>()?;
    let saved_regions = saved
        .weather
        .regions
        .as_ref()
        .ok_or_else(|| invalid("saved weather regions roster is missing"))?;
    let mut expected = BTreeMap::new();
    for (ordinal, region) in saved_regions.iter().enumerate() {
        if usize::try_from(region.slot).ok() != Some(ordinal)
            || expected.insert(region.slot, region).is_some()
        {
            return Err(invalid(
                "saved weather region roster is not unique and dense in native order",
            ));
        }
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.id) {
            return Err(invalid("duplicate selected-attack weather region"));
        }
        let label = format!("WeatherRegions {}.attack", row.id);
        match expected.get(&row.id) {
            Some(expected) => eq(
                report,
                &format!("{label} saved region identity"),
                Some(row.id),
                expected.region,
            ),
            None => report.difference(format!("{label} saved region"), "present", "absent"),
        }
        let Some(packet) = report.recorded(&row.attack, "WeatherRegions.attack", &label) else {
            continue;
        };
        if !fresh(report, &label, packet.sample_hour, frame.hour) {
            continue;
        }
        required(report, &format!("{label}.attack"), &packet.attack);
        required(
            report,
            &format!("{label}.breakthrough"),
            &packet.breakthrough,
        );
    }
    eq(
        report,
        "WeatherRegions selected-attack saved identity census",
        seen,
        expected.keys().copied().collect(),
    );
    report.compared(
        "WeatherRegions.attack saved identities / packet checks",
        rows.len(),
    );
    report.limitation("effective weather attack modifiers cannot be certified by saved weather flags; no save numerical comparator is implemented, and its omission needs an independent matched save check", "WeatherRegions.attack effective modifiers");
    Ok(())
}

fn saved_fort_sum(
    province: u32,
    saved: &Save,
    buildings: &installed_content::buildings::Buildings,
) -> io::Result<i32> {
    let mut sum = 0_i32;
    for ((location, _), building) in saved
        .construction
        .province_buildings
        .range((province, Vec::new())..)
        .take_while(|((location, _), _)| *location == province)
    {
        debug_assert_eq!(*location, province);
        let token = std::str::from_utf8(&building.building_type)
            .map_err(|_| invalid("saved fort building token is not UTF-8"))?;
        let template = buildings.by_token(token)?.ok_or_else(|| {
            invalid(format!(
                "saved building type {token} has no verified content definition"
            ))
        })?;
        // PENDING producer/token/scan audit: proposed land_fort retains the
        // signed i32 exactly. No name-based fort classification occurs.
        sum = sum.wrapping_add(i32::from(building.healthy_level).wrapping_mul(template.land_fort));
    }
    Ok(sum)
}

fn fort_keyword(frame: &DecodedFrame) -> io::Result<()> {
    let dictionary = frame.complete::<tables::LexicalTokens>()?;
    let mut matches = dictionary.iter().filter(|entry| entry.token == 0x310A);
    let token = matches
        .next()
        .ok_or_else(|| invalid("fort comparison requires recorded native token310A spelling"))?;
    if matches.next().is_some() || !token.literal.eq_ignore_ascii_case(b"land_fort") {
        return Err(invalid(
            "recorded native token310A is ambiguous or is not land_fort; fort content mapping refused",
        ));
    }
    Ok(())
}

fn provinces(
    frame: &DecodedFrame,
    saved: &Save,
    buildings: &ContentRead<'_, installed_content::buildings::Buildings>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = frame.complete::<tables::Provinces>()?;
    // Authenticate the complete shared dictionary once, then keep each row's
    // report boundary independent. A failure here never skips prefix checks
    // or historical-field reporting and is reported only for demanded sums.
    let keyword = fort_keyword(frame).map_err(|error| error.to_string());
    for row in rows {
        let label = format!("Provinces {}.attack", row.id);
        let Some(packet) = report.recorded(&row.attack, "Provinces.attack", &label) else {
            continue;
        };
        if !fresh(report, &label, packet.sample_hour, frame.hour) {
            continue;
        }
        required(report, &format!("{label}.fort_sum"), &packet.fort_sum);
        if let Some(sum) = packet.fort_sum {
            eq(
                report,
                &format!("{label}.fort_positive packet consistency"),
                row.fort_positive,
                u8::from(sum > 0),
            );
            // One content refusal does not erase another province's saved
            // difference. Each province owns its independent failure group.
            report.attempt(
                &format!("Provinces selected attack saved fort {}", row.id),
                |r| {
                    // Static keyword-name certification is still held. A verified
                    // session's complete native dictionary must independently
                    // establish this exact keyword before content contributes a
                    // weight, including an empty building list's zero sum.
                    if let Err(reason) = &keyword {
                        return Err(invalid(reason.clone()));
                    }
                    let buildings =
                        super::required_context("verified building fort content", buildings)?;
                    eq(
                        r,
                        &format!("{label}.fort_sum saved healthy buildings"),
                        sum,
                        saved_fort_sum(row.id, saved, buildings)?,
                    );
                    Ok(())
                },
            );
            if sum > 0 && row.controller as i32 > 0 {
                required(
                    report,
                    &format!("{label}.land_bunker_effectiveness_factor"),
                    &packet.land_bunker_effectiveness_factor,
                );
            }
        }
    }
    report.compared(
        "Provinces.attack saved fort sums / packet checks",
        rows.len(),
    );
    report.limitation("state key272 is an effective controller-specific runtime accumulator; saved state modifier declarations do not certify its evaluated value or canonical/raw alias selection", "Provinces.attack.land_bunker_effectiveness_factor");
    Ok(())
}

#[derive(Default)]
struct CountryDemand {
    night: BTreeSet<u32>,
    fort: BTreeSet<u32>,
    zero_night: bool,
    zero_fort: bool,
}

fn demand_country(
    raw: u32,
    links: &BTreeMap<u32, u32>,
    countries: &BTreeMap<u32, u32>,
) -> io::Result<Option<u32>> {
    // Only literal zero names the separately authenticated actual owner0.
    // Unsupported signed-negative IDs do not acquire its identity or values.
    if raw == 0 {
        return Ok(None);
    }
    if (raw as i32) < 0 {
        return Err(invalid(format!(
            "attack owner/controller raw {} is a signed-negative unsupported reference",
            raw as i32
        )));
    }
    let slot = links.get(&raw).copied().ok_or_else(|| {
        invalid(format!(
            "attack owner/controller raw {raw} has no country link"
        ))
    })?;
    if slot == 0 {
        return Ok(None);
    }
    countries
        .get(&slot)
        .copied()
        .map(Some)
        .ok_or_else(|| invalid("attack country link has no canonical country"))
}

fn demand_country_observed(
    frame: &DecodedFrame,
    raw: u32,
    links: &BTreeMap<u32, u32>,
    countries: &BTreeMap<u32, u32>,
) -> io::Result<Option<u32>> {
    if (raw as i32) < 0 {
        return Err(invalid(format!(
            "attack owner/controller raw {} is a signed-negative unsupported reference",
            raw as i32
        )));
    }
    if raw != 0
        && let Some(slot) = links.get(&raw)
    {
        if *slot == 0 {
            return Err(invalid("normal CountryLinks row has reserved mapping"));
        }
        return demand_country(raw, links, countries);
    }
    let [row] = frame.complete::<tables::ReservedCountryNaming>()? else {
        return Err(invalid("raw mapping requires naming singleton"));
    };
    let packet = row
        .raw_country_observations
        .require("ReservedCountryNaming.raw_country_observations")
        .map_err(invalid)?
        .as_ref()
        .ok_or_else(|| invalid("raw mapping packet refused"))?;
    match packet
        .entry(frame.hour, raw as i32)
        .map_err(invalid)?
        .mapping
    {
        RawCountryMapping::Observed {
            canonical_slot: 0,
            country_id: 0,
        } => Ok(None),
        RawCountryMapping::Observed { .. } => Err(invalid(
            "omitted normal country link has no reserved certificate",
        )),
        RawCountryMapping::Refused => Err(invalid("raw country mapping refused")),
        RawCountryMapping::Unreached => Err(invalid("raw country mapping unreached")),
    }
}

fn country_demand(frame: &DecodedFrame) -> io::Result<CountryDemand> {
    let mut demand = CountryDemand::default();
    let mut links = BTreeMap::new();
    for link in frame.complete::<tables::CountryLinks>()? {
        if links.insert(link.id, link.canonical_slot).is_some() {
            return Err(invalid("ambiguous attack country link"));
        }
    }
    let mut countries = BTreeMap::new();
    for country in frame.complete::<tables::Countries>()? {
        if countries
            .insert(country.canonical_slot, country.id)
            .is_some()
        {
            return Err(invalid("ambiguous canonical attack country"));
        }
    }
    for division in frame.complete::<tables::Divisions>()? {
        let raw = if division.expeditionary_owner as i32 > 0 {
            division.expeditionary_owner
        } else {
            division.roster_owner
        };
        match demand_country_observed(frame, raw, &links, &countries)? {
            Some(country) => {
                demand.night.insert(country);
            }
            None => demand.zero_night = true,
        }
    }
    for province in frame.complete::<tables::Provinces>()? {
        if province.fort_positive == 0 || province.controller as i32 <= 0 {
            continue;
        }
        match demand_country_observed(frame, province.controller, &links, &countries)? {
            Some(country) => {
                demand.fort.insert(country);
            }
            None => demand.zero_fort = true,
        }
    }
    Ok(demand)
}

fn raw_observations(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let [row] = frame.complete::<tables::ReservedCountryNaming>()? else {
        return Err(invalid("raw observations require naming singleton"));
    };
    let Some(packet) = report.recorded(
        &row.raw_country_observations,
        "ReservedCountryNaming.raw_country_observations",
        "exact signed raw country observations",
    ) else {
        return Ok(());
    };
    let Some(packet) = packet else {
        report.difference(
            "ReservedCountryNaming.raw_country_observations",
            "capture refused",
            "complete sparse packet",
        );
        return Ok(());
    };
    packet.check()?;
    if !fresh(
        report,
        "ReservedCountryNaming.raw_country_observations",
        packet.sample_hour,
        frame.hour,
    ) {
        return Ok(());
    }
    let links = frame.complete::<tables::CountryLinks>()?;
    let countries = frame.complete::<tables::Countries>()?;
    for entry in &packet.entries {
        if let RawCountryMapping::Observed {
            canonical_slot,
            country_id,
        } = entry.mapping
        {
            if canonical_slot != 0 {
                let normal = links.iter().find(|link| link.id == entry.raw as u32);
                if normal.map(|link| link.canonical_slot) != Some(canonical_slot)
                    || !countries.iter().any(|country| {
                        country.canonical_slot == canonical_slot && country.id == country_id
                    })
                {
                    report.difference(
                        format!("raw mapping {} identity", entry.raw),
                        (canonical_slot, country_id),
                        "same-hour normal country/link identity",
                    );
                }
            } else if links.iter().any(|link| link.id == entry.raw as u32) {
                report.difference(
                    format!("raw mapping {} zero alias", entry.raw),
                    "reserved certificate",
                    "normal CountryLinks priority",
                );
            }
        }
    }
    let daily = frame.complete::<tables::DivisionTemplates>().ok();
    for icon in frame.complete::<tables::DivisionTemplateIcons>()? {
        let dictionary =
            daily.and_then(|rows| rows.iter().find(|row| row.template == icon.template));
        if let Ok(parts) = icon.display_name_inputs(frame.hour, dictionary)
            && parts.literal.is_empty()
            && parts.fallback_key.is_some_and(|key| !key.is_empty())
        {
            match packet.entry(frame.hour, icon.country) {
                Ok(RawCountryObservation {
                    tag: RawCountryTag::Bytes(_),
                    ..
                }) => {}
                _ => report.difference(
                    format!("template {} exact raw tag {}", icon.template, icon.country),
                    "unavailable",
                    "reached fallback tag bytes",
                ),
            }
        }
    }
    report.compared(
        "raw country observations packet checks",
        packet.entries.len(),
    );
    report.limitation("signed raw tag spellings, omitted raw-to-reserved mappings and native selecting/attempt controls are unsaved runtime witnesses; packet checks establish consistency only and require a matched native witness", "ReservedCountryNaming.raw_country_observations");
    Ok(())
}

fn country_packet(
    hour: i32,
    label: &str,
    packet: &CountryAttackInputs,
    night_demanded: bool,
    fort_demanded: bool,
    report: &mut Report,
) {
    if !fresh(report, label, packet.sample_hour, hour) {
        return;
    }
    if night_demanded {
        required(
            report,
            &format!("{label}.land_night_attack"),
            &packet.land_night_attack,
        );
    }
    if fort_demanded {
        required(
            report,
            &format!("{label}.land_bunker_effectiveness_factor"),
            &packet.land_bunker_effectiveness_factor,
        );
    }
}

fn countries(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::CountryCounterInputs>()?;
    let [reserved] = frame.complete::<tables::ReservedCountryNaming>()? else {
        return Err(invalid(
            "reserved attack owner requires the existing naming singleton",
        ));
    };
    // Historical suffix absence is reported by recorded below. It cannot
    // demand a newly introduced raw certificate to compare no current input.
    let current = rows
        .iter()
        .any(|row| matches!(&row.attack, Recorded::Value(_)))
        || matches!(&reserved.attack, Recorded::Value(_));
    let demand = if current {
        country_demand(frame)?
    } else {
        CountryDemand::default()
    };
    let mut seen = BTreeSet::new();
    for row in rows {
        if !seen.insert(row.country) {
            return Err(invalid("duplicate selected-attack country"));
        }
        let label = format!("CountryCounterInputs {}.attack", row.country);
        let Some(packet) = report.recorded(&row.attack, "CountryCounterInputs.attack", &label)
        else {
            continue;
        };
        country_packet(
            frame.hour,
            &label,
            packet,
            demand.night.contains(&row.country),
            demand.fort.contains(&row.country),
            report,
        );
    }
    for country in demand.night.union(&demand.fort) {
        if !seen.contains(country) {
            report.difference(
                format!("CountryCounterInputs {country}.attack identity"),
                "absent",
                "country demanded by actual division owner or positive fort controller",
            );
        }
    }
    if let Some(packet) = report.recorded(
        &reserved.attack,
        "ReservedCountryNaming.attack",
        "actual owner0 attack",
    ) {
        match packet {
            Some(packet) => country_packet(
                frame.hour,
                "ReservedCountryNaming.attack",
                packet,
                demand.zero_night,
                demand.zero_fort,
                report,
            ),
            None if demand.zero_night || demand.zero_fort => report.difference(
                "ReservedCountryNaming.attack",
                "capture refused",
                "demanded actual owner0 packet",
            ),
            None => {} // Genuinely outside demand; no invented numerical zero.
        }
    }
    report.compared(
        "CountryCounterInputs.attack runtime packet checks",
        rows.len(),
    );
    report.limitation("country effective key103/key272 are runtime getter results; saved raw country modifiers, aliases and defines do not independently certify their evaluated values. Demand reconstructed from replay owner/controller operands is a consistency check, not native admission proof", "CountryCounterInputs.attack / ReservedCountryNaming.attack");
    Ok(())
}

fn scalars(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let [row] = frame.complete::<tables::SimulationFeatures>()? else {
        return Err(invalid(
            "political runtime scalars require the existing SimulationFeatures singleton",
        ));
    };
    let Some(packet) = report.recorded(
        &row.runtime_scalars,
        "SimulationFeatures.runtime_scalars",
        "political runtime scalars",
    ) else {
        return Ok(());
    };
    if fresh(
        report,
        "SimulationFeatures.runtime_scalars",
        packet.sample_hour,
        frame.hour,
    ) {
        for (name, value) in [
            ("river_small", packet.river_small),
            ("river_large", packet.river_large),
            ("night", packet.night),
            ("fort", packet.fort),
            ("move_strength_threshold", packet.move_strength_threshold),
            (
                "move_organisation_threshold",
                packet.move_organisation_threshold,
            ),
        ] {
            required(
                report,
                &format!("SimulationFeatures.runtime_scalars.{name}"),
                &value,
            );
        }
    }
    report.compared(
        "SimulationFeatures.runtime_scalars runtime packet checks",
        1,
    );
    report.limitation("the six freshly held native qwords have no numeric save comparator in this proposal. Authenticated runtime reads identify four NMilitary globals; the two runtime constants and any initialization/load writers need separate native evidence. Offline content numbers are not substitutes for current native memory, and a replay round-trip is not independent acceptance", "SimulationFeatures.runtime_scalars");
    Ok(())
}

#[cfg(test)]
mod tests;
