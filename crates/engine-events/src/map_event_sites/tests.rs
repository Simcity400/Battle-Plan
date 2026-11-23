use super::*;
use engine_native::ccountry::synthetic::{START, World};

const MODULE: usize = 0x4000_0000;

#[test]
fn scripted_refresh_records_the_selected_declaration_after_the_effect_gates() {
    struct Selection {
        pointer: u64,
        mode: i32,
    }
    impl Memory for Selection {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == MODULE + MAP_MODE_SINGLETON_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&self.pointer.to_le_bytes());
            } else if address == 0x1000 && output.len() == 4 {
                output.copy_from_slice(&self.mode.to_le_bytes());
            } else {
                return false;
            }
            true
        }
    }
    let mut memory = Selection {
        pointer: 0x1000,
        mode: FIRST_SCRIPTED_MAP_MODE + 9,
    };
    assert_eq!(scripted_map_mode(&memory, MODULE), Ok(9));
    memory.mode = FIRST_SCRIPTED_MAP_MODE - 1;
    assert!(scripted_map_mode(&memory, MODULE).is_err());
    memory.pointer = 0;
    assert!(scripted_map_mode(&memory, MODULE).is_err());
}

struct Map {
    world: World,
    provinces: Vec<usize>,
}

impl Map {
    fn new(countries: u32, provinces: u32, flags: impl Fn(u32) -> u8) -> Self {
        let mut world = World::new(MODULE, countries, COUNTRY_DIPLOMACY + 8);
        let table = world.alloc((provinces as usize + 1) * 8);
        world.array(START + cgame_state::PROVINCES, table, provinces + 1);
        let mut objects = vec![0];
        for id in 1..=provinces {
            let province = world.alloc(0x200);
            let definition = world.alloc(0x100);
            world.u64(table + id as usize * 8, province as u64);
            world.u32(province + cprovince::ID, id);
            world.u64(province + cprovince::DEFINITION, definition as u64);
            world.put(
                definition + engine_native::cprovince::definition::TERRAIN_FLAGS,
                &[flags(id)],
            );
            objects.push(province);
        }
        Self {
            world,
            provinces: objects,
        }
    }

    fn unit(&mut self, class: u32, province: u32, route: &[u32]) -> usize {
        let unit = self.world.alloc(0x300);
        self.world.u32(unit + cunit::CLASS, class);
        self.world.u64(
            unit + cunit::LOCATION,
            self.provinces[province as usize] as u64,
        );
        self.world
            .u32(unit + cunit::ORDINARY_ROUTE_COUNT, route.len() as u32);
        let path = self.world.alloc(route.len().max(1) * 4);
        for (index, id) in route.iter().enumerate() {
            self.world.u32(path + index * 4, *id);
        }
        self.world
            .u64(unit + cunit::ORDINARY_ROUTE_DATA, path as u64);
        unit
    }
}

#[test]
fn level_health_follows_the_healthy_level_and_the_partial_level() {
    assert_eq!(health_of(0, 3, 5, 1), 0);
    assert_eq!(health_of(4, 2, 7_000_000, 0), LEVEL_HEALTH);
    assert_eq!(health_of(4, 2, 7_000_000, 2), LEVEL_HEALTH);
    assert_eq!(health_of(4, 2, 7_000_000, 3), 7_000_000);
    assert_eq!(health_of(4, 2, 7_000_000, 4), 0);
    assert_eq!(health_of(4, -1, 9, 0), 9);
    assert_eq!(health_of(4, i16::MAX, 9, i16::MAX), LEVEL_HEALTH);
}

#[test]
fn a_unit_stands_on_its_province_unless_it_embarks_onto_an_enemy_sea() {
    let mut map = Map::new(4, 3, |id| if id == 3 { 1 } else { 0 });
    let still = map.unit(0, 1, &[]);
    let marching = map.unit(0, 1, &[2]);
    let gun = map.unit(13, 1, &[3]);
    let memory = map.world.memory();
    let state = map.world.game_state();
    assert_eq!(unit_province(&memory, MODULE, state, still), Ok(1));
    assert_eq!(unit_province(&memory, MODULE, state, marching), Ok(1));
    assert_eq!(unit_province(&memory, MODULE, state, gun), Ok(1));
}

#[test]
fn intel_entries_keep_native_order_signed_levels_and_repeats_across_batches() {
    let mut world = World::new(MODULE, 1, 0x20);
    let unit = world.alloc(0x300);
    let count = 5_000_u32;
    let entries = world.alloc(count as usize * INTEL_STRIDE);
    world.array(unit + cunit::VIEWER_DATA, entries, count);
    for index in 0..count as usize {
        let entry = entries + index * INTEL_STRIDE;
        world.u32(entry, (count - 1 - index as u32) % 997);
        world.put(entry + INTEL_LEVEL, &[(index % 256) as u8]);
    }
    let memory = world.memory();
    let array = intel(&memory, unit).unwrap();
    assert_eq!(array.count, count);
    let mut output = vec![(0, 0); 70];
    intel_entries(&memory, array, 4_000, &mut output).unwrap();
    for (offset, (country, level)) in output.iter().enumerate() {
        let index = 4_000 + offset;
        assert_eq!(*country, (count - 1 - index as u32) % 997);
        assert_eq!(*level, (index % 256) as u8 as i8);
    }
    let mut past = vec![(0, 0); 2];
    assert!(intel_entries(&memory, array, count as usize + 1_000_000, &mut past).is_err());
}

#[test]
fn a_resistance_text_takes_the_best_province_its_controller_and_the_owners_overlord() {
    let mut map = Map::new(6, 4, |_| 0);
    let state = map.world.alloc(0x900);
    let provinces = map.world.alloc(4 * 8);
    for index in 0..4 {
        map.world
            .u64(provinces + index * 8, map.provinces[index + 1] as u64);
    }
    map.world.u64(state + cstate::PROVINCES, provinces as u64);
    map.world.u32(state + cstate::OWNER, 3);
    map.world.u32(state + cstate::CONTROLLER, 5);
    let diplomacy = map.world.alloc(0x200);
    let owner = map.world.object(3);
    map.world.u64(owner + COUNTRY_DIPLOMACY, diplomacy as u64);
    map.world.u32(diplomacy + DIPLOMACY_OVERLORD, 4);
    let resistance_object = map.world.alloc(0x20);
    map.world
        .u64(resistance_object + RESISTANCE_STATE, state as u64);
    map.world.u32(state + STATE_BEST_PROVINCE, 2);
    let game_state = map.world.game_state();
    {
        let memory = map.world.memory();
        assert_eq!(
            resistance(&memory, MODULE, game_state, resistance_object),
            Ok(Resistance {
                province: 3,
                controller: 5,
                overlord: 4,
            })
        );
    }
    map.world.u32(state + STATE_BEST_PROVINCE, (-1_i32) as u32);
    let memory = map.world.memory();
    assert_eq!(
        resistance(&memory, MODULE, game_state, resistance_object).map(|r| r.province),
        Ok(1)
    );
}

#[test]
fn a_building_text_stands_on_its_province_or_its_states_first_province() {
    let mut map = Map::new(3, 5, |_| 0);
    let game_state = map.world.game_state();
    for (province, controller) in [(2, 7), (4, 9)] {
        map.world
            .u32(map.provinces[province] + cprovince::CONTROLLER, controller);
    }
    let states = map.world.alloc(3 * 8);
    map.world.array(game_state + cgame_state::STATES, states, 3);
    let state = map.world.alloc(0x100);
    let members = map.world.alloc(16);
    map.world.u64(members, map.provinces[4] as u64);
    map.world.u64(members + 8, map.provinces[5] as u64);
    map.world.u64(state + cstate::PROVINCES, members as u64);
    map.world.u64(states + 2 * 8, state as u64);
    let mut make = |flags: u32, id: u32, building_type: u32| {
        let status = map.world.alloc(0x80);
        map.world.u32(status + BUILDING_STATUS_FLAGS, flags);
        map.world.u32(status + BUILDING_STATUS_ID, id);
        let building = map.world.alloc(0x200);
        map.world.u32(building + BUILDING_TYPE, building_type);
        map.world.u64(building + BUILDING_STATUS, status as u64);
        building
    };
    let port = make(1, 2, 11);
    let factory = make(BUILDING_STATUS_STATE_FLAG, 2, 12);
    let memory = map.world.memory();
    assert_eq!(
        building(&memory, game_state, port),
        Ok(Building {
            building_type: 11,
            province: 2,
            controller: 7
        })
    );
    assert_eq!(
        building(&memory, game_state, factory),
        Ok(Building {
            building_type: 12,
            province: 4,
            controller: 9
        })
    );
}

#[test]
fn an_inline_and_a_heap_activity_key_are_both_located_exactly() {
    let mut world = World::new(MODULE, 1, 0x20);
    let activity = world.alloc(0x400);
    let at_ = activity + ACTIVITY_MAP_TEXT;
    world.put(at_, b"short\0");
    world.u64(at_ + 16, 5);
    world.u64(at_ + 24, 15);
    let memory = world.memory();
    assert_eq!(activity_key(&memory, activity), Ok((at_, 5)));
    let heap = world.alloc(64);
    world.put(heap, b"RESISTANCE_ACTIVITY_LONG_KEY\0");
    world.u64(at_, heap as u64);
    world.u64(at_ + 16, 28);
    world.u64(at_ + 24, 31);
    let memory = world.memory();
    assert_eq!(activity_key(&memory, activity), Ok((heap, 28)));
    world.u64(at_ + 16, 40);
    let memory = world.memory();
    assert!(activity_key(&memory, activity).is_err());
}
