//! Embedded raid location scope retaining every observed arm. Province
//! resolution tries the province, target building province, secondary province,
//! then state when allowed. The building arm records its lexer type token
//! for `LexicalTokens` and raid-map sprite selection.

use crate::{
    Memory, cbuilding, cbuilding_status, cprovince, cstate, raid_location_scope_layout as scope,
    read,
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// `scope + 8` CProvince id; zero when that arm is null.
    pub province: u32,
    /// `scope + 0x20` CProvince id, the save's `leader_province`; zero when
    /// that arm is null.
    pub leader_province: u32,
    /// `scope + 0x10` CState id; zero when that arm is null.
    pub state: u32,
    /// `scope + 0` CBuilding's lexer type token; zero when that arm is null.
    pub building_type: u32,
    /// The target building's `CBuildingStatus + 0x6c` location id, which the
    /// save writes as `target.building.location`; zero without a building arm.
    pub building_location: u32,
    /// Building-location flags: bit 0 selects province and bit 1 selects state.
    pub building_flags: u32,
    /// Raw `CBuildingStatus + 0x70` country override; the country resolver
    /// uses it when positive, else the location's controller.
    pub building_country: i32,
    /// `scope + 0x18`, a full character reference; zero when that arm is null.
    pub leader_reference: u64,
    /// Resolved leader's captor is the target country. `None` when building,
    /// province or state answers first, or an unresolved reference falls through
    /// to the leader province; those branches do not read the leader.
    pub leader_country: Option<i32>,
    /// The leader arm resolved to an object whose captor is not proven (not a
    /// `CArmyLeader`, or unreadable). Not recorded: the walks refuse
    /// and count just this row.
    pub leader_refused: bool,
}

/// The scope's building arm, at offset zero.
pub const BUILDING: usize = 0x00;
/// Province arm serialized as `province`.
pub const PROVINCE_ARM: usize = scope::SECOND_POINTER;
/// The state arm (`state`).
pub const STATE_ARM: usize = scope::THIRD_POINTER;
/// The character reference arm (`leader`).
pub const LEADER_REFERENCE: usize = 0x18;
/// The leader's province arm (`leader_province`).
pub const LEADER_PROVINCE_ARM: usize = scope::FIFTH_POINTER;

/// Army leaders hold the captor within their object. Navy leaders are too
/// small, and base/operative layouts do not establish this field. Any other
/// class refuses the row.
const CAPTOR_VTABLE: usize = crate::carmy_leader::VTABLE_RVA;

/// Leader arm is reached only when building, province and state arms are null.
pub(crate) fn reads_leader(building: bool, province: u32, state: u32) -> bool {
    !building && province == 0 && state == 0
}

/// The leader arm's captor: `Ok(None)` when the reference does not resolve,
/// `Err(())` when it resolves to an object whose captor is not proven.
pub(crate) fn leader_captor(
    memory: &impl Memory,
    base: usize,
    handle: u64,
) -> Result<Option<i32>, ()> {
    let Some(leader) = crate::creference_registry::resolve(memory, base, handle).map_err(|_| ())?
    else {
        return Ok(None);
    };
    let vtable = read::<u64>(memory, leader)
        .and_then(|vtable| usize::try_from(vtable).ok())
        .and_then(|vtable| vtable.checked_sub(base))
        .ok_or(())?;
    if vtable != CAPTOR_VTABLE {
        return Err(());
    }
    leader
        .checked_add(crate::carmy_leader::CAPTOR_COUNTRY)
        .and_then(|at| read::<i32>(memory, at))
        .map(Some)
        .ok_or(())
}

/// The scope's five arms, contiguous from its start.
pub(crate) const ARMS_BYTES: usize = LEADER_PROVINCE_ARM + 8;

pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    address: usize,
) -> Result<Row, &'static str> {
    let mut arms = [0_u8; ARMS_BYTES];
    if !crate::memory::read_slice(memory, address, &mut arms) {
        return Err("raid target scope unreadable");
    }
    row_from_arms(memory, base, &arms)
}

/// Resolve a scope whose arms were already copied (in a caller's batch).
pub(crate) fn row_from_arms(
    memory: &impl Memory,
    base: usize,
    arms: &[u8; ARMS_BYTES],
) -> Result<Row, &'static str> {
    let arm = |at: usize| -> [u8; 8] { arms[at..at + 8].try_into().unwrap() };
    let (building, province, state, secondary, leader) = (
        arm(BUILDING),
        arm(PROVINCE_ARM),
        arm(STATE_ARM),
        arm(LEADER_PROVINCE_ARM),
        arm(LEADER_REFERENCE),
    );
    let pointer = |bytes: [u8; 8]| {
        usize::try_from(u64::from_le_bytes(bytes))
            .map_err(|_| "raid target scope arm address overflow")
    };
    // Each arm's identity and id in one guarded scattered copy.
    let identified = |pointer: usize, vtable: usize, id: usize, what: &'static str| {
        if pointer == 0 {
            return Ok(0);
        }
        let expected = base.checked_add(vtable).ok_or("module address overflow")?;
        let (mut identity, mut value) = ([0_u8; 8], [0_u8; 4]);
        let at = pointer.checked_add(id).ok_or(what)?;
        if memory.copy_scattered([pointer, at], &mut [&mut identity, &mut value]) != 2 {
            return Err(what);
        }
        if u64::from_le_bytes(identity) != expected as u64 {
            return Err(what);
        }
        Ok(u32::from_le_bytes(value))
    };
    let building = pointer(building)?;
    let mut building_type = 0;
    let (building_location, building_flags, building_country) = if building == 0 {
        (0, 0, 0)
    } else {
        let what = "raid target building unreadable or not a building";
        let (mut identity, mut kind, mut status) = ([0_u8; 8], [0_u8; 4], [0_u8; 8]);
        let at = |offset: usize| building.checked_add(offset).ok_or(what);
        if memory.copy_scattered(
            [
                at(0)?,
                at(cbuilding::TYPE_ID)?,
                at(cbuilding::BUILDING_STATUS)?,
            ],
            &mut [&mut identity, &mut kind, &mut status],
        ) != 3
        {
            return Err(what);
        }
        let expected = base
            .checked_add(cbuilding::VTABLE_RVA)
            .ok_or("module address overflow")?;
        if u64::from_le_bytes(identity) != expected as u64 {
            return Err(what);
        }
        building_type = u32::from_le_bytes(kind);
        let status = usize::try_from(u64::from_le_bytes(status))
            .ok()
            .filter(|&pointer| pointer != 0)
            .ok_or("raid target building status unreadable or null")?;
        let unreadable = "raid target building location unreadable";
        let at = |offset: usize| status.checked_add(offset).ok_or(unreadable);
        let (mut identity, mut id, mut flags, mut country) =
            ([0_u8; 8], [0_u8; 4], [0_u8; 4], [0_u8; 4]);
        let copied = memory.copy_scattered(
            [
                status,
                at(cbuilding_status::ID)?,
                at(cbuilding_status::FLAGS)?,
                at(cbuilding_status::COUNTRY)?,
            ],
            &mut [&mut identity, &mut id, &mut flags, &mut country],
        );
        if copied == 0 {
            return Err("raid target building status vtable mismatch");
        }
        let expected = base
            .checked_add(cbuilding_status::VTABLE_RVA)
            .ok_or("module address overflow")?;
        if u64::from_le_bytes(identity) != expected as u64 {
            return Err("raid target building status vtable mismatch");
        }
        if copied != 4 {
            return Err(unreadable);
        }
        (
            u32::from_le_bytes(id),
            u32::from_le_bytes(flags),
            i32::from_le_bytes(country),
        )
    };
    let leader_reference = u64::from_le_bytes(leader);
    let province = identified(
        pointer(province)?,
        cprovince::VTABLE_RVA,
        cprovince::ID,
        "raid target province unreadable or not a province",
    )?;
    let state = identified(
        pointer(state)?,
        cstate::VTABLE_RVA,
        cstate::ID,
        "raid target state unreadable or not a state",
    )?;
    let (leader_country, leader_refused) = if reads_leader(building != 0, province, state) {
        match leader_captor(memory, base, leader_reference) {
            Ok(captor) => (captor, false),
            Err(()) => (None, true),
        }
    } else {
        (None, false)
    };
    Ok(Row {
        building_location,
        building_flags,
        building_country,
        leader_reference,
        leader_country,
        leader_refused,
        province,
        leader_province: identified(
            pointer(secondary)?,
            cprovince::VTABLE_RVA,
            cprovince::ID,
            "raid target leader province unreadable or not a province",
        )?,
        state,
        building_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const SCOPE: usize = 0x100;
    const BUILDING_AT: usize = 0x200;
    const PROVINCE_AT: usize = 0x300;
    const STATE_AT: usize = 0x400;
    const STATUS_AT: usize = 0x500;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x600];
        put(&mut bytes, SCOPE, &(BUILDING_AT as u64).to_le_bytes());
        put(
            &mut bytes,
            SCOPE + scope::SECOND_POINTER,
            &(PROVINCE_AT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SCOPE + scope::THIRD_POINTER,
            &(STATE_AT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            BUILDING_AT,
            &(cbuilding::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            BUILDING_AT + cbuilding::TYPE_ID,
            &0x2fb6_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BUILDING_AT + cbuilding::BUILDING_STATUS,
            &(STATUS_AT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_AT,
            &(cbuilding_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_AT + cbuilding_status::ID,
            &9421_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_AT + cbuilding_status::FLAGS,
            &cbuilding_status::PROVINCE_FLAG.to_le_bytes(),
        );
        put(
            &mut bytes,
            PROVINCE_AT,
            &(cprovince::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            PROVINCE_AT + cprovince::ID,
            &6521_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATE_AT,
            &(cstate::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, STATE_AT + cstate::ID, &64_u32.to_le_bytes());
        bytes
    }

    fn read(bytes: &[u8]) -> Result<Row, &'static str> {
        read_row(&SliceMemory { base: 0, bytes }, 0, SCOPE)
    }

    #[test]
    fn every_arm_is_recorded_separately() {
        assert_eq!(
            read(&fixture()),
            Ok(Row {
                province: 6521,
                leader_province: 0,
                state: 64,
                building_type: 0x2fb6,
                building_location: 9421,
                building_flags: cbuilding_status::PROVINCE_FLAG,
                ..Row::default()
            })
        );
    }

    #[test]
    fn a_null_arm_is_zero_and_a_wrong_object_is_refused() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            SCOPE + scope::SECOND_POINTER,
            &0_u64.to_le_bytes(),
        );
        assert_eq!(read(&bytes).unwrap().province, 0);
        let mut bytes = fixture();
        put(&mut bytes, PROVINCE_AT, &7_u64.to_le_bytes());
        assert!(read(&bytes).is_err());
    }

    /// Sparse memory at native addresses, for the reference registry roots.
    #[derive(Default)]
    struct Sparse(std::collections::BTreeMap<usize, u8>);

    impl Sparse {
        fn put(&mut self, at: usize, value: &[u8]) {
            self.0
                .extend(value.iter().enumerate().map(|(i, &b)| (at + i, b)));
        }
    }

    impl Memory for Sparse {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (i, byte) in destination.iter_mut().enumerate() {
                let Some(&value) = self.0.get(&(address + i)) else {
                    return false;
                };
                *byte = value;
            }
            true
        }
    }

    /// A scope whose only arm is a leader reference resolving to an object
    /// with `vtable` and captor 12; `province` also sets the province arm.
    fn leader_scope(vtable: usize, province: bool) -> Sparse {
        use crate::{creference_registry as registry, reference_bucket as bucket};
        const TABLE: usize = 0x1000;
        const ENTRIES: usize = 0x2000;
        const LEADER: usize = 0x4000;
        let kind = 7_u32;
        let handle = (u64::from(0x55_u32) << 32) | u64::from(kind);
        let mut memory = Sparse::default();
        memory.put(SCOPE, &[0; 0x28]);
        memory.put(SCOPE + LEADER_REFERENCE, &handle.to_le_bytes());
        if province {
            memory.put(SCOPE + PROVINCE_ARM, &(PROVINCE_AT as u64).to_le_bytes());
            memory.put(PROVINCE_AT, &(cprovince::VTABLE_RVA as u64).to_le_bytes());
            memory.put(PROVINCE_AT + cprovince::ID, &6521_u32.to_le_bytes());
        }
        memory.put(
            registry::LOW_ROOTS_RVA + kind as usize * 8,
            &(TABLE as u64).to_le_bytes(),
        );
        memory.put(TABLE + registry::ENTRIES, &(ENTRIES as u64).to_le_bytes());
        memory.put(TABLE + registry::COUNT, &1_u32.to_le_bytes());
        memory.put(TABLE + registry::MASK, &7_u32.to_le_bytes());
        memory.put(TABLE + registry::OVERFLOW_DISTANCE, &[5]);
        memory.put(ENTRIES, &[0; 14 * bucket::STRIDE]);
        memory.put(
            ENTRIES + 13 * bucket::STRIDE + bucket::DISTANCE,
            &[bucket::TERMINAL_DISTANCE],
        );
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let entry = ENTRIES + index * bucket::STRIDE;
        memory.put(entry + bucket::DISTANCE, &[1]);
        memory.put(entry + bucket::TYPE, &handle.to_le_bytes());
        memory.put(entry + bucket::OBJECT, &(LEADER as u64).to_le_bytes());
        memory.put(LEADER, &(vtable as u64).to_le_bytes());
        memory.put(
            LEADER + crate::carmy_leader::CAPTOR_COUNTRY,
            &12_i32.to_le_bytes(),
        );
        memory
    }

    #[test]
    fn an_army_leader_answers_only_when_every_earlier_arm_is_null() {
        let row = read_row(&leader_scope(CAPTOR_VTABLE, false), 0, SCOPE).unwrap();
        assert_eq!((row.leader_country, row.leader_refused), (Some(12), false));
        // The navy, operative and base classes are refused, row by row.
        for vtable in [
            crate::cnavy_leader::VTABLE_RVA,
            crate::coperative_leader::VTABLE_RVA,
            crate::cunit_leader::VTABLE_RVA,
        ] {
            let row = read_row(&leader_scope(vtable, false), 0, SCOPE).unwrap();
            assert_eq!((row.leader_country, row.leader_refused), (None, true));
        }
        // The province arm answers without reading the leader, so a foreign object
        // in the unreached leader arm is not refused.
        let row = read_row(&leader_scope(7, true), 0, SCOPE).unwrap();
        assert_eq!(row.province, 6521);
        assert_eq!((row.leader_country, row.leader_refused), (None, false));
    }

    #[test]
    fn an_unproven_leader_refuses_only_its_row() {
        let row = read_row(&leader_scope(7, false), 0, SCOPE).unwrap();
        assert_eq!((row.leader_country, row.leader_refused), (None, true));
    }
}
