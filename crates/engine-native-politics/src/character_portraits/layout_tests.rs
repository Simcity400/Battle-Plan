use crate::character_portraits::{PortraitRow, PortraitWalk, walk_portraits};
use engine_native::ccharacter::*;
use engine_native::ccharacter::{manager as ccharacter_manager, template as ccharacter_template};
use engine_native_memory::{Memory, read, walk::Outcome};
use engine_native_memory::msvc_string;
use engine_native::{cadvisor, cgame_state, cprovince::UnitOrderWalk};
use engine_native_memory::memory::SliceMemory;
use crate::ccountry_leader;

const GS: usize = 0x1000;
const MANAGER: usize = 0x100;
const FIRST: usize = 0x400;
const SECOND: usize = 0x600;

fn walk_all(
    memory: &impl Memory,
    rows: &mut Vec<PortraitRow>,
    entries: &mut Vec<crate::character_portrait_entry::Row<engine_native_memory::capture_text::Owned>>,
) -> UnitOrderWalk {
    walk_portraits(memory, 0, GS, rows, entries, PortraitWalk::default())
}

fn first_entry(memory: &impl Memory) -> ccharacter_manager::Entry {
    let mut first = None;
    ccharacter_manager::visit(memory, 0, GS, |entry| {
        first = first.or(Some(entry));
        Ok(())
    })
    .unwrap();
    first.unwrap()
}

fn advisor_entry(
    memory: &impl Memory,
    base: usize,
    entry: ccharacter_manager::Entry,
    rows: &mut Vec<cadvisor::Row>,
    bytes: &mut Vec<u8>,
) -> NameWalk {
    let prefix = cadvisor::walk_prefix_entry(
        memory,
        base,
        entry,
        engine_native_memory::walk::Buffer::Growing(rows),
        engine_native_memory::walk::Buffer::Growing(bytes),
    );
    cadvisor::append_tooltips(memory, rows, engine_native_memory::walk::Buffer::Growing(bytes), prefix)
}

fn walk_roster(memory: &impl Memory) -> (Outcome, Vec<Row>, Vec<u8>) {
    let (mut rows, mut names) = (Vec::new(), Vec::new());
    let walked = (|| {
        let source = ccharacter_manager::Roster::read(memory, 0, GS)?;
        for index in 0..source.count() {
            let entry = source.at(memory, index)?;
            let (mut row, mut text) = (Vec::new(), Vec::new());
            let result = walk_entry(
                memory,
                0,
                entry,
                |_| None,
                engine_native_memory::walk::Buffer::Growing(&mut row),
                engine_native_memory::walk::Buffer::Growing(&mut text),
            );
            match result.outcome {
                Outcome::Rows { len: 1 } => {
                    let start = names.len();
                    let mut row = row[0];
                    row.name_start += start;
                    row.token_start += start;
                    row.template_file_start += start;
                    names.extend_from_slice(&text[..result.bytes_len]);
                    rows.push(row);
                }
                Outcome::Unavailable { reason } => return Err(reason),
                _ => return Err("character entry returned an invalid row count"),
            }
        }
        Ok(())
    })();
    let outcome = match walked {
        Ok(()) => Outcome::Rows { len: rows.len() },
        Err(reason) => Outcome::Unavailable { reason },
    };
    (outcome, rows, names)
}

fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn string(bytes: &mut [u8], at: usize, value: &[u8]) {
    assert!(value.len() <= 15);
    bytes[at..at + 32].fill(0);
    bytes[at..at + value.len()].copy_from_slice(value);
    u64_at(bytes, at + 16, value.len() as u64);
    u64_at(bytes, at + 24, 15);
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    u64_at(bytes, at, data as u64);
    u32_at(bytes, at + 8, count);
    u32_at(bytes, at + 12, count);
}
fn empty_tree(bytes: &mut [u8], character: usize, offset: usize, head: usize) {
    u64_at(bytes, character + offset, head as u64);
    u64_at(bytes, character + offset + 8, 0);
    for offset in [0, 8, 16] {
        u64_at(bytes, head + offset, head as u64);
    }
    bytes[head + 0x19] = 1;
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x2000];
    u64_at(
        &mut bytes,
        GS + cgame_state::CHARACTER_MANAGER,
        MANAGER as u64,
    );
    u64_at(&mut bytes, MANAGER, ccharacter_manager::VTABLE_RVA.0 as u64);
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        0x200,
        1,
    );
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0x208,
        1,
    );
    u64_at(&mut bytes, 0x200, FIRST as u64);
    u64_at(&mut bytes, 0x208, SECOND as u64);
    for (pointer, id, country, name, token) in [
        (FIRST, 900_u32, 0_u32, &b"same name"[..], &b""[..]),
        (SECOND, 22, 7, &b"same name"[..], &b"dynamic_token"[..]),
    ] {
        u64_at(&mut bytes, pointer, VTABLE_RVA.0 as u64);
        u32_at(&mut bytes, pointer + TYPE, REFERENCE_TYPE);
        u32_at(&mut bytes, pointer + ID, id);
        u32_at(&mut bytes, pointer + NAME_TOKEN_ID, (-1_i32) as u32);
        u32_at(&mut bytes, pointer + COUNTRY, country);
        string(&mut bytes, pointer + NAME, name);
        string(&mut bytes, pointer + TOKEN, token);
    }
    empty_tree(&mut bytes, FIRST, LEADER_MAP_HEAD, 0x800);
    empty_tree(&mut bytes, FIRST, ADVISOR_MAP_HEAD, 0x840);
    empty_tree(&mut bytes, SECOND, LEADER_MAP_HEAD, 0x880);
    empty_tree(&mut bytes, SECOND, ADVISOR_MAP_HEAD, 0x8c0);
    bytes
}
fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

struct TracedMemory<'a> {
    memory: SliceMemory<'a>,
    guards: std::cell::Cell<usize>,
    scalar_reads: std::cell::Cell<usize>,
    reads: std::cell::RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}

impl<'a> TracedMemory<'a> {
    fn new(bytes: &'a [u8], fault: Option<usize>) -> Self {
        Self {
            memory: memory(bytes),
            guards: std::cell::Cell::new(0),
            scalar_reads: std::cell::Cell::new(0),
            reads: std::cell::RefCell::new(Vec::new()),
            fault,
        }
    }

    fn field(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if let Some(fault) = self.fault
            && address <= fault
            && fault < address.saturating_add(output.len())
        {
            self.memory.copy(address, &mut output[..fault - address]);
            return false;
        }
        self.memory.copy(address, output)
    }
}

impl Memory for TracedMemory<'_> {
    fn read_scalar<T: engine_native_memory::memory::Raw>(&self, address: usize) -> Option<T> {
        self.scalar_reads.set(self.scalar_reads.get() + 1);
        let mut bytes = vec![0; size_of::<T>()];
        if !self.copy(address, &mut bytes) {
            return None;
        }
        engine_native_memory::memory::scalar_from_bytes(&bytes)
    }

    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        self.field(address, output)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        if outputs.iter().any(|output| !output.is_empty()) {
            self.guards.set(self.guards.get() + 1);
        }
        for (index, (address, output)) in addresses.into_iter().zip(outputs).enumerate() {
            if !output.is_empty() && !self.field(address, output) {
                return index;
            }
        }
        N
    }
}

#[test]
fn roster_pointer_chunks_preserve_both_orders_and_never_publish_a_failed_prefix() {
    let mut bytes = fixture();
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        0x200,
        33,
    );
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0x320,
        2,
    );
    for index in 0..33 {
        u64_at(&mut bytes, 0x200 + index * 8, FIRST as u64);
    }
    for index in 0..2 {
        u64_at(&mut bytes, 0x320 + index * 8, SECOND as u64);
    }
    let roster = ccharacter_manager::Roster::read(&memory(&bytes), 0, GS).unwrap();
    let traced = TracedMemory::new(&bytes, None);
    let mut cursor = roster.cursor();
    let mut entries = Vec::new();
    while let Some(view) = cursor.next(&traced, [true, false, false, false]).unwrap() {
        let entry = view.entry().unwrap();
        entries.push((entry.roster, entry.ordinal, entry.id));
    }
    assert_eq!(entries.len(), 35);
    assert_eq!(entries[0], (0, 0, 900));
    assert_eq!(entries[32], (0, 32, 900));
    assert_eq!(entries[33], (1, 0, 22));
    assert_eq!(entries[34], (1, 1, 22));
    assert_eq!(
        traced
            .reads
            .borrow()
            .iter()
            .copied()
            .filter(|&(address, _)| (0x200..0x330).contains(&address))
            .collect::<Vec<_>>(),
        [(0x200, 256), (0x300, 8), (0x320, 16)]
    );
    let traced = TracedMemory::new(&bytes, Some(0x200 + 8));
    let mut cursor = roster.cursor();
    for _ in 0..2 {
        assert!(matches!(
            cursor.next(&traced, [true; 4]),
            Err("character roster pointers unreadable")
        ));
    }
    assert_eq!(&*traced.reads.borrow(), &[(0x200, 256)]);
    for count in [0, 65] {
        let mut bytes = fixture();
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
            0x1800,
            count,
        );
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
            0x1b00,
            2,
        );
        for index in 0..count {
            u64_at(&mut bytes, 0x1800 + index as usize * 8, FIRST as u64);
        }
        for index in 0..2 {
            u64_at(&mut bytes, 0x1b00 + index * 8, SECOND as u64);
        }
        let traced = TracedMemory::new(&bytes, None);
        let roster = ccharacter_manager::Roster::read(&memory(&bytes), 0, GS).unwrap();
        let mut cursor = roster.cursor();
        let mut pages = Vec::new();
        let mut page = ccharacter_manager::ViewPage::default();
        while cursor.refill_page(&traced, &mut page, [true; 4]).unwrap() {
            let entries = page
                .by_ref()
                .map(|view| view.unwrap().entry().unwrap())
                .collect::<Vec<_>>();
            pages.push((entries[0].roster, entries[0].ordinal, entries.len()));
        }
        if count == 0 {
            assert_eq!(pages, [(1, 0, 2)]);
        } else {
            assert_eq!(pages, [(0, 0, 32), (0, 32, 32), (0, 64, 1), (1, 0, 2)]);
        }
    }
}

#[test]
fn advisor_exact_batches_reduce_guards_and_discard_every_failed_span() {
    use engine_native_parts::character_advisor_node as node;
    let mut bytes = fixture();
    let head = 0x840;
    let role_node = 0xa00;
    let role = 0x1300;
    u64_at(&mut bytes, FIRST + ADVISOR_MAP_SIZE, 1);
    for offset in [0, 8, 16] {
        u64_at(&mut bytes, head + offset, role_node as u64);
        u64_at(&mut bytes, role_node + offset, head as u64);
    }
    u64_at(&mut bytes, role_node + node::ADVISOR, role as u64);
    u32_at(&mut bytes, role_node + node::KEY_HASH, 17);
    u64_at(&mut bytes, role, cadvisor::VTABLE_RVA.0 as u64);
    u64_at(&mut bytes, role + cadvisor::CHARACTER, FIRST as u64);
    for (header, payload) in [
        (role_node + node::KEY, 0x1a00),
        (role + cadvisor::IDEA_TOKEN, 0x1a40),
        (role + cadvisor::SLOT, 0x1a80),
    ] {
        u64_at(&mut bytes, header, payload as u64);
        u64_at(&mut bytes, header + 16, 17);
        u64_at(&mut bytes, header + 24, 17);
        bytes[payload..payload + 17].copy_from_slice(b"literal bytes\xff123");
    }
    let entry = ccharacter_manager::Entry {
        pointer: FIRST,
        id: 900,
        country: 0,
        roster: 0,
        ordinal: 0,
    };
    let mut rows = vec![cadvisor::Row::default(); 1];
    let mut names = vec![0; 51];
    let traced = TracedMemory::new(&bytes, None);
    let result = advisor_entry(&traced, 0, entry, &mut rows, &mut names);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(traced.guards.get(), 11);
    assert_eq!(
        traced
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, len)| address == role + cadvisor::TOOLTIP_KEYS + 8 && len == 17)
            .count(),
        1
    );
    assert_eq!(
        traced
            .reads
            .borrow()
            .iter()
            .map(|&(_, len)| len)
            .sum::<usize>(),
        253 + 17
    );
    assert_eq!(&names[..17], b"literal bytes\xff123");
    for (fault, untouched) in [
        (role_node + node::KEY + 7, role),
        (role + cadvisor::SLOT + 17, 0x1a00 + 17),
        (0x1a00 + 17, 0x1a00),
        (0x1a00 + 7, 0x1a40),
    ] {
        let traced = TracedMemory::new(&bytes, Some(fault));
        let result = advisor_entry(&traced, 0, entry, &mut rows, &mut names);
        assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
        assert_eq!(result.bytes_len, 0);
        assert!(
            !traced
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| address == untouched)
        );
    }
}

#[test]
fn advisor_cache_fits_and_a_short_arena_grows_keeping_the_existing_role() {
    use engine_native_parts::character_advisor_node as node;
    let mut bytes = fixture();
    let head = 0x840;
    let role_node = 0xa00;
    let role = 0x1300;
    let data = 0x1c00;
    u64_at(&mut bytes, FIRST + ADVISOR_MAP_SIZE, 1);
    for offset in [0, 8, 16] {
        u64_at(&mut bytes, head + offset, role_node as u64);
        u64_at(&mut bytes, role_node + offset, head as u64);
    }
    u64_at(&mut bytes, role_node + node::ADVISOR, role as u64);
    u32_at(&mut bytes, role_node + node::KEY_HASH, 17);
    u64_at(&mut bytes, role, cadvisor::VTABLE_RVA.0 as u64);
    u64_at(&mut bytes, role + cadvisor::CHARACTER, FIRST as u64);
    string(&mut bytes, role_node + node::KEY, b"key");
    string(&mut bytes, role + cadvisor::IDEA_TOKEN, b"idea");
    string(&mut bytes, role + cadvisor::SLOT, b"slot");
    array(&mut bytes, role + cadvisor::MODIFIER_ENTRIES, data, 1);
    u32_at(&mut bytes, data, 23);
    u64_at(&mut bytes, data + 8, (-17_i64) as u64);
    let entry = ccharacter_manager::Entry {
        pointer: FIRST,
        id: 900,
        country: 7,
        roster: 0,
        ordinal: 0,
    };
    for (capacity, recorded, bytes_len) in [(27, true, 27), (0, true, 27)] {
        let traced = TracedMemory::new(&bytes, None);
        let mut rows = vec![cadvisor::Row::default(); 1];
        let mut arena = vec![0; capacity];
        let result = advisor_entry(&traced, 0, entry, &mut rows, &mut arena);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(result.required_bytes, 27);
        assert_eq!(result.bytes_len, bytes_len);
        assert_eq!((rows[0].character, rows[0].country), (900, 7));
        assert_eq!(&arena[..11], b"keyideaslot");
        assert_eq!(rows[0].modifier_recorded, recorded);
        assert_eq!(
            rows[0].modifier_failure,
            (!recorded).then_some(cadvisor::MODIFIER_CAPACITY_REASON)
        );
        assert_eq!(
            traced
                .reads
                .borrow()
                .iter()
                .any(|&(address, _)| address == data),
            recorded
        );
        if recorded {
            assert_eq!(&arena[11..27], &bytes[data..data + 16]);
        }
    }
}

#[test]
fn copied_character_spans_preserve_first_fault_order() {
    let bytes = fixture();
    let memory = TracedMemory::new(&bytes, None);
    let roster = ccharacter_manager::Roster::read(&memory, 0, GS).unwrap();
    let view = roster.view(&memory, 0, [true; 4]).unwrap();
    let mut failed = [0xaa; 8];
    let mut later = [0xbb; 4];
    assert_eq!(
        view.copy_scattered([0x4000, FIRST + COUNTRY], &mut [&mut failed, &mut later]),
        0
    );
    assert_eq!(later, [0xbb; 4]);
    let scalar_before = memory.scalar_reads.get();
    memory.reads.borrow_mut().clear();
    assert_eq!(read::<u32>(&view, FIRST + ID), Some(900));
    assert_eq!(read::<i32>(&view, FIRST + NAME_TOKEN_ID), Some(-1));
    assert_eq!(read::<u32>(&view, FIRST + COUNTRY + 1), None);
    assert_eq!(memory.scalar_reads.get(), scalar_before);
    assert!(memory.reads.borrow().is_empty());
    assert_eq!(read::<u32>(&view, SECOND + ID), Some(22));
    assert_eq!(read::<u32>(&view, 0x4000), None);
    assert_eq!(memory.scalar_reads.get(), scalar_before + 2);
    assert_eq!(&*memory.reads.borrow(), &[(SECOND + ID, 4), (0x4000, 4)]);

    for (fault, required) in [(None, [false; 4]), (Some(FIRST + COUNTRY), [true; 4])] {
        let memory = TracedMemory::new(&bytes, fault);
        let view = roster.view(&memory, 0, required).unwrap();
        memory.reads.borrow_mut().clear();
        let scalar_before = memory.scalar_reads.get();
        assert_eq!(read::<u32>(&view, FIRST + COUNTRY), None);
        assert_eq!(read::<[u8; 0]>(&view, 0), Some([]));
        assert_eq!(memory.scalar_reads.get(), scalar_before);
        assert!(memory.reads.borrow().is_empty());
    }
}

#[test]
fn gender_is_copied_once_and_failure_does_not_poison_roles() {
    let mut bytes = fixture();
    bytes[FIRST + GENDER..FIRST + GENDER + 4].copy_from_slice(&2_u32.to_le_bytes());
    for fault in [None, Some(FIRST + GENDER)] {
        let memory = TracedMemory::new(&bytes, fault);
        let roster = ccharacter_manager::Roster::read(&memory, 0, GS).unwrap();
        let view = roster.view(&memory, 0, [true; 4]).unwrap();
        memory.reads.borrow_mut().clear();
        assert_eq!(
            read::<u32>(&view, FIRST + GENDER),
            fault.is_none().then_some(2)
        );
        assert!(read::<u64>(&view, FIRST + UNIT_LEADER).is_some());
        assert!(memory.reads.borrow().is_empty());
    }
}

#[test]
fn advisor_native_hash_order_can_reverse_literal_order() {
    use engine_native_parts::character_advisor_node as advisor_node;
    let mut bytes = fixture();
    let head = 0x840;
    let left = 0xa00;
    let root = 0xa60;
    u64_at(&mut bytes, FIRST + ADVISOR_MAP_SIZE, 2);
    u64_at(&mut bytes, head, left as u64);
    u64_at(&mut bytes, head + 8, root as u64);
    u64_at(&mut bytes, head + 16, root as u64);
    for (node, parent, child, role, hash, key) in [
        (left, root, head, 0x1300, 1_u32, &b"zulu"[..]),
        (root, head, left, 0x1800, 2_u32, &b"alpha"[..]),
    ] {
        u64_at(&mut bytes, node, child as u64);
        u64_at(&mut bytes, node + 8, parent as u64);
        u64_at(&mut bytes, node + 16, head as u64);
        string(&mut bytes, node + advisor_node::KEY, key);
        u32_at(&mut bytes, node + advisor_node::KEY_HASH, hash);
        u64_at(&mut bytes, node + advisor_node::ADVISOR, role as u64);
        u64_at(&mut bytes, role, cadvisor::VTABLE_RVA.0 as u64);
        u64_at(&mut bytes, role + cadvisor::CHARACTER, FIRST as u64);
        string(&mut bytes, role + cadvisor::IDEA_TOKEN, b"idea");
        string(&mut bytes, role + cadvisor::SLOT, key);
    }
    let mut rows = vec![cadvisor::Row::default(); 2];
    let mut names = vec![0; 64];
    let result = advisor_entry(&memory(&bytes), 0, first_entry(&memory(&bytes)), &mut rows, &mut names);
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        &names[rows[0].key_start..rows[0].key_start + rows[0].key_len],
        b"zulu"
    );
    assert_eq!(
        &names[rows[1].key_start..rows[1].key_start + rows[1].key_len],
        b"alpha"
    );
    assert_eq!(rows.iter().map(|row| row.ordinal).collect::<Vec<_>>(), [0, 1]);
    u32_at(&mut bytes, root + advisor_node::KEY_HASH, 0);
    assert_eq!(
        advisor_entry(&memory(&bytes), 0, first_entry(&memory(&bytes)), &mut rows, &mut names).outcome,
        Outcome::Unavailable {
            reason: "advisor role map hash order invalid"
        }
    );
    u32_at(&mut bytes, root + advisor_node::KEY_HASH, 1);
    assert_eq!(
        advisor_entry(&memory(&bytes), 0, first_entry(&memory(&bytes)), &mut rows, &mut names).outcome,
        Outcome::Unavailable {
            reason: "advisor role hash collision needs native locale comparison"
        }
    );
}

#[test]
fn complete_rosters_keep_duplicate_names_negative_tokens_and_unrecruited_characters() {
    let bytes = fixture();
    let (outcome, rows, names) = walk_roster(&memory(&bytes));
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [900, 22]);
    assert_eq!(
        (rows[0].country, rows[0].name_token_id, rows[0].token_len),
        (0, -1, 0)
    );
    assert_eq!((rows[0].roster, rows[1].roster), (0, 1));
    assert_eq!(
        &names[rows[1].token_start..rows[1].token_start + rows[1].token_len],
        b"dynamic_token"
    );
}

#[test]
fn a_roster_occurrence_of_another_type_fails_the_roster() {
    let mut bytes = fixture();
    u32_at(&mut bytes, SECOND + TYPE, 12);
    assert!(matches!(
        walk_roster(&memory(&bytes)).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn duplicate_ids_reach_off_thread_validation_and_corrupt_strings_fail() {
    let mut bytes = fixture();
    u32_at(&mut bytes, SECOND + ID, 900);
    let (outcome, rows, _) = walk_roster(&memory(&bytes));
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [900, 900]);
    u32_at(&mut bytes, SECOND + ID, 22);
    u64_at(&mut bytes, SECOND + TOKEN + 16, 16);
    assert!(matches!(
        walk_roster(&memory(&bytes)).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn heap_name_bytes_are_exact_and_a_bad_terminator_fails() {
    let mut bytes = fixture();
    let raw = [0xff_u8, 0, 42, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
    u64_at(&mut bytes, FIRST + NAME, 0x1a00);
    u64_at(&mut bytes, FIRST + NAME + 16, raw.len() as u64);
    u64_at(&mut bytes, FIRST + NAME + 24, 31);
    bytes[0x1a00..0x1a10].copy_from_slice(&raw);
    let (outcome, rows, names) = walk_roster(&memory(&bytes));
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(rows[0].name_len, raw.len());
    assert_eq!(
        &names[rows[0].name_start..rows[0].name_start + raw.len()],
        &raw
    );
    bytes[0x1a10] = 1;
    assert!(matches!(
        walk_roster(&memory(&bytes)).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn templates_are_read_or_reused_by_pointer_and_their_file_follows_the_names() {
    use ccharacter_template as template;
    const AT: usize = 0x2000;
    const FILE: usize = 0x2400;
    let path = b"common/characters/GER.txt";
    let mut bytes = fixture();
    bytes.resize(0x3000, 0);
    u64_at(&mut bytes, SECOND + TEMPLATE, AT as u64);
    u64_at(&mut bytes, AT, template::VTABLE_RVA.0 as u64);
    u32_at(&mut bytes, AT + template::TAG, 0x4242);
    u64_at(&mut bytes, AT + template::SOURCE_FILE, FILE as u64);
    u64_at(
        &mut bytes,
        AT + template::SOURCE_FILE + 16,
        path.len() as u64,
    );
    u64_at(&mut bytes, AT + template::SOURCE_FILE + 24, 31);
    bytes[FILE..FILE + path.len()].copy_from_slice(path);
    u32_at(&mut bytes, AT + template::SOURCE_LINE, 12);
    bytes[AT + template::GENERATED] = 1;
    let (outcome, rows, names) = walk_roster(&memory(&bytes));
    assert_eq!(outcome, Outcome::Rows { len: 2 });
    assert_eq!(names.len(), 9 + 9 + 13 + path.len());
    assert_eq!((rows[0].template, rows[0].template_file_len), (0, 0));
    assert_eq!(rows[0].template_identity, template::Identity::default());
    assert_eq!(rows[1].template, AT);
    assert_eq!(
        rows[1].template_identity,
        template::Identity {
            tag: 0x4242,
            line: 12,
            generated: 1
        }
    );
    let file = rows[1].template_file_start..rows[1].template_file_start + path.len();
    assert_eq!(&names[file], path);
    assert_eq!(
        demand(&memory(&bytes), 0, GS),
        Ok(Demand {
            rows: 2,
            bytes: names.len()
        })
    );

    let mut names = [0_u8; 64];
    let traced = TracedMemory::new(&bytes, None);
    let roster = ccharacter_manager::Roster::read(&traced, 0, GS).unwrap();
    let entry = roster.at(&traced, 1).unwrap();
    traced.reads.borrow_mut().clear();
    let known = template::Identity {
        tag: 7,
        line: -1,
        generated: 0,
    };
    let mut row = [Row::default()];
    let result = walk_entry(
        &traced,
        0,
        entry,
        |pointer| {
            (pointer == AT).then_some(template::Known {
                identity: known,
                file: b"",
            })
        },
        engine_native_memory::walk::Buffer::Exact(&mut row),
        engine_native_memory::walk::Buffer::Exact(&mut names),
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (
            row[0].template,
            row[0].template_identity,
            row[0].template_file_len
        ),
        (AT, known, 0)
    );
    assert!(
        traced
            .reads
            .borrow()
            .iter()
            .all(|&(address, _)| !(AT..AT + 0x248).contains(&address)
                && address != FILE + path.len())
    );

    let mut short = [0_u8; 9 + 13 + 4];
    let result = walk_entry(
        &memory(&bytes),
        0,
        entry,
        |_| None,
        engine_native_memory::walk::Buffer::Exact(&mut row),
        engine_native_memory::walk::Buffer::Exact(&mut short),
    );
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(result.required_bytes, 9 + 13 + path.len());

    u64_at(&mut bytes, AT + template::SOURCE_FILE, 0);
    u64_at(&mut bytes, AT + template::SOURCE_FILE + 16, 0);
    u64_at(&mut bytes, AT + template::SOURCE_FILE + 24, 15);
    u32_at(&mut bytes, AT + template::SOURCE_LINE, u32::MAX);
    bytes[AT + template::GENERATED] = 0;
    let result = walk_entry(
        &memory(&bytes),
        0,
        entry,
        |_| None,
        engine_native_memory::walk::Buffer::Exact(&mut row),
        engine_native_memory::walk::Buffer::Exact(&mut names),
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(row[0].template_identity.line, -1);
    assert_eq!(row[0].template_file_len, 0);

    u64_at(&mut bytes, AT, 0x1234);
    assert_eq!(
        walk_entry(
            &memory(&bytes),
            0,
            entry,
            |_| None,
            engine_native_memory::walk::Buffer::Exact(&mut row),
            engine_native_memory::walk::Buffer::Exact(&mut names)
        )
        .outcome,
        Outcome::Unavailable {
            reason: "character template vtable mismatch"
        }
    );
}

#[test]
fn role_walks_keep_distinct_ideologies_and_do_not_claim_advisor_appointment() {
    let mut bytes = fixture();
    let head = 0x800;
    let root = 0x900;
    let left = 0x940;
    u64_at(&mut bytes, FIRST + LEADER_MAP_SIZE, 2);
    u64_at(&mut bytes, head, left as u64);
    u64_at(&mut bytes, head + 8, root as u64);
    u64_at(&mut bytes, head + 16, root as u64);
    for (node, parent, child, ideology, role) in [
        (root, head, left, 3_i32, 0x1200),
        (left, root, head, -7_i32, 0x1240),
    ] {
        u64_at(&mut bytes, node, child as u64);
        u64_at(&mut bytes, node + 8, parent as u64);
        u64_at(&mut bytes, node + 16, head as u64);
        u32_at(&mut bytes, node + 0x20, ideology as u32);
        u64_at(&mut bytes, node + 0x28, role as u64);
        u64_at(&mut bytes, role, engine_native_parts::ccountry_leader_layout::VTABLE_RVA.0 as u64);
        u64_at(&mut bytes, role + engine_native_parts::ccountry_leader_layout::CHARACTER, FIRST as u64);
    }
    let mut leaders = [ccountry_leader::Row::default(); 2];
    assert_eq!(
        ccountry_leader::walk(
            &memory(&bytes),
            0,
            GS,
            engine_native_memory::walk::Buffer::Exact(&mut leaders)
        ),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(leaders.map(|row| row.ideology), [-7, 3]);
    let head = 0x840;
    let node = 0xa00;
    let role = 0x1300;
    u64_at(&mut bytes, FIRST + ADVISOR_MAP_SIZE, 1);
    for offset in [0, 8, 16] {
        u64_at(&mut bytes, head + offset, node as u64);
    }
    for offset in [0, 8, 16] {
        u64_at(&mut bytes, node + offset, head as u64);
    }
    string(&mut bytes, node + 0x20, b"role_key");
    u64_at(&mut bytes, node + 0x48, role as u64);
    u64_at(&mut bytes, role, cadvisor::VTABLE_RVA.0 as u64);
    u64_at(&mut bytes, role + cadvisor::CHARACTER, FIRST as u64);
    string(&mut bytes, role + cadvisor::IDEA_TOKEN, b"idea_token");
    string(&mut bytes, role + cadvisor::SLOT, b"army_chief");
    let mut advisors = vec![cadvisor::Row::default(); 1];
    let mut names = vec![0; 64];
    let result = advisor_entry(&memory(&bytes), 0, first_entry(&memory(&bytes)), &mut advisors, &mut names);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (
            advisors[0].character,
            advisors[0].country,
            advisors[0].ordinal
        ),
        (900, 0, 0)
    );
    assert_eq!(
        &names[advisors[0].idea_start..advisors[0].idea_start + advisors[0].idea_len],
        b"idea_token"
    );
    u64_at(&mut bytes, role + cadvisor::CHARACTER, SECOND as u64);
    assert!(matches!(
        advisor_entry(&memory(&bytes), 0, first_entry(&memory(&bytes)), &mut advisors, &mut names).outcome,
        Outcome::Unavailable { .. }
    ));
    u64_at(&mut bytes, root + 8, root as u64);
    assert!(matches!(
        ccountry_leader::walk(
            &memory(&bytes),
            0,
            GS,
            engine_native_memory::walk::Buffer::Exact(&mut leaders)
        ),
        Outcome::Unavailable { .. }
    ));
}
#[test]
fn view_pages_keep_native_boundaries_occurrences_and_failed_root_atomicity() {
    let mut bytes = fixture();
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        0x200,
        33,
    );
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0x320,
        2,
    );
    for index in 0..33 {
        u64_at(&mut bytes, 0x200 + index * 8, FIRST as u64);
    }
    for index in 0..2 {
        u64_at(&mut bytes, 0x320 + index * 8, SECOND as u64);
    }
    let roster = ccharacter_manager::Roster::read(&memory(&bytes), 0, GS).unwrap();
    let traced = TracedMemory::new(&bytes, None);
    let mut cursor = roster.cursor();
    let mut entries = Vec::new();
    let mut lengths = Vec::new();
    let mut page = ccharacter_manager::ViewPage::default();
    while cursor.refill_page(&traced, &mut page, [true; 4]).unwrap() {
        let before = entries.len();
        for view in page.by_ref() {
            let entry = view.unwrap().entry().unwrap();
            entries.push((entry.roster, entry.ordinal, entry.id));
        }
        lengths.push(entries.len() - before);
    }
    assert_eq!(lengths, [32, 1, 2]);
    assert_eq!(entries.len(), 35);
    assert_eq!(entries[0], (0, 0, 900));
    assert_eq!(entries[32], (0, 32, 900));
    assert_eq!(entries[33], (1, 0, 22));
    assert_eq!(entries[34], (1, 1, 22));
    assert_eq!(
        traced
            .reads
            .borrow()
            .iter()
            .copied()
            .filter(|&(address, _)| (0x200..0x330).contains(&address))
            .collect::<Vec<_>>(),
        [(0x200, 256), (0x300, 8), (0x320, 16)]
    );
    assert_eq!(
        traced
            .reads
            .borrow()
            .iter()
            .filter(|&&(address, len)| address == FIRST && len == 16)
            .count(),
        33
    );
    let traced = TracedMemory::new(&bytes, Some(0x200 + 8));
    let mut cursor = roster.cursor();
    let mut page = ccharacter_manager::ViewPage::default();
    for _ in 0..2 {
        assert!(matches!(
            cursor.refill_page(&traced, &mut page, [true; 4]),
            Err("character roster pointers unreadable")
        ));
        assert!(page.next().is_none());
    }
    assert_eq!(&*traced.reads.borrow(), &[(0x200, 256)]);
}

#[test]
fn view_pages_refill_hides_stale_slots_errors_and_prior_source_values() {
    let mut full = fixture();
    array(
        &mut full,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        0x200,
        32,
    );
    array(
        &mut full,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0,
        0,
    );
    for index in 0..32 {
        u64_at(&mut full, 0x200 + index * 8, FIRST as u64);
    }
    let mut short = full.clone();
    array(
        &mut short,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        0x200,
        1,
    );
    u32_at(&mut short, FIRST + ID, 777);
    let mut terminal = full.clone();
    u64_at(&mut terminal, 0x208, SECOND as u64);
    u32_at(&mut terminal, SECOND + TYPE, 0);
    let mut empty = short.clone();
    array(
        &mut empty,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        0,
        0,
    );

    let full_memory = TracedMemory::new(&full, None);
    let short_memory = TracedMemory::new(&short, None);
    let terminal_memory = TracedMemory::new(&terminal, None);
    let root_fault = TracedMemory::new(&short, Some(0x200));
    let identity_fault = TracedMemory::new(&short, Some(FIRST));
    let empty_memory = TracedMemory::new(&empty, None);
    let roster = |bytes: &[u8]| ccharacter_manager::Roster::read(&memory(bytes), 0, GS).unwrap();
    let mut page = ccharacter_manager::ViewPage::default();
    assert!(
        roster(&full)
            .cursor()
            .refill_page(&full_memory, &mut page, [true; 4])
            .unwrap()
    );
    assert_eq!(page.next().unwrap().unwrap().reference().1, 900);
    assert!(
        roster(&short)
            .cursor()
            .refill_page(&short_memory, &mut page, [true; 4])
            .unwrap()
    );
    assert_eq!(page.next().unwrap().unwrap().reference().1, 777);
    assert!(page.next().is_none());
    assert!(
        roster(&terminal)
            .cursor()
            .refill_page(&terminal_memory, &mut page, [true; 4])
            .unwrap()
    );
    assert_eq!(page.next().unwrap().unwrap().reference().1, 900);
    assert!(matches!(
        roster(&short)
            .cursor()
            .refill_page(&root_fault, &mut page, [true; 4]),
        Err("character roster pointers unreadable")
    ));
    assert!(page.next().is_none());
    assert_eq!(&*root_fault.reads.borrow(), &[(0x200, 8)]);
    assert!(
        roster(&short)
            .cursor()
            .refill_page(&identity_fault, &mut page, [true; 4])
            .unwrap()
    );
    assert!(matches!(
        page.next(),
        Some(Err("character identity unreadable"))
    ));
    assert!(page.next().is_none());
    assert!(
        roster(&full)
            .cursor()
            .refill_page(&full_memory, &mut page, [true; 4])
            .unwrap()
    );
    assert!(
        !roster(&empty)
            .cursor()
            .refill_page(&empty_memory, &mut page, [true; 4])
            .unwrap()
    );
    assert!(page.next().is_none());
    assert!(empty_memory.reads.borrow().is_empty());
    assert!(
        roster(&short)
            .cursor()
            .refill_page(&short_memory, &mut page, [true; 4])
            .unwrap()
    );
    assert_eq!(page.next().unwrap().unwrap().reference().1, 777);
    assert!(page.next().is_none());
}

#[test]
fn view_pages_defer_later_fatal_slots_and_retain_independent_optional_fields() {
    let mut bytes = fixture();
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::HISTORICAL_CHARACTERS,
        0x200,
        3,
    );
    array(
        &mut bytes,
        MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
        0,
        0,
    );
    for (index, pointer) in [FIRST, SECOND, FIRST].into_iter().enumerate() {
        u64_at(&mut bytes, 0x200 + index * 8, pointer as u64);
    }
    for fault in [SECOND, SECOND + TYPE] {
        let roster = ccharacter_manager::Roster::read(&memory(&bytes), 0, GS).unwrap();
        let traced = TracedMemory::new(&bytes, Some(fault));
        let mut page = ccharacter_manager::ViewPage::default();
        assert!(
            roster
                .cursor()
                .refill_page(&traced, &mut page, [true; 4])
                .unwrap()
        );
        let first = page.next().unwrap().unwrap();
        assert_eq!(first.reference().1, 900);
        assert!(matches!(
            page.next(),
            Some(Err("character identity unreadable"))
        ));
        assert!(page.next().is_none());
        assert_eq!(
            traced
                .reads
                .borrow()
                .iter()
                .filter(|&&(address, len)| address == FIRST && len == 16)
                .count(),
            1
        );
    }
    let roster = ccharacter_manager::Roster::read(&memory(&bytes), 0, GS).unwrap();
    let traced = TracedMemory::new(&bytes, Some(FIRST + NAME + 32));
    let mut page = ccharacter_manager::ViewPage::default();
    assert!(
        roster
            .cursor()
            .refill_page(&traced, &mut page, [true; 4])
            .unwrap()
    );
    let first = page.next().unwrap().unwrap();
    assert!(first.entry().is_ok());
    assert!(matches!(
        walk_entry(
            &first,
            0,
            first.entry().unwrap(),
            |_| None,
            engine_native_memory::walk::Buffer::Exact(&mut [Row::default(); 1]),
            engine_native_memory::walk::Buffer::Exact(&mut [0; 64])
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
    let second = page.next().unwrap().unwrap();
    assert_eq!(second.reference().1, 22);
    assert!(second.entry().is_ok());
}
#[test]
fn hourly_portrait_holder_preserves_entry_order_and_requires_whole_character_capacity() {
    let mut bytes = fixture();
    array(&mut bytes, FIRST + PORTRAIT_DATA, 0x1800, 2);
    for (index, size) in [1, 0].into_iter().enumerate() {
        let at = 0x1800 + index * 0x38;
        u32_at(&mut bytes, at, 2);
        u32_at(&mut bytes, at + 4, size);
        bytes[at + 8] = 255;
        u32_at(&mut bytes, at + 12, 1);
        string(
            &mut bytes,
            at + 16,
            if index == 0 { b"a.dds" } else { b"b.dds" },
        );
    }
    let mut rows = vec![PortraitRow::default(); 2];
    let mut entries = vec![crate::character_portrait_entry::Row::<engine_native_memory::capture_text::Owned>::default(); 2];
    let result = walk_all(&memory(&bytes), &mut rows, &mut entries);
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!([rows[0].character, rows[1].character], [22, 900]);
    assert_eq!([entries[0].size, entries[1].size], [1, 0]);
    assert_eq!(entries[0].is_set, 255);
    u32_at(&mut bytes, FIRST + PORTRAIT_COUNT, 129);
    assert!(matches!(
        walk_all(&memory(&bytes), &mut rows, &mut entries).outcome,
        Outcome::Unavailable { .. }
    ));
    use crate::portrait_cache::{EntryImage, HolderImage, test_images};
    let logged: &'static [EntryImage] = Box::leak(Box::new([EntryImage {
        head: [3, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0],
        text: [0, 5],
    }]));
    let mut header = [0; 16];
    header[..8].copy_from_slice(&0x1800_u64.to_le_bytes());
    header[8..12].copy_from_slice(&1_u32.to_le_bytes());
    header[12..].copy_from_slice(&1_u32.to_le_bytes());
    let image = HolderImage::new(header, logged, b"c.dds");
    test_images::set(FIRST + PORTRAIT_DATA, Some(image));
    let result = walk_all(&memory(&bytes), &mut rows, &mut entries);
    test_images::set(0, None);
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    let changed = rows.iter().find(|row| row.portraits[1] == 1).unwrap();
    let entry = &entries[changed.portraits[0]];
    assert_eq!((entry.category, entry.size, entry.is_set), (3, 1, 1));
    assert_eq!(entry.identity.as_bytes(), b"c.dds");
}

#[test]
fn portrait_census_and_walk_have_no_fixed_character_limit() {
    use engine_native_memory::memory::SliceMemory;
    use ccharacter_manager::{PortraitCensus, PortraitIdentity};
    const CHARACTERS: usize = 70_000;
    const OBJECTS: usize = 0x10_0000;
    let bytes = vec![0; OBJECTS + PORTRAIT_DATA + CHARACTERS * 16];
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let identity = |index: usize| PortraitIdentity {
        pointer: OBJECTS + index * 16,
        character: index as u32 + 1,
    };
    let mut census = PortraitCensus::new(PortraitCensus::INITIAL_CAPACITY);
    const { assert!(CHARACTERS > PortraitCensus::INITIAL_CAPACITY) };
    let fill = |census: &mut PortraitCensus| {
        census.begin(0, GS);
        census.expect(CHARACTERS);
        for index in 0..CHARACTERS {
            census.push(identity(index));
        }
        census.finish(CHARACTERS);
    };
    fill(&mut census);
    assert!(
        census.rows(0, GS).is_none(),
        "a short census is never published"
    );
    assert!(census.needs_growth());
    census.grow().unwrap();
    assert!(!census.needs_growth());
    fill(&mut census);
    let rows = census.rows(0, GS).expect("grown census is complete");
    assert_eq!(rows.len(), CHARACTERS);
    assert_eq!(rows[CHARACTERS - 1], identity(CHARACTERS - 1));

    let mut output = vec![PortraitRow::default(); CHARACTERS];
    let walked = walk_portraits(
        &memory,
        0,
        GS,
        &mut output,
        &mut Vec::new(),
        PortraitWalk {
            census: Some(&census),
            ..Default::default()
        },
    );
    assert_eq!(walked.outcome, Outcome::Rows { len: CHARACTERS });
    assert_eq!(output[CHARACTERS - 1].character, CHARACTERS as u32);
}

#[test]
fn shared_portrait_census_removes_only_common_reads_and_preserves_faults() {
    use ccharacter_manager::{PortraitCensus, PortraitIdentity};
    let mut bytes = fixture();
    array(&mut bytes, FIRST + PORTRAIT_DATA, 0x1800, 1);
    string(&mut bytes, 0x1810, b"portrait.dds");
    let mut census = PortraitCensus::new(2);
    census.begin(0, GS);
    for (pointer, character) in [(FIRST, 900), (SECOND, 22)] {
        census.push(PortraitIdentity { pointer, character });
    }
    census.finish(2);
    let shared = TracedMemory::new(&bytes, None);
    let mut rows = vec![PortraitRow::default(); 2];
    let mut entries = vec![crate::character_portrait_entry::Row::<engine_native_memory::capture_text::Owned>::default(); 2];
    let capture = |memory: &TracedMemory<'_>,
                   census: Option<&PortraitCensus>,
                   rows: &mut Vec<PortraitRow>,
                   entries: &mut Vec<crate::character_portrait_entry::Row<engine_native_memory::capture_text::Owned>>| {
        let walk = PortraitWalk {
            census,
            ..Default::default()
        };
        walk_portraits(memory, 0, GS, rows, entries, walk)
    };
    assert_eq!(
        capture(&shared, Some(&census), &mut rows, &mut entries).outcome,
        Outcome::Rows { len: 2 }
    );
    let reads = shared.reads.borrow().clone();
    assert!(reads.iter().all(|&(at, _)| at == FIRST + PORTRAIT_DATA
        || at == SECOND + PORTRAIT_DATA
        || (0x1800..0x1838).contains(&at)));
    for fault in reads.iter().map(|&(at, _)| Some(at)).chain([None]) {
        {
            let direct = TracedMemory::new(&bytes, fault);
            let shared = TracedMemory::new(&bytes, fault);
            let mut expected_rows = vec![PortraitRow::default(); 2];
            let mut expected_entries = vec![crate::character_portrait_entry::Row::<engine_native_memory::capture_text::Owned>::default(); 2];
            let mut actual_rows = expected_rows.clone();
            let mut actual_entries = expected_entries.clone();
            let expected = capture(&direct, None, &mut expected_rows, &mut expected_entries);
            let actual = capture(&shared, Some(&census), &mut actual_rows, &mut actual_entries);
            assert_eq!(
                (actual.outcome, actual.len, actual.required),
                (expected.outcome, expected.len, expected.required)
            );
            assert_eq!(actual_rows, expected_rows);
            assert_eq!(actual_entries, expected_entries);
            let portrait_reads: Vec<_> = direct
                .reads
                .borrow()
                .iter()
                .copied()
                .filter(|&(at, _)| reads.iter().any(|&(portrait, _)| at == portrait))
                .collect();
            assert_eq!(*shared.reads.borrow(), portrait_reads);
        }
    }
    for scope in [(1, GS), (0, GS)] {
        census.begin(scope.0, scope.1);
        if scope.0 == 1 {
            census.finish(0);
        }
        let direct = TracedMemory::new(&bytes, None);
        let fallback = TracedMemory::new(&bytes, None);
        capture(&direct, None, &mut rows, &mut entries);
        capture(&fallback, Some(&census), &mut rows, &mut entries);
        assert_eq!(*direct.reads.borrow(), *fallback.reads.borrow());
    }
    census.begin(0, GS);
    for _ in 0..2 {
        census.push(PortraitIdentity {
            pointer: FIRST,
            character: 900,
        });
    }
    census.finish(2);
    let duplicate = TracedMemory::new(&bytes, None);
    assert_eq!(
        capture(&duplicate, Some(&census), &mut rows, &mut entries).outcome,
        Outcome::Unavailable {
            reason: "duplicate character portrait identity"
        }
    );
    assert_eq!(
        duplicate
            .reads
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == FIRST + PORTRAIT_DATA)
            .count(),
        2
    );
}

#[test]
fn observed_portraits_reuse_only_certified_entries() {
    use crate::portrait_cache::{Cache, Changes};
    use std::cell::Cell;
    struct Observer {
        generation: Cell<u64>,
        enabled: Cell<bool>,
        change_after: Cell<Option<usize>>,
    }
    impl Changes for Observer {
        fn revision(&self, _: usize) -> Option<(u64, u64)> {
            if !self.enabled.get() {
                return None;
            }
            if let Some(remaining) = self.change_after.get() {
                self.change_after.set(remaining.checked_sub(1));
                if remaining == 0 {
                    self.generation.set(self.generation.get() + 1);
                }
            }
            Some((1, self.generation.get()))
        }
    }
    let observer = Observer {
        generation: Cell::new(1),
        enabled: Cell::new(true),
        change_after: Cell::new(None),
    };
    let mut bytes = fixture();
    array(&mut bytes, FIRST + PORTRAIT_DATA, 0x1800, 1);
    u32_at(&mut bytes, 0x1800, 2);
    u32_at(&mut bytes, 0x1804, 1);
    bytes[0x1808] = 1;
    u64_at(&mut bytes, 0x1800 + 0x10 + msvc_string::CAPACITY, 15);
    let mut cache = Cache::new(2).unwrap();
    let mut rows = vec![PortraitRow::default(); 2];
    let mut entries = vec![crate::character_portrait_entry::Row::<engine_native_memory::capture_text::Owned>::default(); 2];
    let capture = |source: &TracedMemory<'_>,
                   cache: &mut Cache,
                   rows: &mut Vec<PortraitRow>,
                   entries: &mut Vec<crate::character_portrait_entry::Row<engine_native_memory::capture_text::Owned>>| {
        walk_portraits(
            source,
            0,
            GS,
            rows,
            entries,
            PortraitWalk {
                observation: Some((cache, &observer)),
                ..Default::default()
            },
        )
    };
    assert_eq!(
        capture(
            &TracedMemory::new(&bytes, None),
            &mut cache,
            &mut rows,
            &mut entries
        )
        .outcome,
        Outcome::Rows { len: 2 }
    );
    let expected = entries.clone();
    let warm = TracedMemory::new(&bytes, Some(0x1800));
    assert_eq!(
        capture(&warm, &mut cache, &mut rows, &mut entries).outcome,
        Outcome::Rows { len: 2 }
    );
    assert_eq!(cache.counts.reused_entries, 1);
    assert_eq!(entries, expected);
    assert!(!warm.reads.borrow().iter().any(|&(at, _)| at == 0x1800));

    bytes[0x1808] = 7;
    observer.generation.set(2);
    assert_eq!(
        capture(
            &TracedMemory::new(&bytes, None),
            &mut cache,
            &mut rows,
            &mut entries
        )
        .outcome,
        Outcome::Rows { len: 2 }
    );
    assert_eq!(entries[0].is_set, 7);
    assert_eq!(cache.counts.reused_entries, 0);

    bytes[0x1808] = 9;
    observer.generation.set(3);
    let failed = capture(
        &TracedMemory::new(&bytes, Some(0x1800)),
        &mut cache,
        &mut rows,
        &mut entries,
    );
    assert!(matches!(failed.outcome, Outcome::Unavailable { .. }));
    capture(
        &TracedMemory::new(&bytes, None),
        &mut cache,
        &mut rows,
        &mut entries,
    );
    assert_eq!(cache.counts.reused_entries, 0);
    assert_eq!(entries[0].is_set, 9);

    observer.change_after.set(Some(1));
    let overlap = capture(
        &TracedMemory::new(&bytes, None),
        &mut cache,
        &mut rows,
        &mut entries,
    );
    assert_eq!(
        overlap.outcome,
        Outcome::Unavailable {
            reason: "character portrait mutated during acquisition",
        }
    );

    observer.enabled.set(false);
    let unavailable = capture(
        &TracedMemory::new(&bytes, Some(0x1800)),
        &mut cache,
        &mut rows,
        &mut entries,
    );
    assert!(matches!(unavailable.outcome, Outcome::Unavailable { .. }));
    assert_eq!(cache.counts.reused_entries, 0);
}

