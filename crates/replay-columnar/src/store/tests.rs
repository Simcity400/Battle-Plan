use super::{journal::Journal, *};
use crate::{
    section::{self as journal_section, Op, Run, TableHour},
    state::{Table, Values},
};
use std::collections::BTreeMap;

const KINDS: [Kind; 6] = [Kind::U8, Kind::I32, Kind::OptionalI64, Kind::Bool, Kind::Bytes, Kind::U32List];

fn store() -> Store {
    let mut store = Store::new(vec![KINDS.to_vec()], vec![false]);
    store.begin_hour(0, &[b"zero", b"one", b"two"]).unwrap();
    store
}

type Model = BTreeMap<u32, (u128, Vec<Option<Owned>>)>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Owned {
    Missing,
    Number(u64),
    Text(u32),
    List(Vec<u8>),
}

fn owned(value: Value<'_>) -> Option<Owned> {
    match value {
        Value::Unrecorded => None,
        Value::Missing => Some(Owned::Missing),
        Value::Number(bits) => Some(Owned::Number(bits)),
        Value::Text(id) => Some(Owned::Text(id)),
        Value::List(bytes) => Some(Owned::List(bytes.to_vec())),
    }
}

fn check(store: &Store, model: &Model) {
    let table = store.table_state(0).unwrap();
    let live: Vec<u32> = table.rows().collect();
    assert_eq!(live, model.keys().copied().collect::<Vec<_>>());
    for (&slot, (key, cells)) in model {
        assert_eq!(table.key(slot), Some(*key), "slot {slot}");
        for (column, cell) in cells.iter().enumerate() {
            assert_eq!(owned(table.value(column, slot)), *cell, "slot {slot} column {column}");
        }
    }
}

fn cells(seed: u64) -> [Cell<'static>; 6] {
    static LISTS: [[u32; 3]; 4] = [[1, 2, 3], [7, 8, 9], [0, 0, 0], [5, 5, 5]];
    [
        Cell::U8((seed * 37) as u8),
        Cell::I32(-((seed * 1_000_003) as i32)),
        Cell::OptionalI64((seed % 3 != 0).then(|| (seed as i64) << 40)),
        Cell::Bool(seed % 2 == 0),
        Cell::Bytes((seed % 3) as u32),
        Cell::U32List(&LISTS[seed as usize % 4][..1 + seed as usize % 3]),
    ]
}

fn model_cells(seed: u64) -> Vec<Option<Owned>> {
    let mut scratch = Vec::new();
    cells(seed).iter().zip(KINDS).map(|(&cell, kind)| owned(value_of(kind, cell, &mut scratch).unwrap())).collect()
}

#[test]
fn each_entry_point_matches_a_plain_map_of_cells() {
    let mut store = store();
    let mut model = Model::new();
    for step in 0..400_u64 {
        let mut changes = store.table(0).unwrap();
        let slot = (step * 7919 % 97) as u32;
        let live = model.contains_key(&slot);
        match step % 9 {
            0 | 1 if !live => {
                let key = u128::from(step) << 70 | u128::from(slot);
                changes.insert_at(slot, key).unwrap();
                for (column, cell) in cells(step).into_iter().enumerate() {
                    changes.write(slot, column, cell).unwrap();
                }
                model.insert(slot, (key, model_cells(step)));
            }
            2 if live => {
                changes.delete(slot).unwrap();
                model.remove(&slot);
            }
            3 if live => {
                let column = (step % 6) as usize;
                changes.write(slot, column, cells(step + 1)[column]).unwrap();
                model.get_mut(&slot).unwrap().1[column] = model_cells(step + 1)[column].clone();
            }
            4 if live => {
                let held = model.get_mut(&slot).unwrap();
                changes.add(slot, 1, (-5_i64) as u64).unwrap();
                let Some(Owned::Number(bits)) = held.1[1].clone() else { unreachable!("an I32 is a number") };
                held.1[1] = Some(Owned::Number(bits.wrapping_add((-5_i64) as u64) as i32 as i64 as u64));
            }
            5 if live => {
                changes.unrecord(slot, 2).unwrap();
                model.get_mut(&slot).unwrap().1[2] = None;
            }
            6 if live => {
                let held = model.get_mut(&slot).unwrap();
                let Some(Owned::List(old)) = held.1[5].clone() else { continue };
                let items: Vec<u8> = [4_u32, 6].iter().flat_map(|item| item.to_le_bytes()).collect();
                let removed = (old.len() / 4 - 1) as u32;
                changes.edit(slot, 5, Edits { hunks: &[(1, removed, 2)], items: &items }).unwrap();
                let mut new = old[..4].to_vec();
                new.extend_from_slice(&items);
                held.1[5] = Some(Owned::List(new));
            }
            7 => {
                let except: Vec<u32> = model.keys().copied().filter(|held| held % 3 == 0).collect();
                let filled = changes.fill(0, Cell::U8(250), &except).unwrap();
                for (held, (_, row)) in model.iter_mut() {
                    if !except.contains(held) {
                        row[0] = Some(Owned::Number(250));
                    }
                }
                assert!(filled.iter().all(|held| !except.contains(held)));
            }
            8 if live => {
                let changed = changes.offer(slot, 3, Cell::Bool(true)).unwrap();
                let held = model.get_mut(&slot).unwrap();
                assert_eq!(changed, held.1[3] != Some(Owned::Number(1)));
                held.1[3] = Some(Owned::Number(1));
            }
            _ => {}
        }
        store.end_hour(&mut crate::unit::Discard).unwrap();
        check(&store, &model);
        store.begin_hour(step as i32 + 1, &[]).unwrap();
    }
}

#[test]
fn narrow_columns_widen_and_narrow() {
    let mut column = Narrow::new(true);
    column.set(3, 0, 4);
    assert_eq!(column.width(), 0);
    column.set(1, (-1_i64) as u64, 4);
    assert_eq!(column.width(), 1);
    column.set(2, 70_000, 4);
    assert_eq!(column.width(), 4);
    column.set(0, i64::MIN as u64, 4);
    assert_eq!(column.width(), 8);
    assert_eq!([column.get(0), column.get(1), column.get(2), column.get(3)], [i64::MIN as u64, (-1_i64) as u64, 70_000, 0]);
    column.set(0, 0, 4);
    column.narrow(4);
    assert_eq!(column.width(), 4);
    assert_eq!(column.get(1), (-1_i64) as u64);
    let mut unsigned = Narrow::new(false);
    unsigned.set(0, u64::MAX, 1);
    assert_eq!((unsigned.width(), unsigned.get(0)), (8, u64::MAX));
}

#[test]
fn the_key_index_follows_inserts_and_deletes() {
    let mut store = store();
    let mut changes = store.table(0).unwrap();
    for slot in 0..5000_u32 {
        changes.insert_at(slot, u128::from(slot) * 0x1_0000_0001 + (1 << 100)).unwrap();
    }
    assert_eq!(changes.slot(4999 * 0x1_0000_0001 + (1 << 100)), Some(4999));
    for slot in (0..5000_u32).filter(|slot| slot % 3 == 0) {
        changes.delete(slot).unwrap();
    }
    for slot in 0..5000_u32 {
        let found = changes.slot(u128::from(slot) * 0x1_0000_0001 + (1 << 100));
        assert_eq!(found, (slot % 3 != 0).then_some(slot), "slot {slot}");
    }
    assert_eq!(changes.insert(7).unwrap(), 0, "the lowest free slot");
    assert_eq!(changes.slot(7), Some(0));
}

#[test]
fn a_packed_table_keeps_its_rows_in_slot_order() {
    let mut store = store();
    let mut changes = store.table(0).unwrap();
    for slot in 0..20_u32 {
        changes.insert_at(slot, u128::from(slot) + 100).unwrap();
        for (column, cell) in cells(u64::from(slot)).into_iter().enumerate() {
            changes.write(slot, column, cell).unwrap();
        }
    }
    changes.unrecord(13, 4).unwrap();
    for slot in (0..20_u32).filter(|slot| slot % 4 != 1) {
        changes.delete(slot).unwrap();
    }
    let before: Vec<(u128, Vec<Option<Owned>>)> = {
        let table = store.table_state(0).unwrap();
        table.rows().map(|slot| (table.key(slot).unwrap(), (0..KINDS.len()).map(|column| owned(table.value(column, slot))).collect())).collect()
    };
    let moved = store.tables[0].pack().unwrap();
    assert_eq!(moved[13], Some(3));
    assert_eq!(moved[12], None);
    let table = store.table_state(0).unwrap();
    assert_eq!(table.slots(), 5);
    let after: Vec<(u128, Vec<Option<Owned>>)> =
        table.rows().map(|slot| (table.key(slot).unwrap(), (0..KINDS.len()).map(|column| owned(table.value(column, slot))).collect())).collect();
    assert_eq!(before, after);
    assert_eq!(table.rows().collect::<Vec<_>>(), (0..5).collect::<Vec<_>>());
}

#[test]
fn a_table_packed_in_place_across_pages_keeps_every_row_and_its_index() {
    let mut store = store();
    let rows = 3 * PAGE as u32 + 17;
    let mut changes = store.table(0).unwrap();
    for slot in 0..rows {
        changes.insert_at(slot, u128::from(slot) * 7 + 1).unwrap();
        for (column, cell) in cells(u64::from(slot)).into_iter().enumerate() {
            changes.write(slot, column, cell).unwrap();
        }
        if slot % 9 == 4 {
            changes.unrecord(slot, 5).unwrap();
        }
    }
    for slot in (0..rows).filter(|slot| slot % 3 != 2) {
        changes.delete(slot).unwrap();
    }
    store.tables[0].slot(0);
    let before: Vec<(u128, Vec<Option<Owned>>)> = {
        let table = store.table_state(0).unwrap();
        table.rows().map(|slot| (table.key(slot).unwrap(), (0..KINDS.len()).map(|column| owned(table.value(column, slot))).collect())).collect()
    };
    let moved = store.tables[0].pack().unwrap();
    let table = store.table_state(0).unwrap();
    assert_eq!(table.slots() as usize, before.len());
    assert_eq!(table.rows().collect::<Vec<_>>(), (0..before.len() as u32).collect::<Vec<_>>());
    let after: Vec<(u128, Vec<Option<Owned>>)> =
        table.rows().map(|slot| (table.key(slot).unwrap(), (0..KINDS.len()).map(|column| owned(table.value(column, slot))).collect())).collect();
    assert_eq!(before, after);
    for (old, new) in moved.iter().enumerate() {
        if let Some(new) = new {
            assert_eq!(store.tables[0].slot(u128::from(old as u32) * 7 + 1), Some(*new), "row {old} found at its new slot");
        }
    }
    assert_eq!(store.tables[0].slot(1), None, "a deleted row's key finds nothing");
}

#[test]
fn a_hinted_lookup_needs_no_key_index_and_a_block_s_end_lets_go_of_one_little_used() {
    let mut store = self::store();
    let mut changes = store.table(0).unwrap();
    for slot in 0..100_u32 {
        changes.insert_at(slot, u128::from(slot) + 1000).unwrap();
    }
    assert_eq!(changes.slot_of(1004, Some(4)), Some(4));
    assert!(store.tables[0].index.is_none(), "a hint that holds its key builds no index");
    let mut changes = store.table(0).unwrap();
    assert_eq!(changes.slot_of(1004, Some(7)), Some(4), "a hint naming another row's slot is found by key");
    assert_eq!(changes.slot_of(1500, Some(500)), None, "a hint past the table's slots");
    assert_eq!(changes.slot_of(1500, None), None);
    assert!(store.tables[0].index.is_some(), "a miss builds the index");
    store.end_hour(&mut crate::unit::Discard).unwrap();
    store.end_block();
    assert!(store.tables[0].index.is_none(), "three lookups in a block of a table of 100 rows");
    store.begin_hour(1, &[]).unwrap();
    let mut changes = store.table(0).unwrap();
    for key in 1000..1100 {
        assert_eq!(changes.slot_of(key, None), Some((key - 1000) as u32));
    }
    store.end_hour(&mut crate::unit::Discard).unwrap();
    store.end_block();
    assert!(store.tables[0].index.is_some(), "as many lookups as rows: kept");
    store.tables[0].pack().unwrap();
    assert!(store.tables[0].index.is_none(), "a pack lets go of it");
    assert_eq!(store.tables[0].slot(1050), Some(50));
}

#[test]
fn a_list_page_across_segments_holds_every_cell_through_rewrites_and_compaction() {
    let mut store = store();
    let mut expected: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut changes = store.table(0).unwrap();
    for slot in 0..PAGE as u32 {
        changes.insert_at(slot, u128::from(slot) + 1).unwrap();
    }
    for round in 0..6_u32 {
        for slot in (0..PAGE as u32).filter(|slot| (slot + round) % 3 != 0) {
            let length = 200 + ((slot * 37 + round * 101) % 1_400) as usize;
            let cell: Vec<u32> = (0..length as u32).map(|item| item ^ slot ^ round).collect();
            changes.write(slot, 5, Cell::U32List(&cell)).unwrap();
            expected.insert(slot, cell);
        }
        if round == 3 {
            for slot in (0..PAGE as u32).step_by(7) {
                changes.write(slot, 5, Cell::U32List(&[])).unwrap();
                expected.insert(slot, Vec::new());
            }
        }
    }
    let table = store.table_state(0).unwrap();
    for (&slot, cell) in &expected {
        let bytes: Vec<u8> = cell.iter().flat_map(|item| item.to_le_bytes()).collect();
        assert_eq!(table.value(5, slot), Value::List(&bytes), "slot {slot}");
    }
}

struct Archive(Vec<Vec<u8>>);

impl StringSource for Archive {
    fn text(&mut self, id: u32) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.get(id as usize).cloned())
    }
}

#[test]
fn strings_let_go_are_read_again_from_their_file() {
    let mut store = store();
    let mut changes = store.table(0).unwrap();
    changes.insert_at(0, 1).unwrap();
    changes.write(0, 4, Cell::Bytes(2)).unwrap();
    store.end_block();
    assert_eq!(store.strings().get(2), Some(&b"two"[..]));
    assert_eq!(store.strings().get(1), None);
    let refused = store.table(0).unwrap().write(0, 4, Cell::Bytes(1)).unwrap_err();
    assert!(refused.to_string().contains("let go of"), "{refused}");
    store.read_strings_from(Box::new(Archive(vec![b"zero".to_vec(), b"one".to_vec(), b"two".to_vec()])));
    store.table(0).unwrap().write(0, 4, Cell::Bytes(1)).unwrap();
    assert_eq!(store.strings().get(1), Some(&b"one"[..]));
}

#[test]
fn the_journal_adapter_matches_the_section_grammar() {
    let kinds = vec![Kind::U16, Kind::OptionalU32, Kind::Opaque];
    let mut table = Table::new(&kinds).unwrap();
    let mut store = Store::new(vec![kinds.clone()], vec![false]);
    let mut adapter = Journal::default();
    let mut changes = crate::section::Changes::default();
    let mut scratch = crate::section::Scratch::default();
    let (big_ends, big_items) = {
        let (mut ends, mut items) = (Vec::new(), Vec::new());
        for row in 0..400_u32 {
            let length = if row == 7 { 300_000 } else { 1_000 + row as usize * 3 };
            items.extend((0..length).map(|at| (at as u32 ^ row) as u8));
            ends.push(items.len() as u32);
        }
        (ends, items)
    };
    let big_slots: Vec<u32> = (3..403).collect();
    let big_keys: Vec<u128> = (3..403_u32).map(|slot| u128::from(slot) + 100).collect();
    let big_u16: Vec<u16> = (0..400).collect();
    let big_present = vec![0_u64; 7];
    let (set_ends, set_items) = {
        let (mut ends, mut items) = (Vec::new(), Vec::new());
        for row in 0..2_u32 {
            items.extend((0..200_000).map(|at| (at as u32 + row) as u8));
            ends.push(items.len() as u32);
        }
        (ends, items)
    };
    let many_slots: Vec<u32> = (403..140_403).collect();
    let many_keys: Vec<u128> = many_slots.iter().map(|&slot| u128::from(slot) * 3 + 1).collect();
    let many_u16: Vec<u16> = many_slots.iter().map(|&slot| (slot % 60_000) as u16).collect();
    let many_present = vec![u64::MAX; many_slots.len().div_ceil(64)];
    let many_present = { let mut words = many_present; if many_slots.len() % 64 != 0 { *words.last_mut().unwrap() = (1 << (many_slots.len() % 64)) - 1; } words };
    let many_values: Vec<u32> = many_slots.iter().map(|&slot| slot ^ 0x55).collect();
    let many_ends: Vec<u32> = (1..=many_slots.len() as u32).map(|at| at * 2).collect();
    let many_items: Vec<u8> = (0..many_slots.len() * 2).map(|at| at as u8).collect();
    let hours: Vec<Vec<u8>> = vec![
        journal_section::write(
            &kinds,
            &TableHour {
                read: None,
                changed: &[],
                deleted: &[],
                inserted: &[0, 1, 2],
                keys: &[10, 11, 12],
                inserts: &[
                    Values::U16(&[1, 2, 3]),
                    Values::OptionalU32 { present: &[0b101], values: &[7, 9] },
                    Values::Opaque { ends: &[2, 2, 5], items: b"abcde" },
                ],
                runs: &[],
                unrecorded: &[],
                touched: &[],
            },
        )
        .unwrap(),
        journal_section::write(
            &kinds,
            &TableHour {
                read: None,
                changed: &[0, 2],
                deleted: &[1],
                inserted: &[],
                keys: &[],
                inserts: &[],
                runs: &[
                    Run { column: 0, op: Op::Add, positions: &[0, 1], values: Some(Values::U16(&[u16::MAX, 5])), except: &[], patches: None },
                    Run { column: 1, op: Op::Fill, positions: &[], values: Some(Values::OptionalU32 { present: &[1], values: &[4] }), except: &[2], patches: None },
                    Run { column: 2, op: Op::Noop, positions: &[1], values: None, except: &[], patches: None },
                ],
                unrecorded: &[(2, &[0])],
                touched: &[2],
            },
        )
        .unwrap(),
        journal_section::write(
            &kinds,
            &TableHour {
                read: None,
                changed: &[0, 2],
                deleted: &[],
                inserted: &big_slots,
                keys: &big_keys,
                inserts: &[
                    Values::U16(&big_u16),
                    Values::OptionalU32 { present: &big_present, values: &[] },
                    Values::Opaque { ends: &big_ends, items: &big_items },
                ],
                runs: &[Run { column: 2, op: Op::Set, positions: &[0, 1], values: Some(Values::Opaque { ends: &set_ends, items: &set_items }), except: &[], patches: None }],
                unrecorded: &[],
                touched: &[],
            },
        )
        .unwrap(),
        journal_section::write(
            &kinds,
            &TableHour {
                read: None,
                changed: &[],
                deleted: &[],
                inserted: &many_slots,
                keys: &many_keys,
                inserts: &[
                    Values::U16(&many_u16),
                    Values::OptionalU32 { present: &many_present, values: &many_values },
                    Values::Opaque { ends: &many_ends, items: &many_items },
                ],
                runs: &[],
                unrecorded: &[],
                touched: &[],
            },
        )
        .unwrap(),
    ];
    for (hour, bytes) in hours.iter().enumerate() {
        let manifest = crate::manifest::Manifest {
            branch: 0,
            hour: hour as i32,
            flags: 0,
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: Vec::new(),
            statuses: vec![Status::Complete],
            read: vec![1],
            samples: Vec::new(),
            sections: vec![crate::manifest::Section { table: 0, bytes: bytes.len() as u64, columns: vec![0b111], ops: 0 }],
        };
        crate::section::apply(bytes, &kinds, false, &mut table, &mut scratch, &mut changes).unwrap();
        adapter.begin_hour(&mut store, &manifest).unwrap();
        adapter.section(&mut store, &manifest.sections[0], bytes).unwrap();
        let log = &store.log().tables[0];
        assert!(
            log.born_written.capacity() == 0,
            "hour {hour}: a section gives every column of the rows it inserts, so the log keeps none of their cells; it holds {} for {} rows",
            log.born_written.len(),
            log.inserted.len()
        );
        adapter.end_hour(&mut store, &mut crate::unit::Discard).unwrap();
        let held = store.table_state(0).unwrap();
        assert_eq!(held.rows().collect::<Vec<_>>(), table.live().ones().collect::<Vec<_>>());
        for slot in table.live().ones() {
            assert_eq!(held.key(slot), table.key(slot));
            for column in 0..kinds.len() {
                let expected = if !table.recorded(column, slot) {
                    None
                } else {
                    let mut scratch = Vec::new();
                    owned(value_of(kinds[column], table.column(column).unwrap().cell(slot).unwrap(), &mut scratch).unwrap())
                };
                assert_eq!(owned(held.value(column, slot)), expected, "hour {hour} slot {slot} column {column}");
            }
        }
        if hour == 1 {
            assert_eq!(store.log().tables[0].touched, vec![2]);
            assert_eq!(store.log().tables[0].deleted, vec![(1, 11)]);
        }
    }
}

#[test]
fn trimming_keeps_the_bits_of_live_rows() {
    let mut flags = Flags::default();
    flags.set(130);
    flags.set(140);
    flags.truncate(300);
    assert_eq!(flags.ones().collect::<Vec<_>>(), vec![130, 140]);
    flags.truncate(135);
    assert_eq!(flags.ones().collect::<Vec<_>>(), vec![130]);
    let mut store = store();
    let mut changes = store.table(0).unwrap();
    for slot in 0..300_u32 {
        changes.insert_at(slot, u128::from(slot) + 1).unwrap();
        changes.write(slot, 3, Cell::Bool(slot == 130)).unwrap();
        changes.write(slot, 2, Cell::OptionalI64((slot == 140).then_some(7))).unwrap();
    }
    for slot in 200..300_u32 {
        changes.delete(slot).unwrap();
    }
    store.end_hour(&mut crate::unit::Discard).unwrap();
    store.end_block();
    let table = store.table_state(0).unwrap();
    assert_eq!(table.slots(), 200);
    assert_eq!(table.value(3, 130), Value::Number(1));
    assert_eq!(table.value(2, 140), Value::Number(7));
    let mut store = self::store();
    let mut changes = store.table(0).unwrap();
    for slot in 0..1100_u32 {
        changes.insert_at(slot, u128::from(slot) + 1).unwrap();
    }
    changes.write(130, 5, Cell::U32List(&[4, 5])).unwrap();
    for slot in 1050..1100_u32 {
        changes.delete(slot).unwrap();
    }
    store.end_hour(&mut crate::unit::Discard).unwrap();
    store.end_block();
    let table = store.table_state(0).unwrap();
    assert_eq!(table.slots(), 1050);
    assert_eq!(table.value(5, 130), Value::List(&[4, 0, 0, 0, 5, 0, 0, 0]));
}

#[test]
fn an_unwritten_column_of_one_inserted_row_leaves_the_next_rows_written_cells_recorded() {
    let kinds = vec![Kind::I32, Kind::U16];
    let mut store = Store::new(vec![kinds], vec![false]);
    store.begin_hour(0, &[]).unwrap();
    let mut changes = store.table(0).unwrap();
    changes.insert_at(1, 201).unwrap();
    changes.write(1, 0, Cell::I32(1)).unwrap();
    changes.insert_at(7, 207).unwrap();
    changes.write(7, 0, Cell::I32(7)).unwrap();
    changes.insert_at(9, 209).unwrap();
    changes.write(9, 0, Cell::I32(9)).unwrap();
    changes.write(9, 1, Cell::U16(3)).unwrap();
    store.settle();
    store.settle();
    let table = store.table_state(0).unwrap();
    assert_eq!(table.value(0, 1), Value::Number(1));
    assert_eq!(table.value(0, 7), Value::Number(7), "a later row's written cell stays recorded");
    assert_eq!(table.value(0, 9), Value::Number(9));
    assert_eq!(table.value(1, 9), Value::Number(3));
    assert_eq!(table.value(1, 1), Value::Unrecorded);
    assert_eq!(table.value(1, 7), Value::Unrecorded);
}

fn a_first_hour(store: &mut Store, hour: i32, appends: &[&[u8]]) {
    store.begin_hour(hour, appends).unwrap();
    store.status(0, Status::Complete, true, Some(hour), true).unwrap();
    store.status(1, Status::Truncated { missing: 3 }, true, Some(hour), true).unwrap();
    let mut changes = store.table(0).unwrap();
    for slot in 0..40_u32 {
        changes.insert_at(slot, 1000 + u128::from(slot)).unwrap();
        for (column, cell) in cells(u64::from(slot)).into_iter().enumerate() {
            if (slot, column) != (7, 0) {
                changes.write(slot, column, cell).unwrap();
            }
        }
    }
    changes.unrecord(9, 2).unwrap();
    changes.touch(3).unwrap();
    let mut changes = store.table(1).unwrap();
    changes.read_rows(&[]).unwrap();
    for slot in [0_u32, 2, 5] {
        changes.insert_at(slot, u128::from(slot) + 1).unwrap();
        for (column, cell) in cells(u64::from(slot)).into_iter().enumerate() {
            changes.write(slot, column, cell).unwrap();
        }
    }
    changes.read_done();
    store.status(2, Status::Complete, true, Some(hour), true).unwrap();
    let mut changes = store.table(2).unwrap();
    for slot in [0_u32, 2, 5] {
        changes.insert_at(slot, u128::from(slot) + 1).unwrap();
        for (column, cell) in cells(u64::from(slot)).into_iter().enumerate() {
            changes.write(slot, column, cell).unwrap();
        }
    }
    changes.unrecord(2, 1).unwrap();
    changes.delete(2).unwrap();
    store.settle();
}

#[test]
fn a_run_s_first_hour_keeps_no_log_and_reads_as_the_same_hour_logged() {
    use crate::summary::{Builder, Column, Facet, Shape, Source, aggregate};
    let hour = 1_000_003;
    let appends: [&[u8]; 3] = [b"zero", b"one", b"two"];
    let shapes = [
        Shape::Values(vec![Column { source: Source::Cell(1), aggregates: aggregate::ALL, rank: 0 }, Column { source: Source::Cell(2), aggregates: aggregate::FALLS, rank: 0 }]),
        Shape::Log(vec![Facet::Column(0)]),
    ];
    let pieces = |store: &mut Store, apply: &mut dyn FnMut(&mut Store)| -> Vec<Vec<u8>> {
        let mut builders: Vec<Builder> = shapes.iter().map(|shape| Builder::open(shape, &store.view(0), store.view(0).status(), hour)).collect();
        apply(store);
        for builder in &mut builders {
            let view = store.view(0);
            builder.hour(hour, &view, &store.log().tables[0], view.status(), view.view());
        }
        builders.into_iter().map(|builder| builder.finish(hour + 1).unwrap().unwrap().assemble(3).unwrap()).collect()
    };
    let schema = vec![KINDS.to_vec(); 3];
    let mut first = Store::new(schema.clone(), vec![false; 3]);
    first.begin_run(hour);
    let first_pieces = pieces(&mut first, &mut |store| a_first_hour(store, hour, &appends));
    for (table, log) in first.log().tables.iter().enumerate() {
        assert!(
            log.inserted.capacity() == 0 && log.deleted.capacity() == 0 && log.written.capacity() == 0 && log.touched.capacity() == 0 && log.born_written.capacity() == 0,
            "table {table}: a run's first hour logs nothing per row or cell; it holds {} inserted, {} written, {} written of rows born",
            log.inserted.len(),
            log.written.len(),
            log.born_written.len()
        );
    }
    first.end_hour(&mut crate::unit::Discard).unwrap();
    let mut logged = Store::new(schema, vec![false; 3]);
    logged.begin_run(hour - 1);
    logged.begin_hour(hour - 1, &appends).unwrap();
    logged.end_hour(&mut crate::unit::Discard).unwrap();
    let logged_pieces = pieces(&mut logged, &mut |store| a_first_hour(store, hour, &[]));
    logged.end_hour(&mut crate::unit::Discard).unwrap();
    for table in 0..3 {
        let (one, other) = (first.table_state(table).unwrap(), logged.table_state(table).unwrap());
        assert_eq!((one.status(), one.sampled_at(), one.view()), (other.status(), other.sampled_at(), other.view()), "table {table}");
        assert_eq!(one.rows().map(|slot| one.key(slot)).collect::<Vec<_>>(), other.rows().map(|slot| other.key(slot)).collect::<Vec<_>>(), "table {table}");
        for slot in one.rows() {
            for column in 0..KINDS.len() {
                assert_eq!(owned(one.value(column, slot)), owned(other.value(column, slot)), "table {table} slot {slot} column {column}");
            }
        }
    }
    assert_eq!(first.table_state(0).unwrap().value(0, 7), Value::Unrecorded);
    assert_eq!(first.table_state(0).unwrap().value(2, 9), Value::Unrecorded);
    assert_eq!(first.table_state(1).unwrap().view(), Some(&[0, 2, 5][..]));
    assert_eq!(first.table_state(2).unwrap().rows().collect::<Vec<_>>(), vec![0, 5]);
    assert_eq!(first.unwritten(), logged.unwritten());
    assert_eq!(first.unwritten()[0], 1);
    assert_eq!(first_pieces, logged_pieces);
}

#[test]
fn a_log_summary_of_a_run_s_first_hour_counts_the_rows_live_at_its_end() {
    use crate::summary::{Builder, Facet, Piece, Shape};
    let hour = 2_000_011;
    let shape = Shape::Log(vec![Facet::Column(0)]);
    let mut store = Store::new(vec![KINDS.to_vec()], vec![false]);
    store.begin_run(hour);
    let mut builder = Builder::open(&shape, &store.view(0), store.view(0).status(), hour);
    store.begin_hour(hour, &[b"zero", b"one", b"two"]).unwrap();
    store.status(0, Status::Complete, true, Some(hour), true).unwrap();
    let mut changes = store.table(0).unwrap();
    for slot in 0..5_u32 {
        changes.insert_at(slot, u128::from(slot) + 1).unwrap();
        for (column, cell) in cells(u64::from(slot)).into_iter().enumerate() {
            changes.write(slot, column, cell).unwrap();
        }
    }
    changes.delete(3).unwrap();
    store.settle();
    let view = store.view(0);
    builder.hour(hour, &view, &store.log().tables[0], view.status(), view.view());
    let piece = builder.finish(hour + 1).unwrap().unwrap().assemble(3).unwrap();
    let read = Piece::read(&piece).unwrap();
    let frame = read.column(0).unwrap();
    let total: u64 = (0..read.keys.len()).map(|at| frame.counts(&read, at).unwrap().iter().sum::<u64>()).sum();
    assert_eq!(total, 4, "the four rows live at the hour's end");
}

#[test]
fn a_partly_read_table_s_view_after_a_run_s_first_hour_holds_its_live_rows() {
    let hour = 3_000_017;
    let mut store = Store::new(vec![KINDS.to_vec()], vec![false]);
    store.begin_run(hour);
    store.begin_hour(hour, &[b"zero", b"one", b"two"]).unwrap();
    store.status(0, Status::Truncated { missing: 1 }, true, Some(hour), true).unwrap();
    let mut changes = store.table(0).unwrap();
    changes.read_rows(&[]).unwrap();
    for slot in [1_u32, 4, 6] {
        changes.insert_at(slot, u128::from(slot) + 1).unwrap();
        for (column, cell) in cells(u64::from(slot)).into_iter().enumerate() {
            changes.write(slot, column, cell).unwrap();
        }
    }
    changes.delete(4).unwrap();
    changes.read_done();
    store.end_hour(&mut crate::unit::Discard).unwrap();
    assert_eq!(store.table_state(0).unwrap().view(), Some(&[1, 6][..]));
}

fn with_log() -> Store {
    let mut store = Store::new(vec![KINDS.to_vec(); 2], vec![false; 2]);
    store.declare_logs(&[false, true]).unwrap();
    store.begin_hour(0, &[b"zero", b"one", b"two"]).unwrap();
    store
}

#[test]
fn what_may_not_be_done_to_a_log_is_refused_by_name() {
    let mut store = with_log();
    let mut changes = store.table(0).unwrap();
    changes.insert_at(0, 7).unwrap();
    let refused = changes.append(1, &[None; 6]).unwrap_err().to_string();
    assert!(refused.contains("holds state") && refused.contains("never appended"), "{refused}");
    let refused = changes.forget(0).unwrap_err().to_string();
    assert!(refused.contains("holds state") && refused.contains("never forgotten"), "{refused}");
    let mut changes = store.table(1).unwrap();
    let cells = cells(1).map(Some);
    assert_eq!(changes.append(10, &cells).unwrap(), 0);
    let edits = Edits { hunks: &[], items: &[] };
    let attempts: Vec<(&str, io::Result<()>)> = vec![
        ("inserted", changes.insert(5).map(drop)),
        ("inserted", changes.insert_at(1, 5)),
        ("inserted", changes.insert_given(5).map(drop)),
        ("inserted", changes.insert_given_at(1, 5)),
        ("deleted", changes.delete(0)),
        ("written", changes.write(0, 0, Cell::U8(1))),
        ("written", changes.put_value(0, 0, Value::Number(1))),
        ("written", changes.rewrite(0, 0)),
        ("offered", changes.offer(0, 0, Cell::U8(1)).map(drop)),
        ("added to", changes.add(0, 1, 1)),
        ("edited", changes.edit(0, 5, edits)),
        ("filled", changes.fill(0, Cell::U8(1), &[]).map(drop)),
        ("unrecorded", changes.unrecord(0, 0)),
        ("touched", changes.touch(0)),
        ("read in part", changes.read_rows(&[0])),
    ];
    for (what, attempt) in attempts {
        let refused = attempt.unwrap_err().to_string();
        assert!(refused.contains("Table 1 is a log") && refused.ends_with(&format!("never {what}")), "{refused}");
    }
    assert_eq!(changes.slot(10), None, "a log keeps no row to find by key");
    let refused = changes.forget(1).unwrap_err().to_string();
    assert!(refused.contains("never appended"), "{refused}");
    changes.forget(0).unwrap();
    let refused = changes.forget(0).unwrap_err().to_string();
    assert!(refused.contains("forgot before"), "{refused}");
    let refused = changes.append(11, &[None; 5]).unwrap_err().to_string();
    assert!(refused.contains("5 cells") && refused.contains("6 columns"), "{refused}");
    let refused = changes.append(11, &[Some(Cell::I32(1)), None, None, None, None, None]).unwrap_err().to_string();
    assert!(refused.contains("I32") && refused.contains("U8"), "{refused}");
    assert_eq!(store.table_state(1).unwrap().count(), Some(1), "a refused append appends nothing");
    let refused = store.declare_logs(&[true, true]).unwrap_err().to_string();
    assert!(refused.contains("after it holds rows"), "{refused}");
}

#[test]
fn a_log_holds_a_count_and_its_live_span_alone() {
    let mut store = with_log();
    let cells = cells(4).map(Some);
    let mut ordinal = 0_u64;
    for hour in 0..1_000 {
        store.begin_hour(hour, &[]).unwrap();
        let mut changes = store.table(1).unwrap();
        for _ in 0..1_000 {
            assert_eq!(changes.append(u128::from(ordinal) << 64 | 99, &cells).unwrap(), ordinal);
            if ordinal >= 200_000 {
                changes.forget(ordinal - 200_000).unwrap();
            }
            ordinal += 1;
        }
        store.end_hour(&mut crate::unit::Discard).unwrap();
    }
    store.end_block();
    let table = store.table_state(1).unwrap();
    assert_eq!(table.count(), Some(1_000_000));
    let listed = table.listed().unwrap();
    assert_eq!((listed.len(), listed.lowest()), (200_000, Some(800_000)));
    let (_, tables) = store.bytes();
    let bytes = tables[1];
    let span = 200_000_usize.div_ceil(64) * 8;
    assert_eq!((bytes.live_rows, bytes.keys, bytes.index, bytes.views, bytes.last_complete), (0, 0, 0, 0, 0), "{bytes:?}");
    assert_eq!(bytes.columns, ColumnBytes::default(), "no cell is held");
    assert!(bytes.live >= span && bytes.live <= 2 * span + 64, "the live span's bits alone: {} bytes for {span}", bytes.live);
    assert!(bytes.total() <= 2 * span + 64, "{bytes:?}");
}

#[test]
fn an_hour_s_appended_rows_are_the_log_s_view_and_summary() {
    use crate::summary::{Builder, Facet, Piece, Shape};
    let hour = 4_000_037;
    let mut store = with_log();
    store.begin_hour(hour - 1, &[]).unwrap();
    store.table(1).unwrap().append(1, &cells(0).map(Some)).unwrap();
    store.end_hour(&mut crate::unit::Discard).unwrap();
    let shape = Shape::Log(vec![Facet::Column(0)]);
    let mut builder = Builder::open(&shape, &store.view(1), store.view(1).status(), hour);
    store.begin_hour(hour, &[]).unwrap();
    store.status(1, Status::Truncated { missing: 2 }, true, Some(hour), true).unwrap();
    let mut changes = store.table(1).unwrap();
    let mut given = Vec::new();
    for seed in 1..=4_u64 {
        let mut row = cells(seed).map(Some);
        if seed == 2 {
            row[2] = None;
        }
        given.push(row);
        assert_eq!(changes.append(u128::from(seed) + 100, &given[seed as usize - 1]).unwrap(), seed);
    }
    changes.forget(0).unwrap();
    changes.forget(3).unwrap();
    let view = store.view(1);
    assert!(view.is_log() && view.appended_from() == Some(1));
    assert_eq!((view.status(), view.sampled_at(), view.view()), (Status::Truncated { missing: 2 }, Some(hour), None));
    assert_eq!(view.rows().collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    let mut scratch = Vec::new();
    for (at, row) in given.iter().enumerate() {
        assert_eq!(view.key(at as u32), Some(at as u128 + 101));
        for (column, cell) in row.iter().enumerate() {
            let wanted = owned(cell.map_or(Value::Unrecorded, |cell| value_of(KINDS[column], cell, &mut scratch).unwrap()));
            assert_eq!(owned(view.value(column, at as u32)), wanted, "row {at} column {column}");
        }
    }
    let log = &store.log().tables[1];
    assert_eq!((log.appended, log.appended_from, log.forgotten.as_slice()), (4, 1, &[0..1, 3..4][..]));
    builder.hour(hour, &view, log, view.status(), view.view());
    let piece = builder.finish(hour + 1).unwrap().unwrap().assemble(3).unwrap();
    let read = Piece::read(&piece).unwrap();
    let frame = read.column(0).unwrap();
    let total: u64 = (0..read.keys.len()).map(|at| frame.counts(&read, at).unwrap().iter().sum::<u64>()).sum();
    assert_eq!(total, 4, "the hour's four appended rows");
    let listed = store.table_state(1).unwrap().listed().unwrap();
    assert_eq!(listed.ones().collect::<Vec<_>>(), vec![1, 2, 4]);
    store.end_hour(&mut crate::unit::Discard).unwrap();
    store.begin_hour(hour + 1, &[]).unwrap();
    assert_eq!(store.view(1).rows().count(), 0);
    assert_eq!(store.log().tables[1].appended, 0);
}

#[test]
fn the_key_index_holds_the_last_slot_the_format_records() {
    let keys = Keys::new();
    let mut index = KeyIndex::default();
    index.insert(0, SLOTS - 1, &keys);
    assert_eq!(index.find(0, &keys), Some(SLOTS - 1));
    index.remove(0, SLOTS - 1, &keys);
    assert_eq!(index.find(0, &keys), None);
}

#[test]
fn a_row_past_the_format_s_slot_width_is_refused_by_name() {
    let mut store = store();
    let mut changes = store.table(0).unwrap();
    for refused in [changes.insert_at(SLOTS, 7), changes.insert_given_at(SLOTS, 7)] {
        let refused = refused.unwrap_err().to_string();
        assert!(refused.contains("Table 0: more than 4,294,967,295 rows"), "{refused}");
    }
    assert_eq!((changes.held().slots(), changes.held().rows().count()), (0, 0), "a refused row holds nothing");
    changes.insert_at(3, 7).unwrap();
    assert_eq!(changes.slot(7), Some(3));
    let mut apart = TableState::new(&KINDS);
    let refused = apart.insert_at(SLOTS, 7).unwrap_err().to_string();
    assert!(refused.contains("more than 4,294,967,295 rows"), "{refused}");
    assert_eq!(apart.slots(), 0);
    let last_word = (SLOTS / 64) as usize;
    assert_eq!(slot_at(last_word, 62), Some(SLOTS - 1));
    assert_eq!(slot_at(last_word, 63), None);
    assert_eq!(slot_at(last_word + 1, 0), None);
}

#[test]
fn strings_past_their_width_or_into_a_writer_s_are_refused_by_name() {
    let mut wide = Store::new(vec![KINDS.to_vec()], vec![false]);
    wide.resume(vec![TableState::new(&KINDS)], STRINGS - 1).unwrap();
    wide.begin_hour(0, &[b"last"]).unwrap();
    assert_eq!(wide.strings().get(STRINGS - 1), Some(&b"last"[..]));
    let refused = wide.begin_hour(1, &[b"past"]).unwrap_err().to_string();
    assert!(refused.contains("4,294,967,295, the format's string id width"), "{refused}");
    assert_eq!((wide.hour(), wide.strings().next()), (0, STRINGS), "a refused hour changes nothing");
    assert!(wide.intern(b"past").unwrap_err().to_string().contains("string id width"));
    assert_eq!(wide.intern(b"last").unwrap(), STRINGS - 1);

    let mut store = store();
    store.pin_string(5, b"a writer's").unwrap();
    store.begin_hour(1, &[b"three", b"four"]).unwrap();
    let refused = store.begin_hour(2, &[b"five"]).unwrap_err().to_string();
    assert!(refused.contains("reach 5, a writer's string"), "{refused}");
    assert!(store.intern(b"five").unwrap_err().to_string().contains("reach 5"));
    assert_eq!(store.strings().get(5), Some(&b"a writer's"[..]));
}

#[test]
fn a_log_s_listed_set_holds_its_last_ordinal() {
    let mut listed = Listed::default();
    listed.append(u64::MAX - 1);
    assert!(listed.get(u64::MAX - 1) && listed.all(u64::MAX - 1, u64::MAX));
    listed.clear_range(u64::MAX - 1, u64::MAX);
    assert!(listed.is_empty());
}

#[test]
fn the_journal_adapter_refuses_a_state_table_past_the_slot_width_by_name() {
    let kinds = vec![Kind::U16];
    let mut store = Store::new(vec![kinds.clone(); 2], vec![false; 2]);
    let mut adapter = Journal::default();
    let inserting = |slot: u32, key: u128| {
        let hour = TableHour { read: None, changed: &[], deleted: &[], inserted: &[slot], keys: &[key], inserts: &[Values::U16(&[5])], runs: &[], unrecorded: &[], touched: &[] };
        journal_section::write(&kinds, &hour).unwrap()
    };
    let hours = [vec![(0, inserting(0, 1))], vec![(0, inserting(SLOTS, 2)), (1, inserting(0, 3))], vec![(0, inserting(1, 4))]];
    for (hour, sections) in hours.iter().enumerate() {
        if hour == 2 {
            let mut again = Journal::default();
            again.load(&adapter.save()).unwrap();
            adapter = again;
        }
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
            sections: sections.iter().map(|(table, bytes)| crate::manifest::Section { table: *table, bytes: bytes.len() as u64, columns: vec![0b1], ops: 0 }).collect(),
        };
        adapter.begin_hour(&mut store, &manifest).unwrap();
        for (entry, (_, bytes)) in manifest.sections.iter().zip(sections) {
            adapter.section(&mut store, entry, bytes).unwrap();
        }
        adapter.end_hour(&mut store, &mut crate::unit::Discard).unwrap();
        let held = store.table_state(0).unwrap();
        assert_eq!((held.rows().collect::<Vec<_>>(), held.key(0)), (vec![0], Some(1)), "hour {hour}: nothing of a refused section lands");
        if hour == 0 {
            assert_eq!(held.status(), Status::Complete);
            continue;
        }
        assert_eq!(held.status(), Status::Unavailable { reason: u32::MAX }, "hour {hour}");
        let reason = String::from_utf8(store.strings().get(u32::MAX).unwrap().to_vec()).unwrap();
        assert_eq!(reason, "table 0: more than 4,294,967,295 rows, the format's slot width");
        let other = store.table_state(1).unwrap();
        assert_eq!((other.status(), other.rows().collect::<Vec<_>>()), (Status::Complete, vec![0]), "hour {hour}: the hour's other tables land");
    }
}

#[test]
fn the_journal_adapter_refuses_a_section_of_a_table_it_leaves_by_name() {
    let kinds = vec![Kind::U16];
    let mut store = Store::new(vec![kinds.clone(); 2], vec![false; 2]);
    let names = super::journal::Names { table: |table| format!("Written{table}"), column: |_, column| format!("{column}") };
    let mut adapter = Journal::leaving(&[1], names);
    let hour = TableHour { read: None, changed: &[], deleted: &[], inserted: &[0], keys: &[1], inserts: &[Values::U16(&[5])], runs: &[], unrecorded: &[], touched: &[] };
    let bytes = journal_section::write(&kinds, &hour).unwrap();
    let manifest = crate::manifest::Manifest {
        branch: 0,
        hour: 0,
        flags: 0,
        total_ns: 0,
        pool_bytes: 0,
        dropped_frames: 0,
        appends: Vec::new(),
        statuses: vec![Status::Complete; 2],
        read: vec![0b11],
        samples: Vec::new(),
        sections: vec![crate::manifest::Section { table: 1, bytes: bytes.len() as u64, columns: vec![0b1], ops: 0 }],
    };
    let refused = adapter.begin_hour(&mut store, &manifest).unwrap_err();
    assert_eq!(refused.to_string(), "The journal holds a section of Written1, which the recorder writes");
}

#[test]
fn a_kept_table_logs_each_older_cell_before_its_writes_and_each_deleted_row_s_cells() {
    let kinds = vec![Kind::I64, Kind::U8];
    let mut store = Store::new(vec![kinds.clone(), kinds.clone(), kinds], vec![false; 3]);
    store.keep_previous(0);
    store.keep_dropped(2);
    store.begin_run(0);
    for hour in 0..3 {
        store.begin_hour(hour, &[]).unwrap();
        for table in 0..3 {
            let mut changes = store.table(table).unwrap();
            match hour {
                0 => {
                    for key in [5_u128, 6] {
                        let slot = changes.insert(key).unwrap();
                        changes.write(slot, 0, Cell::I64(key as i64 * 10)).unwrap();
                        changes.write(slot, 1, Cell::U8(1)).unwrap();
                    }
                }
                1 => {
                    let five = changes.slot(5).unwrap();
                    changes.write(five, 0, Cell::I64(51)).unwrap();
                    changes.write(five, 0, Cell::I64(52)).unwrap();
                    let six = changes.slot(6).unwrap();
                    changes.write(six, 0, Cell::I64(61)).unwrap();
                    changes.delete(six).unwrap();
                    let seven = changes.insert(7).unwrap();
                    changes.write(seven, 0, Cell::I64(70)).unwrap();
                    changes.delete(seven).unwrap();
                }
                _ => {}
            }
        }
        store.settle();
        let (kept, plain, deletions) = (&store.log().tables[0], &store.log().tables[1], &store.log().tables[2]);
        assert!(plain.previous.is_empty() && plain.dropped.is_empty(), "a table not kept logs nothing more, hour {hour}");
        assert!(deletions.previous.is_empty(), "a table keeping its deleted rows keeps no older cell of a write, hour {hour}");
        assert_eq!(deletions.dropped, kept.dropped, "it keeps each deleted row's cells as a kept table does, hour {hour}");
        if hour == 1 {
            let five = store.view(0).slot(5).unwrap();
            let first = kept.previous.iter().find(|(slot, column, _)| *slot == five && *column == 0).map(|(_, _, value)| value.clone());
            assert_eq!(first, Some(Kept::Number(50)), "a cell's first entry is its value before the hour");
            assert_eq!(kept.dropped.iter().next(), Some((&6, &[Kept::Number(61), Kept::Number(1)][..])), "a deleted row's cells as the deletion found them");
            let born: Vec<(u128, Kept)> = kept.dropped.iter().skip(1).map(|(key, cells)| (*key, cells[0].clone())).collect();
            assert_eq!(born, [(7, Kept::Number(70))], "a row born and deleted within the hour is dropped too");
        } else {
            assert!(kept.previous.is_empty() && kept.dropped.is_empty(), "hour {hour} keeps nothing: a run's first hour, then an hour that changed nothing");
        }
        store.end_hour(&mut crate::unit::Discard).unwrap();
    }
}

#[test]
fn texts_sharing_a_digest_each_reuse_their_own_id_across_a_trim() {
    COLLIDING.set(true);
    let mut strings = Strings::default();
    assert_eq!(strings.intern(b"first").unwrap(), 0);
    assert_eq!(strings.intern(b"second").unwrap(), 1);
    assert_eq!(strings.intern(b"second").unwrap(), 1, "the second text is found behind the first");
    assert_eq!(strings.intern(b"first").unwrap(), 0);
    let mut named = Flags::default();
    named.set(1);
    strings.retain(&named);
    assert_eq!(strings.get(0), None);
    assert_eq!(strings.intern(b"second").unwrap(), 1, "the rebuilt index still finds a held text");
    assert_eq!(strings.intern(b"first").unwrap(), 2);
    assert_eq!(strings.intern(b"first").unwrap(), 2);
    assert_eq!(strings.next(), 3);
    COLLIDING.set(false);
}

#[test]
fn key_index_and_strings_use_keyed_hashing_and_preserve_pack_lookup_parity() {
    use std::hash::BuildHasher;

    let index = KeyIndex::default();
    let _: &foldhash::fast::RandomState = &index.hasher;
    let mut strings = Strings::default();
    let _: &foldhash::fast::RandomState = strings.held.hasher();
    let _: &foldhash::fast::RandomState = strings.pinned.hasher();
    strings.intern(b"alpha").unwrap();
    let text_index = strings.by_text.as_ref().expect("by_text built on intern");
    let _: &foldhash::fast::RandomState = &text_index.hasher;
    let _: &foldhash::fast::RandomState = text_index.first.hasher();
    let _: &foldhash::fast::RandomState = text_index.more.hasher();

    let legacy_bucket = |key: u128, mask: usize| -> usize {
        let mixed = (key as u64 ^ (key >> 64) as u64 ^ (key >> 64) as u64 >> 29).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        (mixed >> 32) as usize & mask
    };
    let count = 512usize;
    let buckets = count.next_power_of_two();
    let mask = buckets - 1;
    let mut adversarial = Vec::with_capacity(count);
    let mut candidate = 1u128;
    while adversarial.len() < count {
        if legacy_bucket(candidate, mask) == 0 {
            adversarial.push(candidate);
        }
        candidate += 1;
    }
    assert!(adversarial.iter().all(|&key| legacy_bucket(key, mask) == 0));

    let seeded = foldhash::fast::FixedState::with_seed(0x9e37_79b9_7f4a_7c15);
    let mut load = vec![0u32; buckets];
    for &key in &adversarial {
        load[seeded.hash_one(key) as usize & mask] += 1;
    }
    let longest = *load.iter().max().unwrap();
    assert!(longest <= 12, "{longest} legacy-bucket keys shared a bucket under explicit foldhash seed");

    let mut table = TableState::new(&KINDS);
    let mut twin = TableState::new(&KINDS);
    for (slot, &key) in adversarial.iter().enumerate() {
        table.insert_at(slot as u32, key).unwrap();
        twin.insert_at(slot as u32, key).unwrap();
    }
    table.index_keys();
    twin.index_keys();
    assert_eq!(table, twin);
    for (slot, &key) in adversarial.iter().enumerate() {
        assert_eq!(table.find(key), Some(slot as u32));
        assert_eq!(table.slot(key), Some(slot as u32));
    }
    assert_eq!(table.find(u128::MAX), None);

    for slot in (0..count as u32).step_by(2) {
        assert_eq!(table.delete(slot).unwrap(), adversarial[slot as usize]);
    }
    table.pack().unwrap();
    table.index_keys();
    for (slot, &key) in adversarial.iter().enumerate() {
        if slot % 2 == 0 {
            assert_eq!(table.find(key), None);
        } else {
            assert_eq!(table.find(key), Some((slot / 2) as u32));
            assert_eq!(table.key((slot / 2) as u32), Some(key));
        }
    }
}

#[test]
fn put_rows_rejects_duplicate_live_keys_by_name() {
    let mut table = TableState::new(&KINDS);
    table.set_slots(3);
    let refused = table.put_rows(0, 3, &[0b101], &[0x77, 0x77]).unwrap_err().to_string();
    assert!(refused.contains("0x77") && refused.contains("already held"), "{refused}");

    let mut unindexed = TableState::new(&KINDS);
    unindexed.insert_at(0, 0x88).unwrap();
    unindexed.insert_at(2, 0x88).unwrap();
    assert_eq!(unindexed.slot(0x88), Some(0));
    let refused = unindexed.try_index_keys().unwrap_err().to_string();
    assert!(refused.contains("0x88") && refused.contains("already held"), "{refused}");
    assert_eq!(unindexed.delete(0).unwrap(), 0x88);
    assert_eq!(unindexed.find(0x88), Some(2));
    assert_eq!(unindexed.slot(0x88), Some(2));
    unindexed.try_index_keys().unwrap();
}
