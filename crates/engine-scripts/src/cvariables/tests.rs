use super::*;
use engine_native_memory::memory::SliceMemory;

const BASE: usize = 0x1400_0000;
const CONTAINER: usize = BASE + 0x100;
const STORAGE_ADDRESS: usize = BASE + 0x200;
const TAIL: usize = 5;
const SLOTS: usize = 8 + TAIL;

fn fixture(count: u32) -> Vec<u8> {
    let mut bytes = vec![0; 0x1000];
    put64(&mut bytes, 0x100, (BASE + VTABLE_RVA.0) as u64);
    put64(&mut bytes, 0x118, STORAGE_ADDRESS as u64);
    put32(&mut bytes, 0x120, count);
    put32(&mut bytes, 0x124, 7);
    bytes[0x128] = TAIL as u8;
    bytes[0x200 + SLOTS * variable_hash_node::STRIDE + variable_hash_node::DISTANCE] = 0xff;
    bytes
}

fn put32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn node(bytes: &mut [u8], index: usize, hash: u32, key: &[u8], value: i64) {
    let offset = 0x200 + index * variable_hash_node::STRIDE;
    put32(bytes, offset, hash);
    bytes[offset + variable_hash_node::DISTANCE] = (index - (hash & 7) as usize + 1) as u8;
    put64(bytes, offset + 0x18, key.len() as u64);
    if key.len() <= 15 {
        bytes[offset + variable_hash_node::KEY..offset + variable_hash_node::KEY + key.len()]
            .copy_from_slice(key);
        put64(bytes, offset + 0x20, 15);
    } else {
        put64(
            bytes,
            offset + variable_hash_node::KEY,
            (BASE + 0x900) as u64,
        );
        bytes[0x900..0x900 + key.len()].copy_from_slice(key);
        put64(bytes, offset + 0x20, key.len() as u64);
    }
    put64(bytes, offset + variable_hash_node::VALUE, value as u64);
}

fn run(bytes: &[u8], output: &mut Vec<Row>, names: &mut Vec<u8>) -> NameWalk {
    walk(
        &SliceMemory { base: BASE, bytes },
        BASE,
        CONTAINER,
        output,
        names,
    )
}

#[test]
fn growing_variable_rows_and_key_bytes_capture_first_and_later_mod_sized_maps() {
    let mut small = fixture(2);
    node(&mut small, 0, 0, b"a", i64::MIN);
    node(&mut small, 1, 1, b"b", i64::MAX);
    let mut rows = Vec::new();
    let mut names = Vec::new();
    let mut expected_rows = vec![Row::default(); 2];
    let mut expected_names = vec![0; 2];
    let expected = run(&small, &mut expected_rows, &mut expected_names);
    assert_eq!(expected.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        walk(
            &SliceMemory {
                base: BASE,
                bytes: &small
            },
            BASE,
            CONTAINER,
            &mut rows,
            &mut names
        ),
        expected
    );
    assert_eq!(&rows[..2], &expected_rows);
    assert_eq!(&names[..2], &expected_names);
    const COUNT: usize = 5_000;
    const BUCKETS: usize = 8_192;
    const TAIL: usize = 15;
    let mut bytes = fixture(COUNT as u32);
    bytes.resize(0x200 + (BUCKETS + TAIL + 1) * variable_hash_node::STRIDE, 0);
    bytes[0x200..].fill(0);
    put32(&mut bytes, 0x124, BUCKETS as u32 - 1);
    bytes[0x128] = TAIL as u8;
    bytes[0x200 + (BUCKETS + TAIL) * variable_hash_node::STRIDE + variable_hash_node::DISTANCE] =
        0xff;
    for index in 0..COUNT {
        let at = 0x200 + index * variable_hash_node::STRIDE;
        let key = format!("key_{index:04}");
        put32(&mut bytes, at, index as u32);
        bytes[at + variable_hash_node::DISTANCE] = 1;
        bytes[at + variable_hash_node::KEY..at + variable_hash_node::KEY + key.len()]
            .copy_from_slice(key.as_bytes());
        put64(&mut bytes, at + 0x18, key.len() as u64);
        put64(&mut bytes, at + 0x20, 15);
        put64(&mut bytes, at + variable_hash_node::VALUE, index as u64);
    }
    let mut expected_rows = vec![Row::default(); COUNT];
    let mut expected_names = vec![0; COUNT * 8];
    let expected = run(&bytes, &mut expected_rows, &mut expected_names);
    assert_eq!(expected.outcome, Outcome::Rows { len: COUNT });
    assert_eq!(
        (expected.bytes_len, expected.required_bytes),
        (COUNT * 8, COUNT * 8)
    );
    assert_eq!(
        walk(
            &SliceMemory {
                base: BASE,
                bytes: &bytes
            },
            BASE,
            CONTAINER,
            &mut rows,
            &mut names
        ),
        expected
    );
    assert_eq!(&rows[..COUNT], expected_rows.as_slice());
    assert_eq!(&names[..expected.bytes_len], expected_names.as_slice());
}

#[test]
fn growing_variable_scopes_extend_all_three_buffers_on_the_first_pass() {
    let bytes = scope_fixture();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut expected_scopes = vec![ScopeRow::default(); 5];
    let mut expected_entries = vec![Row::default(); 4];
    let mut expected_names = vec![0; 6];
    let expected = walk_scopes(
        &memory,
        BASE,
        BASE + 0x1000,
        &mut expected_scopes,
        &mut expected_entries,
        &mut expected_names,
    );
    assert_eq!(expected.outcome, Outcome::Rows { len: 5 });
    let mut scopes = Vec::new();
    let mut entries = Vec::new();
    let mut names = Vec::new();
    assert_eq!(
        walk_scopes(
            &memory,
            BASE,
            BASE + 0x1000,
            &mut scopes,
            &mut entries,
            &mut names
        ),
        expected
    );
    assert_eq!(&scopes[..5], &expected_scopes);
    assert_eq!(&entries[..4], &expected_entries);
    assert_eq!(&names[..6], &expected_names);
}

#[test]
fn holes_probe_overflow_and_literal_bytes_preserve_exact_values() {
    let mut bytes = fixture(3);
    node(&mut bytes, 2, 2, b"", i64::MIN);
    node(&mut bytes, 7, 7, b"\xff\0x", -100_001);
    node(&mut bytes, 8, 7, b"long_variable_key_bytes", i64::MAX);
    let stale = 0x200 + variable_hash_node::STRIDE;
    bytes[stale + variable_hash_node::KEY..stale + variable_hash_node::KEY + 3]
        .copy_from_slice(b"old");
    put64(&mut bytes, stale + 0x18, 3);
    put64(&mut bytes, stale + 0x20, 15);
    let mut output = vec![Row::default(); 3];
    let mut names = vec![0; 64];
    let result = run(&bytes, &mut output, &mut names);
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!(result.bytes_len, 26);
    assert_eq!(&names[..3], b"\xff\0x");
    assert_eq!(&names[3..26], b"long_variable_key_bytes");
    assert_eq!(
        output.iter().map(|row| row.value_raw).collect::<Vec<_>>(),
        [i64::MIN, -100_001, i64::MAX]
    );
    assert_eq!(output[0].name_len, 0);
}

#[test]
fn rows_grow_and_the_count_probe_reports_required_key_bytes() {
    let mut bytes = fixture(3);
    node(&mut bytes, 0, 0, b"alpha", 1);
    node(&mut bytes, 1, 1, b"long_variable_key_bytes", 2);
    node(&mut bytes, 2, 2, b"z", 3);
    let mut output = vec![Row::default(); 2];
    let mut names = vec![0; 6];
    let result = run(&bytes, &mut output, &mut names);
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!((result.bytes_len, result.required_bytes), (29, 29));
    let result = walk_buffer(
        &SliceMemory { base: BASE, bytes: &bytes },
        BASE,
        CONTAINER,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
    );
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 3
        }
    );
    assert_eq!(result.required_bytes, 29);
}

#[test]
fn declared_count_probe_law_and_end_sentinel_are_all_checked() {
    let mut bytes = fixture(1);
    node(&mut bytes, 3, 3, b"x", 7);
    let original = bytes.clone();
    let mut output = vec![Row::default(); 2];
    let mut names = vec![0; 16];
    for (offset, replacement) in [
        (0x120, 0),
        (0x120, 2),
        (0x124, 6),
        (0x128, 4),
        (
            0x200 + SLOTS * variable_hash_node::STRIDE + variable_hash_node::DISTANCE,
            0,
        ),
        (
            0x200 + 3 * variable_hash_node::STRIDE + variable_hash_node::DISTANCE,
            2,
        ),
        (
            0x200 + 3 * variable_hash_node::STRIDE + variable_hash_node::DISTANCE,
            0xff,
        ),
    ] {
        bytes.copy_from_slice(&original);
        bytes[offset] = replacement;
        assert!(matches!(
            run(&bytes, &mut output, &mut names).outcome,
            Outcome::Unavailable { .. }
        ));
    }
}

#[test]
fn empty_shared_storage_needs_no_node_memory() {
    let mut bytes = fixture(0);
    put64(&mut bytes, 0x118, (BASE + EMPTY_STORAGE_RVA.0) as u64);
    put32(&mut bytes, 0x124, 0);
    bytes[0x128] = 0;
    assert_eq!(
        run(&bytes[..0x129], &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Rows { len: 0 }
    );
    put32(&mut bytes, 0x120, 1);
    assert!(matches!(
        run(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn scoped_walk_keeps_alias_identity_literal_keys_and_authenticated_empty_scopes() {
    let mut bytes = scope_fixture();
    let seeds = [0, u32::MAX, 1, 0x8000_0000, 9];
    for (container, seed) in [
        0xa000,
        0xb000,
        0xc000,
        0x8000 + ccharacter::VARIABLES,
        0x9000 + ccharacter::VARIABLES,
    ]
    .into_iter()
    .zip(seeds)
    {
        put32(&mut bytes, container + SEED, seed);
    }
    let mut scopes = vec![ScopeRow::default(); 5];
    let mut entries = vec![Row::default(); 4];
    let mut names = vec![0; 6];
    let captured = walk_scopes(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        BASE,
        BASE + 0x1000,
        &mut scopes,
        &mut entries,
        &mut names,
    );
    assert_eq!(
        captured,
        ScopeWalk {
            outcome: Outcome::Rows { len: 5 },
            entries_len: 4,
            required_entries: 4,
            bytes_len: 6,
            required_bytes: 6
        }
    );
    assert_eq!(
        scopes.iter().map(|row| row.scope).collect::<Vec<_>>(),
        [
            Scope::Global,
            Scope::Country(2),
            Scope::State(1),
            Scope::Character(44),
            Scope::Character(45)
        ]
    );
    assert_eq!(scopes[2].entries_len, 0);
    assert_eq!(scopes[4].entries_len, 0);
    assert_eq!(scopes.iter().map(|row| row.seed).collect::<Vec<_>>(), seeds.map(Some));
    assert_eq!(
        entries.iter().map(|row| row.value_raw).collect::<Vec<_>>(),
        [i64::MIN, i64::MAX, -100001, 300_000]
    );
    assert_eq!(names, [b'g', b'g', 0xff, 0, b'x', b'c']);
    assert_eq!(entries.iter().map(|row| row.name_start).collect::<Vec<_>>(), [0, 1, 2, 5]);
}

#[test]
fn the_scope_count_counts_every_scope_and_a_bad_later_scope_invalidates_everything() {
    let mut bytes = scope_fixture();
    let mut scopes = vec![ScopeRow::default(); 5];
    let mut entries = vec![Row::default(); 4];
    let mut names = vec![0; 6];
    let captured = count_scopes(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        BASE,
        BASE + 0x1000,
    );
    assert_eq!(
        captured.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 5
        }
    );
    assert_eq!((captured.required_entries, captured.required_bytes), (4, 6));
    put64(&mut bytes, 0x7000 + cstate::VARIABLES, 0);
    let captured = walk_scopes(
        &SliceMemory {
            base: BASE,
            bytes: &bytes,
        },
        BASE,
        BASE + 0x1000,
        &mut scopes,
        &mut entries,
        &mut names,
    );
    assert_eq!(
        captured.outcome,
        Outcome::Unavailable {
            reason: "script variable scope root is null"
        }
    );
    assert_eq!((captured.entries_len, captured.bytes_len), (0, 0));
}

fn scope_fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0xd000];
    for (offset, data, count) in [
        (cgame_state::COUNTRIES, 0x2000, 3),
        (cgame_state::COUNTRY_LINKS, 0x2100, 3),
        (cgame_state::STATES, 0x2200, 3),
    ] {
        put64(&mut bytes, 0x1000 + offset, (BASE + data) as u64);
        put32(&mut bytes, 0x1000 + offset + 8, count);
        put32(&mut bytes, 0x1000 + offset + 12, count);
    }
    for slot in [1, 2] {
        put64(&mut bytes, 0x2000 + slot * 8, (BASE + 0x4000) as u64);
        put32(&mut bytes, 0x2100 + slot * 4, 1);
    }
    put64(&mut bytes, 0x4000, (BASE + ccountry::VTABLE_RVA.0) as u64);
    put32(&mut bytes, 0x4000 + ccountry::ID, 2);
    put64(&mut bytes, 0x2208, (BASE + 0x7000) as u64);
    put64(&mut bytes, 0x7000, (BASE + cstate::VTABLE_RVA.0) as u64);
    put32(&mut bytes, 0x7000 + cstate::ID, 1);
    for (root, offset, container, count) in [
        (0x1000, cgame_state::GLOBAL_VARIABLES, 0xa000, 1),
        (0x4000, ccountry::VARIABLES, 0xb000, 2),
        (0x7000, cstate::VARIABLES, 0xc000, 0),
    ] {
        let mut block = fixture(count);
        if count != 0 {
            node(
                &mut block,
                0,
                0,
                b"g",
                if count == 1 { i64::MIN } else { i64::MAX },
            );
        }
        if count == 2 {
            node(&mut block, 1, 1, b"\xff\0x", -100001);
        }
        bytes[container - 0x100..container + 0xf00].copy_from_slice(&block);
        put64(&mut bytes, root + offset, (BASE + container) as u64);
        put64(
            &mut bytes,
            container + STORAGE,
            (BASE + container + 0x100) as u64,
        );
    }
    let container = 0x8000 + ccharacter::VARIABLES;
    let mut block = fixture(1);
    node(&mut block, 0, 0, b"c", 300_000);
    bytes[container - 0x100..container + 0xf00].copy_from_slice(&block);
    put64(
        &mut bytes,
        container + STORAGE,
        (BASE + container + 0x100) as u64,
    );
    let empty = 0x9000 + ccharacter::VARIABLES;
    put64(&mut bytes, empty, (BASE + VTABLE_RVA.0) as u64);
    put64(
        &mut bytes,
        empty + STORAGE,
        (BASE + EMPTY_STORAGE_RVA.0) as u64,
    );
    put64(
        &mut bytes,
        0x1000 + cgame_state::CHARACTER_MANAGER,
        (BASE + 0x5000) as u64,
    );
    put64(
        &mut bytes,
        0x5000,
        (BASE + ccharacter_manager::VTABLE_RVA.0) as u64,
    );
    let historical = 0x5000 + ccharacter_manager::HISTORICAL_CHARACTERS;
    put64(&mut bytes, historical, (BASE + 0x5100) as u64);
    put32(&mut bytes, historical + 8, 2);
    put32(&mut bytes, historical + 12, 2);
    for (index, (character, id)) in [(0x8000_usize, 44_u32), (0x9000, 45)]
        .into_iter()
        .enumerate()
    {
        put64(&mut bytes, 0x5100 + index * 8, (BASE + character) as u64);
        put64(
            &mut bytes,
            character,
            (BASE + ccharacter::VTABLE_RVA.0) as u64,
        );
        put32(
            &mut bytes,
            character + ccharacter::TYPE,
            ccharacter::REFERENCE_TYPE,
        );
        put32(&mut bytes, character + ccharacter::ID, id);
    }
    bytes
}

#[test]
fn bad_names_and_faults_do_not_publish_a_partial_scope() {
    let mut bytes = fixture(2);
    node(&mut bytes, 0, 0, b"good", 7);
    node(&mut bytes, 1, 1, b"long_variable_key_bytes", 8);
    let mut output = vec![Row::default(); 2];
    let mut names = vec![0; 64];
    let failed = run(&bytes[..0x900], &mut output, &mut names);
    assert!(matches!(failed.outcome, Outcome::Unavailable { .. }));
    assert_eq!((failed.bytes_len, failed.required_bytes), (0, 0));
    bytes[0x900 + 23] = 1;
    assert!(matches!(
        run(&bytes, &mut output, &mut names).outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn scoped_seed_failure_preserves_entries_and_distinguishes_unknown_from_zero() {
    struct SeedFault<'a>(SliceMemory<'a>);
    impl Memory for SeedFault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == BASE + 0xb000 + SEED && destination.len() == 4 {
                false
            } else {
                self.0.copy(address, destination)
            }
        }
    }
    let bytes = scope_fixture();
    let mut scopes = vec![ScopeRow::default(); 5];
    let mut entries = vec![Row::default(); 4];
    let mut names = vec![0; 6];
    let captured = walk_scopes(
        &SeedFault(SliceMemory {
            base: BASE,
            bytes: &bytes,
        }),
        BASE,
        BASE + 0x1000,
        &mut scopes,
        &mut entries,
        &mut names,
    );
    assert_eq!(captured.outcome, Outcome::Rows { len: 5 });
    assert_eq!(captured.entries_len, 4);
    assert_eq!(scopes[0].seed, Some(0));
    assert_eq!(scopes[1].seed, None);
    assert_eq!(scopes[1].entries_len, 2);
    assert_eq!(entries[1].value_raw, i64::MAX);
    assert_eq!(names, [b'g', b'g', 0xff, 0, b'x', b'c']);
}

struct Calls<'a> {
    bytes: &'a [u8],
    denied: Option<usize>,
    calls: std::cell::RefCell<Vec<usize>>,
}

impl Calls<'_> {
    fn new(bytes: &[u8], denied: Option<usize>) -> Calls<'_> {
        Calls {
            bytes,
            denied,
            calls: std::cell::RefCell::default(),
        }
    }

    fn span(&self, address: usize, output: &mut [u8]) -> bool {
        !self
            .denied
            .is_some_and(|at| (address..address + output.len()).contains(&at))
            && SliceMemory {
                base: BASE,
                bytes: self.bytes,
            }
            .copy(address, output)
    }

    fn within(&self, start: usize, len: usize) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|address| (BASE + start..BASE + start + len).contains(address))
            .count()
    }
}

impl Memory for Calls<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.borrow_mut().push(address);
        self.span(address, output)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.borrow_mut().push(addresses[0]);
        for (index, (address, output)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if !output.is_empty() && !self.span(address, output) {
                return index;
            }
        }
        N
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        if count == 0 {
            return 0;
        }
        let mut completed = 0;
        for row in 0..sources.len() {
            let Some(pointer) = sources.pointer(row).filter(|&pointer| pointer != 0) else {
                return completed;
            };
            self.calls.borrow_mut().push(pointer);
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = pointer
                        .checked_add(field.source_offset)
                        .filter(|&address| address.checked_add(field.len).is_some())
                    else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.span(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }
}

#[test]
fn scope_words_and_character_identity_share_one_read_with_sequential_verdicts() {
    let bytes = scope_fixture();
    let walk = |denied| {
        let memory = Calls::new(&bytes, denied);
        let mut scopes = vec![ScopeRow::default(); 5];
        let mut entries = vec![Row::default(); 4];
        let mut names = vec![0; 6];
        let captured = walk_scopes(
            &memory,
            BASE,
            BASE + 0x1000,
            &mut scopes,
            &mut entries,
            &mut names,
        );
        (captured.outcome, scopes, memory)
    };
    let (outcome, _, memory) = walk(None);
    assert_eq!(outcome, Outcome::Rows { len: 5 });
    assert_eq!(memory.within(0x9000, 0x1000), 1);
    assert_eq!(memory.within(0x8000, 0x1000), 3);
    assert_eq!(memory.within(0xb000, 0x1000), 3);
    let character = 0x9000 + ccharacter::VARIABLES;
    for (denied, reason) in [
        (0x9000, "character identity unreadable"),
        (character, "variables vtable mismatch"),
        (character + STORAGE, "variables header unreadable"),
        (0xb000 + STORAGE + 16, "variables header unreadable"),
    ] {
        assert_eq!(walk(Some(BASE + denied)).0, Outcome::Unavailable { reason });
    }
    for (container, scope) in [(character, 4), (0xb000, 1)] {
        let (outcome, scopes, _) = walk(Some(BASE + container + SEED));
        assert_eq!(outcome, Outcome::Rows { len: 5 });
        assert_eq!(scopes[scope].seed, None);
        assert_eq!(scopes.iter().filter(|row| row.seed.is_none()).count(), 1);
    }
}

#[test]
fn heap_key_terminator_and_payload_share_one_read_with_sequential_verdicts() {
    let mut bytes = fixture(2);
    node(&mut bytes, 0, 0, b"good", 7);
    node(&mut bytes, 1, 1, b"long_variable_key_bytes", 8);
    let heap = 0x900;
    let len = b"long_variable_key_bytes".len();
    fn walk_traced<'a>(
        bytes: &'a [u8],
        denied: Option<usize>,
        names: &mut Vec<u8>,
    ) -> (NameWalk, Calls<'a>) {
        let memory = Calls::new(bytes, denied);
        let mut output = vec![Row::default(); 2];
        let captured = walk(&memory, BASE, CONTAINER, &mut output, names);
        (captured, memory)
    }
    let walk = walk_traced;
    let mut names = vec![0; 64];
    let (captured, memory) = walk(&bytes, None, &mut names);
    assert_eq!(captured.outcome, Outcome::Rows { len: 2 });
    assert_eq!(&names[4..4 + len], b"long_variable_key_bytes");
    assert_eq!(memory.within(heap, 0x100), 1);
    for (denied, reason) in [
        (heap + len, "variable name terminator invalid or unreadable"),
        (heap, "variable name bytes unreadable"),
    ] {
        let (captured, _) = walk(&bytes, Some(BASE + denied), &mut names);
        assert_eq!(captured.outcome, Outcome::Unavailable { reason });
    }
    bytes[heap + len] = 1;
    let (captured, _) = walk(&bytes, None, &mut names);
    assert_eq!(
        captured.outcome,
        Outcome::Unavailable {
            reason: "variable name terminator invalid or unreadable"
        }
    );
}
