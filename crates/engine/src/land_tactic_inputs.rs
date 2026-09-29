//! Per-combat simulation operands of the possible-tactics weights
//! (141299340 -> 1412A77A0): each side's country, general and field-marshal
//! preferred tactics and the country preference modifier. Tactic definitions,
//! weights and preference factors are content; unlocks are the daily
//! CountryTacticUnlocks census.

use crate::{
    Memory, ccombat_tactic, ccountry, cland_combat, cnull_combat_tactic, cpdx_array::Array, read,
    walk::Outcome,
};

///1412A77A0 reads the general's preferred tactic pointer directly.
pub const LEADER_PREFERRED_TACTIC: usize = 0x10b0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ContextRow {
    /// Country, general and field marshal database ids, separately per side.
    pub preferred_tactic_ids: [[i32; 3]; 2],
    pub country_weights: [ccombat_tactic::WeightRow; 2],
    pub country_weight_present: [u8; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextCapture {
    pub outcome: Outcome,
    pub row: ContextRow,
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("tactic input unreadable")
}

fn pointer(memory: &impl Memory, parent: usize, offset: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, parent, offset).map(|value| value as usize)
}

fn array(
    memory: &impl Memory,
    parent: usize,
    offset: usize,
    stride: usize,
    limit: usize,
) -> Result<Array, &'static str> {
    parent
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, stride))
        .filter(|a| a.count as usize <= limit)
        .ok_or("tactic input array invalid or exceeds safety bound")
}

fn identity(memory: &impl Memory, base: usize, tactic: usize) -> Result<i32, &'static str> {
    let vtable = field::<u64>(memory, tactic, 0)?;
    if base
        .checked_add(cnull_combat_tactic::VTABLE_RVA)
        .map(|p| p as u64)
        == Some(vtable)
    {
        return Ok(0);
    }
    if base
        .checked_add(ccombat_tactic::VTABLE_RVA)
        .map(|p| p as u64)
        != Some(vtable)
    {
        return Err("tactic vtable mismatch");
    }
    let id = field(memory, tactic, ccombat_tactic::ID)?;
    if id < 1 {
        return Err("tactic id invalid");
    }
    Ok(id)
}

fn authentic(
    memory: &impl Memory,
    base: usize,
    object: usize,
    rvas: &[usize],
) -> Result<(), &'static str> {
    let vtable = field::<u64>(memory, object, 0)?;
    if rvas
        .iter()
        .any(|rva| base.checked_add(*rva).map(|p| p as u64) == Some(vtable))
    {
        Ok(())
    } else {
        Err("tactic context vtable mismatch")
    }
}

///1412A77A0 finds the first formation in native theatre order with this
/// commander and a parent. This intentionally does not substitute leader+1048.
fn parent_commander(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    leader: usize,
) -> Result<usize, &'static str> {
    if leader == 0 {
        return Ok(0);
    }
    let owner = countries.resolve(field(memory, leader, 0x120)?)?;
    let theatres = array(memory, owner.pointer, 0x168, 8, 4096)?;
    let mut visited = 0usize;
    for ordinal in 0..theatres.count {
        let theatre = pointer(memory, theatres.data, ordinal as usize * 8)?;
        authentic(memory, base, theatre, &[crate::ctheatre::VTABLE_RVA])?;
        let groups = array(memory, theatre, 0x80, 8, 4096)?;
        visited = visited
            .checked_add(groups.count as usize)
            .filter(|n| *n <= 4096)
            .ok_or("tactic formation census exceeds safety bound")?;
        for index in 0..groups.count {
            let group = pointer(memory, groups.data, index as usize * 8)?;
            authentic(
                memory,
                base,
                group,
                &[
                    crate::corders_group::VTABLE_RVA,
                    crate::carmy_group::VTABLE_RVA,
                ],
            )?;
            if pointer(memory, group, crate::corders_group::LEADER)? != leader {
                continue;
            }
            let parent = pointer(memory, group, crate::corders_group::PARENT)?;
            if parent == 0 {
                continue;
            }
            authentic(memory, base, parent, &[crate::carmy_group::VTABLE_RVA])?;
            let result = pointer(memory, parent, crate::corders_group::LEADER)?;
            if result != 0 {
                authentic(memory, base, result, &[crate::carmy_leader::VTABLE_RVA])?;
            }
            return Ok(result);
        }
    }
    Ok(0)
}

/// One combat's preference operands, in native side order. The combat's
/// commander selections are shared with its other readers
/// (`cland_combat::Commanders`); `members` are the census's participants.
pub fn capture_context(
    memory: &impl Memory,
    base: usize,
    game: usize,
    source: cland_combat::Source,
    members: &cland_combat::Members<'_>,
    selections: &mut cland_combat::Commanders,
) -> ContextCapture {
    let mut row = ContextRow::default();
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game)?;
        let commanders =
            cland_combat::commander_sources(memory, base, source, members, selections)?;
        let sides = [
            pointer(memory, source.pointer, cland_combat::ATTACKER)?,
            pointer(memory, source.pointer, cland_combat::DEFENDER)?,
        ];
        let weight_context = ccombat_tactic::WeightContext::read(memory, base)?;
        for (index, side) in sides.into_iter().enumerate() {
            //141299340 passes the first linked-set node as a CCountryTag*.
            // A side whose last unit has left keeps an empty set until the
            // combat is removed: no country scope, preference or weight.
            let first = pointer(memory, side, crate::cland_combatant::COUNTRIES)?;
            if first != 0 {
                let tag = field::<u32>(memory, first, 0)?;
                if tag == 0 {
                    return Err("tactic side country scope is null");
                }
                let country = countries.resolve(tag)?;
                // 1412A77A0 weighs the selection without its name, so a name
                // of any length is not read here.
                let preferred =
                    ccombat_tactic::preferred_identity(memory, base, country.pointer, country.id);
                if preferred.outcome != (Outcome::Rows { len: 1 }) {
                    return Err("tactic country preference unavailable");
                }
                row.preferred_tactic_ids[index][0] = preferred.row.tactic_id;
                if let Some(weight) =
                    ccombat_tactic::weight(memory, country.pointer, &preferred, weight_context)?
                {
                    row.country_weights[index] = weight;
                    row.country_weight_present[index] = 1;
                }
            }
            let leader = commanders[index].pointer;
            let parent = parent_commander(memory, base, &countries, leader)?;
            for (position, leader) in [leader, parent].into_iter().enumerate() {
                if leader == 0 {
                    continue;
                }
                row.preferred_tactic_ids[index][position + 1] = identity(
                    memory,
                    base,
                    pointer(memory, leader, LEADER_PREFERRED_TACTIC)?,
                )?;
            }
        }
        Ok(())
    })();
    ContextCapture {
        outcome: match result {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(()) => Outcome::Rows { len: 1 },
        },
        row,
    }
}
