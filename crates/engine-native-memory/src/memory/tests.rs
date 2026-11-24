use std::cell::RefCell;

use super::*;

struct TracedMemory {
    calls: RefCell<Vec<usize>>,
    fail_at: usize,
}

impl Memory for TracedMemory {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.borrow_mut().push(address);
        if address == self.fail_at {
            destination[0] = 0x7e;
            false
        } else {
            destination.fill(address as u8);
            true
        }
    }
}

#[test]
fn record_source_envelope_never_rejects_fields_across_a_canonical_gap() {
    let fields = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 4,
        },
        RecordField {
            source_offset: 0xffff_8000_0000_0000 - 1,
            destination_offset: 4,
            len: 4,
        },
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 8,
            len: 0,
        },
    ];
    let (first, end) = RecordLayout::new(&fields, 8).unwrap().envelope.unwrap();
    assert!(!is_canonical_range(1 + first, end - first));
    assert_eq!(
        record_source_prefix(
            RecordSources::Pointers(&[1]),
            &RecordLayout::new(&fields, usize::MAX).unwrap()
        ),
        3
    );
    assert_eq!(
        record_source_prefix(
            RecordSources::Pointers(&[1]),
            &RecordLayout::new(&[fields[1], fields[0], fields[2]], usize::MAX).unwrap()
        ),
        3
    );
    assert_eq!(
        RecordLayout::new(&fields[..1], 8).unwrap().envelope,
        Some((0, 4))
    );
    assert_eq!(
        RecordLayout::new(&[fields[0], fields[2]], 8)
            .unwrap()
            .envelope,
        Some((0, 4))
    );
    assert_eq!(
        record_source_prefix(
            RecordSources::Pointers(&[0, usize::MAX]),
            &RecordLayout::new(&fields[2..], usize::MAX).unwrap()
        ),
        2
    );
    assert_eq!(
        record_source_prefix(
            RecordSources::Pointers(&[]),
            &RecordLayout::new(&fields, usize::MAX).unwrap()
        ),
        0
    );
}

#[test]
fn record_source_prefix_matches_ordered_scan_at_address_boundaries() {
    fn ordered(pointers: &[usize], fields: &[RecordField]) -> usize {
        let mut completed = 0;
        for &pointer in pointers {
            for field in fields {
                if field.len != 0 {
                    let valid = pointer != 0
                        && pointer
                            .checked_add(field.source_offset)
                            .is_some_and(|address| {
                                address.checked_add(field.len).is_some()
                                    && is_canonical_range(address, field.len)
                            });
                    if !valid {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }
    let addresses = [
        0,
        1,
        0x1000,
        0x0000_7fff_ffff_fff0,
        0x0000_8000_0000_0000,
        0xffff_7fff_ffff_ffff,
        0xffff_8000_0000_0000,
        usize::MAX - 3,
        usize::MAX,
    ];
    for first_offset in [0, 7, 0x0000_8000_0000_0000, usize::MAX - 1] {
        for second_offset in [0, 31, 0xffff_8000_0000_0000, usize::MAX] {
            for first_len in [0, 1, 8, 32] {
                for second_len in [0, 1, 8, 32] {
                    let fields = [
                        RecordField {
                            source_offset: first_offset,
                            destination_offset: 0,
                            len: first_len,
                        },
                        RecordField {
                            source_offset: usize::MAX,
                            destination_offset: 32,
                            len: 0,
                        },
                        RecordField {
                            source_offset: second_offset,
                            destination_offset: 32,
                            len: second_len,
                        },
                    ];
                    for pointer in addresses {
                        let pointers = [0x1000, pointer, 0x1000];
                        assert_eq!(
                            record_source_prefix(
                                RecordSources::Pointers(&pointers),
                                &RecordLayout::new(&fields, usize::MAX).unwrap()
                            ),
                            ordered(&pointers, &fields),
                            "pointers={pointers:?}, fields={fields:?}"
                        );
                        for stride in [0, 8, 0x0000_8000_0000_0000, usize::MAX] {
                            let sources = RecordSources::Strided {
                                base: pointer,
                                stride,
                                count: 3,
                            };
                            let pointers = std::array::from_fn::<_, 3, _>(|row| {
                                sources.pointer(row).unwrap_or(0)
                            });
                            assert_eq!(
                                record_source_prefix(
                                    sources,
                                    &RecordLayout::new(&fields, 64).unwrap()
                                ),
                                ordered(&pointers, &fields),
                                "base={pointer:x}, stride={stride:x}, fields={fields:?}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn strided_records_keep_empty_overflow_and_repeated_observations() {
    const FIELDS: [RecordField; 3] = [
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        },
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 4,
        },
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 4,
            len: 0,
        },
    ];
    const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 4).unwrap();
    let overflow = RecordSources::Strided {
        base: 0x1000,
        stride: usize::MAX - 0xfff,
        count: 2,
    };
    for fail_at in [0, 0x1000] {
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at,
        };
        let mut output = [0xa5; 8];
        let copied = memory.copy_records(overflow, &LAYOUT, &mut output);
        assert_eq!(copied, if fail_at == 0 { 4 } else { 1 });
        assert_eq!(*memory.calls.borrow(), [0x1000]);
        assert_eq!(&output[4..], &[0xa5; 4]);
        assert_eq!(
            LAYOUT.completed_bytes(copied),
            if fail_at == 0 { 4 } else { 0 }
        );
        assert_eq!(LAYOUT.completed_spans(copied), usize::from(fail_at == 0));
    }
    let memory = TracedMemory {
        calls: RefCell::new(Vec::new()),
        fail_at: 0,
    };
    assert_eq!(
        memory.copy_records(
            RecordSources::Strided {
                base: 0x1000,
                stride: 0,
                count: 2
            },
            &LAYOUT,
            &mut [0; 8]
        ),
        6
    );
    assert_eq!(*memory.calls.borrow(), [0x1000, 0x1000]);

    memory.calls.borrow_mut().clear();
    assert_eq!(
        memory.copy_records(
            RecordSources::Strided {
                base: 0x0000_8000_0000_0000,
                stride: 0,
                count: 1
            },
            &LAYOUT,
            &mut [0; 4]
        ),
        3
    );
    assert_eq!(*memory.calls.borrow(), [0x0000_8000_0000_0000]);

    let empty = [FIELDS[0]];
    let empty_layout = RecordLayout::new(&empty, 0).unwrap();
    memory.calls.borrow_mut().clear();
    assert_eq!(
        memory.copy_records(
            RecordSources::Strided {
                base: 0,
                stride: usize::MAX,
                count: usize::MAX
            },
            &empty_layout,
            &mut []
        ),
        usize::MAX
    );
    assert!(memory.calls.borrow().is_empty());
}

#[test]
fn records_copy_exact_fields_and_keep_failed_prefix_and_padding() {
    let fields = [
        RecordField {
            source_offset: 1,
            destination_offset: 0,
            len: 2,
        },
        RecordField {
            source_offset: 0x20,
            destination_offset: 4,
            len: 4,
        },
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 8,
            len: 0,
        },
        RecordField {
            source_offset: 0x38,
            destination_offset: 8,
            len: 8,
        },
    ];
    let memory = TracedMemory {
        calls: RefCell::new(Vec::new()),
        fail_at: 0x1120,
    };
    let mut bytes = [0xa5; 48];
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[0x1000, 0x1100]),
            &RecordLayout::new(&fields, 20).expect("valid record layout"),
            &mut bytes
        ),
        5
    );
    assert_eq!(
        *memory.calls.borrow(),
        [0x1001, 0x1020, 0x1038, 0x1101, 0x1120]
    );
    let mut expected = [0xa5; 48];
    expected[..2].fill(1);
    expected[4..8].fill(0x20);
    expected[8..16].fill(0x38);
    expected[20..22].fill(1);
    expected[24] = 0x7e;
    assert_eq!(bytes, expected);
}

#[test]
fn record_destination_bounds_fail_before_access_and_sources_fail_in_order() {
    let memory = TracedMemory {
        calls: RefCell::new(Vec::new()),
        fail_at: 0,
    };
    for (field, stride) in [
        (
            RecordField {
                source_offset: 0,
                destination_offset: 7,
                len: 2,
            },
            8,
        ),
        (
            RecordField {
                source_offset: 0,
                destination_offset: usize::MAX,
                len: 1,
            },
            8,
        ),
        (
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 1,
            },
            usize::MAX,
        ),
    ] {
        let mut bytes = [0xa5; 16];
        let fields = [field];
        let layout = RecordLayout::new(&fields, stride);
        assert_eq!(
            layout.as_ref().map_or(0, |layout| memory.copy_records(
                RecordSources::Pointers(&[0x1000, 0x1100]),
                layout,
                &mut bytes
            )),
            0
        );
        assert_eq!(bytes, [0xa5; 16]);
        #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
        {
            let source = [0x6d; 16];
            let live = unsafe { LiveMemory::new() };
            assert_eq!(
                layout.as_ref().map_or(0, |layout| live.copy_records(
                    RecordSources::Pointers(&[source.as_ptr() as usize; 2]),
                    layout,
                    &mut bytes
                )),
                0
            );
            assert_eq!(bytes, [0xa5; 16]);
        }
    }
    assert!(memory.calls.borrow().is_empty());
    let fields = [
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        },
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 2,
        },
        RecordField {
            source_offset: 8,
            destination_offset: 2,
            len: 2,
        },
    ];
    let mut bytes = [0xa5; 4];
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[0]),
            &RecordLayout::new(&fields, 4).expect("valid record layout"),
            &mut bytes
        ),
        1
    );
    assert!(memory.calls.borrow().is_empty());
    assert_eq!(bytes, [0xa5; 4]);
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[usize::MAX - 3]),
            &RecordLayout::new(&fields, 4).expect("valid record layout"),
            &mut bytes
        ),
        2
    );
    assert_eq!(*memory.calls.borrow(), [usize::MAX - 3]);
    assert_eq!(bytes, [0xfc, 0xfc, 0xa5, 0xa5]);
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[0, usize::MAX]),
            &RecordLayout::new(&fields[..1], 0).expect("valid record layout"),
            &mut []
        ),
        2
    );
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[]),
            &RecordLayout::new(&fields, 4).expect("valid record layout"),
            &mut []
        ),
        0
    );
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn native_records_preserve_exact_unaligned_bytes_and_fault_field_positions() {
    let fields = [
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        },
        RecordField {
            source_offset: 1,
            destination_offset: 1,
            len: 1,
        },
        RecordField {
            source_offset: 3,
            destination_offset: 3,
            len: 2,
        },
        RecordField {
            source_offset: 8,
            destination_offset: 5,
            len: 12,
        },
        RecordField {
            source_offset: 24,
            destination_offset: 17,
            len: 16,
        },
        RecordField {
            source_offset: 40,
            destination_offset: 33,
            len: 8,
        },
    ];
    let source: [u8; 64] = std::array::from_fn(|index| 255 - index as u8);
    let memory = unsafe { LiveMemory::new() };
    for bad_pointer in [
        0,
        usize::MAX,
        1,
        0x0072_6f74_6361_665f,
        0x0000_8000_0000_0000,
        0xffff_0000_0000_0000,
    ] {
        let mut bytes = [0xa5; 96];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[source.as_ptr() as usize, bad_pointer]),
                &RecordLayout::new(&fields, 48).expect("valid record layout"),
                &mut bytes
            ),
            7
        );
        for field in &fields[1..] {
            assert_eq!(
                bytes[field.destination_offset..field.destination_offset + field.len],
                source[field.source_offset..field.source_offset + field.len]
            );
        }
        for index in [0, 2, 41, 47, 48] {
            assert_eq!(bytes[index], 0xa5);
        }
        assert_eq!(bytes[50..], [0xa5; 96][50..]);
    }
    let mut bytes = [0xa5; 48];
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[source.as_ptr() as usize]),
            &RecordLayout::new(&fields, 48).expect("valid record layout"),
            &mut bytes
        ),
        fields.len()
    );
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[0, usize::MAX]),
            &RecordLayout::new(&fields[..1], 0).expect("valid record layout"),
            &mut []
        ),
        2
    );
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn native_records_copy_only_the_proven_prefix_including_partial_rows() {
    unsafe extern "C" {
        fn seh_guarded_copy_records(
            pointers: *const usize,
            source_base: usize,
            source_stride: usize,
            validated_field_count: usize,
            fields: *const RecordField,
            field_count: usize,
            row_stride: usize,
            destination: *mut u8,
        ) -> usize;
    }
    let source: [u8; 16] = std::array::from_fn(|index| index as u8);
    let pointers = [source.as_ptr() as usize; 2];
    let fields = [
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        },
        RecordField {
            source_offset: 1,
            destination_offset: 1,
            len: 2,
        },
        RecordField {
            source_offset: 4,
            destination_offset: 4,
            len: 4,
        },
    ];
    for prefix in 0..=6 {
        let mut bytes = [0xa5; 16];
        assert_eq!(
            unsafe {
                seh_guarded_copy_records(
                    pointers.as_ptr(),
                    0,
                    0,
                    prefix,
                    fields.as_ptr(),
                    fields.len(),
                    8,
                    bytes.as_mut_ptr(),
                )
            },
            prefix
        );
        let mut expected = [0xa5; 16];
        for index in 0..prefix {
            let field = fields[index % fields.len()];
            if field.len != 0 {
                let target = index / fields.len() * 8 + field.destination_offset;
                expected[target..target + field.len]
                    .copy_from_slice(&source[field.source_offset..field.source_offset + field.len]);
            }
        }
        assert_eq!(bytes, expected, "prefix={prefix}");
    }
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn native_records_unreadable_prefix_wins_over_later_preflight_failures() {
    let memory = unsafe { LiveMemory::new() };
    for bad_offset in [usize::MAX, 0x0000_8000_0000_0000 - 1] {
        let fields = [
            RecordField {
                source_offset: usize::MAX,
                destination_offset: 0,
                len: 0,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 8,
            },
            RecordField {
                source_offset: bad_offset,
                destination_offset: 8,
                len: 8,
            },
            RecordField {
                source_offset: 0,
                destination_offset: 16,
                len: 8,
            },
        ];
        assert_eq!(
            record_source_prefix(
                RecordSources::Pointers(&[1]),
                &RecordLayout::new(&fields, usize::MAX).unwrap()
            ),
            2
        );
        let mut bytes = [0xa5; 24];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[1]),
                &RecordLayout::new(&fields, 24).expect("valid record layout"),
                &mut bytes
            ),
            1
        );
        assert_eq!(bytes, [0xa5; 24]);
    }
}

#[test]
fn gathered_values_come_only_from_covered_bytes_of_complete_records() {
    const SPANS: [(usize, usize); 3] = [(0, 8), (0x10, 4), (0x20, 16)];
    const FIELDS: [RecordField; 3] = packed_fields(SPANS);
    const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, packed_stride(SPANS)).unwrap();
    const MAP: [u16; 0x30] = packed_map(SPANS);
    let object: Vec<u8> = (0..0x30).collect();
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &object,
    };
    let mut bytes = [0; 2 * 28];
    let mut complete = [false; 2];
    gather_records(&memory, &[0x1000, 0], &LAYOUT, &mut bytes, &mut complete);
    assert_eq!(complete, [true, false]);
    let record = Gathered::row(&LAYOUT, &MAP, &bytes, &complete, 0);
    let expect = |offset: usize| u32::from_le_bytes(object[offset..offset + 4].try_into().unwrap());
    for offset in [0, 4, 0x10, 0x20, 0x2c] {
        assert_eq!(
            record.get::<u32>(offset),
            Some(expect(offset)),
            "{offset:#x}"
        );
    }
    assert_eq!(record.get::<[u8; 16]>(0x20).unwrap(), object[0x20..0x30]);
    for offset in [5, 8, 0xe, 0x11, 0x1e, 0x2d, 0x30, usize::MAX] {
        assert_eq!(record.get::<u32>(offset), None, "{offset:#x}");
    }
    assert_eq!(record.get::<u64>(0x10), None);
    assert_eq!(
        Gathered::row(&LAYOUT, &MAP, &bytes, &complete, 1).get::<u32>(0),
        None
    );
    assert_eq!(Gathered::NONE.get::<u32>(0), None);
}

#[test]
fn unaligned_raw_values_preserve_all_bits_and_refuse_missing_bytes() {
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[0, 255, 255, 255, 255],
    };
    assert_eq!(read::<i32>(&memory, 0x1001), Some(-1));
    assert_eq!(read::<u32>(&memory, 0x1002), None);
    assert_eq!(read::<u64>(&memory, usize::MAX - 3), None);
    assert_eq!(read::<u32>(&memory, 0), None);
    assert_eq!(read::<[u8; 4]>(&memory, 0x1001), Some([255; 4]));
    assert_eq!(read::<[u8; 0]>(&memory, 0), Some([]));
    assert_eq!(scalar_from_bytes::<i32>(&[255; 4]), Some(-1));
    assert_eq!(scalar_from_bytes::<u32>(&[255; 3]), None);
}

#[test]
fn slice_hints_accept_every_address_and_length_without_changing_reads() {
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &[1, 2, 3, 4],
    };
    for (address, len) in [
        (0, 4),
        (0xfff, 2),
        (0x1000, 0),
        (0x1002, usize::MAX),
        (0x1004, 1),
        (usize::MAX, usize::MAX),
    ] {
        memory.prefetch(address, len);
    }
    assert_eq!(read::<[u8; 4]>(&memory, 0x1000), Some([1, 2, 3, 4]));
}

#[test]
fn scattered_fields_preserve_unaligned_full_width_values() {
    let mut bytes = [0; 20];
    bytes[1..3].copy_from_slice(&u16::MAX.to_le_bytes());
    bytes[5..9].copy_from_slice(&(u32::MAX - 1).to_le_bytes());
    bytes[11..19].copy_from_slice(&(i64::MIN + 0x1234).to_le_bytes());
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut narrow = [0; 2];
    let mut medium = [0; 4];
    let mut wide = [0; 8];
    assert_eq!(
        memory.copy_scattered(
            [0x1001, 0x1005, 0x100b],
            &mut [&mut narrow, &mut medium, &mut wide],
        ),
        3
    );
    assert_eq!(u16::from_le_bytes(narrow), u16::MAX);
    assert_eq!(u32::from_le_bytes(medium), u32::MAX - 1);
    assert_eq!(i64::from_le_bytes(wide), i64::MIN + 0x1234);
}

#[test]
fn scattered_failure_returns_only_complete_fields_and_stops_copying() {
    let memory = TracedMemory {
        calls: RefCell::new(Vec::new()),
        fail_at: 0x1002,
    };
    let mut first = [0; 2];
    let mut failed = [0; 2];
    let mut later = [0xa5; 2];
    assert_eq!(
        memory.copy_scattered(
            [0x1001, 0x1002, 0x1003],
            &mut [&mut first, &mut failed, &mut later],
        ),
        1
    );
    assert_eq!(*memory.calls.borrow(), [0x1001, 0x1002]);
    assert_eq!(first, [1; 2]);
    assert_eq!(failed, [0x7e, 0]);
    assert_eq!(later, [0xa5; 2]);
}

#[test]
fn prefetched_scattered_preserves_ordered_adapter_observations_and_failed_prefixes() {
    fn scatter(bytes: &mut [[u8; 8]; 6]) -> [&mut [u8]; 6] {
        let [first, empty, repeated, failed, invalid, later] = bytes;
        [
            first.as_mut_slice(),
            &mut empty[..0],
            repeated.as_mut_slice(),
            failed.as_mut_slice(),
            invalid.as_mut_slice(),
            later.as_mut_slice(),
        ]
    }
    for fail_at in [0, 0x1002] {
        for invalid in [0, usize::MAX, 0x1003] {
            let ordered = TracedMemory {
                calls: RefCell::new(Vec::new()),
                fail_at,
            };
            let prefetched = TracedMemory {
                calls: RefCell::new(Vec::new()),
                fail_at,
            };
            let addresses = [0x1001, 0, 0x1001, 0x1002, invalid, 0x1004];
            let mut expected = [[0xa5; 8]; 6];
            let mut actual = expected;
            let expected_count = ordered.copy_scattered(addresses, &mut scatter(&mut expected));
            let actual_count =
                prefetched.copy_scattered_prefetched(addresses, &mut scatter(&mut actual));
            assert_eq!(actual_count, expected_count);
            assert_eq!(actual, expected);
            assert_eq!(*prefetched.calls.borrow(), *ordered.calls.borrow());
        }
    }
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn native_prefetched_scattered_keeps_empty_repeated_and_invalid_prefixes() {
    let source: [u8; 16] = std::array::from_fn(|index| index as u8);
    let memory = unsafe { LiveMemory::new() };
    for invalid in [0, usize::MAX, 1, 0x0000_8000_0000_0000] {
        let mut empty = [];
        let mut first = [0xa5; 8];
        let mut repeated = [0xa5; 8];
        let mut failed = [0xa5; 8];
        let mut later = [0xa5; 8];
        assert_eq!(
            memory.copy_scattered_prefetched(
                [
                    0,
                    source.as_ptr() as usize + 1,
                    source.as_ptr() as usize + 1,
                    invalid,
                    source.as_ptr() as usize
                ],
                &mut [
                    &mut empty,
                    &mut first,
                    &mut repeated,
                    &mut failed,
                    &mut later
                ],
            ),
            3,
        );
        assert_eq!(first, source[1..9]);
        assert_eq!(repeated, first);
        assert_eq!(later, [0xa5; 8]);
    }
    let mut first_empty = [];
    let mut second_empty = [];
    assert_eq!(
        memory
            .copy_scattered_prefetched([0, usize::MAX], &mut [&mut first_empty, &mut second_empty]),
        2
    );
    assert_eq!(memory.copy_scattered_prefetched([], &mut []), 0);
}

#[test]
fn scattered_invalid_addresses_leave_the_valid_prefix_copied() {
    for invalid in [0, usize::MAX - 1] {
        let memory = TracedMemory {
            calls: RefCell::new(Vec::new()),
            fail_at: usize::MAX,
        };
        let mut first = [0; 2];
        let mut failed = [0xa5; 2];
        let mut later = [0xa5; 2];
        assert_eq!(
            memory.copy_scattered(
                [0x1001, invalid, 0x1003],
                &mut [&mut first, &mut failed, &mut later],
            ),
            1
        );
        assert_eq!(*memory.calls.borrow(), [0x1001]);
        assert_eq!(first, [1; 2]);
        assert_eq!(failed, [0xa5; 2]);
        assert_eq!(later, [0xa5; 2]);
    }
}

#[test]
fn scattered_empty_fields_succeed_without_copying_or_validating_addresses() {
    let memory = TracedMemory {
        calls: RefCell::new(Vec::new()),
        fail_at: usize::MAX,
    };
    let mut first_empty = [];
    let mut second_empty = [];
    let mut value = [0; 2];
    assert_eq!(
        memory.copy_scattered(
            [0, usize::MAX, 0x1001],
            &mut [&mut first_empty, &mut second_empty, &mut value],
        ),
        3
    );
    assert_eq!(*memory.calls.borrow(), [0x1001]);
    assert_eq!(value, [1; 2]);
    assert_eq!(memory.copy_scattered([], &mut []), 0);
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn native_scattered_copy_preserves_the_prefix_before_an_invalid_address() {
    let source = (i64::MIN + 0x1234).to_le_bytes();
    let memory = unsafe { LiveMemory::new() };
    let mut empty = [];
    let mut first = [0; 8];
    let mut failed = [0xa5; 4];
    let mut later = [0xa5; 8];
    assert_eq!(
        memory.copy_scattered(
            [0, source.as_ptr() as usize, 0, source.as_ptr() as usize],
            &mut [&mut empty, &mut first, &mut failed, &mut later],
        ),
        2
    );
    assert_eq!(first, source);
    assert_eq!(failed, [0xa5; 4]);
    assert_eq!(later, [0xa5; 8]);
    let mut second_empty = [];
    assert_eq!(
        memory.copy_scattered(
            [source.as_ptr() as usize, 0, usize::MAX],
            &mut [&mut first, &mut empty, &mut second_empty],
        ),
        3
    );
    assert_eq!(first, source);
    assert_eq!(
        memory.copy_scattered([0, usize::MAX], &mut [&mut empty, &mut second_empty]),
        2
    );
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn native_scattered_fault_preserves_completed_fields_and_leaves_later_fields_alone() {
    let source: [u8; 65] = std::array::from_fn(|index| index as u8);
    let source = &source[1..];
    let memory = unsafe { LiveMemory::new() };
    for width in [1, 2, 4, 8, 12, 16, 24, 26, 32, 48, 64] {
        let mut first = [0xa5; 64];
        let mut empty = [];
        let mut failed = [0xa5; 64];
        let mut later = [0xa5; 8];
        assert_eq!(
            memory.copy_scattered(
                [source.as_ptr() as usize, 0, 1, source.as_ptr() as usize],
                &mut [
                    &mut first[..width],
                    &mut empty,
                    &mut failed[..width],
                    &mut later,
                ],
            ),
            2
        );
        assert_eq!(first[..width], source[..width]);
        assert_eq!(first[width..], [0xa5; 64][width..]);
        assert_eq!(later, [0xa5; 8]);
    }
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn uninitialized_copies_expose_only_complete_copies() {
    use std::{ffi::c_void, mem::MaybeUninit};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    }
    struct Pages(*mut c_void);
    impl Drop for Pages {
        fn drop(&mut self) {
            unsafe { VirtualFree(self.0, 0, 0x8000) };
        }
    }
    let pages = Pages(unsafe { VirtualAlloc(std::ptr::null_mut(), 8192, 0x3000, 4) });
    assert!(!pages.0.is_null());
    let boundary = pages.0 as usize + 4096;
    let mut old = 0;
    assert_ne!(
        unsafe { VirtualProtect(boundary as *mut c_void, 4096, 1, &mut old) },
        0
    );
    let first = unsafe { std::slice::from_raw_parts_mut(pages.0.cast::<u8>(), 4096) };
    for (index, byte) in first.iter_mut().enumerate() {
        *byte = index as u8 ^ 0x5a;
    }
    let live = unsafe { LiveMemory::new() };

    struct Plain<'a>(&'a LiveMemory);
    impl Memory for Plain<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.0.copy(address, destination)
        }
    }
    let counted = CountedMemory::new(unsafe { LiveMemory::new() });
    let slice = SliceMemory {
        base: pages.0 as usize,
        bytes: &first[..],
    };
    fn check(memory: &impl Memory, boundary: usize) {
        for width in [1, 4, 8, 13, 64, 256, 512] {
            for (address, whole) in [
                (boundary - width, true),
                (boundary - width + 1, false),
                (0, false),
                (usize::MAX - 2, false),
            ] {
                let mut expected = vec![0xa5; width];
                let copied = memory.copy(address, &mut expected);
                assert_eq!(copied, whole, "{address:#x}+{width}");
                let mut scratch = [MaybeUninit::<u8>::uninit(); 512];
                match memory.copy_uninit(address, &mut scratch[..width]) {
                    Some(bytes) => {
                        assert!(whole);
                        assert_eq!(&bytes[..], &expected[..]);
                    }
                    None => assert!(!whole),
                }
                let mut scratch = [MaybeUninit::<u8>::uninit(); 512];
                let mut plain = vec![0; width];
                assert_eq!(
                    read_slice_uninit(memory, address, &mut scratch[..width])
                        .map(|bytes| bytes.to_vec()),
                    read_slice(memory, address, &mut plain).then_some(plain)
                );
            }
        }
        let mut empty: [MaybeUninit<u8>; 0] = [];
        assert_eq!(
            read_slice_uninit(memory, 0, &mut empty).map(|b| b.len()),
            Some(0)
        );
    }
    check(&live, boundary);
    check(&Plain(&live), boundary);
    check(&counted, boundary);
    check(&slice, boundary);
    ReadCounts::take_current();
    let mut scratch = [MaybeUninit::<u8>::uninit(); 16];
    assert!(counted.copy_uninit(boundary - 16, &mut scratch).is_some());
    let uninit = ReadCounts::take_current();
    assert!(counted.copy(boundary - 16, &mut [0; 16]));
    assert_eq!(uninit, ReadCounts::take_current());
    assert_eq!((uninit.copy_calls, uninit.spans, uninit.bytes), (1, 1, 16));
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn native_copy_widths_stop_at_exact_page_boundaries_and_keep_field_prefixes() {
    use std::ffi::c_void;
    unsafe extern "C" {
        fn seh_guarded_copy(dst: *mut u8, src: *const u8, len: usize) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
    }
    struct Pages(*mut c_void);
    impl Drop for Pages {
        fn drop(&mut self) {
            unsafe { VirtualFree(self.0, 0, 0x8000) };
        }
    }
    let pages = Pages(unsafe { VirtualAlloc(std::ptr::null_mut(), 8192, 0x3000, 4) });
    assert!(!pages.0.is_null());
    let boundary = pages.0 as usize + 4096;
    let mut old = 0;
    assert_ne!(
        unsafe { VirtualProtect(boundary as *mut c_void, 4096, 1, &mut old) },
        0
    );
    unsafe { std::ptr::write_bytes(pages.0.cast::<u8>(), 0x6d, 4096) };
    let memory = unsafe { LiveMemory::new() };
    let zero = [0_u8; 5];
    let all_bits = [255_u8; 5];
    assert_eq!(read::<u32>(&memory, zero[1..].as_ptr() as usize), Some(0));
    assert_eq!(
        read::<i32>(&memory, all_bits[1..].as_ptr() as usize),
        Some(-1)
    );
    for width in [1, 2, 4, 5, 8, 12, 13, 16, 24, 26, 32, 48, 64] {
        let valid = boundary - width;
        let crossing = valid + 1;
        let mut exact = [0xa5; 66];
        assert!(memory.copy(valid, &mut exact[1..1 + width]));
        assert_eq!(exact[1..1 + width], [0x6d; 64][..width]);
        assert!(memory.copy(valid - 1, &mut exact[1..1 + width]));
        assert_eq!(exact[1..1 + width], [0x6d; 64][..width]);
        assert_eq!(exact[0], 0xa5);
        assert!(exact[1 + width..].iter().all(|&byte| byte == 0xa5));
        assert!(!memory.copy(crossing, &mut exact[1..1 + width]));
        assert_eq!(exact[0], 0xa5);
        assert!(exact[1 + width..].iter().all(|&byte| byte == 0xa5));
        if matches!(width, 4 | 8) {
            for (address, expected) in [(valid, 1), (valid - 1, 1), (crossing, 0)] {
                exact.fill(0xa5);
                let result = unsafe {
                    seh_guarded_copy(exact[1..].as_mut_ptr(), address as *const u8, width)
                };
                assert_eq!(result, expected);
                if expected != 0 {
                    assert_eq!(exact[1..1 + width], [0x6d; 64][..width]);
                }
                assert_eq!(exact[0], 0xa5);
                assert!(exact[1 + width..].iter().all(|&byte| byte == 0xa5));
            }
        }
        if width == 4 {
            for address in [valid, valid - 1] {
                assert_eq!(read::<u32>(&memory, address), Some(0x6d6d_6d6d));
                assert_eq!(read::<i32>(&memory, address), Some(0x6d6d_6d6d));
                assert_eq!(read::<[u8; 4]>(&memory, address), Some([0x6d; 4]));
            }
            assert_eq!(read::<u32>(&memory, crossing), None);
            assert_eq!(read::<i32>(&memory, crossing), None);
            assert_eq!(read::<[u8; 4]>(&memory, crossing), None);
        }

        let mut first = [0xa5; 64];
        let mut failed = [0xa5; 64];
        let mut later = [0xa5; 64];
        assert_eq!(
            memory.copy_scattered(
                [valid, crossing, valid],
                &mut [
                    &mut first[..width],
                    &mut failed[..width],
                    &mut later[..width]
                ],
            ),
            1
        );
        assert_eq!(first[..width], [0x6d; 64][..width]);
        assert_eq!(later, [0xa5; 64]);
        let field = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: width,
        }];
        let mut records = [0xa5; 192];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[valid, crossing, valid]),
                &RecordLayout::new(&field, width).expect("valid record layout"),
                &mut records
            ),
            1
        );
        assert_eq!(records[..width], [0x6d; 64][..width]);
        assert!(records[width * 2..].iter().all(|&byte| byte == 0xa5));
    }
    assert_ne!(
        unsafe { VirtualProtect(boundary as *mut c_void, 4096, 0x104, &mut old) },
        0
    );
    assert_eq!(read::<u32>(&memory, boundary - 3), None);
}

#[test]
fn canonical_range_rejects_live_crash_vectors_and_keeps_valid_spans() {
    assert!(!is_canonical_range(0x0072_6f74_6361_665f, 8));
    assert!(!is_canonical_range(0x6f74_6361_665f_7374, 8));
    assert!(!is_canonical_range(0x0000_8000_0000_0000, 8));
    assert!(!is_canonical_range(0xFFFF_0000_0000_0000, 8));
    assert!(is_canonical_range(0x1_4000_0000, 8));
    assert!(is_canonical_range(0xFFFF_8000_0000_0000, 8));
    assert!(!is_canonical_range(0x0000_7FFF_FFFF_FFF0, 32));
    assert!(is_canonical_range(0x2000, 0));
    assert!(!is_canonical_range(usize::MAX - 3, 8));
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn records_gate_derived_noncanonical_fields_in_copy_order() {
    let source = 0x1234_5678_9abc_def0_u64.to_le_bytes();
    let pointer = source.as_ptr() as usize;
    let fields = [
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        },
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 8,
            len: 0,
        },
        RecordField {
            source_offset: 0x0000_8000_0000_0000 - pointer,
            destination_offset: 8,
            len: 8,
        },
        RecordField {
            source_offset: 0,
            destination_offset: 16,
            len: 8,
        },
    ];
    let memory = unsafe { LiveMemory::new() };
    let mut bytes = [0xa5; 24];
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[pointer]),
            &RecordLayout::new(&fields, 24).expect("valid record layout"),
            &mut bytes
        ),
        3
    );
    assert_eq!(bytes[..8], source);
    assert_eq!(bytes[8..], [0xa5; 16]);
    bytes.fill(0xa5);
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[1]),
            &RecordLayout::new(&fields, 24).expect("valid record layout"),
            &mut bytes
        ),
        1
    );
    assert_eq!(bytes, [0xa5; 24]);
    let crossing = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 24,
    }];
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[0x0000_7fff_ffff_fff0]),
            &RecordLayout::new(&crossing, 24).expect("valid record layout"),
            &mut bytes
        ),
        0
    );
    assert_eq!(bytes, [0xa5; 24]);
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn live_memory_rejects_noncanonical_sources_pre_copy() {
    let memory = unsafe { LiveMemory::new() };
    let mut out = [0xa5; 8];
    assert!(!memory.copy(0x0072_6f74_6361_665f, &mut out));
    assert!(!memory.copy(0x6f74_6361_665f_7374, &mut out));
    assert_eq!(out, [0xa5; 8]);
    let source = 0x1234_5678_u64.to_le_bytes();
    let mut valid = [0; 8];
    assert!(memory.copy(source.as_ptr() as usize, &mut valid));
    assert_eq!(valid, source);
    let mut first = [0; 8];
    let mut failed = [0xa5; 8];
    let mut later = [0xa5; 8];
    assert_eq!(
        memory.copy_scattered(
            [
                source.as_ptr() as usize,
                0x0072_6f74_6361_665f,
                source.as_ptr() as usize
            ],
            &mut [&mut first, &mut failed, &mut later],
        ),
        1
    );
    assert_eq!(first, source);
    assert_eq!(failed, [0xa5; 8]);
    assert_eq!(later, [0xa5; 8]);
    let fields = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    }];
    let mut bytes = [0xa5; 16];
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[source.as_ptr() as usize, 0x0072_6f74_6361_665f]),
            &RecordLayout::new(&fields[..1], 8).expect("valid record layout"),
            &mut bytes
        ),
        1
    );
    assert_eq!(&bytes[..8], &source);
    assert_eq!(&bytes[8..], &[0xa5; 8]);
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn guarded_copy_handles_noncanonical_fault_address_gracefully() {
    unsafe extern "C" {
        fn seh_guarded_copy(dst: *mut u8, src: *const u8, len: usize) -> i32;
        fn seh_guarded_copy4(dst: *mut u8, src: *const u8) -> i32;
        fn seh_guarded_copy8(dst: *mut u8, src: *const u8) -> i32;
        fn seh_guarded_read4(src: *const u8) -> u64;
    }
    let mut dst = [0xa5; 8];
    let result =
        unsafe { seh_guarded_copy(dst.as_mut_ptr(), 0x0072_6f74_6361_665f as *const u8, 8) };
    assert_eq!(result, 0);
    assert_eq!(dst, [0xa5; 8]);
    for copy in [seh_guarded_copy4, seh_guarded_copy8] {
        let result = unsafe { copy(dst.as_mut_ptr(), 0x0072_6f74_6361_665f as *const u8) };
        assert_eq!(result, 0);
        assert_eq!(dst, [0xa5; 8]);
    }
    assert_eq!(
        unsafe { seh_guarded_read4(0x0072_6f74_6361_665f as *const u8) },
        0
    );
}

#[test]
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn guard_page_reads_are_unreadable_and_leave_the_guard_armed() {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
        fn VirtualQuery(address: *const c_void, info: *mut [u32; 12], size: usize) -> usize;
    }
    struct Pages(*mut c_void);
    impl Drop for Pages {
        fn drop(&mut self) {
            unsafe { VirtualFree(self.0, 0, 0x8000) };
        }
    }
    let pages = Pages(unsafe { VirtualAlloc(std::ptr::null_mut(), 4096, 0x3000, 0x104) });
    assert!(!pages.0.is_null());
    let guarded = pages.0 as usize;
    let protection = || {
        let mut info = [0_u32; 12];
        assert_eq!(
            unsafe { VirtualQuery(guarded as *const c_void, &mut info, 48) },
            48
        );
        info[9]
    };
    assert_eq!(protection(), 0x104);
    let memory = unsafe { LiveMemory::new() };
    let field = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    }];
    let layout = RecordLayout::new(&field, 8).expect("valid record layout");
    let reads: [(&str, &dyn Fn() -> bool); 7] = [
        ("copy 4", &|| memory.copy(guarded, &mut [0; 4])),
        ("copy 8", &|| memory.copy(guarded, &mut [0; 8])),
        ("copy 13", &|| memory.copy(guarded, &mut [0; 13])),
        ("read u32", &|| read::<u32>(&memory, guarded).is_some()),
        ("read u64", &|| read::<u64>(&memory, guarded).is_some()),
        ("scattered", &|| {
            memory.copy_scattered([guarded], &mut [&mut [0; 8]]) != 0
        }),
        ("records", &|| {
            memory.copy_records(RecordSources::Pointers(&[guarded]), &layout, &mut [0; 8]) != 0
        }),
    ];
    for (name, attempt) in reads {
        assert!(!attempt(), "{name} read a guard page");
        assert_eq!(protection(), 0x104, "{name} disarmed the guard page");
    }
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
mod native_record_lookahead;
