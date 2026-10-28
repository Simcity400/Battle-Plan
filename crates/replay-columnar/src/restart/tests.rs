use super::*;
use crate::state::Values;

const KINDS: [Kind; 5] = [
    Kind::Bool,
    Kind::OptionalU32,
    Kind::U32List,
    Kind::Opaque,
    Kind::I16,
];

fn table() -> Table {
    let mut table = Table::new(&KINDS).unwrap();
    table
        .insert(
            &[0, 1, 2, 65],
            &[7, 8, u128::MAX, 1 << 100],
            &[
                Values::Bool(&[1, 0, 1, 1]),
                Values::OptionalU32 {
                    present: &[0b1001],
                    values: &[3, 4],
                },
                Values::U32List {
                    ends: &[0, 2, 3, 3],
                    items: &[5, 6, 7],
                },
                Values::Opaque {
                    ends: &[1, 1, 1, 4],
                    items: b"abcd",
                },
                Values::I16(&[-1, 2, i16::MIN, 0]),
            ],
        )
        .unwrap();
    // A freed slot keeps stale values; a restart holds only the live rows.
    table.delete(&[1]).unwrap();
    table
}

#[test]
fn a_restart_reads_back_the_same_rows_and_acquisition() {
    let written = table();
    let acquisition = Acquisition {
        last_read: Some(-60_759_371),
        available_from: Some(60_759_000),
        refusal: Some(0),
    };
    let status = Status::Truncated { missing: 2 };
    let bytes = write(&written, status, &acquisition, &[0, 65]).unwrap();
    let restored = read(&bytes, &KINDS, &mut Scratch::default()).unwrap();
    assert_eq!(restored.table, written);
    assert_eq!(restored.status, status);
    assert_eq!(restored.acquisition, acquisition);
    assert_eq!(restored.view, [0, 65]);
    assert_eq!(restored.table.slots(), 66);
    assert_eq!(restored.table.key(65), Some(1 << 100));
    assert_eq!(restored.table.key(1), None);

    let empty = Table::new(&KINDS).unwrap();
    let bytes = write(&empty, Status::NotRecorded, &Acquisition::default(), &[]).unwrap();
    let restored = read(&bytes, &KINDS, &mut Scratch::default()).unwrap();
    assert_eq!(restored.table, empty);
    assert_eq!(restored.status, Status::NotRecorded);
    assert_eq!(restored.acquisition, Acquisition::default());
    assert!(restored.view.is_empty());
}

/// A restart keeps which cells the recording did not record.
#[test]
fn a_restart_keeps_unrecorded_cells() {
    let mut held = table();
    held.unrecord(1, &[2, 65]).unwrap();
    let bytes = write(&held, Status::Complete, &Acquisition::default(), &[]).unwrap();
    let restored = read(&bytes, &KINDS, &mut Scratch::default()).unwrap();
    assert_eq!(restored.table, held);
    assert!(!restored.table.recorded(1, 2) && restored.table.recorded(1, 0));
}

/// Two truncated tables holding the same rows but having read different ones restart to
/// their own views; a view is a truncated table's alone and names live rows only.
#[test]
fn a_truncated_restart_keeps_which_rows_its_view_read() {
    let held = table();
    let status = Status::Truncated { missing: 1 };
    let read_a = write(&held, status, &Acquisition::default(), &[0]).unwrap();
    let read_b = write(&held, status, &Acquisition::default(), &[2]).unwrap();
    assert_ne!(read_a, read_b);
    let mut scratch = Scratch::default();
    assert_eq!(read(&read_a, &KINDS, &mut scratch).unwrap().view, [0]);
    assert_eq!(read(&read_b, &KINDS, &mut scratch).unwrap().view, [2]);
    assert!(write(&held, Status::Complete, &Acquisition::default(), &[0]).is_err());
    assert!(
        write(&held, status, &Acquisition::default(), &[1]).is_err(),
        "a freed slot"
    );
}

#[test]
fn damaged_restarts_are_refused() {
    let bytes = write(&table(), Status::Complete, &Acquisition::default(), &[]).unwrap();
    let mut scratch = Scratch::default();
    for cut in [0, 5, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            read(&bytes[..cut], &KINDS, &mut scratch).is_err(),
            "cut at {cut}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(read(&trailing, &KINDS, &mut scratch).is_err());
    assert!(
        read(&bytes, &KINDS[..4], &mut scratch).is_err(),
        "a schema with a column fewer"
    );
}
