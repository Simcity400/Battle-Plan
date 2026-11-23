use super::*;
use engine_native_memory::memory::SliceMemory;

#[test]
fn held_daylight_direction_keeps_signed_words_and_refuses_short_spans() {
    let mut bytes = vec![0_u8; DAYLIGHT_DIRECTION + 24];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    let direction = [i64::MIN, -1, i64::MAX];
    for (index, value) in direction.into_iter().enumerate() {
        bytes[DAYLIGHT_DIRECTION + index * 8..DAYLIGHT_DIRECTION + index * 8 + 8]
            .copy_from_slice(&value.to_le_bytes());
    }
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(
        Reader::with_manager(&memory, 0, 0x1000)
            .unwrap()
            .daylight_direction(),
        Ok(direction)
    );
    let short = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..bytes.len() - 1],
    };
    assert!(
        Reader::with_manager(&short, 0, 0x1000)
            .unwrap()
            .daylight_direction()
            .is_err()
    );
}

#[test]
fn suspension_flags_keep_raw_bytes_and_refuse_short_reads() {
    let mut bytes = vec![0; SUSPEND_FLAGS + 2];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[SUSPEND_FLAGS..].copy_from_slice(&[3, 255]);
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let reader = Reader::with_manager(&memory, 0, 0x1000).unwrap();
    assert_eq!(reader.suspend_flags(), Ok([3, 255]));
    let short = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..SUSPEND_FLAGS + 1],
    };
    let reader = Reader::with_manager(&short, 0, 0x1000).unwrap();
    assert_eq!(
        reader.suspend_flags(),
        Err("weather suspension flags unreadable")
    );
}

#[test]
fn temperature_pass_words_keep_native_widths_and_refuse_short_reads() {
    let mut bytes = vec![0; SEED + 4];
    bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[UPDATE_CURSOR..UPDATE_CURSOR + 4].copy_from_slice(&13_413_i32.to_le_bytes());
    bytes[PROVINCES_PER_UPDATE..PROVINCES_PER_UPDATE + 4].copy_from_slice(&(-1_i32).to_le_bytes());
    bytes[SEED..SEED + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let reader = Reader::with_manager(&memory, 0, 0x1000).unwrap();
    assert_eq!(reader.temperature_words(), Ok((13_413, -1, u32::MAX)));
    let short = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..SEED + 3],
    };
    let reader = Reader::with_manager(&short, 0, 0x1000).unwrap();
    assert!(reader.temperature_words().is_err());
}

#[test]
fn custom_sea_reads_only_the_live_regions_ordered_active_words() {
    const IMAGE: usize = 0x1400_0000;
    const START: usize = 0x1000;
    const MANAGER: usize = 0x1100;
    const ROWS: usize = 0x1400;
    const VALUES: usize = 0x1a00;
    let mut bytes = vec![0_u8; 0x1000];
    let mut put = |address: usize, value: &[u8]| {
        bytes[address - START..address - START + value.len()].copy_from_slice(value);
    };
    put(MANAGER, &((IMAGE + VTABLE_RVA.0) as u64).to_le_bytes());
    put(MANAGER + REGIONS, &(ROWS as u64).to_le_bytes());
    put(MANAGER + REGIONS_COUNT, &3_i32.to_le_bytes());
    let selected = ROWS + 2 * sweather_per_region::STRIDE;
    put(
        selected,
        &((IMAGE + sweather_per_region::VTABLE_RVA.0) as u64).to_le_bytes(),
    );
    put(selected + sweather_per_region::ID, &2_i32.to_le_bytes());
    put(selected + 0x30, &(VALUES as u64).to_le_bytes());
    for (index, value) in [1_i16, 0, -3, 8].into_iter().enumerate() {
        put(VALUES + index * 2, &value.to_le_bytes());
    }
    let memory = SliceMemory {
        base: START,
        bytes: &bytes,
    };
    let reader = Reader::with_manager(&memory, IMAGE, MANAGER).unwrap();
    assert_eq!(reader.custom_sea(2, 4), Ok(vec![1, 0, -3, 8]));
    assert_eq!(reader.custom_sea(2, 0), Ok(vec![]));
    assert!(reader.custom_sea(3, 4).is_err());
}

struct ViewSource<'a> {
    memory: SliceMemory<'a>,
    enabled: bool,
    views: std::cell::RefCell<Vec<(usize, usize)>>,
    copies: std::cell::Cell<usize>,
}

impl<'a> ViewSource<'a> {
    fn new(base: usize, bytes: &'a [u8], enabled: bool) -> Self {
        Self {
            memory: SliceMemory { base, bytes },
            enabled,
            views: std::cell::RefCell::default(),
            copies: std::cell::Cell::new(0),
        }
    }
}

impl Memory for ViewSource<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.copies.set(self.copies.get() + 1);
        self.memory.copy(address, destination)
    }

    fn immutable_view(&self, address: usize, len: usize) -> Option<SliceMemory<'_>> {
        self.views.borrow_mut().push((address, len));
        if !self.enabled || address == 0 || len == 0 {
            return None;
        }
        address.checked_add(len)?;
        let start = address.checked_sub(self.memory.base)?;
        Some(SliceMemory {
            base: address,
            bytes: self.memory.bytes.get(start..start.checked_add(len)?)?,
        })
    }
}

#[test]
fn weather_immutable_view_keeps_batch_prefixes_and_declined_read_failures() {
    const STRIDE: usize = 32;
    const FIELDS: [RecordField; 2] = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 4,
        },
        RecordField {
            source_offset: 12,
            destination_offset: 4,
            len: 4,
        },
    ];
    let mut bytes = vec![0_u8; 65 * STRIDE];
    for index in 0..65 {
        bytes[index * STRIDE..index * STRIDE + 4].copy_from_slice(&(index as u32).to_le_bytes());
        bytes[index * STRIDE + 12..index * STRIDE + 16]
            .copy_from_slice(&(index as u32 ^ 0x9a71).to_le_bytes());
    }
    for count in [0, 1, 64, 65] {
        for capacity in [0, 1, 63, 64, 65, 66] {
            for readable in [0, 4, 15, 16, 64 * STRIDE + 4, 64 * STRIDE + 15, bytes.len()] {
                for bad_row in [None, Some(32)] {
                    let borrowed = ViewSource::new(0x1000, &bytes[..readable], true);
                    let ordinary = ViewSource::new(0x1000, &bytes[..readable], false);
                    let mut expected = None;
                    for source in [&ordinary, &borrowed] {
                        let mut output = vec![[u32::MAX; 2]; capacity];
                        let outcome = walk_dense_records(
                            source,
                            Array {
                                data: 0x1000,
                                count,
                            },
                            STRIDE,
                            &FIELDS,
                            8,
                            &mut output,
                            |index, record, copied| {
                                if copied == 0 {
                                    return Err("identity unreadable");
                                }
                                let first = u32::from_le_bytes(record[..4].try_into().unwrap());
                                if Some(index) == bad_row {
                                    return Err("identity mismatch");
                                }
                                if copied < 2 {
                                    return Err("second field unreadable");
                                }
                                Ok([first, u32::from_le_bytes(record[4..8].try_into().unwrap())])
                            },
                        );
                        if let Some(expected) = &expected {
                            assert_eq!(
                                &(outcome, output),
                                expected,
                                "{count}/{capacity}/{readable}/{bad_row:?}"
                            );
                        } else {
                            expected = Some((outcome, output));
                        }
                    }
                    let admitted = (count as usize).min(capacity);
                    if admitted == 0 {
                        assert!(borrowed.views.borrow().is_empty());
                        assert_eq!(borrowed.copies.get(), 0);
                    } else if readable >= (admitted - 1) * STRIDE + 16 {
                        assert_eq!(borrowed.copies.get(), 0);
                    } else {
                        assert_eq!(borrowed.copies.get(), ordinary.copies.get());
                    }
                }
            }
        }
    }
}

#[test]
fn weather_immutable_view_arithmetic_declines_without_changing_old_outcomes() {
    let storage = [7; 64];
    let cases: &[(Array, usize, RecordField, usize)] = &[
        (
            Array {
                data: usize::MAX - 16,
                count: 2,
            },
            32,
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            4,
        ),
        (
            Array {
                data: 0x1000,
                count: 2,
            },
            usize::MAX,
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            4,
        ),
        (
            Array {
                data: 0x1000,
                count: 1,
            },
            32,
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 4,
            },
            4,
        ),
        (
            Array {
                data: 0x1000,
                count: 1,
            },
            32,
            RecordField {
                source_offset: 31,
                destination_offset: 0,
                len: 4,
            },
            4,
        ),
        (
            Array {
                data: 0x1000,
                count: 1,
            },
            32,
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            0,
        ),
        (
            Array {
                data: 0x1000,
                count: 1,
            },
            32,
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            DENSE_RECORD_BYTES + 1,
        ),
    ];
    for &(array, stride, field, record_stride) in cases {
        let borrowed = ViewSource::new(array.data, &storage, true);
        let ordinary = ViewSource::new(array.data, &storage, false);
        let mut expected = None;
        for source in [&ordinary, &borrowed] {
            let mut output = [usize::MAX; 2];
            let outcome = walk_dense_records(
                source,
                array,
                stride,
                &[field],
                record_stride,
                &mut output,
                |_, _, copied| {
                    if copied == 0 {
                        Err("field unreadable")
                    } else {
                        Ok(copied)
                    }
                },
            );
            if let Some(expected) = &expected {
                assert_eq!(&(outcome, output), expected);
            } else {
                expected = Some((outcome, output));
            }
        }
        assert_eq!(borrowed.copies.get(), ordinary.copies.get());
    }
    let plain = SliceMemory {
        base: 0x1000,
        bytes: &storage,
    };
    assert!(plain.immutable_view(0x1001, 4).is_none());
}

struct Traced<'a> {
    inner: SliceMemory<'a>,
    reads: std::cell::RefCell<Vec<usize>>,
}
impl engine_native_memory::Memory for Traced<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        self.inner.copy(address, destination)
    }
}

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    bytes[address - 0x1000..address - 0x1000 + value.len()].copy_from_slice(value);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x5000];
    put(
        &mut bytes,
        0x1000 + cgame_state::WEATHER_MANAGER,
        &0x2000_u64.to_le_bytes(),
    );
    put(&mut bytes, 0x2000, &(VTABLE_RVA.0 as u64).to_le_bytes());
    put(
        &mut bytes,
        0x2000 + MAX_SNOW,
        &((1_i64 << 43) + 7).to_le_bytes(),
    );
    for (offset, address, count, capacity, stride, vtable) in [
        (
            PROVINCES,
            0x3000_u64,
            3_i32,
            5_i32,
            sweather_per_province::STRIDE,
            sweather_per_province::VTABLE_RVA.0,
        ),
        (
            REGIONS,
            0x5000_u64,
            2_i32,
            4_i32,
            sweather_per_region::STRIDE,
            sweather_per_region::VTABLE_RVA.0,
        ),
    ] {
        put(&mut bytes, 0x2000 + offset, &address.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 8, &capacity.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 12, &count.to_le_bytes());
        for index in 0..count {
            let row = address as usize + index as usize * stride;
            put(&mut bytes, row, &(vtable as u64).to_le_bytes());
            put(&mut bytes, row + 8, &index.to_le_bytes());
        }
    }
    for (offset, data, values) in [
        (DISPLAY_SNOW, 0x4f00_u64, [0, 128, 255]),
        (DISPLAY_MUD, 0x4f10, [255, 0, 0]),
    ] {
        put(&mut bytes, 0x2000 + offset, &data.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 8, &4_u32.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 12, &3_u32.to_le_bytes());
        put(&mut bytes, data as usize, &values);
    }
    put(
        &mut bytes,
        0x2000 + PROVINCE_CLOCKS,
        &0x4000_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2000 + PROVINCE_CLOCKS + 8,
        &5_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x2000 + PROVINCE_CLOCKS + 12,
        &3_u32.to_le_bytes(),
    );
    for (index, (hour, night)) in [(0_i32, 0_i64), (-37, i64::MIN), (1002, i64::MAX)]
        .into_iter()
        .enumerate()
    {
        let clock = 0x4000 + index * sweather_per_province::CLOCK_STRIDE;
        put(
            &mut bytes,
            clock + sweather_per_province::CLOCK_LOCAL_HOUR,
            &hour.to_le_bytes(),
        );
        put(
            &mut bytes,
            clock + sweather_per_province::CLOCK_NIGHT_FACTOR,
            &night.to_le_bytes(),
        );
    }
    put(&mut bytes, 0x2000 + SUSPEND_FLAGS, &[3, 255]);
    bytes
}

#[test]
fn movement_weather_uses_authored_region_id_and_native_absence() {
    let mut bytes = fixture();
    let province = 0x5900;
    let region = 0x5b00;
    put(
        &mut bytes,
        province + crate::cprovince::ID,
        &2_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        province + crate::cprovince::STRATEGIC_REGION,
        &(region as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        region,
        &(engine_native_parts::cstrategic_region::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        region + engine_native_parts::cstrategic_region::ID,
        &1_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        region + engine_native_parts::cstrategic_region::RUNTIME_SLOT,
        &99_i32.to_le_bytes(),
    );
    assert_eq!(
        movement_modifiers(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000,
            province
        ),
        Ok((0x3000 + 2 * 0x180 + 0x58, 0x5000 + 0x160 + 0x58))
    );
    for id in [-1_i32, 3] {
        put(
            &mut bytes,
            province + crate::cprovince::ID,
            &id.to_le_bytes(),
        );
        put(
            &mut bytes,
            region + engine_native_parts::cstrategic_region::ID,
            &2_i32.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(movement_modifiers(&memory, 0, 0x1000, province), Ok((0, 0)));
        if id == 3 {
            let traced = Traced {
                inner: memory,
                reads: Default::default(),
            };
            assert_eq!(movement_modifiers(&traced, 0, 0x1000, province), Ok((0, 0)));
            let reads = traced.reads.borrow();
            assert!(reads.contains(&(0x2000 + PROVINCES_COUNT)));
            assert!(!reads.contains(&(0x2000 + PROVINCES)), "{reads:x?}");
            assert!(!reads.contains(&(0x2000 + REGIONS)), "{reads:x?}");
        }
    }
    put(
        &mut bytes,
        province + crate::cprovince::STRATEGIC_REGION,
        &0_u64.to_le_bytes(),
    );
    assert_eq!(
        movement_modifiers(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes
            },
            0,
            0x1000,
            province
        ),
        Ok((0, 0))
    );
}

#[test]
fn movement_modifiers_refuse_a_missing_weather_manager() {
    let mut bytes = fixture();
    let province = 0x5900;
    put(
        &mut bytes,
        province + crate::cprovince::ID,
        &1_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x1000 + cgame_state::WEATHER_MANAGER,
        &0_u64.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert!(movement_modifiers(&memory, 0, 0x1000, province).is_err());
}

#[test]
fn movement_weather_does_not_convert_faults_or_identity_errors_to_absence() {
    let mut bytes = fixture();
    let province = 0x5900;
    put(
        &mut bytes,
        province + crate::cprovince::ID,
        &1_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        0x3180 + sweather_per_province::ID,
        &2_i32.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert!(movement_modifiers(&memory, 0, 0x1000, province).is_err());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..0x49a7],
    };
    assert!(movement_modifiers(&memory, 0, 0x1000, province).is_err());
}

#[test]
fn live_counts_not_capacity_control_dense_rows_and_exact_truncation() {
    let bytes = fixture();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let reader = Reader::new(&memory, 0, 0x1000).unwrap();
    let mut provinces = [sweather_per_province::Row::default(); 4];
    assert_eq!(reader.provinces(&mut provinces), Outcome::Rows { len: 3 });
    assert_eq!(
        provinces[..3]
            .iter()
            .map(|row| (row.id, row.display_snow, row.display_mud))
            .collect::<Vec<_>>(),
        vec![(0, 0, 255), (1, 128, 0), (2, 255, 0)]
    );
    assert_eq!(
        provinces[..3]
            .iter()
            .map(|row| (row.local_hour, row.night_factor))
            .collect::<Vec<_>>(),
        vec![(0, 0), (-37, i64::MIN), (1002, i64::MAX)]
    );
    assert_eq!(
        reader.provinces(&mut provinces[..1]),
        Outcome::Truncated {
            len: 1,
            unread_rows: 2
        }
    );
    let mut regions = [sweather_per_region::Row::default(); 4];
    assert_eq!(reader.regions(&mut regions), Outcome::Rows { len: 2 });
    assert_eq!(regions[1].max_snow, (1_i64 << 43) + 7);
    let short = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..0x1000 + MAX_SNOW + 7],
    };
    assert!(matches!(
        Reader::new(&short, 0, 0x1000)
            .unwrap()
            .regions(&mut regions),
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn invalid_province_or_display_header_does_not_hide_available_regions() {
    for (offset, value) in [
        (PROVINCES_COUNT, -1_i32),
        (DISPLAY_MUD + 12, 2),
        (PROVINCE_CLOCKS + 12, -1),
        (PROVINCE_CLOCKS + 12, 2),
    ] {
        let mut bytes = fixture();
        put(&mut bytes, 0x2000 + offset, &value.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let reader = Reader::new(&memory, 0, 0x1000).unwrap();
        assert!(matches!(
            reader.provinces(&mut [sweather_per_province::Row::default(); 4]),
            Outcome::Unavailable { .. }
        ));
        assert_eq!(
            reader.regions(&mut [sweather_per_region::Row::default(); 4]),
            Outcome::Rows { len: 2 }
        );
    }
}

#[test]
fn absent_or_wrong_manager_is_unavailable() {
    let mut bytes = fixture();
    put(&mut bytes, 0x2000, &0_u64.to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert!(matches!(
        Reader::new(&memory, 0, 0x1000),
        Err("weather manager vtable mismatch")
    ));
    put(
        &mut bytes,
        0x1000 + cgame_state::WEATHER_MANAGER,
        &0_u64.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert!(matches!(
        Reader::new(&memory, 0, 0x1000),
        Err("weather manager pointer unreadable or absent")
    ));
}

struct FirstWeatherFault<'a> {
    inner: SliceMemory<'a>,
    fault: std::cell::Cell<Option<usize>>,
    reads: std::cell::RefCell<Vec<usize>>,
}

impl Memory for FirstWeatherFault<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.reads.borrow_mut().push(address);
        if self.fault.get() == Some(address) {
            self.fault.set(None);
            return false;
        }
        self.inner.copy(address, destination)
    }
}

#[test]
fn movement_weather_original_count_failure_survives_other_region_selectors() {
    let bytes = fixture();
    let address = 0x2000 + REGIONS_COUNT;
    let memory = FirstWeatherFault {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        fault: std::cell::Cell::new(Some(address)),
        reads: Default::default(),
    };
    let reader = Reader::new(&memory, 0, 0x1000).unwrap();
    assert_eq!(
        reader.movement_record(0, true),
        Err("movement weather field unreadable")
    );
    assert_eq!(
        reader.movement_record(1, true),
        Err("movement weather field unreadable")
    );
    assert_eq!(reader.movement_record(-1, true), Ok(0));
    assert_eq!(reader.movement_record(0, false), Ok(0x3000));
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == address)
            .count(),
        1
    );
    assert_eq!(
        Reader::new(&memory, 0, 0x1000)
            .unwrap()
            .movement_record(1, true),
        Ok(0x5000 + sweather_per_region::STRIDE)
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == address)
            .count(),
        2
    );
}

#[test]
fn movement_weather_original_data_failure_survives_admitted_region_selectors() {
    let bytes = fixture();
    let address = 0x2000 + REGIONS;
    let memory = FirstWeatherFault {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        fault: std::cell::Cell::new(Some(address)),
        reads: Default::default(),
    };
    let reader = Reader::new(&memory, 0, 0x1000).unwrap();
    assert_eq!(
        reader.movement_record(0, true),
        Err("movement weather field unreadable")
    );
    assert_eq!(
        reader.movement_record(1, true),
        Err("movement weather field unreadable")
    );
    assert_eq!(reader.movement_record(2, true), Ok(0));
    let reads = memory.reads.borrow();
    assert_eq!(reads.iter().filter(|&&at| at == address).count(), 1);
    assert_eq!(
        reads
            .iter()
            .filter(|&&at| at == 0x2000 + REGIONS_COUNT)
            .count(),
        1
    );
    assert!(!reads.contains(&0x5000));
}

#[test]
fn movement_weather_region_selectors_borrow_original_successful_header_words() {
    let bytes = fixture();
    let memory = Traced {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        reads: Default::default(),
    };
    let reader = Reader::new(&memory, 0, 0x1000).unwrap();
    assert_eq!(reader.movement_record(0, true), Ok(0x5000));
    assert_eq!(
        reader.movement_record(1, true),
        Ok(0x5000 + sweather_per_region::STRIDE)
    );
    assert_eq!(reader.movement_record(2, true), Ok(0));
    let reads = memory.reads.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|&&at| at == 0x2000 + REGIONS_COUNT)
            .count(),
        1
    );
    assert_eq!(
        reads.iter().filter(|&&at| at == 0x2000 + REGIONS).count(),
        1
    );
}

#[test]
fn original_weather_clocks_keep_failure_until_the_reader_retires() {
    let bytes = fixture();
    let address = 0x2000 + PROVINCE_CLOCKS;
    let memory = FirstWeatherFault {
        inner: SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        },
        fault: std::cell::Cell::new(Some(address)),
        reads: Default::default(),
    };
    let reader = Reader::new(&memory, 0, 0x1000).unwrap();
    assert_eq!(
        reader.province_clocks(),
        Err("weather clock array unreadable or invalid")
    );
    assert_eq!(
        reader.province_clocks(),
        Err("weather clock array unreadable or invalid")
    );
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == address)
            .count(),
        1
    );
    let next = Reader::new(&memory, 0, 0x1000).unwrap();
    assert_eq!(next.province_clocks().unwrap().count, 3);
    assert_eq!(next.province_clocks().unwrap().data, 0x4000);
    assert_eq!(
        memory
            .reads
            .borrow()
            .iter()
            .filter(|&&at| at == address)
            .count(),
        2
    );
}
