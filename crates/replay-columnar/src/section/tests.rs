use super::*;

const KINDS: [Kind; 4] = [Kind::U32, Kind::OptionalI64, Kind::U64List, Kind::Bytes];

fn hour<'a>(
    changed: &'a [u32],
    deleted: &'a [u32],
    inserted: &'a [u32],
    keys: &'a [u128],
    inserts: &'a [Values<'a>],
    runs: &'a [Run<'a>],
) -> TableHour<'a> {
    TableHour {
        read: None,
        changed,
        deleted,
        inserted,
        keys,
        inserts,
        runs,
        touched: &[],
        unrecorded: &[],
    }
}

fn run<'a>(column: usize, op: Op, positions: &'a [u32], values: Option<Values<'a>>) -> Run<'a> {
    Run {
        column,
        op,
        positions,
        values,
        except: &[],
        patches: None,
    }
}

fn applied(table: &mut Table, bytes: &[u8]) -> Changes {
    let mut changes = Changes::default();
    apply(
        bytes,
        &KINDS,
        false,
        table,
        &mut Scratch::default(),
        &mut changes,
    )
    .unwrap();
    changes
}

#[test]
fn written_sections_apply_like_the_same_runs_on_the_state() {
    let inserts = [
        Values::U32(&[5, 6, 7]),
        Values::OptionalI64 {
            present: &[0b101],
            values: &[-1, 9],
        },
        Values::U64List {
            ends: &[2, 2, 3],
            items: &[1, 2, 3],
        },
        Values::Bytes(&[40, 41, 42]),
    ];
    let first = hour(&[], &[], &[0, 1, 2], &[100, 101, u128::MAX], &inserts, &[]);
    let mut read = Table::new(&KINDS).unwrap();
    let changes = applied(&mut read, &write(&KINDS, &first).unwrap());
    assert_eq!(changes.inserted, [0, 1, 2]);
    let mut direct = Table::new(&KINDS).unwrap();
    direct
        .insert(&[0, 1, 2], &[100, 101, u128::MAX], &inserts)
        .unwrap();
    assert_eq!(read, direct);

    // A later hour: deletes, a reused slot, SET/ADD/NOOP runs, a FILL with an exception.
    let reinsert = [
        Values::U32(&[8]),
        Values::OptionalI64 {
            present: &[0],
            values: &[],
        },
        Values::U64List {
            ends: &[0],
            items: &[],
        },
        Values::Bytes(&[43]),
    ];
    let runs = [
        run(0, Op::Set, &[0], Some(Values::U32(&[50]))),
        run(1, Op::Add, &[0, 1], Some(Values::I64(&[2, -3]))),
        run(
            2,
            Op::Set,
            &[1],
            Some(Values::U64List {
                ends: &[1],
                items: &[u64::MAX],
            }),
        ),
        run(3, Op::Noop, &[1], None),
        Run {
            column: 3,
            op: Op::Fill,
            positions: &[],
            values: Some(Values::Bytes(&[44])),
            except: &[2],
            patches: None,
        },
    ];
    let later = TableHour {
        touched: &[2],
        unrecorded: &[],
        ..hour(&[0, 2], &[1], &[1], &[102], &reinsert, &runs)
    };
    let changes = applied(&mut read, &write(&KINDS, &later).unwrap());
    assert_eq!(
        changes,
        Changes {
            changed: vec![0, 2],
            // Slot 0: SET 0 and ADD 1; slot 2: ADD 1, SET 2 and the NOOP of 3.
            masks: vec![0b0011, 0b1110],
            deleted: vec![1],
            deleted_keys: vec![101],
            inserted: vec![1],
            touched: vec![2],
            filled: vec![3],
            fill_slots: vec![0, 1],
            fill_masks: vec![0b1000, 0b1000],
            ..Changes::default()
        }
    );
    direct.delete(&[1]).unwrap();
    direct.insert(&[1], &[102], &reinsert).unwrap();
    direct.set(0, &[0], Values::U32(&[50])).unwrap();
    direct.add(1, &[0, 2], Values::I64(&[2, -3])).unwrap();
    direct
        .set(
            2,
            &[2],
            Values::U64List {
                ends: &[1],
                items: &[u64::MAX],
            },
        )
        .unwrap();
    let mut targets = Vec::new();
    direct
        .fill(3, &[2], Values::Bytes(&[44]), &mut targets)
        .unwrap();
    assert_eq!(targets, [0, 1]);
    assert_eq!(read, direct);
    assert_eq!(read.key(1), Some(102));
    assert_eq!(
        read.column(1).unwrap().cell(0),
        Some(crate::state::Cell::OptionalI64(Some(1)))
    );
}

/// A section inserting a slot far past the table, which the writer never assigns, is
/// refused before the table grows to it; slots run on from the table's last.
#[test]
fn an_insert_past_the_next_free_slot_is_refused_before_growing() {
    let inserts = [
        Values::U32(&[5]),
        Values::OptionalI64 {
            present: &[0],
            values: &[],
        },
        Values::U64List {
            ends: &[0],
            items: &[],
        },
        Values::Bytes(&[40]),
    ];
    let far = hour(&[], &[], &[u32::MAX], &[100], &inserts, &[]);
    let mut table = Table::new(&KINDS).unwrap();
    let mut changes = Changes::default();
    let bytes = write(&KINDS, &far).unwrap();
    assert!(
        apply(
            &bytes,
            &KINDS,
            false,
            &mut table,
            &mut Scratch::default(),
            &mut changes
        )
        .is_err()
    );
    assert_eq!(table.slots(), 0);
    let next = hour(&[], &[], &[0], &[100], &inserts, &[]);
    let bytes = write(&KINDS, &next).unwrap();
    apply(
        &bytes,
        &KINDS,
        false,
        &mut table,
        &mut Scratch::default(),
        &mut changes,
    )
    .unwrap();
    assert_eq!(table.slots(), 1);
}

/// FILL targets are reported, and a partly read table may fill only slots it read.
#[test]
fn fills_report_their_targets_and_stay_within_a_partial_read() {
    let inserts = [
        Values::U32(&[5, 6]),
        Values::OptionalI64 {
            present: &[0],
            values: &[],
        },
        Values::U64List {
            ends: &[0, 0],
            items: &[],
        },
        Values::Bytes(&[40, 41]),
    ];
    let first = hour(&[], &[], &[0, 1], &[100, 101], &inserts, &[]);
    let mut table = Table::new(&KINDS).unwrap();
    applied(&mut table, &write(&KINDS, &first).unwrap());
    let fill = [Run {
        column: 0,
        op: Op::Fill,
        positions: &[],
        values: Some(Values::U32(&[9])),
        except: &[],
        patches: None,
    }];
    let complete = hour(&[], &[], &[], &[], &[], &fill);
    let mut whole = table.clone();
    let changes = applied(&mut whole, &write(&KINDS, &complete).unwrap());
    assert_eq!(changes.fill_slots, [0, 1]);
    // Read in part: slot 0 only, yet the FILL names no exception, so it would set slot 1.
    let partial = TableHour {
        read: Some(&[0]),
        ..hour(&[], &[], &[], &[], &[], &fill)
    };
    let mut changes = Changes::default();
    assert!(
        apply(
            &write(&KINDS, &partial).unwrap(),
            &KINDS,
            true,
            &mut table.clone(),
            &mut Scratch::default(),
            &mut changes,
        )
        .is_err()
    );
}

/// A cell the recording did not record is unknown, not the value its column holds there,
/// until a value is written to it; only changed or inserted slots become unrecorded.
#[test]
fn unrecorded_cells_stay_unknown_until_written() {
    let inserts = [
        Values::U32(&[5, 6]),
        Values::OptionalI64 {
            present: &[0],
            values: &[],
        },
        Values::U64List {
            ends: &[0, 0],
            items: &[],
        },
        Values::Bytes(&[40, 41]),
    ];
    let unrecorded: &[(usize, &[u32])] = &[(0, &[1])];
    let first = TableHour {
        unrecorded,
        ..hour(&[], &[], &[0, 1], &[100, 101], &inserts, &[])
    };
    let mut table = Table::new(&KINDS).unwrap();
    applied(&mut table, &write(&KINDS, &first).unwrap());
    assert!(table.recorded(0, 0) && !table.recorded(0, 1) && table.recorded(1, 1));
    let mut direct = Table::new(&KINDS).unwrap();
    direct.insert(&[0, 1], &[100, 101], &inserts).unwrap();
    assert_ne!(
        table, direct,
        "an unrecorded cell differs from a recorded one"
    );
    direct.unrecord(0, &[1]).unwrap();
    assert_eq!(table, direct);
    let set = [run(0, Op::Set, &[0], Some(Values::U32(&[7])))];
    let later = hour(&[1], &[], &[], &[], &[], &set);
    applied(&mut table, &write(&KINDS, &later).unwrap());
    assert!(table.recorded(0, 1));
    let unchanged: &[(usize, &[u32])] = &[(0, &[0])];
    let stray = TableHour {
        unrecorded: unchanged,
        ..hour(&[1], &[], &[], &[], &[], &set)
    };
    assert!(write(&KINDS, &stray).is_err(), "slot 0 did not change");
}

#[test]
fn sections_that_disagree_with_the_schema_or_end_early_are_refused() {
    let wrong_kind = [run(0, Op::Set, &[0], Some(Values::I32(&[1])))];
    assert!(write(&KINDS, &hour(&[3], &[], &[], &[], &[], &wrong_kind)).is_err());
    let too_few = [run(0, Op::Set, &[0, 1], Some(Values::U32(&[1])))];
    assert!(write(&KINDS, &hour(&[3, 4], &[], &[], &[], &[], &too_few)).is_err());
    let add_to_list = [run(2, Op::Add, &[0], Some(Values::U64(&[1])))];
    assert!(write(&KINDS, &hour(&[3], &[], &[], &[], &[], &add_to_list)).is_err());
    let noop_values = [run(3, Op::Noop, &[0], Some(Values::Bytes(&[1])))];
    assert!(write(&KINDS, &hour(&[3], &[], &[], &[], &[], &noop_values)).is_err());

    let mut table = Table::new(&KINDS).unwrap();
    let inserts = [
        Values::U32(&[1]),
        Values::OptionalI64 {
            present: &[1],
            values: &[2],
        },
        Values::U64List {
            ends: &[0],
            items: &[],
        },
        Values::Bytes(&[3]),
    ];
    let bytes = write(&KINDS, &hour(&[], &[], &[0], &[9], &inserts, &[])).unwrap();
    let mut changes = Changes::default();
    let mut scratch = Scratch::default();
    for cut in [1, bytes.len() / 2, bytes.len() - 1] {
        let mut fresh = Table::new(&KINDS).unwrap();
        assert!(
            apply(
                &bytes[..cut],
                &KINDS,
                false,
                &mut fresh,
                &mut scratch,
                &mut changes
            )
            .is_err()
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(
        apply(
            &trailing,
            &KINDS,
            false,
            &mut table,
            &mut scratch,
            &mut changes
        )
        .is_err()
    );
    // A SET to a slot this table never inserted.
    let stray = [run(0, Op::Set, &[0], Some(Values::U32(&[1])))];
    let bytes = write(&KINDS, &hour(&[5], &[], &[], &[], &[], &stray)).unwrap();
    let mut fresh = Table::new(&KINDS).unwrap();
    assert!(
        apply(
            &bytes,
            &KINDS,
            false,
            &mut fresh,
            &mut scratch,
            &mut changes
        )
        .is_err()
    );
}

#[test]
fn a_partly_read_table_changes_only_the_slots_it_read() {
    let inserts = [
        Values::U32(&[1, 2, 3]),
        Values::OptionalI64 {
            present: &[0],
            values: &[],
        },
        Values::U64List {
            ends: &[0, 0, 0],
            items: &[],
        },
        Values::Bytes(&[4, 5, 6]),
    ];
    let mut table = Table::new(&KINDS).unwrap();
    applied(
        &mut table,
        &write(
            &KINDS,
            &hour(&[], &[], &[0, 1, 2], &[7, 8, 9], &inserts, &[]),
        )
        .unwrap(),
    );
    let runs = [run(0, Op::Set, &[0], Some(Values::U32(&[20])))];
    let partial = TableHour {
        read: Some(&[0, 2]),
        ..hour(&[2], &[0], &[], &[], &[], &runs)
    };
    let bytes = write(&KINDS, &partial).unwrap();
    let mut changes = Changes::default();
    apply(
        &bytes,
        &KINDS,
        true,
        &mut table,
        &mut Scratch::default(),
        &mut changes,
    )
    .unwrap();
    assert!(changes.truncated);
    assert_eq!(changes.read, [0, 2]);
    assert_eq!(changes.deleted, [0]);
    assert_eq!(table.key(1), Some(8), "an unread row is kept, not deleted");
    assert_eq!(
        table.column(0).unwrap().cell(2),
        Some(crate::state::Cell::U32(20))
    );
    let unread = TableHour {
        read: Some(&[2]),
        ..hour(&[1], &[], &[], &[], &[], &runs)
    };
    assert!(
        write(&KINDS, &unread).is_err(),
        "a change to a slot not read"
    );
}

#[test]
fn patch_runs_edit_held_cells_and_refuse_edits_past_them() {
    let inserts = [
        Values::U32(&[1, 2]),
        Values::OptionalI64 {
            present: &[0],
            values: &[],
        },
        Values::U64List {
            ends: &[5, 7],
            items: &[10, 11, 12, 13, 14, 20, 21],
        },
        Values::Bytes(&[0, 0]),
    ];
    let mut table = Table::new(&KINDS).unwrap();
    applied(
        &mut table,
        &write(&KINDS, &hour(&[], &[], &[0, 1], &[5, 6], &inserts, &[])).unwrap(),
    );
    // Slot 0: keep 1, remove 2, insert [99]; keep 1, remove 0, insert [7, 8]. Slot 1: no hunks.
    let patches = Patches {
        hunks: &[2, 0],
        edits: &[(1, 2, 1), (1, 0, 2)],
        items: PatchItems::U64(&[99, 7, 8]),
    };
    let runs = [Run {
        column: 2,
        op: Op::Patch,
        positions: &[0, 1],
        values: None,
        except: &[],
        patches: Some(patches),
    }];
    let bytes = write(&KINDS, &hour(&[0, 1], &[], &[], &[], &[], &runs)).unwrap();
    applied(&mut table, &bytes);
    assert_eq!(
        table.column(2).unwrap().cell(0),
        Some(crate::state::Cell::U64List(&[10, 99, 13, 7, 8, 14]))
    );
    assert_eq!(
        table.column(2).unwrap().cell(1),
        Some(crate::state::Cell::U64List(&[20, 21]))
    );

    let past = Patches {
        hunks: &[1],
        edits: &[(1, 5, 0)],
        items: PatchItems::U64(&[]),
    };
    let runs = [Run {
        patches: Some(past),
        positions: &[0],
        ..runs[0]
    }];
    let bytes = write(&KINDS, &hour(&[1], &[], &[], &[], &[], &runs)).unwrap();
    let mut changes = Changes::default();
    assert!(
        apply(
            &bytes,
            &KINDS,
            false,
            &mut table,
            &mut Scratch::default(),
            &mut changes
        )
        .is_err(),
        "removing past the held cell"
    );
    let wrong = Patches {
        items: PatchItems::Bytes(&[]),
        ..past
    };
    let runs = [Run {
        patches: Some(wrong),
        ..runs[0]
    }];
    assert!(
        write(&KINDS, &hour(&[1], &[], &[], &[], &[], &runs)).is_err(),
        "bytes for a list column"
    );
}

/// Each FILL marks the slots it set with its own column, whatever the other FILLs except.
#[test]
fn fills_with_different_exceptions_mark_each_slot_with_its_own_columns() {
    let inserts = [
        Values::U32(&[5, 6, 7]),
        Values::OptionalI64 {
            present: &[0],
            values: &[],
        },
        Values::U64List {
            ends: &[0, 0, 0],
            items: &[],
        },
        Values::Bytes(&[40, 41, 42]),
    ];
    let first = hour(&[], &[], &[0, 1, 2], &[100, 101, 102], &inserts, &[]);
    let mut table = Table::new(&KINDS).unwrap();
    applied(&mut table, &write(&KINDS, &first).unwrap());
    let fills = [
        Run {
            column: 0,
            op: Op::Fill,
            positions: &[],
            values: Some(Values::U32(&[9])),
            except: &[1],
            patches: None,
        },
        Run {
            column: 3,
            op: Op::Fill,
            positions: &[],
            values: Some(Values::Bytes(&[8])),
            except: &[0],
            patches: None,
        },
    ];
    let filled = hour(&[], &[], &[], &[], &[], &fills);
    let changes = applied(&mut table, &write(&KINDS, &filled).unwrap());
    assert_eq!(changes.fill_slots, [0, 1, 2]);
    assert_eq!(changes.fill_masks, [0b1, 0b1000, 0b1001]);
}
