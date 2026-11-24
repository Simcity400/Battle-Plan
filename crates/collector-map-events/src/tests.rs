use crate::{hooks, interval, lanes};
use engine_events::map_event_sites as site;
use engine_native::{cgame_state, cprovince, cunit};
use replay_data::{
    rows::MapEventBuildingDamage, rows::MapEventBuildingReason, rows::MapEventFamily as Family,
    rows::MapEventFault as Fault, rows::MapEventIntelEntry, rows::MapEventPayload,
    rows::MapEventUnitIntel,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) static LOCK: Mutex<()> = Mutex::new(());

struct World {
    bytes: Vec<u64>,
    next: usize,
}

impl World {
    fn new() -> Self {
        Self {
            bytes: vec![0; 1 << 16],
            next: 0x1000,
        }
    }
    fn start(&self) -> usize {
        self.bytes.as_ptr() as usize
    }
    fn base(&self) -> usize {
        self.start() - cgame_state::SINGLETON_RVA.0
    }
    fn alloc(&mut self, size: usize) -> usize {
        let at = self.start() + self.next;
        self.next += (size + 15) & !15;
        assert!(self.next <= self.bytes.len() * 8);
        at
    }
    fn put(&mut self, at: usize, value: &[u8]) {
        let offset = at - self.start();
        let bytes = unsafe {
            std::slice::from_raw_parts_mut(
                self.bytes.as_mut_ptr().cast::<u8>(),
                self.bytes.len() * 8,
            )
        };
        bytes[offset..offset + value.len()].copy_from_slice(value);
    }
    fn u32(&mut self, at: usize, value: u32) {
        self.put(at, &value.to_le_bytes());
    }
    fn u64(&mut self, at: usize, value: u64) {
        self.put(at, &value.to_le_bytes());
    }
    fn array(&mut self, at: usize, data: usize, count: u32) {
        self.u64(at, data as u64);
        self.u32(at + 8, count);
        self.u32(at + 12, count);
    }
    fn game(
        &mut self,
        hour: i32,
        fog: bool,
        count: u32,
        controller: impl Fn(u32) -> u32,
    ) -> (usize, Vec<usize>) {
        let state = self.alloc(0x800);
        let singleton = self.base() + cgame_state::SINGLETON_RVA.0;
        self.u64(singleton, state as u64);
        let fog_at = self.base() + site::FOG_OF_WAR_RVA.0;
        self.put(fog_at, &[u8::from(fog)]);
        self.u32(state + cgame_state::CURRENT_HOUR, hour as u32);
        let table = self.alloc((count as usize + 1) * 8);
        self.array(state + cgame_state::PROVINCES, table, count + 1);
        let mut provinces = vec![0];
        for id in 1..=count {
            let province = self.alloc(0x200);
            self.u32(province + cprovince::ID, id);
            self.u32(province + cprovince::CONTROLLER, controller(id));
            self.u64(table + id as usize * 8, province as u64);
            provinces.push(province);
        }
        (state, provinces)
    }
}

static ORIGINALS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system-unwind" fn native_building(
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
) -> usize {
    ORIGINALS.fetch_add(1, Ordering::SeqCst);
    0x5150
}

unsafe extern "system-unwind" fn native_visibility(unit: usize) -> usize {
    ORIGINALS.fetch_add(1, Ordering::SeqCst);
    unit ^ 0x96
}

fn reset() {
    lanes::reset_for_tests();
    interval::reset_for_tests();
    for family in Family::ALL {
        hooks::set_for_tests(family, true, true);
        let _ = hooks::take_faults(family);
    }
    hooks::reset_missed();
    lanes::grow(2, lanes::MINIMUM_LANE_BYTES).unwrap();
}

fn drain(hour: i32) -> replay_data::rows::MapEventsRow {
    let sealed = unsafe { interval::seal(hour, Some(crate::world_lifetime::generation())) };
    let mut assembled = lanes::Assembled::default();
    // SAFETY: tests serialize lane access with `LOCK` and are the sole assembler.
    unsafe { lanes::assemble(sealed.seal, &mut assembled) }.unwrap();
    let row = interval::row(&sealed, &assembled).unwrap();
    // SAFETY: `assembled` was produced by `lanes::assemble` under `LOCK`.
    unsafe { assembled.commit() };
    row
}

#[test]
fn scripted_refresh_call_publishes_the_mode_ordinal_and_forwards_even_on_read_faults() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    unsafe extern "system-unwind" fn native_refresh(manager: usize, refresh_all: u8) {
        assert_eq!((manager, refresh_all), (0x42, 1));
        assert_eq!(hooks::active(), 1);
        ORIGINALS.fetch_add(1, Ordering::SeqCst);
    }
    ORIGINALS.store(0, Ordering::SeqCst);
    hooks::set_original_for_tests(
        Family::ScriptedMapRefresh,
        native_refresh as *const () as usize,
    );
    let mut world = World::new();
    world.game(77, false, 1, |_| 0);
    let mode = world.alloc(8);
    world.u32(mode, site::FIRST_SCRIPTED_MAP_MODE as u32 + 3);
    world.u64(world.base() + site::MAP_MODE_SINGLETON_RVA.0, mode as u64);
    hooks::set_base_for_tests(world.base());
    unsafe {
        hooks::scripted_refresh_for_tests(0x42, 1);
    }
    let row = drain(77);
    assert_eq!(row.events.len(), 1);
    assert_eq!(
        row.events[0].payload,
        MapEventPayload::ScriptedMapRefresh(3)
    );
    world.u64(world.base() + site::MAP_MODE_SINGLETON_RVA.0, 0);
    unsafe {
        hooks::scripted_refresh_for_tests(0x42, 1);
    }
    let row = drain(78);
    let health = row.health(Family::ScriptedMapRefresh);
    assert!(row.events.is_empty());
    assert!(!health.complete && health.faults.contains(Fault::Read));
    assert_eq!(health.missed, 1);
    assert_eq!(hooks::active(), 0);
    hooks::set_for_tests(Family::ScriptedMapRefresh, true, false);
    unsafe { hooks::scripted_refresh_for_tests(0x42, 1) };
    assert!(drain(79).events.is_empty());
    assert_eq!(ORIGINALS.load(Ordering::SeqCst), 3);
    assert_eq!(hooks::active(), 0);
}

#[test]
fn a_building_notification_records_only_a_lowered_level_with_text_and_a_reason() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    let mut world = World::new();
    let (state, provinces) = world.game(77, true, 3, |id| id + 10);
    hooks::set_base_for_tests(world.base());
    hooks::set_original_for_tests(
        Family::BuildingDamage,
        native_building as *const () as usize,
    );
    let status = world.alloc(0x80);
    world.u32(status + site::BUILDING_STATUS_FLAGS, 1);
    world.u32(status + site::BUILDING_STATUS_ID, 2);
    let building = world.alloc(0x200);
    world.u32(building + site::BUILDING_TYPE, 41);
    world.u64(building + site::BUILDING_STATUS, status as u64);
    world.put(building + site::BUILDING_LEVEL, &3_u16.to_le_bytes());
    world.put(
        building + site::BUILDING_HEALTHY_LEVEL,
        &2_i16.to_le_bytes(),
    );
    world.u64(building + site::BUILDING_PARTIAL_HEALTH, 4_000_000);
    let source = world.alloc(16);
    let _ = (state, provinces);
    let call =
        |reason: u32, level: usize, before: i64, client: usize, text: usize, world: &mut World| {
            world.u32(source, reason);
            unsafe {
                hooks::building_notification_for_tests(
                    building,
                    source,
                    level,
                    before as usize,
                    client,
                    text,
                )
            }
        };
    ORIGINALS.store(0, Ordering::SeqCst);
    assert_eq!(call(1, 3, 10_000_000, 1, 1, &mut world), 0x5150);
    call(4, 3, 1_000_000, 1, 1, &mut world);
    call(1, 3, 4_000_000, 1, 1, &mut world);
    call(1, 2, 10_000_000, 1, 1, &mut world);
    call(1, 3, 10_000_000, 1, 0, &mut world);
    call(1, 3, 10_000_000, 0, 1, &mut world);
    call(0, 3, 10_000_000, 1, 1, &mut world);
    call(3, 0, 10_000_000 + 1, 1, 1, &mut world);
    assert_eq!(ORIGINALS.load(Ordering::SeqCst), 8);
    let row = drain(77);
    let expected = |reason| {
        MapEventPayload::BuildingDamage(MapEventBuildingDamage {
            building_type: 41,
            province: 2,
            controller: 12,
            reason,
        })
    };
    assert_eq!(
        row.events
            .iter()
            .map(|event| (event.hour, event.payload.clone()))
            .collect::<Vec<_>>(),
        vec![
            (77, expected(MapEventBuildingReason::Bombed)),
            (77, expected(MapEventBuildingReason::Collateral)),
        ]
    );
    assert!(
        row.health(Family::BuildingDamage)
            .faults
            .contains(Fault::ObservationStart)
    );
    let quiet = drain(78);
    assert!(quiet.events.is_empty());
    assert!(quiet.health(Family::BuildingDamage).complete);
    assert!(quiet.continues(&row));
    assert_eq!(hooks::active(), 0);
}

#[test]
fn a_revealed_fake_division_keeps_its_owner_province_fog_and_whole_intel_array() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    let mut world = World::new();
    let (_, provinces) = world.game(-3, false, 4, |_| 0);
    hooks::set_base_for_tests(world.base());
    hooks::set_original_for_tests(
        Family::FakeDivisionRevealed,
        native_visibility as *const () as usize,
    );
    let unit = world.alloc(0x300);
    world.u32(unit + cunit::ROSTER_OWNER, 0xdead_beef);
    world.u64(unit + cunit::LOCATION, provinces[3] as u64);
    let entries = 40_u32;
    let intel = world.alloc(entries as usize * site::INTEL_STRIDE);
    world.array(unit + cunit::VIEWER_DATA, intel, entries);
    for index in 0..entries as usize {
        world.u32(intel + index * site::INTEL_STRIDE, (index % 7) as u32);
        world.put(
            intel + index * site::INTEL_STRIDE + site::INTEL_LEVEL,
            &[(index as u8).wrapping_mul(37)],
        );
    }
    ORIGINALS.store(0, Ordering::SeqCst);
    assert_eq!(unsafe { hooks::fake_reveal_for_tests(unit) }, unit ^ 0x96);
    assert_eq!(ORIGINALS.load(Ordering::SeqCst), 1);
    let row = drain(-3);
    assert_eq!(row.events.len(), 1);
    assert_eq!(
        row.events[0].payload,
        MapEventPayload::FakeDivisionRevealed(MapEventUnitIntel {
            owner: 0xdead_beef,
            anchor_province: 3,
            visibility_province: 3,
            fog: false,
            intel: (0..entries as usize)
                .map(|index| MapEventIntelEntry {
                    country: (index % 7) as u32,
                    level: (index as u8).wrapping_mul(37) as i8,
                })
                .collect(),
        })
    );
    let unreadable = 0x10;
    unsafe { hooks::fake_reveal_for_tests(unreadable) };
    assert_eq!(ORIGINALS.load(Ordering::SeqCst), 2);
    let row = drain(-2);
    assert!(row.events.is_empty());
    let health = row.health(Family::FakeDivisionRevealed);
    assert!(!health.complete && health.faults.contains(Fault::Read));
    assert_eq!(health.missed, 1);
}

#[test]
fn a_disabled_family_records_nothing_and_its_interval_is_unknown() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    let mut world = World::new();
    world.game(5, true, 2, |_| 0);
    hooks::set_base_for_tests(world.base());
    hooks::set_original_for_tests(
        Family::FakeDivisionRevealed,
        native_visibility as *const () as usize,
    );
    hooks::set_for_tests(Family::FakeDivisionRevealed, true, false);
    hooks::set_for_tests(Family::ParadropLanding, false, false);
    let unit = world.alloc(0x300);
    unsafe { hooks::fake_reveal_for_tests(unit) };
    let row = drain(5);
    assert!(row.events.is_empty());
    let fake = row.health(Family::FakeDivisionRevealed);
    assert!(fake.installed && !fake.enabled && fake.faults.contains(Fault::Disabled));
    let paradrop = row.health(Family::ParadropLanding);
    assert!(!paradrop.installed && paradrop.faults.contains(Fault::Unavailable));
}

#[test]
fn a_new_epoch_restarts_coverage_and_the_sequence_continues_the_history() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    lanes::reset_for_tests();
    interval::reset_for_tests();
    interval::begin(4, 900).unwrap();
    lanes::grow(1, lanes::MINIMUM_LANE_BYTES).unwrap();
    for family in Family::ALL {
        hooks::set_for_tests(family, true, true);
        let _ = hooks::take_faults(family);
    }
    let first = drain(10);
    assert_eq!(
        (first.epoch, first.first_sequence, first.next_sequence),
        (5, 901, 901)
    );
    let second = drain(11);
    assert!(second.continues(&first));
    assert!(second.health.iter().all(|health| health.complete));
    unsafe { interval::invalidate() };
    let reset = drain(12);
    assert_eq!(reset.epoch, 6);
    assert!(!reset.continues(&second));
    assert!(
        reset
            .health
            .iter()
            .all(|health| health.faults.contains(Fault::ObservationStart))
    );
    unsafe { interval::lost_output() };
    let lost = drain(13);
    assert!(
        lost.health
            .iter()
            .all(|health| health.faults.contains(Fault::OutputLost))
    );
}

#[test]
fn an_event_left_pending_by_an_ended_world_is_never_listed_by_the_next_one() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    let seal = |hour, world| {
        let sealed = unsafe { interval::seal(hour, world) };
        let mut assembled = lanes::Assembled::default();
        // SAFETY: test holds `LOCK` as the sole assembler.
        unsafe { lanes::assemble(sealed.seal, &mut assembled) }.unwrap();
        let row = interval::row(&sealed, &assembled).unwrap();
        // SAFETY: `assembled` was produced by `lanes::assemble` under `LOCK`.
        unsafe { assembled.commit() };
        row
    };
    let critical = |world| {
        lanes::publish(
            Family::CriticalShoreBombardment as u8,
            7,
            world,
            4,
            |writer| {
                writer.u32(77);
                Ok(())
            },
        )
        .unwrap()
    };
    let first = seal(6, Some(3));
    critical(3);
    let old = seal(7, Some(3));
    assert_eq!(old.events.len(), 1);
    assert!(old.continues(&first));
    critical(3);
    let stale = interval::stale();
    let loaded = seal(8, Some(4));
    assert!(loaded.events.is_empty());
    assert_eq!(interval::stale(), stale + 1);
    assert!(!loaded.continues(&old));
    assert!(
        loaded
            .health
            .iter()
            .all(|health| health.faults.contains(Fault::ObservationStart))
    );
    let new = critical(4);
    let next = seal(9, Some(4));
    assert_eq!(
        next.events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [new]
    );
    critical(4);
    let unadmitted = seal(10, None);
    assert!(unadmitted.events.is_empty());
    assert!(unadmitted.health.iter().all(|health| !health.complete));
}

#[test]
fn a_world_change_while_the_operands_are_read_refuses_the_event() {
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    let family = Family::CriticalShoreBombardment;
    let missed = hooks::missed(family);
    let old = crate::world_lifetime::generation();
    let overlapped = hooks::publish_in_world(family, 7, old, 4, |writer| {
        writer.u32(77);
        crate::world_lifetime::invalidate_for_test();
        Ok(())
    });
    let Err(refusal) = overlapped else {
        panic!("an event read across a world change was published");
    };
    assert!(matches!(refusal, lanes::Refusal::Read(_)));
    hooks::refuse_for_tests(family, refusal);
    assert_eq!(hooks::missed(family), missed + 1);
    let new = crate::world_lifetime::generation();
    assert_ne!(new, old);
    let published = hooks::publish_in_world(family, 8, new, 4, |writer| {
        writer.u32(78);
        Ok(())
    })
    .unwrap();
    let sealed = unsafe { interval::seal(8, Some(new)) };
    let mut assembled = lanes::Assembled::default();
    // SAFETY: test holds `LOCK` as the sole assembler.
    unsafe { lanes::assemble(sealed.seal, &mut assembled) }.unwrap();
    let row = interval::row(&sealed, &assembled).unwrap();
    // SAFETY: `assembled` was produced by `lanes::assemble` under `LOCK`.
    unsafe { assembled.commit() };
    assert_eq!(
        row.events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [published]
    );
    assert!(
        !row.health[family.index()].complete
            && row.health[family.index()].faults.contains(Fault::Read)
    );
}

#[test]
fn the_building_gate_compares_signed_health_like_the_game() {
    assert!(hooks::building_shows(1, 1, 3, 10_000_000, 9_999_999));
    assert!(hooks::building_shows(1, 1, 1, 0, -1));
    assert!(!hooks::building_shows(1, 1, 1, -1, 0));
    assert!(!hooks::building_shows(1, 1, 1, 5, 5));
    assert!(!hooks::building_shows(0, 1, 1, 5, 4));
    assert!(!hooks::building_shows(1, 0, 1, 5, 4));
    assert!(!hooks::building_shows(1, 1, 0, 5, 4));
}

#[test]
fn every_payload_layout_decodes_and_trailing_or_short_bytes_refuse() {
    let mut paradrop = Vec::new();
    paradrop.extend(9_u32.to_le_bytes());
    paradrop.extend(0_u32.to_le_bytes());
    paradrop.extend([1, 0]);
    assert_eq!(
        interval::payload(Family::ParadropLanding as u8, &paradrop),
        Ok(MapEventPayload::ParadropLanding(
            replay_data::rows::MapEventParadrop {
                province: 9,
                country: 0,
                lost: true,
                fog: false
            }
        ))
    );
    assert!(interval::payload(Family::ParadropLanding as u8, &paradrop[..9]).is_err());
    paradrop.push(0);
    assert!(interval::payload(Family::ParadropLanding as u8, &paradrop).is_err());
    let mut resistance = Vec::new();
    for value in [7_u32, 3, 0, 4] {
        resistance.extend(value.to_le_bytes());
    }
    resistance.extend(b"KEY_");
    match interval::payload(Family::ResistanceActivity as u8, &resistance).unwrap() {
        MapEventPayload::ResistanceActivity(activity) => {
            assert_eq!(
                (activity.province, activity.controller, activity.overlord),
                (7, 3, 0)
            );
            assert_eq!(activity.key, b"KEY_");
        }
        other => panic!("{other:?}"),
    }
    let mut building = Vec::new();
    for value in [1_u32, 2, 3] {
        building.extend(value.to_le_bytes());
    }
    building.push(6);
    assert!(interval::payload(Family::BuildingDamage as u8, &building).is_err());
    assert!(interval::payload(7, &[]).is_err());
}

unsafe extern "system-unwind" fn native_building_quiet(
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
) -> usize {
    0
}

#[test]
fn a_burst_past_the_lanes_with_hours_in_flight_keeps_every_event_and_completes_every_interval() {
    use replay_data::rows::MapEventCriticalBombardment;
    let _lock = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    reset();
    let family = Family::CriticalShoreBombardment;
    let world = crate::world_lifetime::generation();
    drain(1);
    let missed = hooks::missed(family);
    let per_hour = 3_000_u32;
    let mut queued = Vec::new();
    let mut province = 0_u32;
    for hour in 2..5 {
        let mut expected = Vec::new();
        for _ in 0..per_hour {
            province += 1;
            match hooks::publish_in_world(family, hour, world, 4, |writer| {
                writer.u32(province);
                Ok(())
            }) {
                Ok(sequence) => expected.push((sequence, province)),
                Err(refusal) => hooks::refuse_for_tests(family, refusal),
            }
        }
        let sealed = unsafe { interval::seal(hour, Some(world)) };
        let mut assembled = lanes::Assembled::default();
        // SAFETY: test holds `LOCK` as the sole assembler.
        unsafe { lanes::assemble(sealed.seal, &mut assembled) }.unwrap();
        let row = interval::row(&sealed, &assembled).unwrap();
        assert_eq!(expected.len(), per_hour as usize, "hour {hour}");
        assert_eq!(
            row.events
                .iter()
                .map(|event| (event.sequence, event.payload.clone()))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|&(sequence, province)| (
                    sequence,
                    MapEventPayload::CriticalShoreBombardment(MapEventCriticalBombardment {
                        province
                    })
                ))
                .collect::<Vec<_>>(),
            "hour {hour}"
        );
        assert!(row.health[family.index()].complete, "hour {hour}");
        // SAFETY: test holds `LOCK` and admits intervals in assembly order.
        unsafe { assembled.admitted() };
        queued.push(assembled);
    }
    assert_eq!(hooks::missed(family), missed);
    assert!(lanes::capacity().spill_pending > 0);
    for assembled in queued {
        // SAFETY: test holds `LOCK` and commits intervals in assembly order.
        unsafe { assembled.commit() };
    }
    assert_eq!(lanes::capacity().spill_pending, 0);
    assert!(drain(5).events.is_empty());
}
