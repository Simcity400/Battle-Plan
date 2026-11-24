use super::{
    reader::{Advanced, Cursor, LogCursor, LogEvent, Lives, class_of, fetch, log_rows, log_rows_at, log_span, log_state, log_state_named, rebuild, restore},
    *,
};
use crate::{
    state::Cell,
    store::{Store, TableState},
};
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

const KINDS: [Kind; 5] = [Kind::I32, Kind::U16, Kind::U32, Kind::OptionalI64, Kind::U32List];

fn store(settings: Settings) -> Store {
    let mut store = Store::new(vec![KINDS.to_vec()], vec![false]);
    store.attach(Encoder::new(&[KINDS.to_vec()], settings).unwrap());
    store
}

fn rows(table: &TableState) -> BTreeMap<u128, Vec<String>> {
    table
        .rows()
        .map(|slot| (table.key(slot).unwrap(), (0..KINDS.len()).map(|column| format!("{:?}", table.value(column, slot))).collect()))
        .collect()
}

static LISTS: [[u32; 8]; 4] = [[1, 2, 3, 4, 5, 6, 7, 8], [1, 2, 3, 9, 5, 6, 7, 8], [4, 4, 4, 4, 4, 4, 4, 4], [7, 7, 1, 2, 3, 4, 5, 6]];

fn hour(store: &mut Store, hour: u32, rows_held: &mut Vec<u32>) {
    store.begin_hour(hour as i32, &[]).unwrap();
    store.status(0, Status::Complete, true, Some(hour as i32), true).unwrap();
    let mut changes = store.table(0).unwrap();
    if hour % 50 == 7 && rows_held.len() > 3 {
        let slot = rows_held.remove(1);
        changes.delete(slot).unwrap();
    }
    if hour % 40 == 3 || rows_held.len() < 6 {
        let slot = (0..).find(|slot| !rows_held.contains(slot)).unwrap();
        changes.insert_at(slot, u128::from(hour) << 70 | u128::from(slot)).unwrap();
        changes.write(slot, 0, Cell::I32(hour as i32)).unwrap();
        changes.write(slot, 1, Cell::U16(0)).unwrap();
        changes.write(slot, 2, Cell::U32(7)).unwrap();
        changes.write(slot, 3, Cell::OptionalI64(Some(1))).unwrap();
        changes.write(slot, 4, Cell::U32List(&LISTS[0])).unwrap();
        rows_held.push(slot);
        rows_held.sort_unstable();
    }
    for (index, &slot) in rows_held.iter().enumerate() {
        let seed = hour as usize * 31 + index * 7;
        changes.write(slot, 0, Cell::I32((seed % 97) as i32 - 40)).unwrap();
        changes.add(slot, 1, 3).unwrap();
        if seed % 5 == 0 {
            changes.write(slot, 2, Cell::U32((seed * 2_654_435_761 % 1_000_003) as u32)).unwrap();
        }
        if seed % 11 == 0 {
            changes.write(slot, 3, Cell::OptionalI64(None)).unwrap();
        } else if seed % 3 == 0 {
            changes.write(slot, 3, Cell::OptionalI64(Some(seed as i64 * 1000))).unwrap();
        }
        if seed % 4 == 0 {
            changes.write(slot, 4, Cell::U32List(&LISTS[seed / 4 % 4][..4 + seed % 5])).unwrap();
        }
        if seed % 13 == 0 {
            changes.unrecord(slot, 2).unwrap();
        }
    }
}

fn run(settings: Settings, blocks: u32) -> (Memory, Vec<TableState>) {
    let mut store = store(settings);
    let mut memory = Memory::default();
    let mut held = Vec::new();
    let mut states = Vec::new();
    store.begin_run(0);
    for at in 0..blocks * BLOCK {
        hour(&mut store, at, &mut held);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
        states.push(store.table_state(0).unwrap().clone());
    }
    (memory, states)
}

fn read_back(memory: &Memory, states: &[TableState], step: u16) {
    let columns: Vec<usize> = (0..KINDS.len()).collect();
    let mut cells = states[0].clone();
    for (index, page) in memory.pages.iter().enumerate() {
        let entry = &page.tables[0];
        let start = if index == 0 { 1 } else { 0 };
        let mut cursor = Cursor::new(&KINDS, &columns, start);
        let mut next = 0;
        let mut written = vec![Vec::new(); columns.len()];
        let mut hour = start;
        while hour < page.hours {
            let to = (hour + step - 1).min(page.hours - 1);
            let mut lives = Lives::default();
            match cursor.advance(to, step, &mut cells, &mut written, &mut lives).unwrap() {
                Advanced::Reached => {
                    let at = index * BLOCK as usize + to as usize;
                    assert_eq!(rows(&cells), rows(&states[at]), "block {index} hour {to}");
                    hour = to + 1;
                }
                Advanced::Missing { .. } => {
                    let part = &entry.parts[next];
                    let last = entry.parts.get(next + 1).map_or(page.hours - 1, |following| following.first - 1);
                    let part = fetch(&memory.bytes, part, last, &columns, cursor.prefix().as_ref()).unwrap();
                    cursor.push(part);
                    next += 1;
                }
            }
        }
        if entry.packed {
            cells.pack().unwrap();
        }
    }
}

#[test]
fn every_model_and_list_form_reads_back() {
    let (memory, states) = run(Settings::at(3), 2);
    read_back(&memory, &states, 1);
    read_back(&memory, &states, 24);
    let page = BlockPage::read(&{ let mut out = Vec::new(); memory.pages[1].write(&mut out); out }, &mut 0).unwrap();
    assert_eq!(page, memory.pages[1], "a page reads back as written");
}

#[test]
fn parts_closed_by_cap_and_by_budget_read_back() {
    let settings = Settings { cap: 64, ..Settings::at(3) };
    let (memory, states) = run(settings, 1);
    assert!(memory.pages[0].tables[0].parts.len() > 3);
    read_back(&memory, &states, 5);
    let settings = Settings { budget: 256, ..Settings::at(3) };
    let (memory, states) = run(settings, 1);
    assert!(memory.pages[0].tables[0].parts.len() > 3);
    read_back(&memory, &states, 7);
    let settings = Settings { cap: 64, small: 1, ..Settings::at(3) };
    let (memory, states) = run(settings, 1);
    let parts = &memory.pages[0].tables[0].parts;
    let skipping = parts.windows(3).any(|three| {
        let streams = |part: &PartEntry| part.frames.iter().flat_map(|frame| frame.streams.clone()).collect::<Vec<u32>>();
        let (before, between, after) = (streams(&three[0]), streams(&three[1]), streams(&three[2]));
        after.iter().any(|stream| before.contains(stream) && !between.contains(stream))
    });
    assert!(skipping, "a stream missing from one part and in the parts on either side");
    read_back(&memory, &states, 5);
}

#[test]
fn reused_slots_and_short_lives_read_back() {
    let mut store = store(Settings::at(3));
    let mut memory = Memory::default();
    let mut states = Vec::new();
    store.begin_run(0);
    for at in 0..BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        store.status(0, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(0).unwrap();
        match at {
            0 => {
                changes.insert_at(3, 30).unwrap();
                changes.write(3, 0, Cell::I32(1)).unwrap();
            }
            10 => changes.delete(3).unwrap(),
            11 => {
                changes.insert_at(3, 31).unwrap();
                changes.write(3, 0, Cell::I32(50)).unwrap();
            }
            30 => {
                changes.insert_at(5, 50).unwrap();
                changes.write(5, 0, Cell::I32(9)).unwrap();
                changes.write(5, 4, Cell::U32List(&LISTS[0])).unwrap();
            }
            31 => {
                changes.write(5, 0, Cell::I32(10)).unwrap();
                changes.write(5, 4, Cell::U32List(&LISTS[1])).unwrap();
            }
            33 => changes.delete(5).unwrap(),
            _ if at % 2 == 0 && changes.held().live().get(3) => changes.write(3, 0, Cell::I32(at as i32)).unwrap(),
            _ => {}
        }
        store.end_hour(&mut memory).unwrap();
        states.push(store.table_state(0).unwrap().clone());
    }
    read_back(&memory, &states, 1);
    read_back(&memory, &states, 24);
}

#[test]
fn a_chained_part_without_its_predecessor_is_refused_by_name() {
    let settings = Settings { cap: 64, small: 1, ..Settings::at(3) };
    let (memory, _) = run(settings, 1);
    let entry = &memory.pages[0].tables[0];
    let part = entry.parts.iter().skip(1).find(|part| part.frames.iter().any(|frame| frame.chained)).expect("a chained part");
    let columns: Vec<usize> = (0..KINDS.len()).collect();
    let refused = fetch(&memory.bytes, part, BLOCK as u16 - 1, &columns, None).unwrap_err();
    assert!(refused.to_string().contains("without the part before it"), "{refused}");
}

#[test]
fn restores_against_a_whole_one_whose_chunks_grew_read_back() {
    let settings = Settings { restore_blocks: 1, whole_every: 4, ..Settings::at(3) };
    let (memory, states) = run(settings, 3);
    let columns: Vec<usize> = (0..KINDS.len()).collect();
    let first = memory.pages[0].tables[0].first.as_ref().expect("the run's first restore");
    assert!(first.whole);
    let (restored, _) = restore(&memory.bytes, first, &KINDS, &columns).unwrap();
    assert_eq!(rows(&restored), rows(&states[0]));
    for (index, page) in memory.pages.iter().enumerate() {
        let entry = page.tables[0].restore.as_ref().expect("a restore at every block's end");
        assert!(!entry.whole, "the first restore was whole");
        let named = |stream: u32, index: usize, chunk: &restore::Chunk| {
            first.streams.iter().find(|(held, _)| *held == stream).and_then(|(_, chunks)| chunks.get(index)).is_some_and(|whole| whole.frame == chunk.frame)
        };
        assert!(entry.streams.iter().all(|(stream, chunks)| chunks
            .iter()
            .enumerate()
            .all(|(index, chunk)| chunk.prefix.is_some() || chunk.frame.raw == 0 || chunk.first > 0 || named(*stream, index, chunk))));
        let (restored, _) = restore(&memory.bytes, entry, &KINDS, &columns).unwrap();
        assert_eq!(rows(&restored), rows(&states[(index + 1) * BLOCK as usize - 1]), "block {index}");
    }
}

#[test]
fn resume_after_a_block_writes_what_an_unbroken_run_writes() {
    for restore_blocks in [1, 3] {
        let settings = Settings { restore_blocks, ..Settings::at(3) };
        let (unbroken, _) = run(settings, 3);
        let mut store = store(settings);
        let mut memory = Memory::default();
        let mut held = Vec::new();
        store.begin_run(0);
        for at in 0..BLOCK {
            hour(&mut store, at, &mut held);
            store.end_hour(&mut memory).unwrap();
        }
        store.end_block();
        let saved = store.encoder().unwrap().save();
        let (table, _) = rebuild(&memory.bytes, &memory.pages, &KINDS, 0, 0, 0).unwrap();
        let mut resumed = Store::new(vec![KINDS.to_vec()], vec![false]);
        resumed.attach(Encoder::new(&[KINDS.to_vec()], settings).unwrap());
        resumed.begin_run(0);
        resumed.resume(vec![table], 0).unwrap();
        resumed.encoder_mut().unwrap().load(&saved).unwrap();
        for at in BLOCK..3 * BLOCK {
            hour(&mut resumed, at, &mut held);
            resumed.end_hour(&mut memory).unwrap();
            if at % BLOCK == BLOCK - 1 {
                resumed.end_block();
            }
        }
        assert_eq!(memory.bytes, unbroken.bytes, "restores every {restore_blocks} blocks");
        assert_eq!(memory.pages, unbroken.pages);
    }
}

#[test]
fn a_packed_table_is_flagged_and_reads_back() {
    let settings = Settings { restore_blocks: 1, whole_every: 1, pack_margin: 8, ..Settings::at(3) };
    let mut store = store(settings);
    let mut memory = Memory::default();
    let mut states = Vec::new();
    store.begin_run(0);
    for at in 0..2 * BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        store.status(0, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(0).unwrap();
        if at < 100 {
            changes.insert_at(at, u128::from(at) + 1000).unwrap();
            changes.write(at, 0, Cell::I32(at as i32)).unwrap();
        } else if at < 190 {
            changes.delete(at - 100).unwrap();
        } else if at >= BLOCK + 5 && at < BLOCK + 50 {
            for slot in changes.held().rows().collect::<Vec<_>>() {
                changes.write(slot, 0, Cell::I32(at as i32)).unwrap();
            }
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
        states.push(store.table_state(0).unwrap().clone());
    }
    assert!(memory.pages[0].tables[0].packed);
    assert_eq!(store.table_state(0).unwrap().slots(), 10);
    read_back(&memory, &states, 24);
}

fn insert_row(changes: &mut crate::store::TableChanges<'_>, slot: u32, key: u128, value: i32) {
    changes.insert_at(slot, key).unwrap();
    changes.write(slot, 0, Cell::I32(value)).unwrap();
    changes.write(slot, 1, Cell::U16(1)).unwrap();
    changes.write(slot, 2, Cell::U32(2)).unwrap();
    changes.write(slot, 3, Cell::OptionalI64(Some(3))).unwrap();
    changes.write(slot, 4, Cell::U32List(&LISTS[0])).unwrap();
}

fn chunk(entry: &restore::RestoreEntry, stream: u32, index: usize) -> &restore::Chunk {
    &entry.streams.iter().find(|(held, _)| *held == stream).expect("the stream").1[index]
}

#[test]
fn restores_name_clean_chunks_again_and_read_back() {
    let settings = Settings { restore_blocks: 1, whole_every: 4, ..Settings::at(3) };
    let mut store = store(settings);
    let mut memory = Memory::default();
    let mut states = Vec::new();
    store.begin_run(0);
    for at in 0..2 * BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        store.status(0, Status::Complete, true, Some(0), true).unwrap();
        let mut changes = store.table(0).unwrap();
        if at == 0 {
            for slot in 0..8 {
                insert_row(&mut changes, slot, 100 + u128::from(slot), slot as i32);
            }
        } else {
            changes.write(at % 8, 0, Cell::I32(at as i32)).unwrap();
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
        states.push(store.table_state(0).unwrap().clone());
    }
    let columns: Vec<usize> = (0..KINDS.len()).collect();
    let first = memory.pages[0].tables[0].first.clone().expect("the run's first restore");
    for (index, page) in memory.pages.iter().enumerate() {
        let entry = page.tables[0].restore.as_ref().expect("a restore at every block's end");
        assert!(!entry.whole);
        for stream in [restore::ROWS, restore::column_stream(1), restore::column_stream(2), restore::column_stream(3), restore::column_stream(4)] {
            let (held, whole) = (chunk(entry, stream, 0), chunk(&first, stream, 0));
            assert_eq!(held.frame, whole.frame, "block {index} stream {stream} named again");
            assert!(held.prefix.is_none());
        }
        let written = chunk(entry, restore::column_stream(0), 0);
        assert_eq!(written.prefix.as_ref(), Some(&chunk(&first, restore::column_stream(0), 0).frame), "a written chunk is against its whole one");
        let (restored, _) = restore(&memory.bytes, entry, &KINDS, &columns).unwrap();
        assert_eq!(rows(&restored), rows(&states[(index + 1) * BLOCK as usize - 1]), "block {index}");
    }
}

#[test]
fn restores_with_changed_keys_and_rows_past_their_chunks_read_back() {
    let settings = Settings { restore_blocks: 1, whole_every: 4, ..Settings::at(3) };
    let mut store = store(settings);
    let mut memory = Memory::default();
    let mut states = Vec::new();
    store.begin_run(0);
    let far = restore::ROWS_CHUNK + 4_464;
    for at in 0..3 * BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        store.status(0, Status::Complete, true, Some(0), true).unwrap();
        let mut changes = store.table(0).unwrap();
        match at {
            0 => (0..10).for_each(|slot| insert_row(&mut changes, slot, 1_000 + u128::from(slot), slot as i32)),
            5 => changes.delete(3).unwrap(),
            6 => insert_row(&mut changes, 3, 77_777, 6),
            10 => insert_row(&mut changes, far, 88_888, 10),
            800 => changes.delete(far).unwrap(),
            _ => changes.write(at % 10, 0, Cell::I32(at as i32)).unwrap(),
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
        states.push(store.table_state(0).unwrap().clone());
    }
    let columns: Vec<usize> = (0..KINDS.len()).collect();
    let block0 = memory.pages[0].tables[0].restore.as_ref().unwrap();
    let rows_chunks = &block0.streams.iter().find(|(stream, _)| *stream == restore::ROWS).unwrap().1;
    assert_eq!(rows_chunks.len(), 2, "a rows chunk of its own for the row past the whole one's");
    assert!(rows_chunks[0].prefix.is_some() && rows_chunks[1].prefix.is_none() && rows_chunks[1].first == restore::ROWS_CHUNK);
    for (index, page) in memory.pages.iter().enumerate() {
        let entry = page.tables[0].restore.as_ref().unwrap();
        let (restored, _) = restore(&memory.bytes, entry, &KINDS, &columns).unwrap();
        assert_eq!(rows(&restored), rows(&states[(index + 1) * BLOCK as usize - 1]), "block {index}");
    }
    let (table, _) = rebuild(&memory.bytes, &memory.pages, &KINDS, 0, 2, 0).unwrap();
    assert_eq!(rows(&table), rows(&states[3 * BLOCK as usize - 1]));
}

#[test]
fn cells_widen_in_the_middle_of_a_stream() {
    let mut store = store(Settings::at(3));
    let mut memory = Memory::default();
    let mut states = Vec::new();
    store.begin_run(0);
    for at in 0..BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        store.status(0, Status::Complete, true, Some(0), true).unwrap();
        let mut changes = store.table(0).unwrap();
        match at {
            0 => (0..6).for_each(|slot| insert_row(&mut changes, slot, 500 + u128::from(slot), 1)),
            30 => {
                for slot in 0..6 {
                    let value = if slot == 2 { -3_000_000 } else { slot as i32 };
                    changes.write(slot, 0, Cell::I32(value)).unwrap();
                    changes.write(slot, 2, Cell::U32(if slot == 3 { 4_000_000_000 } else { 9 })).unwrap();
                }
                changes.unrecord(4, 1).unwrap();
            }
            _ => {}
        }
        store.end_hour(&mut memory).unwrap();
        states.push(store.table_state(0).unwrap().clone());
    }
    read_back(&memory, &states, 1);
    read_back(&memory, &states, 24);
    read_back(&memory, &states, BLOCK as u16);
}

const TEXT: [Kind; 1] = [Kind::Bytes];
const NAMES: [&[u8]; 3] = [b"held", b"again", b"other"];

struct Named;

impl crate::store::StringSource for Named {
    fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
        Ok(NAMES.get(id as usize).map(|text| text.to_vec()))
    }
}

fn text_run(each: impl Fn(u32) -> (Status, Option<u32>)) -> Memory {
    let settings = Settings { restore_events: 2, restore_blocks: 99, whole_every: 4, ..Settings::at(3) };
    let mut store = Store::new(vec![TEXT.to_vec()], vec![false]);
    store.attach(Encoder::new(&[TEXT.to_vec()], settings).unwrap());
    store.read_strings_from(Box::new(Named));
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..2 * BLOCK {
        store.begin_hour(at as i32, if at == 0 { &NAMES[..] } else { &[] }).unwrap();
        let (status, text) = each(at);
        store.status(0, status, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(0).unwrap();
        if at == 0 {
            changes.insert_at(0, 7).unwrap();
        }
        if let Some(id) = text {
            changes.write(0, 0, Cell::Bytes(id)).unwrap();
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    memory
}

fn every_string_from_the_restore(memory: &Memory) {
    let restored = memory.pages[0].tables[0].restore.as_ref().expect("a restore at block 0's end");
    assert!(!restored.whole);
    assert!(memory.pages[1].tables[0].restore.is_none(), "block 1 ends without a restore");
    let columns = vec![0];
    let (mut cells, meta) = restore(&memory.bytes, restored, &TEXT, &columns).unwrap();
    let mut texts: BTreeMap<u32, Vec<u8>> = meta.strings.into_iter().collect();
    let page = &memory.pages[1];
    let entry = &page.tables[0];
    let mut cursor = Cursor::new(&TEXT, &columns, 0);
    let (mut written, mut lives, mut next) = (vec![Vec::new()], Lives::default(), 0);
    loop {
        match cursor.advance(page.hours - 1, BLOCK as u16, &mut cells, &mut written, &mut lives).unwrap() {
            Advanced::Reached => break,
            Advanced::Missing { .. } => {
                let Some(part) = entry.parts.get(next) else { break };
                let last = entry.parts.get(next + 1).map_or(page.hours - 1, |following| following.first - 1);
                let part = fetch(&memory.bytes, part, last, &columns, cursor.prefix().as_ref()).unwrap();
                texts.extend(part.strings().iter().cloned());
                cursor.push(part);
                next += 1;
            }
        }
    }
    let (held, ..) = reader::status(entry, page.hours - 1, BLOCK as i32).unwrap();
    cells.set_status(held, None);
    let (rebuilt, rebuilt_texts) = rebuild(&memory.bytes, &memory.pages, &TEXT, 0, 1, 0).unwrap();
    let rebuilt_texts: BTreeMap<u32, Vec<u8>> = rebuilt_texts.into_iter().collect();
    for (reader, table, texts) in [("a cursor", &cells, &texts), ("rebuild", &rebuilt, &rebuilt_texts)] {
        let mut named: Vec<u32> = table.rows().filter_map(|slot| match table.value(0, slot) {
            crate::store::Value::Text(id) => Some(id),
            _ => None,
        }).collect();
        if let Status::Unavailable { reason } = table.status() {
            named.push(reason);
        }
        assert!(!named.is_empty());
        for id in named {
            assert_eq!(texts.get(&id).map(Vec::as_slice), Some(NAMES[id as usize]), "{reader} from the restore holds string {id}'s bytes");
        }
    }
}

#[test]
fn a_string_named_again_after_a_restore_reads_from_that_restore() {
    let memory = text_run(|at| {
        let text = match at {
            0 | 10 => Some(0),
            5 => Some(1),
            at if at == BLOCK + 3 => Some(1),
            _ => None,
        };
        (Status::Complete, text)
    });
    every_string_from_the_restore(&memory);
}

#[test]
fn a_refusal_named_again_after_a_restore_reads_from_that_restore() {
    let memory = text_run(|at| {
        let status = if (5..10).contains(&at) || at >= BLOCK + 3 { Status::Unavailable { reason: 1 } } else { Status::Complete };
        let text = match at {
            0 | 21 => Some(0),
            20 => Some(2),
            _ => None,
        };
        (status, text)
    });
    every_string_from_the_restore(&memory);
}

#[test]
fn an_inserted_row_s_cell_of_a_column_without_a_stream_reads_not_recorded() {
    let settings = Settings { small: 0, ..Settings::at(3) };
    let mut store = store(settings);
    let mut memory = Memory::default();
    let mut states = Vec::new();
    store.begin_run(0);
    for at in 0..BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        store.status(0, Status::Complete, true, Some(at as i32), true).unwrap();
        let mut changes = store.table(0).unwrap();
        match at {
            0 => {
                for slot in 0..3 {
                    insert_row(&mut changes, slot, 100 + u128::from(slot), slot as i32);
                }
            }
            5 => changes.delete(1).unwrap(),
            10 => {
                changes.insert_at(1, 201).unwrap();
                changes.write(1, 0, Cell::I32(1)).unwrap();
                changes.insert_at(7, 207).unwrap();
                changes.write(7, 0, Cell::I32(7)).unwrap();
            }
            _ => {}
        }
        store.end_hour(&mut memory).unwrap();
        states.push(store.table_state(0).unwrap().clone());
    }
    store.end_block();
    let last = &states[BLOCK as usize - 1];
    for (slot, column) in [(1, 1), (1, 3), (1, 4), (7, 1), (7, 3), (7, 4)] {
        assert_eq!(last.value(column, slot), crate::store::Value::Unrecorded, "the store holds slot {slot} column {column} not recorded");
    }
    let entry = &mut memory.pages[0].tables[0];
    assert_eq!(entry.parts.len(), 1);
    let dropped = [column_stream(1), column_stream(3), column_stream(4)];
    let before = entry.parts[0].frames.len();
    entry.parts[0].frames.retain(|frame| !frame.streams.iter().any(|stream| dropped.contains(stream)));
    assert_eq!(entry.parts[0].frames.len(), before - 3, "each dropped stream was a frame of its own");
    read_back(&memory, &states, 1);
    read_back(&memory, &states, 24);
    let (rebuilt, _) = rebuild(&memory.bytes, &memory.pages, &KINDS, 0, 0, 0).unwrap();
    assert_eq!(rows(&rebuilt), rows(last));
}

const LOG_KINDS: [Kind; 6] = [Kind::I64, Kind::OptionalU32, Kind::Bool, Kind::Bytes, Kind::U32List, Kind::Opaque];

const LOG_TEXT: u32 = 1 << 30;

#[derive(Clone, Debug, Default, PartialEq)]
struct LogModel {
    appended: Vec<(i32, u64, u128, Vec<String>)>,
    forgotten: Vec<(i32, u64)>,
    ends: Vec<(u64, Vec<u64>, Status)>,
}

fn log_store(settings: Settings) -> Store {
    let kinds = vec![KINDS.to_vec(), LOG_KINDS.to_vec()];
    let mut store = Store::new(kinds.clone(), vec![false, false]);
    store.declare_logs(&[false, true]).unwrap();
    store.attach(Encoder::new(&kinds, settings).unwrap());
    store
}

fn pin_texts(store: &mut Store) {
    for id in 0..8 {
        store.pin_string(LOG_TEXT + id, format!("text {id}").as_bytes()).unwrap();
    }
}

fn log_hour(store: &mut Store, hour: u32, model: &mut LogModel) {
    log_hour_listing(store, hour, model, 40);
}

fn log_hour_listing(store: &mut Store, hour: u32, model: &mut LogModel, cap: usize) {
    static ITEMS: [u32; 6] = [3, 1, 4, 1, 5, 9];
    let status = if (300..340).contains(&hour) { Status::Unavailable { reason: LOG_TEXT + 7 } } else { Status::Complete };
    store.status(1, status, true, Some(hour as i32), true).unwrap();
    let mut changes = store.table(1).unwrap();
    let mut forgotten = Vec::new();
    let count = hour % 5 + u32::from(hour % 97 == 0);
    for index in 0..count {
        let seed = u64::from(hour) * 7 + u64::from(index);
        let opaque = [seed as u8, 0xAB, (seed >> 3) as u8];
        let cells = [
            Some(Cell::I64(seed as i64 * -1_000_003)),
            (seed % 4 != 0).then(|| Cell::OptionalU32((seed % 3 != 0).then_some(seed as u32 * 11))),
            Some(Cell::Bool(seed % 2 == 0)),
            (seed % 6 != 1).then_some(Cell::Bytes(LOG_TEXT + (seed % 7) as u32)),
            Some(Cell::U32List(&ITEMS[..(seed % 6) as usize])),
            (seed % 5 != 2).then_some(Cell::Opaque(&opaque)),
        ];
        let key = u128::from(hour) << 72 | u128::from(index) << 3 | 5;
        let ordinal = changes.append(key, &cells).unwrap();
        let shown = cells.iter().map(|cell| shown(*cell)).collect();
        model.appended.push((hour as i32, ordinal, key, shown));
        if hour % 97 == 0 && index + 1 == count {
            changes.forget(ordinal).unwrap();
            forgotten.push(ordinal);
        }
    }
    let listed: Vec<u64> = changes.held().listed().unwrap().ones().collect();
    for &ordinal in listed.iter().take(listed.len().saturating_sub(cap)) {
        changes.forget(ordinal).unwrap();
        forgotten.push(ordinal);
    }
    forgotten.sort_unstable();
    model.forgotten.extend(forgotten.into_iter().map(|ordinal| (hour as i32, ordinal)));
}

fn shown(cell: Option<Cell<'_>>) -> String {
    let items: Vec<u8>;
    let value = match cell {
        None => Value::Unrecorded,
        Some(Cell::I64(value)) => Value::Number(value as u64),
        Some(Cell::OptionalU32(value)) => value.map_or(Value::Missing, |value| Value::Number(value.into())),
        Some(Cell::Bool(value)) => Value::Number(value.into()),
        Some(Cell::Bytes(id)) => Value::Text(id),
        Some(Cell::U32List(list)) => {
            items = list.iter().flat_map(|item| item.to_le_bytes()).collect();
            Value::List(&items)
        }
        Some(Cell::Opaque(bytes)) => Value::List(bytes),
        Some(other) => unreachable!("no log column holds {other:?}"),
    };
    format!("{value:?}")
}

fn log_run(settings: Settings, blocks: u32) -> (Memory, Vec<TableState>, LogModel) {
    let mut store = log_store(settings);
    let mut memory = Memory::default();
    let (mut held, mut states, mut model) = (Vec::new(), Vec::new(), LogModel::default());
    store.begin_run(0);
    pin_texts(&mut store);
    for at in 0..blocks * BLOCK {
        hour(&mut store, at, &mut held);
        log_hour(&mut store, at, &mut model);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
            let log = store.table_state(1).unwrap();
            model.ends.push((log.count().unwrap(), log.listed().unwrap().ones().collect(), log.status()));
        }
        states.push(store.table_state(0).unwrap().clone());
    }
    (memory, states, model)
}

fn read_log(memory: &Memory) -> LogModel {
    let columns: Vec<usize> = (0..LOG_KINDS.len()).collect();
    let mut read = LogModel::default();
    let mut named = HashMap::new();
    log_span(&memory.bytes, &memory.pages, &LOG_KINDS, 1, &columns, 0, 0..=i32::MAX, &mut |hour, event| match event {
        LogEvent::Appended { ordinal, key, rows, at, .. } => {
            if let Value::Text(id) = rows.value(3, at) {
                assert!(named.contains_key(&id), "string {id} is handed before row {ordinal}, the first naming it");
            }
            read.appended.push((hour, ordinal, key, (0..LOG_KINDS.len()).map(|column| format!("{:?}", rows.value(column, at))).collect()));
        }
        LogEvent::Forgotten { from, to, .. } => read.forgotten.extend((from..to).map(|ordinal| (hour, ordinal))),
        LogEvent::Named { id, text } => assert!(named.insert(id, text.to_vec()).is_none(), "string {id} is handed once a call"),
    })
    .unwrap();
    for (id, text) in named {
        assert_eq!(text, format!("text {}", id - LOG_TEXT).into_bytes());
    }
    for at in 0..memory.pages.len() {
        let state = log_state(&memory.bytes, &memory.pages, &LOG_KINDS, 1, at).unwrap();
        read.ends.push((state.count().unwrap(), state.listed().unwrap().ones().collect(), state.status()));
    }
    read
}

#[test]
fn a_log_s_rows_read_back_by_span_and_by_ordinal() {
    for (settings, closes) in [
        (Settings::at(3), "a block's end"),
        (Settings { cap: 48, ..Settings::at(3) }, "the cap"),
        (Settings { budget: 600, ..Settings::at(3) }, "the budget"),
    ] {
        let (memory, states, model) = log_run(settings, 2);
        let entries: Vec<&TableEntry> = memory.pages.iter().map(|page| &page.tables[1]).collect();
        assert!(entries.iter().all(|entry| entry.log && !memory.pages[0].tables[0].log), "{closes}");
        if closes != "a block's end" {
            assert!(entries[0].parts.len() > 3, "{closes}: {} parts", entries[0].parts.len());
        }
        assert_eq!(entries[0].parts[0].first, 0, "the run's first hour appends into its first part");
        assert_eq!(entries[1].count, model.ends[0].0, "{closes}: a block's entry holds the count at its start");
        let read = read_log(&memory);
        assert_eq!(read.appended.len(), model.appended.len(), "{closes}");
        assert_eq!(read, model, "{closes}");
        assert!(model.forgotten.iter().any(|&(hour, ordinal)| hour == 0 && model.appended.iter().any(|row| row.0 == 0 && row.1 == ordinal)), "the first hour forgets a row it appended");
        let first = memory.pages[0].tables[1].first.as_ref().expect("the run's first restore");
        let (at_first, _) = restore(&memory.bytes, first, &LOG_KINDS, &[]).unwrap();
        let first_count = model.appended.iter().filter(|row| row.0 == 0).count() as u64;
        assert_eq!(at_first.count(), Some(first_count), "the first restore counts the first hour's rows");
        assert_eq!(at_first.listed().unwrap().len() as u64, first_count - 1, "and lists none it forgot");
        let columns: Vec<usize> = (0..LOG_KINDS.len()).collect();
        let (from, to) = (model.ends[0].0 - 7, model.ends[0].0 + 9);
        let mut by_ordinal = Vec::new();
        log_rows(&memory.bytes, &memory.pages, &LOG_KINDS, 1, &columns, from..to, &mut |event| {
            if let LogEvent::Appended { hour, ordinal, key, rows, at } = event {
                let block = by_ordinal_block(&memory, ordinal);
                by_ordinal.push((block + i32::from(hour), ordinal, key, (0..LOG_KINDS.len()).map(|column| format!("{:?}", rows.value(column, at))).collect::<Vec<_>>()));
            }
        })
        .unwrap();
        let wanted: Vec<_> = model.appended.iter().filter(|row| (from..to).contains(&row.1)).cloned().collect();
        assert_eq!(by_ordinal, wanted, "{closes}: rows {from}..{to} by ordinal, across a block's end");
        for (index, page) in memory.pages.iter().enumerate() {
            let entry = &page.tables[1];
            let (mut named, mut prefix) = (std::collections::HashSet::new(), None);
            for (at, part) in entry.parts.iter().enumerate() {
                let last = entry.parts.get(at + 1).map_or(page.hours - 1, |following| following.first - 1);
                let part = fetch(&memory.bytes, part, last, &[3], prefix.as_ref()).unwrap();
                named.extend(part.strings().iter().map(|(id, _)| *id));
                prefix = Some(part.prefix());
            }
            let block = (index as u32 * BLOCK) as i32..((index as u32 + 1) * BLOCK) as i32;
            for row in model.appended.iter().filter(|row| block.contains(&row.0)) {
                if let Some(id) = row.3[3].strip_prefix("Text(").and_then(|id| id.strip_suffix(')')) {
                    assert!(named.contains(&id.parse::<u32>().unwrap()), "{closes}: block {index} names string {id}");
                }
            }
        }
        read_back(&memory, &states, 24);
        for page in &memory.pages {
            let mut out = Vec::new();
            page.write(&mut out);
            assert_eq!(&BlockPage::read(&out, &mut 0).unwrap(), page, "{closes}: a page reads back as written");
        }
    }
}

fn by_ordinal_block(memory: &Memory, ordinal: u64) -> i32 {
    let at = memory.pages.iter().rposition(|page| page.tables[1].count <= ordinal).unwrap();
    (memory.pages[at].block * BLOCK) as i32
}

#[test]
fn a_log_is_restored_at_every_block_end_and_named_again_when_unchanged() {
    let mut store = log_store(Settings::at(3));
    let mut memory = Memory::default();
    store.begin_run(0);
    pin_texts(&mut store);
    for at in 0..4 * BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        let status = if at >= 3 * BLOCK { Status::Truncated { missing: 1 } } else { Status::Complete };
        store.status(1, status, true, Some(0), true).unwrap();
        if at < BLOCK {
            let mut changes = store.table(1).unwrap();
            changes.append(u128::from(at), &[Some(Cell::I64(at.into())), None, None, None, None, None]).unwrap();
            if at % 3 == 0 {
                changes.forget(u64::from(at)).unwrap();
            }
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    let restores: Vec<&restore::RestoreEntry> = memory.pages.iter().map(|page| page.tables[1].restore.as_ref().expect("a restore at every block's end")).collect();
    let streams = |entry: &restore::RestoreEntry| entry.streams.iter().map(|(stream, _)| *stream).collect::<Vec<_>>();
    assert_eq!(streams(restores[0]), vec![restore::META, restore::ROWS], "a log's restore is its meta and live set");
    assert_eq!(restores[1], restores[0], "unchanged: named again");
    assert_ne!(restores[3], restores[0], "a status change restores it");
    let frames = |entry: &restore::RestoreEntry| entry.streams.iter().flat_map(|(_, chunks)| chunks.iter().map(|chunk| chunk.frame.offset)).collect::<Vec<_>>();
    assert!(frames(restores[3]).iter().all(|offset| !frames(restores[0]).contains(offset)));
    for at in 0..4 {
        let state = log_state(&memory.bytes, &memory.pages, &LOG_KINDS, 1, at).unwrap();
        assert_eq!(state.count(), Some(u64::from(BLOCK)));
        assert_eq!(state.listed().unwrap().len(), (BLOCK - BLOCK.div_ceil(3)) as usize);
        assert_eq!(state.status(), if at == 3 { Status::Truncated { missing: 1 } } else { Status::Complete });
    }
    let refused = rebuild(&memory.bytes, &memory.pages, &LOG_KINDS, 1, 3, 0).unwrap_err().to_string();
    assert!(refused.contains("Table 1 is a log: it has rows by hour, not a state"), "{refused}");
    let refused = log_state(&memory.bytes, &memory.pages, &KINDS, 0, 3).unwrap_err().to_string();
    assert!(refused.contains("Table 0 holds state"), "{refused}");
    let mut mixed = memory.pages.clone();
    mixed[2].tables[1].log = false;
    let refused = class_of(&mixed, 1).unwrap_err().to_string();
    assert!(refused.contains("a log in one block of its run and holds state in another"), "{refused}");
}

#[test]
fn a_log_s_replay_at_each_block_end_equals_its_restore() {
    let mut store = log_store(Settings::at(3));
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..2 * BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        store.status(1, Status::Complete, true, Some(at as i32), true).unwrap();
        match at {
            1 => {
                let mut changes = store.table(1).unwrap();
                for key in 0..130_u128 {
                    changes.append(key, &[None; 6]).unwrap();
                }
            }
            2 => store.table(1).unwrap().forget_run(64, 66).unwrap(),
            _ if at == BLOCK + 5 => store.table(1).unwrap().forget_run(0, 64).unwrap(),
            _ => {}
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    let columns: Vec<usize> = (0..LOG_KINDS.len()).collect();
    let (mut count, mut listed) = (0_u64, crate::store::Listed::default());
    for (index, page) in memory.pages.iter().enumerate() {
        let first = (page.block * BLOCK) as i32;
        log_span(&memory.bytes, &memory.pages, &LOG_KINDS, 1, &columns, 0, first..=first + i32::from(page.hours) - 1, &mut |_, event| match event {
            LogEvent::Appended { ordinal, .. } => {
                count += 1;
                listed.append(ordinal);
            }
            LogEvent::Forgotten { from, to, .. } => listed.clear_range(from, to),
            LogEvent::Named { .. } => {}
        })
        .unwrap();
        let state = log_state(&memory.bytes, &memory.pages, &LOG_KINDS, 1, index).unwrap();
        assert_eq!(state.count(), Some(count), "block {index}");
        let restored = state.listed().unwrap();
        assert_eq!(restored.ones().collect::<Vec<_>>(), listed.ones().collect::<Vec<_>>(), "block {index}: the same rows listed");
        assert_eq!(restored, &listed, "block {index}: live sets listing the same rows are equal");
    }
    assert!(listed.is_empty(), "the second block forgot every row");
}

#[test]
fn resume_with_a_log_writes_what_an_unbroken_run_writes() {
    for (settings, burst) in [(Settings { cap: 64, ..Settings::at(3) }, false), (Settings { budget: 4096, ..Settings::at(3) }, true)] {
        let step = |store: &mut Store, at: u32, held: &mut Vec<u32>, model: &mut LogModel| {
            hour(store, at, held);
            log_hour(store, at, model);
            if burst && at == BLOCK / 2 {
                let mut changes = store.table(1).unwrap();
                for index in 0..2000_u128 {
                    let ordinal = changes.append(u128::from(at) << 72 | index << 40 | 1, &[None; 6]).unwrap();
                    if index % 2 == 0 {
                        changes.forget(ordinal).unwrap();
                    }
                }
            }
        };
        let mut unbroken = Memory::default();
        {
            let mut store = log_store(settings);
            let (mut held, mut model) = (Vec::new(), LogModel::default());
            store.begin_run(0);
            pin_texts(&mut store);
            for at in 0..3 * BLOCK {
                step(&mut store, at, &mut held, &mut model);
                store.end_hour(&mut unbroken).unwrap();
                if at % BLOCK == BLOCK - 1 {
                    store.end_block();
                }
            }
        }
        if burst {
            assert!(unbroken.pages[1].tables[1].parts.len() > 3, "the budget closes the log's parts");
        }
        for killed_at in [None, Some(BLOCK + 300)] {
            let mut store = log_store(settings);
            let mut memory = Memory::default();
            let (mut held, mut model) = (Vec::new(), LogModel::default());
            store.begin_run(0);
            pin_texts(&mut store);
            let mut commit = None;
            for at in 0..killed_at.unwrap_or(BLOCK) {
                step(&mut store, at, &mut held, &mut model);
                store.end_hour(&mut memory).unwrap();
                if at % BLOCK == BLOCK - 1 {
                    store.end_block();
                    commit = Some((memory.bytes.len(), memory.pages.len(), store.encoder().unwrap().save(), held.clone()));
                }
            }
            let (length, pages, saved, held_then) = commit.expect("a commit");
            if killed_at.is_some() {
                assert!(memory.bytes.len() > length, "the kill leaves frames past the commit");
            }
            memory.bytes.truncate(length);
            memory.pages.truncate(pages);
            drop(store);
            let (state, _) = rebuild(&memory.bytes, &memory.pages, &KINDS, 0, pages - 1, 0).unwrap();
            let log = log_state(&memory.bytes, &memory.pages, &LOG_KINDS, 1, pages - 1).unwrap();
            let mut resumed = log_store(settings);
            resumed.begin_run(0);
            pin_texts(&mut resumed);
            resumed.resume(vec![state, log], 0).unwrap();
            resumed.encoder_mut().unwrap().load(&saved).unwrap();
            let mut held = held_then;
            let mut model = LogModel::default();
            for at in BLOCK..3 * BLOCK {
                step(&mut resumed, at, &mut held, &mut model);
                resumed.end_hour(&mut memory).unwrap();
                if at % BLOCK == BLOCK - 1 {
                    resumed.end_block();
                }
            }
            assert_eq!(memory.pages, unbroken.pages, "budget {}, killed at {killed_at:?}", settings.budget);
            assert!(memory.bytes == unbroken.bytes, "budget {}, killed at {killed_at:?}: the resumed file differs", settings.budget);
        }
    }
}

#[test]
fn resume_refuses_a_table_of_another_class_by_name() {
    let mut resumed = log_store(Settings::at(3));
    resumed.begin_run(0);
    let refused = resumed.resume(vec![TableState::new(&KINDS), TableState::new(&LOG_KINDS)], 0).unwrap_err().to_string();
    assert!(refused.contains("Table 1 is state in the file and a log in this build"), "{refused}");
    let refused = resumed.resume(vec![TableState::log(&KINDS, 3, Default::default()), TableState::log(&LOG_KINDS, 3, Default::default())], 0).unwrap_err().to_string();
    assert!(refused.contains("Table 0 is a log in the file and state in this build"), "{refused}");
}

#[test]
fn a_cursor_refuses_a_log_s_part_and_a_log_cursor_a_state_table_s() {
    let (memory, _, _) = log_run(Settings::at(3), 1);
    let entries = &memory.pages[0].tables;
    let state_columns: Vec<usize> = (0..KINDS.len()).collect();
    let log_columns: Vec<usize> = (0..LOG_KINDS.len()).collect();
    let log_part = &entries[1].parts[0];
    let state_part = &entries[0].parts[0];
    let mut cursor = Cursor::new(&KINDS, &state_columns, 0);
    cursor.push(fetch(&memory.bytes, log_part, BLOCK as u16 - 1, &log_columns, None).unwrap());
    let mut cells = TableState::new(&KINDS);
    let (mut written, mut lives) = (vec![Vec::new(); KINDS.len()], Lives::default());
    let refused = cursor.advance(BLOCK as u16 - 1, BLOCK as u16, &mut cells, &mut written, &mut lives).unwrap_err();
    assert!(refused.to_string().contains("is read with a LogCursor"), "{refused}");
    let mut logs = super::reader::LogCursor::new(&LOG_KINDS, &log_columns, 0);
    logs.push(fetch(&memory.bytes, state_part, BLOCK as u16 - 1, &state_columns, None).unwrap());
    let refused = logs.advance(BLOCK as u16 - 1, &mut |_| {}).unwrap_err();
    assert!(refused.to_string().contains("is read with a Cursor"), "{refused}");
}

#[test]
fn a_log_passes_two_to_the_thirty_two_rows() {
    let start = u64::from(u32::MAX) - 3;
    let settings = Settings { cap: 64, ..Settings::at(3) };
    let begin = || {
        let mut store = log_store(settings);
        store.begin_run(0);
        pin_texts(&mut store);
        let mut listed = crate::store::Listed::default();
        listed.append(start - 3);
        listed.append(start - 1);
        store.resume(vec![TableState::new(&KINDS), TableState::log(&LOG_KINDS, start, listed)], 0).unwrap();
        store
    };
    let mut store = begin();
    let mut memory = Memory::default();
    let (mut held, mut model) = (Vec::new(), LogModel::default());
    let mut commit = None;
    for at in 0..2 * BLOCK {
        hour(&mut store, at, &mut held);
        log_hour(&mut store, at, &mut model);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
            let log = store.table_state(1).unwrap();
            model.ends.push((log.count().unwrap(), log.listed().unwrap().ones().collect(), log.status()));
            if commit.is_none() {
                commit = Some((memory.bytes.len(), memory.pages.len(), store.encoder().unwrap().save(), held.clone()));
            }
        }
    }
    drop(store);
    let boundary = 1_u64 << 32;
    assert!(model.appended.iter().any(|row| row.1 < boundary) && model.appended.iter().any(|row| row.1 > boundary + 100), "appends cross 2^32");
    assert!(model.forgotten.iter().any(|row| row.1 < boundary) && model.forgotten.iter().any(|row| row.1 > boundary), "forgets cross 2^32");
    assert!(memory.pages[0].tables[1].parts.len() > 3, "the cap closes parts");
    assert_eq!(memory.pages[0].tables[1].count, start);
    assert!(memory.pages[1].tables[1].count > boundary);
    assert_eq!(read_log(&memory), model, "read back by span, and the count and live set at each block's end");
    let columns: Vec<usize> = (0..LOG_KINDS.len()).collect();
    let (from, to) = (boundary - 5, memory.pages[1].tables[1].count + 5);
    let mut by_ordinal = Vec::new();
    log_rows(&memory.bytes, &memory.pages, &LOG_KINDS, 1, &columns, from..to, &mut |event| {
        if let LogEvent::Appended { ordinal, key, rows, at, .. } = event {
            by_ordinal.push((ordinal, key, (0..LOG_KINDS.len()).map(|column| format!("{:?}", rows.value(column, at))).collect::<Vec<_>>()));
        }
    })
    .unwrap();
    let wanted: Vec<_> = model.appended.iter().filter(|row| (from..to).contains(&row.1)).map(|row| (row.1, row.2, row.3.clone())).collect();
    assert!(wanted.len() > 10);
    assert_eq!(by_ordinal, wanted, "rows {from}..{to} by ordinal, across 2^32 and a block's end");
    let asked: Vec<u64> = wanted.iter().map(|row| row.0).step_by(3).collect();
    let every = cursor_rows(&memory, &columns);
    let (at, _) = rows_at(&memory, &columns, &asked).unwrap();
    assert_eq!(at, asked.iter().map(|ordinal| (*ordinal, every[ordinal].clone())).collect::<Vec<_>>(), "rows asked across 2^32 and a block's end equal the cursor's");
    let refused = rows_at(&memory, &columns, &[start - 1, start]).unwrap_err().to_string();
    assert!(refused.contains(&format!("Log table 1 appended no row {} in this run", start - 1)), "{refused}");
    for page in &memory.pages {
        let mut out = Vec::new();
        page.write(&mut out);
        assert_eq!(&BlockPage::read(&out, &mut 0).unwrap(), page);
    }
    let (length, pages, saved, held_then) = commit.expect("a commit");
    let mut killed = Memory { bytes: memory.bytes[..length].to_vec(), pages: memory.pages[..pages].to_vec() };
    let (state, _) = rebuild(&killed.bytes, &killed.pages, &KINDS, 0, pages - 1, 0).unwrap();
    let log = log_state(&killed.bytes, &killed.pages, &LOG_KINDS, 1, pages - 1).unwrap();
    assert!(log.count().unwrap() > boundary);
    let mut resumed = log_store(settings);
    resumed.begin_run(0);
    pin_texts(&mut resumed);
    resumed.resume(vec![state, log], 0).unwrap();
    resumed.encoder_mut().unwrap().load(&saved).unwrap();
    let (mut held, mut ignored) = (held_then, LogModel::default());
    for at in BLOCK..2 * BLOCK {
        hour(&mut resumed, at, &mut held);
        log_hour(&mut resumed, at, &mut ignored);
        resumed.end_hour(&mut killed).unwrap();
        if at % BLOCK == BLOCK - 1 {
            resumed.end_block();
        }
    }
    assert_eq!(killed.pages, memory.pages);
    assert!(killed.bytes == memory.bytes, "the resumed file differs");
}

fn forgets_by(run: bool, refused_too: bool) -> (Memory, Vec<u64>) {
    let mut store = log_store(Settings { cap: 200, ..Settings::at(3) });
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..BLOCK {
        store.begin_hour(at as i32, &[]).unwrap();
        store.status(1, Status::Complete, true, Some(0), true).unwrap();
        let mut changes = store.table(1).unwrap();
        for index in 0..5_u64 {
            changes.append(u128::from(at) << 8 | u128::from(index), &[Some(Cell::I64(i64::from(at))), None, None, None, None, None]).unwrap();
        }
        if at % 10 == 9 {
            let lowest = changes.held().listed().unwrap().lowest().unwrap();
            if refused_too && lowest >= 2 {
                let count = changes.held().count().unwrap();
                let refused = changes.forget_run(lowest - 2, 30).unwrap_err().to_string();
                assert!(refused.contains("forgot before"), "{refused}");
                let refused = changes.forget_run(count - 3, 30).unwrap_err().to_string();
                assert!(refused.contains("never appended"), "{refused}");
                let refused = changes.forget_run(u64::MAX - 2, 30).unwrap_err().to_string();
                assert!(refused.contains("never appended"), "{refused}");
                assert_eq!(changes.held().listed().unwrap().lowest(), Some(lowest), "a refused run changes nothing");
            }
            if run {
                changes.forget_run(lowest, 30).unwrap();
            } else {
                for ordinal in lowest..lowest + 30 {
                    changes.forget(ordinal).unwrap();
                }
            }
            assert_eq!(store.log().tables[1].forgotten.last(), Some(&(lowest..lowest + 30)), "the hour's log holds the run");
        }
        store.end_hour(&mut memory).unwrap();
    }
    store.end_block();
    let listed = store.table_state(1).unwrap().listed().unwrap().ones().collect();
    (memory, listed)
}

#[test]
fn forget_run_writes_what_forgetting_one_at_a_time_writes() {
    let (one_at_a_time, listed) = forgets_by(false, false);
    let (run, run_listed) = forgets_by(true, false);
    let (refused, refused_listed) = forgets_by(true, true);
    assert!(listed.len() < 5 * BLOCK as usize && !listed.is_empty());
    assert_eq!((&run.pages, &run_listed), (&one_at_a_time.pages, &listed));
    assert!(run.bytes == one_at_a_time.bytes, "a run writes what single forgets write");
    assert_eq!((&refused.pages, &refused_listed), (&one_at_a_time.pages, &listed));
    assert!(refused.bytes == one_at_a_time.bytes, "refused runs write nothing");
    let mut store = log_store(Settings::at(3));
    store.begin_hour(0, &[]).unwrap();
    let refused = store.table(0).unwrap().forget_run(0, 3).unwrap_err().to_string();
    assert!(refused.contains("holds state"), "{refused}");
}

#[test]
fn an_hour_s_forgets_may_come_in_any_order_among_its_appends() {
    let row = |at: u32| [Some(Cell::I64(i64::from(at))), None, None, None, None, None];
    let unit = |interleaved: bool| {
        let mut store = log_store(Settings::at(3));
        let mut memory = Memory::default();
        store.begin_run(0);
        for at in 0..BLOCK {
            store.begin_hour(at as i32, &[]).unwrap();
            store.status(1, Status::Complete, true, Some(0), true).unwrap();
            let mut changes = store.table(1).unwrap();
            if at < 10 {
                changes.append(u128::from(at), &row(at)).unwrap();
            } else if at == 10 {
                if interleaved {
                    changes.forget(7).unwrap();
                    assert_eq!(changes.append(100, &row(at)).unwrap(), 10);
                    changes.forget(2).unwrap();
                    changes.append(101, &row(at)).unwrap();
                    changes.forget(10).unwrap();
                    changes.append(102, &row(at)).unwrap();
                    changes.forget(3).unwrap();
                } else {
                    for key in 100..103 {
                        changes.append(key, &row(at)).unwrap();
                    }
                    for ordinal in [2, 3, 7, 10] {
                        changes.forget(ordinal).unwrap();
                    }
                }
            }
            store.end_hour(&mut memory).unwrap();
        }
        store.end_block();
        assert_eq!(store.table_state(1).unwrap().listed().unwrap().ones().collect::<Vec<_>>(), vec![0, 1, 4, 5, 6, 8, 9, 11, 12]);
        memory
    };
    let (interleaved, ordered) = (unit(true), unit(false));
    assert_eq!(interleaved.pages, ordered.pages);
    assert!(interleaved.bytes == ordered.bytes, "an hour's forgets are written ascending, after its appends");
    let mut seen = Vec::new();
    log_span(&interleaved.bytes, &interleaved.pages, &LOG_KINDS, 1, &[0], 0, 10..=10, &mut |_, event| match event {
        LogEvent::Appended { ordinal, .. } => seen.push(format!("+{ordinal}")),
        LogEvent::Forgotten { from, to, .. } => seen.push(format!("-{from}..{to}")),
        LogEvent::Named { .. } => {}
    })
    .unwrap();
    assert_eq!(seen, ["+10", "+11", "+12", "-2..4", "-7..8", "-10..11"]);
}

type Seen = (i32, u128, Vec<String>);

fn cursor_rows(memory: &Memory, columns: &[usize]) -> BTreeMap<u64, Seen> {
    let mut rows = BTreeMap::new();
    for page in &memory.pages {
        let entry = &page.tables[1];
        let Some(first) = entry.parts.first() else { continue };
        let start = (page.block * BLOCK) as i32;
        let mut cursor = LogCursor::new(&LOG_KINDS, columns, first.first);
        let mut next = 0;
        loop {
            let advanced = cursor
                .advance(page.hours - 1, &mut |event| {
                    if let LogEvent::Appended { hour, ordinal, key, rows: held, at } = event {
                        rows.insert(ordinal, (start + i32::from(hour), key, columns.iter().map(|&column| format!("{:?}", held.value(column, at))).collect()));
                    }
                })
                .unwrap();
            match advanced {
                Advanced::Reached => break,
                Advanced::Missing { .. } => {
                    let Some(part) = entry.parts.get(next) else { break };
                    let last = entry.parts.get(next + 1).map_or(page.hours - 1, |following| following.first - 1);
                    cursor.push(fetch(&memory.bytes, part, last, columns, cursor.prefix().as_ref()).unwrap());
                    next += 1;
                }
            }
        }
    }
    rows
}

fn rows_at(memory: &Memory, columns: &[usize], ordinals: &[u64]) -> io::Result<(Vec<(u64, Seen)>, HashMap<u32, Vec<u8>>)> {
    let (mut read, mut named) = (Vec::new(), HashMap::new());
    log_rows_at(&memory.bytes, &memory.pages, &LOG_KINDS, 1, columns, ordinals, &mut |event| match event {
        LogEvent::Appended { hour, ordinal, key, rows, at } => {
            for &column in columns {
                if let Value::Text(id) = rows.value(column, at) {
                    assert!(named.contains_key(&id), "string {id} is handed before row {ordinal}, the first naming it");
                }
            }
            read.push((ordinal, (by_ordinal_block(memory, ordinal) + i32::from(hour), key, columns.iter().map(|&column| format!("{:?}", rows.value(column, at))).collect())));
        }
        LogEvent::Named { id, text } => assert!(named.insert(id, text.to_vec()).is_none(), "string {id} is handed once a call"),
        LogEvent::Forgotten { .. } => panic!("rows by ordinal hand no forget"),
    })?;
    Ok((read, named))
}

fn text_of(cell: &str) -> Option<u32> {
    cell.strip_prefix("Text(")?.strip_suffix(')')?.parse().ok()
}

#[test]
fn rows_asked_by_ordinal_equal_the_cursor_s() {
    for (settings, closes) in [(Settings::at(3), "a block's end"), (Settings { cap: 48, ..Settings::at(3) }, "the cap")] {
        let (memory, _, model) = log_run(settings, 2);
        let count = model.appended.len() as u64;
        if closes == "the cap" {
            assert!(memory.pages.iter().all(|page| page.tables[1].parts.len() > 3), "the cap closes parts");
        }
        let gone: Vec<u64> = model.appended.iter().filter(|row| model.forgotten.contains(&(row.0, row.1))).map(|row| row.1).collect();
        assert!(gone.len() > 4, "rows appended and forgotten in one hour");
        let block = memory.pages[1].tables[1].count;
        let mut ordinals: Vec<u64> = (0..count).step_by(7).chain(gone).chain([0, 1, block - 1, block, count - 1]).collect();
        ordinals.sort_unstable();
        ordinals.dedup();
        for columns in [vec![0, 1, 2, 3, 4, 5], vec![3, 0], vec![5, 4]] {
            let every = cursor_rows(&memory, &columns);
            assert_eq!(every.len() as u64, count);
            let (read, _) = rows_at(&memory, &columns, &ordinals).unwrap();
            let wanted: Vec<(u64, Seen)> = ordinals.iter().map(|ordinal| (*ordinal, every[ordinal].clone())).collect();
            assert_eq!(read, wanted, "{closes}, columns {columns:?}");
            let mut ranged = Vec::new();
            log_rows(&memory.bytes, &memory.pages, &LOG_KINDS, 1, &columns, 0..u64::MAX, &mut |event| {
                if let LogEvent::Appended { hour, ordinal, key, rows, at } = event {
                    ranged.push((ordinal, (by_ordinal_block(&memory, ordinal) + i32::from(hour), key, columns.iter().map(|&column| format!("{:?}", rows.value(column, at))).collect::<Vec<_>>())));
                }
            })
            .unwrap();
            assert_eq!(ranged, every.into_iter().collect::<Vec<_>>(), "{closes}, columns {columns:?}: every row by range");
        }
    }
}

#[test]
fn rows_asked_by_ordinal_are_refused_by_name() {
    let (memory, _, model) = log_run(Settings { cap: 48, ..Settings::at(3) }, 2);
    let count = model.appended.len() as u64;
    let columns = [0, 3];
    let refused = |ordinals: &[u64]| rows_at(&memory, &columns, ordinals).unwrap_err().to_string();
    let past = refused(&[3, count]);
    assert!(past.contains(&format!("Log table 1 appended no row {count} in this run")), "{past}");
    let twice = refused(&[3, 3]);
    assert!(twice.contains("Rows of log table 1 are asked twice or out of order: 3 after 3"), "{twice}");
    let disorder = refused(&[9, 4]);
    assert!(disorder.contains("asked twice or out of order: 4 after 9"), "{disorder}");
    let state = log_rows_at(&memory.bytes, &memory.pages, &KINDS, 0, &[0], &[0], &mut |_| {}).unwrap_err().to_string();
    assert!(state.contains("Table 0 holds state"), "{state}");
    assert!(rows_at(&memory, &columns, &[]).unwrap().0.is_empty());
}

#[test]
fn a_string_first_named_in_an_earlier_part_is_handed_before_its_row() {
    let (memory, _, model) = log_run(Settings { cap: 48, ..Settings::at(3) }, 2);
    let entry = &memory.pages[1].tables[1];
    let mut prefix = None;
    let mut listed = Vec::new();
    for (at, part) in entry.parts.iter().enumerate() {
        let last = entry.parts.get(at + 1).map_or(memory.pages[1].hours - 1, |following| following.first - 1);
        let part = fetch(&memory.bytes, part, last, &[3], prefix.as_ref()).unwrap();
        listed.push(part.strings().iter().map(|(id, _)| *id).collect::<Vec<_>>());
        prefix = Some(part.prefix());
    }
    let last = entry.parts.len() - 1;
    let row = model.appended.iter().find(|row| row.1 >= entry.parts[last].ordinal && text_of(&row.3[3]).is_some_and(|id| !listed[last].contains(&id) && listed[0].contains(&id))).expect("a row of the last part naming a string the first part listed");
    let id = text_of(&row.3[3]).unwrap();
    let (read, named) = rows_at(&memory, &[3], &[row.1]).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(named.get(&id), Some(&format!("text {}", id - LOG_TEXT).into_bytes()));
}

#[test]
fn a_string_named_again_after_a_log_s_restore_reads_from_that_restore() {
    let reason = LOG_TEXT + 3;
    let mut store = log_store(Settings::at(3));
    let mut memory = Memory::default();
    let mut held = Vec::new();
    store.begin_run(0);
    pin_texts(&mut store);
    for at in 0..2 * BLOCK {
        hour(&mut store, at, &mut held);
        let status = if (BLOCK - 20..BLOCK + 5).contains(&at) { Status::Unavailable { reason } } else { Status::Complete };
        store.status(1, status, true, Some(at as i32), true).unwrap();
        let text = if at < BLOCK { LOG_TEXT + 1 } else { reason };
        store.table(1).unwrap().append(u128::from(at), &[Some(Cell::I64(at.into())), None, None, Some(Cell::Bytes(text)), None, None]).unwrap();
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    let (state, strings) = log_state_named(&memory.bytes, &memory.pages, &LOG_KINDS, 1, 0).unwrap();
    assert_eq!(state.status(), Status::Unavailable { reason });
    assert_eq!(strings.get(&reason), Some(&b"text 3".to_vec()), "the restore lists the refusal's reason");
    let entry = &memory.pages[1].tables[1];
    let mut prefix = None;
    for (at, part) in entry.parts.iter().enumerate() {
        let last = entry.parts.get(at + 1).map_or(memory.pages[1].hours - 1, |following| following.first - 1);
        let part = fetch(&memory.bytes, part, last, &[3], prefix.as_ref()).unwrap();
        assert!(part.strings().iter().all(|(id, _)| *id != reason), "block 1's parts name no string its restore lists");
        prefix = Some(part.prefix());
    }
    let ordinal = u64::from(BLOCK) + 30;
    let (read, named) = rows_at(&memory, &[0, 3], &[5, ordinal]).unwrap();
    assert_eq!(read.iter().map(|row| text_of(&row.1.2[1])).collect::<Vec<_>>(), [Some(LOG_TEXT + 1), Some(reason)]);
    assert_eq!(named.get(&reason), Some(&b"text 3".to_vec()), "by ordinal");
    let mut spanned = HashMap::new();
    log_span(&memory.bytes, &memory.pages, &LOG_KINDS, 1, &[3], 0, BLOCK as i32 + 2..=BLOCK as i32 + 40, &mut |_, event| {
        if let LogEvent::Named { id, text } = event {
            assert!(spanned.insert(id, text.to_vec()).is_none(), "string {id} is handed once a call");
        }
    })
    .unwrap();
    assert_eq!(spanned, HashMap::from([(reason, b"text 3".to_vec())]), "by span");
}

fn marks_by_sorting(pending: &Pending, births: &[(u32, u16)], first: u16) -> Vec<u8> {
    let mut found = Vec::new();
    let (bytes, mut at, mut hour, mut slot) = (&pending.buffer, 0, 0_u16, 0_u32);
    while at < bytes.len() {
        let gap = get_varint(bytes, &mut at).unwrap();
        if gap == 0 {
            hour = get_varint(bytes, &mut at).unwrap() as u16;
            slot = 0;
            continue;
        }
        slot += (gap - 1) as u32;
        found.push(Found { slot, hour, at: at as u32, end: at as u32 });
    }
    found.sort_by_key(|event| event.slot);
    let (mut directory, mut events, mut tracks, mut previous_slot) = (Vec::new(), Vec::new(), 0_u64, 0_u32);
    let mut at = 0;
    while at < found.len() {
        let slot = found[at].slot;
        let end = at + found[at..].iter().take_while(|event| event.slot == slot).count();
        tracks_of(&found, at, end, births, |start, stop, born| {
            let start_bytes = events.len();
            let mut previous = born.unwrap_or(first);
            for event in &found[start..stop] {
                put_varint(&mut events, u64::from(event.hour - previous));
                previous = event.hour;
            }
            put_track(&mut directory, slot - previous_slot, (stop - start) as u64, born, first, events.len() - start_bytes);
            previous_slot = slot;
            tracks += 1;
            Ok(())
        })
        .unwrap();
        at = end;
    }
    let mut out = vec![DIFFERENCES];
    for count in [tracks, directory.len() as u64, 0, 0] {
        put_varint(&mut out, count);
    }
    out.extend_from_slice(&directory);
    out.extend_from_slice(&events);
    out
}

#[test]
fn marks_write_what_sorting_every_mark_by_slot_wrote() {
    let cases: [(Vec<(u16, u32)>, Vec<(u32, u16)>, u16); 5] = [
        ((1..40).flat_map(|hour| (0..50).map(move |slot| (hour, slot))).collect(), Vec::new(), 1),
        ((3..30).flat_map(|hour| [9, 4, 4, 7, 0, 7].map(|slot| (hour, slot))).collect(), Vec::new(), 2),
        (
            (1..=30_u16).flat_map(|hour| [(hour, 5), (hour, 6)].into_iter().chain([1, 8].contains(&hour).then_some((hour, 7)))).collect(),
            vec![(5, 10), (5, 20), (6, 1), (7, 3), (7, 4), (9, 6)],
            1,
        ),
        (vec![(4, 100_000), (4, 3), (5, 100_000), (9, 3)], vec![(100_000, 5)], 0),
        (vec![(719, 12)], Vec::new(), 700),
    ];
    let mut scratch = Scratch::default();
    for (index, (marks_held, births, first)) in cases.into_iter().enumerate() {
        let mut pending = Pending::new();
        for (hour, slot) in marks_held {
            pending.head(hour, slot);
        }
        let written = marks(&pending, &births, first, &mut scratch).unwrap();
        assert_eq!(written, marks_by_sorting(&pending, &births, first), "case {index}");
    }
}

fn journal_kinds() -> Vec<Vec<Kind>> {
    vec![vec![Kind::U32], vec![Kind::I64, Kind::U32List, Kind::OptionalI32]]
}

fn journal_section(table: usize, changed: &[u32], deleted: &[u32], inserted: &[u32], keys: &[u128], inserts: &[crate::state::Values<'_>], runs: &[crate::section::Run<'_>]) -> (usize, Vec<u8>) {
    let kinds = &journal_kinds()[table];
    let hour = crate::section::TableHour { read: None, changed, deleted, inserted, keys, inserts, runs, unrecorded: &[], touched: &[] };
    (table, crate::section::write(kinds, &hour).unwrap())
}

fn state_hour(hour: u32, value: u32) -> (usize, Vec<u8>) {
    use crate::{section::{Op, Run}, state::Values};
    if hour == 0 {
        return journal_section(0, &[], &[], &[0], &[1], &[Values::U32(&[value])], &[]);
    }
    let values = [value];
    journal_section(0, &[0], &[], &[], &[], &[], &[Run { column: 0, op: Op::Set, positions: &[0], values: Some(Values::U32(&values)), except: &[], patches: None }])
}

fn through_journal(hours: &[Vec<(usize, Vec<u8>)>]) -> Memory {
    let kinds = journal_kinds();
    let mut store = Store::new(kinds.clone(), vec![false, false]);
    store.declare_logs(&[false, true]).unwrap();
    store.attach(Encoder::new(&kinds, Settings::at(3)).unwrap());
    let mut adapter = crate::store::journal::Journal::default();
    let mut memory = Memory::default();
    store.begin_run(0);
    for (hour, sections) in hours.iter().enumerate() {
        let manifest = crate::manifest::Manifest {
            branch: 0,
            hour: hour as i32,
            flags: 0,
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: Vec::new(),
            statuses: vec![Status::Complete; 2],
            read: vec![0b11],
            samples: Vec::new(),
            sections: sections.iter().map(|(table, bytes)| crate::manifest::Section { table: *table, bytes: bytes.len() as u64, columns: vec![(1 << kinds[*table].len()) - 1], ops: 0 }).collect(),
        };
        adapter.begin_hour(&mut store, &manifest).unwrap();
        for (entry, (_, bytes)) in manifest.sections.iter().zip(sections) {
            adapter.section(&mut store, entry, bytes).unwrap();
        }
        adapter.end_hour(&mut store, &mut memory).unwrap();
    }
    store.end_run(&mut memory).unwrap();
    memory
}

fn journal_log(memory: &Memory) -> (Vec<(i32, u64, u128)>, Vec<(i32, u64)>) {
    let (mut appended, mut forgotten) = (Vec::new(), Vec::new());
    log_span(&memory.bytes, &memory.pages, &journal_kinds()[1], 1, &[0, 1], 0, 0..=i32::MAX, &mut |hour, event| match event {
        LogEvent::Appended { ordinal, key, .. } => appended.push((hour, ordinal, key)),
        LogEvent::Forgotten { from, to, .. } => forgotten.extend((from..to).map(|ordinal| (hour, ordinal))),
        LogEvent::Named { .. } => {}
    })
    .unwrap();
    (appended, forgotten)
}

fn journal_statuses(memory: &Memory, table: usize) -> Vec<(String, bool)> {
    let (_, strings) = log_state_named(&memory.bytes, &memory.pages, &journal_kinds()[1], 1, 0).unwrap();
    let page = &memory.pages[0];
    (0..page.hours)
        .map(|hour| {
            let (status, read, ..) = reader::status(&page.tables[table], hour, 0).unwrap();
            let shown = match status {
                Status::Unavailable { reason } => String::from_utf8(strings[&reason].clone()).unwrap(),
                other => format!("{other:?}"),
            };
            (shown, read)
        })
        .collect()
}

#[test]
fn a_journal_insert_is_an_append_and_its_delete_a_forget() {
    use crate::state::Values;
    let hours = vec![
        vec![state_hour(0, 5), journal_section(1, &[], &[], &[0, 1], &[100, 101], &[Values::I64(&[-1, -2]), Values::U32List { ends: &[1, 3], items: &[7, 8, 9] }, Values::OptionalI32 { present: &[0b01], values: &[3] }], &[])],
        vec![state_hour(1, 6), journal_section(1, &[], &[0], &[0], &[102], &[Values::I64(&[-3]), Values::U32List { ends: &[0], items: &[] }, Values::OptionalI32 { present: &[0], values: &[] }], &[])],
        vec![state_hour(2, 7), journal_section(1, &[], &[1], &[], &[], &[], &[])],
    ];
    let memory = through_journal(&hours);
    assert_eq!(journal_log(&memory), (vec![(0, 0, 100), (0, 1, 101), (1, 2, 102)], vec![(1, 0), (2, 1)]));
    let columns = [0, 1];
    let mut cells = Vec::new();
    log_rows_at(&memory.bytes, &memory.pages, &journal_kinds()[1], 1, &columns, &[1, 2], &mut |event| {
        if let LogEvent::Appended { ordinal, rows, at, .. } = event {
            cells.push((ordinal, format!("{:?}", rows.value(0, at)), format!("{:?}", rows.value(1, at))));
        }
    })
    .unwrap();
    let list = |items: &[u32]| format!("{:?}", Value::List(&items.iter().flat_map(|item| item.to_le_bytes()).collect::<Vec<u8>>()));
    assert_eq!(cells, [(1, format!("{:?}", Value::Number(-2_i64 as u64)), list(&[8, 9])), (2, format!("{:?}", Value::Number(-3_i64 as u64)), list(&[]))]);
    let log = log_state(&memory.bytes, &memory.pages, &journal_kinds()[1], 1, 0).unwrap();
    assert_eq!((log.count(), log.listed().unwrap().ones().collect::<Vec<_>>()), (Some(3), vec![2]));
    assert_eq!(journal_statuses(&memory, 1), vec![("Complete".to_owned(), true); 3], "a log is read in each hour the collector read it");
}

#[test]
fn a_log_row_written_after_its_hour_costs_its_table_from_that_hour_by_name() {
    use crate::{section::{Op, Run}, state::Values};
    let hours = vec![
        vec![state_hour(0, 5), journal_section(1, &[], &[], &[0, 1], &[100, 101], &[Values::I64(&[-1, -2]), Values::U32List { ends: &[1, 3], items: &[7, 8, 9] }, Values::OptionalI32 { present: &[0b01], values: &[3] }], &[])],
        vec![state_hour(1, 6)],
        vec![
            state_hour(2, 7),
            journal_section(1, &[1], &[0], &[2], &[102], &[Values::I64(&[-3]), Values::U32List { ends: &[0], items: &[] }, Values::OptionalI32 { present: &[0], values: &[] }], &[Run { column: 0, op: Op::Set, positions: &[0], values: Some(Values::I64(&[-20])), except: &[], patches: None }]),
        ],
        vec![state_hour(3, 8), journal_section(1, &[], &[], &[3], &[103], &[Values::I64(&[-4]), Values::U32List { ends: &[0], items: &[] }, Values::OptionalI32 { present: &[0], values: &[] }], &[])],
    ];
    let memory = through_journal(&hours);
    assert_eq!(journal_log(&memory), (vec![(0, 0, 100), (0, 1, 101)], vec![]), "nothing of the refused hour or after it lands");
    let refused = "a log row changed after it was written: table 1 key 0x65 column 0".to_owned();
    assert_eq!(journal_statuses(&memory, 1), [("Complete".to_owned(), true), ("Complete".to_owned(), true), (refused.clone(), true), (refused, true)]);
    assert_eq!(journal_statuses(&memory, 0).into_iter().map(|(status, _)| status).collect::<Vec<_>>(), ["Complete"; 4]);
    let (state, _) = rebuild(&memory.bytes, &memory.pages, &journal_kinds()[0], 0, 0, 0).unwrap();
    assert_eq!((state.key(0), format!("{:?}", state.value(0, 0))), (Some(1), format!("{:?}", Value::Number(8))), "the table holding state took every hour");
    let log = log_state(&memory.bytes, &memory.pages, &journal_kinds()[1], 1, 0).unwrap();
    assert_eq!((log.count(), log.listed().unwrap().ones().collect::<Vec<_>>()), (Some(2), vec![0, 1]));
}

#[test]
fn a_log_key_given_again_costs_its_table_from_that_hour_by_name() {
    use crate::state::Values;
    let row = |key: u128| journal_section(1, &[], &[], &[0], &[key], &[Values::I64(&[1]), Values::U32List { ends: &[0], items: &[] }, Values::OptionalI32 { present: &[0], values: &[] }], &[]);
    let hours = vec![
        vec![state_hour(0, 5), row(100)],
        vec![state_hour(1, 6), journal_section(1, &[], &[0], &[], &[], &[], &[])],
        vec![state_hour(2, 7), row(100)],
    ];
    let memory = through_journal(&hours);
    assert_eq!(journal_log(&memory), (vec![(0, 0, 100)], vec![(1, 0)]));
    let statuses: Vec<String> = journal_statuses(&memory, 1).into_iter().map(|(status, _)| status).collect();
    assert_eq!(statuses, ["Complete", "Complete", "a log row changed after it was written: table 1 key 0x64 given again"]);
}

#[test]
fn unit_numbers_hold_their_widest_values_and_refuse_bits_past_them() {
    let (mut narrow, mut wide) = (Vec::new(), Vec::new());
    put_varint(&mut narrow, u64::MAX);
    put_wide(&mut wide, u128::MAX);
    assert_eq!(get_varint(&narrow, &mut 0).unwrap(), u64::MAX);
    assert_eq!(get_wide(&wide, &mut 0).unwrap(), u128::MAX);
    let over = |groups: usize, last: u8| [vec![0x80; groups], vec![last]].concat();
    assert_eq!(get_varint(&over(9, 1), &mut 0).unwrap(), 1 << 63);
    assert_eq!(get_wide(&over(18, 3), &mut 0).unwrap(), 3 << 126);
    assert_eq!(get_code(&over(9, 3), &mut 0).unwrap(), 3 << 63);
    for refused in [get_code(&over(9, 4), &mut 0), get_code(&over(10, 0), &mut 0)] {
        assert!(refused.unwrap_err().to_string().contains("passes 65 bits"));
    }
    for refused in [get_varint(&over(9, 2), &mut 0).map(u128::from), get_wide(&over(18, 4), &mut 0)] {
        assert!(refused.unwrap_err().to_string().contains("passes"));
    }
}
