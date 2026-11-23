
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_key_names::buildings::Buildings;
use content_reader_save::{PartialMapMembership, Save, StateControlEmission};
use replay_data::{rows::CountryRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{Report, complete, invalid, required_country_tag};

const REFRESH_RECALCULATES: u32 = 1 << 1 | 1 << 9;

pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    membership: &PartialMapMembership,
    content: &Buildings,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = BTreeMap::new();
    for row in complete::<tables::CountryFactories>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if actual.insert(tag, row).is_some() {
            return Err(invalid("duplicate resolved country in factory table"));
        }
    }
    let control = saved.state_control_emissions(membership)?;
    let states = complete::<tables::States>(frame).ok();
    let captured = states.map(|rows| {
        rows.iter()
            .map(|row| (row.id, row))
            .collect::<BTreeMap<_, _>>()
    });
    if states
        .zip(captured.as_ref())
        .is_some_and(|(rows, map)| rows.len() != map.len())
    {
        return Err(invalid("duplicate state row in factory inputs"));
    }
    let mut controllers = BTreeMap::new();
    let mut unsaved_controls = 0;
    let building_states: BTreeSet<_> = saved
        .construction
        .state_buildings
        .keys()
        .map(|(id, _)| *id)
        .collect();
    for state in &control {
        match state {
            StateControlEmission::Positive(state) => {
                controllers.insert(state.id, Some(state.controller.as_slice()));
            }
            StateControlEmission::NonPositiveOwner { id } => {
                if !building_states.contains(id) {
                    continue;
                }
                let row = captured
                    .as_ref()
                    .and_then(|rows| rows.get(id))
                    .ok_or_else(|| {
                        invalid(format!(
                            "unowned state {id} has no captured controller for factory inputs"
                        ))
                    })?;
                let controller = if (row.controller as i32) > 0 {
                    Some(required_country_tag(row.controller, links)?)
                } else {
                    None
                };
                controllers.insert(*id, controller);
                unsaved_controls += 1;
            }
        }
    }
    let mut expected = BTreeMap::<&[u8], [i64; 3]>::new();
    let mut ambiguous = BTreeSet::<&[u8]>::new();
    let placed = saved
        .construction
        .state_buildings
        .iter()
        .map(|((state, _), building)| {
            controllers
                .get(state)
                .copied()
                .map(|country| (country, building))
                .ok_or_else(|| {
                    invalid(format!(
                        "saved buildings of state {state} have no saved controller"
                    ))
                })
        })
        .chain(
            saved
                .construction
                .country_buildings
                .iter()
                .map(|((country, _), building)| Ok((Some(country.as_slice()), building))),
        );
    for entry in placed {
        let (country, building) = entry?;
        let name = String::from_utf8_lossy(&building.building_type);
        let flags = content.factory_flags.get(name.as_ref()).ok_or_else(|| {
            invalid(format!(
                "saved building type {name} is not declared in the content"
            ))
        })?;
        let Some(country) = country else {
            continue;
        };
        if content.ambiguous_factory_types.contains(name.as_ref()) {
            ambiguous.insert(country);
        }
        let levels = expected.entry(country).or_default();
        for (sum, counted) in
            levels
                .iter_mut()
                .zip([flags.military(), flags.naval(), flags.civilian()])
        {
            if counted {
                *sum += i64::from(building.level);
            }
        }
    }
    let productions = saved
        .production
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks the production countries"))?;
    let (mut count, mut pending, mut refused) = (0, 0, 0);
    for (tag, country) in report.canonical_countries(&saved.countries) {
        let text = String::from_utf8_lossy(tag);
        let label = format!("{text}.factories");
        let actual = actual.remove(tag.as_slice());
        let Some(actual) = actual else {
            report.difference(label, "absent", "saved country present");
            continue;
        };
        let refresh = country
            .refresh
            .ok_or_else(|| invalid(format!("saved country {text} lacks refresh")))?;
        let dirty = productions
            .get(tag)
            .and_then(|country| country.production.as_ref())
            .and_then(|production| production.dirty)
            .ok_or_else(|| {
                invalid(format!(
                    "saved country {text} lacks its production dirty flag"
                ))
            })?;
        if dirty || refresh & REFRESH_RECALCULATES != 0 {
            pending += 1;
            continue;
        }
        if ambiguous.contains(tag.as_slice()) {
            refused += 1;
            continue;
        }
        count += 1;
        let levels = expected.get(tag.as_slice()).copied().unwrap_or_default();
        for ((name, actual), expected) in [
            ("military_levels", actual.military_levels),
            ("naval_levels", actual.naval_levels),
            ("civilian_levels", actual.civilian_levels),
        ]
        .into_iter()
        .zip(levels)
        {
            if i64::from(actual) != expected {
                report.difference(format!("{label}.{name}"), actual, expected);
            }
        }
    }
    for tag in actual.keys() {
        report.difference(
            format!("{}.factories", String::from_utf8_lossy(tag)),
            "present",
            "country absent",
        );
    }
    report.compared("CountryFactories", count);
    if unsaved_controls > 0 {
        report.limitation(
            "a nonpositive-owner state's controller is omitted by the game; its captured signed-positive control is an input to the saved-building-level invariant, not an independently saved controller comparison",
            format!("CountryFactories: {unsaved_controls} nonpositive-owner states"),
        );
    }
    report.limitation(
        "the save writes neither the stored group-head levels nor the other factory totals (the production save writer omits them; factory-total recalculation rebuilds them); levels are compared only with saved building inputs: totals, controlled, owned and damaged factories (state output factors and country modifiers are not saved), assigned and withheld factories, subject and other-country transfers, licence and trade exports and each civilian consumer's allocation, consumer goods included, are not compared",
        "CountryFactories",
    );
    if pending > 0 {
        report.limitation(
            "a country whose save shows a factory recalculation pending (production dirty, or refresh bit 1 or 9) holds totals from before it; its levels are not compared",
            format!("CountryFactories: {pending} countries"),
        );
    }
    if refused > 0 {
        report.limitation(
            "a building type declared again with different factory flags: which declaration the game uses is unverified, so countries with such buildings are not compared",
            format!("CountryFactories: {refused} countries"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
