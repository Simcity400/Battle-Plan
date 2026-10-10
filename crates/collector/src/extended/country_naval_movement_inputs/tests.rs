use super::*;
use engine::{
    cgame_state, cnaval_base, cprovince, cstrategic_navy, cstrategic_navy_manager, ctheatre,
};
use std::cell::RefCell;

mod launch;

const STATE: usize = 0x1000;
const OWNER: usize = 0x8000;
const ZERO: usize = 0x4000;
const MANAGER: usize = 0xa000;
const NAVY: usize = 0xb000;
const FOREIGN_NAVY: usize = 0xb800;

struct TestMemory {
    bytes: Vec<u8>,
    fault: Option<usize>,
    calls: RefCell<Vec<(usize, usize)>>,
    reserved: Option<Result<ccountry::Country, &'static str>>,
}
impl Memory for TestMemory {
    fn authenticated_reserved_country(
        &self,
        base: usize,
        state: usize,
    ) -> Option<Result<ccountry::Country, &'static str>> {
        if base == 0 && state == STATE {
            self.reserved
        } else {
            None
        }
    }
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, destination.len()));
        if self.fault.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        }) {
            return false;
        }
        let Some(source) = address
            .checked_add(destination.len())
            .and_then(|end| self.bytes.get(address..end))
        else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}
fn word(bytes: &mut [u8], address: usize, value: u32) {
    bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
}
fn pointer(bytes: &mut [u8], address: usize, value: usize) {
    bytes[address..address + 8].copy_from_slice(&(value as u64).to_le_bytes());
}
fn header(bytes: &mut [u8], address: usize, data: usize, count: u32) {
    pointer(bytes, address, data);
    word(bytes, address + 8, count);
    word(bytes, address + 12, count);
}
fn world() -> TestMemory {
    let mut bytes = vec![0; 0x20000];
    header(&mut bytes, STATE + cgame_state::COUNTRIES, 0x3000, 4);
    header(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, 0x3100, 4);
    for (slot, country) in [(0, ZERO), (1, OWNER), (3, OWNER)] {
        pointer(&mut bytes, 0x3000 + slot * 8, country);
    }
    for (slot, canonical) in [(0, 0), (1, 1), (2, 2), (3, 1)] {
        word(&mut bytes, 0x3100 + slot * 4, canonical);
    }
    for (country, id) in [(ZERO, 0), (OWNER, 1)] {
        pointer(&mut bytes, country, ccountry::VTABLE_RVA);
        word(&mut bytes, country + ccountry::ID, id);
    }
    pointer(
        &mut bytes,
        STATE + cgame_state::STRATEGIC_NAVY_MANAGER,
        MANAGER,
    );
    pointer(&mut bytes, MANAGER, cstrategic_navy_manager::VTABLE_RVA);
    header(
        &mut bytes,
        MANAGER + cstrategic_navy_manager::NAVIES,
        0xa100,
        4,
    );
    pointer(&mut bytes, 0xa108, NAVY);
    pointer(&mut bytes, 0xa118, FOREIGN_NAVY);
    for navy in [NAVY, FOREIGN_NAVY] {
        pointer(&mut bytes, navy, cstrategic_navy::VTABLE_RVA);
        pointer(
            &mut bytes,
            navy + cstrategic_navy::SECONDARY,
            cstrategic_navy::SECONDARY_VTABLE_RVA,
        );
        pointer(&mut bytes, navy + native::MANAGER, MANAGER);
    }
    header(&mut bytes, NAVY + native::FOREIGN_NAVY_SLOTS, 0xc000, 3);
    for (ordinal, slot) in [3, 0, 3].into_iter().enumerate() {
        word(&mut bytes, 0xc000 + ordinal * 4, slot);
    }
    header(&mut bytes, NAVY + native::SUPPLY_HUBS, 0xc100, 3);
    for (ordinal, id) in [3, 0, 3].into_iter().enumerate() {
        word(&mut bytes, 0xc100 + ordinal * 4, id);
    }
    header(&mut bytes, NAVY + native::NEAREST_BASES, 0xc200, 4);
    header(&mut bytes, FOREIGN_NAVY + native::NEAREST_BASES, 0xc300, 4);
    pointer(&mut bytes, 0xc208, 0xd000);
    pointer(&mut bytes, 0xc218, 0xd100);
    pointer(&mut bytes, 0xc300, 0xd100);
    for (base, id) in [(0xd000, 0), (0xd100, 3)] {
        pointer(&mut bytes, base, cnaval_base::VTABLE_RVA);
        word(&mut bytes, base + cnaval_base::PROVINCE_ID, id);
    }
    header(&mut bytes, STATE + cgame_state::PROVINCES, 0x3200, 4);
    for (id, province, area) in [(1, 0xe000, 0x19000), (2, 0xe200, 0), (3, 0xe400, 0x19100)] {
        pointer(&mut bytes, 0x3200 + id * 8, province);
        pointer(&mut bytes, province, cprovince::VTABLE_RVA);
        word(&mut bytes, province + cprovince::ID, id as u32);
        pointer(&mut bytes, province + cprovince::CONTROLLER_AREA, area);
    }
    header(&mut bytes, OWNER + ccountry::THEATRES, 0xf000, 1);
    header(&mut bytes, ZERO + ccountry::THEATRES, 0xf100, 1);
    for (country_list, theatre, owner, area_list, area) in [
        (0xf000, 0x10000, 3, 0x13000, 0),
        (0xf100, 0x11000, 0, 0x13100, 0x19100),
    ] {
        pointer(&mut bytes, country_list, theatre);
        pointer(&mut bytes, theatre, ctheatre::VTABLE_RVA);
        pointer(
            &mut bytes,
            theatre + ctheatre::HANDLE,
            (1usize << 32) | ctheatre::REFERENCE_TYPE as usize,
        );
        word(&mut bytes, theatre + ctheatre::OWNER, owner);
        header(
            &mut bytes,
            theatre + ctheatre::CONTROLLER_AREA_DATA,
            area_list,
            1,
        );
        pointer(&mut bytes, area_list, area);
    }
    TestMemory {
        bytes,
        fault: None,
        calls: RefCell::new(Vec::new()),
        reserved: None,
    }
}
fn capacity() -> (Capacities, SharedCapacities) {
    (
        Capacities {
            owners: 4,
            owner_slots: 4,
            navies: 4,
            physical_slots: 8,
            foreign: 3,
            hubs: 3,
            provinces: 3,
            area_edges: 0,
            theatre_headers: 1,
            theatre_provinces: 3,
        },
        SharedCapacities {
            nearest_rows: 4,
            nearest_bases: 8,
            reserved_provinces: 3,
            launch_bases: 0,
        },
    )
}

#[test]
fn original_access_headers_grow_the_physical_controls_before_borrowing() {
    let mut memory = world();
    header(
        &mut memory.bytes,
        NAVY + cstrategic_navy::ACCESS_MODES,
        0x18000,
        5000,
    );
    header(
        &mut memory.bytes,
        FOREIGN_NAVY + cstrategic_navy::ACCESS_MODES,
        0x14000,
        7,
    );
    let (mut capacity, shared_capacity) = capacity();
    capacity.navies = 1;
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    let mut reader = captured.begin(&memory, 0, STATE, Some(7), &mut shared);
    assert_eq!(captured.physical_count(), Ok(4));
    assert_eq!(captured.known_access_modes(), 5007);
    assert!(captured.borrow_navy(1).is_ok());
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(address, _)| address == NAVY + cstrategic_navy::ACCESS_MODES)
            .count(),
        1
    );
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|&(address, _)| address == 0x18000)
    );
    reader.clear();
    captured.reset();
    assert_eq!(captured.known_access_modes(), 0);
}
fn owner(memory: &TestMemory) -> ccountry::Country {
    ccountry::Reader::new(memory, 0, STATE)
        .unwrap()
        .resolve(3)
        .unwrap()
}
fn capture(memory: &TestMemory) -> (Captured, Shared) {
    let (capacity, shared_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    let owner = owner(memory);
    captured.capture(
        memory,
        0,
        STATE,
        Some(-71),
        OwnerCensus {
            owners: [Ok(owner), Ok(owner)],
            failure: None,
        },
        &mut shared,
    );
    (captured, shared)
}

#[test]
fn physical_aliases_order_zero_and_actual_null_controller_area_survive_projection() {
    let memory = world();
    let (captured, shared) = capture(&memory);
    let (naval, theatre) = captured.for_country(1).unwrap();
    let naval = naval.unwrap();
    assert_eq!(naval.navy_slot, 1);
    assert_eq!(naval.foreign_navy_slots, Some(vec![3, 0, 3]));
    assert_eq!(naval.supply_hubs, Some(vec![3, 0, 3]));
    assert_eq!(
        theatre.unwrap(),
        TheatreControllerAreas {
            provinces: vec![2],
            null_controller_area: true
        }
    );
    let shared = shared.projection().unwrap().unwrap();
    assert_eq!(shared.sample_hour, -71);
    let caches = shared.nearest_caches.unwrap();
    assert_eq!(
        caches.iter().map(|row| row.slot).collect::<Vec<_>>(),
        vec![0, 1, 3]
    );
    assert!(!caches[0].present);
    assert_eq!(caches[0].nearest_bases, Some(Vec::new()));
    assert_eq!(
        caches[1].nearest_bases,
        Some(vec![None, Some(0), None, Some(3)])
    );
    assert_eq!(
        caches[2].nearest_bases,
        Some(vec![Some(3), None, None, None])
    );
    assert_eq!(shared.reserved_theatre.unwrap().provinces, vec![3]);
    assert!(captured.owners.first.iter().all(|row| row.source.is_none()));
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
    // Raw foreign slot3 is distinct from its current country-link alias1.
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == FOREIGN_NAVY)
            .count(),
        1
    );
}

#[test]
fn one_valid_cache_payload_failure_preserves_other_cache_and_reserved_theatre() {
    let mut memory = world();
    memory.fault = Some(0xc300);
    let (_, shared) = capture(&memory);
    let shared = shared.projection().unwrap().unwrap();
    let caches = shared.nearest_caches.unwrap();
    assert!(
        caches
            .iter()
            .find(|row| row.slot == 1)
            .unwrap()
            .nearest_bases
            .is_some()
    );
    assert!(
        caches
            .iter()
            .find(|row| row.slot == 3)
            .unwrap()
            .nearest_bases
            .is_none()
    );
    assert_eq!(shared.reserved_theatre.unwrap().provinces, vec![3]);
}

#[test]
fn demanded_physical_identity_failure_withholds_whole_cache_domain_only() {
    let mut memory = world();
    pointer(&mut memory.bytes, FOREIGN_NAVY, 0);
    let (captured, shared) = capture(&memory);
    let shared = shared.projection().unwrap().unwrap();
    assert!(shared.nearest_caches.is_none());
    assert_eq!(shared.reserved_theatre.unwrap().provinces, vec![3]);
    assert!(captured.for_country(1).unwrap().0.is_some());
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == FOREIGN_NAVY)
            .count(),
        1
    );
}

#[test]
fn complete_empty_moving_owner_domain_acquires_areas_without_theatre_reads() {
    let memory = world();
    let (capacity, shared_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(71),
        OwnerCensus {
            owners: std::iter::empty(),
            failure: None,
        },
        &mut shared,
    );
    let projection = shared.projection().unwrap().unwrap();
    assert_eq!(projection.nearest_caches, Some(Vec::new()));
    assert!(projection.reserved_theatre.is_none());
    assert!(!shared.reserved_demanded);
    let calls = memory.calls.borrow();
    // Current selected routes need the original province domain even when
    // the naval moving-owner census is empty. No theatre payload is demanded.
    assert!(captured.province_area(1).is_some());
    assert!(calls.iter().any(|&(at, _)| at == 0xe000));
    assert!(
        calls
            .iter()
            .all(|&(at, _)| !(0x10000..0x12000).contains(&at))
    );
    assert!(
        calls
            .iter()
            .all(|&(at, _)| at != OWNER + ccountry::THEATRES && at != ZERO + ccountry::THEATRES)
    );
}

#[test]
fn short_nearest_and_theatre_arrays_grow_from_the_same_capture_demand() {
    let memory = world();
    let (mut capacity, mut shared_capacity) = capacity();
    capacity.theatre_headers = 0;
    capacity.theatre_provinces = 0;
    shared_capacity.nearest_bases = 0;
    shared_capacity.reserved_provinces = 0;
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    let owner = owner(&memory);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(71),
        OwnerCensus {
            owners: [Ok(owner)],
            failure: None,
        },
        &mut shared,
    );
    assert_eq!(shared.required().nearest_bases, 8);
    assert_eq!(captured.required().theatre_headers, 1);
    assert_eq!(captured.required().theatre_provinces, 1);
    assert_eq!(shared.required().reserved_provinces, 1);
    captured.grow(&mut capacity);
    shared.grow(&mut shared_capacity);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(72),
        OwnerCensus {
            owners: [Ok(owner)],
            failure: None,
        },
        &mut shared,
    );
    assert_eq!(
        captured.for_country(1).unwrap().1.unwrap().provinces,
        vec![2]
    );
    let projection = shared.projection().unwrap().unwrap();
    assert!(
        projection
            .nearest_caches
            .unwrap()
            .iter()
            .all(|row| row.nearest_bases.is_some())
    );
    assert_eq!(projection.reserved_theatre.unwrap().provinces, vec![3]);
}

#[test]
fn absent_hour_and_reset_remove_the_packet_and_every_private_source() {
    let memory = world();
    let (mut captured, mut shared) = capture(&memory);
    captured.capture(
        &memory,
        0,
        STATE,
        None,
        OwnerCensus {
            owners: [Ok(owner(&memory))],
            failure: None,
        },
        &mut shared,
    );
    assert!(shared.projection().unwrap().is_none());
    assert_eq!(captured.for_country(1).unwrap(), (None, None));
    assert_eq!(
        captured.source_reason,
        Some("naval movement sample hour unavailable")
    );
    captured.reset();
    shared.reset();
    assert!(captured.owners.first.iter().all(|row| row.source.is_none()));
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
    assert!(shared.projection().unwrap().is_none());
}

#[test]
fn cached_reserved_and_authenticated_navy_survive_failed_positive_links() {
    let mut memory = world();
    let positive = owner(&memory);
    memory.reserved = Some(ccountry::read_reserved_root(&memory, 0, STATE));
    pointer(
        &mut memory.bytes,
        STATE + cgame_state::COUNTRY_LINKS,
        usize::MAX,
    );
    let (capacity, shared_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(71),
        OwnerCensus {
            owners: [Ok(positive)],
            failure: None,
        },
        &mut shared,
    );
    let (naval, theatre) = captured.for_country(1).unwrap();
    let naval = naval.unwrap();
    assert_eq!(naval.foreign_navy_slots, Some(vec![3, 0, 3]));
    assert_eq!(naval.supply_hubs, Some(vec![3, 0, 3]));
    assert!(theatre.is_none());
    assert_eq!(
        captured.owners.first[0].theatre_reason,
        Some("country links unreadable or invalid")
    );
    let projection = shared.projection().unwrap().unwrap();
    assert!(
        projection
            .nearest_caches
            .unwrap()
            .iter()
            .all(|row| row.nearest_bases.is_some())
    );
    assert_eq!(projection.reserved_theatre.unwrap().provinces, vec![3]);
    // The authenticating observation acquired physical slot0 once before the
    // independent positive failure; the naval reader consumes the cached fact.
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0x3000)
            .count(),
        1
    );
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == ZERO)
            .count(),
        1
    );
    assert_eq!(captured.owner_flags.required, 2);
}

#[test]
fn original_full_physical_holder_is_borrowed_before_movement_and_retired_once() {
    let memory = world();
    let (capacity, shared_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    let mut reader = captured.begin(&memory, 0, STATE, Some(-71), &mut shared);
    assert_eq!(captured.physical_count(), Ok(4));
    let original = captured.borrow_navy(3).unwrap();
    assert!(original.present);
    assert_eq!(original.ledger_navy(), Ok(FOREIGN_NAVY));
    let outer = original.take_ledger_regions().unwrap();
    assert_eq!(outer.count, 0);
    assert!(original.take_ledger_regions().is_err());
    captured.finish(
        &memory,
        0,
        STATE,
        OwnerCensus {
            owners: [Ok(owner(&memory))],
            failure: None,
        },
        &mut shared,
        &reader,
    );
    reader.clear();
    assert!(captured.borrow_navy(3).is_err());
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == FOREIGN_NAVY)
            .count(),
        1
    );
    assert_eq!(
        shared
            .projection()
            .unwrap()
            .unwrap()
            .nearest_caches
            .unwrap()
            .iter()
            .map(|row| row.slot)
            .collect::<Vec<_>>(),
        [0, 1, 3]
    );
}

#[test]
fn original_province_domain_refusal_survives_transport_borrow_and_finish() {
    let mut memory = world();
    memory.fault = Some(0xe000);
    let (capacity, shared_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    let mut reader = captured.begin(&memory, 0, STATE, Some(71), &mut shared);
    let reason = captured
        .province_reason
        .expect("original domain must refuse");
    let calls = memory
        .calls
        .borrow()
        .iter()
        .filter(|&&(at, _)| at == 0xe000)
        .count();
    assert!(calls > 0);
    // The independent physical navy is still loaned from this original pass.
    assert_eq!(
        captured.borrow_navy(3).unwrap().ledger_navy(),
        Ok(FOREIGN_NAVY)
    );
    captured.finish(
        &memory,
        0,
        STATE,
        OwnerCensus {
            owners: [Ok(owner(&memory))],
            failure: None,
        },
        &mut shared,
        &reader,
    );
    reader.clear();
    assert_eq!(captured.province_reason, Some(reason));
    assert_eq!(captured.owners.first[0].theatre_reason, Some(reason));
    assert!(captured.for_country(1).unwrap().1.is_none());
    assert!(captured.province_area(1).is_none());
    assert_eq!(
        memory
            .calls
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == 0xe000)
            .count(),
        calls
    );
    assert!(captured.navies.first.iter().all(|row| row.inputs.is_none()));
}

#[test]
fn idle_attachment_projects_current_area_identities_before_naval_owner_gates_and_grows_live_edges()
{
    let mut memory = world();
    for (area, first) in [(0x19000, 0xe000), (0x19100, 0xe400)] {
        pointer(
            &mut memory.bytes,
            area,
            engine::ccontroller_area::VTABLE_RVA,
        );
        pointer(&mut memory.bytes, area + 0x28, first);
        word(&mut memory.bytes, area + 0x3c, 1);
    }
    for (province, owner_area) in [(0xe000, 0x1d000), (0xe200, 0x1d100), (0xe400, 0x1d000)] {
        pointer(
            &mut memory.bytes,
            province + cprovince::OWNER_AREA,
            owner_area,
        );
    }
    // Two contacts to the same current D0 root are separate native entries.
    pointer(&mut memory.bytes, 0x19090, 0x19400);
    pointer(&mut memory.bytes, 0x19098, 0x19420);
    word(&mut memory.bytes, 0x190a0, 2);
    for (node, payload, back, next) in [
        (0x19400, 0x19500, 0, 0x19420),
        (0x19420, 0x19508, 0x19400, 0),
    ] {
        pointer(&mut memory.bytes, node, payload);
        pointer(&mut memory.bytes, node + 8, back);
        pointer(&mut memory.bytes, node + 0x10, next);
        pointer(&mut memory.bytes, payload, 0x19100);
    }
    let (mut capacity, shared_capacity) = capacity();
    let mut captured = Captured::new(capacity);
    let mut shared = Shared::new(shared_capacity);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(71),
        OwnerCensus {
            owners: [],
            failure: None,
        },
        &mut shared,
    );
    let one = captured.province_area(1).unwrap();
    let two = captured.province_area(2).unwrap();
    let three = captured.province_area(3).unwrap();
    assert_eq!(
        (one.controller, two.controller, three.controller),
        (1, 0, 3)
    );
    assert_eq!(
        (one.owner, two.owner, three.owner),
        (Some(1), Some(2), Some(1))
    );
    assert_eq!(one.sample_hour, 71);
    assert_eq!(one.node.unwrap().neighbours, Some(vec![3, 3]));
    assert_eq!(captured.required().area_edges, 2);
    assert!(
        captured
            .domain
            .first
            .iter()
            .all(|row| *row == native::ControllerAreaProvince::default())
    );
    assert!(
        captured
            .area_scratch
            .first
            .iter()
            .all(|row| *row == native::ControllerAreaProvince::default())
    );
    captured.grow(&mut capacity);
    captured.capture(
        &memory,
        0,
        STATE,
        Some(72),
        OwnerCensus {
            owners: [],
            failure: None,
        },
        &mut shared,
    );
    let node = captured.province_area(1).unwrap().node.unwrap();
    assert_eq!(node.first_land, Some(1));
    assert_eq!(node.neighbours, Some(vec![3, 3]));
    assert_eq!(node.nonsea_index, None); // no invented map operand
    assert_eq!(captured.province_area(2).unwrap().node, None);
    let packet = captured.province_area(1).unwrap();
    assert!(
        captured.province_area_matches(1, &replay::rows::Recorded::Value(Some(packet.clone())))
    );
    let mut changed = packet.clone();
    changed
        .node
        .as_mut()
        .unwrap()
        .neighbours
        .as_mut()
        .unwrap()
        .reverse();
    changed
        .node
        .as_mut()
        .unwrap()
        .neighbours
        .as_mut()
        .unwrap()
        .push(3);
    assert!(!captured.province_area_matches(1, &replay::rows::Recorded::Value(Some(changed))));
    let mut stale = packet;
    stale.sample_hour = 71;
    assert!(!captured.province_area_matches(1, &replay::rows::Recorded::Value(Some(stale))));
    assert!(!captured.province_area_matches(1, &replay::rows::Recorded::NotRecorded));
    captured.reset();
    assert!(captured.province_area(1).is_none());
}
