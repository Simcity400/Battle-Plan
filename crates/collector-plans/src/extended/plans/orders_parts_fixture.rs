use collector_extended_support::bench_world::{Heap, World};
use engine_land_inputs::{cfront, cfront_section};
use engine_native::{
    carmy,
    ccountry,
    cgame_state,
    corder_instance as order,
    corders_group as group,
    corders_group_member,
    cprovince,
    cstate,
    ctheatre,
    cunit,
};
use engine_native_parts::{
    carmy_group,
    ccontroller_area,
    creference_registry as registry,
    reference_bucket as bucket,
};

#[cfg(test)]
const STATE: usize = 0x10000;
#[cfg(test)]
const HEAP: usize = 0x0350_0000;
#[cfg(test)]
pub(super) const GROUPS: usize = 8;
#[cfg(test)]
pub(super) const ORDERS: usize = GROUPS * 3;
#[cfg(test)]
pub(super) const FRONTS: usize = 2;
#[cfg(test)]
const CHILD_DATA: usize = 0x380;
#[cfg(test)]
const CHILD_STRIDE: usize = 0x48;
#[cfg(test)]
const CHILD_VTABLE: usize = 0x02a0_2698;
#[cfg(test)]
const SUBSCRIBER_VTABLE: usize = 0x029d_8b80;

#[cfg(test)]
pub(super) fn build() -> World {
    let len = build_in(Vec::new()).bytes.len();
    let world = build_in(Vec::with_capacity(len));
    assert_eq!(world.bytes.len(), len);
    world
}

#[cfg(test)]
fn references(heap: &mut Heap, kind: u32, objects: &[(usize, u64)]) {
    let capacity = (objects.len() * 2).next_power_of_two().max(8);
    let distance = capacity.trailing_zeros() + 2;
    let table = heap.alloc(registry::SIZE);
    let buckets = heap.alloc((capacity + distance as usize + 1) * bucket::STRIDE);
    heap.u64(
        registry::LOW_ROOTS_RVA.0 + kind as usize * 8,
        heap.ptr(table),
    );
    heap.u64(table + registry::ENTRIES, heap.ptr(buckets));
    heap.u32(table + registry::COUNT, objects.len() as u32);
    heap.u32(table + registry::MASK, capacity as u32 - 1);
    heap.put(table + registry::OVERFLOW_DISTANCE, &[distance as u8]);
    let mut occupied = vec![false; capacity + distance as usize];
    for &(object, handle) in objects {
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let first = (((hash >> 32) as u32 ^ hash as u32) & (capacity as u32 - 1)) as usize;
        let slot = (first..occupied.len())
            .find(|&slot| !occupied[slot])
            .unwrap();
        occupied[slot] = true;
        let entry = buckets + slot * bucket::STRIDE;
        heap.put(entry + bucket::DISTANCE, &[(slot - first + 1) as u8]);
        heap.u64(entry + bucket::TYPE, handle);
        heap.u64(entry + bucket::OBJECT, heap.ptr(object));
    }
    heap.put(
        buckets + (capacity + distance as usize) * bucket::STRIDE + bucket::DISTANCE,
        &[bucket::TERMINAL_DISTANCE],
    );
}

#[cfg(test)]
fn build_in(mut bytes: Vec<u8>) -> World {
    let origin = bytes.as_ptr() as usize;
    bytes.resize(HEAP, 0);
    let mut heap = Heap { bytes, origin };
    let countries = heap.alloc(3 * 8);
    heap.array(STATE + cgame_state::COUNTRIES, countries, 3);
    heap.words(STATE + cgame_state::COUNTRY_LINKS, &[0, 1, 2]);
    let mut owner = 0;
    for id in 1..=2 {
        let country =
            heap.alloc((engine_native_memory::country_identity::MODIFIER_ACCUMULATOR + 0x20).max(ccountry::DIPLOMACY_STATUS + 8));
        heap.vtable(country, ccountry::VTABLE_RVA);
        heap.u32(country + ccountry::ID, id);
        heap.u64(countries + id as usize * 8, heap.ptr(country));
        heap.array(country + engine_native_memory::country_identity::MODIFIER_ACCUMULATOR, 0, 0);
        if id == 1 {
            owner = country;
        }
    }
    let diplomacy = heap.alloc(engine_native_parts::cdiplomacy_status_layout::RELATIONS + 16);
    heap.vtable(diplomacy, engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA);
    heap.u64(owner + ccountry::DIPLOMACY_STATUS, heap.ptr(diplomacy));
    heap.pointers(diplomacy + engine_native_parts::cdiplomacy_status_layout::RELATIONS, &[0, 0, 0]);
    let state = heap.alloc(cstate::CONTROLLER + 8);
    heap.vtable(state, cstate::VTABLE_RVA);
    heap.u32(state + cstate::CONTROLLER, 2);
    let area = heap.alloc(ccontroller_area::PROVINCE_COUNT + 8);
    heap.vtable(area, ccontroller_area::VTABLE_RVA);
    heap.u32(area + ccontroller_area::PROVINCE_COUNT, 4);
    let mut provinces = vec![0];
    for id in 1..=4 {
        let province = heap.alloc(cprovince::CONTROLLER_AREA.max(cprovince::STATE) + 8);
        heap.vtable(province, cprovince::VTABLE_RVA);
        heap.u32(province + cprovince::ID, id);
        heap.u64(province + cprovince::STATE, heap.ptr(state));
        heap.u64(province + cprovince::CONTROLLER_AREA, heap.ptr(area));
        provinces.push(province);
    }
    heap.pointers(STATE + cgame_state::PROVINCES, &provinces);
    let mut fronts = Vec::new();
    for index in 0..FRONTS {
        let front = heap.alloc(0x88);
        let handle = ((index as u64 + 1) << 32) | u64::from(cfront::REFERENCE_TYPE);
        heap.vtable(front, cfront::VTABLE_RVA);
        heap.u64(front + cfront::HANDLE, handle);
        let mut sections = Vec::new();
        for (id, pairs) in [(9, true), (3, false)] {
            let section = heap.alloc(0x60);
            heap.vtable(section, cfront_section::VTABLE_RVA);
            heap.u32(section + cfront_section::ID, id);
            heap.u64(section + cfront_section::FRONT, heap.ptr(front));
            if pairs {
                let data = heap.alloc(32);
                for (index, province) in [provinces[1], provinces[2], provinces[2], provinces[1]]
                    .into_iter()
                    .enumerate()
                {
                    heap.u64(data + index * 8, heap.ptr(province));
                }
                heap.array(section + cfront_section::PAIRS, data, 2);
            }
            sections.push(section);
        }
        heap.pointers(front + cfront::SECTIONS, &sections);
        fronts.push((front, handle));
    }
    references(&mut heap, cfront::REFERENCE_TYPE, &fronts);
    let theatre = heap.alloc(ctheatre::SIZE);
    heap.vtable(theatre, ctheatre::VTABLE_RVA);
    heap.u64(
        theatre + ctheatre::HANDLE,
        (1 << 32) | u64::from(ctheatre::REFERENCE_TYPE),
    );
    heap.u32(theatre + ctheatre::OWNER, 1);
    heap.pointers(owner + ccountry::THEATRES, &[theatre]);
    let mut groups = Vec::new();
    for index in 0..GROUPS {
        let pointer = heap.alloc(carmy_group::SIZE);
        let handle = ((index as u64 + 1) << 32) | u64::from(group::REFERENCE_TYPE);
        heap.vtable(pointer, carmy_group::VTABLE_RVA);
        heap.u64(pointer + group::HANDLE, handle);
        heap.put(pointer + group::IS_ARMY_GROUP, &[1]);
        heap.u32(pointer + group::COUNTRY, 1);
        heap.u64(pointer + group::THEATRE, heap.ptr(theatre));
        heap.string(pointer + group::NAME, b"Synthetic group with heap text");
        heap.u32(pointer + group::COLOUR, 0x3f80_0000 + index as u32);
        groups.push((pointer, handle));
    }
    for (index, &(pointer, _)) in groups.iter().enumerate() {
        let unit = heap.alloc(cunit::LOGICAL_OWNER + 8);
        heap.vtable(unit, carmy::VTABLE_RVA);
        heap.vtable(unit + carmy::SECONDARY_BASE, carmy::SECONDARY_VTABLE_RVA);
        heap.u64(
            unit + cunit::HANDLE,
            ((index as u64 + 1) << 32) | u64::from(registry::DYNAMIC_TYPE_BASE + 17),
        );
        heap.u64(unit + carmy::ORDERS_GROUP, heap.ptr(pointer));
        heap.u32(unit + cunit::LOGICAL_OWNER, 1);
        let member = unit + corders_group_member::HOST_OFFSET;
        heap.pointers(pointer + group::MEMBER_DATA, &[member]);
        let children = [
            groups[(index + 1) % GROUPS].0,
            groups[(index + 2) % GROUPS].0,
        ];
        heap.pointers(pointer + carmy_group::MEMBER_DATA, &children);
        let mut orders = Vec::new();
        for (ordinal, kind) in [order::TYPE_LINE, order::TYPE_MOVE, order::TYPE_INVASION]
            .into_iter()
            .enumerate()
        {
            let at = heap.alloc(0x400);
            heap.vtable(at, order::VTABLE_RVA);
            heap.u64(at + order::OWNER, heap.ptr(pointer));
            heap.u32(at + order::ID, ordinal as u32 + 1);
            heap.u32(at + order::TYPE, kind as u32);
            heap.words(at + order::PATH_DATA, &[1, 2]);
            heap.words(at + order::TARGET_STATE_DATA, &[17 + index as u32]);
            heap.u64(
                at + order::FRONT_BINDING,
                fronts[usize::from(kind == order::TYPE_INVASION)].1,
            );
            heap.u32(at + order::FRONT_SECTION, 9);
            heap.u32(at + order::SEGMENT_START, u32::MAX);
            heap.u32(at + order::SEGMENT_END, 1);
            if kind == order::TYPE_LINE {
                let entries = heap.alloc(2 * CHILD_STRIDE);
                heap.array(at + CHILD_DATA, entries, 2);
                for (child, &target) in children.iter().enumerate() {
                    let entry = entries + child * CHILD_STRIDE;
                    heap.u64(entry, heap.ptr(CHILD_VTABLE));
                    heap.u64(entry + 8, heap.ptr(target));
                    heap.u32(entry + 0x20, if child == 0 { u32::MAX } else { 0 });
                    heap.u32(entry + 0x24, child as u32);
                }
            } else {
                heap.pointers(at + order::ASSIGNED_MEMBER_DATA, &[member]);
                for offset in order::NAME_STRINGS {
                    heap.string(at + offset, b"Synthetic order name with heap text");
                }
            }
            if kind == order::TYPE_INVASION {
                heap.u64(at + 8, heap.ptr(SUBSCRIBER_VTABLE));
                heap.u32(at + 0x10, index as u32 + 2);
                heap.u64(at + order::ACCUMULATED_PREPARATION, 123_456 + index as u64);
            }
            orders.push(at);
        }
        for pair in orders.windows(2) {
            heap.pointers(pair[0] + order::SUCCESSOR_DATA, &[pair[1]]);
            heap.pointers(pair[1] + order::PREDECESSOR_DATA, &[pair[0]]);
        }
        heap.pointers(pointer + group::PRIMARY_ORDER_DATA, &orders);
    }
    let roster: Vec<_> = groups.iter().map(|&(pointer, _)| pointer).collect();
    heap.pointers(theatre + ctheatre::ARMY_GROUP_DATA, &roster);
    references(&mut heap, group::REFERENCE_TYPE, &groups);
    World {
        bytes: heap.bytes,
        base: origin,
        game_state: origin + STATE,
        groups: GROUPS as u32,
        orders: ORDERS as u32,
        faction_theatres: 0,
    }
}
