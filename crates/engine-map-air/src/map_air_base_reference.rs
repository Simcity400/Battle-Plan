
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{
        Raw,
        RecordField,
        RecordLayout,
        RecordSources,
        packed_fields,
        packed_stride,
    },
    read,
};
use engine_native_air::{cair_base, cstrategic_air_manager};
use engine_native::{cgame_state, cprovince, cship, cstate, ctask_force, cunit};
use engine_native_parts::creference_registry;
use engine_sites::Rva;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BaseReference {
    pub object: usize,
    pub state: Option<u32>,
    pub province: Option<u32>,
    pub host: u64,
    pub kind: i32,
}

fn field<T: Raw>(memory: &impl Memory, object: usize, offset: usize) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("air reference field unreadable")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "air reference pointer overflow")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable_rva: Rva,
) -> Result<(), &'static str> {
    let expected =
        engine_sites::at(base, vtable_rva).ok_or("air reference module address overflow")?;
    if object == 0 || read::<u64>(memory, object) != Some(expected as u64) {
        return Err("air reference object identity mismatch");
    }
    Ok(())
}

pub fn read_base(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    handle: u64,
) -> Result<Option<BaseReference>, &'static str> {
    Pass::new(base).read_base(memory, game_state, handle)
}

pub const BASE_BATCH: usize = 64;

const BUCKET_FIELDS: [RecordField; 1] = packed_fields([(0, 24)]);
const BUCKET_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&BUCKET_FIELDS, 24) {
    Some(layout) => layout,
    None => panic!("air reference bucket layout"),
};
const BASE_SPANS: [(usize, usize); 6] = [
    (0, 8),
    (engine_native_parts::cair_base_layout::HANDLE, 8),
    (engine_native_parts::cair_base_layout::HOST_REFERENCE, 8),
    (cair_base::STATE, 8),
    (cair_base::PROVINCE, 8),
    (cair_base::KIND, 4),
];
const BASE_FIELDS: [RecordField; 6] = packed_fields(BASE_SPANS);
const BASE_ROW: usize = packed_stride(BASE_SPANS);
const BASE_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&BASE_FIELDS, BASE_ROW) {
    Some(layout) => layout,
    None => panic!("air reference base layout"),
};
const STATE_FIELDS: [RecordField; 2] = packed_fields([(0, 8), (cstate::ID, 4)]);
const PROVINCE_FIELDS: [RecordField; 2] = packed_fields([(0, 8), (cprovince::ID, 4)]);
const IDENTITY_ROW: usize = 12;
const STATE_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&STATE_FIELDS, IDENTITY_ROW) {
    Some(layout) => layout,
    None => panic!("air reference state layout"),
};
const PROVINCE_LAYOUT: RecordLayout<'static> =
    match RecordLayout::new(&PROVINCE_FIELDS, IDENTITY_ROW) {
        Some(layout) => layout,
        None => panic!("air reference province layout"),
    };

struct Gather {
    pointers: [usize; BASE_BATCH],
    len: usize,
    rows: usize,
}

impl Gather {
    fn new() -> Self {
        Self {
            pointers: [0; BASE_BATCH],
            len: 0,
            rows: 0,
        }
    }

    fn push(&mut self, pointer: usize) -> usize {
        self.pointers[self.len] = pointer;
        self.len += 1;
        self.len - 1
    }

    fn copy(&mut self, memory: &impl Memory, layout: &RecordLayout<'_>, rows: &mut [u8]) {
        let fields = layout.field_count();
        self.rows = memory.copy_records(
            RecordSources::Pointers(&self.pointers[..self.len]),
            layout,
            &mut rows[..self.len * layout.stride()],
        ) / fields;
    }
}

fn identified(rows: &[u8], row: usize, gathered: usize, base: usize, rva: Rva) -> Option<u32> {
    if row >= gathered {
        return None;
    }
    let record = &rows[row * IDENTITY_ROW..(row + 1) * IDENTITY_ROW];
    let vtable = u64::from_le_bytes(record[..8].try_into().unwrap());
    (engine_sites::at(base, rva).map(|expected| expected as u64) == Some(vtable))
        .then(|| u32::from_le_bytes(record[8..].try_into().unwrap()))
}

pub struct Pass {
    base: usize,
    resolver: creference_registry::ScopedResolver,
    manager: Option<usize>,
    lists: [Option<Array>; 3],
    provinces: Option<Array>,
}

impl Pass {
    pub fn new(base: usize) -> Self {
        Self {
            base,
            resolver: creference_registry::ScopedResolver::new(base),
            manager: None,
            lists: [None; 3],
            provinces: None,
        }
    }

    pub fn read_base(
        &mut self,
        memory: &impl Memory,
        game_state: usize,
        handle: u64,
    ) -> Result<Option<BaseReference>, &'static str> {
        let base = self.base;
        let Some(object) = self.resolver.resolve(memory, handle)? else {
            return Ok(None);
        };
        let [mut vtable, mut stored, mut host, mut state, mut province] = [[0_u8; 8]; 5];
        let mut kind = [0_u8; 4];
        let at = |offset: usize| object.checked_add(offset).unwrap_or(0);
        if memory.copy_scattered(
            [
                object,
                at(engine_native_parts::cair_base_layout::HANDLE),
                at(engine_native_parts::cair_base_layout::HOST_REFERENCE),
                at(cair_base::STATE),
                at(cair_base::PROVINCE),
                at(cair_base::KIND),
            ],
            &mut [
                &mut vtable,
                &mut stored,
                &mut host,
                &mut state,
                &mut province,
                &mut kind,
            ],
        ) != 6
        {
            return Err("air base reference fields unreadable");
        }
        if u64::from_le_bytes(vtable)
            != engine_sites::at(base, engine_native_parts::cair_base_layout::VTABLE_RVA)
                .ok_or("air reference module address overflow")? as u64
            || u64::from_le_bytes(stored) != handle
        {
            return Err("air base reference identity mismatch");
        }
        let mut reference = BaseReference {
            object,
            host: u64::from_le_bytes(host),
            kind: i32::from_le_bytes(kind),
            ..Default::default()
        };
        let identified = |pointer: [u8; 8], vtable_rva: Rva, id: usize| {
            let pointer = usize::try_from(u64::from_le_bytes(pointer))
                .map_err(|_| "air reference pointer overflow")?;
            authenticate(memory, base, pointer, vtable_rva)?;
            field::<u32>(memory, pointer, id)
        };
        if u64::from_le_bytes(state) != 0 {
            let state = identified(state, cstate::VTABLE_RVA, cstate::ID)?;
            reference.state = Some(state);
            reference.province = if u64::from_le_bytes(province) != 0 {
                Some(identified(province, cprovince::VTABLE_RVA, cprovince::ID)?)
            } else {
                self.first_province(memory, game_state, state, reference.kind)?
            };
        }
        Ok(Some(reference))
    }

    pub fn read_bases(
        &mut self,
        memory: &impl Memory,
        game_state: usize,
        handles: &[u64],
        output: &mut [Option<BaseReference>],
    ) -> usize {
        const NONE: usize = usize::MAX;
        let base = self.base;
        let count = handles.len().min(output.len()).min(BASE_BATCH);
        let mut limit = count;
        let mut headers = [None; BASE_BATCH];
        let mut bucket = [NONE; BASE_BATCH];
        let mut buckets = Gather::new();
        for (index, &handle) in handles[..count].iter().enumerate() {
            match self.resolver.first_cell(memory, handle) {
                Ok(Some((header, cell))) => {
                    headers[index] = Some(header);
                    bucket[index] = buckets.push(cell);
                }
                Ok(None) => {}
                Err(_) => {
                    limit = index;
                    break;
                }
            }
        }
        let mut bucket_rows = [0_u8; BASE_BATCH * 24];
        buckets.copy(memory, &BUCKET_LAYOUT, &mut bucket_rows);
        let mut object = [NONE; BASE_BATCH];
        let mut objects = Gather::new();
        for index in 0..limit {
            let Some(header) = headers[index] else {
                continue;
            };
            let row = bucket[index];
            if row >= buckets.rows {
                limit = index;
                break;
            }
            let first = bucket_rows[row * 24..(row + 1) * 24].try_into().unwrap();
            match self
                .resolver
                .resolve_gathered(memory, header, handles[index], first)
            {
                Ok(Some(pointer)) => object[index] = objects.push(pointer),
                Ok(None) => {}
                Err(_) => {
                    limit = index;
                    break;
                }
            }
        }
        let mut field_rows = [0_u8; BASE_BATCH * BASE_ROW];
        objects.copy(memory, &BASE_LAYOUT, &mut field_rows);
        let mut fields = [(0_u64, 0_u64, 0_u64, 0_i32); BASE_BATCH];
        let (mut state, mut province) = ([NONE; BASE_BATCH], [NONE; BASE_BATCH]);
        let (mut states, mut provinces) = (Gather::new(), Gather::new());
        let expected = engine_sites::at(base, engine_native_parts::cair_base_layout::VTABLE_RVA).map(|vtable| vtable as u64);
        for index in 0..limit {
            let row = object[index];
            if row == NONE {
                continue;
            }
            if row >= objects.rows {
                limit = index;
                break;
            }
            let record = &field_rows[row * BASE_ROW..(row + 1) * BASE_ROW];
            let word = |field: usize| {
                let at = BASE_FIELDS[field].destination_offset;
                u64::from_le_bytes(record[at..at + 8].try_into().unwrap())
            };
            if expected != Some(word(0)) || word(1) != handles[index] {
                limit = index;
                break;
            }
            let at = BASE_FIELDS[5].destination_offset;
            let kind = i32::from_le_bytes(record[at..at + 4].try_into().unwrap());
            fields[index] = (word(2), word(3), word(4), kind);
            if word(3) != 0 {
                state[index] = states.push(word(3) as usize);
                if word(4) != 0 {
                    province[index] = provinces.push(word(4) as usize);
                }
            }
        }
        let mut state_rows = [0_u8; BASE_BATCH * IDENTITY_ROW];
        let mut province_rows = [0_u8; BASE_BATCH * IDENTITY_ROW];
        states.copy(memory, &STATE_LAYOUT, &mut state_rows);
        provinces.copy(memory, &PROVINCE_LAYOUT, &mut province_rows);
        for index in 0..limit {
            let row = object[index];
            if row == NONE {
                output[index] = None;
                continue;
            }
            let (host, stored_state, stored_province, kind) = fields[index];
            let mut reference = BaseReference {
                object: objects.pointers[row],
                host,
                kind,
                ..Default::default()
            };
            if stored_state != 0 {
                let Some(id) = identified(
                    &state_rows,
                    state[index],
                    states.rows,
                    base,
                    cstate::VTABLE_RVA,
                ) else {
                    limit = index;
                    break;
                };
                reference.state = Some(id);
                reference.province = if stored_province != 0 {
                    let Some(id) = identified(
                        &province_rows,
                        province[index],
                        provinces.rows,
                        base,
                        cprovince::VTABLE_RVA,
                    ) else {
                        limit = index;
                        break;
                    };
                    Some(id)
                } else {
                    match self.first_province(memory, game_state, id, kind) {
                        Ok(province) => province,
                        Err(_) => {
                            limit = index;
                            break;
                        }
                    }
                };
            }
            output[index] = Some(reference);
        }
        if limit < count {
            engine_native_memory::batch_fallback::record(engine_native_memory::batch_fallback::Batch::BaseWitnesses);
        }
        limit
    }

    fn first_province(
        &mut self,
        memory: &impl Memory,
        game_state: usize,
        state: u32,
        kind: i32,
    ) -> Result<Option<u32>, &'static str> {
        let manager = match self.manager {
            Some(manager) => manager,
            None => {
                let manager = pointer(memory, game_state, cgame_state::STRATEGIC_AIR_MANAGER)?;
                authenticate(
                    memory,
                    self.base,
                    manager,
                    cstrategic_air_manager::VTABLE_RVA,
                )?;
                *self.manager.insert(manager)
            }
        };
        let (index, offset) = match kind {
            1 => (1, cstrategic_air_manager::KIND1_STATE_PROVINCE_LISTS),
            2 => (2, cstrategic_air_manager::KIND2_STATE_PROVINCE_LISTS),
            _ => (0, cstrategic_air_manager::DEFAULT_STATE_PROVINCE_LISTS),
        };
        const LIST_INVALID: &str = "air manager state province list invalid";
        let lists = match self.lists[index] {
            Some(lists) => lists,
            None => *self.lists[index].insert(
                manager
                    .checked_add(offset)
                    .and_then(|at| Array::read(memory, at, engine_native_memory::cpdx_array::STRIDE))
                    .ok_or(LIST_INVALID)?,
            ),
        };
        let list = lists
            .element(state, engine_native_memory::cpdx_array::STRIDE)
            .and_then(|at| Array::read(memory, at, 4))
            .ok_or(LIST_INVALID)?;
        let Some(first) = list.element(0, 4) else {
            return Ok(None);
        };
        let id = field::<u32>(memory, first, 0)?;
        const OUTSIDE: &str = "air manager first province outside the province roster";
        let provinces = match self.provinces {
            Some(provinces) => provinces,
            None => *self.provinces.insert(
                game_state
                    .checked_add(cgame_state::PROVINCES)
                    .and_then(|at| Array::read(memory, at, 8))
                    .ok_or(OUTSIDE)?,
            ),
        };
        let province = provinces.element(id, 8).ok_or(OUTSIDE)?;
        let province = pointer(memory, province, 0)?;
        authenticate(memory, self.base, province, cprovince::VTABLE_RVA)?;
        if field::<u32>(memory, province, cprovince::ID)? != id {
            return Err("air manager first province identity mismatch");
        }
        Ok(Some(id))
    }

    pub fn read_host_owner(
        &mut self,
        memory: &impl Memory,
        handle: u64,
    ) -> Result<Host, &'static str> {
        let base = self.base;
        let Some(ship) = self.resolver.resolve(memory, handle)? else {
            return Ok(Host::Unresolved);
        };
        authenticate(memory, base, ship, cship::VTABLE_RVA)?;
        if field::<u64>(memory, ship, cship::HANDLE)? != handle {
            return Err("carrier host reference handle mismatch");
        }
        let task_force = pointer(memory, ship, cship::TASK_FORCE)?;
        if task_force == 0 {
            return Ok(Host::NoTaskForce);
        }
        authenticate(memory, base, task_force, ctask_force::VTABLE_RVA)?;
        let secondary = task_force
            .checked_add(ctask_force::SECONDARY_BASE)
            .ok_or("carrier task force address overflow")?;
        authenticate(memory, base, secondary, ctask_force::SECONDARY_VTABLE_RVA)?;
        Ok(Host::Owner(field(memory, task_force, cunit::ROSTER_OWNER)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Unresolved,
    NoTaskForce,
    Owner(u32),
}

pub fn read_host_owner(
    memory: &impl Memory,
    base: usize,
    handle: u64,
) -> Result<Host, &'static str> {
    Pass::new(base).read_host_owner(memory, handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::reference_bucket as bucket;
    use std::collections::BTreeMap;

    const BASE: usize = 0x30_000;
    const STATE: usize = 0x32_000;
    const PROVINCE: usize = 0x34_000;
    const SHIP: usize = 0x36_000;
    const TASK_FORCE: usize = 0x38_000;
    const GAME: usize = 0x3a_000;
    const MANAGER: usize = 0x3b_000;
    const LISTS: usize = 0x3c_000;
    const IDS: usize = 0x3d_000;
    const PROVINCES: usize = 0x3e_000;
    const BASE_HANDLE: u64 = (10_u64 << 32) | engine_native_parts::cair_base_layout::TYPE as u64;
    const SHIP_HANDLE: u64 = (12_u64 << 32) | cship::HANDLE_TYPE as u64;

    #[derive(Default)]
    struct Fixture(BTreeMap<usize, u8>);

    impl Fixture {
        fn bytes(&mut self, at: usize, value: &[u8]) {
            for (index, byte) in value.iter().enumerate() {
                self.0.insert(at + index, *byte);
            }
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.bytes(at, &value.to_le_bytes());
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.bytes(at, &value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.u64(at, data as u64);
            self.u32(at + 8, count);
            self.u32(at + 12, count);
        }
        fn unresolved(&mut self, handle: u64) {
            self.u64(
                creference_registry::LOW_ROOTS_RVA.0 + (handle as u32) as usize * 8,
                0,
            );
        }
        fn register(&mut self, handle: u64, object: usize) {
            let kind = handle as u32 as usize;
            let table = 0x4000 + kind * 0x100;
            let entries = 0x10_000 + kind * 0x1000;
            self.u64(
                creference_registry::LOW_ROOTS_RVA.0 + kind * 8,
                table as u64,
            );
            self.u64(table + creference_registry::ENTRIES, entries as u64);
            self.u32(table + creference_registry::COUNT, 1);
            self.u32(table + creference_registry::MASK, 7);
            self.bytes(table + creference_registry::OVERFLOW_DISTANCE, &[5]);
            self.bytes(entries, &[0; 14 * bucket::STRIDE]);
            self.bytes(entries + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(creference_registry::HASH_MULTIPLIER);
            let first = ((hash >> 32) as u32 ^ hash as u32) & 7;
            let cell = entries + first as usize * bucket::STRIDE;
            self.bytes(cell + bucket::DISTANCE, &[1]);
            self.u64(cell + bucket::TYPE, handle);
            self.u64(cell + bucket::OBJECT, object as u64);
        }
        fn base(&mut self, state: usize, province: usize, host: u64, kind: i32) {
            self.register(BASE_HANDLE, BASE);
            self.u64(BASE, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
            self.u64(BASE + engine_native_parts::cair_base_layout::HANDLE, BASE_HANDLE);
            self.u64(BASE + engine_native_parts::cair_base_layout::HOST_REFERENCE, host);
            self.u64(BASE + cair_base::STATE, state as u64);
            self.u64(BASE + cair_base::PROVINCE, province as u64);
            self.u32(BASE + cair_base::KIND, kind as u32);
        }
        fn manager(&mut self) {
            self.u64(GAME + cgame_state::STRATEGIC_AIR_MANAGER, MANAGER as u64);
            self.u64(MANAGER, cstrategic_air_manager::VTABLE_RVA.0 as u64);
            self.array(
                MANAGER + cstrategic_air_manager::KIND2_STATE_PROVINCE_LISTS,
                LISTS,
                8,
            );
            self.array(LISTS + 7 * engine_native_memory::cpdx_array::STRIDE, IDS, 2);
            self.u32(IDS, 23);
            self.u32(IDS + 4, 24);
            self.array(GAME + cgame_state::PROVINCES, PROVINCES, 30);
            self.u64(PROVINCES + 23 * 8, PROVINCE as u64);
            self.u32(PROVINCE + cprovince::ID, 23);
        }
    }

    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    #[test]
    fn an_unregistered_handle_is_a_miss_not_an_error() {
        let mut memory = Fixture::default();
        memory.unresolved(BASE_HANDLE);
        memory.unresolved(SHIP_HANDLE);
        assert_eq!(read_base(&memory, 0, GAME, BASE_HANDLE), Ok(None));
        assert_eq!(read_base(&memory, 0, GAME, 0), Ok(None));
        assert_eq!(
            read_host_owner(&memory, 0, SHIP_HANDLE),
            Ok(Host::Unresolved)
        );
    }

    #[test]
    fn a_state_base_takes_its_cached_province_or_the_managers_first() {
        let mut memory = Fixture::default();
        memory.base(STATE, PROVINCE, 0, 2);
        memory.u64(STATE, cstate::VTABLE_RVA.0 as u64);
        memory.u32(STATE + cstate::ID, 7);
        memory.u64(PROVINCE, cprovince::VTABLE_RVA.0 as u64);
        memory.u32(PROVINCE + cprovince::ID, 19);
        let expected = BaseReference {
            object: BASE,
            state: Some(7),
            province: Some(19),
            host: 0,
            kind: 2,
        };
        assert_eq!(read_base(&memory, 0, GAME, BASE_HANDLE), Ok(Some(expected)));
        memory.u64(BASE + cair_base::PROVINCE, 0);
        assert!(read_base(&memory, 0, GAME, BASE_HANDLE).is_err());
        memory.manager();
        assert_eq!(
            read_base(&memory, 0, GAME, BASE_HANDLE),
            Ok(Some(BaseReference {
                province: Some(23),
                ..expected
            }))
        );
        memory.u32(LISTS + 7 * engine_native_memory::cpdx_array::STRIDE + 12, 0);
        assert_eq!(
            read_base(&memory, 0, GAME, BASE_HANDLE),
            Ok(Some(BaseReference {
                province: None,
                ..expected
            }))
        );
    }

    #[test]
    fn a_carrier_base_keeps_its_host_whose_task_force_owner_is_read() {
        let mut memory = Fixture::default();
        memory.base(0, PROVINCE, SHIP_HANDLE, 0);
        assert_eq!(
            read_base(&memory, 0, GAME, BASE_HANDLE),
            Ok(Some(BaseReference {
                object: BASE,
                state: None,
                province: None,
                host: SHIP_HANDLE,
                kind: 0,
            }))
        );
        memory.register(SHIP_HANDLE, SHIP);
        memory.u64(SHIP, cship::VTABLE_RVA.0 as u64);
        memory.u64(SHIP + cship::HANDLE, SHIP_HANDLE);
        memory.u64(SHIP + cship::TASK_FORCE, TASK_FORCE as u64);
        memory.u64(TASK_FORCE, ctask_force::VTABLE_RVA.0 as u64);
        memory.u64(
            TASK_FORCE + ctask_force::SECONDARY_BASE,
            ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
        );
        memory.u32(TASK_FORCE + cunit::ROSTER_OWNER, 42);
        assert_eq!(
            read_host_owner(&memory, 0, SHIP_HANDLE),
            Ok(Host::Owner(42))
        );
        memory.u64(TASK_FORCE + ctask_force::SECONDARY_BASE, 0);
        assert!(read_host_owner(&memory, 0, SHIP_HANDLE).is_err());
        memory.u64(SHIP + cship::TASK_FORCE, 0);
        assert_eq!(
            read_host_owner(&memory, 0, SHIP_HANDLE),
            Ok(Host::NoTaskForce)
        );
        memory.u64(BASE + engine_native_parts::cair_base_layout::HANDLE, BASE_HANDLE + 1);
        assert_eq!(
            read_base(&memory, 0, GAME, BASE_HANDLE),
            Err("air base reference identity mismatch")
        );
    }

    struct Scale {
        bytes: Vec<u8>,
        handles: Vec<u64>,
    }

    impl Scale {
        const WORLD: usize = 0x1000_0000;
        const MODULE: usize = Self::WORLD + 0x100 - creference_registry::LOW_ROOTS_RVA.0;
        const TABLE: usize = 0x1000;
        const ENTRIES: usize = 0x2000;
        const EXPONENT: u32 = 14;
        const GAME: usize = 0x80_000;
        const MANAGER: usize = 0x81_000;
        const LISTS: usize = 0x82_000;
        const IDS: usize = 0x88_000;
        const PROVINCES: usize = 0x8a_000;
        const BASES: usize = 0x100_000;
        const STATES: usize = 0x400_000;
        const PROVINCE_OBJECTS: usize = 0x500_000;
        const STATE_COUNT: usize = 100;

        fn address(offset: usize) -> usize {
            Self::WORLD + offset
        }

        fn handle(id: u64) -> u64 {
            (id << 32) | u64::from(engine_native_parts::cair_base_layout::TYPE)
        }

        fn put(&mut self, offset: usize, value: &[u8]) {
            self.bytes[offset..offset + value.len()].copy_from_slice(value);
        }

        fn pointer(&mut self, offset: usize, target: usize) {
            self.put(offset, &(Self::address(target) as u64).to_le_bytes());
        }

        fn array(&mut self, offset: usize, data: usize, count: u32) {
            self.pointer(offset, data);
            self.put(offset + 8, &count.to_le_bytes());
            self.put(offset + 12, &count.to_le_bytes());
        }

        fn new(bases: usize, misses: usize) -> Self {
            let mut world = Self {
                bytes: vec![0; (Self::BASES + bases * 0x80).max(0x600_000)],
                handles: Vec::new(),
            };
            let module = Self::MODULE;
            let vtable = |rva: Rva| ((module + rva.0) as u64).to_le_bytes();
            let capacity = 1_usize << Self::EXPONENT;
            let overflow = Self::EXPONENT as usize + 2;
            world.pointer(0x100 + engine_native_parts::cair_base_layout::TYPE as usize * 8, Self::TABLE);
            world.pointer(Self::TABLE + creference_registry::ENTRIES, Self::ENTRIES);
            world.put(
                Self::TABLE + creference_registry::COUNT,
                &(bases as u32).to_le_bytes(),
            );
            world.put(
                Self::TABLE + creference_registry::MASK,
                &(capacity as u32 - 1).to_le_bytes(),
            );
            world.put(
                Self::TABLE + creference_registry::OVERFLOW_DISTANCE,
                &[overflow as u8],
            );
            world.put(
                Self::ENTRIES + (capacity + overflow) * bucket::STRIDE + bucket::DISTANCE,
                &[255],
            );
            world.pointer(
                Self::GAME + cgame_state::STRATEGIC_AIR_MANAGER,
                Self::MANAGER,
            );
            world.put(Self::MANAGER, &vtable(cstrategic_air_manager::VTABLE_RVA));
            for offset in [
                cstrategic_air_manager::DEFAULT_STATE_PROVINCE_LISTS,
                cstrategic_air_manager::KIND1_STATE_PROVINCE_LISTS,
                cstrategic_air_manager::KIND2_STATE_PROVINCE_LISTS,
            ] {
                world.array(
                    Self::MANAGER + offset,
                    Self::LISTS,
                    Self::STATE_COUNT as u32,
                );
            }
            world.array(
                Self::GAME + cgame_state::PROVINCES,
                Self::PROVINCES,
                Self::STATE_COUNT as u32 + 2,
            );
            for state in 0..Self::STATE_COUNT {
                let object = Self::STATES + state * 0x80;
                world.put(object, &vtable(cstate::VTABLE_RVA));
                world.put(object + cstate::ID, &(state as u32).to_le_bytes());
                world.array(
                    Self::LISTS + state * engine_native_memory::cpdx_array::STRIDE,
                    Self::IDS + state * 4,
                    1,
                );
                let province = state + 1;
                world.put(Self::IDS + state * 4, &(province as u32).to_le_bytes());
                let object = Self::PROVINCE_OBJECTS + province * 0xb0;
                world.pointer(Self::PROVINCES + province * 8, object);
                world.put(object, &vtable(cprovince::VTABLE_RVA));
                world.put(object + cprovince::ID, &(province as u32).to_le_bytes());
            }
            for id in 1..=bases {
                let handle = Self::handle(id as u64);
                let object = Self::BASES + (id - 1) * 0x80;
                world.put(object, &vtable(engine_native_parts::cair_base_layout::VTABLE_RVA));
                world.put(object + engine_native_parts::cair_base_layout::HANDLE, &handle.to_le_bytes());
                world.put(
                    object + engine_native_parts::cair_base_layout::HOST_REFERENCE,
                    &(id as u64 * 3).to_le_bytes(),
                );
                let state = id % Self::STATE_COUNT;
                if id % 5 != 0 {
                    world.pointer(object + cair_base::STATE, Self::STATES + state * 0x80);
                    if id % 2 == 0 {
                        world.pointer(
                            object + cair_base::PROVINCE,
                            Self::PROVINCE_OBJECTS + (state + 1) * 0xb0,
                        );
                    }
                }
                world.put(object + cair_base::KIND, &((id % 3) as u32).to_le_bytes());
                world.insert(handle, Self::address(object), capacity);
                world.handles.push(handle);
            }
            world
                .handles
                .extend((bases + 1..=bases + misses).map(|id| Self::handle(id as u64)));
            world.handles.sort_unstable();
            world
        }

        fn insert(&mut self, handle: u64, object: usize, capacity: usize) {
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(creference_registry::HASH_MULTIPLIER);
            let mut index = (((hash >> 32) as u32 ^ hash as u32) as usize) & (capacity - 1);
            let (mut handle, mut object, mut distance) = (handle, object as u64, 1_u8);
            loop {
                let cell = Self::ENTRIES + index * bucket::STRIDE;
                let stored = self.bytes[cell + bucket::DISTANCE];
                if stored == 0 {
                    self.bytes[cell + bucket::DISTANCE] = distance;
                    self.put(cell + bucket::TYPE, &handle.to_le_bytes());
                    self.put(cell + bucket::OBJECT, &object.to_le_bytes());
                    return;
                }
                if stored < distance {
                    let held = (
                        u64::from_le_bytes(
                            self.bytes[cell + bucket::TYPE..][..8].try_into().unwrap(),
                        ),
                        u64::from_le_bytes(
                            self.bytes[cell + bucket::OBJECT..][..8].try_into().unwrap(),
                        ),
                    );
                    self.bytes[cell + bucket::DISTANCE] = distance;
                    self.put(cell + bucket::TYPE, &handle.to_le_bytes());
                    self.put(cell + bucket::OBJECT, &object.to_le_bytes());
                    (handle, object, distance) = (held.0, held.1, stored);
                }
                index += 1;
                distance += 1;
                assert!(distance as usize <= Self::EXPONENT as usize + 3);
            }
        }

        fn memory(&self) -> engine_native_memory::memory::SliceMemory<'_> {
            engine_native_memory::memory::SliceMemory {
                base: Self::WORLD,
                bytes: &self.bytes,
            }
        }
    }

    type Witnesses = (Vec<Option<BaseReference>>, Result<(), &'static str>);

    fn serial(memory: &impl Memory, handles: &[u64]) -> Witnesses {
        let mut pass = Pass::new(Scale::MODULE);
        let game = Scale::address(Scale::GAME);
        let mut references = Vec::new();
        for &handle in handles {
            match pass.read_base(memory, game, handle) {
                Ok(reference) => references.push(reference),
                Err(reason) => return (references, Err(reason)),
            }
        }
        (references, Ok(()))
    }

    fn gathered(memory: &impl Memory, handles: &[u64]) -> (Witnesses, usize) {
        use engine_native_memory::batch_fallback::{Batch, count};
        let counted = count(Batch::BaseWitnesses);
        let mut pass = Pass::new(Scale::MODULE);
        let game = Scale::address(Scale::GAME);
        let mut fallbacks = 0;
        let mut references = Vec::new();
        let mut output = [None; BASE_BATCH];
        let result = (|| {
            for chunk in handles.chunks(BASE_BATCH) {
                let read = pass.read_bases(memory, game, chunk, &mut output);
                fallbacks += usize::from(read < chunk.len());
                references.extend_from_slice(&output[..read]);
                for &handle in &chunk[read..] {
                    references.push(pass.read_base(memory, game, handle)?);
                }
            }
            Ok(())
        })();
        assert_eq!(count(Batch::BaseWitnesses) - counted, fallbacks as u64);
        ((references, result), fallbacks)
    }

    #[test]
    fn gathered_bases_equal_the_per_object_reads() {
        let world = Scale::new(150, 20);
        let memory = world.memory();
        let expected = serial(&memory, &world.handles);
        assert_eq!(expected.1, Ok(()));
        assert_eq!(expected.0.iter().flatten().count(), 150);
        let cached = |base: &BaseReference| {
            let at = base.object - Scale::WORLD + cair_base::PROVINCE;
            u64::from_le_bytes(world.bytes[at..at + 8].try_into().unwrap()) != 0
        };
        assert!(
            expected
                .0
                .iter()
                .flatten()
                .any(|base| base.province.is_some() && !cached(base))
        );
        assert_eq!(gathered(&memory, &world.handles), (expected, 0));
        let counted = |gather: bool| {
            let memory = engine_native_memory::memory::CountingMemory {
                inner: world.memory(),
                calls: std::cell::Cell::new(0),
                bytes: std::cell::Cell::new(0),
            };
            let read = if gather {
                gathered(&memory, &world.handles).0
            } else {
                serial(&memory, &world.handles)
            };
            assert_eq!(read.1, Ok(()));
            memory.calls.get()
        };
        let (serial, gathered) = (counted(false), counted(true));
        assert!(
            serial >= gathered + 3 * world.handles.len(),
            "{serial} {gathered}"
        );
    }

    struct Faulted<'a> {
        inner: engine_native_memory::memory::SliceMemory<'a>,
        fault: usize,
        once: bool,
        met: std::cell::Cell<bool>,
    }

    impl Memory for Faulted<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let touches = address <= self.fault && self.fault < address + destination.len();
            if touches && !(self.once && self.met.replace(true)) {
                return false;
            }
            self.inner.copy(address, destination)
        }
    }

    #[test]
    fn gathered_base_faults_refuse_no_more_than_the_per_object_reads() {
        let world = Scale::new(150, 20);
        let healthy = serial(&world.memory(), &world.handles);
        let base = |id: usize| Scale::BASES + (id - 1) * 0x80;
        let bucket_of = |handle: u64| {
            let memory = world.memory();
            let mut resolver = creference_registry::ScopedResolver::new(Scale::MODULE);
            resolver.first_cell(&memory, handle).unwrap().unwrap().1 - Scale::WORLD
        };
        let mut faults = vec![
            Scale::TABLE + creference_registry::ENTRIES,
            bucket_of(Scale::handle(70)),
            bucket_of(Scale::handle(150)),
            Scale::STATES + 7 * 0x80,
            Scale::STATES + 7 * 0x80 + cstate::ID,
            Scale::PROVINCE_OBJECTS + 8 * 0xb0,
            Scale::LISTS + 9 * engine_native_memory::cpdx_array::STRIDE,
            Scale::PROVINCES + 10 * 8,
        ];
        for id in [1, 64, 65, 70, 128, 150] {
            faults.extend([
                base(id),
                base(id) + engine_native_parts::cair_base_layout::HANDLE,
                base(id) + cair_base::STATE,
                base(id) + cair_base::KIND,
            ]);
        }
        for fault in faults {
            let faulted = |once| Faulted {
                inner: world.memory(),
                fault: Scale::address(fault),
                once,
                met: std::cell::Cell::new(false),
            };
            let reference = serial(&faulted(false), &world.handles);
            assert_ne!(reference, healthy, "{fault:x}");
            assert_eq!(
                gathered(&faulted(false), &world.handles).0,
                reference,
                "{fault:x}"
            );
            let (once, reference) = (
                gathered(&faulted(true), &world.handles).0,
                serial(&faulted(true), &world.handles),
            );
            assert!(once == healthy || once == reference, "{fault:x}: {once:?}");
        }
        let faulted = Faulted {
            inner: world.memory(),
            fault: Scale::address(base(70)),
            once: true,
            met: std::cell::Cell::new(false),
        };
        assert_eq!(gathered(&faulted, &world.handles), (healthy, 1));
    }
}
