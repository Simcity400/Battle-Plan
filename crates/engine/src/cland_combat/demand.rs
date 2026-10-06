//! Off-thread sizing hints, never recorded facts. The mixed combat roster and
//! side vectors are native array headers (1413CEE30, 1412A8760). The existing
//! detail reader owns every count and native commander selection (14129E190).
use super::{Commanders, DetailArenas, DetailCounts, Members, Reader, Source, read_details};
use crate::{Memory, cgame_state, cpdx_array::Array};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetailDemand {
    pub rows: usize,
    pub census_participants: usize,
    pub arenas: DetailCounts,
}

/// Exact demand at this observation, including border wars and repeated
/// commander occurrences. Empty storage runs the ordinary authenticated detail
/// census; mutable headers are checked again after their selected walks.
/// A fault or concurrent change discards this hint, never a recorded value.
pub fn detail_demand(
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
        let (id, attackers, defenders) = reader.header(pointer, border)?;
        let mut arenas = DetailArenas::new(&mut [], &mut [], &mut [], &mut [], &mut [], &mut []);
        arenas.verify_headers = true;
        read_details(
            memory,
            base,
            Source {
                pointer,
                id,
                attackers,
                defenders,
                provinces,
            },
            &Members::none(),
            &mut Commanders::default(),
            &mut arenas,
        )?;
        if reader.header(pointer, border)? != (id, attackers, defenders)
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
