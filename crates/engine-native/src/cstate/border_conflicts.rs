use engine_native_memory::{Memory, msvc_tree_node as node, read};
use engine_native_parts::{cland_border_war_combat, cland_border_war_combatant};
use crate::cland_combat;

pub(super) fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<[i32; 2]>,
) -> Result<(), &'static str> {
    let at = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .ok_or("state border conflict address overflow")
    };
    let pointer = |address: usize| {
        read::<u64>(memory, address)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("state border conflict pointer unreadable or null")
    };
    let size = read::<u64>(memory, at(state, super::BORDER_CONFLICT_SIZE)?)
        .ok_or("state border conflict size unreadable")?;
    let head = pointer(at(state, super::BORDER_CONFLICT_TREE)?)?;
    let expected = |rva: engine_sites::Rva| {
        engine_sites::at(base, rva)
            .map(|p| p as u64)
            .ok_or("module address overflow")
    };
    let combat_vtable = expected(cland_border_war_combat::VTABLE_RVA)?;
    let side_vtable = expected(cland_border_war_combatant::VTABLE_RVA)?;
    node::walk_set(memory, head, size, tree_error, |payload| {
        let combat = payload as usize;
        if combat == 0 || read::<u64>(memory, combat) != Some(combat_vtable) {
            return Err("state border conflict CLandBorderWarCombat identity mismatch");
        }
        let mut countries = [0; 2];
        for (country, offset) in countries
            .iter_mut()
            .zip([cland_combat::ATTACKER, cland_combat::DEFENDER])
        {
            let side = pointer(at(combat, offset)?)?;
            if read::<u64>(memory, side) != Some(side_vtable) {
                return Err("state border conflict CLandBorderWarCombatant identity mismatch");
            }
            *country = read::<i32>(memory, at(side, cland_border_war_combatant::COUNTRY)?)
                .ok_or("state border conflict country unreadable")?;
        }
        engine_allocation::reserve(output, 1)
            .map_err(|_| "state border conflict country allocation failed")?;
        output.push(countries);
        Ok(())
    })
}

fn tree_error(error: node::SetError) -> &'static str {
    use node::SetError::*;
    match error {
        NullLink => "state border conflict tree link null",
        UnreadableNode => "state border conflict tree node unreadable",
        InvalidHead => "state border conflict tree head is not its sentinel",
        ForeignSentinel => "state border conflict tree reached a foreign sentinel",
        ExcessNodes => "state border conflict tree exceeds its size",
        MissingNodes => "state border conflict tree shorter than its size",
        Cycle => "state border conflict tree links cycle",
        BudgetOverflow => "state border conflict traversal budget overflow",
    }
}

#[cfg(test)]
mod tests;
