use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content_save::{Division, naval_history as saved};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, Recorded, tables},
};

use super::{Report, complete, equal, invalid, tag};

const UNSAVED_RESERVED_THEATRE: &str = "the game omits actual country slot zero; this certified reserved theatre has no serialized payload";

#[derive(Default)]
struct ReservedTheatres {
    omitted: BTreeSet<u64>,
    census_difference: Option<(bool, bool)>,
}

/// An omission is proved by the actual reserved roster and physical country
/// census together. A positive physical alias reaches the ordinary serializer,
/// so its missing country block or payload remains a difference.
fn reserved_theatres(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    emissions: Option<&[installed_content_save::SavedCountryEmission]>,
) -> io::Result<ReservedTheatres> {
    let Ok(land) = complete::<tables::LandTheatres>(frame) else {
        return Ok(ReservedTheatres::default());
    };
    let certified = complete::<tables::ReservedCountryNaming>(frame).is_ok_and(
        |rows| matches!(rows, [row] if matches!(&row.land_theatre_count, Recorded::Value(Some(_)))),
    );
    if !land.iter().any(|row| row.owner == 0) && !certified {
        return Ok(ReservedTheatres::default());
    }
    let naming = complete::<tables::ReservedCountryNaming>(frame)?;
    let [naming] = naming else {
        return Err(invalid(
            "reserved theatre partition requires one reserved naming row",
        ));
    };
    let count = match &naming.land_theatre_count {
        Recorded::Value(Some(count)) => *count,
        Recorded::Value(None) => return Err(invalid("reserved theatre certificate was refused")),
        Recorded::NotRecorded => {
            return Err(invalid("reserved theatre certificate was not recorded"));
        }
    };
    let mut zero = BTreeMap::new();
    let mut handles = BTreeSet::new();
    for row in land.iter().filter(|row| row.owner == 0) {
        if row.id as u32 != 67 || row.id >> 32 == 0 {
            return Err(invalid("reserved theatre has an invalid full handle"));
        }
        if zero.insert(row.ordinal, row.id).is_some() || !handles.insert(row.id) {
            return Err(invalid(
                "reserved theatre has a duplicate handle or ordinal",
            ));
        }
    }
    if zero.len() != count as usize
        || zero.keys().copied().ne(0..count)
        || land
            .iter()
            .filter(|row| row.owner != 0)
            .any(|row| handles.contains(&row.id))
    {
        return Err(invalid(
            "reserved theatre rows do not match their atomic count certificate",
        ));
    }
    let physical = match &naming.country_array {
        Recorded::Value(Some(physical)) if physical.first() == Some(&0) => physical,
        _ => {
            return Err(invalid(
                "reserved theatre partition requires the authenticated physical country array",
            ));
        }
    };
    let aliases = physical.iter().skip(1).filter(|&&id| id == 0).count();
    let admitted_aliases = emissions.map(|emissions| {
        emissions
            .iter()
            .filter(|emission| {
                emission.identity.actual == installed_content_save::ActualCountry::Reserved
                    && emission.identity.tag == naming.tag
            })
            .count()
    });
    if admitted_aliases.is_some_and(|count| count != aliases) {
        return Err(invalid(
            "reserved theatre aliases differ from the admitted full-save census",
        ));
    }
    if aliases > 1 && admitted_aliases.is_none() {
        return Err(invalid(
            "repeated reserved country save tags require admitted full-save occurrences",
        ));
    }
    let countries = theatre_countries(saved)?;
    let serialized = countries.contains_key(naming.tag.as_slice());
    let mut scope = ReservedTheatres {
        census_difference: (serialized != (aliases > 0)).then_some((aliases > 0, serialized)),
        ..Default::default()
    };
    if !serialized && aliases == 0 {
        let saved_handles: BTreeSet<_> = countries
            .values()
            .flat_map(|country| &country.land)
            .map(|theatre| theatre.id)
            .collect();
        // A handle serialized under any country is always compared, including
        // a contradictory enclosing country. No saved payload is partitioned out.
        scope
            .omitted
            .extend(handles.difference(&saved_handles).copied());
    }
    Ok(scope)
}

fn omitted_reserved_theatres(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    report: &mut Report,
    report_census_difference: bool,
) -> (BTreeSet<u64>, bool) {
    let mut omitted = BTreeSet::new();
    let passed = report.attempt("reserved land theatre serialization", |report| {
        let scope = reserved_theatres(frame, saved, report.country_emissions.as_deref())?;
        if report_census_difference && let Some((actual, expected)) = scope.census_difference {
            report.difference("reserved country physical save census", actual, expected);
        }
        omitted = scope.omitted;
        Ok(())
    });
    (omitted, passed)
}

fn omitted_groups(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    theatres: &BTreeSet<u64>,
) -> BTreeSet<u64> {
    let Some(countries) = saved.theatre_countries.as_ref() else {
        return BTreeSet::new();
    };
    let serialized: BTreeSet<_> = countries
        .values()
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        .map(|group| group.id)
        .collect();
    complete::<tables::OrdersGroups>(frame)
        .map(|rows| {
            rows.iter()
                .filter(|row| theatres.contains(&row.theatre) && !serialized.contains(&row.handle))
                .map(|row| row.handle)
                .collect()
        })
        .unwrap_or_default()
}

fn reserved_tag(frame: &DecodedFrame) -> io::Result<&[u8]> {
    let rows = complete::<tables::ReservedCountryNaming>(frame)?;
    let [row] = rows else {
        return Err(invalid(
            "serialized reserved theatre requires one recorded reserved tag",
        ));
    };
    Ok(row.tag.as_slice())
}

fn check_omitted_ordinals(
    table: &str,
    rows: impl Iterator<Item = (u64, u32)>,
    omitted: &BTreeSet<u64>,
) -> io::Result<()> {
    let mut ordinals: BTreeMap<u64, BTreeSet<u32>> = BTreeMap::new();
    for (group, ordinal) in rows.filter(|(group, _)| omitted.contains(group)) {
        if !ordinals.entry(group).or_default().insert(ordinal) {
            return Err(invalid(format!("duplicate reserved {table} ordinal")));
        }
    }
    for positions in ordinals.values() {
        if positions
            .iter()
            .copied()
            .enumerate()
            .any(|(expected, actual)| expected != actual as usize)
        {
            return Err(invalid(format!(
                "reserved {table} ordinals are not contiguous"
            )));
        }
    }
    Ok(())
}

fn name(report: &mut Report, label: String, actual: &[u8], expected: Option<&[u8]>) {
    // The theatre-group and air-theatre writers
    // emit name unconditionally outside checksum mode, even when empty.
    let text = super::serialized_text(report, &label, actual);
    equal(report, label, Some(text), expected);
}

fn check_member_units<'a>(
    frame: &DecodedFrame,
    members: impl Iterator<Item = &'a replay::rows::OrdersGroupMemberRow>,
) -> io::Result<()> {
    let divisions: BTreeSet<_> = complete::<tables::Divisions>(frame)?
        .iter()
        .map(|row| row.id)
        .collect();
    for member in members {
        if !divisions.contains(&member.unit) {
            return Err(invalid(
                "reserved group member unit is absent from the complete division census; its parent cannot be independently joined",
            ));
        }
    }
    Ok(())
}

fn check_theatre_group_ordinals(
    groups: &[replay::rows::OrdersGroupRow],
    theatres: &BTreeSet<u64>,
) -> io::Result<()> {
    let mut ordinals: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for group in groups
        .iter()
        .filter(|group| theatres.contains(&group.theatre))
    {
        if group.handle as u32 != 53 {
            return Err(invalid("reserved orders group has an invalid full handle"));
        }
        if let Some(ordinal) = group.theatre_ordinal
            && !ordinals
                .entry((group.theatre, group.is_army_group != 0))
                .or_default()
                .insert(ordinal)
        {
            return Err(invalid(
                "reserved orders groups have duplicate theatre roster ordinals",
            ));
        }
    }
    for positions in ordinals.values() {
        if positions
            .iter()
            .copied()
            .enumerate()
            .any(|(expected, actual)| expected != actual as usize)
        {
            return Err(invalid(
                "reserved orders group theatre roster ordinals are not contiguous",
            ));
        }
    }
    Ok(())
}

/// These recorded regular-group units have no serialized grouping payload.
/// Callers qualify only units also present in the save; totals stay exact.
pub fn omitted_regular_group_units(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    emissions: Option<&[installed_content_save::SavedCountryEmission]>,
) -> io::Result<BTreeSet<u64>> {
    let scope = reserved_theatres(frame, saved, emissions)?;
    if scope.omitted.is_empty() {
        return Ok(BTreeSet::new());
    }
    let omitted = omitted_groups(frame, saved, &scope.omitted);
    let groups = complete::<tables::OrdersGroups>(frame)?;
    let mut seen_groups = BTreeSet::new();
    if groups.iter().any(|row| !seen_groups.insert(row.handle)) {
        return Err(invalid("duplicate recorded orders group identity"));
    }
    check_theatre_group_ordinals(groups, &scope.omitted)?;
    let regular: BTreeSet<_> = groups
        .iter()
        .filter(|row| omitted.contains(&row.handle) && row.is_army_group == 0)
        .map(|row| row.handle)
        .collect();
    if regular.is_empty() {
        return Ok(BTreeSet::new());
    }
    let members = complete::<tables::OrdersGroupMembers>(frame)?;
    check_omitted_ordinals(
        "OrdersGroupMembers",
        members.iter().map(|row| (row.group, row.ordinal)),
        &regular,
    )?;
    let mut divisions = BTreeMap::new();
    for division in complete::<tables::Divisions>(frame)? {
        if divisions.insert(division.id, division).is_some() {
            return Err(invalid("duplicate recorded division identity"));
        }
    }
    let serialized: BTreeSet<_> = theatre_countries(saved)?
        .values()
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        .flat_map(|group| &group.members)
        .copied()
        .collect();
    let mut units = BTreeSet::new();
    let mut seen = BTreeSet::new();
    if members.iter().any(|row| !seen.insert(row.unit)) {
        return Err(invalid(
            "recorded orders group membership duplicates a unit",
        ));
    }
    for member in members.iter().filter(|row| regular.contains(&row.group)) {
        let division = divisions.get(&member.unit).ok_or_else(|| invalid(
            "reserved group member unit is absent from the complete division census; its parent cannot be independently joined",
        ))?;
        if division.orders_group != member.group {
            return Err(invalid(
                "omitted regular-group members disagree with division parents",
            ));
        }
        units.insert(member.unit);
    }
    if divisions
        .values()
        .any(|division| regular.contains(&division.orders_group) && !units.contains(&division.id))
    {
        return Err(invalid(
            "division parent names an omitted regular group without reciprocal member membership",
        ));
    }
    units.retain(|unit| !serialized.contains(unit));
    Ok(units)
}

pub(super) fn compare_theatres(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let (omitted, mut passed) = omitted_reserved_theatres(frame, saved, report, true);
    if let Ok([row]) = complete::<tables::ReservedCountryNaming>(frame)
        && let Recorded::Value(Some(count)) = &row.land_theatre_count
    {
        report.limitation("the text save does not carry the reserved roster's atomic capture certificate; a serialized alias permits a payload comparison, while an omitted actual slot-zero roster's count and completeness require a matched native witness", format!("ReservedCountryNaming.land_theatre_count={count}"));
    }
    passed &= report.attempt("naval LandTheatres", |report| {
        let countries = theatre_countries(saved)?;
        compare_partitioned_theatre_table(
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
                let owner = if actual.owner == 0 { reserved_tag(frame)? } else { tag(actual.owner, links)? };
                equal(report, format!("{label}.country roster"), owner, country);
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
                    report.limitation("the theatre writer omits signed-nonpositive volunteers_theatre references; omission does not distinguish their raw native bits", label);
                }
                Ok(())
            },
            |row| omitted.contains(&row.id),
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
                // The air-group save writer always emits its name.
                let field = format!("{label}.name");
                let text = super::serialized_text(report, &field, &actual.name);
                equal(report, field, text, expected.name.as_slice());
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

fn order_of_battle<'a>(
    saved: &'a saved::Pack,
    omitted: &BTreeSet<u64>,
) -> io::Result<OrderOfBattle<'a>> {
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
        if !result.definitions.contains_key(child) && !omitted.contains(child) {
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
pub fn compare_order_of_battle(
    frame: &DecodedFrame,
    divisions: &BTreeMap<u64, Division>,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    // The land comparison reports the physical country-census difference once.
    // This independent join still applies the same strict partition rules.
    let (omitted_theatres, mut passed) = omitted_reserved_theatres(frame, saved, report, false);
    let omitted = omitted_groups(frame, saved, &omitted_theatres);
    passed &= report.attempt("reserved OrdersGroups", |_report| {
        if omitted_theatres.is_empty() {
            return Ok(());
        }
        let groups = complete::<tables::OrdersGroups>(frame)?;
        check_theatre_group_ordinals(groups, &omitted_theatres)?;
        for group in groups
            .iter()
            .filter(|group| omitted_theatres.contains(&group.theatre))
        {
            if group.country != 0 {
                tag(group.country, links)?;
            }
        }
        Ok(())
    });
    passed &= report.attempt("reserved OrdersGroupMembers", |_report| {
        if omitted.is_empty() {
            return Ok(());
        }
        let members = complete::<tables::OrdersGroupMembers>(frame)?;
        check_omitted_ordinals(
            "OrdersGroupMembers",
            members.iter().map(|row| (row.group, row.ordinal)),
            &omitted,
        )?;
        check_member_units(
            frame,
            members.iter().filter(|row| omitted.contains(&row.group)),
        )
    });
    passed &= report.attempt("reserved ArmyGroupMembers", |report| {
        if omitted.is_empty() {
            return Ok(());
        }
        let members = complete::<tables::ArmyGroupMembers>(frame)?;
        check_omitted_ordinals(
            "ArmyGroupMembers",
            members.iter().map(|row| (row.group, row.ordinal)),
            &omitted,
        )?;
        check_reserved_parents(frame, &omitted, report)
    });
    passed &= report.attempt("army OrdersGroups", |report| {
        let oob = order_of_battle(saved, &omitted)?;
        compare_partitioned_theatre_table(
            "OrdersGroups",
            complete::<tables::OrdersGroups>(frame)?,
            |row| row.handle,
            oob.definitions
                .iter()
                .map(|(&id, entry)| Ok((id, (entry, &oob)))),
            report,
            |actual, ((country, theatre, expected), oob), report| {
                let label = format!("orders group {}", expected.id);
                // The formation writer never emits its raw country. The serialized enclosing country
                // proves roster placement, not this independent raw reference.
                // Keep strict positive-reference resolution without inventing a
                // country equality for cross-country or reserved groups.
                if actual.country != 0 {
                    let actual_country = tag(actual.country, links)?;
                    if actual_country != *country {
                        report.limitation("the formation writer does not serialize the orders group's raw country; a cross-country reference needs an independent native witness", format!("{label}.country"));
                    }
                } else {
                    report.limitation("the formation writer does not serialize the orders group's raw reserved-country reference", format!("{label}.country"));
                }
                match &actual.name {
                    Recorded::NotRecorded => report.limitation("stored command name not in this recording", format!("{label}.name")),
                    Recorded::Value(None) => return Err(invalid("stored command name capture refused")),
                    Recorded::Value(Some(bytes)) => {
                        // The formation writer gates on stored length before the quoted-text writer writes
                        // the first-NUL prefix, including an emitted empty one.
                        let name = (!bytes.is_empty()).then(|| {
                            super::serialized_text(report, &format!("{label}.name"), bytes)
                        });
                        equal(report, format!("{label}.name"), name, expected.name.as_deref());
                    }
                }
                equal(report, format!("{label}.theatre"), actual.theatre, *theatre);
                // The theatre writer emits each theatre roster in its own native order.
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
                        "the formation writer writes the raw discriminator through the boolean writer; a nonzero byte other than one cannot be distinguished from one in the save",
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
                if !oob.parent.contains_key(&expected.id) && omitted.contains(&actual.parent) {
                    report.limitation("the parent pointer is inferred from serialized army-group children; the actual reserved parent's child payload is omitted", format!("{label}.parent"));
                } else {
                    equal(report, format!("{label}.parent"), actual.parent,
                        oob.parent.get(&expected.id).copied().unwrap_or(0));
                }
                // The formation writer emits leader and leader_unit only for a non-null
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
            |row| omitted.contains(&row.handle),
        )
    });
    passed &= report.attempt("army OrdersGroupMembers", |report| {
        let oob = order_of_battle(saved, &omitted)?;
        let members = complete::<tables::OrdersGroupMembers>(frame)?;
        compare_partitioned_theatre_table(
            "OrdersGroupMembers",
            members,
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
            |row| omitted.contains(&row.group),
        )
    });
    passed &= report.attempt("army ArmyGroupMembers", |report| {
        let oob = order_of_battle(saved, &omitted)?;
        let members = complete::<tables::ArmyGroupMembers>(frame)?;
        compare_partitioned_theatre_table(
            "ArmyGroupMembers",
            members,
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
            |row| omitted.contains(&row.group),
        )
    });
    passed &= report.attempt("army division orders group parents", |report| {
        let oob = order_of_battle(saved, &omitted)?;
        compare_division_parents(frame, divisions, &oob, &omitted, report)
    });
    report.limitation("the formation writer does not serialize the group's raw country reference; enclosing saved country tags certify theatre roster placement, not raw group-country identity", "OrdersGroups.country");
    if passed {
        Ok(())
    } else {
        Err(invalid("an order-of-battle comparison group failed"))
    }
}

/// The group's own member vector and each division's back-pointer are
/// two independent native sources for the same membership. Both are recorded;
/// this compares the division pointers against the save and reports any
/// disagreement between the two recorded sources without resolving it.
fn compare_division_parents(
    frame: &DecodedFrame,
    divisions: &BTreeMap<u64, Division>,
    oob: &OrderOfBattle<'_>,
    omitted: &BTreeSet<u64>,
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
        if !oob.unit_parent.contains_key(&division.id)
            && omitted.contains(&division.orders_group)
            && recorded == division.orders_group
        {
            report.limitation("CArmy does not serialize its parent pointer and the actual reserved group's member payload is omitted; this saved division's inferred parent is not comparable", format!("{label}.orders_group"));
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

fn check_reserved_parents(
    frame: &DecodedFrame,
    omitted: &BTreeSet<u64>,
    report: &mut Report,
) -> io::Result<()> {
    if omitted.is_empty() {
        return Ok(());
    }
    let groups: BTreeMap<_, _> = complete::<tables::OrdersGroups>(frame)?
        .iter()
        .map(|row| (row.handle, row))
        .collect();
    let mut parents = BTreeMap::new();
    for member in complete::<tables::ArmyGroupMembers>(frame)? {
        if !omitted.contains(&member.group) && !omitted.contains(&member.child) {
            continue;
        }
        let parent = groups
            .get(&member.group)
            .ok_or_else(|| invalid("reserved army-group child joins an absent parent"))?;
        if !groups.contains_key(&member.child) {
            return Err(invalid("reserved army-group child joins an absent child"));
        }
        if parent.is_army_group == 0 || member.group == member.child {
            return Err(invalid(
                "reserved army-group child has an invalid parent kind or self-reference",
            ));
        }
        if parents.insert(member.child, member.group).is_some() {
            return Err(invalid(
                "reserved army-group child has duplicate parent membership",
            ));
        }
    }
    for group in groups.values().filter(|row| {
        omitted.contains(&row.handle)
            || omitted.contains(&row.parent)
            || parents.contains_key(&row.handle)
    }) {
        equal(
            report,
            format!("orders group {}.parent recorded child vector", group.handle),
            group.parent,
            parents.get(&group.handle).copied().unwrap_or(0),
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "theatres/tests.rs"]
mod tests;

fn compare_theatre_table<K: Copy + Ord + std::fmt::Debug, R, E>(
    table: &str,
    recorded: &[R],
    key: impl Fn(&R) -> K,
    expected: impl Iterator<Item = io::Result<(K, E)>>,
    report: &mut Report,
    compare: impl FnMut(&R, E, &mut Report) -> io::Result<()>,
) -> io::Result<()> {
    compare_partitioned_theatre_table(table, recorded, key, expected, report, compare, |_| false)
}

fn compare_partitioned_theatre_table<K: Copy + Ord + std::fmt::Debug, R, E>(
    table: &str,
    recorded: &[R],
    key: impl Fn(&R) -> K,
    expected: impl Iterator<Item = io::Result<(K, E)>>,
    report: &mut Report,
    mut compare: impl FnMut(&R, E, &mut Report) -> io::Result<()>,
    omitted: impl Fn(&R) -> bool,
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
    for (key, row) in rows {
        if omitted(row) {
            report.limitation(UNSAVED_RESERVED_THEATRE, format!("{table} {key:?}"));
        } else {
            report.difference(format!("{table} {key:?}"), "present", "absent");
        }
    }
    report.compared(table, compared);
    Ok(())
}
