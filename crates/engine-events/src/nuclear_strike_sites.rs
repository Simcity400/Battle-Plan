
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{carmy, ccountry, cprovince, cstate, cunit};
use crate::{
    cnuke,
    equipment_destruction,
    event_feed_sites::{CountryIdentity, identity},
    manpower_value_entry,
};
use engine_native_parts::crailway_gun;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
}

pub const IMPACT: Site = Site {
    name: "impact",
    rva: Rva(0x0111_fdf0),
};
pub const NOTIFY: Site = Site {
    name: "notify",
    rva: Rva(0x0112_19b0),
};
pub const UNIT_DAMAGE: Site = Site {
    name: "unit_damage",
    rva: Rva(0x0111_f4d0),
};
pub const SITES: [Site; 3] = [IMPACT, NOTIFY, UNIT_DAMAGE];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Strike {
    pub launcher: CountryIdentity,
    pub nuke_type: u32,
    pub province: u32,
    pub state: u32,
    pub owner: CountryIdentity,
    pub controller: u32,
}

fn at(object: usize, offset: usize) -> Result<usize, &'static str> {
    object
        .checked_add(offset)
        .ok_or("nuclear strike address overflow")
}

fn field<T: engine_native_memory::memory::Raw>(
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
    vtable: Rva,
    reason: &'static str,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, vtable).ok_or(reason)? as u64;
    if object == 0 || read::<u64>(memory, object) != Some(expected) {
        return Err(reason);
    }
    Ok(())
}

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

pub const ARMY_CLASS: u32 = 0;
pub const SKIPPED_CLASS: u32 = 1;
pub const RAILWAY_GUN_CLASS: u32 = 13;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unit {
    pub handle: u64,
    pub class: u32,
    pub owner: u32,
    pub strength: i64,
    pub organisation: Option<i64>,
    pub manpower: i64,
}

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
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

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
            world.u64(NUKE, cnuke::VTABLE_RVA.0 as u64);
            world.u64(NUKE + cnuke::COUNTRY, COUNTRY as u64);
            world.u32(NUKE + cnuke::TYPE, 1);
            world.array(GAME + cgame_state::COUNTRIES, SLOTS, 3);
            world.array(GAME + cgame_state::COUNTRY_LINKS, LINKS, 4);
            for (slot, country, id, instance) in [
                (0, RESERVED, 0, 0xaaaa),
                (1, COUNTRY, 1, 0xbbbb),
                (2, OWNER, 2, 0xcccc),
            ] {
                world.u64(SLOTS + slot * 8, country as u64);
                world.u64(country, ccountry::VTABLE_RVA.0 as u64);
                world.u32(country + ccountry::ID, id);
                world.u32(country + ccountry::SURRENDER_TARGET, instance);
            }
            for (raw, slot) in [0, 1, 2, 2].into_iter().enumerate() {
                world.u32(LINKS + raw * 4, slot);
            }
            world.u64(PROVINCE, cprovince::VTABLE_RVA.0 as u64);
            world.u32(PROVINCE + cprovince::ID, 13_000);
            world.u64(PROVINCE + cprovince::STATE, STATE as u64);
            world.u32(PROVINCE + cprovince::CONTROLLER, 77);
            world.u64(STATE, cstate::VTABLE_RVA.0 as u64);
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
        world.u64(ARMY, carmy::VTABLE_RVA.0 as u64);
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
        world.u64(GUN, crailway_gun::VTABLE_RVA.0 as u64);
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
