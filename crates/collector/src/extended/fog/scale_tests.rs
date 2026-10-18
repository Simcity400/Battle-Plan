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
            (modifier_definition::TABLE_RVA, vec![0; 8]),
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
        world.u64(MANAGER, engine_native::cstrategic_navy_manager::VTABLE_RVA);
        world.array(
            MANAGER + engine_native::cstrategic_navy_manager::NAVIES,
            NAVIES,
            countries + 1,
        );
        world.u64(modifier_definition::TABLE_RVA, METADATA);
        for slot in 1..=countries {
            let country = OBJECTS + slot * 0x1_0000;
            world.0.insert(country, vec![0; 0x5000]);
            world.u64(COUNTRIES + slot * 8, country);
            world.u32(LINKS + slot * 4, slot as u32);
            world.u64(country, ccountry::VTABLE_RVA);
            world.u32(country + ccountry::ID, slot as u32);
            let navy = country + 0x1800;
            world.u64(NAVIES + slot * 8, navy);
            world.u64(navy, engine_native::cstrategic_navy::VTABLE_RVA);
            world.u64(
                navy + 8,
                engine_native::cstrategic_navy::SECONDARY_VTABLE_RVA,
            );
            world.u32(navy + engine_native::cstrategic_navy::OWNER, slot as u32);
            world.array(
                navy + engine_native::cstrategic_navy::ACCESS_MODES,
                country + 0x4100,
                population,
            );
            world.put(country + 0x4100, &vec![2; population]);
            let group = country + 0x1a00;
            world.u64(country + ccountry::RADARS_POOL, cradars_pool::VTABLE_RVA);
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
            world.u64(theatre, ctheatre::VTABLE_RVA);
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
            world.u64(area, ccontroller_area::VTABLE_RVA);
            world.array(
                area + ccontroller_area::PROVINCE_DATA,
                country + 0x4800,
                population,
            );
            world.u64(province, cprovince::VTABLE_RVA);
            world.u32(province + cprovince::ID, slot as u32);
            world.u64(unit, carmy::VTABLE_RVA);
            world.u64(unit + cunit::HANDLE, (slot << 32) | 1);
            world.array(country + ccountry::DIVISIONS, country + 0x4900, 1);
            world.u64(country + 0x4900, unit);
            world.array(unit + cunit::VIEWER_DATA, country + 0x4a00, population);
            let state = country + 0x3000;
            world.u64(STATES + slot * 8, state);
            world.u64(state, cstate::VTABLE_RVA);
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
fn first_and_later_fog_visibility_samples_grow_from_five_thousand_country_headers() {
    let capacity = Capacities {
        rows: [1; N],
        radar_provinces: 1,
        area_provinces: 1,
        wings: 1,
    };
    let mut fog = Captured::with_capacities(&capacity);
    let mut visibility_countries = vec![visibility_inputs::CountryRow::default(); 1];
    let mut visibility_states = vec![visibility_inputs::StateRow::default(); 1];
    let mut access = vec![engine_native::cstrategic_navy::AccessRow::default(); 1];
    for (hour, population) in [1, 5].into_iter().enumerate() {
        let world = World::new(5_000, population);
        let mut hint = capacity;
        assert!(hint.presize(&world, 0, GS).is_err()); // independent absent air-registration root
        for index in [0, 1, 2, 3, 4, 6] {
            assert!(hint.rows[index] >= 5_000 * population);
        }
        assert!(hint.radar_provinces >= 5_000 * population * population);
        assert!(hint.area_provinces >= 5_000 * population * population);
        let mut visibility_hint = crate::extended::visibility::Capacities::default();
        assert!(visibility_hint.presize(&world, 0, GS).is_err()); // independent absent diplomacy root
        let mut hinted_visibility =
            crate::extended::visibility::Captured::with_capacities(&visibility_hint);
        hinted_visibility.capture(&world, 0, GS, hour as i32, true);
        assert_eq!(
            hinted_visibility.outcomes()[1],
            Outcome::Rows { len: 5_000 }
        );
        assert_eq!(
            hinted_visibility.outcomes()[2],
            Outcome::Rows { len: 5_000 }
        );
        assert_eq!(
            hinted_visibility.outcomes()[5],
            Outcome::Rows {
                len: 5_000 * population
            }
        );
        fog.capture_growing(&world, 0, GS, hour as i32, true);
        for index in [0, 1, 2, 3, 4, 6] {
            assert_eq!(
                fog.outcomes()[index],
                Outcome::Rows {
                    len: 5_000 * population
                },
                "table {index}"
            );
        }
        assert_eq!(fog.radar_provinces.len, 5_000 * population * population);
        assert_eq!(fog.area_provinces.len, 5_000 * population * population);
        assert_eq!(
            visibility_inputs::countries_growing(&world, 0, GS, &mut visibility_countries).0,
            Outcome::Rows { len: 5_000 }
        );
        assert_eq!(
            visibility_inputs::states_growing(&world, 0, GS, &mut visibility_states),
            Outcome::Rows { len: 5_000 }
        );
        assert_eq!(
            engine_native::cstrategic_navy::access_modes_growing(&world, 0, GS, &mut access),
            Outcome::Rows {
                len: 5_000 * population
            }
        );
        let mut radar_job = RadarJob::default();
        let report = copy_radar_refresh_growing(
            &world,
            RadarCopyContext {
                base: 0,
                game_state: GS,
                hour: hour as i32,
                force_full: true,
                row_limit: 1,
                province_limit: 1,
            },
            &mut radar_job,
        );
        assert!(report.raw_ok);
        let mut fresh = Captured::with_capacities(&capacity);
        fresh.decode_radar_payload(&radar_job, &mut [0]);
        assert_eq!(fresh.radar.outcome, fog.radar.outcome);
        assert_eq!(fresh.radar_provinces.len, fog.radar_provinces.len);
        let mut intel_job = IntelJob::default();
        assert!(
            copy_intel_refresh_with_divisions_growing(
                &world,
                IntelCopyContext {
                    base: 0,
                    game_state: GS,
                    row_limit: 1
                },
                &mut intel_job,
                None
            )
            .raw_ok
        );
        fresh.decode_intel_payload(&intel_job, &mut [0]);
        assert_eq!(fresh.intel.outcome, fog.intel.outcome);
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
            radar_group::walk_growing(&trace, 0, GS, &mut grown_rows, &mut grown_provinces);
        assert_eq!(actual, expected);
        assert_eq!(
            &grown_rows[..actual.outcome.len()],
            &rows[..expected.outcome.len()]
        );
        assert_eq!(*trace.calls.borrow(), expected_calls);
    }
}
