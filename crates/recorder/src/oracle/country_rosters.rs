//! Country roster set comparisons.

use super::{Report, Save, invalid};
use replay::rows::CountryCounterInputRow;
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
        // The loader supplies an omitted controller from its owner.
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
        if let Some(cache) = report.recorded(
            &row.co_belligerents,
            "CountryCounterInputs.co_belligerents",
            &label,
        ) {
            report.limitation(
                if cache.is_some() {
                    "the directed alliance cache truth is runtime-only; saved wars and factions cannot certify rebuild timing"
                } else {
                    "directed co-belligerent cache was unavailable at capture"
                },
                format!("{label}.co_belligerents"),
            );
        }
        if let Some(war) = report.recorded(&row.zero_war, "CountryCounterInputs.zero_war", &label) {
            report.limitation(
                if war.is_some() {
                    "slot-zero relation war-pointer presence and raw retired byte are runtime-only; zero is outside the saved country census"
                } else {
                    "slot-zero war operands were unavailable at capture"
                },
                format!("{label}.zero_war"),
            );
        }
        if let Some(subject) = report.recorded(
            &row.zero_subject,
            "CountryCounterInputs.zero_subject",
            &label,
        ) {
            report.limitation(
                if subject.is_some() {
                    "slot-zero subject pointer/type/raw parties are runtime-only: the saved relation matrix omits slot0; native VM witnesses are required"
                } else {
                    "slot-zero subject operands were unavailable at capture"
                },
                format!("{label}.zero_subject"),
            );
        }
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
        // The country writer saves the ordered core-state roster as `cores`
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
            // Character roles of kinds 0/1 use the leader roster, kind 2 the advisor roster.
            // It resolves raw references; operative fallback kind 3 is
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
#[path = "country_rosters/zero_subject_tests.rs"]
mod zero_subject_tests;

#[cfg(test)]
mod tests;
