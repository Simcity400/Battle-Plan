//! A nuclear drop's impact. No game code is called.
//!
//! Every launch, through an effect or command, only appends a delayed entry to its nuke object's
//! queue. Queue dispatch runs during country pending refresh in the serial country loop before the
//! hourly update, on the main thread only, and calls [`IMPACT`] once per ready entry. The impact:
//!
//! - logs the launcher (the nuke's launcher, passing on its tag word), province
//!   id and controller;
//!
//! - subtracts the province's nuclear war-support damage from the state owner's
//!   war support, resolving the owner through the province's state and its owner
//!   and using tag zero without a state;
//!
//! - for a type other than 1, damages the province's units ([`UNIT_DAMAGE`]),
//!   destroys aircraft in its air bases' wings and removes healthy building
//!   levels. Type 1 does all three for every province of the state
//!   (the state's province list);
//!
//! - removes healthy levels of the state's buildings, adds war score, then
//!   tail-jumps to [`NOTIFY`], its only entry, which dispatches the authored
//!   `on_nuke_drop` effects.
//!
//! A strike's outcome is therefore what runs between impact entry and
//! notification entry: closing it there keeps authored effects out. Type lookup
//! gives ordinals 0 and 1 their lexer tokens; the impact reads only whether the
//! type is 1.

use crate::{
    Memory, carmy, ccountry, cnuke,
    cpdx_array::Array,
    cprovince, crailway_gun, cstate, cunit, equipment_destruction,
    event_feed_sites::{CountryIdentity, identity},
    manpower_value_entry, read,
};

/// A function entry this family patches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Impact takes nuke and province pointers in RCX and RDX, saved immediately; no other argument is
/// read. The five patched bytes save RDX to its home slot.
pub const IMPACT: Site = Site {
    name: "impact",
    rva: 0x0108_3430,
    sentinel: [
        0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41,
        0x54,
    ],
};
/// Notification takes nuke and province pointers, entered only by the impact's tail jump; RCX and
/// RDX are kept in RSI and RDI. The five patched bytes save RBX to its home slot.
pub const NOTIFY: Site = Site {
    name: "notify",
    rva: 0x0108_4ff0,
    sentinel: [
        0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24, 0x20,
        0x55,
    ],
};
/// Unit damage takes nuke and province pointers and returns u64, called by both province-damage
/// branches of the impact. It walks the province's mixed occupants (up to their count) in order.
/// For each whose class word is not 1, it calls the immediate-damage virtual with random strength
/// and organisation fractions between the `NRaids` minimum and maximum, flag 1 and the launcher's
/// tag. The five patched bytes push RBX, RBP, RSI and RDI; RAX is returned unchanged.
pub const UNIT_DAMAGE: Site = Site {
    name: "unit_damage",
    rva: 0x0108_2b10,
    sentinel: [
        0x40, 0x53, 0x55, 0x56, 0x57, 0x41, 0x57, 0x48, 0x83, 0xec, 0x30, 0x83, 0x79, 0x38, 0x01,
        0x4c,
    ],
};
pub const SITES: [Site; 3] = [IMPACT, NOTIFY, UNIT_DAMAGE];

/// A drop's target and launcher, as the impact reads them on entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strike {
    /// The launcher named by its own reference.
    pub launcher: CountryIdentity,
    /// The native type ordinal of the nuke.
    pub nuke_type: u32,
    pub province: u32,
    /// The province's state id (`cstate::ID`); 0 for a province without a state, which has no state
    /// buildings and no war support recipient.
    pub state: u32,
    /// The war support recipient: the state owner's reference resolved, the
    /// reserved country (reference zero) without a state.
    pub owner: CountryIdentity,
    pub controller: u32,
}

fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("nuclear strike address overflow")
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    reason: &'static str,
) -> Result<T, &'static str> {
    read(memory, at(object, offset)?).ok_or(reason)
}

fn pointer(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    reason: &'static str,
) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset, reason)?).map_err(|_| reason)
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: usize,
    reason: &'static str,
) -> Result<(), &'static str> {
    let expected = base.checked_add(vtable).ok_or(reason)? as u64;
    if object == 0 || read::<u64>(memory, object) != Some(expected) {
        return Err(reason);
    }
    Ok(())
}

/// The impact's operands on entry: the nuke object and province it was
/// called with, authenticated by their vtables, and the full native IDs of
/// the launcher and the state owner at this hour.
pub fn strike(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    nuke: usize,
    province: usize,
) -> Result<Strike, &'static str> {
    authenticate(
        memory,
        base,
        nuke,
        cnuke::VTABLE_RVA,
        "nuclear strike nuke vtable mismatch",
    )?;
    authenticate(
        memory,
        base,
        province,
        cprovince::VTABLE_RVA,
        "nuclear strike province vtable mismatch",
    )?;
    let country = pointer(
        memory,
        nuke,
        cnuke::COUNTRY,
        "nuclear strike launcher unreadable",
    )?;
    let countries = ccountry::Reader::new(memory, base, game_state)?;
    let reference = field(
        memory,
        country,
        ccountry::ID,
        "nuclear strike launcher tag unreadable",
    )?;
    let (resolved, launcher) = identity(memory, base, &countries, reference)?;
    if resolved.pointer != country {
        return Err("nuclear strike launcher is not its own reference's country");
    }
    let state = pointer(
        memory,
        province,
        cprovince::STATE,
        "nuclear strike state unreadable",
    )?;
    let (state_id, owner) = if state == 0 {
        (0, 0)
    } else {
        authenticate(
            memory,
            base,
            state,
            cstate::VTABLE_RVA,
            "nuclear strike state vtable mismatch",
        )?;
        (
            field(
                memory,
                state,
                cstate::ID,
                "nuclear strike state id unreadable",
            )?,
            field(
                memory,
                state,
                cstate::OWNER,
                "nuclear strike state owner unreadable",
            )?,
        )
    };
    Ok(Strike {
        launcher,
        nuke_type: field(memory, nuke, cnuke::TYPE, "nuclear strike type unreadable")?,
        province: field(
            memory,
            province,
            cprovince::ID,
            "nuclear strike province id unreadable",
        )?,
        state: state_id,
        owner: identity(memory, base, &countries, owner)?.1,
        controller: field(
            memory,
            province,
            cprovince::CONTROLLER,
            "nuclear strike controller unreadable",
        )?,
    })
}

/// A division's class word (`cunit::CLASS`).
pub const ARMY_CLASS: u32 = 0;
/// The class [`UNIT_DAMAGE`] skips by comparing the occupant's class word (`cunit::CLASS`) with
/// one.
pub const SKIPPED_CLASS: u32 = 1;
/// A railway gun's class word, passed to unit construction by the railway-gun
/// constructor.
pub const RAILWAY_GUN_CLASS: u32 = 13;

/// One unit [`UNIT_DAMAGE`] damages: the fields its immediate-damage operation
/// writes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    pub handle: u64,
    pub class: u32,
    pub owner: u32,
    /// Signed fixed5: `carmy::STRENGTH`, `crailway_gun::CURRENT_STRENGTH`.
    pub strength: i64,
    /// Signed fixed5 `carmy::ORGANISATION`; a railway gun has none (its getter returns a constant
    /// and its damage operation ignores the organisation operand).
    pub organisation: Option<i64>,
    /// A division's manpower entries summed (`carmy::MANPOWER_VALUES`, the signed dwords the loss
    /// walker subtracts from); a railway gun's `crailway_gun::MANPOWER`.
    pub manpower: i64,
}

/// Visits, in native order, each occupant [`UNIT_DAMAGE`] damages: `Ok`
/// for a division or railway gun, `Err` naming any other class. The walk's
/// own failure is the outer error.
pub fn units(
    memory: &impl Memory,
    base: usize,
    province: usize,
    mut visit: impl FnMut(Result<Unit, &'static str>),
) -> Result<(), &'static str> {
    let occupants = Array::read(memory, at(province, cprovince::OCCUPANTS)?, 8)
        .ok_or("nuclear strike occupants unreadable")?;
    for index in 0..occupants.count {
        let unit = occupants
            .element(index, 8)
            .ok_or("nuclear strike occupant address overflow")
            .and_then(|slot| pointer(memory, slot, 0, "nuclear strike occupant unreadable"))?;
        let class = field::<u32>(
            memory,
            unit,
            cunit::CLASS,
            "nuclear strike unit class unreadable",
        )?;
        match class {
            SKIPPED_CLASS => {}
            ARMY_CLASS => visit(army(memory, base, unit)),
            RAILWAY_GUN_CLASS => visit(railway_gun(memory, base, unit)),
            _ => visit(Err("nuclear strike unit class unsupported")),
        }
    }
    Ok(())
}

fn army(memory: &impl Memory, base: usize, army: usize) -> Result<Unit, &'static str> {
    authenticate(
        memory,
        base,
        army,
        carmy::VTABLE_RVA,
        "nuclear strike division vtable mismatch",
    )?;
    let entries = Array::read(memory, at(army, carmy::MANPOWER_DATA)?, 8)
        .ok_or("nuclear strike division manpower unreadable")?;
    let mut manpower = 0_i64;
    for index in 0..entries.count {
        let entry = entries
            .element(index, manpower_value_entry::STRIDE)
            .ok_or("nuclear strike division manpower overflow")?;
        manpower += i64::from(field::<i32>(
            memory,
            entry,
            manpower_value_entry::COUNT,
            "nuclear strike division manpower unreadable",
        )?);
    }
    Ok(Unit {
        handle: field(
            memory,
            army,
            cunit::HANDLE,
            "nuclear strike division handle unreadable",
        )?,
        class: ARMY_CLASS,
        // Active division damage credits the expeditionary owner when positive.
        owner: equipment_destruction::army_owner(memory, army)?,
        strength: field(
            memory,
            army,
            carmy::STRENGTH,
            "nuclear strike division strength unreadable",
        )?,
        organisation: Some(field(
            memory,
            army,
            carmy::ORGANISATION,
            "nuclear strike division organisation unreadable",
        )?),
        manpower,
    })
}

fn railway_gun(memory: &impl Memory, base: usize, gun: usize) -> Result<Unit, &'static str> {
    authenticate(
        memory,
        base,
        gun,
        crailway_gun::VTABLE_RVA,
        "nuclear strike railway gun vtable mismatch",
    )?;
    Ok(Unit {
        handle: field(
            memory,
            gun,
            cunit::HANDLE,
            "nuclear strike railway gun handle unreadable",
        )?,
        class: RAILWAY_GUN_CLASS,
        // Railway-gun immediate damage resolves the casualty victim from the roster owner
        // (`cunit::ROSTER_OWNER`).
        owner: field(
            memory,
            gun,
            cunit::ROSTER_OWNER,
            "nuclear strike railway gun owner unreadable",
        )?,
        strength: field(
            memory,
            gun,
            crailway_gun::CURRENT_STRENGTH,
            "nuclear strike railway gun strength unreadable",
        )?,
        organisation: None,
        manpower: i64::from(field::<i32>(
            memory,
            gun,
            crailway_gun::MANPOWER,
            "nuclear strike railway gun manpower unreadable",
        )?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory};

    const NUKE: usize = 0x100;
    const PROVINCE: usize = 0x400;
    const STATE: usize = 0x800;
    const COUNTRY: usize = 0xa00;
    const OCCUPANTS: usize = 0xc00;
    const ARMY: usize = 0x1000;
    const GUN: usize = 0x2000;
    const SHIPS: usize = 0x3000;
    const MANPOWER: usize = 0x3800;
    const GAME: usize = 0x4000;
    const SLOTS: usize = 0x4400;
    const LINKS: usize = 0x4500;
    const RESERVED: usize = 0x4600;
    const OWNER: usize = 0x4700;

    struct World(Vec<u8>);
    impl World {
        fn new() -> Self {
            let mut world = Self(vec![0; 0x4800]);
            world.u64(NUKE, cnuke::VTABLE_RVA as u64);
            world.u64(NUKE + cnuke::COUNTRY, COUNTRY as u64);
            world.u32(NUKE + cnuke::TYPE, 1);
            // Slots 0 (reserved), 1 (launcher) and 2 (owner); raw reference
            // 3 is an alias of the owner.
            world.array(GAME + cgame_state::COUNTRIES, SLOTS, 3);
            world.array(GAME + cgame_state::COUNTRY_LINKS, LINKS, 4);
            for (slot, country, id, instance) in [
                (0, RESERVED, 0, 0xaaaa),
                (1, COUNTRY, 1, 0xbbbb),
                (2, OWNER, 2, 0xcccc),
            ] {
                world.u64(SLOTS + slot * 8, country as u64);
                world.u64(country, ccountry::VTABLE_RVA as u64);
                world.u32(country + ccountry::ID, id);
                world.u32(country + ccountry::ID_INSTANCE, instance);
            }
            for (raw, slot) in [0, 1, 2, 2].into_iter().enumerate() {
                world.u32(LINKS + raw * 4, slot);
            }
            world.u64(PROVINCE, cprovince::VTABLE_RVA as u64);
            world.u32(PROVINCE + cprovince::ID, 13_000);
            world.u64(PROVINCE + cprovince::STATE, STATE as u64);
            world.u32(PROVINCE + cprovince::CONTROLLER, 77);
            world.u64(STATE, cstate::VTABLE_RVA as u64);
            world.u32(STATE + cstate::ID, 1_204);
            world.u32(STATE + cstate::OWNER, 3);
            world
        }
        fn u32(&mut self, at: usize, value: u32) {
            self.0[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        fn u64(&mut self, at: usize, value: u64) {
            self.0[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.u64(at, data as u64);
            self.u32(at + 8, count);
            self.u32(at + 12, count);
        }
        fn memory(&self) -> SliceMemory<'_> {
            SliceMemory {
                base: 0,
                bytes: &self.0,
            }
        }
    }

    #[test]
    fn a_strike_names_the_state_owner_apart_from_the_controller() {
        let world = World::new();
        assert_eq!(
            strike(&world.memory(), 0, GAME, NUKE, PROVINCE),
            Ok(Strike {
                launcher: CountryIdentity {
                    reference: 1,
                    id: 1,
                    instance: 0xbbbb,
                },
                nuke_type: 1,
                province: 13_000,
                state: 1_204,
                // The aliased reference keeps its raw word beside the full ID.
                owner: CountryIdentity {
                    reference: 3,
                    id: 2,
                    instance: 0xcccc,
                },
                controller: 77,
            })
        );
    }

    #[test]
    fn a_stateless_province_falls_on_the_reserved_country() {
        let mut world = World::new();
        world.u64(PROVINCE + cprovince::STATE, 0);
        let read = strike(&world.memory(), 0, GAME, NUKE, PROVINCE).unwrap();
        assert_eq!((read.state, read.controller), (0, 77));
        assert_eq!(
            read.owner,
            CountryIdentity {
                reference: 0,
                id: 0,
                instance: 0xaaaa,
            }
        );
    }

    #[test]
    fn a_strike_refuses_an_unauthenticated_nuke_province_or_launcher() {
        let mut world = World::new();
        world.u64(NUKE, 0);
        assert!(strike(&world.memory(), 0, GAME, NUKE, PROVINCE).is_err());
        let mut world = World::new();
        world.u64(PROVINCE, 0);
        assert!(strike(&world.memory(), 0, GAME, NUKE, PROVINCE).is_err());
        // A launcher whose reference names another country is not trusted.
        let mut world = World::new();
        world.u32(COUNTRY + ccountry::ID, 2);
        assert!(strike(&world.memory(), 0, GAME, NUKE, PROVINCE).is_err());
    }

    #[test]
    fn units_follow_native_order_skip_task_forces_and_name_other_classes() {
        let mut world = World::new();
        world.array(PROVINCE + cprovince::OCCUPANTS, OCCUPANTS, 4);
        for (index, unit) in [ARMY, SHIPS, GUN, 0x3400].into_iter().enumerate() {
            world.u64(OCCUPANTS + index * 8, unit as u64);
        }
        world.u64(ARMY, carmy::VTABLE_RVA as u64);
        world.u32(ARMY + cunit::CLASS, ARMY_CLASS);
        world.u64(ARMY + cunit::HANDLE, (7 << 32) | 0x41);
        world.u32(ARMY + cunit::ROSTER_OWNER, 3);
        world.u32(ARMY + cunit::EXPEDITIONARY_OWNER, 9);
        world.u64(ARMY + carmy::STRENGTH, 4_200_000);
        world.u64(ARMY + carmy::ORGANISATION, (-1_i64) as u64);
        world.array(ARMY + carmy::MANPOWER_DATA, MANPOWER, 2);
        world.u32(MANPOWER + manpower_value_entry::COUNT, 9_000);
        world.u32(MANPOWER + 8 + manpower_value_entry::COUNT, (-1_i32) as u32);
        world.u32(SHIPS + cunit::CLASS, SKIPPED_CLASS);
        world.u64(GUN, crailway_gun::VTABLE_RVA as u64);
        world.u32(GUN + cunit::CLASS, RAILWAY_GUN_CLASS);
        world.u64(GUN + cunit::HANDLE, (8 << 32) | 0x42);
        world.u32(GUN + cunit::ROSTER_OWNER, 5);
        world.u64(GUN + crailway_gun::CURRENT_STRENGTH, 100_000);
        world.u32(GUN + crailway_gun::MANPOWER, 250);
        world.u32(0x3400 + cunit::CLASS, 2);
        let mut seen = Vec::new();
        units(&world.memory(), 0, PROVINCE, |unit| seen.push(unit)).unwrap();
        assert_eq!(
            seen,
            [
                Ok(Unit {
                    handle: (7 << 32) | 0x41,
                    class: ARMY_CLASS,
                    owner: 9,
                    strength: 4_200_000,
                    organisation: Some(-1),
                    manpower: 8_999,
                }),
                Ok(Unit {
                    handle: (8 << 32) | 0x42,
                    class: RAILWAY_GUN_CLASS,
                    owner: 5,
                    strength: 100_000,
                    organisation: None,
                    manpower: 250,
                }),
                Err("nuclear strike unit class unsupported"),
            ]
        );
    }

    #[test]
    fn an_unreadable_occupant_list_fails_the_walk() {
        let mut world = World::new();
        world.array(PROVINCE + cprovince::OCCUPANTS, 0x10_0000, 1);
        assert!(units(&world.memory(), 0, PROVINCE, |_| {}).is_err());
    }
}
