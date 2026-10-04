use super::*;
use crate::memory::SliceMemory;

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

/// Records the address of every guarded read.
struct Traced<'a> {
    inner: SliceMemory<'a>,
    reads: std::cell::RefCell<Vec<usize>>,
}
impl crate::Memory for Traced<'_> {
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
    put(&mut bytes, 0x2000, &(VTABLE_RVA as u64).to_le_bytes());
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
            sweather_per_province::VTABLE_RVA,
        ),
        (
            REGIONS,
            0x5000_u64,
            2_i32,
            4_i32,
            sweather_per_region::STRIDE,
            sweather_per_region::VTABLE_RVA,
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
    // Three display bytes per array: snow at 0x4f00, mud at 0x4f10.
    for (offset, data, values) in [
        (DISPLAY_SNOW, 0x4f00_u64, [0, 128, 255]),
        (DISPLAY_MUD, 0x4f10, [255, 0, 0]),
    ] {
        put(&mut bytes, 0x2000 + offset, &data.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 8, &4_u32.to_le_bytes());
        put(&mut bytes, 0x2000 + offset + 12, &3_u32.to_le_bytes());
        put(&mut bytes, data as usize, &values);
    }
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
        &(crate::cstrategic_region::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        region + crate::cstrategic_region::ID,
        &1_i32.to_le_bytes(),
    );
    put(
        &mut bytes,
        region + crate::cstrategic_region::RUNTIME_SLOT,
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
            region + crate::cstrategic_region::ID,
            &2_i32.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(movement_modifiers(&memory, 0, 0x1000, province), Ok((0, 0)));
        assert_eq!(movement_snow(&memory, 0, 0x1000, province), Ok(false));
        if id == 3 {
            // Both ids are past their counts: neither data pointer is read.
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
fn movement_snow_keeps_extra_word_signed_ceiling_and_wrapping() {
    let mut bytes = fixture();
    let province = 0x5900;
    put(
        &mut bytes,
        province + crate::cprovince::ID,
        &1_i32.to_le_bytes(),
    );
    for (extra, snow, ceiling, expected) in [
        (1_i64, 0_i64, 5_i64, true),
        (1, 4, 0, false),
        (5, -5, 100, false),
        (i64::MAX, 1, 100, false),
        (i64::MIN, -1, 100, true),
    ] {
        put(&mut bytes, 0x3180 + 0x30, &extra.to_le_bytes());
        put(&mut bytes, 0x3180 + 0x40, &snow.to_le_bytes());
        put(&mut bytes, 0x2000 + MAX_SNOW, &ceiling.to_le_bytes());
        assert_eq!(
            movement_snow(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0,
                0x1000,
                province
            ),
            Ok(expected)
        );
    }
    put(
        &mut bytes,
        0x1000 + cgame_state::WEATHER_MANAGER,
        &0_u64.to_le_bytes(),
    );
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    assert_eq!(movement_snow(&memory, 0, 0x1000, 0), Ok(false));
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
    assert!(movement_snow(&memory, 0, 0x1000, province).is_err());
    assert!(movement_modifiers(&memory, 0, 0x1000, province).is_err());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes[..0x49a7],
    };
    assert!(movement_snow(&memory, 0, 0x1000, province).is_err());
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
    for (offset, value) in [(PROVINCES_COUNT, -1_i32), (DISPLAY_MUD + 12, 2)] {
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
