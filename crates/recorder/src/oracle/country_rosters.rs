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
    for (tag, handles) in report.canonical_countries(&save.country_rosters.owned_goals) {
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
        if let Some(cache) = report.recorded(
            &row.co_belligerents,
            "CountryCounterInputs.co_belligerents",
            &label,
        ) {
            report.limitation(
                if cache.is_some() {
                    "directed CRelationStatus+0x310 cache truth is runtime-only; saved wars and factions cannot certify rebuild timing"
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

/// Current-hour owner domains; saves do not serialize the composed values.
pub(super) fn compare_movement(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::CountryCounterInputs>(frame)?;
    if rows.is_empty() {
        return Ok(());
    }
    let links = complete::<tables::CountryLinks>(frame).map(|rows| {
        rows.iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect::<BTreeMap<_, _>>()
    });
    let mut controllers = None;
    let mut naval_demands = None;
    for row in rows {
        let label = format!("CountryCounterInputs {}.movement", row.country);
        let Some(movement) =
            report.recorded(&row.movement, "CountryCounterInputs.movement", &label)
        else {
            continue;
        };
        let Some(movement) = movement else {
            report.difference(
                &label,
                "unavailable",
                "required current-hour movement packet",
            );
            continue;
        };
        equal(
            report,
            format!("{label}.sample_hour"),
            movement.sample_hour,
            frame.hour,
        );
        if movement.sample_hour != frame.hour {
            continue;
        }
        let mut naval_demanded = false;
        if movement.naval.is_none() || movement.theatre.is_none() {
            report.attempt(&format!("{label}.naval demand"), |_| {
                let slots = naval_demands
                    .get_or_insert_with(|| super::map_inputs::navy_owner_slots(frame))
                    .as_ref()
                    .map_err(|reason| invalid(reason.to_string()))?;
                let links = links
                    .as_ref()
                    .map_err(|reason| invalid(reason.to_string()))?;
                let slot = links
                    .get(&row.country)
                    .filter(|&&slot| slot != 0)
                    .ok_or_else(|| {
                        invalid("naval movement demand owner has no positive country link")
                    })?;
                naval_demanded = slots.contains(slot);
                Ok(())
            });
        }
        if movement.river.is_some()
            || movement.transport.is_some()
            || movement.naval.is_some()
            || movement.theatre.is_some()
        {
            report.attempt(&format!("{label}.owner"), |_| {
                let links = links
                    .as_ref()
                    .map_err(|reason| invalid(reason.to_string()))?;
                if row.country == 0 || !links.contains_key(&row.country) {
                    return Err(invalid(
                        "country movement owner has no positive country link",
                    ));
                }
                Ok(())
            });
        }
        if let Some(river) = &movement.river {
            report.attempt(&format!("{label}.river"), |report| {
                if controllers.is_none() {
                    let links = links
                        .as_ref()
                        .map_err(|reason| invalid(reason.to_string()))?;
                    let mut domain = BTreeSet::new();
                    for province in complete::<tables::Provinces>(frame)? {
                        domain.insert(if province.controller == 0 {
                            0
                        } else {
                            *links
                                .get(&province.controller)
                                .ok_or_else(|| invalid("movement controller has no country link"))?
                        });
                    }
                    controllers = Some(domain);
                }
                if river
                    .windows(2)
                    .any(|pair| pair[0].target_slot >= pair[1].target_slot)
                {
                    return Err(invalid(format!(
                        "{label}.river keys are not unique and increasing"
                    )));
                }
                let actual: BTreeSet<_> = river.iter().map(|entry| entry.target_slot).collect();
                equal(
                    report,
                    format!("{label}.river controller domain"),
                    &actual,
                    controllers.as_ref().unwrap(),
                );
                for entry in river {
                    if entry.modifier.is_none() {
                        report.difference(
                            format!("{label}.river target {}", entry.target_slot),
                            "unavailable",
                            "required runtime modifier",
                        );
                    }
                }
                Ok(())
            });
        } else {
            report.limitation("owner river domain was not demanded or unavailable; absence is not a complete empty domain", format!("{label}.river"));
        }
        if let Some(transport) = &movement.transport {
            report.attempt(&format!("{label}.transport"), |report| {
                if transport
                    .windows(2)
                    .any(|pair| pair[0].region >= pair[1].region)
                {
                    return Err(invalid(format!(
                        "{label}.transport keys are not unique and increasing"
                    )));
                }
                for entry in transport {
                    if entry.speed.is_none() {
                        report.difference(
                            format!("{label}.transport region {}", entry.region),
                            "unavailable",
                            "required runtime speed",
                        );
                    }
                }
                Ok(())
            });
        } else {
            report.limitation("owner transport domain was not demanded or unavailable; absence is not a complete empty domain", format!("{label}.transport"));
        }
        if let Some(naval) = &movement.naval {
            report.attempt(&format!("{label}.naval owner slot"), |report| {
                let links = links
                    .as_ref()
                    .map_err(|reason| invalid(reason.to_string()))?;
                let expected = links
                    .get(&row.country)
                    .ok_or_else(|| invalid("naval movement owner has no country link"))?;
                equal(
                    report,
                    format!("{label}.naval.navy_slot"),
                    naval.navy_slot,
                    *expected,
                );
                Ok(())
            });
            for (name, values) in [
                ("foreign_navy_slots", &naval.foreign_navy_slots),
                ("supply_hubs", &naval.supply_hubs),
            ] {
                match values {
                    None => report.difference(
                        format!("{label}.naval.{name}"),
                        "unavailable",
                        "required native stored list",
                    ),
                    Some(values) if !naval.present && !values.is_empty() => report.difference(
                        format!("{label}.naval.{name}"),
                        values,
                        "empty for observed null navy",
                    ),
                    Some(_) => {}
                }
            }
        } else if naval_demanded {
            report.difference(
                format!("{label}.naval"),
                "unavailable",
                "required moving task-force owner navy domain",
            );
        } else {
            report.limitation("owner naval domain was not demanded or unavailable; absence is not an observed null navy", format!("{label}.naval"));
        }
        if let Some(theatre) = &movement.theatre {
            report.attempt(&format!("{label}.theatre"), |report| {
                compare_theatre_areas(frame, theatre, &format!("{label}.theatre"), report)
            });
        } else if naval_demanded {
            report.difference(
                format!("{label}.theatre"),
                "unavailable",
                "required moving task-force owner controller-area membership",
            );
        } else {
            report.limitation("owner theatre domain was not demanded or unavailable; absence does not certify empty pointer membership", format!("{label}.theatre"));
        }
    }
    report.compared("CountryCounterInputs.movement", rows.len());
    report.limitation("effective river-against relation modifiers and raw owner/region transport speeds are live composed operands, not direct saved scalar values. Saved diplomacy, ideas, weather and equipment do not certify their accumulation or refresh state; the transport region domain's native census also has no independent text-save comparator", "CountryCounterInputs.movement runtime operands");
    report.limitation("no exact text-save comparator is proved for the native navy's stored foreign physical-slot and supply-hub lists; their order, repeated and zero occurrences require a matched runtime witness, independently of current diplomacy or country aliases", "CountryCounterInputs.movement naval stored lists");
    Ok(())
}

pub(super) fn compare_theatre_areas(
    frame: &DecodedFrame,
    theatre: &replay::rows::TheatreControllerAreas,
    label: &str,
    report: &mut Report,
) -> io::Result<()> {
    if theatre.provinces.contains(&0) || theatre.provinces.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid(format!(
            "{label} provinces are not positive, unique and increasing"
        )));
    }
    let provinces: BTreeSet<_> = complete::<tables::Provinces>(frame)?
        .iter()
        .map(|row| row.id)
        .collect();
    if let Some(id) = theatre
        .provinces
        .iter()
        .find(|&&id| !provinces.contains(&id))
    {
        return Err(invalid(format!("{label} names unknown province {id}")));
    }
    report.limitation("membership compares retained theatre area pointers against actual CProvince+D0, including a separate null-area witness; saved theatre descriptors cannot certify this runtime pointer equality", label);
    Ok(())
}

#[cfg(test)]
#[path = "country_rosters/zero_subject_tests.rs"]
mod zero_subject_tests;

#[cfg(test)]
mod tests;
