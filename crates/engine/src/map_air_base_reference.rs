//! Reference-registry lookups for the air bases used by the flying-aircraft
//! map. Pool and transfer bases resolve through the registry. Transfer admission
//! rejects a transfer only when its handle resolves. A stateless base's controller
//! comes from its host ship and task force's roster owner; a null task force is
//! retained as such.

use crate::{
    Memory, cair_base, cgame_state, cpdx_array::Array, cprovince, creference_registry, cship,
    cstate, cstrategic_air_manager, ctask_force, cunit, memory::Raw, read,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BaseReference {
    /// The resolved CAirBase, acquisition context only.
    pub object: usize,
    /// Raw CState ID, the authored air-base slot key; None for a stateless base.
    pub state: Option<u32>,
    /// A state base's cached province, otherwise the first province in the
    /// manager's state/kind list.
    pub province: Option<u32>,
    /// Complete raw host handle. State bases also retain the invalid reference;
    /// every nonzero full handle is resolved independently of state.
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
    vtable_rva: usize,
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(vtable_rva)
        .ok_or("air reference module address overflow")?;
    if object == 0 || read::<u64>(memory, object) != Some(expected as u64) {
        return Err("air reference object identity mismatch");
    }
    Ok(())
}

/// `None` is a registry miss, even if the manager roster holds the handle.
/// A stateless base ignores its cached province, as the carrier controller does.
pub fn read_base(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    handle: u64,
) -> Result<Option<BaseReference>, &'static str> {
    let Some(object) = creference_registry::resolve(memory, base, handle)? else {
        return Ok(None);
    };
    let [mut vtable, mut stored, mut host, mut state, mut province] = [[0_u8; 8]; 5];
    let mut kind = [0_u8; 4];
    let at = |offset: usize| object.checked_add(offset).unwrap_or(0);
    if memory.copy_scattered(
        [
            object,
            at(cair_base::HANDLE),
            at(cair_base::HOST_REFERENCE),
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
        != base
            .checked_add(cair_base::VTABLE_RVA)
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
    let identified = |pointer: [u8; 8], vtable_rva: usize, id: usize| {
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
            first_province(memory, base, game_state, state, reference.kind)?
        };
    }
    Ok(Some(reference))
}

/// Selects the manager's state-indexed province list by base kind.
/// Province selection uses the first ID.
fn first_province(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    state: u32,
    kind: i32,
) -> Result<Option<u32>, &'static str> {
    let manager = pointer(memory, game_state, cgame_state::STRATEGIC_AIR_MANAGER)?;
    authenticate(memory, base, manager, cstrategic_air_manager::VTABLE_RVA)?;
    let lists = match kind {
        1 => cstrategic_air_manager::KIND1_STATE_PROVINCE_LISTS,
        2 => cstrategic_air_manager::KIND2_STATE_PROVINCE_LISTS,
        _ => cstrategic_air_manager::DEFAULT_STATE_PROVINCE_LISTS,
    };
    let list = manager
        .checked_add(lists)
        .and_then(|at| Array::read(memory, at, crate::cpdx_array::STRIDE))
        .and_then(|lists| lists.element(state, crate::cpdx_array::STRIDE))
        .and_then(|at| Array::read(memory, at, 4))
        .ok_or("air manager state province list invalid")?;
    let Some(first) = list.element(0, 4) else {
        return Ok(None);
    };
    let id = field::<u32>(memory, first, 0)?;
    let province = game_state
        .checked_add(cgame_state::PROVINCES)
        .and_then(|at| Array::read(memory, at, 8))
        .and_then(|provinces| provinces.element(id, 8))
        .ok_or("air manager first province outside the province roster")?;
    let province = pointer(memory, province, 0)?;
    authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
    if field::<u32>(memory, province, cprovince::ID)? != id {
        return Err("air manager first province identity mismatch");
    }
    Ok(Some(id))
}

/// A stateless base's host used to select its controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// The ship handle does not resolve: the native null controller.
    Unresolved,
    /// The ship's task force is null. Sortie efficiency handles this explicitly;
    /// task-force owner selection requires a nonnull task force.
    NoTaskForce,
    /// The task force's roster owner (`CUnit+0x1D8`).
    Owner(u32),
}

/// State bases also test the full host independently of state. A valid live
/// host adds three ship reads (24 bytes), two task-force identities (16 bytes)
/// and its owner (4 bytes) to the bounded registry lookup.
pub fn read_host_owner(
    memory: &impl Memory,
    base: usize,
    handle: u64,
) -> Result<Host, &'static str> {
    let Some(ship) = creference_registry::resolve(memory, base, handle)? else {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference_bucket as bucket;
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
    const BASE_HANDLE: u64 = (10_u64 << 32) | cair_base::TYPE as u64;
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
                creference_registry::LOW_ROOTS_RVA + (handle as u32) as usize * 8,
                0,
            );
        }
        /// A mask-7 table: capacity 8, overflow distance 5, sentinel cell 13.
        fn register(&mut self, handle: u64, object: usize) {
            let kind = handle as u32 as usize;
            let table = 0x4000 + kind * 0x100;
            let entries = 0x10_000 + kind * 0x1000;
            self.u64(creference_registry::LOW_ROOTS_RVA + kind * 8, table as u64);
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
            self.u64(BASE, cair_base::VTABLE_RVA as u64);
            self.u64(BASE + cair_base::HANDLE, BASE_HANDLE);
            self.u64(BASE + cair_base::HOST_REFERENCE, host);
            self.u64(BASE + cair_base::STATE, state as u64);
            self.u64(BASE + cair_base::PROVINCE, province as u64);
            self.u32(BASE + cair_base::KIND, kind as u32);
        }
        /// State 7's kind-2 list holds province 23 first.
        fn manager(&mut self) {
            self.u64(GAME + cgame_state::STRATEGIC_AIR_MANAGER, MANAGER as u64);
            self.u64(MANAGER, cstrategic_air_manager::VTABLE_RVA as u64);
            self.array(
                MANAGER + cstrategic_air_manager::KIND2_STATE_PROVINCE_LISTS,
                LISTS,
                8,
            );
            self.array(LISTS + 7 * crate::cpdx_array::STRIDE, IDS, 2);
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
        memory.u64(STATE, cstate::VTABLE_RVA as u64);
        memory.u32(STATE + cstate::ID, 7);
        memory.u64(PROVINCE, cprovince::VTABLE_RVA as u64);
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
        memory.u32(LISTS + 7 * crate::cpdx_array::STRIDE + 12, 0);
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
        memory.u64(SHIP, cship::VTABLE_RVA as u64);
        memory.u64(SHIP + cship::HANDLE, SHIP_HANDLE);
        memory.u64(SHIP + cship::TASK_FORCE, TASK_FORCE as u64);
        memory.u64(TASK_FORCE, ctask_force::VTABLE_RVA as u64);
        memory.u64(
            TASK_FORCE + ctask_force::SECONDARY_BASE,
            ctask_force::SECONDARY_VTABLE_RVA as u64,
        );
        memory.u32(TASK_FORCE + cunit::ROSTER_OWNER, 42);
        assert_eq!(
            read_host_owner(&memory, 0, SHIP_HANDLE),
            Ok(Host::Owner(42))
        );
        memory.u64(TASK_FORCE + ctask_force::SECONDARY_BASE, 0);
        assert!(read_host_owner(&memory, 0, SHIP_HANDLE).is_err());
        // A resolved ship whose task force is null is state, not a failure.
        memory.u64(SHIP + cship::TASK_FORCE, 0);
        assert_eq!(
            read_host_owner(&memory, 0, SHIP_HANDLE),
            Ok(Host::NoTaskForce)
        );
        memory.u64(BASE + cair_base::HANDLE, BASE_HANDLE + 1);
        assert_eq!(
            read_base(&memory, 0, GAME, BASE_HANDLE),
            Err("air base reference identity mismatch")
        );
    }
}
