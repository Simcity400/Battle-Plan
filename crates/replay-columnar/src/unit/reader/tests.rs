use super::*;
use crate::unit::{FrameEntry, PartEntry, column_stream, put_varint, put_wide, zig_wide};

fn part_of(tracks: u64, directory: &[u8], events: &[u8]) -> (Vec<u8>, PartEntry) {
    part_in(column_stream(0), tracks, directory, events)
}

fn part_in(stream: u32, tracks: u64, directory: &[u8], events: &[u8]) -> (Vec<u8>, PartEntry) {
    let mut raw = vec![0_u8];
    put_varint(&mut raw, tracks);
    put_varint(&mut raw, directory.len() as u64);
    put_varint(&mut raw, 0);
    put_varint(&mut raw, 0);
    raw.extend_from_slice(directory);
    raw.extend_from_slice(events);
    let stored = zstd::bulk::compress(&raw, 1).unwrap();
    let digest = blake3::hash(&stored).as_bytes()[..16].try_into().unwrap();
    let frame = FrameEntry { streams: vec![stream], offset: 0, stored: stored.len() as u32, raw: raw.len() as u32, digest, chained: false, small: false };
    (stored, PartEntry { first: 0, frames: vec![frame], ..PartEntry::default() })
}

fn cells() -> TableState {
    let mut cells = TableState::new(&[Kind::I32]);
    cells.insert_row(0, 10).unwrap();
    cells.insert_row(1, 11).unwrap();
    cells
}

fn advanced(bytes: &[u8], entry: &PartEntry) -> io::Result<Advanced> {
    let part = fetch(&bytes.to_vec(), entry, BLOCK as u16 - 1, &[0], None)?;
    let mut cursor = Cursor::new(&[Kind::I32], &[0], 0);
    cursor.push(part);
    let mut cells = cells();
    cursor.advance(23, 24, &mut cells, &mut [Vec::new()], &mut Lives::default())
}

#[test]
fn a_whole_stream_reads() {
    let (bytes, entry) = part_of(2, &[0, 2, 2, 1, 2, 2], &[0, 2, 1, 2]);
    assert_eq!(advanced(&bytes, &entry).unwrap(), Advanced::Reached);
}

#[test]
fn a_directory_that_disagrees_is_refused_by_name() {
    let (bytes, entry) = part_of(1, &[0, 2, 2, 0], &[0, 2]);
    let refused = fetch(&bytes, &entry, BLOCK as u16 - 1, &[0], None).unwrap_err();
    assert!(refused.to_string().contains("directory disagrees"), "{refused}");
}

#[test]
fn a_track_running_past_its_stream_is_refused_by_name() {
    let (bytes, entry) = part_of(1, &[0, 2, 9], &[0, 2]);
    let refused = fetch(&bytes, &entry, BLOCK as u16 - 1, &[0], None).unwrap_err();
    assert!(refused.to_string().contains("passes its stream"), "{refused}");
}

#[test]
fn an_event_running_into_the_next_track_is_refused_by_name() {
    let (bytes, entry) = part_of(2, &[0, 2, 2, 1, 2, 2], &[0, 0x82, 1, 2]);
    let refused = advanced(&bytes, &entry).unwrap_err();
    assert!(refused.to_string().contains("damaged"), "{refused}");
    let part = fetch(&bytes, &entry, BLOCK as u16 - 1, &[0], None).unwrap();
    let walked = part.tracks(0, false, |_, _, _| {}).unwrap_err();
    assert!(walked.to_string().contains("ends inside a number"), "the event walk reads no byte of the next track: {walked}");
    let (bytes, entry) = part_in(crate::unit::READ, 2, &[0, 2, 2, 1, 2, 2], &[0, 0x82, 1, 2]);
    let part = fetch(&bytes, &entry, BLOCK as u16 - 1, &[], None).unwrap();
    let marked = part.reads(0, BLOCK as u16 - 1, &mut Vec::new()).unwrap_err();
    assert!(marked.to_string().contains("ends inside a number"), "a read mark's gap reads no byte of the next track: {marked}");
}

#[test]
fn a_varint_running_past_the_buffer_is_refused_by_name() {
    let (bytes, entry) = part_of(1, &[0, 2, 3], &[0, 0x80, 0x80]);
    let refused = advanced(&bytes, &entry).unwrap_err();
    assert!(refused.to_string().contains("damaged"), "{refused}");
}

#[test]
fn events_past_their_part_are_refused_by_name() {
    let (bytes, entry) = part_of(1, &[0, 2, 3], &[0x80, 0x05, 2]);
    let part = fetch(&bytes, &entry, 10, &[0], None).unwrap();
    let mut cursor = Cursor::new(&[Kind::I32], &[0], 0);
    cursor.push(part);
    let mut cells = cells();
    let refused = cursor.advance(20, 24, &mut cells, &mut [Vec::new()], &mut Lives::default()).unwrap_err();
    assert!(refused.to_string().contains("past its part"), "{refused}");
}

#[test]
fn a_mark_past_its_part_is_refused_by_name() {
    for stream in [crate::unit::READ, crate::unit::TOUCHED] {
        let (bytes, entry) = part_in(stream, 1, &[0, 2, 2], &[4, 3]);
        let part = fetch(&bytes, &entry, 5, &[], None).unwrap();
        let (mut touched, mut read) = (Vec::new(), Vec::new());
        let refused = part.marks(0, 10, &mut touched, &mut read).unwrap_err();
        assert!(refused.to_string().contains("past its part"), "the mark at hour 7 lies past hour 5, the part's last: {refused}");
        let (bytes, entry) = part_in(stream, 1, &[0, 2, 2], &[4, 1]);
        let part = fetch(&bytes, &entry, 5, &[], None).unwrap();
        let (mut touched, mut read) = (Vec::new(), Vec::new());
        part.marks(0, 10, &mut touched, &mut read).unwrap();
        assert_eq!([touched, read].concat(), [(4, 0), (5, 0)], "a mark at the part's last hour reads");
    }
}

fn written(script: impl Fn(u32, &mut crate::store::TableChanges<'_>)) -> (Vec<u8>, crate::unit::BlockPage, Vec<TableState>) {
    use crate::{
        store::Store,
        unit::{BlockPage, Encoder, PartSink, Settings},
    };
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
    let kinds = vec![Kind::I32, Kind::U32List];
    let mut store = Store::new(vec![kinds.clone()], vec![false]);
    store.attach(Encoder::new(&[kinds], Settings::at(3)).unwrap());
    let mut memory = Memory::default();
    let mut states = Vec::new();
    store.begin_run(0);
    for hour in 0..BLOCK {
        store.begin_hour(hour as i32, &[]).unwrap();
        store.status(0, Status::Complete, true, Some(hour as i32), true).unwrap();
        script(hour, &mut store.table(0).unwrap());
        store.end_hour(&mut memory).unwrap();
        states.push(store.table_state(0).unwrap().clone());
    }
    store.end_block();
    (memory.bytes, memory.pages.remove(0), states)
}

fn row(cells: &TableState, slot: u32) -> (Option<u128>, Vec<String>) {
    (cells.key(slot), (0..2).map(|column| format!("{:?}", cells.value(column, slot))).collect())
}

fn live_rows(cells: &TableState) -> Vec<(u32, Option<u128>, Vec<String>)> {
    cells.rows().map(|slot| {
        let (key, values) = row(cells, slot);
        (slot, key, values)
    }).collect()
}

fn cursor_over(bytes: &[u8], page: &crate::unit::BlockPage, states: &[TableState]) -> (Cursor, TableState) {
    let part = fetch(&bytes.to_vec(), &page.tables[0].parts[0], page.hours - 1, &[0, 1], None).unwrap();
    let mut cursor = Cursor::new(&[Kind::I32, Kind::U32List], &[0, 1], 1);
    cursor.push(part);
    (cursor, states[0].clone())
}

#[test]
fn a_deleted_row_is_seen_at_its_last_live_hour() {
    use crate::state::Cell;
    let list = |items: &[u32]| items.to_vec();
    let script = |hour: u32, changes: &mut crate::store::TableChanges<'_>| {
        let lists = [list(&[1, 2, 3]), list(&[1, 9, 3]), list(&[4, 4]), list(&[7, 7, 7, 7, 7])];
        match hour {
            0 => {
                changes.insert_at(0, 1).unwrap();
                changes.write(0, 0, Cell::I32(5)).unwrap();
                changes.write(0, 1, Cell::U32List(&lists[0])).unwrap();
                changes.insert_at(1, 4).unwrap();
                changes.write(1, 0, Cell::I32(100)).unwrap();
                changes.write(1, 1, Cell::U32List(&lists[2])).unwrap();
            }
            26 => {
                changes.insert_at(2, 5).unwrap();
                changes.write(2, 0, Cell::I32(-3)).unwrap();
                changes.write(2, 1, Cell::U32List(&lists[3])).unwrap();
            }
            27 => {
                changes.write(2, 0, Cell::I32(-30)).unwrap();
                changes.write(2, 1, Cell::U32List(&lists[1])).unwrap();
            }
            29 => changes.write(2, 0, Cell::I32(-31)).unwrap(),
            30 => changes.delete(0).unwrap(),
            31 => changes.delete(2).unwrap(),
            32 => {
                changes.insert_at(0, 2).unwrap();
                changes.write(0, 0, Cell::I32(70)).unwrap();
                changes.write(0, 1, Cell::U32List(&lists[0])).unwrap();
            }
            33 => changes.write(0, 1, Cell::U32List(&lists[1])).unwrap(),
            35 => changes.write(0, 0, Cell::I32(71)).unwrap(),
            40 => changes.delete(0).unwrap(),
            44 => {
                changes.insert_at(0, 3).unwrap();
                changes.write(0, 0, Cell::I32(9)).unwrap();
            }
            45 => changes.write(0, 1, Cell::U32List(&lists[2])).unwrap(),
            _ => {}
        }
        if hour % 3 == 0 && hour > 0 && changes.held().live().get(1) {
            changes.write(1, 0, Cell::I32(100 + hour as i32)).unwrap();
        }
        if hour > 0 && hour < 30 && hour % 4 == 1 {
            changes.write(0, 0, Cell::I32(5 + hour as i32)).unwrap();
        }
    };
    let (bytes, page, states) = written(script);
    let (mut hourly, mut cells) = cursor_over(&bytes, &page, &states);
    let mut expected = Vec::new();
    let mut before = cells.clone();
    for hour in 1..48_u16 {
        let mut lives = Lives::default();
        hourly.advance(hour, 1, &mut cells, &mut [Vec::new(), Vec::new()], &mut lives).unwrap();
        for &(at, slot, key) in &lives.deleted {
            expected.push((at, slot, key, row(&before, slot)));
        }
        before = cells.clone();
    }
    let (mut stepped, mut seen_cells) = cursor_over(&bytes, &page, &states);
    let (mut plain, mut plain_cells) = cursor_over(&bytes, &page, &states);
    let mut seen = Vec::new();
    for (to, step) in [(23_u16, 23_u16), (47, 24)] {
        let (mut written, mut lives) = (vec![Vec::new(), Vec::new()], Lives::default());
        stepped.advance_seeing(to, step, &mut seen_cells, &mut written, &mut lives, &mut |hour, slot, key, cells| seen.push((hour, slot, key, row(cells, slot)))).unwrap();
        let (mut plain_written, mut plain_lives) = (vec![Vec::new(), Vec::new()], Lives::default());
        plain.advance(to, step, &mut plain_cells, &mut plain_written, &mut plain_lives).unwrap();
        assert_eq!(live_rows(&seen_cells), live_rows(&plain_cells), "the cells after the step to {to}");
        assert_eq!(written, plain_written, "the slots written in the step to {to}");
        assert_eq!(lives, plain_lives, "the lives of the step to {to}");
        assert_eq!(live_rows(&seen_cells), live_rows(&states[usize::from(to)]), "the store's table after hour {to}");
    }
    assert_eq!(expected.len(), 3, "three rows are deleted: {expected:?}");
    assert_eq!(seen, expected, "each deleted row as it stood at its last live hour");
}

#[test]
fn a_run_of_deleted_rows_is_cleared_as_one() {
    use crate::state::Cell;
    let script = |hour: u32, changes: &mut crate::store::TableChanges<'_>| {
        let list = [3_u32, 4];
        match hour {
            0 => {
                for slot in 0..200 {
                    changes.insert_at(slot, 1000 + u128::from(slot)).unwrap();
                    changes.write(slot, 0, Cell::I32(slot as i32)).unwrap();
                    changes.write(slot, 1, Cell::U32List(&list[..(slot % 3) as usize])).unwrap();
                }
            }
            30 => {
                for slot in (3..5).chain(10..150).chain([160, 190]) {
                    changes.delete(slot).unwrap();
                }
            }
            31 => {
                changes.insert_at(40, 77).unwrap();
                changes.write(40, 0, Cell::I32(-7)).unwrap();
            }
            _ if hour % 5 == 0 => {
                for slot in [0, 150, 199] {
                    changes.write(slot, 0, Cell::I32(hour as i32)).unwrap();
                }
            }
            _ => {}
        }
    };
    let (bytes, page, states) = written(script);
    let (mut hourly, mut hourly_cells) = cursor_over(&bytes, &page, &states);
    let (mut stepped, mut cells) = cursor_over(&bytes, &page, &states);
    let mut hourly_lives = Lives::default();
    for hour in 1..48_u16 {
        hourly.advance(hour, 1, &mut hourly_cells, &mut [Vec::new(), Vec::new()], &mut hourly_lives).unwrap();
    }
    let mut lives = Lives::default();
    for (to, step) in [(23_u16, 23_u16), (47, 24)] {
        stepped.advance(to, step, &mut cells, &mut [Vec::new(), Vec::new()], &mut lives).unwrap();
        assert_eq!(live_rows(&cells), live_rows(&states[usize::from(to)]), "the store's table after hour {to}");
    }
    assert_eq!(lives, hourly_lives, "the lives, in slot order within each hour");
    assert_eq!(live_rows(&cells), live_rows(&hourly_cells));
    assert_eq!(lives.deleted.len(), 144);
    assert_eq!(cells.value(0, 100), Value::Number(0), "a cleared slot holds nothing");
}

#[test]
fn stepping_past_values_and_summing_keys_equal_reading_them_one_at_a_time() {
    let mut differences: Vec<i128> = (0..40).map(|index| index % 3 - 1).collect();
    differences.extend((0..40).map(|index| (200 + index * 37) * if index % 2 == 0 { 1 } else { -1 }));
    differences.extend((0..30).map(|index| match index % 5 {
        0 => 5,
        1 => -3000,
        2 => 1 << 40,
        3 => -(1 << 100),
        _ => 70,
    }));
    let (mut raw, mut ends) = (Vec::new(), vec![0]);
    for &difference in &differences {
        put_wide(&mut raw, zig_wide(difference));
        ends.push(raw.len());
    }
    for start in 0..differences.len() {
        for count in 0..=differences.len() - start {
            let mut at = ends[start];
            let sum = logs::sum_keys(&raw, &mut at, count as u64).unwrap();
            assert_eq!(at, ends[start + count], "from {start}, {count} keys");
            assert_eq!(sum, differences[start..start + count].iter().fold(0_i128, |sum, difference| sum.wrapping_add(*difference)), "from {start}, {count} keys");
            assert_eq!(numbers::skip(&raw, ends[start], raw.len(), count), Some(ends[start + count]), "from {start}, {count} values");
        }
    }
    assert_eq!(numbers::skip(&raw, 0, ends[3], 4), None, "four values past an end after three");
}

#[test]
fn a_number_wider_than_its_track_marks_the_track_damaged() {
    let over = |groups: usize, last: u8| [vec![0x80; groups], vec![last]].concat();
    for (bytes, value, damaged, code_damaged) in [
        (over(9, 1), 1 << 63, false, false),
        (over(9, 2), 0, true, false),
        (over(9, 4), 0, true, true),
        (over(10, 0), 0, true, true),
        (over(18, 4), 0, true, true),
    ] {
        let (mut at, mut seen) = (0, false);
        assert_eq!(numbers::varint(&bytes, &mut at, &mut seen), value);
        assert_eq!((at, seen), (bytes.len(), damaged), "{bytes:02x?}");
        let (mut at, mut seen) = (0, false);
        numbers::code(&bytes, &mut at, &mut seen);
        assert_eq!((at, seen), (bytes.len(), code_damaged), "{bytes:02x?} as a code");
    }
}

#[test]
fn an_event_code_past_its_track_width_is_refused_by_the_event_walk_as_by_the_reader() {
    let mut events = vec![0];
    events.extend([0x80; 10]);
    events.push(0x00);
    let (bytes, entry) = part_of(1, &[0, 2, events.len() as u8], &events);
    let part = fetch(&bytes, &entry, BLOCK as u16 - 1, &[0], None).unwrap();
    let refused = part.tracks(0, false, |_, _, _| {}).unwrap_err();
    assert!(refused.to_string().contains("passes 65 bits"), "{refused}");
    let damaged = advanced(&bytes, &entry).unwrap_err();
    assert!(damaged.to_string().contains("damaged"), "{damaged}");
}

#[test]
fn a_reserved_event_code_is_refused_by_the_event_walk_as_by_the_reader() {
    let (bytes, entry) = part_of(1, &[0, 2, 2], &[0, 7]);
    let part = fetch(&bytes, &entry, BLOCK as u16 - 1, &[0], None).unwrap();
    let refused = part.tracks(0, false, |_, _, _| {}).unwrap_err();
    assert!(refused.to_string().contains("reserved"), "{refused}");
    let damaged = advanced(&bytes, &entry).unwrap_err();
    assert!(damaged.to_string().contains("damaged"), "{damaged}");
}

#[test]
fn a_restore_with_duplicate_live_keys_is_refused_by_name() {
    use crate::unit::restore::{Chunk, META, ROWS, RestoreEntry};
    let push_frame = |bytes: &mut Vec<u8>, stream: u32, raw: &[u8]| -> FrameEntry {
        let stored = zstd::bulk::compress(raw, 1).unwrap();
        let digest = blake3::hash(&stored).as_bytes()[..16].try_into().unwrap();
        let offset = bytes.len() as u64;
        bytes.extend_from_slice(&stored);
        FrameEntry { streams: vec![stream], offset, stored: stored.len() as u32, raw: raw.len() as u32, digest, chained: false, small: false }
    };
    let encode_meta = |slots: u32| -> Vec<u8> {
        let mut raw = Vec::new();
        put_varint(&mut raw, u64::from(slots));
        raw.push(0);
        raw.push(0);
        raw.push(0);
        put_varint(&mut raw, 0);
        raw.push(0);
        raw
    };
    let encode_rows = |bits: &[u8], keys: &[u128]| -> Vec<u8> {
        let mut raw = bits.to_vec();
        let mut prev = 0_u128;
        for &key in keys {
            put_wide(&mut raw, zig_wide(key.wrapping_sub(prev) as i128));
            prev = key;
        }
        raw
    };

    let mut single_bytes = Vec::new();
    let meta = push_frame(&mut single_bytes, META, &encode_meta(3));
    let rows = push_frame(&mut single_bytes, ROWS, &encode_rows(&[0b101], &[0xdead, 0xdead]));
    let entry = RestoreEntry {
        whole: true,
        streams: vec![
            (META, vec![Chunk { first: 0, frame: meta, prefix: None }]),
            (ROWS, vec![Chunk { first: 0, frame: rows, prefix: None }]),
        ],
        log: false,
    };
    let refused = restore(&single_bytes, &entry, &[Kind::I32], &[]).unwrap_err().to_string();
    assert!(refused.contains("0xdead") && refused.contains("already held"), "{refused}");

    let mut multi_bytes = Vec::new();
    let meta = push_frame(&mut multi_bytes, META, &encode_meta(16));
    let first = push_frame(&mut multi_bytes, ROWS, &encode_rows(&[0b010], &[0xbeef]));
    let second = push_frame(&mut multi_bytes, ROWS, &encode_rows(&[0b010], &[0xbeef]));
    let multi = RestoreEntry {
        whole: true,
        streams: vec![
            (META, vec![Chunk { first: 0, frame: meta, prefix: None }]),
            (
                ROWS,
                vec![
                    Chunk { first: 0, frame: first, prefix: None },
                    Chunk { first: 8, frame: second, prefix: None },
                ],
            ),
        ],
        log: false,
    };
    let refused = restore(&multi_bytes, &multi, &[Kind::I32], &[]).unwrap_err().to_string();
    assert!(refused.contains("0xbeef") && refused.contains("already held"), "{refused}");
    let refused_keyed = restore_keyed(&multi_bytes, &multi, &[Kind::I32], &[]).unwrap_err().to_string();
    assert!(refused_keyed.contains("0xbeef") && refused_keyed.contains("already held"), "{refused_keyed}");

    let mut clean_bytes = Vec::new();
    let meta = push_frame(&mut clean_bytes, META, &encode_meta(3));
    let rows = push_frame(&mut clean_bytes, ROWS, &encode_rows(&[0b101], &[0x11, 0x22]));
    let clean = RestoreEntry {
        whole: true,
        streams: vec![
            (META, vec![Chunk { first: 0, frame: meta, prefix: None }]),
            (ROWS, vec![Chunk { first: 0, frame: rows, prefix: None }]),
        ],
        log: false,
    };
    let (unkeyed, _) = restore(&clean_bytes, &clean, &[Kind::I32], &[]).unwrap();
    assert_eq!(unkeyed.bytes().index, 0);
    let (keyed, _) = restore_keyed(&clean_bytes, &clean, &[Kind::I32], &[]).unwrap();
    assert!(keyed.bytes().index > 0);
    assert_eq!((keyed.find(0x11), keyed.find(0x22), keyed.find(0x33)), (Some(0), Some(2), None));
}
