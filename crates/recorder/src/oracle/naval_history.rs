//! Exact retained naval state comparisons, including native list order.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::save::{Division, naval_history as saved};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, NavalSupremacyRow, SunkConvoyRow, SunkShipEntry, tables},
};

use super::{Report, complete, country_tag, invalid};

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

fn serialized_text<'a>(report: &mut Report, field: &str, bytes: &'a [u8]) -> &'a [u8] {
    let end = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    if end < bytes.len() {
        report.limitation("native text serialization ends at the first NUL; captured trailing bytes are not save-comparable", field);
    }
    &bytes[..end]
}

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut passed = report.attempt("naval SunkConvoys", |report| {
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
    // This helper records six independent attempts before returning its error.
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
        "class definition name bytes are not serialized by C1F090; the independently recorded lexical dictionary verifies the class token spelling",
        "SunkShips.class_name / attribution_class_name",
    );
    Ok(())
}

fn compare_ship(
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
            // C1F090 names use ADF40 -> AF5A0, which consumes c_str.
            let actual = serialized_text(report, &field, actual);
            equal(report, field, actual, expected.as_slice());
        } else {
            report.limitation("C1F090 can omit name bytes under its writer mode; absence does not prove an empty native string", format!("{label}.{field}"));
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
        if token == 0x167 {
            return Err(invalid(
                "sunk ship class uses the fixed15 formatter token, not a directly serialized lexical spelling",
            ));
        }
        // C1F090 writes both class tokens unconditionally through ADCE0.
        // AAC50 passes the lexical c_str to stream+30, but the concrete sink's
        // termination behavior is not proved by the retained formatter alone.
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
            report.limitation("C1F090 omits null or unresolved equipment/source references; absence does not verify their raw native words", format!("{label}.{field}"));
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
    for row in complete::<tables::NavalResults>(frame)? {
        if row.id == 0 || rows.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate retained naval result id"));
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
    report.limitation("CD73D0 does not serialize winner/loser side selectors, subtype or readiness; captured raw fields are not verified by inferred battle outcomes", "NavalResults");
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
                    report.limitation("1962D80 omits cached_info.sunk_by when empty or suppressed by writer mode; absence does not verify the raw snapshot string length", label);
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

fn compare_railways(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let expected = saved
        .railways
        .as_ref()
        .ok_or_else(|| invalid("save lacks railway manager"))?;
    let mut rows = BTreeMap::new();
    for row in complete::<tables::RailCooldowns>(frame)? {
        if rows.insert(row.province, row).is_some() {
            return Err(invalid("duplicate recorded railway province"));
        }
    }
    let mut compared = 0;
    for (&province, expected) in expected {
        let label = format!("railway province {province}");
        let Some(actual) = rows.remove(&province) else {
            report.difference(label, "absent", "present");
            continue;
        };
        compared += 1;
        if let Some(cooldown) = expected.cooldown {
            equal(
                report,
                format!("{label}.cooldown"),
                actual.cooldown_ticks,
                cooldown,
            );
        } else if actual.cooldown_ticks > 0 {
            report.difference(
                format!("{label}.cooldown"),
                actual.cooldown_ticks,
                "omitted signed-nonpositive value",
            );
        } else {
            report.limitation("E80C40 omits signed-nonpositive cooldown; absence verifies that predicate but not the exact native integer", label);
        }
    }
    for province in rows.keys() {
        report.difference(format!("railway province {province}"), "present", "absent");
    }
    report.compared("RailCooldowns", compared);
    Ok(())
}

pub(super) fn compare_rail_edges(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    adjacency: &saved::RailwayAdjacency,
    report: &mut Report,
) -> io::Result<()> {
    let expected = saved
        .railways
        .as_ref()
        .ok_or_else(|| invalid("save lacks railway manager"))?;
    let mut rows = BTreeMap::new();
    for row in complete::<tables::RailEdges>(frame)? {
        if rows.insert((row.from, row.to), row.level).is_some() {
            return Err(invalid("duplicate recorded directed railway edge"));
        }
    }
    let mut compared = 0;
    let mut nonpositive = 0;
    for (&source, info) in expected {
        let neighbors = adjacency.neighbors(source).ok_or_else(|| {
            invalid(format!(
                "saved railway province {source} lacks verified content adjacency"
            ))
        })?;
        if neighbors.len() != info.levels.len() {
            return Err(invalid(format!(
                "railway province {source} has {} saved levels but {} verified content neighbors",
                info.levels.len(),
                neighbors.len()
            )));
        }
        for (&target, &level) in neighbors.iter().zip(&info.levels) {
            let label = format!("railway edge {source}->{target}");
            if let Some(actual) = rows.remove(&(source, target)) {
                equal(report, format!("{label}.level"), actual, level);
                compared += 1;
            } else if level > 0 {
                // E80070 reconstructs every positive entry in the sorted pairs.
                report.difference(label, "absent", level);
            }
            if level <= 0 {
                nonpositive += 1;
            }
        }
    }
    for ((source, target), level) in rows {
        report.difference(
            format!("railway edge {source}->{target}"),
            level,
            "absent from saved railway adjacency",
        );
    }
    report.compared("RailEdges", compared);
    if nonpositive != 0 {
        // E80070 loads only positive pairs, while E806E0 can insert a pair
        // after a signed max/clamp produces zero. The serialized indexed array
        // therefore does not encode the presence of every nonpositive pair.
        report.limitation("saved nonpositive railway levels do not uniquely encode sorted-pair presence; present recorded pairs compare their exact signed level, and every positive saved edge is required", format!("RailEdges: {nonpositive} nonpositive saved neighbor positions"));
    }
    Ok(())
}

fn name(report: &mut Report, label: String, actual: &[u8], expected: Option<&[u8]>) {
    if let Some(expected) = expected {
        equal(report, label, actual, expected);
    } else {
        report.limitation("native theatre serializer can suppress name bytes by writer mode; omission does not establish an empty string", label);
    }
}

fn compare_theatres(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut passed = report.attempt("naval LandTheatres", |report| {
        let countries = theatre_countries(saved)?;
        compare_theatre_table(
            "LandTheatres",
            complete::<tables::LandTheatres>(frame)?,
            |row| row.id,
            countries.iter().flat_map(|(country, saved)| {
                saved.land.iter().enumerate().map(move |(ordinal, row)| {
                    Ok((row.id, (country.as_slice(), ordinal, row)))
                })
            }),
            report,
            |actual, (country, ordinal, expected), report| {
                let label = format!("land theatre {}", expected.id);
                equal(report, format!("{label}.country roster"), tag(actual.owner, links)?, country);
                equal(report, format!("{label}.ordinal"), actual.ordinal as usize, ordinal);
                if let Some(expected) = &expected.volunteers_country {
                    if actual.volunteers_theatre <= 0 {
                        report.difference(format!("{label}.volunteers_theatre"), actual.volunteers_theatre, expected);
                    } else {
                        equal(report, format!("{label}.volunteers_theatre"), tag(actual.volunteers_theatre as u32, links)?, expected.as_slice());
                    }
                } else if actual.volunteers_theatre > 0 {
                    report.difference(format!("{label}.volunteers_theatre"), actual.volunteers_theatre, "omitted signed-nonpositive reference");
                } else {
                    report.limitation("EED430 omits signed-nonpositive volunteers_theatre references; omission does not distinguish their raw native bits", label);
                }
                Ok(())
            },
        )
    });
    passed &= report.attempt("naval LandTheatreGroups", |report| {
        let countries = theatre_countries(saved)?;
        compare_theatre_table(
            "LandTheatreGroups",
            complete::<tables::LandTheatreGroups>(frame)?,
            |row| row.id,
            countries
                .values()
                .flat_map(|country| &country.land)
                .flat_map(|theatre| {
                    theatre
                        .groups
                        .iter()
                        .enumerate()
                        .map(move |(ordinal, group)| Ok((group.id, (theatre.id, ordinal, group))))
                }),
            report,
            |actual, (theatre, ordinal, expected), report| {
                let label = format!("land theatre group {}", expected.id);
                equal(report, format!("{label}.theatre"), actual.theatre, theatre);
                equal(
                    report,
                    format!("{label}.ordinal"),
                    actual.ordinal as usize,
                    ordinal,
                );
                equal(
                    report,
                    format!("{label}.priority"),
                    actual.priority,
                    expected.priority,
                );
                name(
                    report,
                    format!("{label}.name"),
                    &actual.name,
                    expected.name.as_deref(),
                );
                Ok(())
            },
        )
    });
    passed &= report.attempt("naval LandTheatreMembers", |report| {
        let countries = theatre_countries(saved)?;
        compare_theatre_table(
            "LandTheatreMembers",
            complete::<tables::LandTheatreMembers>(frame)?,
            |row| (row.theater_group, row.orders_group),
            countries
                .values()
                .flat_map(|country| &country.land)
                .flat_map(|theatre| &theatre.groups)
                .flat_map(|group| {
                    group
                        .members
                        .iter()
                        .enumerate()
                        .map(move |(ordinal, &member)| Ok(((group.id, member), ordinal)))
                }),
            report,
            |actual, ordinal, report| {
                equal(
                    report,
                    format!(
                        "land theatre group {} member {}.ordinal",
                        actual.theater_group, actual.orders_group
                    ),
                    actual.ordinal as usize,
                    ordinal,
                );
                Ok(())
            },
        )
    });
    passed &= report.attempt("naval AirTheatres", |report| {
        let countries = theatre_countries(saved)?;
        compare_theatre_table(
            "AirTheatres",
            complete::<tables::AirTheatres>(frame)?,
            |row| row.id,
            countries.iter().flat_map(|(country, saved)| {
                saved.air.iter().enumerate().map(move |(ordinal, row)| {
                    if row.country != *country {
                        return Err(invalid(
                            "saved air theatre country differs from enclosing roster",
                        ));
                    }
                    Ok((row.id, (ordinal, row)))
                })
            }),
            report,
            |actual, (ordinal, expected), report| {
                let label = format!("air theatre {}", expected.id);
                equal(
                    report,
                    format!("{label}.country"),
                    tag(actual.owner, links)?,
                    expected.country.as_slice(),
                );
                equal(
                    report,
                    format!("{label}.ordinal"),
                    actual.ordinal as usize,
                    ordinal,
                );
                name(
                    report,
                    format!("{label}.name"),
                    &actual.name,
                    expected.name.as_deref(),
                );
                Ok(())
            },
        )
    });
    passed &= report.attempt("naval AirTheatreGroups", |report| {
        let countries = theatre_countries(saved)?;
        compare_theatre_table(
            "AirTheatreGroups",
            complete::<tables::AirTheatreGroups>(frame)?,
            |row| row.id,
            countries
                .values()
                .flat_map(|country| &country.air)
                .flat_map(|theatre| {
                    theatre
                        .groups
                        .iter()
                        .enumerate()
                        .map(move |(ordinal, group)| {
                            if group.theatre != theatre.id {
                                return Err(invalid(
                                    "saved air group parent differs from enclosing theatre",
                                ));
                            }
                            Ok((group.id, (ordinal, group)))
                        })
                }),
            report,
            |actual, (ordinal, expected), report| {
                let label = format!("air theatre group {}", expected.id);
                equal(
                    report,
                    format!("{label}.theatre"),
                    actual.theatre,
                    expected.theatre,
                );
                equal(
                    report,
                    format!("{label}.ordinal"),
                    actual.ordinal as usize,
                    ordinal,
                );
                equal(
                    report,
                    format!("{label}.name"),
                    actual.name.as_slice(),
                    expected.name.as_slice(),
                );
                Ok(())
            },
        )
    });
    passed &= report.attempt("naval AirTheatreMembers", |report| {
        let countries = theatre_countries(saved)?;
        compare_theatre_table(
            "AirTheatreMembers",
            complete::<tables::AirTheatreMembers>(frame)?,
            |row| (row.group, row.wing),
            countries
                .values()
                .flat_map(|country| &country.air)
                .flat_map(|theatre| &theatre.groups)
                .flat_map(|group| {
                    group
                        .wings
                        .iter()
                        .enumerate()
                        .map(move |(ordinal, &wing)| Ok(((group.id, wing), ordinal)))
                }),
            report,
            |actual, ordinal, report| {
                equal(
                    report,
                    format!(
                        "air theatre group {} wing {}.ordinal",
                        actual.group, actual.wing
                    ),
                    actual.ordinal as usize,
                    ordinal,
                );
                Ok(())
            },
        )
    });
    if passed {
        Ok(())
    } else {
        Err(invalid(
            "one or more theatre table comparisons failed; see individual comparison diagnostics",
        ))
    }
}

/// The serialized order of battle, joined once for all four comparisons.
///
/// The save carries each group's identity and ordered membership inside its
/// theatre, its theatre-group membership in the enclosing `theater_group`
/// listings, and its parent army group in the enclosing `field_marshal_group`
/// child listing. None of those is a field of the definition block, so they are
/// reconstructed here from the exact serialized structure rather than guessed.
struct OrderOfBattle<'a> {
    definitions: BTreeMap<u64, (&'a [u8], u64, &'a saved::OrdersGroup)>,
    theater_group: BTreeMap<u64, u64>,
    parent: BTreeMap<u64, u64>,
    unit_parent: BTreeMap<u64, u64>,
}

fn order_of_battle(saved: &saved::Pack) -> io::Result<OrderOfBattle<'_>> {
    let mut result = OrderOfBattle {
        definitions: BTreeMap::new(),
        theater_group: BTreeMap::new(),
        parent: BTreeMap::new(),
        unit_parent: BTreeMap::new(),
    };
    for (country, rows) in theatre_countries(saved)? {
        for theatre in &rows.land {
            for group in &theatre.groups {
                for &member in &group.members {
                    if result.theater_group.insert(member, group.id).is_some() {
                        return Err(invalid(
                            "a saved orders group belongs to more than one theatre group",
                        ));
                    }
                }
            }
            for definition in &theatre.definitions {
                if result
                    .definitions
                    .insert(definition.id, (country.as_slice(), theatre.id, definition))
                    .is_some()
                {
                    return Err(invalid("duplicate saved orders group definition"));
                }
                for &child in &definition.children {
                    if result.parent.insert(child, definition.id).is_some() {
                        return Err(invalid(
                            "a saved orders group is a child of more than one army group",
                        ));
                    }
                }
                for &unit in &definition.members {
                    if result.unit_parent.insert(unit, definition.id).is_some() {
                        return Err(invalid(
                            "a saved unit belongs to more than one orders group",
                        ));
                    }
                }
            }
        }
    }
    for (child, parent) in &result.parent {
        if !result.definitions.contains_key(child) {
            return Err(invalid(
                "a saved army group lists a child with no definition block",
            ));
        }
        if !result.definitions.contains_key(parent) {
            return Err(invalid(
                "a saved army group child lists an undefined parent",
            ));
        }
    }
    Ok(result)
}

/// COrdersGroup identity, its ordered unit membership, an army group's ordered
/// child membership, and the divisions' independent parent pointers.
pub(super) fn compare_order_of_battle(
    frame: &DecodedFrame,
    divisions: &BTreeMap<u64, Division>,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut passed = report.attempt("army OrdersGroups", |report| {
        let oob = order_of_battle(saved)?;
        compare_theatre_table(
            "OrdersGroups",
            complete::<tables::OrdersGroups>(frame)?,
            |row| row.handle,
            oob.definitions
                .iter()
                .map(|(&id, entry)| Ok((id, (entry, &oob)))),
            report,
            |actual, ((country, theatre, expected), oob), report| {
                let label = format!("orders group {}", expected.id);
                equal(
                    report,
                    format!("{label}.country"),
                    tag(actual.country, links)?,
                    *country,
                );
                equal(report, format!("{label}.theatre"), actual.theatre, *theatre);
                equal(
                    report,
                    format!("{label}.field_marshal_group"),
                    actual.is_army_group != 0,
                    expected.is_army_group,
                );
                if actual.is_army_group > 1 {
                    report.limitation(
                        "BE8100 writes the raw +0x39 discriminator through the boolean writer; a nonzero byte other than one cannot be distinguished from one in the save",
                        label.clone(),
                    );
                }
                equal(
                    report,
                    format!("{label}.motorization_level"),
                    i32::from(actual.motorization_level),
                    expected.motorization_level,
                );
                equal(
                    report,
                    format!("{label}.theater_group"),
                    actual.theater_group,
                    oob.theater_group.get(&expected.id).copied().unwrap_or(0),
                );
                equal(
                    report,
                    format!("{label}.parent"),
                    actual.parent,
                    oob.parent.get(&expected.id).copied().unwrap_or(0),
                );
                // BE8100 emits leader and leader_unit only for a non-null
                // pointer, so an omission proves the native pointer is null.
                equal(
                    report,
                    format!("{label}.leader"),
                    actual.leader,
                    expected.leader.unwrap_or(0),
                );
                equal(
                    report,
                    format!("{label}.leader_unit"),
                    actual.hq_provider,
                    expected.leader_unit.unwrap_or(0),
                );
                Ok(())
            },
        )
    });
    passed &= report.attempt("army OrdersGroupMembers", |report| {
        let oob = order_of_battle(saved)?;
        compare_theatre_table(
            "OrdersGroupMembers",
            complete::<tables::OrdersGroupMembers>(frame)?,
            |row| (row.group, row.ordinal),
            oob.definitions.values().flat_map(|(_, _, definition)| {
                definition
                    .members
                    .iter()
                    .enumerate()
                    .map(move |(ordinal, &unit)| Ok(((definition.id, ordinal as u32), unit)))
            }),
            report,
            |actual, unit, report| {
                equal(
                    report,
                    format!("orders group {} member {}", actual.group, actual.ordinal),
                    actual.unit,
                    unit,
                );
                Ok(())
            },
        )
    });
    passed &= report.attempt("army ArmyGroupMembers", |report| {
        let oob = order_of_battle(saved)?;
        compare_theatre_table(
            "ArmyGroupMembers",
            complete::<tables::ArmyGroupMembers>(frame)?,
            |row| (row.group, row.ordinal),
            oob.definitions.values().flat_map(|(_, _, definition)| {
                definition
                    .children
                    .iter()
                    .enumerate()
                    .map(move |(ordinal, &child)| Ok(((definition.id, ordinal as u32), child)))
            }),
            report,
            |actual, child, report| {
                equal(
                    report,
                    format!("army group {} child {}", actual.group, actual.ordinal),
                    actual.child,
                    child,
                );
                Ok(())
            },
        )
    });
    passed &= report.attempt("army division orders group parents", |report| {
        let oob = order_of_battle(saved)?;
        compare_division_parents(frame, divisions, &oob, report)
    });
    if passed {
        Ok(())
    } else {
        Err(invalid("an order-of-battle comparison group failed"))
    }
}

/// The group's own member vector and each division's `+0xc0` back-pointer are
/// two independent native sources for the same membership. Both are recorded;
/// this compares the division pointers against the save and reports any
/// disagreement between the two recorded sources without resolving it.
fn compare_division_parents(
    frame: &DecodedFrame,
    divisions: &BTreeMap<u64, Division>,
    oob: &OrderOfBattle<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut members = BTreeMap::new();
    for row in complete::<tables::OrdersGroupMembers>(frame)? {
        if members.insert(row.unit, row.group).is_some() {
            return Err(invalid(
                "a unit occurs in more than one recorded orders group member vector",
            ));
        }
    }
    let mut compared = 0;
    let mut cross_checked = 0;
    for division in complete::<tables::Divisions>(frame)? {
        let label = format!(
            "division type={} id={}",
            division.id as u32,
            division.id >> 32
        );
        let recorded = members.get(&division.id).copied().unwrap_or(0);
        if recorded != division.orders_group {
            report.difference(
                format!("{label}.orders_group recorded member vector"),
                division.orders_group,
                recorded,
            );
        }
        cross_checked += 1;
        // A division the save does not contain has no saved parent to compare
        // against; its absence is already reported by the divisions group.
        if !divisions.contains_key(&division.id) {
            continue;
        }
        equal(
            report,
            format!("{label}.orders_group"),
            division.orders_group,
            oob.unit_parent.get(&division.id).copied().unwrap_or(0),
        );
        compared += 1;
    }
    report.compared("Divisions.orders_group", compared);
    report.compared(
        "OrdersGroupMembers against division parent pointers",
        cross_checked,
    );
    Ok(())
}

fn theatre_countries(saved: &saved::Pack) -> io::Result<&BTreeMap<Vec<u8>, saved::TheatreCountry>> {
    saved
        .theatre_countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks theatre country roster"))
}

fn compare_theatre_table<K: Copy + Ord + std::fmt::Debug, R, E>(
    table: &str,
    recorded: &[R],
    key: impl Fn(&R) -> K,
    expected: impl Iterator<Item = io::Result<(K, E)>>,
    report: &mut Report,
    mut compare: impl FnMut(&R, E, &mut Report) -> io::Result<()>,
) -> io::Result<()> {
    let mut rows = BTreeMap::new();
    for row in recorded {
        if rows.insert(key(row), row).is_some() {
            return Err(invalid(format!("duplicate recorded {table} key")));
        }
    }
    let mut seen = BTreeSet::new();
    let mut compared = 0;
    for expected in expected {
        let (key, expected) = expected?;
        if !seen.insert(key) {
            return Err(invalid(format!("duplicate saved {table} key")));
        }
        if let Some(actual) = rows.remove(&key) {
            compare(actual, expected, report)?;
            compared += 1;
        } else {
            report.difference(format!("{table} {key:?}"), "absent", "present");
        }
    }
    for key in rows.keys() {
        report.difference(format!("{table} {key:?}"), "present", "absent");
    }
    report.compared(table, compared);
    Ok(())
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
            // The save exposes tags, so raw country aliases can collapse to one
            // tag. Compare every exact six-field tuple with its multiplicity.
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
mod tests {
    use super::*;
    use installed_content::save::{Decimal, GameDate};
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            AirTheatreGroupRow, AirTheatreMemberRow, AirTheatreRow, ArmyGroupMemberRow,
            DivisionRow, LandTheatreGroupRow, LandTheatreMemberRow, LandTheatreRow,
            NavalResultCountryRow, NavalResultMemberRow, NavalResultRow, OrdersGroupMemberRow,
            OrdersGroupRow, RailCooldownRow, RailEdgeRow, SunkShipsRow, TableId, TableRows,
        },
    };

    fn supremacy_frame(rows: Vec<NavalSupremacyRow>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::NavalSupremacy.index()] =
            TableView::Complete(TableRows::NavalSupremacy(rows));
        frame
    }

    const OOB_SAVE: &[u8] = br#"countries={GER={theatres={theatre={
        id={ id=2 type=67 }
        orders_group={
            id={ id=57 type=53 } name="Army 1"
            member={ unit={ id=12456 type=4713 } }
            member={ unit={ id=111 type=51 } }
            leader_unit={ id=12456 type=4713 }
            leader={ id=279 type=4713 }
            field_marshal_group=no motorization_level=0
        }
        field_marshal_group={
            orders_group={ id=57 type=53 }
            id={ id=431 type=53 } name="Army Group 1"
            field_marshal_group=yes motorization_level=0
        }
        theater_group={ id={ id=45 type=68 } priority=1 name="t"
            orders_group={ id=57 type=53 } orders_group={ id=431 type=53 } }
    }}}}"#;

    fn oob_handle(id: u64, kind: u64) -> u64 {
        (id << 32) | kind
    }

    fn oob_frame() -> DecodedFrame {
        let mut frame = supremacy_frame(Vec::new());
        *frame.tables[TableId::OrdersGroups.index()] =
            TableView::Complete(TableRows::OrdersGroups(vec![
                OrdersGroupRow {
                    handle: oob_handle(57, 53),
                    country: 7,
                    is_army_group: 0,
                    motorization_level: 0,
                    theatre: oob_handle(2, 67),
                    theater_group: oob_handle(45, 68),
                    parent: oob_handle(431, 53),
                    leader: oob_handle(279, 4713),
                    hq_provider: oob_handle(12456, 4713),
                },
                OrdersGroupRow {
                    handle: oob_handle(431, 53),
                    country: 7,
                    is_army_group: 1,
                    motorization_level: 0,
                    theatre: oob_handle(2, 67),
                    theater_group: oob_handle(45, 68),
                    parent: 0,
                    leader: 0,
                    hq_provider: 0,
                },
            ]));
        *frame.tables[TableId::OrdersGroupMembers.index()] =
            TableView::Complete(TableRows::OrdersGroupMembers(vec![
                OrdersGroupMemberRow {
                    group: oob_handle(57, 53),
                    ordinal: 0,
                    unit: oob_handle(12456, 4713),
                },
                OrdersGroupMemberRow {
                    group: oob_handle(57, 53),
                    ordinal: 1,
                    unit: oob_handle(111, 51),
                },
            ]));
        *frame.tables[TableId::ArmyGroupMembers.index()] =
            TableView::Complete(TableRows::ArmyGroupMembers(vec![ArmyGroupMemberRow {
                group: oob_handle(431, 53),
                ordinal: 0,
                child: oob_handle(57, 53),
            }]));
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![
                DivisionRow {
                    id: oob_handle(111, 51),
                    orders_group: oob_handle(57, 53),
                    ..DivisionRow::default()
                },
                DivisionRow {
                    id: oob_handle(12456, 4713),
                    orders_group: oob_handle(57, 53),
                    ..DivisionRow::default()
                },
            ]));
        frame
    }

    fn oob_divisions() -> BTreeMap<u64, Division> {
        [oob_handle(111, 51), oob_handle(12456, 4713)]
            .into_iter()
            .map(|id| {
                (
                    id,
                    Division {
                        id,
                        roster_owner: b"GER".to_vec(),
                        logical_owner: b"GER".to_vec(),
                        expeditionary_owner: None,
                        province: None,
                        template: None,
                        alternate_template: None,
                        separate_counter: false,
                        hit_points: Decimal::parse(b"0").unwrap(),
                        organisation: Decimal::parse(b"0").unwrap(),
                        country_intel: Vec::new(),
                        presentation: Default::default(),
                    },
                )
            })
            .collect()
    }

    fn run_oob(frame: &DecodedFrame, divisions: &BTreeMap<u64, Division>) -> Report {
        let germany = CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            ..CountryRow::default()
        };
        let links = BTreeMap::from([(7, &germany)]);
        let saved = saved::parse(&installed_content::parser::parse(OOB_SAVE).unwrap().items).unwrap();
        let mut report = Report::default();
        let _ = compare_order_of_battle(frame, divisions, &saved, &links, &mut report);
        report
    }

    #[test]
    fn a_matching_order_of_battle_has_no_differences() {
        let report = run_oob(&oob_frame(), &oob_divisions());
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
    }

    #[test]
    fn identity_membership_and_parent_disagreements_are_each_reported_once() {
        let mut frame = oob_frame();
        let TableView::Complete(TableRows::OrdersGroups(groups)) =
            &mut *frame.tables[TableId::OrdersGroups.index()]
        else {
            unreachable!()
        };
        groups[0].leader = 0;
        groups[0].theater_group = 0;
        groups[0].motorization_level = 4;
        groups[1].parent = oob_handle(57, 53);
        let TableView::Complete(TableRows::OrdersGroupMembers(members)) =
            &mut *frame.tables[TableId::OrdersGroupMembers.index()]
        else {
            unreachable!()
        };
        members[1].unit = oob_handle(999, 4713);
        let report = run_oob(&frame, &oob_divisions());
        let differences = report.differences.join("\n");
        assert!(
            differences.contains("orders group 244813135925.leader"),
            "{differences}"
        );
        assert!(differences.contains("theater_group"), "{differences}");
        assert!(differences.contains("motorization_level"), "{differences}");
        assert!(differences.contains("parent"), "{differences}");
        assert!(differences.contains("member 1"), "{differences}");
        // The division parent pointers disagree with the recorded member
        // vector for the unit that moved, and with the save for both.
        assert!(
            differences.contains("recorded member vector"),
            "{differences}"
        );
    }

    #[test]
    fn a_division_parent_pointer_is_compared_only_where_the_save_has_that_division() {
        let mut frame = oob_frame();
        let TableView::Complete(TableRows::Divisions(divisions)) =
            &mut *frame.tables[TableId::Divisions.index()]
        else {
            unreachable!()
        };
        // A division the save does not contain still cross-checks against the
        // recorded member vector, but has no saved parent to compare with.
        divisions.push(DivisionRow {
            id: oob_handle(65580, 4713),
            orders_group: 0,
            ..DivisionRow::default()
        });
        let report = run_oob(&frame, &oob_divisions());
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["Divisions.orders_group"], 2);
        assert_eq!(
            report.compared_tables["OrdersGroupMembers against division parent pointers"],
            3
        );
    }

    #[test]
    fn unavailable_tables_do_not_suppress_independent_naval_and_theatre_diagnostics() {
        let saved = saved::parse(&installed_content::parser::parse(br#"
            region={} rail_way={rail_way={1={cooldown=2}}}
            countries={GER={
                theatres={theatre={id={id=0 type=67}
                    theater_group={id={id=0 type=68} priority=1 name="land" orders_group={id=0 type=53}}}}
                air_theatre={id={id=0 type=77} country=GER name="theatre"
                    air_group={id={id=0 type=78} air_theatre={id=0 type=77} name="air" air_wing={{id=0 type=69}}}}
            }}
        "#).unwrap().items).unwrap();
        let mut frame = supremacy_frame(Vec::new());
        *frame.tables[TableId::SunkConvoys.index()] = TableView::Unavailable {
            reason: "convoy read failed".into(),
        };
        *frame.tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow::default()]));
        *frame.tables[TableId::NavalResults.index()] = TableView::Unavailable {
            reason: "result read failed".into(),
        };
        *frame.tables[TableId::RailCooldowns.index()] =
            TableView::Complete(TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 1,
            }]));
        *frame.tables[TableId::LandTheatres.index()] = TableView::Unavailable {
            reason: "land theatre read failed".into(),
        };
        *frame.tables[TableId::LandTheatreGroups.index()] =
            TableView::Complete(TableRows::LandTheatreGroups(vec![LandTheatreGroupRow {
                id: 68,
                theatre: 67,
                ordinal: 0,
                priority: 2,
                name: b"land".to_vec(),
            }]));
        *frame.tables[TableId::LandTheatreMembers.index()] =
            TableView::Complete(TableRows::LandTheatreMembers(vec![LandTheatreMemberRow {
                theater_group: 68,
                orders_group: 53,
                ordinal: 1,
            }]));
        *frame.tables[TableId::AirTheatres.index()] = TableView::Truncated {
            rows: TableRows::AirTheatres(Vec::new()),
            missing: 1,
        };
        *frame.tables[TableId::AirTheatreGroups.index()] =
            TableView::Complete(TableRows::AirTheatreGroups(vec![AirTheatreGroupRow {
                id: 78,
                theatre: 77,
                ordinal: 0,
                name: b"air".to_vec(),
            }]));
        *frame.tables[TableId::AirTheatreMembers.index()] =
            TableView::Complete(TableRows::AirTheatreMembers(vec![AirTheatreMemberRow {
                group: 78,
                wing: 69,
                ordinal: 9,
            }]));
        let mut report = Report::default();
        assert!(
            compare(
                &frame,
                &saved,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &mut report
            )
            .is_err()
        );
        for group in [
            "naval SunkConvoys",
            "naval NavalResults",
            "naval LandTheatres",
            "naval AirTheatres",
        ] {
            assert!(
                report.failed_comparisons.contains_key(group),
                "missing failure for {group}"
            );
        }
        assert_eq!(report.failed_comparisons.len(), 4);
        for table in [
            "SunkShips",
            "NavalResultMembers",
            "NavalResultCountries",
            "NavalSupremacy",
        ] {
            assert_eq!(
                report.compared_tables.get(table),
                Some(&0),
                "missing successful empty comparison for {table}"
            );
        }
        for table in [
            "RailCooldowns",
            "LandTheatreGroups",
            "LandTheatreMembers",
            "AirTheatreGroups",
            "AirTheatreMembers",
        ] {
            assert_eq!(
                report.compared_tables.get(table),
                Some(&1),
                "missing comparison for {table}"
            );
        }
        assert_eq!(report.differences.len(), 4);
        for group in [
            "naval RailCooldowns",
            "naval LandTheatreGroups",
            "naval LandTheatreMembers",
            "naval AirTheatreMembers",
        ] {
            assert_eq!(
                report.difference_groups.get(group),
                Some(&1),
                "missing difference for {group}"
            );
        }
    }

    #[test]
    fn railway_cooldown_omission_does_not_invent_zero() {
        let saved = saved::parse(
            &installed_content::parser::parse(b"rail_way={rail_way={1={cooldown=2} 2={}}}")
                .unwrap()
                .items,
        )
        .unwrap();
        let mut frame = supremacy_frame(Vec::new());
        *frame.tables[TableId::RailCooldowns.index()] =
            TableView::Complete(TableRows::RailCooldowns(vec![
                RailCooldownRow {
                    province: 1,
                    cooldown_ticks: 2,
                },
                RailCooldownRow {
                    province: 2,
                    cooldown_ticks: i32::MIN,
                },
            ]));
        *frame.tables[TableId::RailEdges.index()] =
            TableView::Complete(TableRows::RailEdges(vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: i32::MAX,
            }]));
        let mut report = Report::default();
        compare_railways(&frame, &saved, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["RailCooldowns"], 2);
        assert!(!report.compared_tables.contains_key("RailEdges"));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("signed-nonpositive cooldown"))
        );
        *frame.tables[TableId::RailCooldowns.index()] =
            TableView::Complete(TableRows::RailCooldowns(vec![
                RailCooldownRow {
                    province: 1,
                    cooldown_ticks: 1,
                },
                RailCooldownRow {
                    province: 2,
                    cooldown_ticks: 1,
                },
            ]));
        compare_railways(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
    }

    fn rail_adjacency() -> saved::RailwayAdjacency {
        let definitions = b"0;0;0;0;land\n1;1;0;0;land\n2;2;0;0;sea\n3;3;0;0;lake\n4;4;0;0;land\n";
        let mut bitmap = vec![0; 54];
        bitmap[..2].copy_from_slice(b"BM");
        bitmap[2..6].copy_from_slice(&70_u32.to_le_bytes());
        bitmap[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bitmap[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bitmap[18..22].copy_from_slice(&2_i32.to_le_bytes());
        bitmap[22..26].copy_from_slice(&2_i32.to_le_bytes());
        bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
        bitmap.extend_from_slice(&[0, 0, 1, 0, 0, 2, 0, 0, 0, 0, 3, 0, 0, 4, 0, 0]);
        saved::RailwayAdjacency::parse(definitions, &bitmap, None).unwrap()
    }

    fn rail_save() -> saved::Pack {
        saved::parse(&installed_content::parser::parse(b"rail_way={rail_way={1={rail_way={5 0 -7}} 2={rail_way={-1 2147483647 9}} 3={rail_way={0 8 0}} 4={rail_way={6 0 0}}}}").unwrap().items).unwrap()
    }

    fn rail_rows() -> Vec<RailEdgeRow> {
        [
            (1, 2, 5),
            (1, 4, -7),
            (2, 4, i32::MAX),
            (2, 3, 9),
            (3, 2, 8),
            (4, 1, 6),
        ]
        .into_iter()
        .map(|(from, to, level)| RailEdgeRow { from, to, level })
        .collect()
    }

    #[test]
    fn railway_edges_compare_exact_directed_levels_in_content_neighbor_order() {
        let adjacency = rail_adjacency();
        let saved = rail_save();
        let mut frame = supremacy_frame(Vec::new());
        *frame.tables[TableId::RailEdges.index()] =
            TableView::Complete(TableRows::RailEdges(rail_rows()));
        let mut report = Report::default();
        compare_rail_edges(&frame, &saved, &adjacency, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["RailEdges"], 6);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("nonpositive railway levels"))
        );

        let mut rows = rail_rows();
        rows[2].level = -8;
        rows.remove(3);
        rows.push(RailEdgeRow {
            from: 1,
            to: 3,
            level: 1,
        });
        rows.push(RailEdgeRow {
            from: 5,
            to: 1,
            level: 99,
        });
        *frame.tables[TableId::RailEdges.index()] = TableView::Complete(TableRows::RailEdges(rows));
        let mut report = Report::default();
        compare_rail_edges(&frame, &saved, &adjacency, &mut report).unwrap();
        assert_eq!(report.differences.len(), 4);
    }

    #[test]
    fn railway_edge_mapping_errors_and_duplicate_keys_fail() {
        let adjacency = rail_adjacency();
        let mut saved = rail_save();
        let mut frame = supremacy_frame(Vec::new());
        *frame.tables[TableId::RailEdges.index()] =
            TableView::Complete(TableRows::RailEdges(rail_rows()));
        saved
            .railways
            .as_mut()
            .unwrap()
            .get_mut(&2)
            .unwrap()
            .levels
            .pop();
        assert!(
            compare_rail_edges(&frame, &saved, &adjacency, &mut Report::default())
                .unwrap_err()
                .to_string()
                .contains("saved levels")
        );
        let mut rows = rail_rows();
        rows.push(rows[0].clone());
        *frame.tables[TableId::RailEdges.index()] = TableView::Complete(TableRows::RailEdges(rows));
        assert!(
            compare_rail_edges(&frame, &rail_save(), &adjacency, &mut Report::default())
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
    }

    #[test]
    fn theatre_oracle_checks_typed_parent_handles_priority_and_membership_order() {
        let saved = saved::parse(&installed_content::parser::parse(br#"countries={GER={
            theatres={theatre={id={id=4294967295 type=67} theater_group={id={id=0 type=68} priority=-1 name="group" orders_group={id=4294967295 type=53}}}}
            air_theatre={id={id=4294967295 type=77} country=GER name="theatre"
                air_group={id={id=0 type=78} air_theatre={id=4294967295 type=77} name="air" air_wing={{id=4294967295 type=69}}}}
        }}"#).unwrap().items).unwrap();
        let land = (u64::from(u32::MAX) << 32) | 67;
        let air = (u64::from(u32::MAX) << 32) | 77;
        let orders = (u64::from(u32::MAX) << 32) | 53;
        let wing = (u64::from(u32::MAX) << 32) | 69;
        let country = CountryRow {
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(123, &country)]);
        let mut frame = supremacy_frame(Vec::new());
        *frame.tables[TableId::LandTheatres.index()] =
            TableView::Complete(TableRows::LandTheatres(vec![LandTheatreRow {
                id: land,
                owner: 123,
                volunteers_theatre: 0,
                ordinal: 0,
            }]));
        *frame.tables[TableId::LandTheatreGroups.index()] =
            TableView::Complete(TableRows::LandTheatreGroups(vec![LandTheatreGroupRow {
                id: 68,
                theatre: land,
                ordinal: 0,
                priority: -1,
                name: b"group".to_vec(),
            }]));
        *frame.tables[TableId::LandTheatreMembers.index()] =
            TableView::Complete(TableRows::LandTheatreMembers(vec![LandTheatreMemberRow {
                orders_group: orders,
                theater_group: 68,
                ordinal: 0,
            }]));
        *frame.tables[TableId::AirTheatres.index()] =
            TableView::Complete(TableRows::AirTheatres(vec![AirTheatreRow {
                id: air,
                owner: 123,
                ordinal: 0,
                name: b"theatre".to_vec(),
            }]));
        *frame.tables[TableId::AirTheatreGroups.index()] =
            TableView::Complete(TableRows::AirTheatreGroups(vec![AirTheatreGroupRow {
                id: 78,
                theatre: air,
                ordinal: 0,
                name: b"air".to_vec(),
            }]));
        *frame.tables[TableId::AirTheatreMembers.index()] =
            TableView::Complete(TableRows::AirTheatreMembers(vec![AirTheatreMemberRow {
                wing,
                group: 78,
                ordinal: 0,
            }]));
        let mut report = Report::default();
        compare_theatres(&frame, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        for table in [
            "LandTheatres",
            "LandTheatreGroups",
            "LandTheatreMembers",
            "AirTheatres",
            "AirTheatreGroups",
            "AirTheatreMembers",
        ] {
            assert_eq!(report.compared_tables[table], 1);
        }
        let TableView::Complete(TableRows::LandTheatreGroups(groups)) =
            &mut *frame.tables[TableId::LandTheatreGroups.index()]
        else {
            unreachable!()
        };
        groups[0].priority = 0;
        groups[0].theatre ^= 1;
        let TableView::Complete(TableRows::AirTheatreMembers(members)) =
            &mut *frame.tables[TableId::AirTheatreMembers.index()]
        else {
            unreachable!()
        };
        members[0].ordinal = 1;
        compare_theatres(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3);
        *frame.tables[TableId::AirTheatreMembers.index()] = TableView::Unavailable {
            reason: "reverse registry incomplete".into(),
        };
        assert!(compare_theatres(&frame, &saved, &links, &mut report).is_err());
    }

    #[test]
    fn supremacy_uses_runtime_slot_and_compares_every_fixed5_unit() {
        let text = installed_content::parser::parse(br#"region={1={dominance={values={
            { --- {current=90071992547.40993 base_target=-92233720368547.75808 target=92233720368547.75807 individual_ratio=-0.00001 previous=0 decline_from=1.00001} }
        }}}}"#).unwrap();
        let saved = saved::parse(&text.items).unwrap();
        let row = NavalSupremacyRow {
            region: 0x8765_4321,
            region_slot: 1,
            country: 0,
            current: 9_007_199_254_740_993,
            base_target: i64::MIN,
            target: i64::MAX,
            individual_ratio: -1,
            previous: 0,
            decline_from: 100_001,
        };
        let mut report = Report::default();
        compare_supremacy(
            &supremacy_frame(vec![row.clone()]),
            &saved,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        let mut changed = row.clone();
        changed.current -= 1;
        changed.base_target += 1;
        changed.target -= 1;
        changed.individual_ratio += 1;
        changed.previous += 1;
        changed.decline_from -= 1;
        compare_supremacy(
            &supremacy_frame(vec![changed]),
            &saved,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 6);
        let mut wrong_slot = row.clone();
        wrong_slot.region_slot = 2;
        let mut report = Report::default();
        compare_supremacy(
            &supremacy_frame(vec![wrong_slot]),
            &saved,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 2);
        assert!(
            compare_supremacy(
                &supremacy_frame(vec![row.clone(), row]),
                &saved,
                &BTreeMap::new(),
                &mut report
            )
            .is_err()
        );
        assert!(
            compare_supremacy(
                &supremacy_frame(Vec::new()),
                &saved::Pack::default(),
                &BTreeMap::new(),
                &mut report
            )
            .is_err()
        );
        let mut unavailable = supremacy_frame(Vec::new());
        *unavailable.tables[TableId::NavalSupremacy.index()] = TableView::Unavailable {
            reason: "source read failed".into(),
        };
        assert!(compare_supremacy(&unavailable, &saved, &BTreeMap::new(), &mut report).is_err());
    }

    #[test]
    fn supremacy_country_aliases_keep_tuple_multiplicity_and_missing_values_are_empty() {
        let text = installed_content::parser::parse(br#"region={1={dominance={values={
            { GER {current=0.00001 base_target=0 target=0 individual_ratio=0 previous=0 decline_from=0} }
            { GER {current=0.00002 base_target=0 target=0 individual_ratio=0 previous=0 decline_from=0} }
        }}}}"#).unwrap();
        let saved = saved::parse(&text.items).unwrap();
        let country = CountryRow {
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(1, &country), (2, &country)]);
        let one = NavalSupremacyRow {
            region: 99,
            region_slot: 1,
            country: 1,
            current: 2,
            ..Default::default()
        };
        let two = NavalSupremacyRow {
            region: 99,
            region_slot: 1,
            country: 2,
            current: 1,
            ..Default::default()
        };
        let mut report = Report::default();
        compare_supremacy(
            &supremacy_frame(vec![one.clone(), two]),
            &saved,
            &links,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("multiplicities"))
        );
        compare_supremacy(
            &supremacy_frame(vec![one.clone()]),
            &saved,
            &links,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        let empty = installed_content::parser::parse(b"region={1={dominance={}}}").unwrap();
        let empty = saved::parse(&empty.items).unwrap();
        let mut report = Report::default();
        compare_supremacy(&supremacy_frame(vec![one]), &empty, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn retained_result_census_checks_full_handles_and_child_order() {
        let date = GameDate::parse(b"1939.1.1.1").unwrap();
        let id = (u64::from(u32::MAX) << 32) | 61;
        let member = (u64::from(u32::MAX) << 32) | 69;
        let saved = saved::Pack {
            results: BTreeMap::from([(
                id,
                saved::NavalResult {
                    id,
                    province: u32::MAX,
                    result_date: date,
                    retire_date: date,
                    category: i32::MIN as u32,
                    port_strike: true,
                    naval_strike: false,
                    sides: [
                        saved::ResultSide {
                            details: Default::default(),
                            members: vec![saved::ResultMember {
                                details: Default::default(),
                                id: member,
                                sunk_by: Some(vec![0xff, b'a']),
                            }],
                            countries: vec![b"---".to_vec()],
                        },
                        saved::ResultSide::default(),
                    ],
                    admitted: Vec::new(),
                },
            )]),
            ..Default::default()
        };
        let mut frame = DecodedFrame {
            branch: 1,
            hour: date.engine_hour().unwrap(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::NavalResults.index()] =
            TableView::Complete(TableRows::NavalResults(vec![NavalResultRow {
                id,
                province: u32::MAX,
                result_hour: frame.hour,
                retire_hour: frame.hour,
                category: i32::MIN as u32,
                flag_a: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::NavalResultMembers.index()] =
            TableView::Complete(TableRows::NavalResultMembers(vec![NavalResultMemberRow {
                result: id,
                member,
                presence_value: 2,
                ..Default::default()
            }]));
        *frame.tables[TableId::NavalResultCountries.index()] =
            TableView::Complete(TableRows::NavalResultCountries(vec![
                NavalResultCountryRow {
                    result: id,
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        compare_results(&frame, &saved, &mut report).unwrap();
        compare_members(&frame, &saved, &mut report).unwrap();
        compare_countries(&frame, &saved, &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty());
        let TableView::Complete(TableRows::NavalResultMembers(rows)) =
            &mut *frame.tables[TableId::NavalResultMembers.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[0];
        row.ordinal = 1;
        compare_members(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        let TableView::Complete(TableRows::NavalResultMembers(rows)) =
            &mut *frame.tables[TableId::NavalResultMembers.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[0];
        row.member ^= 1;
        compare_members(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3);
        *frame.tables[TableId::NavalResultCountries.index()] = TableView::Unavailable {
            reason: "native read failed".into(),
        };
        assert!(compare_countries(&frame, &saved, &BTreeMap::new(), &mut report).is_err());
    }

    #[test]
    fn anonymous_ship_fields_compare_exactly_and_omissions_remain_limits() {
        let expected = saved::SunkShip {
            ship_name: Some(vec![0xff]),
            attribution_name: None,
            victim_country: b"---".to_vec(),
            attacker_country: b"---".to_vec(),
            sink_date: GameDate::parse(b"1939.1.1.1").unwrap(),
            class: b"destroyer".to_vec(),
            attribution_class: b"none".to_vec(),
            model_level: i32::MIN,
            victim_equipment_variant: Some((u64::from(u32::MAX) << 32) | 70),
            damage_source_reference: None,
            province: None,
            battle: 0,
            cause_record: false,
            assisted_kill: false,
        };
        let mut actual = SunkShipEntry {
            ship_name: vec![0xff],
            attribution_name: b"retained".to_vec(),
            sink_hour: expected.sink_date.engine_hour().unwrap(),
            class_token: 1,
            attribution_class_token: 2,
            attribution_class_present: 1,
            model_level: i32::MIN,
            victim_equipment_variant: expected.victim_equipment_variant.unwrap(),
            damage_source_reference: u64::MAX,
            ..Default::default()
        };
        let tokens = BTreeMap::from([(1, b"destroyer".as_slice()), (2, b"none".as_slice())]);
        let mut report = Report::default();
        compare_ship(
            &actual,
            &expected,
            "ship",
            &BTreeMap::new(),
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 2);
        actual.victim_equipment_variant ^= 1;
        actual.model_level += 1;
        actual.province_present = 1;
        compare_ship(
            &actual,
            &expected,
            "ship",
            &BTreeMap::new(),
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 3);
        let incomplete_tokens = BTreeMap::from([(1, b"destroyer".as_slice())]);
        assert!(
            compare_ship(
                &actual,
                &expected,
                "ship",
                &BTreeMap::new(),
                &incomplete_tokens,
                &mut report
            )
            .is_err()
        );
        let mut unavailable_report = Report::default();
        compare_ship(
            &actual,
            &expected,
            "ship",
            &BTreeMap::new(),
            &BTreeMap::new(),
            &mut unavailable_report,
        )
        .unwrap();
        assert!(
            unavailable_report
                .limitations
                .keys()
                .any(|reason| reason.contains("lexical tokens are unavailable"))
        );
    }

    #[test]
    fn sunk_ship_lexical_classes_keep_nul_limits_and_explicit_pointer_presence() {
        let saved = saved::parse(
            &installed_content::parser::parse(
                br#"history={sunk_ship={
            name="visible" killer_name="" country=--- killer_country=--- level=0
            definition=destroyer killer_definition=none date=1939.1.1.1
            battle={id=0 type=0} convoy=no
        }}"#,
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let expected = &saved.sunk_ships[0];
        let mut actual = SunkShipEntry {
            ship_name: b"visible\0\xffretained".to_vec(),
            sink_hour: expected.sink_date.engine_hour().unwrap(),
            class_token: 1,
            attribution_class_token: 2,
            attribution_class_present: 1,
            // Neither literal definition-name string substitutes for its token.
            class_name: b"different captured name".to_vec(),
            attribution_class_name: Vec::new(),
            ..Default::default()
        };
        let tokens = BTreeMap::from([
            (1, b"destroyer\0\xffsuffix".as_slice()),
            (2, b"none\0suffix".as_slice()),
        ]);
        let mut report = Report::default();
        compare_ship(
            &actual,
            expected,
            "ship",
            &BTreeMap::new(),
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(
            report
                .limitations
                .iter()
                .find(|(reason, _)| reason.contains("first NUL"))
                .unwrap()
                .1
                .len(),
            1
        );
        assert_eq!(
            report
                .limitations
                .iter()
                .find(|(reason, _)| reason.contains("unproved text-stream termination"))
                .unwrap()
                .1
                .len(),
            2
        );

        let wrong = BTreeMap::from([(1, b"submarine".as_slice()), (2, b"none".as_slice())]);
        let mut report = Report::default();
        compare_ship(
            &actual,
            expected,
            "ship",
            &BTreeMap::new(),
            &wrong,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("ship.definition"));
        assert!(
            compare_ship(
                &actual,
                expected,
                "ship",
                &BTreeMap::new(),
                &BTreeMap::from([(1, b"destroyer".as_slice())]),
                &mut Report::default()
            )
            .is_err()
        );

        actual.attribution_class_present = 0;
        actual.attribution_class_token = 0;
        let mut report = Report::default();
        compare_ship(
            &actual,
            expected,
            "ship",
            &BTreeMap::new(),
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("ship.killer_definition"));
        actual.attribution_class_present = 2;
        assert!(
            compare_ship(
                &actual,
                expected,
                "ship",
                &BTreeMap::new(),
                &tokens,
                &mut Report::default()
            )
            .is_err()
        );
        actual.attribution_class_present = 1;
        actual.attribution_class_token = 2;
        actual.class_token = 0x167;
        let reserved = BTreeMap::from([(0x167, b"destroyer".as_slice()), (2, b"none".as_slice())]);
        assert!(
            compare_ship(
                &actual,
                expected,
                "ship",
                &BTreeMap::new(),
                &reserved,
                &mut Report::default()
            )
            .is_err()
        );
    }

    #[test]
    fn sunk_convoy_count_uses_signed_native_integer_without_scaling() {
        let expected = saved::SunkConvoy {
            month: i32::MIN,
            killer_country: b"---".to_vec(),
            owner_country: b"---".to_vec(),
            convoys: i32::MAX,
        };
        let mut actual = SunkConvoyRow {
            month: i32::MIN,
            convoys: i32::MAX,
            ..Default::default()
        };
        let mut report = Report::default();
        compare_convoy(&actual, &expected, "convoy", &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty());
        actual.convoys -= 1;
        compare_convoy(&actual, &expected, "convoy", &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(flag(128).is_err());
    }
}
