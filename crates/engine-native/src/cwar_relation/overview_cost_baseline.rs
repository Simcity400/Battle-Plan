use engine_native_parts::cdiplomacy_status_layout as cdiplomacy_status;
use super::super::*;
pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut [OwnedRow],
    goals: &mut [cwar_goal::relation::Row],
    scores: &mut [CountryRow],
) -> [Outcome; 3] {
    let mut used = [0_usize; 3];
    let mut wanted = [0_usize; 3];
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, state)?;
        let playing = field::<i32>(memory, state, crate::cgame_state::LOCAL_PLAYER_COUNTRY)?;
        let view = if playing > 0 {
            playing as u32
        } else {
            field::<u32>(memory, state, crate::cgame_state::OBSERVED_COUNTRY)?
        };
        let view = if view == 0 {
            None
        } else {
            Some(countries.resolve(view)?)
        };
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let diplomacy = diplomacy(memory, base, country)?;
            let owned = array(memory, diplomacy, cdiplomacy_status::OWNED_RELATIONS, 8)?;
            let mut has_war = false;
            for ordinal in 0..owned.count {
                let relation = element(memory, owned, ordinal)?;
                if field::<u32>(memory, relation, crelation::TYPE_ID)? != crate::cwar_relation::TYPE
                {
                    continue;
                }
                has_war = true;
                vtable(memory, base, relation, crate::cwar_relation::VTABLE_RVA)?;
                let first_country = field(memory, relation, crelation::FIRST_COUNTRY)?;
                let second_country = field(memory, relation, crelation::SECOND_COUNTRY)?;
                let first = countries.resolve(first_country)?;
                let second = countries.resolve(second_country)?;
                if first.canonical_slot == second.canonical_slot
                    || (slot != first.canonical_slot && slot != second.canonical_slot)
                {
                    return Err("war overview owned endpoints mismatch");
                }
                let lists = [
                    array(memory, relation, crate::cwar_relation::GOALS[0], 8)?,
                    array(memory, relation, crate::cwar_relation::GOALS[1], 8)?,
                ];
                wanted[0] += 1;
                if let Some(row) = output.get_mut(used[0]) {
                    *row = OwnedRow {
                        source: Source {
                            relation,
                            diplomacy,
                            owner_slot: country.canonical_slot,
                            first_slot: first.canonical_slot,
                            second_slot: second.canonical_slot,
                        },
                        owner: country.id,
                        ordinal,
                        first_country,
                        second_country,
                        retired: field(memory, relation, crelation::RETIRED)?,
                        first_goal_count: lists[0].count,
                        second_goal_count: lists[1].count,
                    };
                    used[0] += 1;
                }
                for (side, list) in lists.into_iter().enumerate() {
                    wanted[1] = wanted[1]
                        .checked_add(list.count as usize)
                        .ok_or("war goal count overflow")?;
                    for index in 0..list.count {
                        let Some(row) = goals.get_mut(used[1]) else {
                            break;
                        };
                        *row = cwar_goal::relation::read_goal(
                            memory,
                            base,
                            &countries,
                            cwar_goal::relation::Row {
                                owner: country.id,
                                relation_ordinal: ordinal,
                                side: side as u8,
                                ordinal: index,
                                handle: element(memory, list, index)? as u64,
                                ..Default::default()
                            },
                        )?;
                        used[1] += 1;
                    }
                }
            }
            if has_war {
                wanted[2] += 1;
                if let Some(row) = scores.get_mut(used[2]) {
                    *row = CountryRow {
                        country: country.id,
                        major: field(memory, country.pointer, ccountry::DYNAMIC_MAJOR)?,
                        divisions: array(memory, country.pointer, ccountry::DIVISIONS, 8)?.count
                            as i32,
                        factories: factories(memory, base, &countries, country)?,
                        view_subject: view
                            .map(|view| cached_subject(memory, base, &countries, view, country))
                            .transpose()?
                            .unwrap_or(false) as u8,
                        ..Default::default()
                    };
                    used[2] += 1;
                }
            }
        }
        Ok(())
    })();
    if let Err(reason) = result {
        return [Outcome::Unavailable { reason }; 3];
    }
    std::array::from_fn(|index| {
        let unread = wanted[index] - used[index];
        match u32::try_from(unread) {
            Ok(0) => Outcome::Rows { len: used[index] },
            Ok(unread_rows) => Outcome::Truncated {
                len: used[index],
                unread_rows,
            },
            Err(_) => Outcome::Unavailable {
                reason: "war overview unread count overflow",
            },
        }
    })
}

fn factories(
    memory: &impl crate::Memory,
    base: usize,
    countries: &crate::ccountry::Reader<'_, impl crate::Memory>,
    country: crate::ccountry::Country,
) -> Result<i32, &'static str> {
    use crate::{
        cbuilding,
        cprovince::buildings as cbuilding_status,
        cbuilding_template,
        ccountry,
        cpdx_array::Array,
        cstate,
        read,
    };
    fn field<T: crate::memory::Raw>(
        memory: &impl crate::Memory,
        pointer: usize,
        offset: usize,
    ) -> Result<T, &'static str> {
        pointer
            .checked_add(offset)
            .and_then(|p| read(memory, p))
            .ok_or("war factory field unreadable or address overflow")
    }
    fn authenticate(
        memory: &impl crate::Memory,
        base: usize,
        pointer: usize,
        rva: engine_sites::Rva,
    ) -> Result<(), &'static str> {
        let expected = engine_sites::at(base, rva).map(|p| p as u64);
        if pointer == 0 || expected.is_none() || read::<u64>(memory, pointer) != expected {
            Err("war factory vtable mismatch")
        } else {
            Ok(())
        }
    }
    let production = field::<u64>(memory, country.pointer, ccountry::PRODUCTION_STATUS)? as usize;
    authenticate(
        memory,
        base,
        production,
        crate::cproduction_status::VTABLE_RVA,
    )?;
    if field::<u64>(memory, production, crate::cproduction_status::COUNTRY)?
        != country.pointer as u64
    {
        return Err("war factory production owner mismatch");
    }
    let core = country
        .pointer
        .checked_add(ccountry::WAR_OVERVIEW_CORE_STATES)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("war factory core array invalid or unreadable")?;
    let mut total = 0_i32;
    for index in 0..=core.count {
        let state = if index == core.count {
            0
        } else {
            core.element(index, 8)
                .and_then(|p| read::<u64>(memory, p))
                .ok_or("war factory core pointer unreadable")? as usize
        };
        let buildings = if state == 0 {
            let status =
                field::<u64>(memory, country.pointer, ccountry::WAR_OVERVIEW_OFFSITE)? as usize;
            authenticate(memory, base, status, cbuilding_status::VTABLE_RVA)?;
            status.checked_add(cbuilding_status::ACTIVE_BUILDINGS)
        } else {
            authenticate(memory, base, state, cstate::VTABLE_RVA)?;
            let controller = field::<u32>(memory, state, cstate::CONTROLLER)?;
            if controller == 0
                || countries.resolve(controller)?.canonical_slot != country.canonical_slot
            {
                continue;
            }
            state.checked_add(cstate::BUILDING_STATUS + cbuilding_status::ACTIVE_BUILDINGS)
        };
        let buildings = buildings
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("war factory building array invalid or unreadable")?;
        for ordinal in 0..buildings.count {
            let building = buildings
                .element(ordinal, 8)
                .and_then(|p| read::<u64>(memory, p))
                .ok_or("war factory building pointer unreadable")?
                as usize;
            authenticate(memory, base, building, cbuilding::VTABLE_RVA)?;
            let template = field::<u64>(memory, building, cbuilding::TEMPLATE)? as usize;
            authenticate(memory, base, template, cbuilding_template::VTABLE_RVA)?;
            let mut sum = 0_i32;
            for offset in cbuilding_template::WAR_OVERVIEW_PRODUCTION {
                sum = sum.wrapping_add(field::<i32>(memory, template, offset)?);
            }
            if sum > 0 {
                total = total.wrapping_add(i32::from(field::<i16>(
                    memory,
                    building,
                    cbuilding::LEVEL,
                )?));
            }
        }
    }
    Ok(total)
}
