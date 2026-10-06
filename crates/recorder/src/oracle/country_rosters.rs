//! Ordered owned-goal payloads and country roster set comparisons.

use super::{Report, Save, TokenNames, complete, invalid, required_country_tag};
use replay::{
    frame::DecodedFrame,
    rows::{CountryCounterInputRow, CountryRow, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

fn equal<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    label: impl AsRef<str>,
    actual: T,
    expected: T,
) {
    report.scalar_checks += 1;
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

pub(super) fn compare_owned(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::OwnedWargoals>(frame)?;
    let mut by_tag: BTreeMap<&[u8], BTreeMap<u32, _>> = BTreeMap::new();
    for row in rows {
        let tag = required_country_tag(row.country, links)?;
        if by_tag
            .entry(tag)
            .or_default()
            .insert(row.ordinal, row)
            .is_some()
        {
            return Err(invalid("duplicate OwnedWargoals country ordinal"));
        }
    }
    // 140D32BD0 saves every +0x68 slot, including null references. Its +0x80
    // object list supplies the payloads through 1415CE720 -> 1415CE460.
    for (tag, handles) in &save.country_rosters.owned_goals {
        let recorded = by_tag.remove(tag.as_slice()).unwrap_or_default();
        let label = format!("OwnedWargoals {}", String::from_utf8_lossy(tag));
        if recorded.keys().copied().ne(0..recorded.len() as u32) {
            return Err(invalid(format!("{label} ordinals are not contiguous")));
        }
        equal(
            report,
            format!("{label}.count"),
            recorded.len(),
            handles.len(),
        );
        for (&ordinal, row) in &recorded {
            let Some(handle) = handles.get(ordinal as usize) else {
                continue;
            };
            let item = format!("{label}:{ordinal}");
            if *handle == 0 {
                report.limitation(
                    "the save's available_wargoals slot is a null reference; it has no goal payload to compare",
                    &item,
                );
                continue;
            }
            let Some(expected) = save.country_rosters.goals.get(handle) else {
                report.limitation(
                    "the save's available_wargoals reference has no serialized goal payload; recipient, type and claimed states cannot be compared",
                    format!("{item} saved handle {handle}"),
                );
                continue;
            };
            if row.recipient == 0 {
                report.limitation(
                    "ccountry_tag::tag_string@140BA5C20 saves the null database tag; its literal is not recorded by CountryLinks",
                    format!("{item}.recipient"),
                );
            } else {
                equal(
                    report,
                    format!("{item}.recipient"),
                    required_country_tag(row.recipient, links)?,
                    expected.recipient.as_slice(),
                );
            }
            if let Some(actual) = tokens.get(&(row.goal_type as u32)) {
                if actual.contains(&0) {
                    report.limitation(
                        "wargoal type lexer literal contains an interior NUL; the token text stream's projection is unproved",
                        format!("{item}.goal_type"),
                    );
                } else {
                    equal(
                        report,
                        format!("{item}.goal_type"),
                        *actual,
                        expected.goal_type.as_slice(),
                    );
                }
            } else {
                report.limitation(
                    "owned wargoal type lexer literal is not recorded",
                    format!("{item}.goal_type {}", row.goal_type),
                );
            }
            if row.states.contains(&0) {
                report.limitation(
                    "1415CE460 dereferences every claimed-state pointer; the save cannot serialize a null membership recorded as state id zero",
                    format!("{item}.states"),
                );
            }
            equal(
                report,
                format!("{item}.states"),
                row.states
                    .iter()
                    .copied()
                    .filter(|&state| state != 0)
                    .collect::<Vec<_>>(),
                expected.states.clone(),
            );
        }
    }
    for (tag, rows) in by_tag {
        report.difference(
            format!("OwnedWargoals {} country", String::from_utf8_lossy(tag)),
            rows.len(),
            "no saved country",
        );
    }
    report.compared("OwnedWargoals", rows.len());
    report.limitation(
        "saved goal tags do not preserve raw country-reference aliases; payload comparison uses CountryLinks",
        "OwnedWargoals.country/recipient",
    );
    Ok(())
}

fn roster<T: Copy + Ord + Default + std::fmt::Debug>(
    report: &mut Report,
    label: &str,
    recorded: Option<&[T]>,
    expected: BTreeSet<T>,
) {
    if let Some(recorded) = recorded {
        if recorded.contains(&T::default()) {
            report.limitation(
                "saved state ownership and active character roles cannot certify null country roster slots recorded as zero",
                label,
            );
        }
        equal(
            report,
            label,
            recorded
                .iter()
                .copied()
                .filter(|value| *value != T::default())
                .collect(),
            expected,
        );
    } else {
        report.limitation("country roster operand was unavailable at capture", label);
    }
}

pub(super) fn compare_lists(
    rows: &[CountryCounterInputRow],
    save: &Save,
    tags: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut owned: BTreeMap<&[u8], BTreeSet<u32>> = BTreeMap::new();
    let mut controlled: BTreeMap<&[u8], BTreeSet<u32>> = BTreeMap::new();
    for (&id, state) in &save.states {
        if let Some(owner) = &state.owner {
            owned.entry(owner.as_slice()).or_default().insert(id);
        }
        // PostRead@1409CDF80 supplies an omitted controller from its owner.
        if let Some(controller) = state.controller.as_ref().or(state.owner.as_ref()) {
            controlled
                .entry(controller.as_slice())
                .or_default()
                .insert(id);
        }
    }
    let mut leader_roles = BTreeMap::new();
    for leader in save
        .characters
        .manager
        .iter()
        .flat_map(|manager| manager.unit_leaders.values())
    {
        if leader_roles.insert(leader.character, leader).is_some() {
            return Err(invalid(format!(
                "duplicate saved military leader roles for character {}",
                leader.character
            )));
        }
    }
    for row in rows {
        let tag = *tags
            .get(&row.country)
            .ok_or_else(|| invalid("country roster has no recorded country"))?;
        let label = format!("CountryCounterInputs {}", String::from_utf8_lossy(tag));
        let owned_states = report.recorded(
            &row.owned_states,
            "CountryCounterInputs.owned_states",
            &label,
        );
        let controlled_states = report.recorded(
            &row.controlled_states,
            "CountryCounterInputs.controlled_states",
            &label,
        );
        let core_states =
            report.recorded(&row.core_states, "CountryCounterInputs.core_states", &label);
        let army_leaders = report.recorded(
            &row.army_leaders,
            "CountryCounterInputs.army_leaders",
            &label,
        );
        let navy_leaders = report.recorded(
            &row.navy_leaders,
            "CountryCounterInputs.navy_leaders",
            &label,
        );
        let Some(country) = save.countries.get(tag) else {
            report.difference(format!("{label} country"), "recorded", "absent from save");
            continue;
        };
        if let Some(states) = owned_states {
            roster(
                report,
                &format!("{label}.owned_states"),
                states.as_deref(),
                owned.remove(tag).unwrap_or_default(),
            );
        }
        if let Some(states) = controlled_states {
            roster(
                report,
                &format!("{label}.controlled_states"),
                states.as_deref(),
                controlled.remove(tag).unwrap_or_default(),
            );
        }
        // CCountry::Write@14070C580 writes +0x4A8/count+0x4B4 as `cores`
        // in native order, retaining duplicates rather than reconstructing it.
        match core_states {
            Some(Some(states)) => {
                if states.contains(&0) {
                    report.limitation(
                        "saved cores dereference state pointers and cannot certify null roster slots recorded as zero",
                        format!("{label}.core_states"),
                    );
                }
                equal(
                    report,
                    format!("{label}.core_states"),
                    states
                        .iter()
                        .copied()
                        .filter(|&state| state != 0)
                        .collect::<Vec<_>>(),
                    country.cores.clone(),
                );
            }
            Some(None) => report.limitation(
                "country roster operand was unavailable at capture",
                format!("{label}.core_states"),
            ),
            None => {}
        }
        let Some(manager) = &save.characters.manager else {
            report.limitation(
                "save lacks character_manager; army/navy roster sets cannot be joined to saved roles",
                format!("{label}.army_leaders/navy_leaders"),
            );
            continue;
        };
        let Some(statuses) = save
            .characters
            .character_status
            .get(tag)
            .filter(|statuses| statuses.iter().all(|status| status.unit_leader.is_some()))
        else {
            report.limitation(
                "save lacks complete active unit_leader status bytes; army/navy roster membership cannot be reconstructed",
                format!("{label}.army_leaders/navy_leaders"),
            );
            continue;
        };
        let mut army = BTreeSet::new();
        let mut navy = BTreeSet::new();
        let mut roles_complete = true;
        for status in statuses
            .iter()
            .filter(|status| status.unit_leader == Some(true))
        {
            let character = &status.character;
            // 1410DAF80 inserts kind 0/1 into +0x70 and kind 2 into +0x88.
            // It resolves through 140F8A840; operative fallback kind 3 is
            // independent of the serialized military roles and both rosters.
            match leader_roles.get(character) {
                Some(leader) if leader.category < 2 => {
                    army.insert(leader.id);
                }
                Some(leader) if leader.category == 2 => {
                    navy.insert(leader.id);
                }
                _ if manager
                    .characters
                    .get(character)
                    .is_some_and(|character| character.operative.is_some()) => {}
                _ => {
                    roles_complete = false;
                    report.limitation(
                        "active saved unit_leader status has no serialized military role or operative reference",
                        format!("{label} character {character}"),
                    );
                }
            }
        }
        if roles_complete {
            for (name, recorded, expected) in [
                ("army_leaders", army_leaders, army),
                ("navy_leaders", navy_leaders, navy),
            ] {
                if let Some(recorded) = recorded {
                    let item = format!("{label}.{name}");
                    roster(report, &item, recorded.as_deref(), expected);
                }
            }
        }
    }
    report.limitation(
        "owned and controlled states and military leader rosters compare as sets; their order, duplicate multiplicity and null slots are outside this comparison, and controlled states assume nonzero province membership as the existing count comparison does; cores compare in saved native order",
        "CountryCounterInputs owned/controlled states and leader rosters",
    );
    Ok(())
}

#[cfg(test)]
mod tests;
