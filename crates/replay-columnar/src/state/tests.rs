use super::*;
use std::collections::BTreeMap;

const KINDS: [Kind; 4] = [Kind::U32, Kind::OptionalI64, Kind::U32List, Kind::Bool];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    key: u128,
    count: u32,
    balance: Option<i64>,
    members: Vec<u32>,
    open: bool,
}

fn assert_matches(table: &Table, model: &BTreeMap<u32, Row>) {
    assert_eq!(
        table.live().ones().collect::<Vec<_>>(),
        model.keys().copied().collect::<Vec<_>>()
    );
    for (&slot, row) in model {
        assert_eq!(table.key(slot), Some(row.key), "slot {slot}");
        assert_eq!(
            table.column(0).unwrap().cell(slot),
            Some(Cell::U32(row.count))
        );
        assert_eq!(
            table.column(1).unwrap().cell(slot),
            Some(Cell::OptionalI64(row.balance))
        );
        assert_eq!(
            table.column(2).unwrap().cell(slot),
            Some(Cell::U32List(&row.members))
        );
        assert_eq!(
            table.column(3).unwrap().cell(slot),
            Some(Cell::Bool(row.open))
        );
    }
}

fn insert(table: &mut Table, model: &mut BTreeMap<u32, Row>, rows: &[(u32, Row)]) {
    let slots: Vec<u32> = rows.iter().map(|(slot, _)| *slot).collect();
    let keys: Vec<u128> = rows.iter().map(|(_, row)| row.key).collect();
    let counts: Vec<u32> = rows.iter().map(|(_, row)| row.count).collect();
    let mut present = vec![0u64; rows.len().div_ceil(64)];
    let mut balances = Vec::new();
    for (index, (_, row)) in rows.iter().enumerate() {
        if let Some(balance) = row.balance {
            present[index / 64] |= 1 << (index % 64);
            balances.push(balance);
        }
    }
    let mut ends = Vec::new();
    let mut items = Vec::new();
    for (_, row) in rows {
        items.extend_from_slice(&row.members);
        ends.push(items.len() as u32);
    }
    let open: Vec<u8> = rows.iter().map(|(_, row)| u8::from(row.open)).collect();
    table
        .insert(
            &slots,
            &keys,
            &[
                Values::U32(&counts),
                Values::OptionalI64 {
                    present: &present,
                    values: &balances,
                },
                Values::U32List {
                    ends: &ends,
                    items: &items,
                },
                Values::Bool(&open),
            ],
        )
        .unwrap();
    for (slot, row) in rows {
        model.insert(*slot, row.clone());
    }
}

fn row(key: u128, count: u32, balance: Option<i64>, members: &[u32]) -> Row {
    Row {
        key,
        count,
        balance,
        members: members.to_vec(),
        open: count % 2 == 0,
    }
}

#[test]
fn runs_apply_in_place_like_an_independent_row_model() {
    let mut table = Table::new(&KINDS).unwrap();
    let mut model = BTreeMap::new();
    insert(
        &mut table,
        &mut model,
        &[
            (0, row(10, 1, Some(-5), &[1, 2])),
            (1, row(11, 2, None, &[])),
            (2, row(12, 3, Some(7), &[3])),
            (70, row(13, 4, Some(0), &[4, 5, 6])),
        ],
    );
    assert_matches(&table, &model);

    table.set(0, &[1, 70], Values::U32(&[20, 40])).unwrap();
    model.get_mut(&1).unwrap().count = 20;
    model.get_mut(&70).unwrap().count = 40;
    table
        .set(
            1,
            &[0, 1],
            Values::OptionalI64 {
                present: &[0b10],
                values: &[9],
            },
        )
        .unwrap();
    model.get_mut(&0).unwrap().balance = None;
    model.get_mut(&1).unwrap().balance = Some(9);
    table
        .set(
            2,
            &[2, 70],
            Values::U32List {
                ends: &[0, 2],
                items: &[8, 9],
            },
        )
        .unwrap();
    model.get_mut(&2).unwrap().members = vec![];
    model.get_mut(&70).unwrap().members = vec![8, 9];
    assert_matches(&table, &model);

    table.add(0, &[0, 2], Values::U32(&[u32::MAX, 5])).unwrap();
    model.get_mut(&0).unwrap().count = 0;
    model.get_mut(&2).unwrap().count = 8;
    table.add(1, &[1, 70], Values::I64(&[-10, 3])).unwrap();
    model.get_mut(&1).unwrap().balance = Some(-1);
    model.get_mut(&70).unwrap().balance = Some(3);
    assert!(
        table.add(1, &[0], Values::I64(&[1])).is_err(),
        "ADD to missing"
    );
    assert_matches(&table, &model);

    let mut targets = Vec::new();
    table
        .fill(0, &[2], Values::U32(&[100]), &mut targets)
        .unwrap();
    assert_eq!(
        targets,
        model
            .keys()
            .copied()
            .filter(|&slot| slot != 2)
            .collect::<Vec<_>>()
    );
    for (slot, row) in &mut model {
        if *slot != 2 {
            row.count = 100;
        }
    }
    assert_matches(&table, &model);

    table.delete(&[1, 2]).unwrap();
    model.remove(&1);
    model.remove(&2);
    assert_eq!(table.key(1), None);
    insert(&mut table, &mut model, &[(1, row(14, 6, None, &[7]))]);
    assert_matches(&table, &model);
    assert_eq!(table.slots(), 71);
    for (&slot, row) in &model {
        assert_eq!(table.slot(row.key), Some(slot));
    }
    assert_eq!(table.slot(11), None, "a deleted row's key");
    let mut taken = table.clone();
    assert!(
        taken.insert_keys(&[2], &[14]).is_err(),
        "a key another live row holds"
    );
}

#[test]
fn a_restart_restores_the_same_table() {
    let mut table = Table::new(&KINDS).unwrap();
    let mut model = BTreeMap::new();
    insert(
        &mut table,
        &mut model,
        &[
            (0, row(1, 2, Some(3), &[4])),
            (3, row(5, 6, None, &[])),
            (64, row(7, 8, Some(-9), &[10, 11])),
        ],
    );
    let mut live = Bits::default();
    for slot in [0, 3, 64] {
        live.set(slot).unwrap();
    }
    let restored = Table::restore(
        &KINDS,
        65,
        live,
        &[1, 5, 7],
        &[
            Values::U32(&[2, 6, 8]),
            Values::OptionalI64 {
                present: &[0b101],
                values: &[3, -9],
            },
            Values::U32List {
                ends: &[1, 1, 3],
                items: &[4, 10, 11],
            },
            Values::Bool(&[1, 1, 1]),
        ],
    )
    .unwrap();
    assert_matches(&restored, &model);
    assert_eq!(restored.slots(), table.slots());
}

#[test]
fn a_restart_past_the_last_u32_slot_is_refused() {
    let error = Table::with_rows(&KINDS, SLOT_WORDS * 64 + 1, Bits::default(), &[]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Restart table's slot count passes the last slot"
    );
    assert!(Bits::from_words(vec![1 << 63]).is_ok());
}

#[test]
fn runs_the_grammar_forbids_are_refused_by_name() {
    let mut table = Table::new(&KINDS).unwrap();
    let mut model = BTreeMap::new();
    insert(
        &mut table,
        &mut model,
        &[(0, row(1, 1, None, &[])), (1, row(2, 2, None, &[]))],
    );
    let refused = |result: io::Result<()>| result.unwrap_err().kind();
    assert_eq!(
        refused(table.set(0, &[5], Values::U32(&[1]))),
        io::ErrorKind::InvalidData,
        "a slot holding no row"
    );
    assert_eq!(
        refused(table.set(0, &[1, 0], Values::U32(&[1, 2]))),
        io::ErrorKind::InvalidData,
        "slots out of order"
    );
    assert_eq!(
        refused(table.set(0, &[0], Values::I32(&[1]))),
        io::ErrorKind::InvalidData,
        "values of another kind"
    );
    assert_eq!(
        refused(table.set(0, &[0, 1], Values::U32(&[1]))),
        io::ErrorKind::InvalidData,
        "fewer values than slots"
    );
    assert_eq!(
        refused(table.set(3, &[0], Values::Bool(&[2]))),
        io::ErrorKind::InvalidData,
        "a bool that is neither 0 nor 1"
    );
    assert_eq!(
        refused(table.add(2, &[0], Values::U32(&[1]))),
        io::ErrorKind::InvalidData,
        "ADD to a list"
    );
    assert_eq!(
        refused(table.set(9, &[0], Values::U32(&[1]))),
        io::ErrorKind::InvalidData,
        "a column past the schema"
    );
    assert!(
        table
            .insert(
                &[1],
                &[3],
                &[
                    Values::U32(&[1]),
                    Values::OptionalI64 {
                        present: &[0],
                        values: &[],
                    },
                    Values::U32List {
                        ends: &[0],
                        items: &[],
                    },
                    Values::Bool(&[0]),
                ],
            )
            .is_err(),
        "an insert at a slot holding a row"
    );
    assert_matches(&table, &model);
}

#[test]
fn uniform_pages_narrow_keys_and_empty_list_pages_roundtrip_and_widen() {
    let mut table = Table::new(&KINDS).unwrap();
    let mut model = BTreeMap::new();
    let slots: Vec<(u32, Row)> = (0..300)
        .map(|slot| (slot, row(u128::from(slot + 1), 7, None, &[])))
        .collect();
    insert(&mut table, &mut model, &slots);
    let initial_model = model.clone();
    let weak = table.downgrade().unwrap();
    assert!(weak.alive());
    let upgraded_initial = weak.upgrade().unwrap().unwrap();
    assert_matches(&upgraded_initial, &initial_model);
    drop(upgraded_initial);
    assert_matches(&table, &model);

    table.set(0, &[10], Values::U32(&[99])).unwrap();
    model.get_mut(&10).unwrap().count = 99;
    assert_matches(&table, &model);

    table.set(0, &[10], Values::U32(&[7])).unwrap();
    model.get_mut(&10).unwrap().count = 7;
    assert_matches(&table, &model);

    table
        .set(
            2,
            &[260],
            Values::U32List {
                ends: &[2],
                items: &[5, 6],
            },
        )
        .unwrap();
    model.get_mut(&260).unwrap().members = vec![5, 6];
    assert_matches(&table, &model);

    table
        .set(
            2,
            &[260],
            Values::U32List {
                ends: &[0],
                items: &[],
            },
        )
        .unwrap();
    model.get_mut(&260).unwrap().members.clear();
    assert_matches(&table, &model);

    let wide_key = u128::from(u64::MAX) + 7;
    insert(&mut table, &mut model, &[(300, row(wide_key, 42, Some(-3), &[9]))]);
    assert_eq!(table.slot(wide_key), Some(300));
    assert_eq!(table.key(300), Some(wide_key));
    assert_matches(&table, &model);

    let weak_after = table.downgrade().unwrap();
    assert!(weak_after.alive());
    let upgraded = weak_after.upgrade().unwrap().unwrap();
    assert_matches(&upgraded, &model);
}
