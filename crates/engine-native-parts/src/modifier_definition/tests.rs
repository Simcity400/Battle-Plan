use engine_native_memory::modifier_cache::PassDefinitions;
use super::*;
use std::collections::BTreeMap;

#[test]
fn checked_definition_memo_capacity_overflow_returns_oom() {
    let error = match DefinitionMemo::try_with_capacity(usize::MAX) {
        Ok(_) => panic!("impossible memo capacity accepted"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), std::io::ErrorKind::OutOfMemory);
}

const BASE: usize = 0x140000000;
const TABLE: usize = 0x2000;
const KEY: u32 = 0x54;

fn key(memory: &impl Memory, name: &[u8]) -> Result<Option<u32>, &'static str> {
    keys_by_names_at(memory, BASE, &[name]).map(|(keys, _)| keys[0])
}

#[derive(Default)]
struct Fixture {
    bytes: BTreeMap<usize, u8>,
}
impl Fixture {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.bytes
            .extend(bytes.iter().enumerate().map(|(i, &b)| (address + i, b)));
    }
    fn row(key: u32) -> usize {
        TABLE
            .checked_add_signed(key as i32 as isize * STRIDE as isize)
            .unwrap()
    }
    fn new(key: u32, selector: u32, flags: u32) -> Self {
        let mut result = Self::default();
        result.put(BASE + TABLE_RVA.0, &(TABLE as u64).to_le_bytes());
        result.put(Self::row(key) + FEATURE_SELECTOR, &selector.to_le_bytes());
        result.put(Self::row(key) + FLAGS, &flags.to_le_bytes());
        result
    }
    fn callback(&mut self, pointer: u64, flags: u32) {
        self.put(BASE + ELIGIBILITY_CALLBACK_RVA.0, &pointer.to_le_bytes());
        self.put(BASE + TRADE_FEATURE_FLAGS_RVA.0, &flags.to_le_bytes());
    }
}
impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (i, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.bytes.get(&(address + i)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

#[test]
fn effective_preserves_raw_or_clamps_without_category_or_callback_reads() {
    for key in [KEY, u32::MAX] {
        for flags in [0, CLAMP_MAXIMUM, u32::MAX] {
            let memory = Fixture::new(key, 0, flags);
            for raw in [i64::MIN, -1, 0, 99999, 100000, 100001, i64::MAX] {
                let expected = if flags & CLAMP_MAXIMUM == 0 {
                    raw
                } else {
                    raw.clamp(0, 100000)
                };
                assert_eq!(effective(&memory, BASE, key, raw), Ok(expected));
            }
        }
    }
}

#[test]
fn effective_uses_only_authenticated_feature_flags_and_null_disables() {
    let mut memory = Fixture::new(KEY, 0x80000004, CLAMP_MAXIMUM);
    memory.callback(0, u32::MAX);
    assert_eq!(effective(&memory, BASE, KEY, 123456), Ok(0));
    for flags in [0, 2, 4, 0x80000000] {
        memory.callback((BASE + ELIGIBILITY_FLAGS_GETTER_RVA.0) as u64, flags);
        assert_eq!(
            effective(&memory, BASE, KEY, 123456),
            Ok(if flags & 0x80000004 == 0 { 0 } else { 100000 })
        );
    }
    memory.callback((BASE + ELIGIBILITY_FLAGS_GETTER_RVA.0 + 1) as u64, u32::MAX);
    assert_eq!(
        effective(&memory, BASE, KEY, 123456),
        Err("modifier eligibility callback unsupported")
    );
    memory.callback(0, 0);
    memory.bytes.remove(&(Fixture::row(KEY) + FLAGS));
    assert_eq!(effective(&memory, BASE, KEY, 123456), Ok(0));
}

#[test]
fn effective_first_faults_never_become_a_derived_value() {
    for address in [
        BASE + TABLE_RVA.0,
        Fixture::row(KEY) + FEATURE_SELECTOR,
        Fixture::row(KEY) + FLAGS,
        BASE + ELIGIBILITY_CALLBACK_RVA.0,
        BASE + TRADE_FEATURE_FLAGS_RVA.0,
    ] {
        let mut memory = Fixture::new(KEY, 4, CLAMP_MAXIMUM);
        memory.callback((BASE + ELIGIBILITY_FLAGS_GETTER_RVA.0) as u64, 4);
        memory.bytes.remove(&address);
        assert!(effective(&memory, BASE, KEY, 1).is_err());
    }
    let mut memory = Fixture::new(KEY, 0, 0);
    memory.put(BASE + TABLE_RVA.0, &0_u64.to_le_bytes());
    assert!(effective(&memory, BASE, KEY, 1).is_err());
    assert!(effective(&memory, usize::MAX, KEY, 1).is_err());
}

type Definition = (u32, u32, &'static [u8], &'static [u8], &'static [u8]);
const HEAP: usize = 0x8000;
const NAMES: usize = 0xa000;

struct Table {
    memory: Fixture,
    heap: usize,
}

impl Table {
    fn string(&mut self, address: usize, value: &[u8]) {
        let mut storage = [0_u8; 16];
        let capacity = if value.len() <= 15 {
            storage[..value.len()].copy_from_slice(value);
            15_u64
        } else {
            storage[..8].copy_from_slice(&(self.heap as u64).to_le_bytes());
            self.memory.put(self.heap, value);
            self.memory.put(self.heap + value.len(), &[0]);
            self.heap += value.len() + 1;
            value.len() as u64 | 31
        };
        self.memory.put(address, &storage);
        self.memory
            .put(address + 16, &(value.len() as u64).to_le_bytes());
        self.memory.put(address + 24, &capacity.to_le_bytes());
    }

    fn new(rows: &[Option<Definition>]) -> Self {
        let mut table = Self {
            memory: Fixture::default(),
            heap: HEAP,
        };
        let count = rows.len() as u32;
        table.header(TABLE, count + 3, count);
        table.memory.put(
            BASE + ELIGIBILITY_CALLBACK_RVA.0,
            &((BASE + ELIGIBILITY_FLAGS_GETTER_RVA.0) as u64).to_le_bytes(),
        );
        for (position, row) in rows.iter().enumerate() {
            let address = TABLE + position * STRIDE;
            table.memory.put(address, &[0; STRIDE]);
            let (index, token, name, postfix, argument) = row.unwrap_or((0, 0, b"", b"", b""));
            table.string(address + NAME_KEY, name);
            table.string(address + POSTFIX_KEY, postfix);
            table.string(address + ARGUMENT, argument);
            let words = match row {
                Some(_) => [3, 8 + index, 0, index, token, 2],
                None => [0, u32::MAX, 0, 0, 0, 0],
            };
            for (offset, value) in [
                FLAGS,
                CATEGORY_MASK,
                FEATURE_SELECTOR,
                INDEX,
                TOKEN,
                PRECISION,
            ]
            .into_iter()
            .zip(words)
            {
                table.memory.put(address + offset, &value.to_le_bytes());
            }
        }
        table
    }

    fn header(&mut self, data: usize, capacity: u32, count: u32) {
        let mut header = [0_u8; HEADER_LEN];
        header[..8].copy_from_slice(&(data as u64).to_le_bytes());
        header[8..12].copy_from_slice(&capacity.to_le_bytes());
        header[12..].copy_from_slice(&count.to_le_bytes());
        self.memory.put(BASE + TABLE_RVA.0, &header);
    }

    fn lexer(&mut self, names: &[&[u8]]) {
        let mut header = [0_u8; 64];
        let at = |rva: usize| rva - cpdx_lexer::INITIALIZED_RVA.0;
        header[0] = 1;
        let maximum = names.len() as i32 - 1;
        for (rva, value) in [
            (cpdx_lexer::MAX_TOKEN_ID_RVA.0, maximum),
            (cpdx_lexer::STATIC_MAX_TOKEN_ID_RVA.0, maximum),
            (cpdx_lexer::TOKEN_ALLOCATION_RVA.0, names.len() as i32),
            (cpdx_lexer::TOKEN_CAPACITY_RVA.0, names.len() as i32),
        ] {
            header[at(rva)..at(rva) + 4].copy_from_slice(&value.to_le_bytes());
        }
        let table = at(cpdx_lexer::TOKEN_NAMES_RVA.0);
        header[table..table + 8].copy_from_slice(&(NAMES as u64).to_le_bytes());
        self.memory
            .put(BASE + cpdx_lexer::INITIALIZED_RVA.0, &header);
        for (id, name) in names.iter().enumerate() {
            self.string(NAMES + id * cpdx_lexer::TOKEN_NAME_STRIDE, name);
        }
    }

    fn walk(&self, rows: usize, text: usize) -> (Walk, Vec<Row>, Vec<u8>) {
        let mut output = vec![Row::default(); rows];
        let mut bytes = vec![0; text];
        let result = walk(&self.memory, BASE, &mut output, &mut bytes);
        (result, output, bytes)
    }
}

const LONG: &[u8] = b"MODIFIER_STABILITY_FACTOR";

fn registry() -> Table {
    Table::new(&[
        Some((0, 357, b"MODIFIER_NONE", b"", b"")),
        Some((1, 0x3433, LONG, b"MOD_DAYS", b"")),
        None,
        Some((3, 0x4b00, b"custom", b"", b"an_argument_of_20b")),
    ])
}

#[test]
fn walk_records_the_eligibility_callback_state() {
    let mut table = registry();
    assert!(
        table.walk(8, 128).1[..3]
            .iter()
            .all(|row| row.eligibility == 1)
    );
    table
        .memory
        .put(BASE + ELIGIBILITY_CALLBACK_RVA.0, &0_u64.to_le_bytes());
    let (result, rows, _) = table.walk(8, 128);
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert!(rows[..3].iter().all(|row| row.eligibility == 0));
    table
        .memory
        .put(BASE + ELIGIBILITY_CALLBACK_RVA.0, &0x1234_u64.to_le_bytes());
    assert_eq!(
        table.walk(8, 128).0.outcome,
        Outcome::Unavailable {
            reason: "modifier eligibility callback unsupported"
        }
    );
}

#[test]
fn walk_copies_registered_rows_and_skips_the_default_gap() {
    let (result, rows, text) = registry().walk(8, 128);
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    let bytes = |span: Span| &text[span.start..span.start + span.len];
    assert_eq!(
        rows[..3]
            .iter()
            .map(|row| (row.index, row.token))
            .collect::<Vec<_>>(),
        [(0, 357), (1, 0x3433), (3, 0x4b00)]
    );
    assert_eq!(bytes(rows[0].name_key), b"MODIFIER_NONE");
    assert_eq!(bytes(rows[1].name_key), LONG);
    assert_eq!(bytes(rows[1].postfix_key), b"MOD_DAYS");
    assert_eq!(rows[1].argument.len, 0);
    assert_eq!(bytes(rows[2].argument), b"an_argument_of_20b");
    assert_eq!(
        (
            rows[2].flags,
            rows[2].category_mask,
            rows[2].feature_selector,
            rows[2].precision
        ),
        (3, 11, 0, 2)
    );
    assert_eq!(result.text_len, result.text_required);
}

#[test]
fn inconsistent_rows_and_every_short_read_are_unavailable() {
    let mut table = registry();
    table
        .memory
        .put(TABLE + 2 * STRIDE + TOKEN, &7_u32.to_le_bytes());
    assert!(matches!(
        table.walk(8, 128).0.outcome,
        Outcome::Unavailable { .. }
    ));
    for address in [
        BASE + TABLE_RVA.0 + 12,
        TABLE + STRIDE + FLAGS,
        HEAP + 3,
        HEAP + LONG.len(),
    ] {
        let mut table = registry();
        table.memory.bytes.remove(&address);
        let (result, ..) = table.walk(8, 128);
        assert!(
            matches!(result.outcome, Outcome::Unavailable { .. }),
            "{address:#x}"
        );
        assert_eq!(result.text_len, 0);
    }
    let mut table = registry();
    table.memory.put(HEAP + LONG.len(), b"!");
    assert!(matches!(
        table.walk(8, 128).0.outcome,
        Outcome::Unavailable { .. }
    ));
    let mut table = registry();
    table.header(TABLE, 2, 4);
    assert!(table.walk(8, 128).0.outcome.is_empty());
    assert!(matches!(
        table.walk(8, 128).0.outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn the_header_is_the_refresh_control_and_a_moved_table_discards_the_copy() {
    let mut table = registry();
    let before = super::table(&table.memory, BASE).unwrap();
    assert_eq!((before.data, before.count), (TABLE, 4));
    table.header(TABLE, 7, 5);
    table.memory.put(TABLE + 4 * STRIDE, &[0; STRIDE]);
    for field in [NAME_KEY, POSTFIX_KEY, ARGUMENT] {
        table.string(TABLE + 4 * STRIDE + field, b"");
    }
    table
        .memory
        .put(TABLE + 4 * STRIDE + CATEGORY_MASK, &u32::MAX.to_le_bytes());
    let after = super::table(&table.memory, BASE).unwrap();
    assert_ne!(before, after);
    assert_eq!(table.walk(8, 128).0.outcome, Outcome::Rows { len: 3 });

    struct Moving<'a> {
        inner: &'a Fixture,
        header_reads: std::cell::Cell<usize>,
    }
    impl Memory for Moving<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == BASE + TABLE_RVA.0 {
                self.header_reads.set(self.header_reads.get() + 1);
                if self.header_reads.get() > 1 {
                    destination.copy_from_slice(&[0x11; HEADER_LEN]);
                    destination[8..16].copy_from_slice(&[9, 0, 0, 0, 4, 0, 0, 0]);
                    return true;
                }
            }
            self.inner.copy(address, destination)
        }
    }
    let table = registry();
    let moving = Moving {
        inner: &table.memory,
        header_reads: std::cell::Cell::new(0),
    };
    let result = walk(&moving, BASE, &mut vec![Row::default(); 8], &mut vec![0; 128]);
    assert_eq!(
        result.outcome,
        Outcome::Unavailable {
            reason: "modifier definition table changed during copy"
        }
    );
}

#[test]
fn key_by_name_resolves_the_registered_index_through_the_lexer() {
    let mut table = Table::new(&[
        Some((0, 1, b"MODIFIER_NONE", b"", b"")),
        None,
        Some((2, 3, LONG, b"", b"")),
        Some((3, 2, b"MODIFIER_WS", b"", b"")),
    ]);
    table.lexer(&[b"", b"none", b"war_support_factor", b"stability_factor"]);
    assert_eq!(key(&table.memory, b"stability_factor").unwrap(), Some(2));
    assert_eq!(key(&table.memory, b"war_support_factor").unwrap(), Some(3));
    assert_eq!(key(&table.memory, b"none").unwrap(), Some(0));
    let (keys, header) = keys_by_names_at(
        &table.memory,
        BASE,
        &[b"war_support_factor", b"stability", b"stability_factor"],
    )
    .unwrap();
    assert_eq!(keys, [Some(3), None, Some(2)]);
    assert_eq!((header.data, header.count), (TABLE, 4));
    assert_eq!(key(&table.memory, b"stability").unwrap(), None);
    table
        .memory
        .put(TABLE + 3 * STRIDE + TOKEN, &3_u32.to_le_bytes());
    assert!(key(&table.memory, b"stability_factor").is_err());
    table
        .memory
        .put(TABLE + 3 * STRIDE + TOKEN, &9_u32.to_le_bytes());
    assert!(key(&table.memory, b"stability_factor").is_err());
    table
        .memory
        .put(TABLE + 3 * STRIDE + TOKEN, &2_u32.to_le_bytes());
    table
        .memory
        .put(BASE + cpdx_lexer::WRITER_FLAG_RVA.0, &1_u32.to_le_bytes());
    assert!(key(&table.memory, b"stability_factor").is_err());
}

#[test]
fn named_key_resolves_once_per_table_header() {
    let mut table = Table::new(&[
        Some((0, 1, b"MODIFIER_NONE", b"", b"")),
        None,
        Some((2, 3, LONG, b"", b"")),
        Some((3, 2, b"MODIFIER_WS", b"", b"")),
    ]);
    table.lexer(&[b"", b"none", b"war_support_factor", b"stability_factor"]);
    let mut held = NamedKey::new(b"stability_factor");
    let mut absent = NamedKey::new(b"stability");
    assert_eq!(held.get(&table.memory, BASE), Ok(Some(2)));
    assert_eq!(absent.get(&table.memory, BASE), Ok(None));
    table
        .memory
        .put(TABLE + 2 * STRIDE + TOKEN, &9_u32.to_le_bytes());
    assert_eq!(held.get(&table.memory, BASE), Ok(Some(2)));
    assert_eq!(absent.get(&table.memory, BASE), Ok(None));
    table.header(TABLE, 7, 3);
    assert!(held.get(&table.memory, BASE).is_err());
    table
        .memory
        .put(TABLE + 2 * STRIDE + TOKEN, &3_u32.to_le_bytes());
    assert_eq!(held.get(&table.memory, BASE), Ok(Some(2)));
    assert_eq!(absent.get(&table.memory, BASE), Ok(None));
}

#[test]
fn modifier_definitions_grow_rows_and_text_from_original_headers_for_mod_populations() {
    struct Traced<'a> {
        source: &'a Fixture,
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, at: usize, bytes: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((at, bytes.len()));
            self.source.copy(at, bytes)
        }
    }
    let mut rows = Vec::new();
    let mut text = Vec::new();
    for (count, name) in [
        (50_usize, b"key" as &'static [u8]),
        (50, b"longer_key"),
        (120, b"longer_key"),
    ] {
        let definitions: Vec<_> = (0..count)
            .map(|index| {
                Some((
                    index as u32,
                    index as u32 + 1,
                    name,
                    b"p" as &[u8],
                    b"a" as &[u8],
                ))
            })
            .collect();
        let table = Table::new(&definitions);
        let demand = storage_demand(&table.memory, BASE).unwrap();
        assert_eq!(demand, (count, count * (name.len() + 2)));
        let source = Traced {
            source: &table.memory,
            calls: std::cell::RefCell::new(Vec::new()),
        };
        let mut expected_rows = vec![Row::default(); count];
        let mut expected_text = vec![0; demand.1];
        let expected = walk(&source, BASE, &mut expected_rows, &mut expected_text);
        let trace = source.calls.replace(Vec::new());
        let captured = walk(&source, BASE, &mut rows, &mut text);
        assert_eq!(captured, expected);
        assert_eq!(captured.outcome, Outcome::Rows { len: count });
        assert_eq!(&rows[..count], expected_rows.as_slice());
        assert_eq!(&text[..captured.text_len], expected_text.as_slice());
        assert_eq!(*source.calls.borrow(), trace);
        assert_eq!(
            trace
                .iter()
                .filter(|&&(at, _)| at == BASE + TABLE_RVA.0)
                .count(),
            2
        );
    }
}

#[test]
fn pass_metadata_memo_retains_original_keys_past_sixty_four() {
    let mut source = Fixture::new(KEY, 0, 0);
    for key in 1..=65 {
        source.put(Fixture::row(key) + FEATURE_SELECTOR, &0_u32.to_le_bytes());
        source.put(Fixture::row(key) + FLAGS, &key.to_le_bytes());
    }
    let mut memo = PassDefinitions::default();
    for key in 1..=65 {
        assert_eq!(memo.flags(BASE, key, || read_flags(&source, BASE, key)), Ok(Some(key)));
    }
    assert_eq!(memo.cached_keys(), 65);
    for key in 1..=65 {
        source.put(Fixture::row(key) + FLAGS, &0_u32.to_le_bytes());
        assert_eq!(memo.flags(BASE, key, || read_flags(&source, BASE, key)), Ok(Some(key)));
    }
    memo.clear();
    assert_eq!(memo.flags(BASE, 65, || read_flags(&source, BASE, 65)), Ok(Some(0)));
}

#[test]
fn requested_modifier_name_uses_its_native_lexer_length() {
    let name = vec![b'm'; 5_001];
    let mut table = Table::new(&[Some((0, 1, b"custom", b"", b""))]);
    table.lexer(&[b"", &name]);
    assert_eq!(key(&table.memory, &name), Ok(Some(0)));
}
