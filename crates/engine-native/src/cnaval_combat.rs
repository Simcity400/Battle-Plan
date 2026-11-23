
use engine_sites::Rva;
use crate::{
    Memory, cfex_group, cfex_member, cgame_state, cland_border_war_combat, cland_combat,
    cnaval_combatant,
    cnaval_combatant::{Side, field},
    cpdx_array::Array,
    cprovince, read,
    walk::Outcome,
};
use std::collections::{HashMap, HashSet};

type GroupOrdinals = HashMap<usize, Option<u32>>;
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DetailsRow {
    pub combat: u64,
    pub native_combat: usize,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    pub progress: i64,
    pub sides: [cnaval_combatant::DetailsRow; 2],
    pub custom_sea: Option<[usize; 2]>,
    pub custom_sea_error: Option<&'static str>,
}

#[allow(
    clippy::too_many_arguments,
    reason = "naval combat walker threads combat context and shared detail buffers"
)]
pub fn walk_details_timed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut details: crate::walk::Buffer<'_, DetailsRow>,
    ships: &mut Vec<crate::cfex_member::DetailsRow>,
    groups: &mut Vec<usize>,
    buffers: &mut crate::cfex_member::DetailBuffers<'_>,
    mut now: impl FnMut(usize) -> u64,
) -> TimedCapture {
    let start = now(0);
    let mut member_ticks = 0u64;
    buffers.lengths = [0; 3];
    buffers.required = [0; 3];
    buffers.planes_len = 0;
    buffers.planes_required = 0;
    buffers.plane_hit_lengths = [0; 2];
    buffers.plane_hit_required = [0; 2];
    buffers.weapon_lengths = [0; 2];
    buffers.weapon_required = [0; 2];
    buffers.plane_kills_len = 0;
    buffers.plane_kills_required = 0;
    buffers.commander_inputs_len = 0;
    buffers.commander_inputs_required = 0;
    buffers.commander_timed_traits_len = 0;
    buffers.side_units_len = 0;
    buffers.side_units_required = 0;
    buffers.summary_countries_len = 0;
    let mut required_group_capacity = 0;
    let result = (|| {
        let at = game_state
            .checked_add(cgame_state::COMBATS + 8)
            .ok_or("naval details registry overflow")?;
        let registry = Array::read(memory, at, 8).ok_or("naval details registry invalid")?;
        details.ensure(registry.count as usize)?;
        let mut lengths = [0usize; 2];
        let mut unread = [0u32; 2];
        let mut errors = [None; 2];
        for index in 0..registry.count {
            let body = pointer(memory, registry, index)?;
            let vtable = field::<u64>(memory, body, 0)?;
            if vtable
                != engine_sites::at(base, VTABLE_RVA)
                    .ok_or("naval details module overflow")? as u64
            {
                if [
                    cland_combat::VTABLE_RVA,
                    cland_border_war_combat::VTABLE_RVA,
                ]
                .into_iter()
                .any(|key| engine_sites::at(base, key).is_some_and(|p| p as u64 == vtable))
                {
                    continue;
                }
                return Err("naval details unclassified combat");
            }
            let combat = field::<u64>(memory, body, HANDLE)?;
            if combat == 0 {
                return Err("naval details null reference");
            }
            let roots = [
                field::<u64>(memory, body, ATTACKER)? as usize,
                field::<u64>(memory, body, DEFENDER)? as usize,
            ];
            if roots[0] == roots[1] {
                return Err("naval details side alias");
            }
            let sides = [
                cnaval_combatant::side(memory, base, roots[0], body, false)?,
                cnaval_combatant::side(memory, base, roots[1], body, false)?,
            ];
            if errors[0].is_none() {
                if lengths[0] == details.len() {
                    unread[0] += 1;
                } else {
                    let row = (|| {
                        let province = field::<u64>(memory, body, PROVINCE)? as usize;
                        if field::<u64>(memory, province, 0)?
                            != engine_sites::at(base, cprovince::VTABLE_RVA)
                                .ok_or("naval details province overflow")?
                                as u64
                        {
                            return Err("naval details province identity mismatch");
                        }
                        let mut arenas = cnaval_combatant::SideArenas {
                            countries: &mut *buffers.summary_countries,
                            countries_len: &mut buffers.summary_countries_len,
                            units: buffers.side_units.suffix(0),
                            units_len: &mut buffers.side_units_len,
                            units_required: &mut buffers.side_units_required,
                            damage_sources: buffers.damage_sources.suffix(0),
                            damage_sources_len: &mut buffers.damage_sources_len,
                            damage_sources_required: &mut buffers.damage_sources_required,
                        };
                        let sides_read = [
                            cnaval_combatant::details(
                                memory,
                                base,
                                game_state,
                                body,
                                [&sides[0], &sides[1]],
                                &mut arenas,
                            )?,
                            cnaval_combatant::details(
                                memory,
                                base,
                                game_state,
                                body,
                                [&sides[1], &sides[0]],
                                &mut arenas,
                            )?,
                        ];
                        let custom_sea = custom_sea(memory, base, game_state, province, buffers);
                        let mut row = DetailsRow {
                            combat,
                            native_combat: body,
                            province: field(memory, province, cprovince::ID)?,
                            day: field(memory, body, cland_combat::DAY)?,
                            duration: field(memory, body, cland_combat::DURATION)?,
                            progress: field(memory, body, 0x118)?,
                            sides: sides_read,
                            custom_sea: custom_sea.as_ref().ok().copied(),
                            custom_sea_error: custom_sea.err(),
                        };
                        for (side, value) in row.sides.iter_mut().enumerate() {
                            value.commander_inputs =
                                commander_input(memory, base, value.commander, buffers)?;
                            value.planes = read_planes(
                                memory,
                                base,
                                game_state,
                                body,
                                roots,
                                side,
                                groups,
                                buffers,
                                &mut required_group_capacity,
                            )?;
                        }
                        Ok(row)
                    })();
                    match row {
                        Ok(row) => {
                            details[lengths[0]] = row;
                            lengths[0] += 1;
                        }
                        Err(PLANE_CAPACITY | crate::cfex_member::DETAIL_ARENA_CAPACITY) => {
                            unread[0] += 1
                        }
                        Err(reason) => errors[0] = Some(reason),
                    }
                }
            }
            if errors[1].is_none() {
                let member_start = now(1);
                let result = (|| {
                    let mut group_count = 0usize;
                    let mut ordinals: [GroupOrdinals; 2] = Default::default();
                    let mut taken = GroupSet::default();
                    for (side_index, side) in sides.into_iter().enumerate() {
                        let array = cnaval_combatant::groups(memory, side)?;
                        let capacity = (array.count as usize)
                            .checked_mul(2)
                            .ok_or("naval details group capacity overflow")?;
                        crate::allocation::reserve_map(
                            &mut ordinals[side_index],
                            array.count as usize,
                        )
                        .map_err(|_| "collector allocation failed")?;
                        crate::allocation::reserve_set(&mut taken, capacity)
                            .map_err(|_| "collector allocation failed")?;
                        for index in 0..array.count {
                            let group = pointer(memory, array, index)?;
                            ordinals[side_index]
                                .entry(group)
                                .and_modify(|ordinal| *ordinal = None)
                                .or_insert(Some(index));
                            let other =
                                field::<u64>(memory, group, cfex_group::OPPONENT_GROUP)? as usize;
                            for group in [group, other] {
                                if group == 0 || taken.contains(&group) {
                                    continue;
                                }
                                if field::<u64>(memory, group, 0)?
                                    != engine_sites::at(base, cfex_group::VTABLE_RVA)
                                        .ok_or("naval details group module overflow")?
                                        as u64
                                {
                                    return Err("naval details group vtable mismatch");
                                }
                                let owner = field::<u64>(memory, group, cfex_group::SIDE)? as usize;
                                if !roots.contains(&owner) {
                                    return Err("naval details group belongs to another combat");
                                }
                                required_group_capacity =
                                    required_group_capacity.max(group_count + 1);
                                if group_count == groups.len() {
                                    if groups.len() == groups.capacity() {
                                        crate::allocation::reserve(groups, 1)
                                            .map_err(|_| "collector allocation failed")?;
                                    }
                                    groups.push(0);
                                }
                                groups[group_count] = group;
                                group_count += 1;
                                taken.insert(group);
                            }
                        }
                    }
                    for &group in &groups[..group_count] {
                        let owner = field::<u64>(memory, group, cfex_group::SIDE)? as usize;
                        let side = u8::from(owner == roots[1]);
                        let group_index = match ordinals[usize::from(side)].get(&group) {
                            Some(Some(ordinal)) => *ordinal,
                            Some(None) => return Err("naval duplicate forward group"),
                            None => u32::MAX,
                        };
                        for (member_source, member_offset) in [(0, cfex_group::MEMBERS), (1, 0x70)]
                        {
                            let array = Array::read(memory, group + member_offset, 8)
                                .ok_or("naval details member array invalid")?;
                            let required = lengths[1]
                                .checked_add(array.count as usize)
                                .ok_or("naval details member capacity overflow")?;
                            crate::allocation::reserve_push_batch(
                                ships,
                                required.saturating_sub(ships.len()),
                            )
                            .map_err(|_| "collector allocation failed")?;
                            for index in 0..array.count {
                                let member = pointer(memory, array, index)?;
                                let mut row = crate::cfex_member::details(
                                    memory, base, member, group, combat, side, game_state, buffers,
                                )?;
                                row.member_source = member_source;
                                row.group_index = group_index;
                                row.member_index = index;
                                if lengths[1] == ships.len() {
                                    ships.push(row);
                                } else {
                                    ships[lengths[1]] = row;
                                }
                                lengths[1] += 1;
                            }
                        }
                    }
                    Ok(())
                })();
                member_ticks = member_ticks.saturating_add(now(0).saturating_sub(member_start));
                if let Err(reason) = result {
                    errors[1] = Some(reason);
                }
            }
        }
        Ok(Capture {
            combats: errors[0].map_or_else(
                || outcome(lengths[0], unread[0]),
                |reason| Outcome::Unavailable { reason },
            ),
            participants: errors[1].map_or_else(
                || outcome(lengths[1], unread[1]),
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
    let total = now(0).saturating_sub(start);
    TimedCapture {
        capture,
        read_ticks: [total.saturating_sub(member_ticks), member_ticks],
    }
}

fn custom_sea(
    memory: &impl Memory,
    base: usize,
    state: usize,
    province: usize,
    buffers: &mut cfex_member::DetailBuffers<'_>,
) -> Result<[usize; 2], &'static str> {
    let region = field::<u64>(memory, province, cprovince::STRATEGIC_REGION)? as usize;
    if region == 0 {
        return Ok([buffers.custom_sea_len, 0]);
    }
    let id = crate::cstrategic_region::id(memory, base, region)?
        .ok_or("naval strategic region absent")?;
    let template = field::<u64>(memory, region, crate::cstrategic_region::TEMPLATE)? as usize;
    if field::<u64>(memory, template, 0)?
        != engine_sites::at(base, crate::cstrategic_region::TEMPLATE_VTABLE_RVA)
            .ok_or("naval strategic template module overflow")? as u64
    {
        return Err("naval strategic template identity mismatch");
    }
    let capacity = field::<i32>(
        memory,
        template,
        crate::cstrategic_region::TEMPLATE_MODIFIERS_CAPACITY,
    )?;
    let count = field::<i32>(
        memory,
        template,
        crate::cstrategic_region::TEMPLATE_MODIFIERS_COUNT,
    )?;
    if capacity < 0 || count < 0 || count > capacity {
        return Err("naval custom sea modifier vector invalid");
    }
    let count = count as u32;
    if count == 0 {
        return Ok([buffers.custom_sea_len, 0]);
    }
    let start = buffers.custom_sea_len;
    let end = start
        .checked_add(count as usize)
        .ok_or("naval custom sea arena length overflow")?;
    buffers.custom_sea_required = buffers.custom_sea_required.max(end);
    buffers.custom_sea.ensure(end)?;
    let output = buffers
        .custom_sea
        .get_mut(start..end)
        .ok_or("naval custom sea arena capacity exceeded")?;
    crate::cweather_manager::Reader::new(memory, base, state)?
        .custom_sea_into(id as i32, output)?;
    buffers.custom_sea_len = end;
    Ok([start, count as usize])
}

const PLANE_CAPACITY: &str = "naval combat plane arena capacity exceeded";

fn commander_input(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    buffers: &mut crate::cfex_member::DetailBuffers<'_>,
) -> Result<[usize; 2], &'static str> {
    if handle == 0 {
        return Ok([0; 2]);
    }
    if let Some(index) = buffers.commander_inputs[..buffers.commander_inputs_len]
        .iter()
        .position(|row| row.commander == handle)
    {
        return Ok([index, 1]);
    }
    buffers.commander_inputs_required = buffers
        .commander_inputs_required
        .checked_add(1)
        .ok_or("naval commander demand overflow")?;
    let index = buffers.commander_inputs_len;
    if !buffers.commander_inputs.ensure(index + 1)? {
        return Err(crate::cfex_member::DETAIL_ARENA_CAPACITY);
    }
    buffers.commander_inputs[index] = crate::naval_commander_inputs::capture(
        memory,
        base,
        handle,
        buffers.commander_timed_traits,
        &mut buffers.commander_timed_traits_len,
    )?;
    buffers.commander_inputs_len += 1;
    Ok([index, 1])
}

#[allow(
    clippy::too_many_arguments,
    reason = "naval plane reader threads combat context and shared detail buffers"
)]
fn read_planes(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combat: usize,
    roots: [usize; 2],
    side: usize,
    groups: &mut Vec<usize>,
    buffers: &mut crate::cfex_member::DetailBuffers<'_>,
    required_groups: &mut usize,
) -> Result<[usize; 2], &'static str> {
    let start = buffers.planes_len;
    let mut count = 0usize;
    let mut fits = true;
    let mut taken = GroupSet::default();
    for root in [roots[side], roots[1 - side]] {
        let array = Array::read(memory, root + cnaval_combatant::GROUPS, 8)
            .ok_or("naval plane group array invalid")?;
        let capacity = (array.count as usize)
            .checked_mul(2)
            .ok_or("naval plane group capacity overflow")?;
        crate::allocation::reserve_set(&mut taken, capacity)
            .map_err(|_| "collector allocation failed")?;
        for i in 0..array.count {
            let group = pointer(memory, array, i)?;
            let other = field::<u64>(memory, group, cfex_group::OPPONENT_GROUP)? as usize;
            for group in [group, other] {
                if group == 0 || taken.contains(&group) {
                    continue;
                }
                if field::<u64>(memory, group, 0)?
                    != engine_sites::at(base, cfex_group::VTABLE_RVA)
                        .ok_or("naval plane group module overflow")? as u64
                {
                    return Err("naval plane group identity mismatch");
                }
                let owner = field::<u64>(memory, group, cfex_group::SIDE)? as usize;
                if !roots.contains(&owner) {
                    return Err("naval plane group belongs to another combat");
                }
                if owner != roots[side] {
                    continue;
                }
                *required_groups = (*required_groups).max(count + 1);
                if count == groups.len() {
                    if groups.len() == groups.capacity() {
                        crate::allocation::reserve(groups, 1)
                            .map_err(|_| "collector allocation failed")?;
                    }
                    groups.push(0);
                }
                groups[count] = group;
                count += 1;
                taken.insert(group);
            }
        }
    }
    for (index, &group) in groups[..count].iter().enumerate() {
        for (source, offset) in [(0, 0x58), (1, 0x88)] {
            fits &= append_planes(
                memory,
                base,
                game_state,
                combat,
                group + offset,
                PlaneOwner::Group(group),
                source,
                index as u32,
                buffers,
            )?;
        }
    }
    fits &= append_planes(
        memory,
        base,
        game_state,
        combat,
        roots[side] + 0x100,
        PlaneOwner::Side(&taken),
        2,
        0,
        buffers,
    )?;
    if !fits {
        return Err(PLANE_CAPACITY);
    }
    Ok([start, buffers.planes_len - start])
}

#[derive(Clone, Copy)]
enum PlaneOwner<'a> {
    Group(usize),
    Side(&'a GroupSet),
}

#[allow(
    clippy::too_many_arguments,
    reason = "naval plane appender threads combat context and shared detail buffers"
)]
fn append_planes(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    combat: usize,
    address: usize,
    owner: PlaneOwner<'_>,
    source: u8,
    group_index: u32,
    buffers: &mut crate::cfex_member::DetailBuffers<'_>,
) -> Result<bool, &'static str> {
    let array = Array::read(memory, address, 8).ok_or("naval plane array invalid")?;
    buffers.planes_required = buffers
        .planes_required
        .checked_add(array.count as usize)
        .ok_or("naval plane required capacity overflow")?;
    let end = buffers
        .planes_len
        .checked_add(array.count as usize)
        .ok_or("naval plane length overflow")?;
    let fits = buffers.planes.ensure(end)?;
    if fits {
        for index in 0..array.count {
            let pointer = pointer(memory, array, index)?;
            let group = match owner {
                PlaneOwner::Group(group) => group,
                PlaneOwner::Side(groups) => {
                    let group = field::<u64>(memory, pointer, 8)? as usize;
                    if group != 0 && !groups.contains(&group) {
                        return Err("naval side plane owned by another side's group");
                    }
                    group
                }
            };
            let mut row = crate::cfex_air::read_buffer(
                memory,
                base,
                pointer,
                group,
                buffers.wing_types.suffix(0),
                &mut buffers.wing_types_len,
                &mut buffers.wing_types_required,
                buffers.plane_wings.suffix(0),
                &mut buffers.plane_wings_len,
                &mut buffers.plane_wings_required,
            )?;
            crate::cfex_air::read_hits(memory, base, pointer, &mut row, buffers)?;
            row.efficiency = crate::naval_air_efficiency_inputs::read(
                memory, base, game_state, pointer, group, combat,
            )?;
            row.kills =
                crate::naval_air_efficiency_inputs::append_kills(memory, base, pointer, buffers)?;
            row.source = source;
            row.group_index = group_index;
            row.index = index;
            buffers.planes[buffers.planes_len] = row;
            buffers.planes_len += 1;
        }
    }
    Ok(fits)
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
        crate::walk::Buffer::Growing(combats),
        crate::walk::Buffer::Growing(participants),
        crate::walk::Buffer::Growing(groups),
        now,
    )
}

pub fn storage_demand(memory: &impl Memory, base: usize, game_state: usize) -> Capture {
    crate::allocation::advisory(|| storage_demand_advisory(memory, base, game_state))
}

fn storage_demand_advisory(memory: &impl Memory, base: usize, game_state: usize) -> Capture {
    let mut groups = Vec::new();
    walk_timed_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Count,
        crate::walk::Buffer::Count,
        crate::walk::Buffer::Growing(&mut groups),
        |_| 0,
    )
    .capture
}

fn walk_timed_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut combats: crate::walk::Buffer<'_, Row>,
    mut participants: crate::walk::Buffer<'_, cnaval_combatant::Row>,
    mut groups: crate::walk::Buffer<'_, usize>,
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

#[allow(
    clippy::too_many_arguments,
    reason = "naval combat participant walker threads output buffers and counters"
)]
fn members(
    memory: &impl Memory,
    base: usize,
    combat: u64,
    sides: [Side; 2],
    output: &mut crate::walk::Buffer<'_, cnaval_combatant::Row>,
    len: &mut usize,
    unread: &mut u32,
    groups: &mut crate::walk::Buffer<'_, usize>,
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
        crate::allocation::reserve_set(&mut seen, capacity)
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

fn pointer(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    let address = array
        .element(index, 8)
        .ok_or("naval array address overflow")?;
    read::<u64>(memory, address)
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("naval array pointer unreadable")
}

fn outcome(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[cfg(test)]
mod tests;
