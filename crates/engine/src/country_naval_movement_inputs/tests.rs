use super::*;
use std::cell::{Cell, RefCell};

const STATE: usize = 0x1000;
const COUNTRY_ZERO: usize = 0x4000;
const COUNTRY_ONE: usize = 0x4500;
const MANAGER_ROOT: usize = 0x6000;
const NAVY_ONE: usize = 0x7000;
const NAVY_ALIAS_SLOT: usize = 0x7400;
const NEAREST: usize = 0x8000;
const FOREIGN: usize = 0x8200;
const HUBS: usize = 0x8300;
const BASE_ZERO: usize = 0x9000;
const BASE_FOUR: usize = 0x9100;
const THEATRE_ONE: usize = 0xb100;
const THEATRE_TWO: usize = 0xb300;
const THEATRE_ZERO: usize = 0xb900;

struct TestMemory {
    bytes: Vec<u8>,
    reserved_root: Option<Result<ccountry::Country, &'static str>>,
    fail_once: Option<usize>,
    failed: Cell<bool>,
    calls: RefCell<Vec<(usize, usize)>>,
}

impl Memory for TestMemory {
    fn authenticated_reserved_country(
        &self,
        _base: usize,
        _game_state: usize,
    ) -> Option<Result<ccountry::Country, &'static str>> {
        self.reserved_root
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.borrow_mut().push((address, destination.len()));
        if self.fail_once.is_some_and(|fault| {
            address <= fault && fault < address.saturating_add(destination.len())
        }) && !self.failed.replace(true)
        {
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

fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    u64_at(bytes, at, data as u64);
    u32_at(bytes, at + 8, count);
    u32_at(bytes, at + 12, count);
}

fn list(bytes: &mut [u8], at: usize, values: &[u32]) {
    for (index, &value) in values.iter().enumerate() {
        u32_at(bytes, at + index * 4, value);
    }
}

fn pointers(bytes: &mut [u8], at: usize, values: &[usize]) {
    for (index, &value) in values.iter().enumerate() {
        u64_at(bytes, at + index * 8, value as u64);
    }
}

fn world() -> TestMemory {
    let mut bytes = vec![0; 0x10000];
    header(&mut bytes, STATE + cgame_state::COUNTRIES, 0x3000, 4);
    header(&mut bytes, STATE + cgame_state::COUNTRY_LINKS, 0x3100, 4);
    pointers(
        &mut bytes,
        0x3000,
        &[COUNTRY_ZERO, COUNTRY_ONE, 0x4800, COUNTRY_ONE],
    );
    list(&mut bytes, 0x3100, &[0, 1, 2, 1]);
    for (country, id) in [(COUNTRY_ZERO, 0), (COUNTRY_ONE, 1), (0x4800, 2)] {
        u64_at(&mut bytes, country, ccountry::VTABLE_RVA as u64);
        u32_at(&mut bytes, country + ccountry::ID, id);
    }
    u64_at(
        &mut bytes,
        STATE + cgame_state::STRATEGIC_NAVY_MANAGER,
        MANAGER_ROOT as u64,
    );
    u64_at(
        &mut bytes,
        MANAGER_ROOT,
        cstrategic_navy_manager::VTABLE_RVA as u64,
    );
    header(
        &mut bytes,
        MANAGER_ROOT + cstrategic_navy_manager::NAVIES,
        0x6100,
        4,
    );
    pointers(&mut bytes, 0x6100, &[0, NAVY_ONE, 0x7200, NAVY_ALIAS_SLOT]);
    for navy in [NAVY_ONE, 0x7200, NAVY_ALIAS_SLOT] {
        u64_at(&mut bytes, navy, cstrategic_navy::VTABLE_RVA as u64);
        u64_at(
            &mut bytes,
            navy + cstrategic_navy::SECONDARY,
            cstrategic_navy::SECONDARY_VTABLE_RVA as u64,
        );
        u64_at(&mut bytes, navy + MANAGER, MANAGER_ROOT as u64);
        // Foreign cache authentication deliberately does not read this word.
        u32_at(&mut bytes, navy + cstrategic_navy::OWNER, u32::MAX);
    }
    header(&mut bytes, NAVY_ONE + NEAREST_BASES, NEAREST, 5);
    pointers(
        &mut bytes,
        NEAREST,
        &[0, BASE_ZERO, BASE_FOUR, BASE_ZERO, 0],
    );
    header(&mut bytes, NAVY_ONE + FOREIGN_NAVY_SLOTS, FOREIGN, 3);
    list(&mut bytes, FOREIGN, &[3, 0, 3]);
    header(&mut bytes, NAVY_ONE + SUPPLY_HUBS, HUBS, 3);
    list(&mut bytes, HUBS, &[4, 0, 4]);
    header(&mut bytes, NAVY_ALIAS_SLOT + NEAREST_BASES, 0x8400, 5);
    pointers(&mut bytes, 0x8400, &[BASE_FOUR, 0, 0, 0, 0]);
    for (naval_base, id) in [(BASE_ZERO, 0), (BASE_FOUR, 4)] {
        u64_at(&mut bytes, naval_base, cnaval_base::VTABLE_RVA as u64);
        u32_at(&mut bytes, naval_base + cnaval_base::PROVINCE_ID, id);
        // Neither current map nor this deliberately invalid holder is read.
        u64_at(&mut bytes, naval_base + cnaval_base::HOLDER, u64::MAX);
    }
    header(&mut bytes, STATE + cgame_state::PROVINCES, 0x3200, 5);
    pointers(&mut bytes, 0x3200, &[0, 0xa000, 0xa200, 0xa400, 0xa600]);
    for (id, province, area) in [
        (1, 0xa000, 0xd100),
        (2, 0xa200, 0xd200),
        (3, 0xa400, 0),
        (4, 0xa600, 0xd300),
    ] {
        u64_at(&mut bytes, province, cprovince::VTABLE_RVA as u64);
        u32_at(&mut bytes, province + cprovince::ID, id);
        u64_at(&mut bytes, province + cprovince::CONTROLLER_AREA, area);
    }
    header(&mut bytes, COUNTRY_ONE + ccountry::THEATRES, 0xb000, 2);
    pointers(&mut bytes, 0xb000, &[THEATRE_ONE, THEATRE_TWO]);
    header(&mut bytes, COUNTRY_ZERO + ccountry::THEATRES, 0xb800, 1);
    pointers(&mut bytes, 0xb800, &[THEATRE_ZERO]);
    for (id, theatre, owner, area_at, areas) in [
        (1, THEATRE_ONE, 3, 0xb600, &[0xd100, 0][..]),
        (2, THEATRE_TWO, 1, 0xb700, &[0xd300][..]),
        (3, THEATRE_ZERO, 0, 0xbc00, &[0xd200][..]),
    ] {
        u64_at(&mut bytes, theatre, ctheatre::VTABLE_RVA as u64);
        u64_at(
            &mut bytes,
            theatre + ctheatre::HANDLE,
            (id << 32) | ctheatre::REFERENCE_TYPE as u64,
        );
        u32_at(&mut bytes, theatre + ctheatre::OWNER, owner);
        header(
            &mut bytes,
            theatre + ctheatre::CONTROLLER_AREA_DATA,
            area_at,
            areas.len() as u32,
        );
        pointers(&mut bytes, area_at, areas);
        // Both regular theatres have volunteer key zero; neither is skipped.
    }
    TestMemory {
        bytes,
        reserved_root: None,
        fail_once: None,
        failed: Cell::new(false),
        calls: RefCell::new(Vec::new()),
    }
}

fn countries(memory: &TestMemory) -> ccountry::Reader<'_, TestMemory> {
    ccountry::Reader::new(memory, 0, STATE).unwrap()
}

fn domain(reader: &Reader<'_, TestMemory>) -> Vec<ControllerAreaProvince> {
    let mut domain = vec![ControllerAreaProvince::default(); 4];
    let result = reader.capture_controller_area_domain(&mut domain);
    assert_eq!(result.len, Ok(4));
    domain
}

#[test]
fn reserved_launch_null_control_is_observed_once_and_cannot_be_reused() {
    use ccountry::launch_sources::NavalProvider;

    let memory = world();
    let reader = Reader::new(&memory, 0, STATE);
    let mut inputs = reader.observe_navy(0).unwrap();
    memory.calls.borrow_mut().clear();
    let captured = inputs.capture_launch(&memory, 0, &mut [], &mut []);
    assert_eq!(captured.provider, Ok(NavalProvider::NativeNull));
    assert_eq!(captured.required, 0);
    assert_eq!(
        inputs.capture_launch(&memory, 0, &mut [], &mut []).provider,
        Err("reserved launch navy control consumed")
    );
    assert!(memory.calls.borrow().is_empty());
}

#[test]
fn selected_area_sizing_counts_unique_original_roots_without_contact_payloads() {
    let mut memory = world();
    u64_at(
        &mut memory.bytes,
        0xa200 + cprovince::CONTROLLER_AREA,
        0xd100,
    );
    for (area, count) in [(0xd100, 13), (0xd300, 7)] {
        u64_at(
            &mut memory.bytes,
            area,
            crate::ccontroller_area::VTABLE_RVA as u64,
        );
        u32_at(&mut memory.bytes, area + 0xa0, count);
    }
    let reader = Reader::new(&memory, 0, STATE);
    let roots = domain(&reader);
    memory.calls.borrow_mut().clear();
    assert_eq!(reader.selected_area_edge_demand(&roots), Ok(20));
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|&(at, _)| at == 0xd100 + 0x90)
    );
    let mut unsorted = roots.clone();
    unsorted.reverse();
    assert_eq!(
        reader.selected_area_edge_demand(&unsorted),
        Err("selected area sizing domain order invalid")
    );
    drop(reader);
    u32_at(&mut memory.bytes, 0xd100 + 0xa0, u32::MAX);
    assert_eq!(
        Reader::new(&memory, 0, STATE).selected_area_edge_demand(&roots),
        Err("selected area sizing contact count unreadable or negative")
    );
}

#[test]
fn physical_alias_slot_zero_and_stale_nearest_null_id_zero_are_exact() {
    let memory = world();
    let countries = countries(&memory);
    assert!(countries.at(3).unwrap().is_none());
    let reader = Reader::new(&memory, 0, STATE);
    let mut alias = reader.observe_navy(3).unwrap();
    assert!(alias.present);
    let mut nearest = [None; 5];
    assert_eq!(
        alias.capture_nearest(&memory, 0, 5, &mut nearest).len,
        Ok(5)
    );
    assert_eq!(nearest, [Some(4), None, None, None, None]);
    let mut own = reader.observe_navy(1).unwrap();
    assert_eq!(own.capture_nearest(&memory, 0, 5, &mut nearest).len, Ok(5));
    assert_eq!(nearest, [None, Some(0), Some(4), Some(0), None]);
    let mut zero = reader.observe_navy(0).unwrap();
    assert!(!zero.present);
    assert_eq!(zero.capture_nearest(&memory, 0, 5, &mut []).len, Ok(0));
    assert!(!memory.calls.borrow().iter().any(|&(at, len)| {
        [
            NAVY_ONE + cstrategic_navy::OWNER,
            NAVY_ALIAS_SLOT + cstrategic_navy::OWNER,
            BASE_ZERO + cnaval_base::HOLDER,
            BASE_FOUR + cnaval_base::HOLDER,
        ]
        .iter()
        .any(|&field| at <= field && field < at + len)
    }));
}

#[test]
fn complete_vector_capacity_demand_order_and_independent_lists_survive() {
    let memory = world();
    let reader = Reader::new(&memory, 0, STATE);
    let mut inputs = reader.observe_navy(1).unwrap();
    assert_eq!(inputs.demand().nearest_bases.required, 5);
    let result = inputs.capture_nearest(&memory, 0, 5, &mut [None; 2]);
    assert!(result.len.is_err());
    assert_eq!(result.required, 5);
    let mut list = [0; 3];
    assert_eq!(
        inputs.capture_foreign_navy_slots(&memory, &mut list).len,
        Ok(3)
    );
    assert_eq!(list, [3, 0, 3]);
    assert_eq!(inputs.capture_supply_hubs(&memory, &mut list).len, Ok(3));
    assert_eq!(list, [4, 0, 4]);
    let before = memory.calls.borrow().len();
    assert_eq!(
        inputs.capture_nearest(&memory, 0, 5, &mut [None; 5]).len,
        Err(CONSUMED)
    );
    assert_eq!(memory.calls.borrow().len(), before);
}

#[test]
fn failed_header_is_not_retried_and_other_header_demands_remain() {
    let mut memory = world();
    memory.fail_once = Some(NAVY_ONE + NEAREST_BASES);
    let reader = Reader::new(&memory, 0, STATE);
    let mut inputs = reader.observe_navy(1).unwrap();
    let demand = inputs.demand();
    assert!(demand.nearest_bases.first_failure.is_some());
    assert_eq!(demand.foreign_navy_slots.required, 3);
    assert_eq!(demand.supply_hubs.required, 3);
    let before = memory.calls.borrow().len();
    assert!(
        inputs
            .capture_nearest(&memory, 0, 5, &mut [None; 5])
            .len
            .is_err()
    );
    assert_eq!(memory.calls.borrow().len(), before);
    let mut foreign = [0; 3];
    assert_eq!(
        inputs.capture_foreign_navy_slots(&memory, &mut foreign).len,
        Ok(3)
    );
    assert_eq!(foreign, [3, 0, 3]);
}

#[test]
fn failed_nearest_payload_exposes_no_prefix_and_keeps_hubs() {
    let mut memory = world();
    memory.fail_once = Some(BASE_FOUR + cnaval_base::PROVINCE_ID);
    let reader = Reader::new(&memory, 0, STATE);
    let mut inputs = reader.observe_navy(1).unwrap();
    let mut nearest = [None; 5];
    let result = inputs.capture_nearest(&memory, 0, 5, &mut nearest);
    assert!(result.len.is_err());
    assert_eq!(result.required, 5);
    assert_eq!(nearest, [None; 5]);
    let mut hubs = [0; 3];
    assert_eq!(inputs.capture_supply_hubs(&memory, &mut hubs).len, Ok(3));
    assert_eq!(hubs, [4, 0, 4]);
}

#[test]
fn all_owned_theatres_use_actual_area_equality_including_null_and_alias_owner() {
    let memory = world();
    let reader = Reader::new(&memory, 0, STATE);
    let countries = countries(&memory);
    let domain = domain(&reader);
    let mut headers = [TheatreInput::default(), TheatreInput::default()];
    let observed = reader.capture_theatre_headers(
        &countries,
        Owner::Positive(countries.resolve(1).unwrap()),
        &mut headers,
    );
    assert_eq!(observed.len, Ok(2));
    assert_eq!(observed.areas.required, 3);
    let mut marks = [0; 4];
    let mut provinces = [0; 4];
    let result =
        reader.capture_theatre_membership(&mut headers, &domain, &mut marks, &mut provinces);
    assert_eq!(result.provinces.len, Ok(3));
    assert_eq!(&provinces[..3], &[1, 3, 4]);
    assert!(result.null_controller_area);
    assert_eq!(marks, [0; 4]);
    assert!(headers.iter().all(|input| input.areas.is_none()));
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|&(at, _)| [0xd100, 0xd200, 0xd300].contains(&at))
    );
}

#[test]
fn reserved_theatre_and_navy_roots_are_independent() {
    let mut memory = world();
    u64_at(&mut memory.bytes, MANAGER_ROOT, 0);
    let reader = Reader::new(&memory, 0, STATE);
    assert!(reader.observe_navy(0).is_err());
    let countries = countries(&memory);
    let domain = domain(&reader);
    let mut headers = [TheatreInput::default()];
    assert_eq!(
        reader
            .capture_theatre_headers(&countries, reader.reserved_owner().unwrap(), &mut headers)
            .len,
        Ok(1)
    );
    let mut provinces = [0; 4];
    let result =
        reader.capture_theatre_membership(&mut headers, &domain, &mut [0; 4], &mut provinces);
    assert_eq!(result.provinces.len, Ok(1));
    assert_eq!(provinces[0], 2);
    assert!(!result.null_controller_area);
}

#[test]
fn reserved_shared_result_is_sticky_and_independent_of_positive_roster() {
    let mut memory = world();
    memory.reserved_root = Some(Err("first reserved authentication failed"));
    let reader = Reader::new(&memory, 0, STATE);
    assert_eq!(
        reader.reserved_owner().unwrap_err(),
        "first reserved authentication failed"
    );
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|&(at, _)| { at == STATE + cgame_state::COUNTRIES || at == COUNTRY_ZERO })
    );
    drop(reader);

    memory.calls.borrow_mut().clear();
    memory.reserved_root = Some(Ok(ccountry::Country {
        pointer: COUNTRY_ZERO,
        canonical_slot: 0,
        id: 0,
    }));
    u32_at(
        &mut memory.bytes,
        STATE + cgame_state::COUNTRY_LINKS + 12,
        0,
    );
    assert!(ccountry::Reader::new(&memory, 0, STATE).is_err());
    memory.calls.borrow_mut().clear();
    let reader = Reader::new(&memory, 0, STATE);
    assert_eq!(reader.reserved_owner().unwrap().pointer(), Ok(COUNTRY_ZERO));
    assert!(
        !memory
            .calls
            .borrow()
            .iter()
            .any(|&(at, _)| { at == STATE + cgame_state::COUNTRIES || at == COUNTRY_ZERO })
    );
    let mut headers = [TheatreInput::default()];
    assert_eq!(
        reader
            .capture_theatre_headers_optional(None, reader.reserved_owner().unwrap(), &mut headers)
            .len,
        Ok(1)
    );
    assert_eq!(
        reader
            .capture_theatre_headers_optional(
                None,
                Owner::Positive(ccountry::Country {
                    pointer: COUNTRY_ONE,
                    canonical_slot: 1,
                    id: 1,
                }),
                &mut headers,
            )
            .len,
        Err("naval movement positive country root unavailable")
    );
}

#[test]
fn theatre_capacity_refusal_keeps_exact_header_and_known_area_demand() {
    let memory = world();
    let reader = Reader::new(&memory, 0, STATE);
    let countries = countries(&memory);
    let mut headers = [TheatreInput::default()];
    let observed = reader.capture_theatre_headers(
        &countries,
        Owner::Positive(countries.resolve(1).unwrap()),
        &mut headers,
    );
    assert!(observed.len.is_err());
    assert_eq!(observed.required_headers, 2);
    assert_eq!(observed.areas.required, 3);
    assert!(headers[0].areas.is_none());
    let mut complete = [TheatreInput::default(), TheatreInput::default()];
    assert_eq!(
        reader
            .capture_theatre_headers(
                &countries,
                Owner::Positive(countries.resolve(1).unwrap()),
                &mut complete
            )
            .len,
        Ok(2)
    );
    let result = reader.capture_theatre_membership(
        &mut complete,
        &domain(&reader),
        &mut [0; 4],
        &mut [0; 2],
    );
    assert!(result.provinces.len.is_err());
    assert_eq!(result.provinces.required, 3);
    assert!(!result.null_controller_area);
}

#[test]
fn live_header_floors_keep_other_physical_slots_and_country_domains() {
    let mut memory = world();
    memory.fail_once = Some(NAVY_ONE + NEAREST_BASES);
    let reader = Reader::new(&memory, 0, STATE);
    let countries = countries(&memory);
    let mut slots: [DemandSlot; 4] = std::array::from_fn(|_| DemandSlot::default());
    let observed = reader.live_demand(&countries, [countries.resolve(1)], &mut slots, &mut [0; 3]);
    assert_eq!(observed.theatre_header_scratch_required, 2);
    let demand = observed.live;
    assert_eq!(demand.bounds.navy_slots, Ok(4));
    assert_eq!(demand.bounds.provinces, Ok(4));
    assert_eq!(demand.navy.nearest_bases.required, 5);
    assert!(demand.navy.nearest_bases.first_failure.is_some());
    assert_eq!(demand.navy.foreign_navy_slots.required, 3);
    assert_eq!(demand.navy.supply_hubs.required, 3);
    assert_eq!(demand.theatres.required, 3);
    assert_eq!(demand.theatre_areas.required, 4);
}

#[test]
fn capacity_first_aggregate_does_not_hide_a_later_native_header_refusal() {
    let mut memory = world();
    u32_at(&mut memory.bytes, 0x7200 + NEAREST_BASES + 12, 1);
    let reader = Reader::new(&memory, 0, STATE);
    let countries = countries(&memory);
    let mut slots: [DemandSlot; 4] = std::array::from_fn(|_| DemandSlot::default());
    let observed = reader.live_demand(
        &countries,
        [countries.resolve(1), countries.resolve(2)],
        &mut slots,
        &mut [],
    );
    assert_eq!(observed.foreign_scratch_required, 3);
    assert_eq!(
        observed.live.navy.nearest_bases.first_failure,
        Some("naval movement foreign-slot arena exhausted")
    );
    assert_eq!(
        observed.first_native_failure,
        Some("naval movement array unreadable or invalid")
    );
    assert_eq!(observed.physical_slots.required, 2);
    assert_eq!(observed.live.navy.nearest_bases.required, 5);
}

#[test]
fn empty_output_reports_exact_membership_count_only_after_source_success() {
    let memory = world();
    let reader = Reader::new(&memory, 0, STATE);
    let countries = countries(&memory);
    let mut headers = [TheatreInput::default(), TheatreInput::default()];
    assert_eq!(
        reader
            .capture_theatre_headers(
                &countries,
                Owner::Positive(countries.resolve(1).unwrap()),
                &mut headers
            )
            .len,
        Ok(2)
    );
    let counted =
        reader.capture_theatre_membership(&mut headers, &domain(&reader), &mut [0; 4], &mut []);
    assert_eq!(counted.provinces.len, Err(THEATRE_PROVINCE_CAPACITY));
    assert_eq!(counted.exact_province_count(), Ok(3));
    let failed = reader.capture_theatre_membership(&mut [], &domain(&reader), &mut [], &mut []);
    assert!(failed.exact_province_count().is_err());
}

#[test]
fn original_naval_headers_grow_before_first_and_later_mod_sized_publication() {
    let mut headers = Vec::new();
    let mut foreign = Vec::new();
    let mut hubs = Vec::new();
    let mut nearest = Vec::new();
    for count in [5000_usize, 12000] {
        let mut memory = world();
        let theatre_list = 0x10000;
        let foreign_list = theatre_list + count * 8;
        let hubs_list = foreign_list + count * 4;
        let nearest_list = hubs_list + count * 4;
        memory.bytes.resize(nearest_list + count * 8, 0);
        header(
            &mut memory.bytes,
            COUNTRY_ONE + ccountry::THEATRES,
            theatre_list,
            count as u32,
        );
        header(
            &mut memory.bytes,
            NAVY_ONE + FOREIGN_NAVY_SLOTS,
            foreign_list,
            count as u32,
        );
        header(
            &mut memory.bytes,
            NAVY_ONE + SUPPLY_HUBS,
            hubs_list,
            count as u32,
        );
        header(
            &mut memory.bytes,
            NAVY_ONE + NEAREST_BASES,
            nearest_list,
            count as u32,
        );
        for index in 0..count {
            u64_at(
                &mut memory.bytes,
                theatre_list + index * 8,
                THEATRE_ONE as u64,
            );
            u32_at(&mut memory.bytes, foreign_list + index * 4, 3);
            u32_at(&mut memory.bytes, hubs_list + index * 4, 4);
            u64_at(
                &mut memory.bytes,
                nearest_list + index * 8,
                BASE_FOUR as u64,
            );
        }
        let reader = Reader::new(&memory, 0, STATE);
        let countries = countries(&memory);
        let owner = Owner::Positive(countries.resolve(1).unwrap());
        assert_eq!(
            reader
                .capture_theatre_headers(&countries, owner, &mut [])
                .len,
            Err("naval movement theatre header arena exhausted")
        );
        memory.calls.borrow_mut().clear();
        let result =
            reader.capture_theatre_headers_optional_growing(Some(&countries), owner, &mut headers);
        assert_eq!((result.len, result.required_headers), (Ok(count), count));
        assert!(headers[..count].iter().all(|h| h.reference() != 0));
        assert_eq!(
            memory
                .calls
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == COUNTRY_ONE + ccountry::THEATRES)
                .count(),
            1
        );
        let mut original = reader.observe_navy(1).unwrap();
        memory.calls.borrow_mut().clear();
        assert_eq!(
            original
                .capture_foreign_navy_slots_growing(&memory, &mut foreign)
                .len,
            Ok(count)
        );
        assert_eq!(
            original.capture_supply_hubs_growing(&memory, &mut hubs).len,
            Ok(count)
        );
        assert_eq!(
            original
                .capture_nearest_growing(&memory, 0, 5, &mut nearest)
                .len,
            Ok(count)
        );
        assert_eq!(foreign[..count], vec![3; count]);
        assert_eq!(hubs[..count], vec![4; count]);
        assert_eq!(nearest[..count], vec![Some(4); count]);
        // Consumed original header controls are never read again during growth.
        assert!(memory.calls.borrow().iter().all(|&(at, _)| {
            ![
                NAVY_ONE + FOREIGN_NAVY_SLOTS,
                NAVY_ONE + SUPPLY_HUBS,
                NAVY_ONE + NEAREST_BASES,
            ]
            .contains(&at)
        }));
    }
}
