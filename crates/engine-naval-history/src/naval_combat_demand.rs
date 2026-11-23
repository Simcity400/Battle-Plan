
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use std::cell::RefCell;
use std::collections::HashSet;

fn key(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("naval demand pointer overflow")
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .ok_or("naval demand pointer overflow")
}
fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("naval demand field unreadable")
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub ships: usize,
    pub damage: usize,
    pub air_hits: usize,
    pub naval_hits: usize,
    pub plane_kills: usize,
    pub commander_timed_traits: usize,
    pub custom_sea: usize,
    pub summary_countries: usize,
    pub dense_bases: usize,
    pub dense_wings: usize,
    pub weapon_sources: usize,
    pub weapon_terms: usize,
    pub groups: usize,
    pub planes: usize,
    pub commanders: usize,
    pub side_units: usize,
    pub damage_sources: usize,
    pub wing_types: usize,
    pub plane_wings: usize,
    pub plane_air_hits: usize,
    pub plane_naval_hits: usize,
}

fn add(total: &mut usize, amount: usize) -> Result<(), &'static str> {
    *total = total.checked_add(amount).ok_or("naval demand overflow")?;
    Ok(())
}

fn array(memory: &impl Memory, pointer: usize, offset: usize) -> Result<Array, &'static str> {
    Array::read(memory, at(pointer, offset)?, 8).ok_or("naval demand array invalid")
}

fn pointer(memory: &impl Memory, pointer: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, pointer, offset)?)
        .map_err(|_| "naval demand pointer overflow")
}

fn element(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    let address = array
        .element(index, 8)
        .ok_or("naval demand element overflow")?;
    pointer(memory, address, 0)
}

fn insert(set: &mut HashSet<usize>, address: usize) -> Result<bool, &'static str> {
    if set.contains(&address) {
        return Ok(false);
    }
    set.try_reserve(1)
        .map_err(|_| "naval sizing allocation skipped")?;
    Ok(set.insert(address))
}

struct Headers<'a, M> {
    memory: &'a M,
    observed: RefCell<Vec<(usize, [u8; 16])>>,
}

impl<M: Memory> Memory for Headers<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if !self.memory.copy(address, output) {
            return false;
        }
        if let Ok(header) = <[u8; 16]>::try_from(&*output) {
            let mut observed = self.observed.borrow_mut();
            if observed.try_reserve(1).is_err() {
                return false;
            }
            observed.push((address, header));
        }
        true
    }
}

impl<M: Memory> Headers<'_, M> {
    fn validate(&self) -> Result<(), &'static str> {
        for &(address, header) in &*self.observed.borrow() {
            if engine_native_memory::read::<[u8; 16]>(self.memory, address) != Some(header) {
                return Err("naval demand selecting header changed");
            }
        }
        Ok(())
    }
}

pub fn demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Demand, &'static str> {
    crate::allocation::advisory(|| demand_advisory(memory, base, game_state))
}

fn demand_advisory(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Demand, &'static str> {
    use engine_native_parts::cfex_group;
    use engine_native::{cgame_state, cnaval_combat, cnaval_combatant};
    let headers = Headers {
        memory,
        observed: RefCell::default(),
    };
    let memory = &headers;
    let roster_at = at(game_state, cgame_state::COMBATS + 8)?;
    let roster = Array::read(memory, roster_at, 8).ok_or("naval demand registry invalid")?;
    let mut demand = Demand::default();
    let mut groups = HashSet::new();
    let mut doctrine_keys = engine_native::cship::modifier_sources::DoctrineKeys::default();
    for index in 0..roster.count {
        let combat = element(memory, roster, index)?;
        if field::<u64>(memory, combat, 0)? != key(base, cnaval_combat::VTABLE_RVA)? as u64 {
            continue;
        }
        let sides = [
            pointer(memory, combat, cnaval_combat::ATTACKER)?,
            pointer(memory, combat, cnaval_combat::DEFENDER)?,
        ];
        let handle: u64 = field(memory, combat, cnaval_combat::HANDLE)?;
        add(&mut demand.rows, 1)?;
        let province = pointer(memory, combat, cnaval_combat::PROVINCE)?;
        let region = pointer(memory, province, engine_native::cprovince::STRATEGIC_REGION)?;
        if region != 0 {
            let template = pointer(memory, region, engine_native_parts::cstrategic_region::TEMPLATE)?;
            let capacity = field::<i32>(
                memory,
                template,
                engine_native_parts::cstrategic_region::TEMPLATE_MODIFIERS_CAPACITY,
            )?;
            let count = field::<i32>(
                memory,
                template,
                engine_native_parts::cstrategic_region::TEMPLATE_MODIFIERS_COUNT,
            )?;
            if count < 0 || capacity < count {
                return Err("naval demand custom sea count invalid");
            }
            add(&mut demand.custom_sea, count as usize)?;
        }
        for &side in &sides {
            engine_native::cnaval_combatant::side(memory, base, side, combat, false)?;
            add(
                &mut demand.side_units,
                array(memory, side, cnaval_combatant::UNITS)?.count as usize,
            )?;
            add(
                &mut demand.damage_sources,
                engine_native_naval::naval_side_details::damage_source_demand(memory, side)?,
            )?;
            for offset in [
                engine_native_parts::naval_side_layout::COUNTRY_COUNT,
                engine_native_naval::naval_combat_summary_inputs::SECONDARY_COUNTRIES + 0x10,
            ] {
                let count = usize::try_from(field::<i32>(memory, side, offset)?)
                    .map_err(|_| "naval demand country count negative")?;
                add(&mut demand.summary_countries, count)?;
            }
            let commander = pointer(memory, side, cnaval_combatant::COMMANDER)?;
            if commander != 0 {
                add(&mut demand.commanders, 1)?;
                let timed = Array::read(
                    memory,
                    at(commander, engine_native::cunit_leader::TIMED_TRAIT_DATA)?,
                    engine_native::cunit_leader::TIMED_TRAIT_STRIDE,
                )
                .ok_or("naval demand commander timed traits invalid")?;
                add(&mut demand.commander_timed_traits, timed.count as usize)?;
            }
        }
        groups.clear();
        for &side in &sides {
            let forward = array(memory, side, cnaval_combatant::GROUPS)?;
            for index in 0..forward.count {
                let group = element(memory, forward, index)?;
                let opponent = pointer(memory, group, cfex_group::OPPONENT_GROUP)?;
                for group in [group, opponent].into_iter().filter(|&group| group != 0) {
                    if field::<u64>(memory, group, 0)? != key(base, cfex_group::VTABLE_RVA)? as u64
                        || !sides.contains(&pointer(memory, group, cfex_group::SIDE)?)
                    {
                        return Err("naval demand group identity mismatch");
                    }
                    insert(&mut groups, group)?;
                }
            }
            count_planes(memory, base, array(memory, side, 0x100)?, &mut demand)?;
            if array(memory, side, cnaval_combatant::GROUPS)? != forward {
                return Err("naval demand group roster changed");
            }
        }
        demand.groups = demand.groups.max(groups.len());
        for &group in &groups {
            let side = pointer(memory, group, cfex_group::SIDE)?;
            let active_air = array(memory, group, 0x58)?;
            count_planes(memory, base, active_air, &mut demand)?;
            count_planes(memory, base, array(memory, group, 0x88)?, &mut demand)?;
            for offset in [cfex_group::MEMBERS, 0x70] {
                let members = array(memory, group, offset)?;
                add(&mut demand.ships, members.count as usize)?;
                for index in 0..members.count {
                    let member = element(memory, members, index)?;
                    for (offset, stride, lane) in [
                        (engine_native::cfex_member::DAMAGE_ENTRIES, 0x20, 0),
                        (0x150, 8, 1),
                        (0x168, 8, 1),
                        (0x180, 8, 2),
                    ] {
                        let count = Array::read(memory, at(member, offset)?, stride)
                            .ok_or("naval demand member arena invalid")?
                            .count as usize;
                        add(
                            match lane {
                                0 => &mut demand.damage,
                                1 => &mut demand.air_hits,
                                _ => &mut demand.naval_hits,
                            },
                            count,
                        )?;
                    }
                    let handle = field(memory, member, engine_native::cfex_member::SHIP_HANDLE)?;
                    if let Some(ship) = engine_native_parts::creference_registry::resolve(memory, base, handle)? {
                        use engine_native_naval::naval_ship_modifier_inputs::visit_sources;
                        let context = visit_sources::<false>(
                            memory,
                            base,
                            game_state,
                            ship,
                            side,
                            &mut doctrine_keys,
                            None,
                            &mut |_source, accumulator| {
                                add(&mut demand.weapon_sources, 1)?;
                                if let Some(root) = accumulator {
                                    let entries = Array::read(memory, root, 16)
                                        .ok_or("naval demand weapon terms invalid")?;
                                    add(&mut demand.weapon_terms, entries.count as usize)?;
                                }
                                Ok(())
                            },
                        )?;
                        count_base(memory, base, context.air_base, &mut demand)?;
                    }
                }
                if array(memory, group, offset)? != members {
                    return Err("naval demand member roster changed");
                }
            }
        }
        if pointer(memory, combat, cnaval_combat::ATTACKER)? != sides[0]
            || pointer(memory, combat, cnaval_combat::DEFENDER)? != sides[1]
            || field::<u64>(memory, combat, cnaval_combat::HANDLE)? != handle
        {
            return Err("naval demand combat changed");
        }
    }
    if Array::read(memory, roster_at, 8) != Some(roster) {
        return Err("naval demand registry changed");
    }
    headers.validate()?;
    Ok(demand)
}

fn count_planes(
    memory: &impl Memory,
    base: usize,
    planes: Array,
    demand: &mut Demand,
) -> Result<(), &'static str> {
    add(&mut demand.planes, planes.count as usize)?;
    for index in 0..planes.count {
        let plane = element(memory, planes, index)?;
        let active = array(memory, plane, 0x28)?.count as usize;
        let all_wings = array(memory, plane, 0x40)?;
        let all = all_wings.count as usize;
        for index in 0..all_wings.count {
            let handle = field(
                memory,
                all_wings
                    .element(index, 8)
                    .ok_or("naval demand wing address overflow")?,
                0,
            )?;
            if let Some(wing) = engine_native_naval::naval_air_efficiency_inputs::wing(memory, base, handle)? {
                let kills = Array::read(memory, at(wing, 0x9f8)?, 16)
                    .ok_or("naval demand wing kills invalid")?;
                add(&mut demand.plane_kills, kills.count as usize)?;
            }
        }
        count_base(memory, base, field(memory, plane, 0x1c)?, demand)?;
        add(&mut demand.wing_types, all)?;
        add(&mut demand.plane_wings, active)?;
        add(&mut demand.plane_wings, all)?;
        add(
            &mut demand.plane_naval_hits,
            array(memory, plane, 0x100)?.count as usize,
        )?;
        for offset in [0x118, 0x130] {
            add(
                &mut demand.plane_air_hits,
                array(memory, plane, offset)?.count as usize,
            )?;
        }
    }
    Ok(())
}

fn count_base(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    demand: &mut Demand,
) -> Result<(), &'static str> {
    if handle == 0 {
        return Ok(());
    }
    let Some(pointer) = engine_native_parts::creference_registry::resolve(memory, base, handle)? else {
        return Ok(());
    };
    if field::<u64>(memory, pointer, 0)? != key(base, engine_native_parts::cair_base_layout::VTABLE_RVA)? as u64
        || field::<u64>(memory, pointer, engine_native_parts::cair_base_layout::HANDLE)? != handle
    {
        return Err("naval demand dense base identity mismatch");
    }
    add(&mut demand.dense_bases, 1)?;
    let containers = array(memory, pointer, engine_native_air::cair_base::REGISTERED_CONTAINERS)?;
    for index in 0..containers.count {
        let container = element(memory, containers, index)?;
        let pools = array(memory, container, engine_native_air::ccountry_air_container::POOLS)?;
        for index in 0..pools.count {
            let pool = element(memory, pools, index)?;
            add(
                &mut demand.dense_wings,
                array(memory, pool, engine_native_parts::cair_wing_pool::WINGS)?.count as usize,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::modifier_definition;
    use std::cell::Cell;

    struct Leaves {
        count: u32,
        torn: bool,
        invalid_name: bool,
        visits: Cell<usize>,
    }

    impl Memory for Leaves {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let mut bytes = [0_u8; 32];
            if address == modifier_definition::TABLE_RVA.0 && output.len() == 8 {
                bytes[..8].copy_from_slice(&0x3000_0000_u64.to_le_bytes());
            } else if (0x3000_0000..0x3004_0000).contains(&address) {
            } else if address == 0x100 && output.len() == 16 {
            } else if address == 0x118 && output.len() == 16 {
                let visits = self.visits.get();
                self.visits.set(visits + 1);
                let count = self.count + u32::from(self.torn && visits != 0);
                bytes[..8].copy_from_slice(&0x1000_u64.to_le_bytes());
                bytes[8..12].copy_from_slice(&count.to_le_bytes());
                bytes[12..16].copy_from_slice(&count.to_le_bytes());
            } else if (0x1000..0x1000 + self.count as usize * 8).contains(&address)
                && output.len() == 8
            {
                let index = (address - 0x1000) / 8;
                let child = 0x100_0000 + index * 0x100 - 0x10;
                bytes[..8].copy_from_slice(&(child as u64).to_le_bytes());
            } else if let Some(offset) = address.checked_sub(0x100_0000)
                && offset / 0x100 < self.count as usize
            {
                match (offset % 0x100, output.len()) {
                    (0, 16) => {
                        let entry = 0x200_0000 + offset / 0x100 * 16;
                        bytes[..8].copy_from_slice(&(entry as u64).to_le_bytes());
                        bytes[8..16].copy_from_slice(&0x1_0000_0001_u64.to_le_bytes());
                    }
                    (0x18, 16) => (),
                    (0x48, 32) => {
                        bytes[..8].copy_from_slice(b"leafname");
                        let length = if self.invalid_name { u64::MAX } else { 8 };
                        bytes[16..24].copy_from_slice(&length.to_le_bytes());
                        bytes[24..].copy_from_slice(&15_u64.to_le_bytes());
                    }
                    _ => return false,
                }
            } else if (0x200_0000..0x200_0000 + self.count as usize * 16).contains(&address)
                && output.len() == 16
            {
                bytes[..4].copy_from_slice(&crate::modifier_keys::POSITIONING.to_le_bytes());
                bytes[8..16].copy_from_slice(&7000_i64.to_le_bytes());
            } else {
                return false;
            }
            output.copy_from_slice(&bytes[..output.len()]);
            true
        }
    }

    #[test]
    fn naval_header_demand_drops_a_changed_selecting_header() {
        let memory = Leaves {
            count: 1,
            torn: true,
            invalid_name: false,
            visits: Cell::new(0),
        };
        let headers = Headers {
            memory: &memory,
            observed: RefCell::default(),
        };
        assert!(Array::read(&headers, 0x118, 8).is_some());
        assert_eq!(
            headers.validate(),
            Err("naval demand selecting header changed")
        );
    }

    #[test]
    fn naval_header_demand_scales_from_the_mixed_combat_registry() {
        use engine_native::{cgame_state, cnaval_combat, cnaval_combatant};
        const IMAGE: usize = 0x1400_0000;
        struct Combats {
            count: usize,
        }
        impl Memory for Combats {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let mut bytes = [0_u8; 16];
                match address {
                    address if address == 0x1000 + cgame_state::COMBATS + 8 => {
                        bytes[..8].copy_from_slice(&0x2000_u64.to_le_bytes());
                        let count = (self.count * 2) as u32;
                        bytes[8..12].copy_from_slice(&count.to_le_bytes());
                        bytes[12..].copy_from_slice(&count.to_le_bytes());
                    }
                    address if (0x2000..0x2000 + self.count * 16).contains(&address) => {
                        let pointer = if (address - 0x2000) & 8 == 0 {
                            0x100_0000_u64
                        } else {
                            0x400_0000_u64
                        };
                        bytes[..8].copy_from_slice(&pointer.to_le_bytes());
                    }
                    0x100_0000 => {
                        let vtable = (IMAGE + cnaval_combat::VTABLE_RVA.0) as u64;
                        bytes[..8].copy_from_slice(&vtable.to_le_bytes());
                    }
                    0x100_0018 => bytes[..8].copy_from_slice(&0x1_0000_1269_u64.to_le_bytes()),
                    0x100_0028 => bytes[..8].copy_from_slice(&0x200_0000_u64.to_le_bytes()),
                    0x100_0030 => bytes[..8].copy_from_slice(&0x300_0000_u64.to_le_bytes()),
                    address if address == 0x100_0000 + cnaval_combat::PROVINCE => {
                        bytes[..8].copy_from_slice(&0x600_0000_u64.to_le_bytes());
                    }
                    0x200_0000 | 0x300_0000 => {
                        let vtable = (IMAGE + cnaval_combatant::VTABLE_RVA.0) as u64;
                        bytes[..8].copy_from_slice(&vtable.to_le_bytes());
                    }
                    0x400_0000 => {
                        let vtable = (IMAGE + engine_native::cland_combat::VTABLE_RVA.0) as u64;
                        bytes[..8].copy_from_slice(&vtable.to_le_bytes());
                    }
                    0x200_0018 | 0x300_0018 => {
                        bytes[..8].copy_from_slice(&0x100_0000_u64.to_le_bytes());
                    }
                    0x200_02b0 | 0x300_02b0 | 0x500_0000 => {
                        bytes[..8].copy_from_slice(&0x500_0000_u64.to_le_bytes());
                    }
                    _ => (),
                }
                let Some(bytes) = bytes.get(..output.len()) else {
                    return false;
                };
                output.copy_from_slice(bytes);
                true
            }
        }
        for count in [1500, 5000] {
            let demand = demand(&Combats { count }, IMAGE, 0x1000).unwrap();
            assert_eq!(demand.rows, count);
        }
    }
}
