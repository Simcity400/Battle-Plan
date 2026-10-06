use super::*;
use engine::memory::SliceMemory;
#[test]
fn unreadable_census_is_unavailable_and_raw_colors_preserve_primary_values() {
    let memory = SliceMemory {
        base: 0,
        bytes: &[],
    };
    let mut capture = Captured::with_capacities(&Capacities {
        rows: 1,
        subjects: 1,
    });
    capture.capture(&memory, 0, 0);
    assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    capture.reset();
    capture.rows.rows[0].country = 17;
    capture.rows.rows[0].rgba = [0x7fc01234, 0x80000000, 0xffffffff, 0x3f800000];
    capture.rows.rows[0].ui_rgba = [0x3f000000, 0x00000001, 0x7f800000, 0xbf800000];
    capture.rows.rows[0].cosmetic_tag.bytes[..3].copy_from_slice(&[65, 0, 255]);
    capture.rows.rows[0].cosmetic_tag.len = 3;
    capture.rows.rows[0].leader_character = Some(0);
    capture.rows.outcome = Outcome::Rows { len: 1 };
    let [TableView::Complete(replay::rows::TableRows::CountryAppearance(rows))] = capture.tables()
    else {
        panic!("complete");
    };
    assert_eq!(rows[0].red, 0x7fc01234);
    assert_eq!(rows[0].cosmetic_tag, [65, 0, 255]);
    assert_eq!(rows[0].green, 0x80000000);
    assert_eq!(rows[0].blue, 0xffffffff);
    assert_eq!(rows[0].alpha, 0x3f800000);
    assert_eq!(rows[0].leader_character, Some(0));
    assert_eq!(
        [
            rows[0].ui_red,
            rows[0].ui_green,
            rows[0].ui_blue,
            rows[0].ui_alpha
        ],
        [0x3f000000, 0x00000001, 0x7f800000, 0xbf800000]
    );
    assert_eq!(rows[0].old_name, None);
}

#[test]
fn subject_payload_demand_is_positive_count_only_and_keeps_raw_order() {
    use engine::{
        cpdx_array::Array,
        memory::{CountedMemory, ReadCounts},
    };
    let bytes = [9_i32, -1, 9]
        .into_iter()
        .flat_map(i32::to_le_bytes)
        .collect::<Vec<_>>();
    let memory = CountedMemory::new(SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    });
    let mut capture = Captured::with_capacities(&Capacities {
        rows: 2,
        subjects: 3,
    });
    capture.rows.rows[0].country = 3;
    capture.rows.rows[0].subjects = Array {
        data: 0x1000,
        count: 3,
    };
    capture.rows.rows[1].country = 7;
    capture.rows.outcome = Outcome::Rows { len: 2 };
    ReadCounts::take_current();
    capture.capture_subjects(&memory);
    assert_eq!(ReadCounts::take_current().bytes, 12);
    let [TableView::Complete(replay::rows::TableRows::CountryAppearance(rows))] = capture.tables()
    else {
        panic!("complete subject roster");
    };
    assert_eq!(*rows[0].subjects.require("subjects").unwrap(), [9, -1, 9]);
    assert!(rows[1].subjects.require("subjects").unwrap().is_empty());
}

#[test]
fn subject_shortage_and_payload_fault_omit_only_affected_countries() {
    use engine::cpdx_array::Array;
    let bytes = [9_i32, -1, 9, 17]
        .into_iter()
        .flat_map(i32::to_le_bytes)
        .collect::<Vec<_>>();
    for payload_fault in [false, true] {
        let mut capacity = Capacities {
            rows: 3,
            subjects: if payload_fault { 4 } else { 1 },
        };
        let mut capture = Captured::with_capacities(&capacity);
        capture.rows.rows[0].country = 3;
        capture.rows.rows[0].subjects = Array {
            data: if payload_fault { 0x2000 } else { 0x1000 },
            count: 3,
        };
        capture.rows.rows[1].country = 7;
        capture.rows.rows[2].country = 11;
        capture.rows.rows[2].subjects = Array {
            data: 0x100c,
            count: 1,
        };
        // Include a row omitted by the country walker before subject capture.
        capture.rows.outcome = Outcome::Truncated {
            len: 3,
            unread_rows: 1,
        };
        capture.capture_subjects(&SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        });
        assert_eq!(
            capture.rows.outcome,
            Outcome::Truncated {
                len: 2,
                unread_rows: 2
            }
        );
        assert_eq!(capture.subjects.required, 4);
        let [
            TableView::Truncated {
                rows: replay::rows::TableRows::CountryAppearance(rows),
                missing: 2,
            },
        ] = capture.tables()
        else {
            panic!("unaffected countries must remain partial rows");
        };
        assert_eq!(
            rows.iter().map(|row| row.country).collect::<Vec<_>>(),
            [7, 11]
        );
        assert!(rows[0].subjects.require("subjects").unwrap().is_empty());
        assert_eq!(*rows[1].subjects.require("subjects").unwrap(), [17]);
        capture.grow(&mut capacity);
        assert!(capture.subjects.first.len() >= 4);
        assert_eq!(capture.subject_starts.len(), capture.rows.rows.len());
    }
}

#[test]
fn subject_arena_presizes_from_live_country_slots_without_a_cap() {
    use engine::cpdx_array::Array;
    let slots = 5_001;
    let mut capacity = Capacities::default();
    capacity.raise_for_countries(slots);
    let mut capture = Captured::with_capacities(&capacity);
    let bytes = 17_u32.to_le_bytes();
    for (index, row) in capture.rows.rows.iter_mut().take(slots - 1).enumerate() {
        row.country = index as u32 + 1;
        row.subjects = Array {
            data: 0x1000,
            count: 1,
        };
    }
    capture.rows.outcome = Outcome::Rows { len: slots - 1 };
    capture.capture_subjects(&SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    });
    assert_eq!(capture.rows.outcome, Outcome::Rows { len: 5_000 });
    assert_eq!(capture.subjects.len, 5_000);
    assert!(capacity.subjects >= crate::country_scale::rows(slots, 1));
}
