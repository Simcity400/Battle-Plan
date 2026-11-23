
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native_air::{cair_base, ccountry_air_container};
use engine_native_parts::{cair_wing_pool, creference_registry};
use std::{
    collections::HashSet,
    hash::{BuildHasherDefault, Hasher},
};

#[derive(Default)]
struct HandleHasher(u64);

impl Hasher for HandleHasher {
    fn finish(&self) -> u64 {
        let mut x = self.0;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
        x ^= x >> 33;
        x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        x ^ (x >> 33)
    }
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(byte);
        }
    }
    fn write_u64(&mut self, value: u64) {
        self.0 = value;
    }
}

#[derive(Debug, Default)]
pub struct UniqueHandles {
    order: Vec<u64>,
    members: HashSet<u64, BuildHasherDefault<HandleHasher>>,
}

impl UniqueHandles {
    pub fn try_with_capacity(capacity: usize) -> std::io::Result<Self> {
        let mut handles = Self::default();
        handles.try_reserve(capacity)?;
        Ok(handles)
    }
    pub fn clear(&mut self) {
        self.order.clear();
        self.members.clear();
    }
    pub fn try_insert(&mut self, handle: u64) -> std::io::Result<()> {
        if self.order.len() == self.order.capacity()
            || self.members.len() == self.members.capacity()
        {
            if self.members.contains(&handle) {
                return Ok(());
            }
            crate::allocation::reserve(&mut self.order, 1)?;
            crate::allocation::reserve_set(&mut self.members, 1)?;
        }
        if self.members.insert(handle) {
            self.order.push(handle);
        }
        Ok(())
    }
    pub fn as_slice(&self) -> &[u64] {
        &self.order
    }
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    pub fn capacity(&self) -> usize {
        self.order.capacity().min(self.members.capacity())
    }
    pub fn try_reserve(&mut self, capacity: usize) -> std::io::Result<()> {
        let additional = capacity.saturating_sub(self.order.len());
        crate::allocation::reserve(&mut self.order, additional)?;
        let additional = capacity.saturating_sub(self.members.len());
        crate::allocation::reserve_set(&mut self.members, additional)
    }
    pub fn allocated_bytes(&self) -> usize {
        self.order.capacity() * size_of::<u64>() + self.members.capacity() * (size_of::<u64>() + 1)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub base: u64,
    pub wings_start: usize,
    pub wings_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub wings_len: usize,
    pub required_wings: usize,
}

pub fn walk(
    memory: &impl Memory,
    module_base: usize,
    base_handles: &UniqueHandles,
    rows: &mut Vec<Row>,
    wings: &mut Vec<u64>,
) -> Walk {
    walk_buffer(
        memory,
        module_base,
        base_handles,
        engine_native_memory::walk::Buffer::Growing(rows),
        engine_native_memory::walk::Buffer::Growing(wings),
    )
}

fn walk_buffer(
    memory: &impl Memory,
    module_base: usize,
    base_handles: &UniqueHandles,
    mut rows: engine_native_memory::walk::Buffer<'_, Row>,
    mut wings: engine_native_memory::walk::Buffer<'_, u64>,
) -> Walk {
    let mut state = State::default();
    let result = (|| {
        rows.ensure(base_handles.len())?;
        for &handle in base_handles.as_slice() {
            if handle == 0 || handle as u32 != engine_native_parts::cair_base_layout::TYPE {
                return Err("naval combat air base handle invalid");
            }
            let pointer = creference_registry::resolve(memory, module_base, handle)?
                .ok_or("naval combat air base unresolved")?;
            let start = state.wings_len;
            let fits = state.len < rows.len();
            read_base(
                memory,
                module_base,
                handle,
                pointer,
                &mut wings,
                fits,
                &mut state,
            )?;
            if fits && state.required_wings - state.completed_wings <= wings.len() - start {
                rows[state.len] = Row {
                    base: handle,
                    wings_start: start,
                    wings_len: state.wings_len - start,
                };
                state.len += 1;
            } else {
                state.wings_len = start;
                state.unread_rows = state
                    .unread_rows
                    .checked_add(1)
                    .ok_or("naval combat air base omitted count overflow")?;
            }
            state.completed_wings = state.required_wings;
        }
        Ok(if state.unread_rows == 0 {
            Outcome::Rows { len: state.len }
        } else {
            Outcome::Truncated {
                len: state.len,
                unread_rows: state.unread_rows,
            }
        })
    })();
    Walk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        wings_len: state.wings_len,
        required_wings: state.required_wings,
    }
}

#[derive(Default)]
struct State {
    len: usize,
    unread_rows: u32,
    wings_len: usize,
    required_wings: usize,
    completed_wings: usize,
}

fn read_base(
    memory: &impl Memory,
    module_base: usize,
    handle: u64,
    pointer: usize,
    wings: &mut engine_native_memory::walk::Buffer<'_, u64>,
    store: bool,
    state: &mut State,
) -> Result<(), &'static str> {
    vtable(memory, module_base, pointer, engine_native_parts::cair_base_layout::VTABLE_RVA)?;
    if field(memory, pointer, engine_native_parts::cair_base_layout::HANDLE)? != handle {
        return Err("naval combat air base registry identity mismatch");
    }
    let containers = array(memory, pointer, cair_base::REGISTERED_CONTAINERS)?;
    for container_index in 0..containers.count {
        let container = member_pointer(memory, containers, container_index)?;
        if container == 0 {
            return Err("dense air base container pointer null");
        }
        vtable(
            memory,
            module_base,
            container,
            ccountry_air_container::VTABLE_RVA,
        )?;
        let pools = array(memory, container, ccountry_air_container::POOLS)?;
        for pool_index in 0..pools.count {
            let pool = member_pointer(memory, pools, pool_index)?;
            if pool == 0 {
                return Err("dense air base pool pointer null");
            }
            vtable(memory, module_base, pool, cair_wing_pool::VTABLE_RVA)?;
            let members = array(memory, pool, cair_wing_pool::WINGS)?;
            if store {
                wings.ensure(
                    state
                        .wings_len
                        .checked_add(members.count as usize)
                        .ok_or("dense air base wing length overflow")?,
                )?;
            }
            for wing_index in 0..members.count {
                let wing = member_pointer(memory, members, wing_index)?;
                if wing == 0 {
                    return Err("dense air base wing pointer null");
                }
                vtable(memory, module_base, wing, engine_native_parts::cair_wing_layout::VTABLE_RVA)?;
                vtable(
                    memory,
                    module_base,
                    wing.checked_add(engine_native_parts::cair_wing_layout::SECONDARY)
                        .ok_or("dense air base wing pointer overflow")?,
                    engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA,
                )?;
                let id = field(memory, wing, engine_native_parts::cair_wing_layout::REFERENCE_TYPE)?;
                if id == 0 || id as u32 != engine_native_parts::cair_wing_layout::TYPE {
                    return Err("dense air base wing handle invalid");
                }
                state.required_wings = state
                    .required_wings
                    .checked_add(1)
                    .ok_or("dense air base wing demand overflow")?;
                if store && let Some(destination) = wings.get_mut(state.wings_len) {
                    *destination = id;
                    state.wings_len += 1;
                }
            }
        }
    }
    Ok(())
}

fn field(memory: &impl Memory, pointer: usize, offset: usize) -> Result<u64, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| read::<u64>(memory, at))
        .ok_or("dense air base field unreadable")
}

fn array(memory: &impl Memory, pointer: usize, offset: usize) -> Result<Array, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("dense air base vector unreadable or invalid")
}

fn member_pointer(memory: &impl Memory, array: Array, index: u32) -> Result<usize, &'static str> {
    array
        .element(index, 8)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .ok_or("dense air base member pointer unreadable or overflow")
}

fn vtable(
    memory: &impl Memory,
    module_base: usize,
    pointer: usize,
    key: Rva,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(module_base, key).ok_or("module address overflow")? as u64;
    if read::<u64>(memory, pointer) == Some(expected) {
        Ok(())
    } else {
        Err("dense air base object vtable mismatch")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::reference_bucket as bucket;
    use std::collections::BTreeMap;

    const BASE: usize = 0x4000;
    const EMPTY_BASE: usize = 0x5000;
    const CONTAINER: usize = 0x6000;
    const POOL: usize = 0x7000;
    const WING_A: usize = 0x8000;
    const WING_B: usize = 0x9000;
    const DENSE: usize = 0xa000;
    const POOLS: usize = 0xb000;
    const WINGS: usize = 0xc000;
    const BASE_ID: u64 = (1 << 32) | engine_native_parts::cair_base_layout::TYPE as u64;
    const EMPTY_ID: u64 = (2 << 32) | engine_native_parts::cair_base_layout::TYPE as u64;
    const WING_A_ID: u64 = (3 << 32) | engine_native_parts::cair_wing_layout::TYPE as u64;
    const WING_B_ID: u64 = (4 << 32) | engine_native_parts::cair_wing_layout::TYPE as u64;

    #[derive(Default)]
    struct Fixture(BTreeMap<usize, u8>);

    impl Fixture {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, &byte)| (at + i, byte)));
        }

        fn word(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }

        fn vector(&mut self, at: usize, data: usize, count: u32) {
            self.word(at, data as u64);
            self.put(at + 8, &count.to_le_bytes());
            self.put(at + 12, &count.to_le_bytes());
        }

        fn registry_entry(&mut self, handle: u64, pointer: usize) {
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(creference_registry::HASH_MULTIPLIER);
            let index = ((hash >> 32) as u32 ^ hash as u32) as usize & 7;
            let at = 0x2000 + index * bucket::STRIDE;
            self.put(at + bucket::DISTANCE, &[1]);
            self.word(at + bucket::TYPE, handle);
            self.word(at + bucket::OBJECT, pointer as u64);
        }

        fn new() -> Self {
            let mut f = Self::default();
            f.word(
                creference_registry::LOW_ROOTS_RVA.0 + engine_native_parts::cair_base_layout::TYPE as usize * 8,
                0x1000,
            );
            f.word(0x1000 + creference_registry::ENTRIES, 0x2000);
            f.put(0x1000 + creference_registry::COUNT, &2u32.to_le_bytes());
            f.put(0x1000 + creference_registry::MASK, &7u32.to_le_bytes());
            f.put(0x1000 + creference_registry::OVERFLOW_DISTANCE, &[5]);
            f.put(0x2000, &[0; 14 * bucket::STRIDE]);
            f.put(0x2000 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
            f.registry_entry(BASE_ID, BASE);
            f.registry_entry(EMPTY_ID, EMPTY_BASE);

            f.word(BASE, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
            f.word(BASE + engine_native_parts::cair_base_layout::HANDLE, BASE_ID);
            f.vector(BASE + cair_base::REGISTERED_CONTAINERS, DENSE, 2);
            f.word(DENSE, CONTAINER as u64);
            f.word(DENSE + 8, CONTAINER as u64);
            f.word(EMPTY_BASE, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
            f.word(EMPTY_BASE + engine_native_parts::cair_base_layout::HANDLE, EMPTY_ID);
            f.vector(EMPTY_BASE + cair_base::REGISTERED_CONTAINERS, 0, 0);

            f.word(CONTAINER, ccountry_air_container::VTABLE_RVA.0 as u64);
            f.vector(CONTAINER + ccountry_air_container::POOLS, POOLS, 1);
            f.word(POOLS, POOL as u64);
            f.word(POOL, cair_wing_pool::VTABLE_RVA.0 as u64);
            f.vector(POOL + cair_wing_pool::WINGS, WINGS, 2);
            f.word(WINGS, WING_A as u64);
            f.word(WINGS + 8, WING_B as u64);
            for (pointer, handle) in [(WING_A, WING_A_ID), (WING_B, WING_B_ID)] {
                f.word(pointer, engine_native_parts::cair_wing_layout::VTABLE_RVA.0 as u64);
                f.word(
                    pointer + engine_native_parts::cair_wing_layout::SECONDARY,
                    engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA.0 as u64,
                );
                f.word(pointer + engine_native_parts::cair_wing_layout::REFERENCE_TYPE, handle);
            }
            f
        }
    }

    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (i, byte) in destination.iter_mut().enumerate() {
                let Some(source) = self.0.get(&(address + i)) else {
                    return false;
                };
                *byte = *source;
            }
            true
        }
    }

    fn handles(list: &[u64]) -> UniqueHandles {
        let mut handles = UniqueHandles::default();
        for &handle in list {
            handles.try_insert(handle).unwrap();
        }
        handles
    }

    #[test]
    fn unique_handles_keep_first_reference_order() {
        for countries in [3_u64, 11] {
            let mut unique = UniqueHandles::try_with_capacity(countries as usize).unwrap();
            let reserved = unique.allocated_bytes();
            for round in 0..3 {
                for country in 0..countries {
                    let index = (country * 7 + round) % countries;
                    unique
                        .try_insert((index + 1) << 32 | u64::from(engine_native_parts::cair_base_layout::TYPE))
                        .unwrap();
                }
            }
            assert_eq!(unique.len(), countries as usize);
            let expected: Vec<u64> = (0..countries)
                .map(|country| ((country * 7) % countries + 1) << 32 | u64::from(engine_native_parts::cair_base_layout::TYPE))
                .collect();
            assert_eq!(unique.as_slice(), expected);
            assert_eq!(unique.allocated_bytes(), reserved);
            unique.clear();
            assert!(unique.is_empty() && unique.capacity() >= countries as usize);
        }
    }

    #[test]
    fn dense_occurrences_keep_duplicates_and_empty_base_row() {
        let f = Fixture::new();
        let mut rows = vec![Row::default(); 2];
        let mut wings = vec![0; 4];
        let walked = walk(&f, 0, &handles(&[BASE_ID, EMPTY_ID]), &mut rows, &mut wings);
        assert_eq!(walked.outcome, Outcome::Rows { len: 2 });
        assert_eq!(walked.wings_len, 4);
        assert_eq!(walked.required_wings, 4);
        assert_eq!(wings, [WING_A_ID, WING_B_ID, WING_A_ID, WING_B_ID]);
        assert_eq!(
            rows,
            [
                Row {
                    base: BASE_ID,
                    wings_start: 0,
                    wings_len: 4,
                },
                Row {
                    base: EMPTY_ID,
                    wings_start: 4,
                    wings_len: 0,
                },
            ]
        );
    }

    #[test]
    fn short_storage_grows_and_an_unresolved_base_fails_closed() {
        let mut f = Fixture::new();
        let mut rows = vec![Row::default(); 2];
        let mut wings = vec![0; 3];
        let walked = walk(&f, 0, &handles(&[BASE_ID, EMPTY_ID]), &mut rows, &mut wings);
        assert_eq!(walked.outcome, Outcome::Rows { len: 2 });
        assert_eq!((walked.wings_len, walked.required_wings), (4, 4));

        f.registry_entry(BASE_ID, 0);
        assert_eq!(
            walk(&f, 0, &handles(&[BASE_ID]), &mut rows, &mut wings).outcome,
            Outcome::Unavailable {
                reason: "naval combat air base unresolved",
            }
        );
    }
    #[test]
    fn cold_and_later_dense_wing_growth_keeps_every_mod_sized_occurrence() {
        const DATA: usize = 0x10_0000;
        let mut fixture = Fixture::new();
        let bases = handles(&[BASE_ID, EMPTY_ID]);
        let (mut rows, mut wings) = (Vec::new(), Vec::new());
        for count in [3usize, 8193] {
            fixture.vector(BASE + cair_base::REGISTERED_CONTAINERS, DATA, count as u32);
            for index in 0..count {
                fixture.word(DATA + index * 8, CONTAINER as u64);
            }
            let captured = walk(&fixture, 0, &bases, &mut rows, &mut wings);
            assert_eq!(captured.outcome, Outcome::Rows { len: 2 });
            assert_eq!(
                (captured.wings_len, captured.required_wings),
                (2 * count, 2 * count)
            );
            assert_eq!(rows[0].wings_len, 2 * count);
            assert_eq!(rows[1].wings_len, 0);
            assert!(
                wings[..2 * count]
                    .chunks_exact(2)
                    .all(|pair| pair == [WING_A_ID, WING_B_ID])
            );
        }
    }
}
