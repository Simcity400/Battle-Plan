use engine_native_parts::cdiplomacy_status_layout as cdiplomacy_status;
use crate::{
    Memory,
    ccountry,
     cpdx_array::Array,
    cprovince,
    crelation::status as crelation_status,
    modifier_accumulator::{self, Value},
    modifier_keys,
    read as native_read,
};
use engine_sites::Rva;

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|at| native_read(memory, at))
        .ok_or("targeted spotting source unreadable or null")
}
fn auth(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    vtable: Rva,
) -> Result<(), &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, vtable).ok_or("targeted spotting vtable withheld")? as u64
    {
        return Err("targeted spotting source identity mismatch");
    }
    Ok(())
}
fn slot<M: Memory>(countries: &ccountry::Reader<'_, M>, raw: u32) -> Result<u32, &'static str> {
    if raw == 0 {
        Ok(0)
    } else {
        countries.link(raw).map(|slot| slot.unwrap_or(0))
    }
}

pub fn read<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    roster: ccountry::Country,
    location: Option<usize>,
) -> Result<Value, &'static str> {
    let location = location.ok_or("targeted spotting original location unavailable")?;
    auth(memory, base, location, cprovince::VTABLE_RVA)?;
    raw(
        memory,
        base,
        countries,
        roster,
        field(memory, location, cprovince::CONTROLLER)?,
    )
}

pub fn raw<M: Memory>(
    memory: &M,
    base: usize,
    countries: &ccountry::Reader<'_, M>,
    roster: ccountry::Country,
    controller: u32,
) -> Result<Value, &'static str> {
    let selected = slot(countries, controller)?;
    let diplomacy = field::<u64>(memory, roster.pointer, ccountry::DIPLOMACY_STATUS)? as usize;
    auth(memory, base, diplomacy, cdiplomacy_status::VTABLE_RVA)?;
    if field::<u64>(memory, diplomacy, cdiplomacy_status::COUNTRY)? != roster.pointer as u64 {
        return Err("targeted spotting diplomacy owner mismatch");
    }
    let relations = diplomacy
        .checked_add(cdiplomacy_status::RELATIONS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("targeted spotting relation matrix invalid")?;
    let status = relations
        .element(selected, 8)
        .and_then(|at| native_read::<u64>(memory, at))
        .filter(|pointer| *pointer != 0)
        .ok_or("targeted spotting relation slot unreadable or absent")? as usize;
    auth(memory, base, status, crelation_status::VTABLE_RVA)?;
    if field::<u64>(memory, status, crelation_status::DIPLOMACY_STATUS)? != diplomacy as u64 {
        return Err("targeted spotting relation owner mismatch");
    }
    if slot(
        countries,
        field(memory, status, crelation_status::TARGET_COUNTRY)?,
    )? != selected
    {
        return Err("targeted spotting relation target mismatch");
    }
    let accumulator = status
        .checked_add(crelation_status::MODIFIER_ACCUMULATOR)
        .ok_or("targeted spotting accumulator address overflow")?;
    let mut value = [Value::default()];
    modifier_accumulator::lookup_many_at(
        memory,
        accumulator,
        &[modifier_keys::SPOTTING_CHANCE_AGAINST],
        &mut value,
    )?;
    Ok(value[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    struct Fixture {
        world: ccountry::synthetic::World,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            self.world.memory().copy(address, output)
        }
    }
    fn fixture(controller: u32) -> (Fixture, usize, usize) {
        let mut world = ccountry::synthetic::World::new(0, 2, 0x4000);
        let country = world.object(1);
        let diplomacy = world.alloc(0x100);
        let status = world.alloc(0x300);
        let relations = world.alloc(24);
        let values = world.alloc(16);
        let province = world.alloc(0x300);
        let links = native_read::<u64>(
            &world.memory(),
            world.game_state() + crate::cgame_state::COUNTRY_LINKS,
        )
        .unwrap() as usize;
        world.u32(links + 8, if controller == 0 { 0 } else { 1 });
        world.u64(country + ccountry::DIPLOMACY_STATUS, diplomacy as u64);
        world.u64(diplomacy, cdiplomacy_status::VTABLE_RVA.0 as u64);
        world.u64(diplomacy + cdiplomacy_status::COUNTRY, country as u64);
        world.array(diplomacy + cdiplomacy_status::RELATIONS, relations, 3);
        let selected = if controller == 0 { 0 } else { 1 };
        world.u64(relations + selected * 8, status as u64);
        world.u64(status, crelation_status::VTABLE_RVA.0 as u64);
        world.u64(
            status + crelation_status::DIPLOMACY_STATUS,
            diplomacy as u64,
        );
        world.u32(status + crelation_status::TARGET_COUNTRY, selected as u32);
        world.array(status + crelation_status::MODIFIER_ACCUMULATOR, values, 1);
        world.u32(values, modifier_keys::SPOTTING_CHANCE_AGAINST);
        world.u64(values + 8, -17001_i64 as u64);
        world.u64(province, cprovince::VTABLE_RVA.0 as u64);
        world.u32(province + cprovince::CONTROLLER, controller);
        (
            Fixture {
                world,
                reads: RefCell::new(Vec::new()),
            },
            province,
            status,
        )
    }
    #[test]
    fn raw_directed_lookup_uses_controller_alias_and_zero_slot() {
        for controller in [0, 2] {
            let (memory, location, _) = fixture(controller);
            let countries = ccountry::Reader::new(&memory, 0, memory.world.game_state()).unwrap();
            let roster = countries.resolve(1).unwrap();
            assert_eq!(
                read(&memory, 0, &countries, roster, Some(location)),
                Ok(Value {
                    present: true,
                    raw: -17001
                })
            );
        }
    }
    #[test]
    fn absent_key_is_known_but_owner_mismatch_is_not() {
        let (mut memory, location, status) = fixture(2);
        memory
            .world
            .array(status + crelation_status::MODIFIER_ACCUMULATOR, 0, 0);
        {
            let countries = ccountry::Reader::new(&memory, 0, memory.world.game_state()).unwrap();
            assert_eq!(
                read(
                    &memory,
                    0,
                    &countries,
                    countries.resolve(1).unwrap(),
                    Some(location)
                ),
                Ok(Value::default())
            );
        }
        memory
            .world
            .u64(status + crelation_status::DIPLOMACY_STATUS, 0);
        let countries = ccountry::Reader::new(&memory, 0, memory.world.game_state()).unwrap();
        assert!(
            read(
                &memory,
                0,
                &countries,
                countries.resolve(1).unwrap(),
                Some(location)
            )
            .is_err()
        );
    }
    #[test]
    fn failed_original_location_is_never_retried() {
        let (memory, _, _) = fixture(2);
        let countries = ccountry::Reader::new(&memory, 0, memory.world.game_state()).unwrap();
        let roster = countries.resolve(1).unwrap();
        memory.reads.borrow_mut().clear();
        assert!(read(&memory, 0, &countries, roster, None).is_err());
        assert!(memory.reads.borrow().is_empty());
    }
}
