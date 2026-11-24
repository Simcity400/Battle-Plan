use super::*;
use std::ffi::c_void;

const PAGE: usize = 4096;
const READWRITE: u32 = 4;
const NOACCESS: u32 = 1;
const GUARD: u32 = 0x100;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn VirtualAlloc(address: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn VirtualProtect(address: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
    fn VirtualFree(address: *mut c_void, size: usize, kind: u32) -> i32;
}

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

struct Pages {
    address: *mut c_void,
    count: usize,
}

impl Pages {
    fn new(count: usize) -> Self {
        let address =
            unsafe { VirtualAlloc(std::ptr::null_mut(), count * PAGE, 0x3000, READWRITE) };
        assert!(!address.is_null());
        let pages = Self { address, count };
        for page in 0..count {
            unsafe { std::ptr::write_bytes(pages.at(page) as *mut u8, 0x20 + page as u8, PAGE) };
        }
        pages
    }

    fn at(&self, page: usize) -> usize {
        assert!(page < self.count);
        self.address as usize + page * PAGE
    }

    fn protect(&self, page: usize, protection: u32) -> u32 {
        let mut previous = 0;
        assert_ne!(
            unsafe {
                VirtualProtect(
                    self.at(page) as *mut c_void,
                    PAGE,
                    protection,
                    &mut previous,
                )
            },
            0
        );
        previous
    }
}

impl Drop for Pages {
    fn drop(&mut self) {
        unsafe { VirtualFree(self.address, 0, 0x8000) };
    }
}

#[test]
fn guard_page_reads_are_unreadable_and_leave_the_guard_armed() {
    let pages = Pages::new(1);
    let guarded = pages.at(0);
    let memory = unsafe { LiveMemory::new() };
    let layout = RecordLayout::new(&POINTER_FIELDS, 8).expect("valid record layout");
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
    pages.protect(0, READWRITE | GUARD);
    for (name, attempt) in reads {
        assert!(!attempt(), "{name} read a guard page");
        assert_eq!(
            pages.protect(0, READWRITE | GUARD),
            READWRITE | GUARD,
            "{name} disarmed the guard page"
        );
    }
}

#[test]
fn prefetched_scattered_matches_native_faults_at_every_span_and_page_boundary() {
    const ROWS: usize = 9;
    let pages = Pages::new(ROWS * 2);
    let memory = unsafe { LiveMemory::new() };
    for width in [1, 2, 4, 8, 12, 16, 24, 26, 32, 48, 64] {
        for offset in [0, 61, PAGE - 4] {
            let addresses = std::array::from_fn::<_, ROWS, _>(|row| pages.at(row * 2) + offset);
            for failed in 0..ROWS {
                let failed_page = failed * 2 + usize::from(offset + width > PAGE);
                for protection in [NOACCESS, READWRITE | GUARD] {
                    pages.protect(failed_page, protection);
                    let mut expected = [[0xa5; 64]; ROWS];
                    let count = memory.copy_scattered(
                        addresses,
                        &mut expected.each_mut().map(|bytes| &mut bytes[..width]),
                    );
                    assert_eq!(count, failed);
                    pages.protect(failed_page, protection);
                    let mut actual = [[0xa5; 64]; ROWS];
                    let count = memory.copy_scattered_prefetched(
                        addresses,
                        &mut actual.each_mut().map(|bytes| &mut bytes[..width]),
                    );
                    assert_eq!(count, failed);
                    actual[failed][..width].fill(0xa5);
                    expected[failed][..width].fill(0xa5);
                    assert_eq!(actual, expected);
                    assert!(
                        actual[failed + 1..]
                            .iter()
                            .flatten()
                            .all(|&byte| byte == 0xa5)
                    );
                    assert_eq!(pages.protect(failed_page, READWRITE), protection);
                }
            }
        }
    }
}

#[test]
fn prefetched_scattered_future_hints_leave_guard_pages_armed() {
    let pages = Pages::new(9);
    let addresses = std::array::from_fn::<_, 9, _>(|row| pages.at(row) + 61);
    let memory = unsafe { LiveMemory::new() };
    for (failed, guarded) in [(0, 1), (0, 4), (1, 5), (4, 8)] {
        pages.protect(failed, NOACCESS);
        pages.protect(guarded, READWRITE | GUARD);
        let mut output = [[0xa5; 8]; 9];
        assert_eq!(
            memory.copy_scattered_prefetched(
                addresses,
                &mut output.each_mut().map(|bytes| bytes.as_mut_slice())
            ),
            failed,
        );
        for (row, bytes) in output[..failed].iter().enumerate() {
            assert_eq!(*bytes, [0x20 + row as u8; 8]);
        }
        assert!(output[failed..].iter().flatten().all(|&byte| byte == 0xa5));
        assert_eq!(pages.protect(failed, READWRITE), NOACCESS);
        assert_eq!(pages.protect(guarded, READWRITE), READWRITE | GUARD);
    }
}

#[test]
fn strided_native_faults_keep_exact_fields_and_overflow_prefixes() {
    const FIELDS: [RecordField; 3] = [
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        },
        RecordField {
            source_offset: PAGE - 4,
            destination_offset: 1,
            len: 8,
        },
        RecordField {
            source_offset: 2 * PAGE,
            destination_offset: 12,
            len: 4,
        },
    ];
    const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 16).unwrap();
    let pages = Pages::new(9);
    let memory = unsafe { LiveMemory::new() };
    for failed_row in 0..3 {
        for (failed_field, failed) in FIELDS.iter().enumerate().skip(1) {
            for protection in [NOACCESS, READWRITE | GUARD] {
                let failed_page = failed_row * 3 + failed_field;
                pages.protect(failed_page, protection);
                let mut output = [0xa5; 48];
                let copied = memory.copy_records(
                    RecordSources::Strided {
                        base: pages.at(0),
                        stride: 3 * PAGE,
                        count: 3,
                    },
                    &LAYOUT,
                    &mut output,
                );
                assert_eq!(copied, failed_row * 3 + failed_field);
                let mut expected = [0xa5; 48];
                for index in 0..copied {
                    let row = index / FIELDS.len();
                    let field = FIELDS[index % FIELDS.len()];
                    for offset in 0..field.len {
                        expected[row * 16 + field.destination_offset + offset] =
                            0x20 + (row * 3 + (field.source_offset + offset) / PAGE) as u8;
                    }
                }
                let start = failed_row * 16 + failed.destination_offset;
                output[start..start + failed.len].fill(0xa5);
                assert_eq!(output, expected);
                pages.protect(failed_page, READWRITE);
            }
        }
    }
    const EMPTY_FIRST: [RecordField; 3] = [
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
    ];
    const OVERFLOW_LAYOUT: RecordLayout<'static> = RecordLayout::new(&EMPTY_FIRST, 8).unwrap();
    let sources = RecordSources::Strided {
        base: pages.at(0),
        stride: usize::MAX - pages.at(0) + 1,
        count: 2,
    };
    let mut output = [0xa5; 16];
    assert_eq!(record_source_prefix(sources, &OVERFLOW_LAYOUT), 4);
    assert_eq!(
        memory.copy_records(sources, &OVERFLOW_LAYOUT, &mut output),
        4
    );
    assert_eq!(&output[..8], &[0x20; 8]);
    assert_eq!(&output[8..], &[0xa5; 8]);
    pages.protect(0, NOACCESS);
    output.fill(0xa5);
    assert_eq!(
        memory.copy_records(sources, &OVERFLOW_LAYOUT, &mut output),
        1
    );
    assert_eq!(output, [0xa5; 16]);
}

fn gather(
    pointers: &[usize],
    fields: &[RecordField],
    prefix: usize,
    stride: usize,
    destination: &mut [u8],
) -> usize {
    assert!(
        prefix
            <= RecordLayout::new(fields, stride)
                .and_then(
                    |layout| layout.count(RecordSources::Pointers(pointers), destination.len())
                )
                .unwrap()
    );
    assert!(
        prefix
            <= record_source_prefix(
                RecordSources::Pointers(pointers),
                &RecordLayout::new(fields, usize::MAX).unwrap()
            )
    );
    unsafe {
        seh_guarded_copy_records(
            pointers.as_ptr(),
            0,
            0,
            prefix,
            fields.as_ptr(),
            fields.len(),
            stride,
            destination.as_mut_ptr(),
        )
    }
}

#[test]
fn every_owned_page_fault_keeps_every_flattened_prefix_exact() {
    const ROWS: usize = 9;
    const STRIDE: usize = 64;
    let pages = Pages::new(ROWS * 3);
    let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 3));
    let fields = [
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        },
        RecordField {
            source_offset: 61,
            destination_offset: 1,
            len: 8,
        },
        RecordField {
            source_offset: PAGE + 61,
            destination_offset: 12,
            len: 12,
        },
        RecordField {
            source_offset: 3 * PAGE - 24,
            destination_offset: 28,
            len: 24,
        },
    ];
    for failed_row in 0..ROWS {
        for failed_field in 1..fields.len() {
            let failed_index = failed_row * fields.len() + failed_field;
            let failed_page = failed_row * 3 + failed_field - 1;
            for protection in [NOACCESS, READWRITE | GUARD] {
                for prefix in 0..=ROWS * fields.len() {
                    pages.protect(failed_page, protection);
                    let mut output = [0xa5; ROWS * STRIDE];
                    let expected_prefix = prefix.min(failed_index);
                    assert_eq!(
                        gather(&pointers, &fields, prefix, STRIDE, &mut output),
                        expected_prefix,
                        "row={failed_row} field={failed_field} prefix={prefix}"
                    );
                    let mut expected = [0xa5; ROWS * STRIDE];
                    for index in 0..expected_prefix {
                        let row = index / fields.len();
                        let field = fields[index % fields.len()];
                        if field.len != 0 {
                            let value = 0x20 + (row * 3 + field.source_offset / PAGE) as u8;
                            let start = row * STRIDE + field.destination_offset;
                            expected[start..start + field.len].fill(value);
                        }
                    }
                    assert_eq!(
                        output, expected,
                        "row={failed_row} field={failed_field} prefix={prefix}"
                    );
                    assert_eq!(pages.protect(failed_page, READWRITE), protection);
                }
            }
        }
    }
}

#[test]
fn future_guard_pages_survive_an_earlier_noaccess_failure() {
    let pages = Pages::new(9);
    let pointers: [usize; 9] = std::array::from_fn(|row| pages.at(row));
    let fields = [RecordField {
        source_offset: 61,
        destination_offset: 0,
        len: 8,
    }];
    for (failed_row, guarded_row) in [(0, 1), (0, 4), (1, 5), (4, 8)] {
        pages.protect(failed_row, NOACCESS);
        pages.protect(guarded_row, READWRITE | GUARD);
        let mut output = [0xa5; 9 * 8];
        assert_eq!(gather(&pointers, &fields, 9, 8, &mut output), failed_row);
        for row in 0..failed_row {
            assert_eq!(&output[row * 8..row * 8 + 8], &[0x20 + row as u8; 8]);
        }
        assert!(output[failed_row * 8..].iter().all(|&byte| byte == 0xa5));
        assert_eq!(pages.protect(failed_row, READWRITE), NOACCESS);
        assert_eq!(
            pages.protect(guarded_row, READWRITE),
            READWRITE | GUARD,
            "a hint must not consume the future guard"
        );
        pages.protect(guarded_row, READWRITE | GUARD);
        let memory = unsafe { LiveMemory::new() };
        let mut actual = [0xa5; 8];
        assert!(!memory.copy(pointers[guarded_row] + 61, &mut actual));
        assert_eq!(actual, [0xa5; 8]);
        assert_eq!(pages.protect(guarded_row, READWRITE), READWRITE | GUARD);
    }
}

const POINTER_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];

#[test]
fn pointer_layout_keeps_owned_faults_and_every_prefix_exact() {
    const ROWS: usize = 9;
    let pages = Pages::new(ROWS * 2);
    for offset in [0, 61, PAGE - 4] {
        let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 2) + offset);
        for failed_row in 0..ROWS {
            let failed_page = failed_row * 2 + usize::from(offset + 8 > PAGE);
            for protection in [NOACCESS, READWRITE | GUARD] {
                for prefix in 0..=ROWS {
                    pages.protect(failed_page, protection);
                    let mut output = [0xa5; ROWS * 8];
                    let expected_prefix = prefix.min(failed_row);
                    assert_eq!(
                        gather(&pointers, &POINTER_FIELDS, prefix, 8, &mut output),
                        expected_prefix,
                        "offset={offset} failed_row={failed_row} prefix={prefix}"
                    );
                    let mut expected = [0xa5; ROWS * 8];
                    for row in 0..expected_prefix {
                        for byte in 0..8 {
                            expected[row * 8 + byte] =
                                0x20 + (row * 2 + (offset + byte) / PAGE) as u8;
                        }
                    }
                    assert_eq!(output, expected);
                    assert_eq!(
                        pages.protect(failed_page, READWRITE),
                        protection,
                        "a hint must not consume the failing row's guard"
                    );
                }
            }
        }
    }
}

#[test]
fn pointer_layout_future_hints_preserve_guards_after_an_earlier_fault() {
    let pages = Pages::new(9);
    let pointers: [usize; 9] = std::array::from_fn(|row| pages.at(row) + 61);
    for (failed_row, guarded_row) in [(0, 1), (0, 4), (1, 5), (4, 8)] {
        pages.protect(failed_row, NOACCESS);
        pages.protect(guarded_row, READWRITE | GUARD);
        let mut output = [0xa5; 9 * 8];
        assert_eq!(
            gather(&pointers, &POINTER_FIELDS, 9, 8, &mut output),
            failed_row
        );
        for row in 0..failed_row {
            assert_eq!(&output[row * 8..row * 8 + 8], &[0x20 + row as u8; 8]);
        }
        assert!(output[failed_row * 8..].iter().all(|&byte| byte == 0xa5));
        assert_eq!(pages.protect(failed_row, READWRITE), NOACCESS);
        assert_eq!(pages.protect(guarded_row, READWRITE), READWRITE | GUARD);
        pages.protect(guarded_row, READWRITE | GUARD);
        let memory = unsafe { LiveMemory::new() };
        let mut actual = [0xa5; 8];
        assert!(!memory.copy(pointers[guarded_row], &mut actual));
        assert_eq!(actual, [0xa5; 8]);
        assert_eq!(pages.protect(guarded_row, READWRITE), READWRITE | GUARD);
    }
}

#[test]
fn pointer_layout_duplicates_and_empty_prefix_never_read_metadata_suffixes() {
    const ROWS: usize = 9;
    let metadata = Pages::new(2);
    metadata.protect(1, NOACCESS);
    let layout_metadata = Pages::new(2);
    layout_metadata.protect(1, NOACCESS);
    let field_start = layout_metadata.at(1) - size_of::<RecordField>();
    unsafe { (field_start as *mut RecordField).write(POINTER_FIELDS[0]) };
    let fields = unsafe { std::slice::from_raw_parts(field_start as *const RecordField, 1) };
    let source: [u8; 160] = std::array::from_fn(|index| index as u8);
    let offsets = [61, 4, 61, 127, 4, 61, 0, 63, 61];
    for prefix in 0..=ROWS {
        let pointer_start = metadata.at(1) - prefix * size_of::<usize>();
        for (row, offset) in offsets[..prefix].iter().enumerate() {
            unsafe {
                (pointer_start as *mut usize)
                    .add(row)
                    .write(source.as_ptr() as usize + offset)
            };
        }
        let pointers = unsafe { std::slice::from_raw_parts(pointer_start as *const usize, prefix) };
        let mut output = [0xa5; ROWS * 8 + 2];
        assert_eq!(
            gather(pointers, fields, prefix, 8, &mut output[1..1 + ROWS * 8]),
            prefix
        );
        let mut expected = [0xa5; ROWS * 8 + 2];
        for (row, &offset) in offsets[..prefix].iter().enumerate() {
            expected[1 + row * 8..1 + row * 8 + 8].copy_from_slice(&source[offset..offset + 8]);
        }
        assert_eq!(output, expected, "prefix={prefix}");
    }
    assert_eq!(
        unsafe {
            seh_guarded_copy_records(
                metadata.at(1) as *const usize,
                0,
                0,
                0,
                layout_metadata.at(1) as *const RecordField,
                1,
                8,
                metadata.at(1) as *mut u8,
            )
        },
        0
    );
}

#[test]
fn pointer_layout_nonmatching_fields_keep_generic_destinations_exact() {
    let first = POINTER_FIELDS[0];
    let second = RecordField {
        source_offset: 8,
        destination_offset: 0,
        len: 8,
    };
    let layouts = [
        (
            [
                RecordField {
                    source_offset: 1,
                    ..first
                },
                second,
            ],
            1,
            8,
        ),
        (
            [
                RecordField {
                    destination_offset: 1,
                    ..first
                },
                second,
            ],
            1,
            9,
        ),
        ([RecordField { len: 7, ..first }, second], 1, 8),
        ([first, second], 1, 9),
        ([first, second], 2, 8),
    ];
    let source: [u8; 64] = std::array::from_fn(|index| index as u8);
    let pointers = [source.as_ptr() as usize, source.as_ptr() as usize + 32];
    let memory = unsafe { LiveMemory::new() };
    for (layout, field_count, stride) in layouts {
        let fields = &layout[..field_count];
        let mut output = [0xa5; 32];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&pointers),
                &RecordLayout::new(fields, stride).expect("valid record layout"),
                &mut output
            ),
            2 * field_count
        );
        let mut expected = [0xa5; 32];
        for row in 0..2 {
            for field in fields {
                let start = row * stride + field.destination_offset;
                let source_start = row * 32 + field.source_offset;
                expected[start..start + field.len]
                    .copy_from_slice(&source[source_start..source_start + field.len]);
            }
        }
        assert_eq!(output, expected);
    }
}

fn packed_two_fields(offsets: [usize; 2]) -> [RecordField; 2] {
    [
        RecordField {
            source_offset: offsets[0],
            destination_offset: 0,
            len: 2,
        },
        RecordField {
            source_offset: offsets[1],
            destination_offset: 8,
            len: 8,
        },
    ]
}

#[test]
fn packed_two_dynamic_offsets_duplicates_and_every_metadata_prefix() {
    const ROWS: usize = 65;
    let metadata = Pages::new(2);
    metadata.protect(1, NOACCESS);
    let layouts = Pages::new(2);
    layouts.protect(1, NOACCESS);
    let layout_start = layouts.at(1) - 2 * size_of::<RecordField>();
    let row_offsets = [61, 4, 61, 127, 4, 61, 0, 63, 61];
    let memory = unsafe { LiveMemory::new() };
    for epoch in 0..2 {
        let source: [u8; 1024] = std::array::from_fn(|index| (index * 37 + epoch * 73) as u8);
        for offsets in [[7, 74], [7, 7], [7, 8], [31, 3], [0x158, 8]] {
            let layout = packed_two_fields(offsets);
            for (index, field) in layout.iter().enumerate() {
                unsafe { (layout_start as *mut RecordField).add(index).write(*field) };
            }
            let fields =
                unsafe { std::slice::from_raw_parts(layout_start as *const RecordField, 2) };
            for prefix in 0..=ROWS * 2 {
                let rows = prefix.div_ceil(2);
                let pointer_start = metadata.at(1) - rows * size_of::<usize>();
                for row in 0..rows {
                    unsafe {
                        (pointer_start as *mut usize)
                            .add(row)
                            .write(source.as_ptr() as usize + row_offsets[row % row_offsets.len()]);
                    }
                }
                let pointers =
                    unsafe { std::slice::from_raw_parts(pointer_start as *const usize, rows) };
                let mut output = [0xa5; ROWS * 16 + 2];
                assert_eq!(
                    gather(pointers, fields, prefix, 16, &mut output[1..1 + ROWS * 16]),
                    prefix
                );
                let mut expected = [0xa5; ROWS * 16 + 2];
                for index in 0..prefix {
                    let row = index / 2;
                    let field = fields[index % 2];
                    let source_start = row_offsets[row % row_offsets.len()] + field.source_offset;
                    let start = 1 + row * 16 + field.destination_offset;
                    expected[start..start + field.len]
                        .copy_from_slice(&source[source_start..source_start + field.len]);
                }
                assert_eq!(
                    output, expected,
                    "epoch={epoch} offsets={offsets:?} prefix={prefix}"
                );
            }
            for count in [0, 1, 2, 3, 4, 5, 63, 64, 65] {
                let mut output = [0xa5; ROWS * 16 + 2];
                assert_eq!(
                    memory.copy_records(
                        RecordSources::Strided {
                            base: source.as_ptr() as usize + 1,
                            stride: 7,
                            count,
                        },
                        &RecordLayout::new(fields, 16).unwrap(),
                        &mut output[1..1 + ROWS * 16],
                    ),
                    count * 2
                );
                let mut expected = [0xa5; ROWS * 16 + 2];
                for row in 0..count {
                    for field in fields {
                        let source_start = 1 + row * 7 + field.source_offset;
                        let start = 1 + row * 16 + field.destination_offset;
                        expected[start..start + field.len]
                            .copy_from_slice(&source[source_start..source_start + field.len]);
                    }
                }
                assert_eq!(
                    output, expected,
                    "epoch={epoch} offsets={offsets:?} count={count}"
                );
            }
        }
    }
    assert_eq!(
        unsafe {
            seh_guarded_copy_records(
                metadata.at(1) as *const usize,
                0,
                0,
                0,
                layouts.at(1) as *const RecordField,
                2,
                16,
                metadata.at(1) as *mut u8,
            )
        },
        0
    );
}

#[test]
fn packed_two_every_owned_source_fault_preserves_the_exact_prefix() {
    const ROWS: usize = 9;
    let pages = Pages::new(ROWS * 4);
    let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 4));
    let widths = [2, 8];
    for last_byte in [false, true] {
        let fields = packed_two_fields(std::array::from_fn(|field| {
            field * 2 * PAGE
                + if last_byte {
                    PAGE - widths[field] + 1
                } else {
                    61
                }
        }));
        for failed in 0..ROWS * 2 {
            let failed_page = failed / 2 * 4 + failed % 2 * 2 + usize::from(last_byte);
            for protection in [NOACCESS, READWRITE | GUARD] {
                for prefix in 0..=ROWS * 2 {
                    pages.protect(failed_page, protection);
                    let mut output = [0xa5; ROWS * 16 + 2];
                    let complete = prefix.min(failed);
                    assert_eq!(
                        gather(
                            &pointers,
                            &fields,
                            prefix,
                            16,
                            &mut output[1..1 + ROWS * 16],
                        ),
                        complete,
                        "last_byte={last_byte} failed={failed} prefix={prefix}"
                    );
                    assert_eq!(
                        pages.protect(failed_page, READWRITE),
                        protection,
                        "unreached and odd-prefix excluded guards must remain armed"
                    );
                    let mut expected = [0xa5; ROWS * 16 + 2];
                    for index in 0..complete {
                        let row = index / 2;
                        let field = fields[index % 2];
                        for byte in 0..field.len {
                            expected[1 + row * 16 + field.destination_offset + byte] =
                                0x20 + (row * 4 + (field.source_offset + byte) / PAGE) as u8;
                        }
                    }
                    if prefix > failed {
                        let field = fields[failed % 2];
                        let start = 1 + failed / 2 * 16 + field.destination_offset;
                        output[start..start + field.len].fill(0xa5);
                    }
                    assert_eq!(output, expected);
                }
            }
        }
    }

    let fields = packed_two_fields(std::array::from_fn(|field| {
        field * 2 * PAGE + PAGE - widths[field]
    }));
    for pair in 0..ROWS * 2 {
        pages.protect(pair * 2 + 1, NOACCESS);
    }
    let mut output = [0xa5; ROWS * 16 + 2];
    assert_eq!(
        gather(
            &pointers,
            &fields,
            ROWS * 2,
            16,
            &mut output[1..1 + ROWS * 16],
        ),
        ROWS * 2
    );
    let mut expected = [0xa5; ROWS * 16 + 2];
    for row in 0..ROWS {
        for (index, field) in fields.iter().enumerate() {
            let start = 1 + row * 16 + field.destination_offset;
            expected[start..start + field.len].fill(0x20 + (row * 4 + index * 2) as u8);
        }
    }
    assert_eq!(output, expected);
    for pair in 0..ROWS * 2 {
        assert_eq!(pages.protect(pair * 2 + 1, READWRITE), NOACCESS);
    }
}

#[test]
fn packed_two_future_endpoint_hints_do_not_consume_guards() {
    const ROWS: usize = 9;
    let pages = Pages::new(ROWS * 2);
    let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 2));
    let fields = packed_two_fields([63, PAGE + 61]);
    for (failed_row, future_row) in [(0, 1), (0, 4), (1, 5), (4, 8)] {
        for failed_field in 0..2 {
            for future_field in 0..2 {
                let failed_page = failed_row * 2 + failed_field;
                let future_page = future_row * 2 + future_field;
                pages.protect(failed_page, NOACCESS);
                pages.protect(future_page, READWRITE | GUARD);
                let mut output = [0xa5; ROWS * 16];
                assert_eq!(
                    gather(&pointers, &fields, ROWS * 2, 16, &mut output),
                    failed_row * 2 + failed_field
                );
                assert_eq!(pages.protect(failed_page, READWRITE), NOACCESS);
                assert_eq!(pages.protect(future_page, READWRITE), READWRITE | GUARD);
                pages.protect(future_page, READWRITE | GUARD);
                let memory = unsafe { LiveMemory::new() };
                let mut bytes = [0xa5; 8];
                assert!(!memory.copy(
                    pointers[future_row] + fields[future_field].source_offset,
                    &mut bytes[..fields[future_field].len],
                ));
                assert_eq!(bytes, [0xa5; 8]);
                assert_eq!(pages.protect(future_page, READWRITE), READWRITE | GUARD);
            }
        }
    }
}

#[test]
fn packed_two_layout_mismatches_keep_generic_order_and_padding() {
    let source: [u8; 1024] = std::array::from_fn(|index| (index * 37) as u8);
    let pointers = [
        source.as_ptr() as usize + 13,
        source.as_ptr() as usize + 513,
    ];
    let base = packed_two_fields([31, 3]);
    let mut layouts = vec![(base.to_vec(), 16), (base.to_vec(), 17)];
    for index in 0..2 {
        let mut fields = base;
        fields[index].len -= 1;
        layouts.push((fields.to_vec(), 16));
        let mut fields = base;
        fields[index].destination_offset = if index == 0 { 1 } else { 7 };
        layouts.push((fields.to_vec(), 16));
    }
    let mut overlap = base;
    overlap[1].destination_offset = 1;
    layouts.push((overlap.to_vec(), 16));
    layouts.push((base[..1].to_vec(), 16));
    layouts.push((vec![], 16));
    let mut fields = base.to_vec();
    fields.push(RecordField {
        source_offset: usize::MAX,
        destination_offset: 16,
        len: 0,
    });
    layouts.push((fields, 16));
    let memory = unsafe { LiveMemory::new() };
    for (fields, stride) in layouts {
        let mut output = [0xa5; 36];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&pointers),
                &RecordLayout::new(&fields, stride).expect("valid record layout"),
                &mut output[1..35],
            ),
            2 * fields.len()
        );
        let mut expected = [0xa5; 36];
        for row in 0..2 {
            for field in &fields {
                if field.len != 0 {
                    let source_start = [13, 513][row] + field.source_offset;
                    let start = 1 + row * stride + field.destination_offset;
                    expected[start..start + field.len]
                        .copy_from_slice(&source[source_start..source_start + field.len]);
                }
            }
        }
        assert_eq!(output, expected);
    }
}

fn packed_five_fields(offsets: [usize; 5]) -> [RecordField; 5] {
    let widths = [12, 16, 8, 8, 8];
    let destinations = [0, 12, 28, 36, 44];
    std::array::from_fn(|index| RecordField {
        source_offset: offsets[index],
        destination_offset: destinations[index],
        len: widths[index],
    })
}

#[test]
fn packed_five_dynamic_offsets_duplicates_and_every_metadata_prefix() {
    const ROWS: usize = 9;
    let metadata = Pages::new(2);
    metadata.protect(1, NOACCESS);
    let layouts = Pages::new(2);
    layouts.protect(1, NOACCESS);
    let layout_start = layouts.at(1) - 5 * size_of::<RecordField>();
    for epoch in 0..2 {
        let source: [u8; 1024] = std::array::from_fn(|index| (index * 37 + epoch * 73) as u8);
        for offsets in [[0, 0x158, 0x170, 0x198, 0x1c0], [7, 74, 11, 259, 31]] {
            let layout = packed_five_fields(offsets);
            for (index, field) in layout.iter().enumerate() {
                unsafe { (layout_start as *mut RecordField).add(index).write(*field) };
            }
            let fields =
                unsafe { std::slice::from_raw_parts(layout_start as *const RecordField, 5) };
            let offsets = [61, 4, 61, 127, 4, 61, 0, 63, 61];
            for prefix in 0..=ROWS * 5 {
                let rows = prefix.div_ceil(5);
                let pointer_start = metadata.at(1) - rows * size_of::<usize>();
                for (row, offset) in offsets[..rows].iter().enumerate() {
                    unsafe {
                        (pointer_start as *mut usize)
                            .add(row)
                            .write(source.as_ptr() as usize + offset);
                    }
                }
                let pointers =
                    unsafe { std::slice::from_raw_parts(pointer_start as *const usize, rows) };
                let mut output = [0xa5; ROWS * 52 + 2];
                assert_eq!(
                    gather(pointers, fields, prefix, 52, &mut output[1..1 + ROWS * 52]),
                    prefix
                );
                let mut expected = [0xa5; ROWS * 52 + 2];
                for index in 0..prefix {
                    let row = index / 5;
                    let field = fields[index % 5];
                    let source_start = offsets[row] + field.source_offset;
                    let start = 1 + row * 52 + field.destination_offset;
                    expected[start..start + field.len]
                        .copy_from_slice(&source[source_start..source_start + field.len]);
                }
                assert_eq!(output, expected, "epoch={epoch} prefix={prefix}");
            }
        }
    }
    assert_eq!(
        unsafe {
            seh_guarded_copy_records(
                metadata.at(1) as *const usize,
                0,
                0,
                0,
                layouts.at(1) as *const RecordField,
                5,
                52,
                metadata.at(1) as *mut u8,
            )
        },
        0
    );
}

#[test]
fn packed_five_every_owned_source_fault_preserves_the_exact_prefix() {
    const ROWS: usize = 9;
    let pages = Pages::new(ROWS * 10);
    let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 10));
    for last_byte in [false, true] {
        let widths = [12, 16, 8, 8, 8];
        let fields = packed_five_fields(std::array::from_fn(|field| {
            field * 2 * PAGE
                + if last_byte {
                    PAGE - widths[field] + 1
                } else {
                    61
                }
        }));
        for failed in 0..ROWS * 5 {
            let failed_page = failed / 5 * 10 + failed % 5 * 2 + usize::from(last_byte);
            for protection in [NOACCESS, READWRITE | GUARD] {
                for prefix in 0..=ROWS * 5 {
                    pages.protect(failed_page, protection);
                    let mut output = [0xa5; ROWS * 52 + 2];
                    let complete = prefix.min(failed);
                    assert_eq!(
                        gather(
                            &pointers,
                            &fields,
                            prefix,
                            52,
                            &mut output[1..1 + ROWS * 52]
                        ),
                        complete,
                        "last_byte={last_byte} failed={failed} prefix={prefix}"
                    );
                    assert_eq!(
                        pages.protect(failed_page, READWRITE),
                        protection,
                        "hints must not consume an unreached guard"
                    );
                    let mut expected = [0xa5; ROWS * 52 + 2];
                    for index in 0..complete {
                        let row = index / 5;
                        let field = fields[index % 5];
                        for byte in 0..field.len {
                            expected[1 + row * 52 + field.destination_offset + byte] =
                                0x20 + (row * 10 + (field.source_offset + byte) / PAGE) as u8;
                        }
                    }
                    if prefix > failed {
                        let field = fields[failed % 5];
                        let start = 1 + failed / 5 * 52 + field.destination_offset;
                        output[start..start + field.len].fill(0xa5);
                    }
                    assert_eq!(output, expected);
                }
            }
        }
    }

    let widths = [12, 16, 8, 8, 8];
    let fields = packed_five_fields(std::array::from_fn(|field| {
        field * 2 * PAGE + PAGE - widths[field]
    }));
    for pair in 0..ROWS * 5 {
        pages.protect(pair * 2 + 1, NOACCESS);
    }
    let mut output = [0xa5; ROWS * 52 + 2];
    assert_eq!(
        gather(
            &pointers,
            &fields,
            ROWS * 5,
            52,
            &mut output[1..1 + ROWS * 52]
        ),
        ROWS * 5
    );
    let mut expected = [0xa5; ROWS * 52 + 2];
    for row in 0..ROWS {
        for (index, field) in fields.iter().enumerate() {
            let start = 1 + row * 52 + field.destination_offset;
            expected[start..start + field.len].fill(0x20 + (row * 10 + index * 2) as u8);
        }
    }
    assert_eq!(output, expected);
    for pair in 0..ROWS * 5 {
        assert_eq!(pages.protect(pair * 2 + 1, READWRITE), NOACCESS);
    }
}

#[test]
fn packed_five_future_endpoint_hints_do_not_consume_guards() {
    const ROWS: usize = 9;
    let pages = Pages::new(ROWS * 5);
    let pointers: [usize; ROWS] = std::array::from_fn(|row| pages.at(row * 5));
    let fields = packed_five_fields(std::array::from_fn(|field| field * PAGE + 61));
    for (failed_row, future_row) in [(0, 4), (1, 5), (4, 8)] {
        for failed_field in 0..5 {
            for future_field in 0..5 {
                let failed_page = failed_row * 5 + failed_field;
                let future_page = future_row * 5 + future_field;
                pages.protect(failed_page, NOACCESS);
                pages.protect(future_page, READWRITE | GUARD);
                let mut output = [0xa5; ROWS * 52];
                assert_eq!(
                    gather(&pointers, &fields, ROWS * 5, 52, &mut output),
                    failed_row * 5 + failed_field
                );
                assert_eq!(pages.protect(failed_page, READWRITE), NOACCESS);
                assert_eq!(pages.protect(future_page, READWRITE), READWRITE | GUARD);
                pages.protect(future_page, READWRITE | GUARD);
                let memory = unsafe { LiveMemory::new() };
                let mut bytes = [0xa5; 16];
                assert!(!memory.copy(
                    pointers[future_row] + fields[future_field].source_offset,
                    &mut bytes[..fields[future_field].len]
                ));
                assert_eq!(bytes, [0xa5; 16]);
                assert_eq!(pages.protect(future_page, READWRITE), READWRITE | GUARD);
            }
        }
    }
}

#[test]
fn packed_five_layout_mismatches_keep_generic_order_and_padding() {
    let source: [u8; 1024] = std::array::from_fn(|index| (index * 37) as u8);
    let pointers = [
        source.as_ptr() as usize + 13,
        source.as_ptr() as usize + 513,
    ];
    let base = packed_five_fields([31, 3, 127, 61, 1]);
    let mut layouts = vec![(base.to_vec(), 52), (base.to_vec(), 53)];
    for index in 0..5 {
        let mut fields = base;
        fields[index].len -= 1;
        layouts.push((fields.to_vec(), 52));
        let mut fields = base;
        fields[index].destination_offset = if index == 0 {
            1
        } else {
            fields[index].destination_offset - 1
        };
        layouts.push((fields.to_vec(), 52));
    }
    layouts.push((base[..4].to_vec(), 52));
    let mut fields = base.to_vec();
    fields.push(RecordField {
        source_offset: usize::MAX,
        destination_offset: 52,
        len: 0,
    });
    layouts.push((fields, 52));
    let memory = unsafe { LiveMemory::new() };
    for (fields, stride) in layouts {
        let mut output = [0xa5; 110];
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&pointers),
                &RecordLayout::new(&fields, stride).expect("valid record layout"),
                &mut output[1..109]
            ),
            2 * fields.len()
        );
        let mut expected = [0xa5; 110];
        for row in 0..2 {
            for field in &fields {
                if field.len != 0 {
                    let source_start = [13, 513][row] + field.source_offset;
                    let start = 1 + row * stride + field.destination_offset;
                    expected[start..start + field.len]
                        .copy_from_slice(&source[source_start..source_start + field.len]);
                }
            }
        }
        assert_eq!(output, expected);
    }
}

#[test]
fn packed_five_canonical_preflight_and_native_minus_one_containment() {
    const NONCANONICAL: usize = 0x0000_8000_0000_0000;
    let source: [u8; 128] = std::array::from_fn(|index| (index * 37) as u8);
    let fields = packed_five_fields([0, 16, 32, 40, 48]);
    let valid = source.as_ptr() as usize;
    let memory = unsafe { LiveMemory::new() };
    for invalid in [0, NONCANONICAL, usize::MAX] {
        let pointers = [valid, invalid, valid];
        let mut output = [0xa5; 3 * 52];
        assert_eq!(
            record_source_prefix(
                RecordSources::Pointers(&pointers),
                &RecordLayout::new(&fields, usize::MAX).unwrap()
            ),
            5
        );
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&pointers),
                &RecordLayout::new(&fields, 52).expect("valid record layout"),
                &mut output
            ),
            5
        );
        let mut expected = [0xa5; 3 * 52];
        for field in fields {
            expected[field.destination_offset..field.destination_offset + field.len]
                .copy_from_slice(&source[field.source_offset..field.source_offset + field.len]);
        }
        assert_eq!(output, expected);
    }
    for invalid_field in 0..5 {
        let mut layout = fields;
        layout[invalid_field].source_offset = usize::MAX;
        let mut output = [0xa5; 52];
        assert_eq!(
            record_source_prefix(
                RecordSources::Pointers(&[valid]),
                &RecordLayout::new(&layout, usize::MAX).unwrap()
            ),
            invalid_field
        );
        assert_eq!(
            memory.copy_records(
                RecordSources::Pointers(&[valid]),
                &RecordLayout::new(&layout, 52).expect("valid record layout"),
                &mut output
            ),
            invalid_field
        );
        let mut expected = [0xa5; 52];
        for field in &fields[..invalid_field] {
            expected[field.destination_offset..field.destination_offset + field.len]
                .copy_from_slice(&source[field.source_offset..field.source_offset + field.len]);
        }
        assert_eq!(output, expected);
    }
    let pages = Pages::new(1);
    pages.protect(0, NOACCESS);
    let mut layout = fields;
    layout[4].source_offset = NONCANONICAL.checked_sub(pages.at(0)).unwrap();
    let mut output = [0xa5; 52];
    assert_eq!(
        record_source_prefix(
            RecordSources::Pointers(&[pages.at(0)]),
            &RecordLayout::new(&layout, usize::MAX).unwrap()
        ),
        4
    );
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[pages.at(0)]),
            &RecordLayout::new(&layout, 52).expect("valid record layout"),
            &mut output
        ),
        0,
        "earlier owned unreadable field beats a later noncanonical field"
    );
    assert_eq!(output, [0xa5; 52]);
    assert_eq!(pages.protect(0, READWRITE), NOACCESS);
    let bad = [0x0072_6f74_6361_665fusize];
    assert_eq!(
        unsafe {
            seh_guarded_copy_records(
                bad.as_ptr(),
                0,
                0,
                5,
                fields.as_ptr(),
                5,
                52,
                output.as_mut_ptr(),
            )
        },
        0
    );
    assert_eq!(output, [0xa5; 52]);
}

#[test]
fn packed_five_owned_destination_faults_keep_previous_fields_and_canaries() {
    let source: [u8; 128] = std::array::from_fn(|index| (index * 37) as u8);
    let fields = packed_five_fields([0, 16, 32, 40, 48]);
    let pointers = [source.as_ptr() as usize];
    let pages = Pages::new(3);
    for failed_field in 0..5 {
        for last_byte in [false, true] {
            let field = fields[failed_field];
            let fault_offset = field.destination_offset + if last_byte { field.len - 1 } else { 0 };
            let destination = (pages.at(1) - fault_offset) as *mut u8;
            for protection in [NOACCESS, READWRITE | GUARD] {
                unsafe { std::ptr::write_bytes(destination.sub(1), 0xa5, 54) };
                pages.protect(1, protection);
                let completed = unsafe {
                    seh_guarded_copy_records(
                        pointers.as_ptr(),
                        0,
                        0,
                        5,
                        fields.as_ptr(),
                        5,
                        52,
                        destination,
                    )
                };
                let old = pages.protect(1, READWRITE);
                assert_eq!(completed, failed_field);
                assert_eq!(old, protection);
                let mut actual: [u8; 54] = unsafe {
                    std::slice::from_raw_parts(destination.sub(1), 54)
                        .try_into()
                        .unwrap()
                };
                let mut expected = [0xa5; 54];
                for earlier in &fields[..failed_field] {
                    let start = 1 + earlier.destination_offset;
                    expected[start..start + earlier.len].copy_from_slice(
                        &source[earlier.source_offset..earlier.source_offset + earlier.len],
                    );
                }
                let start = 1 + field.destination_offset;
                actual[start..start + field.len].fill(0xa5);
                assert_eq!(actual, expected, "field={failed_field} last={last_byte}");
            }
        }
    }
}

#[test]
fn partial_final_rows_and_duplicate_fields_never_read_a_pointer_suffix() {
    const ROWS: usize = 9;
    const STRIDE: usize = 40;
    let metadata = Pages::new(2);
    metadata.protect(1, NOACCESS);
    let source: [u8; 160] = std::array::from_fn(|index| index as u8);
    let fields = [
        RecordField {
            source_offset: usize::MAX,
            destination_offset: 0,
            len: 0,
        },
        RecordField {
            source_offset: 61,
            destination_offset: 1,
            len: 12,
        },
        RecordField {
            source_offset: 4,
            destination_offset: 16,
            len: 8,
        },
        RecordField {
            source_offset: 61,
            destination_offset: 25,
            len: 12,
        },
        RecordField {
            source_offset: usize::MAX,
            destination_offset: STRIDE,
            len: 0,
        },
    ];
    for prefix in 0..=ROWS * fields.len() {
        let rows = prefix.div_ceil(fields.len());
        let pointer_start = metadata.at(1) - rows * size_of::<usize>();
        for row in 0..rows {
            unsafe {
                (pointer_start as *mut usize)
                    .add(row)
                    .write(source.as_ptr() as usize)
            };
        }
        let pointers = unsafe { std::slice::from_raw_parts(pointer_start as *const usize, rows) };
        let mut output = [0xa5; ROWS * STRIDE];
        assert_eq!(
            gather(pointers, &fields, prefix, STRIDE, &mut output),
            prefix
        );
        let mut expected = [0xa5; ROWS * STRIDE];
        for index in 0..prefix {
            let field = fields[index % fields.len()];
            if field.len != 0 {
                let start = index / fields.len() * STRIDE + field.destination_offset;
                expected[start..start + field.len]
                    .copy_from_slice(&source[field.source_offset..field.source_offset + field.len]);
            }
        }
        assert_eq!(output, expected, "prefix={prefix}");
    }
}
