use super::*;

fn index() -> Index {
    let unit = |stream, first, hours, offset| Unit {
        stream,
        first,
        hours,
        offset,
        raw: 900,
        stored: 300,
    };
    Index {
        hours: 1_500,
        unit_hours: 720,
        units: vec![
            unit(0, 0, 720, 12),
            unit(5, 0, 720, 312),
            unit(0, 720, 720, 612),
            unit(0, 1_440, 60, 912),
        ],
        restarts: vec![Restart {
            after: 719,
            hour: -60_759_371,
            chunk: Record {
                offset: 1_212,
                raw: 40,
                stored: 30,
            },
            state: Record {
                offset: 1_242,
                raw: 4_000,
                stored: 900,
            },
        }],
    }
}

#[test]
fn an_index_round_trips() {
    let written = index();
    assert_eq!(Index::read(&written.write().unwrap()).unwrap(), written);
}

#[test]
fn a_damaged_index_is_refused() {
    let mut bytes = index().write().unwrap();
    bytes[3] ^= 1;
    assert!(Index::read(&bytes).is_err());
}

#[test]
fn units_off_the_grid_and_restarts_inside_a_unit_are_refused() {
    let mut off_grid = index();
    off_grid.units[1].first = 10;
    assert!(off_grid.write().is_err());
    let mut inside = index();
    inside.restarts[0].after = 700;
    assert!(inside.write().is_err());
}

#[test]
fn a_unit_crossing_a_restart_or_out_of_order_is_refused_on_read() {
    // Written past the writer's checks, a unit of two grid lengths beside a restart inside it.
    let mut long = index();
    long.units = vec![Unit {
        stream: 0,
        first: 0,
        hours: 1_440,
        offset: 12,
        raw: 900,
        stored: 300,
    }];
    let mut writer = Writer::new();
    writer.varint(u64::from(long.hours)).unwrap();
    writer.varint(u64::from(long.unit_hours)).unwrap();
    writer.varint(1).unwrap();
    long.units[0].write(&mut writer).unwrap();
    writer.varint(0).unwrap();
    let mut bytes = writer.into_bytes();
    let sum = checksum(&bytes);
    bytes.extend_from_slice(&sum.to_le_bytes());
    assert!(Index::read(&bytes).is_err());
    assert!(long.write().is_err());
    let mut reordered = index();
    reordered.units.swap(0, 2);
    assert!(reordered.write().is_err());
    let mut restarts = index();
    restarts.restarts.push(restarts.restarts[0]);
    assert!(restarts.write().is_err());
}

#[test]
fn a_unit_at_the_end_of_the_hours_finds_no_overflow() {
    let index = Index {
        hours: u32::MAX,
        unit_hours: 1 << 31,
        // Past the hours, as an unchecked index could hold: its end passes 32 bits.
        units: vec![Unit {
            stream: 0,
            first: 1 << 31,
            hours: 1 << 31,
            offset: 0,
            raw: 1,
            stored: 1,
        }],
        restarts: vec![],
    };
    assert_eq!(index.stream_from(0, u32::MAX).count(), 1);
}

#[test]
fn a_stream_is_found_from_any_hour() {
    let index = index();
    let firsts: Vec<u32> = index.stream_from(0, 800).map(|unit| unit.first).collect();
    assert_eq!(firsts, [720, 1_440]);
}
