use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::save::{Division, naval_history as saved};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};

use super::{Report, complete, equal, invalid, tag};

fn name(report: &mut Report, label: String, actual: &[u8], expected: Option<&[u8]>) {
    if let Some(expected) = expected {
        equal(report, label, actual, expected);
    } else {
        report.limitation("native theatre serializer can suppress name bytes by writer mode; omission does not establish an empty string", label);
    }
}

pub(super) fn compare_theatres(
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
pub(in crate::oracle) fn compare_order_of_battle(
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
                // EED430 emits each theatre roster in its own native order.
                // An unrostered recorded group has None, distinct from the
                // first saved position; every saved definition is rostered.
                equal(
                    report,
                    format!("{label}.theatre_ordinal"),
                    actual.theatre_ordinal,
                    Some(expected.theatre_ordinal),
                );
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
