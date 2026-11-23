use super::*;
use engine_native::{ccountry, cgame_state};
use engine_native_parts::cstrategic_region;
use engine_native_memory::memory::SliceMemory;

const TASK: usize = 0x1000;
const STATS: usize = 0x2000;
const PROVINCE: usize = 0x2400;
const REGION: usize = 0x2500;
const FLEET_AT: usize = 0x2800;
const RANGE_FLEET: usize = 0x2a00;
const ATTACHED: usize = 0x2c00;
const ENTRY: usize = 0x2d00;
const COMMANDER: usize = 0x2e00;
const ACCUMULATOR: usize = 0x3000;

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
    put(bytes, at, &value.to_le_bytes());
}
fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    put(bytes, at, &value.to_le_bytes());
}
fn put_ptr(bytes: &mut [u8], at: usize, pointer: usize) {
    put_u64(bytes, at, pointer as u64);
}

fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put_ptr(bytes, at, data);
    put_u32(bytes, at + 8, count);
    put_u32(bytes, at + 12, count);
}

fn accumulator(bytes: &mut [u8], header: usize, data: usize, entries: &[(u32, i64)]) {
    array(bytes, header, data, entries.len() as u32);
    for (index, (key, raw)) in entries.iter().enumerate() {
        put_u32(bytes, data + index * 0x10, *key);
        put_u64(bytes, data + index * 0x10 + 8, *raw as u64);
    }
}

fn region(bytes: &mut [u8], at: usize, id: u32, slot: u32) {
    put_ptr(bytes, at, cstrategic_region::VTABLE_RVA.0);
    put_u32(bytes, at + cstrategic_region::ID, id);
    put_u32(bytes, at + cstrategic_region::RUNTIME_SLOT, slot);
}

fn task_force() -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    let mission = TASK + MISSION;
    for (at, pointer) in [
        (mission, cnaval_mission::VTABLE_RVA.0),
        (mission + 8, TASK),
        (TASK + cunit::STAT_BLOCK, STATS),
        (TASK + cunit::LOCATION, PROVINCE),
        (PROVINCE, cprovince::VTABLE_RVA.0),
        (PROVINCE + cprovince::STRATEGIC_REGION, REGION),
        (TASK + ctask_force::FLEET, FLEET_AT),
        (FLEET_AT, cfleet::VTABLE_RVA.0),
        (RANGE_FLEET, cfleet::VTABLE_RVA.0),
        (ATTACHED, ENTRY),
        (ATTACHED + 8, ENTRY + 0x80),
        (ENTRY + HEADQUARTERS_COMMANDER, COMMANDER),
        (mission + MISSION_RANGE_FLEET, RANGE_FLEET),
    ] {
        put_ptr(&mut bytes, at, pointer);
    }
    for (offset, value) in [
        (MISSION_OWNER, 9),
        (MISSION_RANGE, 1),
        (MISSION_RANGE + 4, 4),
        (MISSION_CONVOYS, 12),
    ] {
        put_u32(&mut bytes, mission + offset, value);
    }
    put_u64(&mut bytes, STATS + STAT_RAIDING, 20_000);
    region(&mut bytes, REGION, 44, 44);
    accumulator(
        &mut bytes,
        TASK + LOCAL_MODIFIERS,
        ACCUMULATOR,
        &[(0x19, 5_000), (0xcc, -1_000), (0x200, 9)],
    );
    array(
        &mut bytes,
        TASK + ctask_force::ATTACHED_HEADQUARTERS,
        ATTACHED,
        2,
    );
    put_u64(
        &mut bytes,
        COMMANDER + COMMANDER_CHARACTER,
        (5_u64 << 32) | 73,
    );
    put_u64(&mut bytes, RANGE_FLEET + cfleet::HANDLE, (2_u64 << 32) | 61);
    bytes
}

fn visit(kind: u32) -> cfleet::TaskForceVisit {
    cfleet::TaskForceVisit {
        country: engine_native_memory::country_identity::Country {
            pointer: 0,
            canonical_slot: 1,
            id: 1,
        },
        task: TASK,
        id: 77,
        kind,
        fleet: FLEET_AT,
        fleet_handle: (1 << 32) | 61,
        leader: (9 << 32) | 4713,
    }
}

fn read_row(bytes: &[u8], headquarters: &mut Vec<u64>) -> Result<Operands, &'static str> {
    let memory = SliceMemory { base: 0, bytes };
    read_task_force(&memory, 0, visit(ESCORT), headquarters)
}

#[test]
fn task_force_row_keeps_every_efficiency_operand() {
    let bytes = task_force();
    let mut headquarters = vec![u64::MAX; 2];
    let row = read_row(&bytes, &mut headquarters).unwrap();
    let value = |raw| Value { raw, present: true };
    assert_eq!(
        row,
        Operands {
            stat_block: STATS,
            location: PROVINCE,
            id: 77,
            kind: ESCORT,
            mission_owner: 9,
            region: 44,
            raiding_factor: 20_000,
            local: [
                value(5_000),
                Value::default(),
                Value::default(),
                Value::default(),
                Value::default(),
                Value::default(),
                value(-1_000),
                Value::default(),
                Value::default()
            ],
            leader: (9 << 32) | 4713,
            headquarters: 2,
            range_fleet: (2 << 32) | 61,
            range: [1, 4],
            convoys: 12,
            ..Default::default()
        }
    );
    assert_eq!(headquarters, [(5 << 32) | 73, 0]);
    let mut docked = bytes.clone();
    put_u64(&mut docked, TASK + cunit::LOCATION, 0_u64);
    put_u32(&mut docked, TASK + LOCAL_MODIFIERS + 8, u32::MAX);
    let row = read_row(&docked, &mut vec![0; 2]).unwrap();
    assert_eq!((row.region, row.local), (0, [Value::default(); 9]));
    let mut stray = bytes;
    put_u64(&mut stray, PROVINCE, 1_u64);
    assert!(read_row(&stray, &mut vec![0; 2]).is_err());
}

#[test]
fn mission_operands_authenticate_without_reading_accessible_regions() {
    let mut bytes = task_force();
    put_u64(
        &mut bytes,
        TASK + MISSION + cnaval_mission::ACCESSIBLE_REGIONS,
        u64::MAX,
    );
    assert_eq!(read_row(&bytes, &mut vec![0; 2]).unwrap().mission_owner, 9);
    put_u64(&mut bytes, TASK + MISSION + 8, 1);
    assert_eq!(
        read_row(&bytes, &mut vec![0; 2]),
        Err("naval mission identity mismatch")
    );
}

#[test]
fn walk_visits_every_authenticated_task_force() {
    use engine_native::ccountry::synthetic::World;
    const MODULE: usize = 0x4000_0000;
    let mut world = World::new(MODULE, 2, 0x300);
    let country = world.object(1);
    let fleets = world.alloc(8);
    let fleet = world.alloc(0x100);
    let tasks = world.alloc(3 * 8);
    let task: Vec<usize> = (0..3).map(|_| world.alloc(0x400)).collect();
    world.array(country + ccountry::FLEETS, fleets, 1);
    world.array(world.object(2) + ccountry::FLEETS, 0, 0);
    world.u64(fleets, fleet as u64);
    world.u64(fleet, (MODULE + cfleet::VTABLE_RVA.0) as u64);
    world.u64(fleet + cfleet::HANDLE, (3 << 32) | 61);
    world.u64(fleet + cfleet::OWNER, country as u64);
    world.array(fleet + cfleet::TASK_FORCES, tasks, 3);
    for (index, kind) in [4_u32, 0, 3].into_iter().enumerate() {
        let task = task[index];
        world.u64(tasks + index * 8, task as u64);
        world.u64(task, (MODULE + ctask_force::VTABLE_RVA.0) as u64);
        world.u64(
            task + ctask_force::SECONDARY_BASE,
            (MODULE + ctask_force::SECONDARY_VTABLE_RVA.0) as u64,
        );
        world.u64(task + cunit::HANDLE, ((index as u64 + 10) << 32) | 61);
        world.u64(task + ctask_force::FLEET, fleet as u64);
        world.u32(task + ctask_force::MISSION, kind);
    }
    let walked = |world: &World| {
        let memory = world.memory();
        let countries = ccountry::Reader::new(&memory, MODULE, world.game_state()).unwrap();
        let mut visits = Vec::new();
        let mut order = Vec::new();
        cfleet::visit_task_forces(&memory, MODULE, &countries, |event| {
            match event {
                cfleet::Visit::Fleet { handle, .. } => order.push((handle, None)),
                cfleet::Visit::TaskForce(visit, index) => {
                    order.push((visit.id, Some(index)));
                    visits.push((visit.country.canonical_slot, visit.id, visit.kind));
                }
            }
            Ok(())
        })
        .map(|()| {
            assert_eq!(
                order,
                vec![
                    ((3 << 32) | 61, None),
                    ((10 << 32) | 61, Some(0)),
                    ((11 << 32) | 61, Some(1)),
                    ((12 << 32) | 61, Some(2)),
                ]
            );
            visits
        })
    };
    assert_eq!(
        walked(&world),
        Ok(vec![
            (1, (10 << 32) | 61, 4),
            (1, (11 << 32) | 61, 0),
            (1, (12 << 32) | 61, 3)
        ])
    );
    world.u64(task[1] + ctask_force::FLEET, 1);
    assert!(walked(&world).is_err());
}

#[test]
fn ledger_region_census_crosses_batches_and_refuses_short_headers() {
    let mut bytes = vec![0; 0x5000];
    array(
        &mut bytes,
        0x100 + crate::cstrategic_navy::REGION_MISSIONS,
        0x1000,
        130,
    );
    for region in [3, 129] {
        array(
            &mut bytes,
            0x1000 + region * crate::cstrategic_navy::REGION_MISSIONS_STRIDE,
            0x4000,
            1,
        );
    }
    let mut regions = Vec::new();
    assert_eq!(
        ledger_regions(
            &SliceMemory {
                base: 0,
                bytes: &bytes
            },
            0x100,
            |id, _| {
                regions.push(id);
                Ok(())
            }
        ),
        Ok(())
    );
    assert_eq!(regions, [3, 129]);
    let end = 0x1000 + 130 * crate::cstrategic_navy::REGION_MISSIONS_STRIDE;
    assert_eq!(
        ledger_regions(
            &SliceMemory {
                base: 0,
                bytes: &bytes[..end - 1]
            },
            0x100,
            |_, _| Ok(())
        ),
        Err("navy ledger region headers unreadable")
    );
}

#[test]
fn navy_hubs_allow_empty_lists_and_preserve_demand_order_and_grow_short_storage() {
    let mut bytes = vec![0; 0x3000];
    assert_eq!(
        navy_hubs_buffer(&SliceMemory {
                base: 0,
                bytes: &bytes
            }, 0x100, engine_native_memory::walk::Buffer::Count),
        Ok(0)
    );
    array(&mut bytes, 0x100 + HUBS, 0x1000, 300);
    for index in 0..300 {
        put_u32(&mut bytes, 0x1000 + index * 4, (index % 7) as u32);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut short = vec![u32::MAX; 2];
    assert_eq!(navy_hubs_buffer(&memory, 0x100, engine_native_memory::walk::Buffer::Growing(&mut short)), Ok(300));
    assert_eq!(short[..300], (0..300).map(|index| index % 7).collect::<Vec<_>>()[..]);
    assert_eq!(navy_hubs_buffer(&memory, 0x100, engine_native_memory::walk::Buffer::Count), Ok(300));
    let mut out = vec![0; 300];
    assert_eq!(navy_hubs_buffer(&memory, 0x100, engine_native_memory::walk::Buffer::Growing(&mut out)), Ok(300));
    assert_eq!(out, (0..300).map(|index| index % 7).collect::<Vec<_>>());
    assert_eq!(
        navy_hubs_buffer(&SliceMemory {
                base: 0,
                bytes: &bytes[..0x1000 + 300 * 4 - 1]
            }, 0x100, engine_native_memory::walk::Buffer::Growing(&mut out)),
        Err("navy supply hub unreadable")
    );
}

#[test]
fn a_ledger_keeps_its_missions_in_stored_order() {
    const NAVY: usize = 0x100;
    const LEDGERS: usize = 0x800;
    const MISSIONS: usize = 0x1800;
    let mut bytes = vec![0; 0x2000];
    put_ptr(&mut bytes, NAVY + cstrategic_navy::REGION_MISSIONS, LEDGERS);
    let header = LEDGERS + 7 * cstrategic_navy::REGION_MISSIONS_STRIDE;
    array(&mut bytes, header, MISSIONS, 3);
    for (index, mission) in [0x5360_u64, 0x4360, 0x5360].into_iter().enumerate() {
        put_u64(&mut bytes, MISSIONS + index * 8, mission);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let missions = Array::read(&memory, header, 8).unwrap();
    let mut out = [0; 3];
    assert_eq!(ledger(&memory, missions, &mut out), Ok(3));
    assert_eq!(out, [0x5360, 0x4360, 0x5360]);
    assert_eq!(out[1].checked_sub(MISSION), Some(0x4000));
    let mut short = [0; 2];
    assert_eq!(ledger(&memory, missions, &mut short), Ok(3));
    assert_eq!(short, [0; 2]);
    assert_eq!(ledger(&memory, Array::default(), &mut out), Ok(0));
    put_u64(&mut bytes, MISSIONS + 8, 0_u64);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert!(ledger(&memory, missions, &mut out).is_err());
}

#[test]
fn headquarters_sum_only_a_kind_two_unit_leaders_four_sources() {
    const CHARACTER: usize = 0x100;
    const LEADER: usize = 0x400;
    let mut bytes = vec![0; 0x4000];
    put_ptr(&mut bytes, CHARACTER, ccharacter::VTABLE_RVA.0);
    put_u64(
        &mut bytes,
        CHARACTER + CHARACTER_UNIT_LEADER_REFERENCE,
        (6_u64 << 32) | 4713,
    );
    put_u32(&mut bytes, LEADER + UNIT_LEADER_KIND, 2_i32 as u32);
    for (index, source) in UNIT_LEADER_SOURCES.into_iter().enumerate() {
        let object = 0x2000 + index * 0x200;
        put_ptr(&mut bytes, LEADER + source, object);
        accumulator(
            &mut bytes,
            object + SOURCE_MODIFIERS,
            object + 0x100,
            &[(0x19, index as i64 + 1)],
        );
    }
    let resolve = |handle: u64| {
        Ok(match handle >> 32 {
            5 => Some(CHARACTER),
            6 => Some(LEADER),
            _ => None,
        })
    };
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut out = [[Value::default(); 9]; 4];
    assert_eq!(
        headquarters_modifiers(&memory, 0, (5 << 32) | 73, &resolve, &mut out),
        Ok(())
    );
    assert_eq!(out.map(|values| values[0].raw), [1, 2, 3, 4]);
    assert_eq!(
        headquarters_modifiers(&memory, 0, (8 << 32) | 73, &resolve, &mut out),
        Ok(())
    );
    let mut other = bytes.clone();
    put_u32(&mut other, LEADER + UNIT_LEADER_KIND, 1_i32 as u32);
    let memory = SliceMemory {
        base: 0,
        bytes: &other,
    };
    assert_eq!(
        headquarters_modifiers(&memory, 0, (5 << 32) | 73, &resolve, &mut out),
        Ok(())
    );
}

#[test]
fn leader_modifiers_need_a_navy_leader() {
    const LEADER: usize = 0x100;
    let mut bytes = vec![0; 0x1000];
    put_ptr(&mut bytes, LEADER, cnavy_leader::VTABLE_RVA.0);
    accumulator(
        &mut bytes,
        LEADER + LEADER_MODIFIERS,
        0x800,
        &[(0xcc, 2_500)],
    );
    let resolve = |handle: u64| Ok((handle == 1).then_some(LEADER));
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut out = [Value::default(); 9];
    assert_eq!(leader_modifiers(&memory, 0, 1, &resolve, &mut out), Ok(()));
    assert_eq!(
        out[TASK_FORCE_KEYS
            .iter()
            .position(|&key| key == keys::CONVOY_RAIDING_EFFICIENCY_FACTOR)
            .unwrap()],
        Value {
            raw: 2_500,
            present: true
        }
    );
    assert_eq!(leader_modifiers(&memory, 0, 2, &resolve, &mut out), Ok(()));
    put_u64(&mut bytes, LEADER, 1_u64);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert!(leader_modifiers(&memory, 0, 1, &resolve, &mut out).is_err());
}

fn presence_map(entries: &[(u32, u32, u8, usize, i32, i32, i32)]) -> Vec<u8> {
    const NAVY: usize = 0x100;
    const ENTRIES: usize = 0x1000;
    let mut bytes = vec![0; 0x8000];
    put_ptr(&mut bytes, NAVY + PRESENCE_ENTRIES, ENTRIES);
    put_u32(&mut bytes, NAVY + PRESENCE_MASK, 7_i32 as u32);
    bytes[NAVY + PRESENCE_PROBE] = 2;
    for &(slot, key, distance, ring, capacity, head, tail) in entries {
        let entry = ENTRIES + slot as usize * PRESENCE_STRIDE;
        bytes[entry + 4] = distance;
        put_u32(&mut bytes, entry + 8, key);
        put_ptr(&mut bytes, entry + 0x10, ring);
        put_u32(&mut bytes, entry + 0x18, capacity as u32);
        put_u32(&mut bytes, entry + 0x1c, head as u32);
        put_u32(&mut bytes, entry + 0x20, tail as u32);
    }
    bytes
}

fn home(key: u32) -> u32 {
    let hashed = (key >> 16 ^ key).wrapping_mul(PRESENCE_HASH);
    let hashed = (hashed >> 16 ^ hashed).wrapping_mul(PRESENCE_HASH);
    (hashed >> 16 ^ hashed) & 7
}

#[test]
fn presence_follows_the_native_probe_and_counts_the_ring_window() {
    const NAVY: usize = 0x100;
    const RING: usize = 0x3000;
    let first = 11;
    let second = (12..10_000).find(|&key| home(key) == home(first)).unwrap();
    let slot = home(first);
    let mut bytes = presence_map(&[
        (slot, first, 1, RING, 4, 1, 0),
        (slot + 1, second, 2, RING + 0x100, 1_000, 998, 2),
    ]);
    for (index, value) in [7_i32, 2, -1, 0].into_iter().enumerate() {
        put_u32(&mut bytes, RING + index * 4, value as u32);
    }
    for (index, value) in [(998, 5_i32), (999, 0), (0, -3), (1, 1)] {
        put_u32(&mut bytes, RING + 0x100 + index * 4, value as u32);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        PresenceMap::read(&memory, NAVY)
            .unwrap()
            .presence(&memory, first),
        Ok(Some(Presence {
            positive: 1,
            total: 2
        }))
    );
    assert_eq!(
        PresenceMap::read(&memory, NAVY)
            .unwrap()
            .presence(&memory, second),
        Ok(Some(Presence {
            positive: 2,
            total: 3
        }))
    );
    let absent = (1..10_000)
        .find(|&key| key != first && key != second && home(key) == (slot + 4) % 8)
        .unwrap();
    assert_eq!(
        PresenceMap::read(&memory, NAVY)
            .unwrap()
            .presence(&memory, absent),
        Ok(None)
    );
    let mut long = presence_map(&[(slot, first, 1, RING, 1_000, 0, 999)]);
    for index in 0..999 {
        put_u32(&mut long, RING + index * 4, 1_i32 as u32);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &long,
    };
    assert_eq!(
        PresenceMap::read(&memory, NAVY)
            .unwrap()
            .presence(&memory, first),
        Ok(Some(Presence {
            positive: PRESENCE_WINDOW as i32,
            total: PRESENCE_WINDOW as i32
        }))
    );
    let mut short = presence_map(&[(slot, first, 1, RING, 4, 1, -1)]);
    for (index, value) in [7_i32, 2, -1, 0, 9].into_iter().enumerate() {
        put_u32(&mut short, RING + index * 4, value as u32);
    }
    let memory = SliceMemory {
        base: 0,
        bytes: &short,
    };
    assert_eq!(
        PresenceMap::read(&memory, NAVY)
            .unwrap()
            .presence(&memory, first),
        Ok(Some(Presence {
            positive: PRESENCE_WINDOW as i32 / 4 * 2,
            total: PRESENCE_WINDOW as i32 / 4 * 3
        }))
    );
}

#[test]
fn region_weather_reads_the_indexed_record_or_nothing_outside_the_array() {
    const STATE: usize = 0x100;
    const MANAGER: usize = 0x800;
    const RECORDS: usize = 0x1000;
    let mut bytes = vec![0; 0x2000];
    put_ptr(&mut bytes, STATE + cgame_state::WEATHER_MANAGER, MANAGER);
    put_ptr(&mut bytes, MANAGER, cweather_manager::VTABLE_RVA.0);
    array(&mut bytes, MANAGER + cweather_manager::REGIONS, RECORDS, 2);
    let record = RECORDS + sweather_per_region::STRIDE;
    put_ptr(&mut bytes, record, sweather_per_region::VTABLE_RVA.0);
    put_u32(&mut bytes, record + sweather_per_region::ID, 1_i32 as u32);
    accumulator(
        &mut bytes,
        record + sweather_per_region::MODIFIERS,
        0x1800,
        &[(0x19, -20_000)],
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let weather = cweather_manager::Reader::new(&memory, 0, STATE).unwrap();
    let mut out = [Value::default(); 9];
    region_weather(&memory, &weather, 1, &mut out).unwrap();
    assert_eq!(
        out,
        [
            Value {
                raw: -20_000,
                present: true
            },
            Value::default(),
            Value::default(),
            Value::default(),
            Value::default(),
            Value::default(),
            Value::default(),
            Value::default(),
            Value::default()
        ]
    );
    region_weather(&memory, &weather, 2, &mut out).unwrap();
    assert_eq!(out, [Value::default(); 9]);
}

#[test]
fn regional_raiding_packet_is_kind_and_activation_demanded() {
    use std::cell::RefCell;
    struct Trace<'a> {
        memory: SliceMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((at, out.len()));
            self.memory.copy(at, out)
        }
    }
    let mut bytes = task_force();
    let mission = TASK + MISSION;
    let mut resolver = engine_native_parts::creference_registry::ScopedResolver::new(0);
    let trace = Trace {
        memory: SliceMemory {
            base: 0,
            bytes: &bytes,
        },
        reads: Default::default(),
    };
    assert_eq!(
        read_raiding_grid(&trace, 0, visit(ESCORT), &mut resolver),
        None
    );
    assert!(trace.reads.borrow().is_empty());
    assert_eq!(
        read_raiding_grid(&trace, 0, visit(RAIDING), &mut resolver),
        Some(RaidingGrid::default())
    );
    assert_eq!(
        *trace.reads.borrow(),
        vec![(mission + cnaval_mission::IS_IN_REGIONS, 1)]
    );
    bytes[mission + cnaval_mission::IS_IN_REGIONS] = 7;
    put_u64(&mut bytes, mission + RAID_SPOTTING, (-50_001_i64) as u64);
    put_ptr(&mut bytes, mission + RAID_REGION, REGION);
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    assert_eq!(
        read_raiding_grid(&memory, 0, visit(RAIDING), &mut resolver),
        Some(RaidingGrid {
            active: 7,
            region: Some(44),
            spotting: Some(-50_001),
            targets: Some([Some(false); 2]),
            ..Default::default()
        })
    );
}

#[test]
fn active_raider_spotting_refusal_preserves_region_and_target_observations() {
    struct DenySpotting<'a>(SliceMemory<'a>);
    impl Memory for DenySpotting<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            if at == TASK + MISSION + RAID_SPOTTING {
                false
            } else {
                self.0.copy(at, out)
            }
        }
    }
    let mut bytes = task_force();
    bytes[TASK + MISSION + cnaval_mission::IS_IN_REGIONS] = 1;
    put_ptr(&mut bytes, TASK + MISSION + RAID_REGION, REGION);
    let row = read_raiding_grid(
        &DenySpotting(SliceMemory {
            base: 0,
            bytes: &bytes,
        }),
        0,
        visit(RAIDING),
        &mut engine_native_parts::creference_registry::ScopedResolver::new(0),
    )
    .unwrap();
    assert_eq!(row.spotting, None);
    assert_eq!(row.region, Some(44));
    assert_eq!(row.targets, Some([Some(false); 2]));
}

#[test]
fn original_selector_observations_keep_first_fault_and_successful_prefix() {
    struct Deny<'a> {
        memory: SliceMemory<'a>,
        at: usize,
    }
    impl Memory for Deny<'_> {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            at != self.at && self.memory.copy(at, out)
        }
    }
    let bytes = task_force();
    for (denied, stats, location) in [
        (
            TASK + cunit::STAT_BLOCK,
            Some(Err("original task-force stat selector unreadable")),
            None,
        ),
        (TASK + UNDERWAY, Some(Ok(STATS)), Some(Ok(PROVINCE))),
    ] {
        let memory = Deny {
            memory: SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            at: denied,
        };
        let mut selectors = OriginalSelectors::default();
        let result = read_task_force_transport_observed_buffer_with_stats(
            &memory,
            0,
            visit(2),
            engine_native_memory::walk::Buffer::Count,
            &mut None,
            &mut None,
            &mut selectors,
        );
        assert!(result.is_err());
        assert_eq!(selectors.stats, stats);
        assert_eq!(selectors.location, location);
    }
}
