use super::*;
use engine_native::{
    carmy, cgame_state, cprovince, cradars_pool, cstate, cunit, modifier_definition,
};
use engine_visibility::visibility_inputs;
use std::collections::BTreeMap;

const GS: usize = 0x1000;
const COUNTRIES: usize = 0x100_0000;
const LINKS: usize = 0x200_0000;
const STATES: usize = 0x300_0000;
const NAVIES: usize = 0x400_0000;
const MANAGER: usize = 0x500_0000;
const OBJECTS: usize = 0x1000_0000;
const METADATA: usize = 0x9000_0000;

struct World(BTreeMap<usize, Vec<u8>>);
impl World {
    fn put(&mut self, at: usize, bytes: &[u8]) {
        let (&start, storage) = self.0.range_mut(..=at).next_back().unwrap();
        storage[at - start..at - start + bytes.len()].copy_from_slice(bytes);
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: usize) {
        self.put(at, &(value as u64).to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: usize) {
        self.u64(at, data);
        self.u32(at + 8, count as u32);
        self.u32(at + 12, count as u32);
    }
    fn new(countries: usize, population: usize) -> Self {
        let mut world = Self(BTreeMap::from([
            (GS, vec![0; 0x1000]),
            (COUNTRIES, vec![0; (countries + 1) * 8]),
            (LINKS, vec![0; (countries + 1) * 4]),
            (STATES, vec![0; (countries + 1) * 8]),
            (NAVIES, vec![0; (countries + 1) * 8]),
            (MANAGER, vec![0; 0x100]),
            (modifier_definition::TABLE_RVA.0, vec![0; 8]),
            (
                METADATA,
                vec![
                    0;
                    (visibility_inputs::UNIT_MODIFIER as usize + 1) * modifier_definition::STRIDE
                ],
            ),
        ]));
        world.array(GS + cgame_state::COUNTRIES, COUNTRIES, countries + 1);
        world.array(GS + cgame_state::COUNTRY_LINKS, LINKS, countries + 1);
        world.array(GS + cgame_state::STATES, STATES, countries + 1);
        world.u64(GS + cgame_state::STRATEGIC_NAVY_MANAGER, MANAGER);
        world.u64(
            MANAGER,
            engine_native_naval::cstrategic_navy_manager::VTABLE_RVA.0,
        );
        world.array(
            MANAGER + engine_native_naval::cstrategic_navy_manager::NAVIES,
            NAVIES,
            countries + 1,
        );
        world.u64(modifier_definition::TABLE_RVA.0, METADATA);
        for slot in 1..=countries {
            let country = OBJECTS + slot * 0x1_0000;
            world.0.insert(country, vec![0; 0x5000]);
            world.u64(COUNTRIES + slot * 8, country);
            world.u32(LINKS + slot * 4, slot as u32);
            world.u64(country, ccountry::VTABLE_RVA.0);
            world.u32(country + ccountry::ID, slot as u32);
            let navy = country + 0x1800;
            world.u64(NAVIES + slot * 8, navy);
            world.u64(navy, engine_native_naval::cstrategic_navy::VTABLE_RVA.0);
            world.u64(
                navy + 8,
                engine_native_naval::cstrategic_navy::SECONDARY_VTABLE_RVA.0,
            );
            world.u32(navy + engine_native_naval::cstrategic_navy::OWNER, slot as u32);
            world.array(
                navy + engine_native_naval::cstrategic_navy::ACCESS_MODES,
                country + 0x4100,
                population,
            );
            world.put(country + 0x4100, &vec![2; population]);
            let group = country + 0x1a00;
            world.u64(country + ccountry::RADARS_POOL, cradars_pool::VTABLE_RVA.0);
            world.u32(country + ccountry::RADARS_POOL_COUNTRY, slot as u32);
            world.array(
                country + ccountry::RADAR_GROUPS,
                country + 0x4000,
                population,
            );
            world.u64(group + radar_group::PARENT, country + ccountry::RADARS_POOL);
            world.array(group + radar_group::PROVINCES, country + 0x4200, population);
            world.array(
                country + ccountry::VOLUNTEER_HOSTS,
                country + 0x4200,
                population,
            );
            world.array(
                country + ccountry::VOLUNTEER_TALLIES,
                country + 0x4300,
                population,
            );
            world.array(
                country + ccountry::SENT_VOLUNTEERS,
                country + 0x4400,
                population,
            );
            let theatre = country + 0x1b00;
            let area = country + 0x1d00;
            let province = country + 0x1e00;
            let unit = country + 0x2400;
            world.array(country + ccountry::THEATRES, country + 0x4500, 1);
            world.u64(country + 0x4500, theatre);
            world.u64(theatre, ctheatre::VTABLE_RVA.0);
            world.u64(
                theatre + ctheatre::HANDLE,
                (slot << 32) | ctheatre::REFERENCE_TYPE as usize,
            );
            world.u32(theatre + ctheatre::VOLUNTEERS_THEATRE, 1);
            world.array(
                theatre + ctheatre::CONTROLLER_AREA_DATA,
                country + 0x4600,
                population,
            );
            world.array(
                theatre + ctheatre::STATIONED_UNIT_DATA,
                country + 0x4700,
                population,
            );
            world.u64(area, ccontroller_area::VTABLE_RVA.0);
            world.array(
                area + ccontroller_area::PROVINCE_DATA,
                country + 0x4800,
                population,
            );
            world.u64(province, cprovince::VTABLE_RVA.0);
            world.u32(province + cprovince::ID, slot as u32);
            world.u64(unit, carmy::VTABLE_RVA.0);
            world.u64(unit + cunit::HANDLE, (slot << 32) | 1);
            world.array(country + ccountry::DIVISIONS, country + 0x4900, 1);
            world.u64(country + 0x4900, unit);
            world.array(unit + cunit::VIEWER_DATA, country + 0x4a00, population);
            let state = country + 0x3000;
            world.u64(STATES + slot * 8, state);
            world.u64(state, cstate::VTABLE_RVA.0);
            world.u32(state + cstate::ID, slot as u32);
            world.u32(state + cstate::OWNER, slot as u32);
            world.u32(
                state + cstate::CONTROLLER,
                slot as u32 % countries as u32 + 1,
            );
            world.u64(state + cstate::COUNTRY_MODIFIERS, country + 0x3c00);
            for ordinal in 0..population {
                world.u64(country + 0x4000 + ordinal * 8, group);
                world.u32(country + 0x4200 + ordinal * 4, slot as u32);
                world.u32(country + 0x4300 + ordinal * 4, 1);
                world.u64(country + 0x4400 + ordinal * 8, (slot << 32) | (ordinal + 1));
                world.u64(country + 0x4600 + ordinal * 8, area);
                world.u64(country + 0x4700 + ordinal * 8, unit);
                world.u64(country + 0x4800 + ordinal * 8, province);
                world.u32(
                    country + 0x4a00 + ordinal * unit_viewer_entry::STRIDE,
                    slot as u32,
                );
                world.put(
                    country
                        + 0x4a00
                        + ordinal * unit_viewer_entry::STRIDE
                        + unit_viewer_entry::LEVEL,
                    &[2],
                );
            }
        }
        world
    }
}
impl Memory for World {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        let Some((&start, bytes)) = self.0.range(..=at).next_back() else {
            return false;
        };
        let Some(source) = bytes.get(at - start..at - start + output.len()) else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
    fn modifier_accumulator_value(
        &self,
        _at: usize,
        _key: u32,
    ) -> Option<Result<engine_native::modifier_accumulator::Value, &'static str>> {
        Some(Ok(engine_native::modifier_accumulator::Value {
            raw: 2,
            present: true,
        }))
    }
}

#[test]
fn radar_growth_keeps_original_reads_and_fault_precedence() {
    use std::cell::RefCell;
    struct Trace<'a> {
        world: &'a World,
        calls: RefCell<Vec<(usize, usize)>>,
        blocked: Option<usize>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, at: usize, output: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((at, output.len()));
            if self
                .blocked
                .is_some_and(|blocked| (at..at + output.len()).contains(&blocked))
            {
                return false;
            }
            self.world.copy(at, output)
        }
    }
    let world = World::new(2, 5);
    for blocked in [None, Some(OBJECTS + 0x1_0000 + 0x4200)] {
        let trace = Trace {
            world: &world,
            calls: RefCell::new(Vec::new()),
            blocked,
        };
        let mut rows = vec![radar_group::Row::default(); 10];
        let mut provinces = vec![0; 50];
        let expected = radar_group::walk(&trace, 0, GS, &mut rows, &mut provinces);
        let expected_calls = std::mem::take(&mut *trace.calls.borrow_mut());
        let mut grown_rows = vec![radar_group::Row::default(); 1];
        let mut grown_provinces = vec![0; 1];
        let actual =
            radar_group::walk(&trace, 0, GS, &mut grown_rows, &mut grown_provinces);
        assert_eq!(actual, expected);
        assert_eq!(
            &grown_rows[..actual.outcome.len()],
            &rows[..expected.outcome.len()]
        );
        assert_eq!(*trace.calls.borrow(), expected_calls);
    }
}
