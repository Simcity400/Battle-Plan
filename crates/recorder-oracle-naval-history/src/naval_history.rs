
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::naval_history as saved;
use replay_data::{
    rows::CountryRow, rows::NavalSupremacyRow, rows::SunkConvoyRow, rows::SunkShipEntry,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;

use super::{Report, complete, country_tag, invalid, serialized_text};

mod railways;
mod theatres;

use railways::compare_railways;
pub use railways::{compare_rail_edges, compare_railway_buildings};
use theatres::compare_theatres;
pub use theatres::{compare_order_of_battle, omitted_regular_group_units};

fn equal<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    field: impl AsRef<str>,
    actual: T,
    expected: T,
) {
    if actual != expected {
        report.difference(field, actual, expected);
    }
}

fn tag<'a>(reference: u32, links: &BTreeMap<u32, &'a CountryRow>) -> io::Result<&'a [u8]> {
    Ok(country_tag(reference, links)?.unwrap_or(b"---"))
}

fn flag(value: u8) -> io::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid(
            "naval history raw flag exceeds the save's 0/1 representation",
        )),
    }
}

pub fn compare(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut passed = report.attempt("naval NavalAccidentReports", |report| {
        crate::naval_accidents::compare(frame, saved, links, report)
    });
    passed &= report.attempt("naval SunkConvoys", |report| {
        compare_convoys(frame, saved, links, report)
    });
    passed &= report.attempt("naval SunkShips", |report| {
        compare_ships(frame, saved, links, tokens, report)
    });
    passed &= report.attempt("naval NavalResults", |report| {
        compare_results(frame, saved, report)
    });
    passed &= report.attempt("naval NavalResultMembers", |report| {
        compare_members(frame, saved, report)
    });
    passed &= report.attempt("naval NavalResultCountries", |report| {
        compare_countries(frame, saved, links, report)
    });
    passed &= report.attempt("naval NavalSupremacy", |report| {
        compare_supremacy(frame, saved, links, report)
    });
    passed &= report.attempt("naval RailCooldowns", |report| {
        compare_railways(frame, saved, report)
    });
    passed &= compare_theatres(frame, saved, links, report).is_ok();
    report.limitation(
        "serialized country tags cannot distinguish equivalent raw country-reference aliases",
        "SunkConvoys, SunkShips, NavalResultCountries, NavalSupremacy, LandTheatres, AirTheatres",
    );
    if passed {
        Ok(())
    } else {
        Err(invalid(
            "one or more naval table comparisons failed; see individual comparison diagnostics",
        ))
    }
}

fn compare_convoys(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut rows = BTreeMap::new();
    let mut keys = BTreeSet::new();
    for row in complete::<tables::SunkConvoys>(frame)? {
        if !keys.insert((row.month, row.killer_country, row.owner_country))
            || rows.insert(row.ordinal, row).is_some()
        {
            return Err(invalid(
                "duplicate sunk convoy native key or vector position",
            ));
        }
    }
    let mut compared = 0;
    for (ordinal, expected) in saved.sunk_convoys.iter().enumerate() {
        let ordinal = u32::try_from(ordinal).map_err(|_| invalid("convoy ordinal exceeds u32"))?;
        let label = format!("sunk convoy position {ordinal}");
        let Some(actual) = rows.remove(&ordinal) else {
            report.difference(label, "absent", "present");
            continue;
        };
        compared += 1;
        compare_convoy(actual, expected, &label, links, report)?;
    }
    for ordinal in rows.keys() {
        report.difference(
            format!("sunk convoy position {ordinal}"),
            "present",
            "absent",
        );
    }
    report.compared("SunkConvoys", compared);
    Ok(())
}

fn compare_convoy(
    actual: &SunkConvoyRow,
    expected: &saved::SunkConvoy,
    label: &str,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    equal(
        report,
        format!("{label}.month"),
        actual.month,
        expected.month,
    );
    equal(
        report,
        format!("{label}.convoys"),
        actual.convoys,
        expected.convoys,
    );
    equal(
        report,
        format!("{label}.killer_country"),
        tag(actual.killer_country, links)?,
        expected.killer_country.as_slice(),
    );
    equal(
        report,
        format!("{label}.owner"),
        tag(actual.owner_country, links)?,
        expected.owner_country.as_slice(),
    );
    Ok(())
}

fn compare_ships(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let [row] = complete::<tables::SunkShips>(frame)? else {
        return Err(invalid(
            "sunk ship history must contain its one native collection row",
        ));
    };
    equal(
        report,
        "sunk ship history length",
        row.entries.len(),
        saved.sunk_ships.len(),
    );
    for (ordinal, (actual, expected)) in row.entries.iter().zip(&saved.sunk_ships).enumerate() {
        compare_ship(
            actual,
            expected,
            &format!("sunk ship position {ordinal}"),
            links,
            tokens,
            report,
        )?;
    }
    report.compared("SunkShips", row.entries.len().min(saved.sunk_ships.len()));
    report.limitation(
        "class definition name bytes are not serialized by the ship writer; the independently recorded lexical dictionary verifies the class token spelling",
        "SunkShips.class_name / attribution_class_name",
    );
    Ok(())
}

pub fn compare_ship(
    actual: &SunkShipEntry,
    expected: &saved::SunkShip,
    label: &str,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    for (field, actual, expected) in [
        ("name", actual.ship_name.as_slice(), &expected.ship_name),
        (
            "killer_name",
            actual.attribution_name.as_slice(),
            &expected.attribution_name,
        ),
    ] {
        if let Some(expected) = expected {
            let field = format!("{label}.{field}");
            let actual = serialized_text(report, &field, actual);
            equal(report, field, actual, expected.as_slice());
        } else {
            report.limitation("the ship writer can omit name bytes under its writer mode; absence does not prove an empty native string", format!("{label}.{field}"));
        }
    }
    equal(
        report,
        format!("{label}.country"),
        tag(actual.victim_country, links)?,
        expected.victim_country.as_slice(),
    );
    equal(
        report,
        format!("{label}.killer_country"),
        tag(actual.attacker_country, links)?,
        expected.attacker_country.as_slice(),
    );
    equal(
        report,
        format!("{label}.date"),
        actual.sink_hour,
        expected.sink_date.engine_hour()?,
    );
    equal(
        report,
        format!("{label}.level"),
        actual.model_level,
        expected.model_level,
    );
    for (field, token, expected) in [
        ("definition", actual.class_token, expected.class.as_slice()),
        (
            "killer_definition",
            actual.attribution_class_token,
            expected.attribution_class.as_slice(),
        ),
    ] {
        if field == "killer_definition" && !flag(actual.attribution_class_present)? {
            report.difference(
                format!("{label}.{field}"),
                "null pointer",
                "serialized class token",
            );
            continue;
        }
        if tokens.is_empty() {
            report.limitation("recorded lexical tokens are unavailable; sunk ship class token spellings cannot be compared", format!("{label}.{field}"));
            continue;
        }
        let literal = tokens.get(&token).copied().ok_or_else(|| {
            invalid(format!(
                "sunk ship class token {token} lacks its recorded lexical entry"
            ))
        })?;
        if token == engine_native_memory::cpdx_lexer::FIXED15_TOKEN {
            return Err(invalid(
                "sunk ship class uses the fixed15 formatter token, not a directly serialized lexical spelling",
            ));
        }
        let field = format!("{label}.{field}");
        if literal.contains(&0) {
            report.limitation("embedded-NUL lexical class literal has unproved text-stream termination; its saved spelling is not compared", field);
            continue;
        }
        equal(report, field, literal, expected);
    }
    for (field, actual, expected) in [
        (
            "equipment_variant",
            actual.victim_equipment_variant,
            expected.victim_equipment_variant,
        ),
        (
            "air_wing",
            actual.damage_source_reference,
            expected.damage_source_reference,
        ),
    ] {
        if let Some(expected) = expected {
            equal(report, format!("{label}.{field}"), actual, expected);
        } else {
            report.limitation("the ship writer omits null or unresolved equipment/source references; absence does not verify their raw native words", format!("{label}.{field}"));
        }
    }
    let province = flag(actual.province_present)?.then_some(actual.province);
    if province.is_none() && actual.province != 0 {
        return Err(invalid(
            "absent sunk ship province contains nonzero derived payload",
        ));
    }
    equal(
        report,
        format!("{label}.location"),
        province,
        expected.province,
    );
    equal(
        report,
        format!("{label}.battle"),
        actual.battle,
        expected.battle,
    );
    equal(
        report,
        format!("{label}.convoy"),
        flag(actual.cause_record)?,
        expected.cause_record,
    );
    equal(
        report,
        format!("{label}.assist"),
        flag(actual.assisted_kill)?,
        expected.assisted_kill,
    );
    Ok(())
}

fn compare_results(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let mut rows = BTreeMap::new();
    let mut buckets: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for row in complete::<tables::NavalResults>(frame)? {
        if row.id == 0 || rows.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate retained naval result id"));
        }
        buckets
            .entry(row.province)
            .or_default()
            .push(row.province_ordinal);
    }
    for ordinals in buckets.values_mut() {
        ordinals.sort_unstable();
        if ordinals
            .iter()
            .enumerate()
            .any(|(index, &ordinal)| ordinal as usize != index)
        {
            return Err(invalid(
                "retained naval result province ordinals are not dense",
            ));
        }
    }
    let mut compared = 0;
    for (&id, expected) in &saved.results {
        let label = format!("naval result {id}");
        let Some(actual) = rows.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        compared += 1;
        equal(
            report,
            format!("{label}.location"),
            actual.province,
            expected.province,
        );
        equal(
            report,
            format!("{label}.date"),
            actual.result_hour,
            expected.result_date.engine_hour()?,
        );
        equal(
            report,
            format!("{label}.to_discard_date"),
            actual.retire_hour,
            expected.retire_date.engine_hour()?,
        );
        equal(
            report,
            format!("{label}.importance"),
            actual.category,
            expected.category,
        );
        equal(
            report,
            format!("{label}.port_strike"),
            flag(actual.flag_a)?,
            expected.port_strike,
        );
        equal(
            report,
            format!("{label}.naval_strike"),
            flag(actual.flag_b)?,
            expected.naval_strike,
        );
    }
    for id in rows.keys() {
        report.difference(format!("naval result {id}"), "present", "absent");
    }
    report.compared("NavalResults", compared);
    report.limitation("the naval-result writer does not serialize winner/loser side selectors, subtype or readiness; captured raw fields are not verified by inferred battle outcomes", "NavalResults");
    report.limitation(
        "the province bucket order is not serialized; only its density is checked",
        "NavalResults.province_ordinal",
    );
    Ok(())
}

fn compare_members(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let mut rows = BTreeMap::new();
    let mut positions = BTreeSet::new();
    for row in complete::<tables::NavalResultMembers>(frame)? {
        if row.result == 0
            || row.member == 0
            || row.side > 1
            || rows.insert((row.result, row.member), row).is_some()
            || !positions.insert((row.result, row.side, row.ordinal))
        {
            return Err(invalid(
                "invalid naval result member identity, side or position",
            ));
        }
    }
    let mut saved_keys = BTreeSet::new();
    let mut compared = 0;
    for (&id, result) in &saved.results {
        for (side, entries) in result.sides.iter().enumerate() {
            for (ordinal, expected) in entries.members.iter().enumerate() {
                let member = expected.id;
                if !saved_keys.insert((id, member)) {
                    return Err(invalid("duplicate saved naval result member handle"));
                }
                let label = format!("naval result {id} member {member}");
                let Some(actual) = rows.remove(&(id, member)) else {
                    report.difference(label, "absent", "present");
                    continue;
                };
                compared += 1;
                equal(
                    report,
                    format!("{label}.side"),
                    usize::from(actual.side),
                    side,
                );
                equal(
                    report,
                    format!("{label}.position"),
                    actual.ordinal as usize,
                    ordinal,
                );
                if let Some(sunk_by) = &expected.sunk_by {
                    equal(
                        report,
                        format!("{label}.cached_info.sunk_by byte length"),
                        actual.presence_value,
                        sunk_by.len() as u64,
                    );
                } else {
                    report.limitation("the cached-ship writer omits cached_info.sunk_by when empty or suppressed by writer mode; absence does not verify the raw snapshot string length", label);
                }
            }
        }
    }
    for (result, member) in rows.keys() {
        report.difference(
            format!("naval result {result} member {member}"),
            "present",
            "absent",
        );
    }
    report.compared("NavalResultMembers", compared);
    Ok(())
}

fn compare_countries(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut rows = BTreeMap::new();
    let mut keys = BTreeSet::new();
    for row in complete::<tables::NavalResultCountries>(frame)? {
        if row.result == 0
            || row.list > 2
            || rows
                .insert((row.result, row.list, row.ordinal), row)
                .is_some()
            || !keys.insert((row.result, row.list, row.country))
        {
            return Err(invalid(
                "invalid naval result country identity, list or position",
            ));
        }
    }
    let mut compared = 0;
    for (&id, result) in &saved.results {
        for (list, entries) in [
            &result.sides[0].countries,
            &result.sides[1].countries,
            &result.admitted,
        ]
        .into_iter()
        .enumerate()
        {
            for (ordinal, expected) in entries.iter().enumerate() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| invalid("result country ordinal exceeds u32"))?;
                let label = format!("naval result {id} country list {list} position {ordinal}");
                let Some(actual) = rows.remove(&(id, list as u8, ordinal)) else {
                    report.difference(label, "absent", "present");
                    continue;
                };
                compared += 1;
                equal(
                    report,
                    label,
                    tag(actual.country, links)?,
                    expected.as_slice(),
                );
            }
        }
    }
    for key in rows.keys() {
        report.difference(
            format!(
                "naval result {} country list {} position {}",
                key.0, key.1, key.2
            ),
            "present",
            "absent",
        );
    }
    report.compared("NavalResultCountries", compared);
    Ok(())
}

fn supremacy_values(row: &NavalSupremacyRow) -> [i64; 6] {
    [
        row.current,
        row.base_target,
        row.target,
        row.individual_ratio,
        row.previous,
        row.decline_from,
    ]
}

fn compare_supremacy(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let regions = saved
        .supremacy_regions
        .as_ref()
        .ok_or_else(|| invalid("save lacks the naval supremacy region roster"))?;
    let mut actual = BTreeMap::<(u32, Vec<u8>), Vec<[i64; 6]>>::new();
    let mut native_keys = BTreeSet::new();
    let mut region_slots = BTreeMap::new();
    let mut slot_regions = BTreeMap::new();
    for row in complete::<tables::NavalSupremacy>(frame)? {
        if row.region_slot == 0
            || !native_keys.insert((row.region, row.country))
            || region_slots
                .insert(row.region, row.region_slot)
                .is_some_and(|slot| slot != row.region_slot)
            || slot_regions
                .insert(row.region_slot, row.region)
                .is_some_and(|region| region != row.region)
        {
            return Err(invalid(
                "naval supremacy has duplicate native keys or inconsistent authored region/runtime slot identity",
            ));
        }
        actual
            .entry((row.region_slot, tag(row.country, links)?.to_vec()))
            .or_default()
            .push(supremacy_values(row));
    }
    let mut expected = BTreeMap::<(u32, Vec<u8>), Vec<[i64; 6]>>::new();
    for (&slot, region) in regions {
        let Some(values) = &region.values else {
            report.limitation("save region slot omits dominance; it may be an inactive native slot and proves no scalar values", format!("region slot {slot}"));
            continue;
        };
        for row in values {
            expected
                .entry((slot, row.country.clone()))
                .or_default()
                .push([
                    row.current.raw,
                    row.base_target.raw,
                    row.target.raw,
                    row.individual_ratio.raw,
                    row.previous.raw,
                    row.decline_from.raw,
                ]);
        }
    }
    let mut compared = 0;
    for (key, mut expected) in expected {
        let label = format!(
            "naval supremacy region slot {} country {}",
            key.0,
            String::from_utf8_lossy(&key.1)
        );
        let Some(mut actual) = actual.remove(&key) else {
            report.difference(
                label,
                "absent",
                format!("{} saved value rows", expected.len()),
            );
            continue;
        };
        compared += actual.len().min(expected.len());
        if actual.len() == 1 && expected.len() == 1 {
            for ((field, actual), expected) in [
                "current",
                "base_target",
                "target",
                "individual_ratio",
                "previous",
                "decline_from",
            ]
            .into_iter()
            .zip(actual[0])
            .zip(expected[0])
            {
                equal(report, format!("{label}.{field}"), actual, expected);
            }
        } else {
            actual.sort_unstable();
            expected.sort_unstable();
            equal(report, &label, actual, expected);
            report.limitation("equal saved country tags cannot identify individual raw country aliases; all six-value tuples and their multiplicities are compared", label);
        }
    }
    for (key, rows) in actual {
        report.difference(
            format!(
                "naval supremacy region slot {} country {}",
                key.0,
                String::from_utf8_lossy(&key.1)
            ),
            format!("{} recorded value rows", rows.len()),
            "absent",
        );
    }
    report.compared("NavalSupremacy", compared);
    report.limitation("the save serializes runtime region slots; authored region IDs are retained in the recording but not independently serialized here", "NavalSupremacy.region");
    Ok(())
}

#[cfg(test)]
mod tests;
