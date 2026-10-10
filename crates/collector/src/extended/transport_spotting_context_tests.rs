use super::*;
use std::cell::RefCell;
struct Image {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}
impl Memory for Image {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((at, output.len()));
        if self
            .fault
            .is_some_and(|fault| (at..at.saturating_add(output.len())).contains(&fault))
        {
            return false;
        }
        let Some(source) = at
            .checked_add(output.len())
            .and_then(|end| self.bytes.get(at..end))
        else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
}
fn put(memory: &mut Image, at: usize, value: &[u8]) {
    memory.bytes[at..at + value.len()].copy_from_slice(value);
}
fn header(memory: &mut Image, at: usize, data: u64, count: u32) {
    put(memory, at, &data.to_le_bytes());
    put(memory, at + 8, &count.to_le_bytes());
    put(memory, at + 12, &count.to_le_bytes());
}
fn image() -> Image {
    let mut memory = Image {
        bytes: vec![0; 0x12000],
        reads: RefCell::new(Vec::new()),
        fault: None,
    };
    header(
        &mut memory,
        0x1000 + engine::cgame_state::COUNTRIES,
        0x2000,
        4,
    );
    header(
        &mut memory,
        0x1000 + engine::cgame_state::COUNTRY_LINKS,
        0x2100,
        4,
    );
    put(&mut memory, 0x2008, &0x4000_u64.to_le_bytes());
    put(&mut memory, 0x2018, &0x4000_u64.to_le_bytes());
    put(&mut memory, 0x2104, &1_u32.to_le_bytes());
    put(&mut memory, 0x210c, &1_u32.to_le_bytes());
    put(
        &mut memory,
        0x4000,
        &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(&mut memory, 0x4000 + ccountry::ID, &1_u32.to_le_bytes());
    put(
        &mut memory,
        0x4000 + ccountry::OWNED_STATE_COUNT,
        &(-3_i32).to_le_bytes(),
    );
    let pool = 0x4000 + ccountry::RADARS_POOL;
    put(
        &mut memory,
        pool,
        &(engine::cradars_pool::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut memory,
        pool + 8,
        &(engine::cradars_pool::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut memory,
        pool + engine::cradars_pool::COUNTRY,
        &1_u32.to_le_bytes(),
    );
    header(&mut memory, pool + native::RADAR_COUNTS, 0x6000, 2);
    put(&mut memory, 0x6000, &(-7_i32).to_le_bytes());
    put(&mut memory, 0x6004, &23_i32.to_le_bytes());
    put(
        &mut memory,
        0x4000 + ccountry::INTEL_TABLE,
        &0x7000_u64.to_le_bytes(),
    );
    put(
        &mut memory,
        0x7000,
        &(engine::ccountry_intel::VTABLE_RVA as u64).to_le_bytes(),
    );
    header(
        &mut memory,
        0x7000 + engine::ccountry_intel::DATA,
        0x8000,
        2,
    );
    put(
        &mut memory,
        0x8000 + engine::country_intel_row::NAVY,
        &i64::MIN.to_le_bytes(),
    );
    put(
        &mut memory,
        0x8020 + engine::country_intel_row::NAVY,
        &i64::MAX.to_le_bytes(),
    );
    memory
}
fn source_country() -> ccountry::Country {
    ccountry::Country {
        pointer: 0x4000,
        canonical_slot: 1,
        id: 1,
    }
}
fn capacity() -> Capacities {
    Capacities {
        owners: 4,
        regions: 0,
        radar_values: 2,
        intel_values: 2,
    }
}
fn pass(memory: &Image, captured: &mut Captured, hour: i32) {
    let countries = ccountry::Reader::new(memory, 0, 0x1000).unwrap();
    captured.begin(hour, Ok(4));
    captured.country(memory, 0, &countries, 0, Ok(None));
    captured.country(memory, 0, &countries, 1, Ok(Some(source_country())));
    captured.country(memory, 0, &countries, 2, Ok(None));
    captured.country(memory, 0, &countries, 3, Ok(Some(source_country())));
    captured.finish();
}

#[test]
fn physical_nulls_aliases_and_raw_signed_arrays_survive_one_original_pass() {
    let memory = image();
    let mut captured = Captured::new(capacity());
    pass(&memory, &mut captured, 71);
    let packet = captured.projection().unwrap().unwrap();
    assert!(packet.owners_complete);
    assert!(!packet.regions_complete);
    assert_eq!(packet.owners.len(), 1);
    assert_eq!(packet.owners[0].target_owned_states, Some(-3));
    assert_eq!(
        packet.owners[0].radar.as_ref().unwrap().counts,
        Some(vec![-7, 23])
    );
    assert_eq!(
        packet.owners[0].navy_intel.as_ref().unwrap().lanes,
        Some(vec![i64::MIN, i64::MAX])
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x6000)
            .count(),
        1
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x8000)
            .count(),
        1
    );
    assert!(captured.roots.iter().all(Option::is_none));
    assert!(captured.reserved_root.is_none());
}

#[test]
fn failed_original_body_is_not_retried_through_alias_and_sibling_vector_survives() {
    let mut memory = image();
    memory.fault = Some(0x6000);
    let mut captured = Captured::new(capacity());
    pass(&memory, &mut captured, 72);
    let packet = captured.projection().unwrap().unwrap();
    let owner = &packet.owners[0];
    assert_eq!(owner.radar.as_ref().unwrap().signed_count, 2);
    assert!(owner.radar.as_ref().unwrap().counts.is_none());
    assert!(owner.navy_intel.as_ref().unwrap().lanes.is_some());
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x6000)
            .count(),
        1
    );
    captured.reset();
    assert_eq!(captured.projection().unwrap(), None);
    memory.fault = None;
    put(&mut memory, 0x6000, &101_i32.to_le_bytes());
    pass(&memory, &mut captured, 73);
    assert_eq!(
        captured.projection().unwrap().unwrap().owners[0]
            .radar
            .as_ref()
            .unwrap()
            .counts,
        Some(vec![101, 23])
    );
}

#[test]
fn acquired_native_headers_grow_the_first_complete_owner_and_arrays() {
    let memory = image();
    let mut captured = Captured::new(Capacities {
        radar_values: 0,
        intel_values: 0,
        owners: 0,
        regions: 0,
    });
    pass(&memory, &mut captured, 74);
    let original = captured.projection().unwrap().unwrap();
    assert_eq!(captured.radar.required, 2);
    assert_eq!(captured.intel.required, 2);
    assert!(original.owners_complete);
    assert_eq!(
        original.owners[0].radar.as_ref().unwrap().counts,
        Some(vec![-7, 23])
    );
    assert!(
        original.owners[0]
            .navy_intel
            .as_ref()
            .unwrap()
            .lanes
            .is_some()
    );
    captured.reset();
    captured.grow();
    pass(&memory, &mut captured, 75);
    let recovered = captured.projection().unwrap().unwrap();
    assert!(recovered.owners[0].radar.as_ref().unwrap().counts.is_some());
    assert!(
        recovered.owners[0]
            .navy_intel
            .as_ref()
            .unwrap()
            .lanes
            .is_some()
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x6000)
            .count(),
        2
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|(at, _)| *at == 0x8000)
            .count(),
        2
    );
}

#[test]
fn provisional_country_domain_only_counts_an_unhealed_original_failure() {
    let memory = image();
    let mut captured = Captured::new(capacity());
    let before = OWNER_REFUSALS.with(std::cell::Cell::get);
    captured.begin(74, Err("transport original country pass not authenticated"));
    assert_eq!(OWNER_REFUSALS.with(std::cell::Cell::get), before);
    pass(&memory, &mut captured, 74);
    assert!(captured.projection().unwrap().unwrap().owners_complete);
    assert_eq!(OWNER_REFUSALS.with(std::cell::Cell::get), before);
    captured.begin(75, Err("original country domain unreadable"));
    captured.finish();
    assert!(!captured.projection().unwrap().unwrap().owners_complete);
    assert_eq!(OWNER_REFUSALS.with(std::cell::Cell::get), before + 1);
}

#[test]
fn omitted_physical_slot_and_out_of_order_attempt_never_certify_owner_domain() {
    let memory = image();
    let countries = ccountry::Reader::new(&memory, 0, 0x1000).unwrap();
    let mut captured = Captured::new(capacity());
    captured.begin(76, Ok(4));
    captured.country(&memory, 0, &countries, 0, Ok(None));
    captured.country(&memory, 0, &countries, 2, Ok(Some(source_country())));
    captured.finish();
    assert!(!captured.projection().unwrap().unwrap().owners_complete);
    assert!(captured.roots.iter().all(Option::is_none));
}

#[test]
fn independent_original_jobs_merge_numerically_and_reject_cross_hour_association() {
    let owners = TransportSpottingContext {
        sample_hour: 77,
        owners_complete: true,
        owners: vec![TransportSpottingOwner {
            owner_slot: 1,
            ..Default::default()
        }],
        ..Default::default()
    };
    let regions = TransportSpottingContext {
        sample_hour: 77,
        regions_complete: true,
        regions: vec![TransportSpottingRegion {
            region_id: 8,
            weather_detection: Some(None),
            ..Default::default()
        }],
        ..Default::default()
    };
    let joined = merge(Some(owners.clone()), Some(regions.clone())).unwrap();
    assert!(joined.owners_complete && joined.regions_complete);
    assert_eq!(joined.owners, owners.owners);
    assert_eq!(joined.regions, regions.regions);
    assert_eq!(merge(Some(owners.clone()), None), Some(owners.clone()));
    assert_eq!(merge(None, Some(regions.clone())), Some(regions.clone()));
    assert!(
        merge(
            Some(owners),
            Some(TransportSpottingContext {
                sample_hour: 78,
                ..regions
            })
        )
        .is_none()
    );
}

fn region_packet(hour: i32) -> Captured {
    let mut captured = Captured::new(Capacities {
        regions: 2,
        ..Default::default()
    });
    captured.begin(hour, Ok(0));
    captured.begin_regions(Ok(3));
    captured.region_attempts = 3;
    captured.regions.len = 2;
    captured.regions.first[0] = TransportSpottingRegion {
        region_id: 1,
        sampled_province: Some(51),
        weather_detection: Some(None),
        ..Default::default()
    };
    captured.regions.first[1] = TransportSpottingRegion {
        region_id: 2,
        sampled_province: None,
        weather_detection: None,
        ..Default::default()
    };
    captured.finish();
    captured
}

#[test]
fn numerical_region_loan_retains_original_failures_and_returns_the_same_allocation() {
    let mut captured = region_packet(79);
    let pointer = captured.regions.first.as_ptr();
    let capacity = captured.regions.first.capacity();
    let loan = captured.take_regions(79);
    assert_eq!(loan.samples(79).unwrap()[1].sampled_province, None);
    assert!(loan.samples(80).is_err());
    assert!(captured.projection().unwrap().is_none());
    assert!(captured.sampled_regions(79).is_err());
    let duplicate = captured.take_regions(79);
    assert!(duplicate.samples(79).is_err());
    assert!(captured.restore_regions(duplicate).is_err());
    captured.restore_regions(loan).unwrap();
    assert_eq!(captured.regions.first.as_ptr(), pointer);
    assert_eq!(captured.regions.first.capacity(), capacity);
    assert_eq!(captured.sampled_regions(79).unwrap().len(), 2);
    assert!(captured.projection().unwrap().unwrap().regions_complete);
}

#[test]
fn same_hour_reset_cannot_reuse_an_old_region_loan_or_publish_its_previous_values() {
    let mut captured = region_packet(81);
    let loan = captured.take_regions(81);
    captured.reset();
    captured.begin(81, Ok(0));
    captured.begin_regions(Ok(0));
    captured.finish();
    assert!(captured.restore_regions(loan).is_err());
    let packet = captured.projection().unwrap().unwrap();
    assert!(!packet.regions_complete);
    assert!(packet.regions.is_empty());
}

#[test]
fn equal_sized_repeated_hourly_passes_keep_demand_stable_while_terminal_pressure_survives() {
    let memory = image();
    let mut captured = Captured::new(capacity());
    pass(&memory, &mut captured, 82);
    assert_eq!((captured.radar.required, captured.intel.required), (2, 2));
    pass(&memory, &mut captured, 83);
    assert_eq!((captured.radar.required, captured.intel.required), (2, 2));
    captured.fail();
    captured.reset();
    assert_eq!((captured.radar.required, captured.intel.required), (2, 2));
    pass(&memory, &mut captured, 84);
    assert_eq!((captured.radar.required, captured.intel.required), (2, 2));
}

#[test]
fn absent_merge_half_cannot_claim_its_placeholder_empty_domain_was_native() {
    let owners = TransportSpottingContext {
        sample_hour: 85,
        owners_complete: true,
        regions_complete: true,
        ..Default::default()
    };
    let regions = owners.clone();
    let owners_only = merge(Some(owners), None).unwrap();
    assert!(owners_only.owners_complete);
    assert!(!owners_only.regions_complete);
    let regions_only = merge(None, Some(regions)).unwrap();
    assert!(regions_only.regions_complete);
    assert!(!regions_only.owners_complete);
}

#[test]
fn original_weather_record_fault_and_native_null_join_without_any_second_selector() {
    let memory = image();
    let mut captured = Captured::new(Capacities {
        regions: 2,
        ..Default::default()
    });
    captured.begin(43_800_000, Ok(0));
    captured.begin_regions(Ok(2));
    let mut received = Vec::new();
    for (slot, record) in [Err("original weather record fault"), Ok(0)]
        .into_iter()
        .enumerate()
    {
        let original = native::RegionOriginal {
            pointer: 0x3000,
            id: slot as u32,
            runtime_slot: Ok(slot as i32),
            provinces: Err("original province header fault"),
        };
        captured.region_with_observed_weather(
            &memory,
            0,
            slot as u32,
            Ok(Some(original)),
            record,
            &mut |sampled, region, record| {
                assert!(sampled.is_err());
                received.push((region, record));
                (record.map(|_| None), None)
            },
        );
    }
    captured.finish();
    let packet = captured.projection().unwrap().unwrap();
    assert!(packet.regions_complete);
    assert_eq!(packet.regions.len(), 2);
    assert_eq!(packet.regions[0].weather_detection, None);
    assert_eq!(packet.regions[1].weather_detection, Some(None));
    assert!(
        packet
            .regions
            .iter()
            .all(|row| row.sampled_province.is_none())
    );
    assert_eq!(
        received,
        [(0, Err("original weather record fault")), (1, Ok(0))]
    );
    assert!(memory.reads.borrow().is_empty());
}
