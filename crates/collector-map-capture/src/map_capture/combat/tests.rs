use super::*;
use crate::map_capture::{MapCapacities, NoSources};
use engine_land_inputs::cstats_observer as stats;
use engine_native::{
    carmy,
    cequipment_variant::pool as pool,
    cgame_state,
    cland_combatant as side,
    cprovince,
    cunit,
};

struct World {
    state: Vec<u8>,
    _countries: Vec<usize>,
    _province: Vec<u8>,
    _provinces: Vec<usize>,
    _combats: Vec<Vec<u8>>,
    _sides: Vec<[Vec<u8>; 2]>,
    _units: Vec<Vec<u8>>,
    _lists: [Vec<usize>; 2],
    _registry: Vec<usize>,
}

fn q(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
fn d(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn array(bytes: &mut [u8], offset: usize, pointer: usize, count: usize) {
    q(bytes, offset, pointer as u64);
    d(bytes, offset + 8, count as u32);
    d(bytes, offset + 12, count as u32);
}

impl World {
    fn new(combats: usize, members: usize) -> Self {
        let mut units: Vec<Vec<u8>> = (0..members * 2)
            .map(|index| {
                let mut bytes = vec![0; (carmy::ORGANISATION + 8).max(cunit::MAP_ENTITY + 8)];
                q(&mut bytes, 0, carmy::VTABLE_RVA.0 as u64);
                q(
                    &mut bytes,
                    carmy::SECONDARY_BASE,
                    carmy::SECONDARY_VTABLE_RVA.0 as u64,
                );
                q(&mut bytes, cunit::HANDLE, (index as u64 + 1) << 32 | 7);
                q(
                    &mut bytes,
                    carmy::ORGANISATION,
                    (index as u64 + 1) * 100_000,
                );
                bytes
            })
            .collect();
        let lists = std::array::from_fn(|side| {
            units[side * members..(side + 1) * members]
                .iter_mut()
                .map(|bytes| bytes.as_ptr() as usize)
                .collect::<Vec<_>>()
        });
        let mut province = vec![0; cprovince::ID + 4];
        q(&mut province, 0, cprovince::VTABLE_RVA.0 as u64);
        d(&mut province, cprovince::ID, 1);
        let provinces = vec![0, province.as_ptr() as usize];
        let mut battle_objects = (0..combats)
            .map(|_| vec![0; cland_combat::DURATION + 4])
            .collect::<Vec<_>>();
        let mut sides = (0..combats)
            .map(|_| {
                std::array::from_fn(|_| {
                    vec![
                        0;
                        side::STATS_OBSERVER + engine_land_inputs::combat_log_events::OBSERVER_SIZE
                    ]
                })
            })
            .collect::<Vec<[Vec<u8>; 2]>>();
        for (index, (combat, sides)) in battle_objects.iter_mut().zip(&mut sides).enumerate() {
            q(combat, 0, cland_combat::VTABLE_RVA.0 as u64);
            q(
                combat,
                cland_combat::BODY,
                cland_combat::BODY_VTABLE_RVA.0 as u64,
            );
            q(
                combat,
                cland_combat::HANDLE,
                (index as u64 + 1) << 32 | cland_combat::TYPE_WORD as u64,
            );
            q(combat, cland_combat::PROVINCE, province.as_ptr() as u64);
            for (number, side) in sides.iter_mut().enumerate() {
                q(
                    combat,
                    [cland_combat::ATTACKER, cland_combat::DEFENDER][number],
                    side.as_ptr() as u64,
                );
                q(side, 0, side::VTABLE_RVA.0 as u64);
                q(side, side::COMBAT, combat.as_ptr() as u64);
                side[side::IS_ATTACKER] = u8::from(number == 0);
                array(side, side::UNITS, lists[number].as_ptr() as usize, members);
                q(side, side::STATS_OBSERVER, stats::VTABLE_RVA.0 as u64);
                q(
                    side,
                    side::STATS_OBSERVER + stats::EQUIPMENT_LOSSES,
                    pool::VTABLE_RVA.0 as u64,
                );
                q(
                    side,
                    side::STATS_OBSERVER + engine_land_inputs::combat_log_events::OBSERVER_SIDE,
                    engine_land_inputs::combat_log_history::SIDE_VTABLE_RVA.0 as u64,
                );
                for offset in [
                    engine_land_inputs::combat_log_history::SIDE_CAPTURED_BY_ENEMY,
                    engine_land_inputs::combat_log_history::SIDE_RECOVERED,
                ] {
                    q(
                        side,
                        side::STATS_OBSERVER
                            + engine_land_inputs::combat_log_events::OBSERVER_SIDE
                            + offset,
                        pool::VTABLE_RVA.0 as u64,
                    );
                }
            }
        }
        let registry = battle_objects
            .iter()
            .map(|bytes| bytes.as_ptr() as usize)
            .collect::<Vec<_>>();
        let countries = vec![0; 5001];
        let mut state = vec![0; cgame_state::COUNTRIES + 16];
        array(
            &mut state,
            cgame_state::COUNTRIES,
            countries.as_ptr() as usize,
            countries.len(),
        );
        array(
            &mut state,
            cgame_state::PROVINCES,
            provinces.as_ptr() as usize,
            provinces.len(),
        );
        array(
            &mut state,
            cgame_state::COMBATS + 8,
            registry.as_ptr() as usize,
            registry.len(),
        );
        Self {
            state,
            _countries: countries,
            _province: province,
            _provinces: provinces,
            _combats: battle_objects,
            _sides: sides,
            _units: units,
            _lists: lists,
            _registry: registry,
        }
    }
    fn state(&self) -> usize {
        self.state.as_ptr() as usize
    }
}
impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&(self.state() as u64).to_le_bytes());
            true
        } else {
            unsafe { engine_native_memory::memory::LiveMemory::new() }.copy(address, output)
        }
    }
}

fn empty_map() -> MapCaptured {
    // SAFETY: These are session-measured map capacities or capacities owned by an isolated
    // fixture.
    unsafe {
        MapCaptured::with_capacities(&MapCapacities {
            rows: [0; MAP_TABLES],
            player_names: 0,
            air_wing_names: 0,
            land_participants: 0,
            order_geometry: 0,
            order_groups: 0,
            order_visits: 0,
            naval_groups: 0,
        })
    }
}
fn progress() -> combat_progress::Captured {
    combat_progress::Captured::with_capacities(&combat_progress::Capacities::default())
}
struct NoReads;
impl Memory for NoReads {
    fn copy(&self, _: usize, _: &mut [u8]) -> bool {
        panic!("callback repeated a worker combat read")
    }
}

#[test]
fn combat_log_rejected_request_is_terminal_and_finished_land_survives_later_failure() {
    let mut map = empty_map();
    let request =
    // SAFETY: The admitted hour owns this frame and takes its combat job once.
    unsafe { map.take_combat_request(0, 0, progress(), true) };
    let unavailable = Outcome::Unavailable {
        reason: "combat worker rejected",
    };
    let returned =
    // SAFETY: The admitted hour is settling its own combat request in the originating frame.
    unsafe { map.restore_combat_request(
        request,
        CombatReport::default(),
        Some("combat worker rejected"),
    ) };
    assert_eq!(map.land_combats.outcome, unavailable);
    assert_eq!(map.naval_combats.outcome, unavailable);
    assert_eq!(map.naval_combat_participants.outcome, unavailable);
    assert_eq!(returned.outcomes(), [unavailable; combat_progress::N]);
    map.players_raw_mode = true;
    map.owned_air_captured = true;
    // SAFETY: The admitted hour or isolated fixture owns this map and the corresponding game
    // image.
    unsafe { map.sample(&NoReads, 0, 0, &mut NoSources) };

    map.reset();
    let world = World::new(1, 2);
    let mut request =
    // SAFETY: The admitted hour owns this frame and takes its combat job once.
    unsafe { map.take_combat_request(0, world.state(), returned, true) };
    let report =
    // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
    unsafe { request.capture(&world) };
    request.naval_finished = false;
    let returned =
    // SAFETY: The admitted hour is settling its own combat request in the originating frame.
    unsafe { map.restore_combat_request(request, report, Some("naval worker panicked")) };
    assert_eq!(map.land_combats.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        returned.outcomes(),
        [Outcome::Rows { len: 1 }, Outcome::Rows { len: 22 }]
    );
    assert_eq!(
        map.naval_combats.outcome,
        Outcome::Unavailable {
            reason: "naval worker panicked"
        }
    );
}

#[test]
fn combat_log_take_restore_moves_owned_storage_without_callback_allocation() {
    let world = World::new(1, 2);
    let mut map = empty_map();
    let mut request =
    // SAFETY: The admitted hour owns this frame and takes its combat job once.
    unsafe { map.take_combat_request(0, world.state(), progress(), true) };
    let report =
    // SAFETY: The admitted capture or its fixture owns this frame's observer sequencing.
    unsafe { request.capture(&world) };
    let progress =
    // SAFETY: The admitted hour is settling its own combat request in the originating frame.
    unsafe { map.restore_combat_request(request, report, None) };
    let original = (
        map.land_combats.rows.as_ptr(),
        map.land_participants.as_ptr(),
    );
    let request = collector_test_support::with_allocations_denied(|| {
        let request =
        // SAFETY: The admitted hour owns this frame and takes its combat job once.
        unsafe { map.take_combat_request(0, world.state(), progress, true) };
        assert_eq!(map.land_combats.rows.capacity(), 0);
        assert_eq!(map.land_participants.capacity(), 0);
        let progress =
        // SAFETY: The admitted hour is settling its own combat request in the originating frame.
        unsafe { map.restore_combat_request(request, report, None) };
        // SAFETY: The admitted hour owns this frame and takes its combat job once.
        unsafe { map.take_combat_request(0, world.state(), progress, true) }
    });
    // SAFETY: The admitted hour is settling its own combat request in the originating frame.
    unsafe { map.restore_combat_request(request, report, None) };
    assert_eq!(
        (
            map.land_combats.rows.as_ptr(),
            map.land_participants.as_ptr()
        ),
        original
    );
}

#[test]
fn map_combat_initial_hints_follow_native_counts_without_country_ratios() {
    let mut capacity = MapCapacities {
        rows: [0; MAP_TABLES],
        player_names: 0,
        air_wing_names: 0,
        land_participants: 0,
        order_geometry: 0,
        order_groups: 0,
        order_visits: 0,
        naval_groups: 0,
    };
    for (combats, members) in [(5000, 0), (7000, 0), (1, 12_000)] {
        let world = World::new(combats, members);
        capacity.presize_combats(&world, 0).unwrap();
        assert!(capacity.rows[2] >= combats);
        assert!(capacity.land_participants >= combats * members * 2);
    }
}
