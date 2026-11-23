
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native_parts::{cfex_group, cland_border_war_combat};
use crate::{cgame_state, cland_combat, cnaval_combatant::{self, Side, field}, cprovince};
use std::collections::HashSet;
type GroupSet = HashSet<usize>;

pub const VTABLE_RVA: Rva = Rva(0x02a5_6d98);
#[cfg(any(test, feature = "test-support"))]
pub const OBSERVED_REFERENCE_TYPE: u32 = 4713;
pub const HANDLE: usize = 0x18;
pub const ATTACKER: usize = 0x28;
pub const DEFENDER: usize = 0x30;
pub const PROVINCE: usize = 0x38;
pub const PARTICIPANTS: usize = 0xf0;
pub const RESULT_FLAG_A: usize = 0x10c;
pub const RESULT_FLAG_B: usize = 0x10d;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub province: u32,
    pub attacker_positioning: i64,
    pub defender_positioning: i64,
    pub attacker_anti_air: i64,
    pub defender_anti_air: i64,
    pub registry_ordinal: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capture {
    pub combats: Outcome,
    pub participants: Outcome,
    pub required_group_capacity: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedCapture {
    pub capture: Capture,
    pub read_ticks: [u64; 2],
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combats: &mut Vec<Row>,
    participants: &mut Vec<cnaval_combatant::Row>,
    groups: &mut Vec<usize>,
) -> Capture {
    walk_timed(
        memory,
        base,
        game_state,
        combats,
        participants,
        groups,
        |_| 0,
    )
    .capture
}

pub fn walk_timed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combats: &mut Vec<Row>,
    participants: &mut Vec<cnaval_combatant::Row>,
    groups: &mut Vec<usize>,
    now: impl FnMut(usize) -> u64,
) -> TimedCapture {
    walk_timed_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(combats),
        engine_native_memory::walk::Buffer::Growing(participants),
        engine_native_memory::walk::Buffer::Growing(groups),
        now,
    )
}

pub fn storage_demand(memory: &impl Memory, base: usize, game_state: usize) -> Capture {
    engine_allocation::advisory(|| storage_demand_advisory(memory, base, game_state))
}

fn storage_demand_advisory(memory: &impl Memory, base: usize, game_state: usize) -> Capture {
    let mut groups = Vec::new();
    walk_timed_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Growing(&mut groups),
        |_| 0,
    )
    .capture
}

fn walk_timed_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut combats: engine_native_memory::walk::Buffer<'_, Row>,
    mut participants: engine_native_memory::walk::Buffer<'_, cnaval_combatant::Row>,
    mut groups: engine_native_memory::walk::Buffer<'_, usize>,
    mut now: impl FnMut(usize) -> u64,
) -> TimedCapture {
    let start = now(0);
    let mut participant_ticks = 0_u64;
    let mut required_group_capacity = 0;
    let result = (|| {
        let registry = game_state
            .checked_add(cgame_state::COMBATS)
            .and_then(|address| address.checked_add(8))
            .ok_or("naval combat registry address overflow")?;
        let registry = Array::read(memory, registry, 8)
            .ok_or("naval combat registry unreadable or invalid")?;
        combats.ensure(registry.count as usize)?;
        let body_vtable = engine_sites::at(base, VTABLE_RVA)
            .ok_or("naval combat vtable overflow")?;
        let land_vtables = [
            engine_sites::at(base, cland_combat::VTABLE_RVA)
                .ok_or("land combat vtable overflow")? as u64,
            engine_sites::at(base, cland_border_war_combat::VTABLE_RVA)
                .ok_or("border combat vtable overflow")? as u64,
        ];
        let mut combat_len = 0;
        let mut combat_unread = 0_u32;
        let mut combat_error = None;
        let mut participant_len = 0;
        let mut participant_unread = 0_u32;
        let mut participant_error = None;
        let mut province_roster = None;
        for index in 0..registry.count {
            let body = pointer(memory, registry, index)?;
            if body == 0 {
                return Err("null combat registry entry");
            }
            let vtable = field::<u64>(memory, body, 0)?;
            if vtable != body_vtable as u64 {
                if land_vtables.contains(&vtable) {
                    continue;
                }
                return Err("unclassified combat registry object");
            }
            let handle = field::<u64>(memory, body, HANDLE)?;
            if handle == 0 {
                return Err("naval combat reference invalid");
            }
            let attacker = usize::try_from(field::<u64>(memory, body, ATTACKER)?)
                .map_err(|_| "naval side pointer overflow")?;
            let defender = usize::try_from(field::<u64>(memory, body, DEFENDER)?)
                .map_err(|_| "naval side pointer overflow")?;
            if attacker == defender {
                return Err("naval combat sides alias");
            }
            let has_combat_capacity = combat_len < combats.len();
            let read_combat = combat_error.is_none() && has_combat_capacity;
            let attacker = cnaval_combatant::side(memory, base, attacker, body, read_combat)?;
            let defender = cnaval_combatant::side(memory, base, defender, body, read_combat)?;
            if combat_error.is_none() && !has_combat_capacity {
                combat_unread = combat_unread
                    .checked_add(1)
                    .ok_or("naval combat count overflow")?;
            } else if read_combat {
                let row = (|| {
                    let (attacker_positioning, attacker_anti_air) = attacker.scalars?;
                    let (defender_positioning, defender_anti_air) = defender.scalars?;
                    let province_pointer = usize::try_from(field::<u64>(memory, body, PROVINCE)?)
                        .map_err(|_| "naval province pointer overflow")?;
                    if field::<u64>(memory, province_pointer, 0)?
                        != engine_sites::at(base, cprovince::VTABLE_RVA)
                            .ok_or("naval province vtable overflow")?
                            as u64
                    {
                        return Err("naval combat province vtable mismatch");
                    }
                    let province = field::<u32>(memory, province_pointer, cprovince::ID)?;
                    if province == 0 {
                        return Err("naval combat province id is zero");
                    }
                    let roster = match province_roster {
                        Some(roster) => roster,
                        None => {
                            let roster = Array::read(
                                memory,
                                game_state
                                    .checked_add(cgame_state::PROVINCES)
                                    .ok_or("naval province registry overflow")?,
                                8,
                            )
                            .ok_or("naval province registry unreadable or invalid")?;
                            province_roster = Some(roster);
                            roster
                        }
                    };
                    if pointer(memory, roster, province)? != province_pointer {
                        return Err("naval province registry pointer mismatch");
                    }
                    Ok(Row {
                        id: handle,
                        province,
                        attacker_positioning,
                        defender_positioning,
                        attacker_anti_air,
                        defender_anti_air,
                        registry_ordinal: index,
                    })
                })();
                match row {
                    Ok(row) => {
                        combats[combat_len] = row;
                        combat_len += 1;
                    }
                    Err(reason) => combat_error = Some(reason),
                }
            }
            if participant_error.is_none() {
                let member_start = now(1);
                let result = members(
                    memory,
                    base,
                    handle,
                    [attacker, defender],
                    &mut participants,
                    &mut participant_len,
                    &mut participant_unread,
                    &mut groups,
                    &mut required_group_capacity,
                );
                participant_ticks =
                    participant_ticks.saturating_add(now(0).saturating_sub(member_start));
                if let Err(reason) = result {
                    participant_error = Some(reason);
                }
            }
        }
        Ok(Capture {
            combats: combat_error.map_or_else(
                || outcome(combat_len, combat_unread),
                |reason| Outcome::Unavailable { reason },
            ),
            participants: participant_error.map_or_else(
                || outcome(participant_len, participant_unread),
                |reason| Outcome::Unavailable { reason },
            ),
            required_group_capacity,
        })
    })();
    let capture = result.unwrap_or_else(|reason| Capture {
        combats: Outcome::Unavailable { reason },
        participants: Outcome::Unavailable { reason },
        required_group_capacity,
    });
    let total_ticks = now(0).saturating_sub(start);
    TimedCapture {
        capture,
        read_ticks: [
            total_ticks.saturating_sub(participant_ticks),
            participant_ticks,
        ],
    }
}

#[allow(clippy::too_many_arguments)]
fn members(
    memory: &impl Memory,
    base: usize,
    combat: u64,
    sides: [Side; 2],
    output: &mut engine_native_memory::walk::Buffer<'_, cnaval_combatant::Row>,
    len: &mut usize,
    unread: &mut u32,
    groups: &mut engine_native_memory::walk::Buffer<'_, usize>,
    required_group_capacity: &mut usize,
) -> Result<(), &'static str> {
    let group_vtable = engine_sites::at(base, cfex_group::VTABLE_RVA)
        .ok_or("naval group vtable overflow")?;
    let mut group_len = 0_usize;
    let mut seen = GroupSet::default();
    for (side_index, side) in sides.into_iter().enumerate() {
        let roots = cnaval_combatant::groups(memory, side)?;
        let capacity = (roots.count as usize)
            .checked_mul(2)
            .ok_or("naval group capacity overflow")?;
        engine_allocation::reserve_set(&mut seen, capacity)
            .map_err(|_| "collector allocation failed")?;
        for index in 0..roots.count {
            let group = pointer(memory, roots, index)?;
            if group == 0 {
                return Err("null naval side group");
            }
            if field::<u64>(memory, group, 0)? != group_vtable as u64 {
                return Err("naval group vtable mismatch");
            }
            if field::<u64>(memory, group, cfex_group::SIDE)? != side.pointer as u64 {
                return Err("naval forward group side mismatch");
            }
            let opponent =
                usize::try_from(field::<u64>(memory, group, cfex_group::OPPONENT_GROUP)?)
                    .map_err(|_| "naval opposing group pointer overflow")?;
            if opponent != 0
                && (field::<u64>(memory, opponent, 0)? != group_vtable as u64
                    || field::<u64>(memory, opponent, cfex_group::SIDE)?
                        != sides[1 - side_index].pointer as u64)
            {
                return Err("naval opposing group identity or side mismatch");
            }
            for group in [group, opponent] {
                if group == 0 || !seen.insert(group) {
                    continue;
                }
                let needed = group_len
                    .checked_add(1)
                    .ok_or("naval group capacity overflow")?;
                *required_group_capacity = (*required_group_capacity).max(needed);
                groups.ensure(needed)?;
                if let Some(slot) = groups.get_mut(group_len) {
                    *slot = group;
                }
                group_len += 1;
            }
        }
    }
    if group_len > groups.len() {
        return Err("naval group scratch capacity exceeded");
    }
    for &group in &groups[..group_len] {
        let owner = usize::try_from(field::<u64>(memory, group, cfex_group::SIDE)?)
            .map_err(|_| "naval group side pointer overflow")?;
        let side = if owner == sides[0].pointer {
            0
        } else if owner == sides[1].pointer {
            1
        } else {
            return Err("naval group belongs to another combat");
        };
        let array = Array::read(
            memory,
            group
                .checked_add(cfex_group::MEMBERS)
                .ok_or("naval group address overflow")?,
            8,
        )
        .ok_or("naval member array unreadable or invalid")?;
        output.ensure(
            len.checked_add(array.count as usize)
                .ok_or("naval member count overflow")?,
        )?;
        for index in 0..array.count {
            let member = pointer(memory, array, index)?;
            if member == 0 {
                return Err("null naval group member");
            }
            if *len == output.len() {
                *unread = unread.checked_add(1).ok_or("naval member count overflow")?;
                continue;
            }
            output[*len] = cnaval_combatant::member(memory, base, member, group, combat, side)?;
            *len += 1;
        }
    }
    Ok(())
}

pub fn pointer(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    let address = array
        .element(index, 8)
        .ok_or("naval array address overflow")?;
    read::<u64>(memory, address)
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("naval array pointer unreadable")
}

pub fn outcome(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}
