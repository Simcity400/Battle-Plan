use super::*;
use std::cell::RefCell;

fn store_field(job: &mut CharactersJob, view: &mut ViewCopy, field: usize, bytes: &[u8]) {
    assert_eq!(bytes.len(), VIEW_LENS[field]);
    let memory = engine_native_memory::memory::SliceMemory {
        base: 0x1000,
        bytes,
    };
    let start = job
        .log
        .take(&memory, 0x1000, bytes.len(), VIEW_MASKS[field])
        .unwrap();
    view.starts[field] = u32::try_from(start).unwrap();
}

fn view(
    leader: usize,
    leader_count: u64,
    advisor: usize,
    advisor_count: u64,
    unit: usize,
) -> (CharactersJob, ViewCopy) {
    let mut job = CharactersJob {
        log: collector_raw::raw::RawLog::new(3, 40),
        ..CharactersJob::default()
    };
    let mut view = ViewCopy::default();
    for (field, pointer, count) in [(4, leader, leader_count), (6, advisor, advisor_count)] {
        let mut header = [0; 16];
        header[..8].copy_from_slice(&(pointer as u64).to_le_bytes());
        header[8..16].copy_from_slice(&count.to_le_bytes());
        store_field(&mut job, &mut view, field, &header);
    }
    store_field(&mut job, &mut view, 5, &(unit as u64).to_le_bytes());
    (job, view)
}

fn targets(source: &(CharactersJob, ViewCopy), dead: &Dead) -> Vec<(usize, usize)> {
    let mut hints = Vec::new();
    hint_character_targets(&source.0, &source.1, dead, |address, len| {
        hints.push((address, len))
    });
    hints
}

fn heap_header(source: usize, len: usize, capacity: usize) -> [u8; 32] {
    let mut header = [0; 32];
    header[..8].copy_from_slice(&(source as u64).to_le_bytes());
    header[16..24].copy_from_slice(&(len as u64).to_le_bytes());
    header[24..32].copy_from_slice(&(capacity as u64).to_le_bytes());
    header
}

fn literal_hints(header: &[u8; 32]) -> Option<Vec<(usize, usize)>> {
    let literal = parse_character_literal(header, 0x1000)?;
    let mut hints = Vec::new();
    hint_character_literal(&literal, |address, len| hints.push((address, len)));
    Some(hints)
}

#[test]
fn staged_name_hints_require_both_valid_headers_and_keep_inline_values_owned() {
    let _globals = crate::extended::characters::test_globals();
    let mut job = CharactersJob {
        log: collector_raw::raw::RawLog::new(4, 256),
        ..Default::default()
    };
    let mut view = ViewCopy::default();
    let mut headers = [0; 64];
    headers[..32].copy_from_slice(&heap_header(0x2038, 20, 20));
    headers[32..].copy_from_slice(&heap_header(0x3000, 0, 16));
    store_field(&mut job, &mut view, 3, &headers);
    let mut source = (job, view);
    assert_eq!(
        targets(&source, &Dead::default()),
        [(0x204c, 1), (0x2038, 1), (0x3000, 1)]
    );
    assert!(
        targets(
            &source,
            &Dead {
                characters: true,
                ..Default::default()
            }
        )
        .is_empty()
    );
    headers[56..64].copy_from_slice(&14_u64.to_le_bytes());
    store_field(&mut source.0, &mut source.1, 3, &headers);
    assert!(
        targets(&source, &Dead::default()).is_empty(),
        "invalid second header must suppress both hints"
    );
    headers.fill(0);
    headers[24..32].copy_from_slice(&15_u64.to_le_bytes());
    headers[56..64].copy_from_slice(&15_u64.to_le_bytes());
    store_field(&mut source.0, &mut source.1, 3, &headers);
    assert!(targets(&source, &Dead::default()).is_empty());
}

#[test]
fn character_target_hints_obey_independent_dead_guards_and_empty_maps() {
    let _globals = crate::extended::characters::test_globals();
    for bits in 0..32 {
        let dead = Dead {
            characters: bits & 1 != 0,
            leaders: bits & 2 != 0,
            advisors: bits & 4 != 0,
            units: bits & 8 != 0,
            traits: bits & 16 != 0,
        };
        for count in [0, 1, u64::from(u32::MAX)] {
            let mut expected = Vec::new();
            if !dead.leaders {
                expected.push((0x2000, 1));
            }
            if !dead.advisors {
                expected.push((0x3000, 1));
            }
            if !(dead.units && dead.traits) {
                expected.push((0x4000, 1));
            }
            if !dead.units {
                expected.extend([(0x4040, 1), (0x4e60, 1), (0x4f48, 1), (0x4ed4, 1)]);
            }
            assert_eq!(
                targets(&view(0x2000, count, 0x3000, count, 0x4000), &dead),
                expected
            );
        }
    }
    assert!(targets(&view(0, 0, 0, 0, 0), &Dead::default()).is_empty());
    assert_eq!(
        targets(&view(0x2000, u64::MAX, 0x3000, 0, 0x4000), &Dead::default()),
        [
            (0x3000, 1),
            (0x4000, 1),
            (0x4040, 1),
            (0x4e60, 1),
            (0x4f48, 1),
            (0x4ed4, 1)
        ]
    );
    assert_eq!(
        targets(&view(0x2000, 0, 0x2000, 0, 0x2000), &Dead::default()),
        [
            (0x2000, 1),
            (0x2000, 1),
            (0x2000, 1),
            (0x2040, 1),
            (0x2e60, 1),
            (0x2f48, 1),
            (0x2ed4, 1)
        ]
    );
}

#[test]
fn character_target_hints_reject_null_overflow_and_canonical_gap_ranges() {
    let _globals = crate::extended::characters::test_globals();
    let hint = |address, len| {
        let mut result = Vec::new();
        hint_character_target(address, len, &mut |at, width| result.push((at, width)));
        result
    };
    for (address, len) in [
        (0, 16),
        (usize::MAX, 16),
        (usize::MAX - 15, 16),
        (0x0000_8000_0000_0000, 16),
        (0xffff_7fff_ffff_ffff, 16),
        (0x0000_7fff_ffff_fff0, 26),
    ] {
        assert!(hint(address, len).is_empty(), "{address:x}+{len}");
    }
    for (address, len) in [
        (0x200f, 26),
        (0x0000_7fff_ffff_ffe0, 26),
        (0xffff_8000_0000_0000, 16),
        (usize::MAX - 16, 16),
    ] {
        assert_eq!(hint(address, len), [(address, 1)]);
    }
    for field in [4, 5, 6] {
        let mut source = view(0x2000, 0, 0x3000, 0, 0x4000);
        let start = source.1.starts[field] as usize;
        source.0.log.bytes[start..start + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        let hinted = targets(&source, &Dead::default());
        assert_eq!(hinted.len(), if field == 5 { 2 } else { 6 });
        assert!(!hinted.iter().any(|&(address, _)| address == usize::MAX));
    }
}

#[test]
fn character_heap_hints_cover_only_first_and_terminator_lines() {
    let _globals = crate::extended::characters::test_globals();
    for alignment in 0..64 {
        for len in [0, 1, 15, 16, 17, 63, 64, 65, 128] {
            let source = 0x2000 + alignment;
            let hints = literal_hints(&heap_header(source, len, len.max(16))).unwrap();
            let end = source + len;
            let mut expected = vec![(end, 1)];
            if len != 0 && source / 64 != end / 64 {
                expected.push((source, 1));
            }
            assert_eq!(hints, expected, "alignment={alignment}, len={len}");
        }
    }
    let mut inline = [0u8; 32];
    inline[..3].copy_from_slice(b"abc");
    inline[16..24].copy_from_slice(&3u64.to_le_bytes());
    inline[24..32].copy_from_slice(&15u64.to_le_bytes());
    assert_eq!(literal_hints(&inline), Some(Vec::new()));
    inline[3] = 1;
    assert!(literal_hints(&inline).is_none());
    for header in [
        heap_header(0, 17, 17),
        heap_header(0x2000, 17, 16),
        heap_header(0x2000, 0, 14),
    ] {
        assert!(literal_hints(&header).is_none());
    }
    for header in [
        heap_header(usize::MAX - 15, 16, 16),
        heap_header(0x0000_7fff_ffff_fff0, 17, 17),
        heap_header(0x0000_8000_0000_0000, 17, 17),
    ] {
        assert_eq!(literal_hints(&header), Some(Vec::new()));
    }
}

struct TraceMemory<'a> {
    memory: engine_native_memory::memory::SliceMemory<'a>,
    failed_call: Option<usize>,
    calls: RefCell<Vec<(usize, usize)>>,
}

impl Memory for TraceMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let index = self.calls.borrow().len();
        self.calls.borrow_mut().push((address, output.len()));
        if self.failed_call == Some(index) {
            output.fill(0xe7);
            return false;
        }
        self.memory.copy(address, output)
    }
}

#[test]
fn character_fresh_view_hints_observe_no_targets_and_ignore_failed_fields() {
    let _globals = crate::extended::characters::test_globals();
    use super::tests::{ADDRESS, CHARACTERS, IMAGE, UNITS, fixture};
    let bytes = fixture();
    let pointer = ADDRESS + CHARACTERS[0];
    for failed_call in [
        None,
        Some(0),
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(5),
        Some(6),
        Some(7),
    ] {
        let memory = TraceMemory {
            memory: engine_native_memory::memory::SliceMemory {
                base: ADDRESS,
                bytes: &bytes,
            },
            failed_call,
            calls: RefCell::new(Vec::new()),
        };
        let mut job = CharactersJob {
            log: collector_raw::raw::RawLog::new(16, 256),
            game_state: ADDRESS,
            active: [true; N],
            ..CharactersJob::default()
        };
        let copied = copy_view(&mut job, &memory, IMAGE, pointer);
        let calls = memory.calls.borrow().clone();
        let expected_calls: Vec<_> = VIEW_OFFSETS
            .into_iter()
            .zip(VIEW_LENS)
            .take(if failed_call == Some(0) {
                1
            } else {
                VIEW_OFFSETS.len()
            })
            .map(|(offset, len)| (pointer + offset, len))
            .collect();
        assert_eq!(calls, expected_calls);
        let Some(copied) = copied else {
            assert_eq!(failed_call, Some(0));
            continue;
        };
        let spans = job.log.spans.clone();
        let used = job.log.bytes.as_ref().to_vec();
        let mut expected = Vec::new();
        if failed_call != Some(4) {
            expected.push((ADDRESS + 0x2400, 1));
        }
        if failed_call != Some(6) {
            expected.push((ADDRESS + 0x2430, 1));
        }
        if failed_call != Some(5) {
            expected.push((ADDRESS + UNITS[0], 1));
            expected.extend([
                (ADDRESS + UNITS[0] + 0x40, 1),
                (ADDRESS + UNITS[0] + 0xe60, 1),
                (ADDRESS + UNITS[0] + 0xf48, 1),
                (ADDRESS + UNITS[0] + 0xed4, 1),
            ]);
        }
        let mut hints = Vec::new();
        hint_character_targets(&job, &copied, &Dead::default(), |address, len| {
            hints.push((address, len));
        });
        assert_eq!(hints, expected);
        assert_eq!(*memory.calls.borrow(), calls, "hint observed a target");
        assert_eq!(job.log.spans, spans);
        assert_eq!(job.log.bytes.as_ref(), used);

        let fields: Vec<_> = (0..VIEW_LENS.len())
            .map(|index| copied.field(&job, index).to_vec())
            .collect();
        for (index, field) in fields.iter().enumerate() {
            if failed_call == Some(index) {
                assert_eq!(copied.starts[index], u32::MAX);
                assert_eq!(field, &vec![0; VIEW_LENS[index]]);
            } else {
                let start = CHARACTERS[0] + VIEW_OFFSETS[index];
                assert_ne!(copied.starts[index], u32::MAX);
                assert_eq!(field, &bytes[start..start + VIEW_LENS[index]]);
            }
        }
        let extension = [0xa5; 512];
        let owned = engine_native_memory::memory::SliceMemory {
            base: 0x8000,
            bytes: &extension,
        };
        assert!(job.log.take(&owned, 0x8000, 64, MASK_CHARACTERS).is_some());
        let capacity = job.log.bytes.capacity();
        assert!(job.log.take(&owned, 0x8000, 512, MASK_CHARACTERS).is_some());
        assert!(!job.log.overflowed);
        assert!(job.log.bytes.capacity() > capacity);
        for (index, field) in fields.iter().enumerate() {
            assert_eq!(copied.field(&job, index), field);
        }
    }
    for active in [
        [true, false, false, true, true, true],
        [false, true, false, false, false, false],
    ] {
        let memory = engine_native_memory::memory::SliceMemory {
            base: ADDRESS,
            bytes: &bytes,
        };
        let mut job = CharactersJob {
            log: collector_raw::raw::RawLog::new(16, 256),
            active,
            ..CharactersJob::default()
        };
        let copied = copy_view(&mut job, &memory, IMAGE, pointer).unwrap();
        let source_mask = job.source_mask();
        for index in 0..VIEW_LENS.len() {
            if VIEW_MASKS[index] & source_mask == 0 {
                assert_eq!(copied.starts[index], u32::MAX);
                assert!(copied.field(&job, index).iter().all(|&byte| byte == 0));
            } else {
                let start = CHARACTERS[0] + VIEW_OFFSETS[index];
                assert_eq!(
                    copied.field(&job, index),
                    &bytes[start..start + VIEW_LENS[index]]
                );
            }
        }
    }
    assert_eq!(size_of::<ViewCopy>(), 44);
}

fn original_text(
    job: &mut CharactersJob,
    memory: &impl Memory,
    view: &ViewCopy,
    pointer: usize,
    dead: &mut Dead,
) {
    if dead.characters {
        return;
    }
    let name_addr = pointer.checked_add(ccharacter::NAME).unwrap_or(0);
    let token_addr = pointer.checked_add(ccharacter::TOKEN).unwrap_or(0);
    let headers = view.field(job, 3);
    let name = parse_character_literal(&headers[..32], name_addr);
    let token = parse_character_literal(&headers[32..64], token_addr);
    let (Some(name), Some(token)) = (name, token) else {
        dead.characters = true;
        return;
    };
    if !copy_character_literal_bytes(job, memory, &name, MASK_CHARACTERS)
        || !copy_character_literal_bytes(job, memory, &token, MASK_CHARACTERS)
    {
        dead.characters = true;
    }
}

#[test]
fn character_heap_hints_preserve_exact_observations_and_failure_prefix() {
    let _globals = crate::extended::characters::test_globals();
    let bytes = [0u8; 256];
    let expected_reads = [
        (0x2000 + 17, 1),
        (0x2000, 17),
        (0x2080 + 65, 1),
        (0x2080, 65),
    ];
    for failed_call in [None, Some(0), Some(1), Some(2), Some(3)] {
        for malformed_token in [false, true] {
            for already_dead in [false, true] {
                let mut headers = [0; 64];
                headers[..32].copy_from_slice(&heap_header(0x2000, 17, 17));
                headers[32..64].copy_from_slice(&heap_header(
                    0x2080,
                    65,
                    if malformed_token { 64 } else { 65 },
                ));
                let memory = || TraceMemory {
                    memory: engine_native_memory::memory::SliceMemory {
                        base: 0x2000,
                        bytes: &bytes,
                    },
                    failed_call,
                    calls: RefCell::new(Vec::new()),
                };
                let mut actual = CharactersJob {
                    log: collector_raw::raw::RawLog::new(9, 192),
                    game_state: 0,
                    ..CharactersJob::default()
                };
                let mut expected = CharactersJob {
                    log: collector_raw::raw::RawLog::new(9, 192),
                    game_state: 0,
                    ..CharactersJob::default()
                };
                let mut source = ViewCopy::default();
                let mut expected_source = ViewCopy::default();
                store_field(&mut actual, &mut source, 3, &headers);
                store_field(&mut expected, &mut expected_source, 3, &headers);
                let mut dead = Dead {
                    characters: already_dead,
                    ..Dead::default()
                };
                let mut expected_dead = Dead {
                    characters: already_dead,
                    ..Dead::default()
                };
                let actual_memory = memory();
                let expected_memory = memory();
                copy_character_text(&mut actual, &actual_memory, &source, 0x1000, &mut dead);
                original_text(
                    &mut expected,
                    &expected_memory,
                    &expected_source,
                    0x1000,
                    &mut expected_dead,
                );
                assert_eq!(
                    *actual_memory.calls.borrow(),
                    *expected_memory.calls.borrow()
                );
                let count = if malformed_token || already_dead {
                    0
                } else {
                    failed_call.map_or(4, |index| index + 1)
                };
                assert_eq!(
                    actual_memory.calls.borrow().as_slice(),
                    &expected_reads[..count]
                );
                assert_eq!(actual.log.spans, expected.log.spans);
                assert_eq!(actual.log.bytes.as_ref(), expected.log.bytes.as_ref());
                assert_eq!(actual.log.fault, expected.log.fault);
                assert_eq!(actual.log.raw_bytes, expected.log.raw_bytes);
                assert_eq!(actual.log.overflowed, expected.log.overflowed);
                assert_eq!(dead.characters, expected_dead.characters);
            }
        }
    }
}
