//! Off-thread bounds from live source headers and the shared count-only
//! modifier census, without copying display-name payloads. 140C29020,
//! 14195C7D0 and 141963700 select the source/hover union;
//! 14054D0F0 traverses child+0x10 with depth -1. Depth and name bytes have no
//! fixed per-request bound, so their sizing hint visits the retained topology.

use super::{at, field};
use crate::{Memory, cpdx_array::Array};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

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
    /// Maximum group scratch needed by any one combat.
    pub groups: usize,
    pub planes: usize,
    pub commanders: usize,
    pub side_units: usize,
    pub damage_sources: usize,
    pub wing_types: usize,
    pub plane_wings: usize,
    pub plane_air_hits: usize,
    pub plane_naval_hits: usize,
    pub requests: usize,
    pub roots: usize,
    pub nodes: usize,
    pub text: usize,
    pub depth: usize,
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

// Every selecting array header observed by the source visitor is checked
// again before accepting this hint. Hint allocations never call the collector's
// terminal allocation callback.
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
            if crate::read::<[u8; 16]>(self.memory, address) != Some(header) {
                return Err("naval demand selecting header changed");
            }
        }
        Ok(())
    }
}

fn request_keys(
    roots: &mut HashMap<usize, [bool; super::KEYS.len()]>,
    root: usize,
    keys: &[u32],
) -> Result<(), &'static str> {
    if !roots.contains_key(&root) {
        roots
            .try_reserve(1)
            .map_err(|_| "naval sizing allocation skipped")?;
    }
    let wanted = roots.entry(root).or_default();
    for key in keys {
        wanted[super::KEYS
            .binary_search(key)
            .map_err(|_| "naval demand hover key unsupported")?] = true;
    }
    Ok(())
}

fn census_root(
    memory: &impl Memory,
    base: usize,
    root: usize,
    wanted: [bool; super::KEYS.len()],
    demand: &mut Demand,
) -> Result<(), &'static str> {
    let mut stack = Vec::<super::Frame>::new();
    let mut buffers = super::Buffers {
        requests: crate::walk::Buffer::Fixed(&mut []),
        request_len: 0,
        request_required: 0,
        roots: crate::walk::Buffer::Fixed(&mut []),
        root_len: 0,
        root_required: 0,
        nodes: crate::walk::Buffer::Fixed(&mut []),
        node_len: 0,
        node_required: 0,
        text: crate::walk::Buffer::Fixed(&mut []),
        text_len: 0,
        text_required: 0,
        stack: crate::walk::Buffer::Growing(&mut stack),
        stack_required: 0,
    };
    super::capture_requested_root(
        memory,
        base,
        root,
        wanted,
        false,
        &mut buffers,
        &super::StackLimits::current(),
    )?;
    add(&mut demand.roots, buffers.root_required)?;
    add(&mut demand.nodes, buffers.node_required)?;
    add(&mut demand.text, buffers.text_required)?;
    demand.depth = demand.depth.max(buffers.stack_required);
    Ok(())
}

/// A conservative bound at this observation. Repeated member occurrences and
/// the native escape/carrier key candidates bound demand. The shared count-only
/// census reads effective values, so zero leaves need no names. A torn hint is dropped.
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
    use crate::{cfex_group, cgame_state, cnaval_combat, cnaval_combatant};
    let headers = Headers {
        memory,
        observed: RefCell::default(),
    };
    let memory = &headers;
    let roster_at = at(game_state, cgame_state::COMBATS + 8)?;
    let roster = Array::read(memory, roster_at, 8).ok_or("naval demand registry invalid")?;
    let mut demand = Demand::default();
    let mut roots = HashMap::new();
    let mut groups = HashSet::new();
    for index in 0..roster.count {
        let combat = element(memory, roster, index)?;
        if field::<u64>(memory, combat, 0)? != at(base, cnaval_combat::VTABLE_RVA)? as u64 {
            continue;
        }
        let sides = [
            pointer(memory, combat, cnaval_combat::ATTACKER)?,
            pointer(memory, combat, cnaval_combat::DEFENDER)?,
        ];
        let row = cnaval_combat::DetailsRow {
            combat: field(memory, combat, cnaval_combat::HANDLE)?,
            native_combat: combat,
            duration: field(memory, combat, crate::cland_combat::DURATION)?,
            ..Default::default()
        };
        add(&mut demand.rows, 1)?;
        let province = pointer(memory, combat, cnaval_combat::PROVINCE)?;
        let region = pointer(memory, province, crate::cprovince::STRATEGIC_REGION)?;
        if region != 0 {
            let template = pointer(memory, region, crate::cstrategic_region::TEMPLATE)?;
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
            if count < 0 || capacity < count {
                return Err("naval demand custom sea count invalid");
            }
            add(&mut demand.custom_sea, count as usize)?;
        }
        for &side in &sides {
            crate::cnaval_combatant::side(memory, base, side, combat, false)?;
            add(
                &mut demand.side_units,
                array(memory, side, cnaval_combatant::UNITS)?.count as usize,
            )?;
            add(
                &mut demand.damage_sources,
                cnaval_combatant::damage_source_demand(memory, side)?,
            )?;
            for offset in [
                crate::naval_combat_summary_inputs::COUNTRY_COUNT,
                crate::naval_combat_summary_inputs::SECONDARY_COUNTRIES + 0x10,
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
                    at(commander, crate::cunit_leader::TIMED_TRAIT_DATA)?,
                    crate::cunit_leader::TIMED_TRAIT_STRIDE,
                )
                .ok_or("naval demand commander timed traits invalid")?;
                add(&mut demand.commander_timed_traits, timed.count as usize)?;
            }
        }
        // Empty members conservatively include the country-only submarine
        // hover. The native side paths emit at most 1 + 4 + 4 requests each
        // (14160A7B0, 141609460 -> 1415B4BB0).
        let initial = field(memory, base, super::COMBAT_INITIAL_DURATION_RVA)?;
        super::side_trees::<false>(
            &mut |request| {
                add(&mut demand.requests, 1)?;
                request_keys(&mut roots, request.accumulator, &[request.key])?;
                Ok(())
            },
            memory,
            base,
            game_state,
            &row,
            &[],
            initial,
        )?;
        groups.clear();
        for &side in &sides {
            let forward = array(memory, side, cnaval_combatant::GROUPS)?;
            for index in 0..forward.count {
                let group = element(memory, forward, index)?;
                let opponent = pointer(memory, group, cfex_group::OPPONENT_GROUP)?;
                for group in [group, opponent].into_iter().filter(|&group| group != 0) {
                    if field::<u64>(memory, group, 0)? != at(base, cfex_group::VTABLE_RVA)? as u64
                        || !sides.contains(&pointer(memory, group, cfex_group::SIDE)?)
                    {
                        return Err("naval demand group identity mismatch");
                    }
                    insert(&mut groups, group)?;
                }
            }
            count_planes(memory, base, array(memory, side, 0xe0)?, &mut demand)?;
            if array(memory, side, cnaval_combatant::GROUPS)? != forward {
                return Err("naval demand group roster changed");
            }
        }
        demand.groups = demand.groups.max(groups.len());
        let mut carrier_air = [false; 2];
        for &group in &groups {
            let owner = pointer(memory, group, cfex_group::SIDE)?;
            let side = usize::from(owner == sides[1]);
            carrier_air[side] |= array(memory, group, 0x58)?.count != 0;
        }
        for &group in &groups {
            let side = pointer(memory, group, cfex_group::SIDE)?;
            let active_air = array(memory, group, 0x58)?;
            count_planes(memory, base, active_air, &mut demand)?;
            count_planes(memory, base, array(memory, group, 0x88)?, &mut demand)?;
            // 14195C7D0 selects at most two escape keys. Each drawable
            // carrier-air slot (141963700) selects two more, deduplicated
            // per member by enumerate_requests. Plane/wing counts cannot
            // increase that native four-key ceiling.
            let side_index = usize::from(side == sides[1]);
            let keys = if carrier_air[side_index] { 4 } else { 2 };
            for offset in [cfex_group::MEMBERS, 0x70] {
                let members = array(memory, group, offset)?;
                add(&mut demand.ships, members.count as usize)?;
                for index in 0..members.count {
                    let member = element(memory, members, index)?;
                    for (offset, stride, lane) in [
                        (crate::cfex_member::DAMAGE_ENTRIES, 0x20, 0),
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
                    let handle = field(memory, member, crate::cfex_member::SHIP_HANDLE)?;
                    if let Some(ship) = crate::creference_registry::resolve(memory, base, handle)? {
                        use crate::naval_ship_modifier_inputs::visit_sources;
                        let context = visit_sources::<false>(
                            memory,
                            base,
                            game_state,
                            ship,
                            side,
                            &mut |source, accumulator| {
                                add(&mut demand.weapon_sources, 1)?;
                                if let Some(root) = accumulator {
                                    let entries = Array::read(memory, root, 16)
                                        .ok_or("naval demand weapon terms invalid")?;
                                    add(&mut demand.weapon_terms, entries.count as usize)?;
                                }
                                if let Some(root) = accumulator
                                    && matches!(source.scope, 0..=15)
                                {
                                    add(&mut demand.requests, keys)?;
                                    let selected: &[u32] = if keys == 4 {
                                        super::SHIP_CARRIER_ESCAPE_KEYS
                                    } else {
                                        super::SHIP_ESCAPE_KEYS
                                    };
                                    request_keys(&mut roots, root, selected)?;
                                }
                                Ok(())
                            },
                        )?;
                        count_base(memory, base, context.air_base, &mut demand)?;
                    } else {
                        // 14195EEC0's non-ship path reaches at most the four
                        // side accumulators, with the distinct convoy key.
                        let member = crate::cfex_member::DetailsRow {
                            native_group: group,
                            member: cnaval_combatant::Row {
                                combat: row.combat,
                                side: side_index as u8,
                                ..Default::default()
                            },
                            ..Default::default()
                        };
                        super::convoy::<false>(
                            memory,
                            base,
                            game_state,
                            &member,
                            0x5d,
                            &mut |request| {
                                add(&mut demand.requests, 1)?;
                                request_keys(&mut roots, request.accumulator, &[request.key])
                            },
                        )?;
                    }
                }
                if array(memory, group, offset)? != members {
                    return Err("naval demand member roster changed");
                }
            }
        }
        if pointer(memory, combat, cnaval_combat::ATTACKER)? != sides[0]
            || pointer(memory, combat, cnaval_combat::DEFENDER)? != sides[1]
            || field::<u64>(memory, combat, cnaval_combat::HANDLE)? != row.combat
        {
            return Err("naval demand combat changed");
        }
    }
    for (root, wanted) in roots {
        census_root(memory, base, root, wanted, &mut demand)?;
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
    // 141C58360 serializes group +58/+88 to their own counts;
    // 141962F80/141969B60 maintain each plane's +28/+40 wing arrays.
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
            if let Some(wing) = crate::naval_air_efficiency_inputs::wing(memory, base, handle)? {
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

/// CountAirBaseAircraft@140C3E1D0 walks every dense registered occurrence.
/// Repeated references conservatively reserve the same base more than once.
fn count_base(
    memory: &impl Memory,
    base: usize,
    handle: u64,
    demand: &mut Demand,
) -> Result<(), &'static str> {
    if handle == 0 {
        return Ok(());
    }
    let Some(pointer) = crate::creference_registry::resolve(memory, base, handle)? else {
        return Ok(());
    };
    if field::<u64>(memory, pointer, 0)? != at(base, crate::cair_base::VTABLE_RVA)? as u64
        || field::<u64>(memory, pointer, crate::cair_base::HANDLE)? != handle
    {
        return Err("naval demand dense base identity mismatch");
    }
    add(&mut demand.dense_bases, 1)?;
    let containers = array(memory, pointer, crate::cair_base::REGISTERED_CONTAINERS)?;
    for index in 0..containers.count {
        let container = element(memory, containers, index)?;
        let pools = array(memory, container, crate::ccountry_air_container::POOLS)?;
        for index in 0..pools.count {
            let pool = element(memory, pools, index)?;
            add(
                &mut demand.dense_wings,
                array(memory, pool, crate::cair_wing_pool::WINGS)?.count as usize,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifier_definition;
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
            if address == modifier_definition::TABLE_RVA && output.len() == 8 {
                bytes[..8].copy_from_slice(&0x3000_0000_u64.to_le_bytes());
            } else if (0x3000_0000..0x3004_0000).contains(&address) {
                // All requested metadata enables the raw value unchanged.
            } else if address == 0x100 && output.len() == 16 {
                // Empty aggregate entry array.
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

    #[cfg(windows)]
    #[test]
    fn naval_header_demand_uses_shared_census_at_live_tree_scale() {
        for count in [1500, 5000] {
            let memory = Leaves {
                count,
                torn: false,
                invalid_name: false,
                visits: Cell::new(0),
            };
            let mut demand = Demand::default();
            let mut wanted = [false; super::super::KEYS.len()];
            wanted[6] = true;
            census_root(&memory, 0, 0x100, wanted, &mut demand).unwrap();
            assert_eq!(demand.roots, 1);
            assert_eq!(demand.nodes, count as usize + 1);
            assert_eq!(demand.text, count as usize * 8);
            assert_eq!(demand.depth, 2);
        }
    }

    #[cfg(windows)]
    #[test]
    fn naval_header_demand_drops_torn_tree_and_invalid_string_headers() {
        struct ChangedName<'a> {
            memory: &'a Leaves,
            visits: Cell<usize>,
        }
        impl Memory for ChangedName<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if !self.memory.copy(address, output) {
                    return false;
                }
                if address == 0x100_0048 && output.len() == 32 {
                    let visits = self.visits.get();
                    self.visits.set(visits + 1);
                    if visits != 0 {
                        output[16] = 9;
                    }
                }
                true
            }
        }
        let mut memory = Leaves {
            count: 1,
            torn: true,
            invalid_name: false,
            visits: Cell::new(0),
        };
        let mut demand = Demand::default();
        assert_eq!(
            census_root(
                &memory,
                0,
                0x100,
                [true; super::super::KEYS.len()],
                &mut demand,
            ),
            Err("naval modifier child header changed")
        );
        assert_eq!(demand, Demand::default());
        memory.torn = false;
        memory.invalid_name = true;
        assert_eq!(
            census_root(
                &memory,
                0,
                0x100,
                [true; super::super::KEYS.len()],
                &mut demand,
            ),
            Err("string header invalid")
        );
        assert_eq!(demand, Demand::default());
        memory.invalid_name = false;
        assert_eq!(
            census_root(
                &ChangedName {
                    memory: &memory,
                    visits: Cell::new(0),
                },
                0,
                0x100,
                [true; super::super::KEYS.len()],
                &mut demand,
            ),
            Err("naval modifier display header changed")
        );
        assert_eq!(demand, Demand::default());
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
        use crate::{cgame_state, cnaval_combat, cnaval_combatant};
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
                        let vtable = (IMAGE + cnaval_combat::VTABLE_RVA) as u64;
                        bytes[..8].copy_from_slice(&vtable.to_le_bytes());
                    }
                    0x100_0018 => bytes[..8].copy_from_slice(&0x1_0000_1269_u64.to_le_bytes()),
                    0x100_0028 => bytes[..8].copy_from_slice(&0x200_0000_u64.to_le_bytes()),
                    0x100_0030 => bytes[..8].copy_from_slice(&0x300_0000_u64.to_le_bytes()),
                    address if address == 0x100_0000 + cnaval_combat::PROVINCE => {
                        bytes[..8].copy_from_slice(&0x600_0000_u64.to_le_bytes());
                    }
                    0x200_0000 | 0x300_0000 => {
                        let vtable = (IMAGE + cnaval_combatant::VTABLE_RVA) as u64;
                        bytes[..8].copy_from_slice(&vtable.to_le_bytes());
                    }
                    0x400_0000 => {
                        let vtable = (IMAGE + crate::cland_combat::VTABLE_RVA) as u64;
                        bytes[..8].copy_from_slice(&vtable.to_le_bytes());
                    }
                    0x200_0018 | 0x300_0018 => {
                        bytes[..8].copy_from_slice(&0x100_0000_u64.to_le_bytes());
                    }
                    0x200_0290 | 0x300_0290 | 0x500_0000 => {
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
            assert_eq!((demand.requests, demand.roots, demand.nodes), (0, 0, 0));
        }
    }
}
