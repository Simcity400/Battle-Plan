use super::*;
use crate::{
    manifest::Section,
    section::{Op, Run, TableHour, write},
    state::{Cell, Values},
};

fn schema() -> Vec<Vec<Kind>> {
    vec![vec![Kind::U32, Kind::Bytes], vec![Kind::I64]]
}

fn quiet(hour: i32) -> Manifest<'static> {
    Manifest {
        branch: 1,
        hour,
        flags: 0,
        total_ns: 0,
        pool_bytes: 0,
        dropped_frames: 0,
        appends: vec![],
        statuses: vec![Status::Complete, Status::Complete],
        read: vec![0b11],
        samples: vec![],
        sections: vec![],
    }
}

fn section_entry(table: usize, bytes: &[u8], columns: u64) -> Section {
    Section {
        table,
        bytes: bytes.len() as u64,
        columns: vec![columns],
        ops: 0,
    }
}

#[test]
fn hours_apply_their_sections_and_appends_and_clear_the_last_hours_changes() {
    let mut state = State::new(schema()).unwrap();
    let first = write(
        &schema()[0],
        &TableHour {
            read: None,
            changed: &[],
            deleted: &[],
            inserted: &[0, 1],
            keys: &[10, 11],
            inserts: &[Values::U32(&[1, 2]), Values::Bytes(&[0, 1])],
            runs: &[],
            touched: &[],
            unrecorded: &[],
        },
    )
    .unwrap();
    let other = write(
        &schema()[1],
        &TableHour {
            read: None,
            changed: &[],
            deleted: &[],
            inserted: &[0],
            keys: &[50],
            inserts: &[Values::I64(&[-7])],
            runs: &[],
            touched: &[],
            unrecorded: &[],
        },
    )
    .unwrap();
    let hour = Manifest {
        appends: vec![&b"GER"[..], &b"ITA"[..]],
        sections: vec![section_entry(0, &first, 0b11), section_entry(1, &other, 1)],
        ..quiet(100)
    };
    let sections = [first.as_slice(), other.as_slice()];
    state.apply(&hour, |index| Ok(sections[index])).unwrap();
    assert_eq!(state.dictionary().get(1), Some(&b"ITA"[..]));
    let table = state.table(0).unwrap();
    assert_eq!(table.slot(11), Some(1));
    assert_eq!(table.column(1).unwrap().cell(1), Some(Cell::Bytes(1)));
    assert_eq!(state.changes(0).unwrap().inserted, [0, 1]);
    assert_eq!(state.changes(1).unwrap().inserted, [0]);

    let set = write(
        &schema()[1],
        &TableHour {
            read: None,
            changed: &[0],
            deleted: &[],
            inserted: &[],
            keys: &[],
            inserts: &[],
            runs: &[Run {
                column: 0,
                op: Op::Add,
                positions: &[0],
                values: Some(Values::I64(&[10])),
                except: &[],
                patches: None,
            }],
            touched: &[],
            unrecorded: &[],
        },
    )
    .unwrap();
    let next = Manifest {
        sections: vec![section_entry(1, &set, 1)],
        ..quiet(101)
    };
    state.apply(&next, |_| Ok(set.as_slice())).unwrap();
    assert!(
        state.changes(0).unwrap().inserted.is_empty(),
        "last hour's changes cleared"
    );
    assert_eq!(state.changes(1).unwrap().changed, [0]);
    assert_eq!(
        state.table(1).unwrap().column(0).unwrap().cell(0),
        Some(Cell::I64(3))
    );

    let short = Manifest {
        sections: vec![Section {
            bytes: set.len() as u64 + 1,
            ..section_entry(1, &set, 1)
        }],
        ..quiet(102)
    };
    assert!(
        state.apply(&short, |_| Ok(set.as_slice())).is_err(),
        "length mismatch"
    );
}


#[test]
fn a_recording_of_fewer_columns_leaves_the_later_columns_unrecorded() {
    let older = [vec![Kind::U32], vec![Kind::I64]];
    let inserted = write(
        &older[0],
        &TableHour {
            read: None,
            changed: &[],
            deleted: &[],
            inserted: &[0, 1],
            keys: &[30, 31],
            inserts: &[Values::U32(&[9, 8])],
            runs: &[],
            touched: &[],
            unrecorded: &[],
        },
    )
    .unwrap();
    let hour = Manifest {
        sections: vec![section_entry(0, &inserted, 0b1)],
        ..quiet(5)
    };
    let mut wide = State::recorded(schema(), vec![1, 1]).unwrap();
    wide.apply(&hour, |_| Ok(inserted.as_slice())).unwrap();
    let table = wide.table(0).unwrap();
    assert_eq!(table.kinds().collect::<Vec<_>>(), schema()[0]);
    assert_eq!(table.column(0).unwrap().cell(1), Some(Cell::U32(8)));
    assert!(table.recorded(0, 1));
    assert!(!table.recorded(1, 0) && !table.recorded(1, 1));

    assert!(State::recorded(schema(), vec![3, 1]).is_err());
}

#[test]
fn sample_hours_follow_reads_and_listed_samples() {
    let mut state = State::new(schema()).unwrap();
    state.apply(&quiet(10), |_| unreachable!()).unwrap();
    assert_eq!(state.sampled_at(0), Some(10));
    let unread = Manifest {
        read: vec![0b10],
        ..quiet(11)
    };
    state.apply(&unread, |_| unreachable!()).unwrap();
    assert_eq!(
        (state.sampled_at(0), state.sampled_at(1)),
        (Some(10), Some(11))
    );
    let reset = Manifest {
        read: vec![0],
        flags: crate::manifest::SAMPLES,
        samples: vec![(0, None), (1, Some(4))],
        ..quiet(12)
    };
    state.apply(&reset, |_| unreachable!()).unwrap();
    assert_eq!((state.sampled_at(0), state.sampled_at(1)), (None, Some(4)));
}

#[test]
fn a_truncated_view_is_the_hours_rows_read() {
    let mut walked = State::new(schema()).unwrap();
    let first = write(
        &schema()[0],
        &TableHour {
            read: None,
            changed: &[],
            deleted: &[],
            inserted: &[0, 1],
            keys: &[10, 11],
            inserts: &[Values::U32(&[1, 2]), Values::Bytes(&[0, 0])],
            runs: &[],
            touched: &[],
            unrecorded: &[],
        },
    )
    .unwrap();
    let hour = Manifest {
        appends: vec![&b""[..]],
        sections: vec![section_entry(0, &first, 0b11)],
        ..quiet(1)
    };
    walked.apply(&hour, |_| Ok(first.as_slice())).unwrap();
    assert_eq!(walked.truncated_view(0), None);
    let partial = write(
        &schema()[0],
        &TableHour {
            read: Some(&[1]),
            changed: &[],
            deleted: &[],
            inserted: &[2],
            keys: &[12],
            inserts: &[Values::U32(&[3]), Values::Bytes(&[0])],
            runs: &[],
            touched: &[],
            unrecorded: &[],
        },
    )
    .unwrap();
    let truncated = Manifest {
        statuses: vec![Status::Truncated { missing: 1 }, Status::Complete],
        sections: vec![section_entry(0, &partial, 0b11)],
        ..quiet(2)
    };
    walked
        .apply(&truncated, |_| Ok(partial.as_slice()))
        .unwrap();
    assert_eq!(walked.truncated_view(0), Some(&[1, 2][..]));
    let silent = Manifest {
        statuses: vec![Status::Truncated { missing: 1 }, Status::Complete],
        ..quiet(3)
    };
    assert!(walked.apply(&silent, |_| unreachable!()).is_err());
}

#[test]
fn fill_targets_clear_with_the_next_hour() {
    let mut state = State::new(schema()).unwrap();
    let inserted = write(
        &schema()[1],
        &TableHour {
            read: None,
            changed: &[],
            deleted: &[],
            inserted: &[0, 1],
            keys: &[50, 51],
            inserts: &[Values::I64(&[1, 2])],
            runs: &[],
            unrecorded: &[],
            touched: &[],
        },
    )
    .unwrap();
    let first = Manifest {
        sections: vec![section_entry(1, &inserted, 1)],
        ..quiet(1)
    };
    state.apply(&first, |_| Ok(inserted.as_slice())).unwrap();
    let fill = write(
        &schema()[1],
        &TableHour {
            read: None,
            changed: &[],
            deleted: &[],
            inserted: &[],
            keys: &[],
            inserts: &[],
            runs: &[Run {
                column: 0,
                op: Op::Fill,
                positions: &[],
                values: Some(Values::I64(&[9])),
                except: &[],
                patches: None,
            }],
            unrecorded: &[],
            touched: &[],
        },
    )
    .unwrap();
    let filled = Manifest {
        sections: vec![section_entry(1, &fill, 1)],
        ..quiet(2)
    };
    state.apply(&filled, |_| Ok(fill.as_slice())).unwrap();
    assert_eq!(state.changes(1).unwrap().fill_slots, [0, 1]);
    for hour in [3, 4] {
        state.apply(&quiet(hour), |_| unreachable!()).unwrap();
        let changes = state.changes(1).unwrap();
        assert!(changes.fill_slots.is_empty() && changes.filled.is_empty());
    }
}

#[test]
fn a_truncated_view_drops_deleted_rows_and_holds_a_reused_slot_once() {
    let mut walked = State::new(schema()).unwrap();
    let first = write(
        &schema()[0],
        &TableHour {
            read: None,
            changed: &[],
            deleted: &[],
            inserted: &[0, 1],
            keys: &[10, 11],
            inserts: &[Values::U32(&[1, 2]), Values::Bytes(&[0, 0])],
            runs: &[],
            unrecorded: &[],
            touched: &[],
        },
    )
    .unwrap();
    let hour = Manifest {
        appends: vec![&b""[..]],
        sections: vec![section_entry(0, &first, 0b11)],
        ..quiet(1)
    };
    walked.apply(&hour, |_| Ok(first.as_slice())).unwrap();
    for (read, deleted, inserted, keys, view) in [
        (&[0, 1][..], &[0][..], &[][..], &[][..], &[1][..]),
        (&[1][..], &[1][..], &[1][..], &[12][..], &[1][..]),
    ] {
        let partial = write(
            &schema()[0],
            &TableHour {
                read: Some(read),
                changed: &[],
                deleted,
                inserted,
                keys,
                inserts: if inserted.is_empty() {
                    &[]
                } else {
                    &[Values::U32(&[3]), Values::Bytes(&[0])]
                },
                runs: &[],
                unrecorded: &[],
                touched: &[],
            },
        )
        .unwrap();
        let truncated = Manifest {
            statuses: vec![Status::Truncated { missing: 1 }, Status::Complete],
            sections: vec![section_entry(0, &partial, 0b11)],
            ..quiet(2)
        };
        walked
            .apply(&truncated, |_| Ok(partial.as_slice()))
            .unwrap();
        assert_eq!(walked.truncated_view(0), Some(view));
    }
}
