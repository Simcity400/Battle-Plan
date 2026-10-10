use super::*;
use std::cell::RefCell;

struct Image {
    bytes: Vec<u8>,
    fault: Option<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
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
fn put(image: &mut Image, at: usize, value: &[u8]) {
    image.bytes[at..at + value.len()].copy_from_slice(value);
}
fn image() -> Image {
    Image {
        bytes: vec![0; 0x12000],
        fault: None,
        reads: RefCell::new(Vec::new()),
    }
}
fn header(image: &mut Image, at: usize, data: u64, count: i32) {
    put(image, at, &data.to_le_bytes());
    put(image, at + 8, &count.max(0).to_le_bytes());
    put(image, at + 12, &count.to_le_bytes());
}

#[test]
fn dynamic_country_renumbering_keeps_the_original_embedded_radar_inputs() {
    let mut memory = image();
    put(&mut memory, 0x2008, &0x3000_u64.to_le_bytes());
    put(&mut memory, 0x2104, &1_u32.to_le_bytes());
    put(&mut memory, 0x210c, &1_u32.to_le_bytes());
    put(
        &mut memory,
        0x3000,
        &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(&mut memory, 0x3000 + ccountry::ID, &3_u32.to_le_bytes());
    let radar = 0x3000 + ccountry::RADARS_POOL;
    put(
        &mut memory,
        radar,
        &(cradars_pool::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut memory,
        radar + 8,
        &(cradars_pool::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut memory,
        radar + cradars_pool::COUNTRY,
        &1_u32.to_le_bytes(),
    );
    header(&mut memory, radar + RADAR_COUNTS, 0x6000, 2);
    put(&mut memory, 0x6000, &17_i32.to_le_bytes());
    put(&mut memory, 0x6004, &29_i32.to_le_bytes());
    // The getter never reaches the retained constructor tag, even if unreadable.
    for fault in [None, Some(radar + cradars_pool::COUNTRY)] {
        memory.fault = fault;
        let countries = ccountry::Reader::from_arrays(
            &memory,
            0,
            0,
            [
                crate::cpdx_array::Array {
                    data: 0x2000,
                    count: 2,
                },
                crate::cpdx_array::Array {
                    data: 0x2100,
                    count: 4,
                },
            ],
        )
        .unwrap();
        let country = countries.at(1).unwrap().unwrap();
        let owner = Owner::capture_authenticated(&memory, 0, &countries, country);
        let original = owner.radar.unwrap();
        assert_eq!(original.signed_count, 2);
        let mut values = [0; 2];
        copy_radar(&memory, original, &mut values).unwrap();
        assert_eq!(values, [17, 29]);
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|&(at, _)| at != radar + cradars_pool::COUNTRY)
        );
    }
}

#[test]
fn signed_zero_and_negative_intel_counts_skip_unreached_array_fields() {
    for count in [0, -1, i32::MIN] {
        let mut memory = image();
        header(&mut memory, 0x1000, u64::MAX, count);
        memory.fault = Some(0x1000);
        let original = array(&memory, 0x1000, 0, country_intel_row::STRIDE, true).unwrap();
        assert_eq!(
            original,
            RawArray {
                signed_count: count,
                data: Ok(0)
            }
        );
        assert_eq!(*memory.reads.borrow(), [(0x100c, 4)]);
        copy_intel(&memory, original, &mut []).unwrap();
        assert_eq!(memory.reads.borrow().len(), 1);
    }
}

#[test]
fn negative_radar_extent_stays_distinct_from_native_zero() {
    let mut memory = image();
    header(&mut memory, 0x1000, 0x2000, -7);
    let original = array(&memory, 0x1000, 0, 4, false).unwrap();
    assert_eq!(original.signed_count, -7);
    assert!(original.data.is_err());
    assert!(copy_radar(&memory, original, &mut []).is_err());
    assert_eq!(*memory.reads.borrow(), [(0x100c, 4)]);
}

#[test]
fn positive_body_fault_retains_original_signed_count_and_raw_navy_lanes_cross_batches() {
    let mut memory = image();
    header(&mut memory, 0x1000, 0x2000, 131);
    for slot in 0..131 {
        let raw = match slot % 3 {
            0 => i64::MIN,
            1 => i64::MAX,
            _ => -(slot as i64),
        };
        put(
            &mut memory,
            0x2000 + slot * country_intel_row::STRIDE + country_intel_row::NAVY,
            &raw.to_le_bytes(),
        );
    }
    let original = array(&memory, 0x1000, 0, country_intel_row::STRIDE, true).unwrap();
    let mut values = vec![0; 131];
    copy_intel(&memory, original, &mut values).unwrap();
    assert_eq!(values[0], i64::MIN);
    assert_eq!(values[64], i64::MAX);
    assert_eq!(values[130], i64::MAX);
    memory.fault = Some(0x1000);
    let failed = array(&memory, 0x1000, 0, country_intel_row::STRIDE, true).unwrap();
    assert_eq!(failed.signed_count, 131);
    assert!(failed.data.is_err());
    assert!(copy_intel(&memory, failed, &mut values).is_err());
}

fn region_image() -> Image {
    let mut memory = image();
    put(
        &mut memory,
        0x1000 + crate::cgame_state::WEATHER_MANAGER,
        &0x2000_u64.to_le_bytes(),
    );
    put(
        &mut memory,
        0x2000,
        &(cweather_manager::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut memory,
        0x2000 + cweather_manager::REGIONS_COUNT,
        &0_i32.to_le_bytes(),
    );
    header(
        &mut memory,
        0x3000 + cstrategic_region::PROVINCES,
        0x4000,
        2,
    );
    put(
        &mut memory,
        0x3000 + cstrategic_region::RUNTIME_SLOT,
        &(-13_i32).to_le_bytes(),
    );
    for (slot, province, id) in [(0, 0x5000, 17_u32), (1, 0x6000, 29)] {
        put(
            &mut memory,
            0x4000 + slot * 8,
            &(province as u64).to_le_bytes(),
        );
        put(
            &mut memory,
            province,
            &(cprovince::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut memory, province + cprovince::ID, &id.to_le_bytes());
        put(
            &mut memory,
            province + cprovince::STRATEGIC_REGION,
            &0x3000_u64.to_le_bytes(),
        );
    }
    memory
}

#[test]
fn original_region_header_is_not_retried_and_weather_null_remains_independent() {
    let memory = region_image();
    let weather = cweather_manager::Reader::new(&memory, 0, 0x1000).unwrap();
    let original = RegionOriginal::capture_authenticated(&memory, 0x3000, 7);
    memory.reads.borrow_mut().clear();
    let row = capture_region(
        &memory,
        0,
        43_800_001,
        original,
        Ok(&weather),
        &mut WeatherMetadata::default(),
    );
    assert_eq!(row.runtime_slot, Ok(-13));
    assert_eq!(row.province_count, Ok(2));
    assert_eq!(row.sampled_province, Ok(29));
    assert_eq!(row.weather_detection, Ok(None));
    assert!(
        memory
            .reads
            .borrow()
            .iter()
            .all(|(at, _)| *at != 0x3000 + cstrategic_region::PROVINCES
                && *at != 0x3000 + cstrategic_region::PROVINCES_COUNT
                && *at != 0x3000 + cstrategic_region::RUNTIME_SLOT)
    );
    let failed = RegionOriginal {
        provinces: Err("original province header fault"),
        ..original
    };
    let row = capture_region(
        &memory,
        0,
        43_800_001,
        failed,
        Ok(&weather),
        &mut WeatherMetadata::default(),
    );
    assert!(row.sampled_province.is_err());
    assert_eq!(row.weather_detection, Ok(None));
    assert_eq!(row.runtime_slot, Ok(-13));
    let row = capture_region(
        &memory,
        0,
        43_800_001,
        original,
        Err("original weather root fault"),
        &mut WeatherMetadata::default(),
    );
    assert_eq!(row.sampled_province, Ok(29));
    assert!(row.weather_detection.is_err());
}

#[test]
fn sampled_province_uses_native_signed_hour_remainder_and_authentic_region_membership() {
    let mut memory = region_image();
    let original = RegionOriginal::capture_authenticated(&memory, 0x3000, 7);
    assert!(sampled_province(&memory, 0, 43_799_999, original).is_err());
    assert_eq!(sampled_province(&memory, 0, 43_800_000, original), Ok(17));
    assert_eq!(sampled_province(&memory, 0, 43_800_001, original), Ok(29));
    put(
        &mut memory,
        0x6000 + cprovince::STRATEGIC_REGION,
        &0xdead_u64.to_le_bytes(),
    );
    assert!(sampled_province(&memory, 0, 43_800_001, original).is_err());
}

#[test]
fn genuine_reserved_count_fault_is_borrowed_without_a_second_count_attempt() {
    let mut memory = image();
    let country = ccountry::Country {
        pointer: 0x3000,
        canonical_slot: 0,
        id: 0,
    };
    let pool = country.pointer + ccountry::RADARS_POOL;
    put(
        &mut memory,
        pool,
        &(cradars_pool::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut memory,
        pool + 8,
        &(cradars_pool::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut memory,
        country.pointer + ccountry::INTEL_TABLE,
        &0x5000_u64.to_le_bytes(),
    );
    put(
        &mut memory,
        0x5000,
        &(ccountry_intel::VTABLE_RVA as u64).to_le_bytes(),
    );
    let countries = ccountry::Reader::from_arrays(
        &memory,
        0,
        0x1000,
        [
            crate::cpdx_array::Array {
                data: 0x2000,
                count: 2,
            },
            crate::cpdx_array::Array {
                data: 0x2100,
                count: 2,
            },
        ],
    )
    .unwrap();
    let observed = Owner::capture_with_owned_states(
        &memory,
        0,
        &countries,
        country,
        Err("original signed count fault"),
    );
    assert_eq!(observed.owned_states, Err("original signed count fault"));
    assert!(observed.radar.is_ok());
    assert!(observed.intel.is_ok());
    assert!(
        memory
            .reads
            .borrow()
            .iter()
            .all(|(at, _)| *at != country.pointer + ccountry::OWNED_STATE_COUNT)
    );
}

#[test]
fn already_attempted_weather_selector_is_borrowed_without_root_or_record_authentication() {
    let memory = region_image();
    let original = RegionOriginal::capture_authenticated(&memory, 0x3000, 7);
    for selected in [Err("original weather selector fault"), Ok(0), Ok(0xdead)] {
        memory.reads.borrow_mut().clear();
        let (row, received) = capture_region_with_record(
            &memory,
            0,
            43_800_001,
            original,
            selected,
            |sampled, id, record| {
                assert_eq!(sampled, Ok(29));
                assert_eq!(id, 7);
                // This observer exercises the selector loan, without attempting
                // any leaf of the borrowed native record.
                (record.map(|_| None), record)
            },
        );
        assert_eq!(received, selected);
        assert_eq!(row.sampled_province, Ok(29));
        assert_eq!(row.weather_detection, selected.map(|_| None));
        assert!(
            memory
                .reads
                .borrow()
                .iter()
                .all(|(at, _)| *at >= 0x4000 && *at != 0xdead)
        );
    }
}
