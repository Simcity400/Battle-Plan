use super::*;

const COLUMNS: [usize; 3] = [4, 70, 1];

fn manifest() -> Manifest<'static> {
    Manifest {
        branch: 2,
        hour: -60_759_371,
        flags: AFTER_GAP | AFTER_LOAD,
        total_ns: 1_234_567,
        pool_bytes: 1 << 40,
        dropped_frames: 0,
        appends: vec![&b"GER"[..], &b""[..], "Deutsches Reich".as_bytes()],
        statuses: vec![
            Status::Complete,
            Status::Truncated { missing: 3 },
            Status::Unavailable { reason: 7 },
        ],
        read: vec![0b011],
        samples: vec![],
        sections: vec![
            Section {
                table: 0,
                bytes: 40,
                columns: vec![0b1001],
                ops: INSERTS | FILLS,
            },
            Section {
                table: 1,
                bytes: 300,
                columns: vec![1, 1 << 5],
                ops: 0,
            },
        ],
    }
}

#[test]
fn manifests_round_trip_with_every_status_and_wide_tables() {
    let written = manifest();
    let bytes = written.write(&COLUMNS).unwrap();
    assert_eq!(Manifest::read(&bytes, &COLUMNS).unwrap(), written);
    let quiet = Manifest {
        appends: vec![],
        statuses: vec![Status::NotRecorded; 3],
        read: vec![0],
        sections: vec![],
        ..written
    };
    let bytes = quiet.write(&COLUMNS).unwrap();
    assert_eq!(Manifest::read(&bytes, &COLUMNS).unwrap(), quiet);
}

#[test]
fn samples_round_trip_and_disagreeing_ones_are_refused() {
    let sampled = Manifest {
        flags: SAMPLES,
        samples: vec![(0, None), (2, Some(-60_759_000))],
        ..manifest()
    };
    let bytes = sampled.write(&COLUMNS).unwrap();
    assert_eq!(Manifest::read(&bytes, &COLUMNS).unwrap(), sampled);
    let unflagged = Manifest {
        flags: 0,
        ..sampled.clone()
    };
    assert!(unflagged.write(&COLUMNS).is_err());
    let flagged = Manifest {
        samples: vec![],
        ..sampled.clone()
    };
    assert!(flagged.write(&COLUMNS).is_err());
    let unordered = Manifest {
        samples: vec![(2, None), (0, None)],
        ..sampled.clone()
    };
    assert!(unordered.write(&COLUMNS).is_err());
    let past = Manifest {
        samples: vec![(3, None)],
        ..sampled
    };
    assert!(past.write(&COLUMNS).is_err());
}

#[test]
fn unknown_flags_and_columns_past_a_table_are_refused() {
    let mut flags = manifest();
    flags.flags |= 0x80;
    assert!(flags.write(&COLUMNS).is_err());
    let mut ops = manifest();
    ops.sections[0].ops |= 0x80;
    assert!(ops.write(&COLUMNS).is_err());
    let mut column = manifest();
    column.sections[0].columns = vec![1 << 4];
    assert!(column.write(&COLUMNS).is_err(), "table 0 has 4 columns");
    let bytes = manifest().write(&COLUMNS).unwrap();
    let mut patched = bytes;
    let flags_at = 1 + 4;
    assert_eq!(patched[flags_at], AFTER_GAP | AFTER_LOAD);
    patched[flags_at] |= 0x80;
    assert!(Manifest::read(&patched, &COLUMNS).is_err());
}

#[test]
fn manifests_that_disagree_with_the_schema_are_refused() {
    let mut out_of_order = manifest();
    out_of_order.sections.reverse();
    assert!(out_of_order.write(&COLUMNS).is_err());
    let mut narrow = manifest();
    narrow.sections[1].columns = vec![1];
    assert!(narrow.write(&COLUMNS).is_err(), "70 columns need two words");
    let mut missing_status = manifest();
    missing_status.statuses.pop();
    assert!(missing_status.write(&COLUMNS).is_err());

    let bytes = manifest().write(&COLUMNS).unwrap();
    assert!(
        Manifest::read(&bytes, &COLUMNS[..2]).is_err(),
        "fewer tables"
    );
    assert!(Manifest::read(&bytes[..bytes.len() - 1], &COLUMNS).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(Manifest::read(&trailing, &COLUMNS).is_err());
    let (prefixed, read) = Manifest::read_prefix(&trailing, &COLUMNS).unwrap();
    assert_eq!((prefixed, read), (manifest(), bytes.len()));
    let mut stray_read = manifest();
    stray_read.read = vec![1 << 3];
    assert!(
        stray_read.write(&COLUMNS).is_err(),
        "a read bit past the tables"
    );
    let distinct = Manifest {
        read: vec![0b101],
        ..manifest()
    };
    let mut bytes = distinct.write(&COLUMNS).unwrap();
    let word = 0b101_u64.to_le_bytes();
    let at: Vec<usize> = (0..bytes.len() - 7)
        .filter(|&at| bytes[at..at + 8] == word)
        .collect();
    assert_eq!(at.len(), 1);
    bytes[at[0]] |= 1 << 3;
    assert!(
        Manifest::read(&bytes, &COLUMNS).is_err(),
        "a read bit past the tables"
    );
}
