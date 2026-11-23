use super::{Commanders, DetailArenas, DetailCounts, read_details};
use crate::cland_combat::{Members, Reader, Source};
use crate::{Memory, cgame_state, cpdx_array::Array};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetailDemand {
    pub rows: usize,
    pub census_participants: usize,
    pub arenas: DetailCounts,
}

pub fn detail_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<DetailDemand, &'static str> {
    crate::allocation::advisory(|| detail_demand_advisory(memory, base, game_state))
}

fn detail_demand_advisory(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<DetailDemand, &'static str> {
    let registry_at = game_state
        .checked_add(cgame_state::COMBATS + 8)
        .ok_or("land detail demand address overflow")?;
    let registry =
        Array::read(memory, registry_at, 8).ok_or("combat registry unreadable or invalid")?;
    let provinces_at = game_state
        .checked_add(cgame_state::PROVINCES)
        .ok_or("land detail demand address overflow")?;
    let provinces =
        Array::read(memory, provinces_at, 8).ok_or("province array unreadable or invalid")?;
    let reader = Reader {
        memory,
        base,
        provinces,
    };
    let mut demand = DetailDemand::default();
    for index in 0..registry.count {
        let pointer = reader.element(registry, index)?;
        let Some(border) = reader.class(pointer)? else {
            continue;
        };
        let (id, attackers, defenders, sides) = reader.header_with_sides(pointer, border)?;
        let mut arenas = DetailArenas::count();
        arenas.verify_headers = true;
        read_details(
            memory,
            base,
            Source {
                pointer,
                id,
                sides,
                attackers,
                defenders,
                provinces,
            },
            &Members::none(),
            &mut Commanders::default(),
            &mut arenas,
        )?;
        if reader.header_with_sides(pointer, border)? != (id, attackers, defenders, sides)
            || reader.element(registry, index)? != pointer
        {
            return Err("land detail demand combat changed");
        }
        add(&mut demand.rows, 1)?;
        for units in [attackers, defenders] {
            add(&mut demand.census_participants, units.count as usize)?;
        }
        for (total, count) in [
            (
                &mut demand.arenas.participants,
                arenas.required.participants,
            ),
            (&mut demand.arenas.air, arenas.required.air),
            (&mut demand.arenas.text, arenas.required.text),
            (&mut demand.arenas.traits, arenas.required.traits),
            (&mut demand.arenas.timed, arenas.required.timed),
            (&mut demand.arenas.learning, arenas.required.learning),
        ] {
            add(total, count)?;
        }
    }
    if Array::read(memory, registry_at, 8) != Some(registry)
        || Array::read(memory, provinces_at, 8) != Some(provinces)
    {
        return Err("land detail demand registry changed");
    }
    Ok(demand)
}

fn add(total: &mut usize, value: usize) -> Result<(), &'static str> {
    *total = total
        .checked_add(value)
        .ok_or("land detail demand overflow")?;
    Ok(())
}
