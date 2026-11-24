use super::*;
use engine_native_memory::memory::SliceMemory;

#[test]
fn reserved_certificate_readiness_uses_only_the_projectable_naming_prefix() {
    use engine_native::crelation::subject::Name;
    let mut capture = Captured::vacant();
    assert!(!capture.reserved_land_certificate_ready());
    capture.reserved.outcome = Outcome::Rows { len: 1 };
    assert!(capture.reserved_land_certificate_ready());
    capture.reserved.row.original_tag = Some(Name { start: 1, len: 1 });
    capture.reserved.row.faction = None;
    assert!(
        capture.reserved_land_certificate_ready(),
        "optional operands deny the prefix"
    );
    for field in 0..6 {
        let mut row = capture.reserved.row;
        let malformed = Name {
            start: usize::MAX,
            len: 1,
        };
        match field {
            0 => row.tag = malformed,
            1 => row.name_tag = malformed,
            2 => row.cosmetic_tag = malformed,
            3 => row.leader_ideology = malformed,
            4 => row.group = Some(malformed),
            _ => row.autonomy = Some(malformed),
        }
        let original = std::mem::replace(&mut capture.reserved.row, row);
        assert!(!capture.reserved_land_certificate_ready());
        assert!(matches!(
            capture.reserved_table(None),
            TableView::Unavailable { .. }
        ));
        capture.reserved.row = original;
    }
    capture.reserved.row.built = 8;
    assert!(!capture.reserved_land_certificate_ready());
    capture.reserved.row.built = 7;
    capture.naming.len = 1;
    assert!(!capture.reserved_land_certificate_ready());
    capture.naming.len = 0;
    capture.reserved.outcome = Outcome::Truncated {
        len: 0,
        unread_rows: 1,
    };
    assert!(!capture.reserved_land_certificate_ready());
}

#[test]
fn reserved_naming_keeps_exact_bytes_and_does_not_withhold_existing_appearance() {
    use engine_native::crelation::subject::Name;
    let mut capacity = Capacities {
        rows: 1,
        subjects: 0,
        naming_bytes: 4,
        ..Default::default()
    };
    let mut capture = Captured::with_capacities(&capacity);
    capture.rows.rows[0].country = 7;
    capture.rows.outcome = Outcome::Rows { len: 1 };
    capture.reserved.outcome = Outcome::Truncated {
        len: 0,
        unread_rows: 1,
    };
    capture.naming.required = 1025;
    let [
        TableView::Complete(replay_data::rows::TableRows::CountryAppearance(rows)),
        TableView::Truncated { missing: 1, .. },
        ..,
    ] = capture.tables()
    else {
        panic!("singleton shortage must preserve the country table");
    };
    assert_eq!(rows[0].country, 7);
    capture.grow(&mut capacity);
    assert!(capture.naming.first.len() >= 1025);
    capture.naming.first[..4].copy_from_slice(&[0xff, 0, b'X', b'Y']);
    capture.naming.len = 4;
    capture.reserved.row = ccountry::ReservedNamingRow {
        tag: Name { start: 0, len: 2 },
        cosmetic_tag: Name { start: 2, len: 2 },
        group: Some(Name { start: 4, len: 0 }),
        built: 5,
        original_tag: Some(Name { start: 0, len: 2 }),
        original_tag_reference: i32::MIN,
        ..Default::default()
    };
    capture.reserved.outcome = Outcome::Rows { len: 1 };
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::ReservedCountryNaming(rows)),
        ..,
    ] = capture.tables()
    else {
        panic!("singleton must publish after growth");
    };
    assert_eq!(rows[0].tag, [0xff, 0]);
    assert_eq!(rows[0].cosmetic_tag, b"XY");
    assert_eq!(rows[0].group, Some(Vec::new()));
    assert_eq!(rows[0].autonomy, None);
    assert_eq!(rows[0].built, 5);
    assert_eq!(
        rows[0].original_tag_reference,
        replay_data::rows::Recorded::Value(Some(i32::MIN))
    );
    assert_eq!(
        rows[0].original_tag,
        replay_data::rows::Recorded::Value(Some(vec![0xff, 0]))
    );
    capture.reserved.row.original_tag = Some(Name { start: 4, len: 0 });
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::ReservedCountryNaming(rows)),
        ..,
    ] = capture.tables()
    else {
        panic!("empty getter bytes are exact");
    };
    assert_eq!(
        rows[0].original_tag,
        replay_data::rows::Recorded::Value(Some(Vec::new()))
    );
    for original in [None, Some(Name { start: 4, len: 1 })] {
        capture.reserved.row.original_tag = original;
        let [
            _,
            TableView::Complete(replay_data::rows::TableRows::ReservedCountryNaming(rows)),
            ..,
        ] = capture.tables()
        else {
            panic!("optional getter refusal preserves naming operands");
        };
        assert_eq!(
            rows[0].original_tag,
            replay_data::rows::Recorded::Value(None)
        );
        assert_eq!(rows[0].tag, [0xff, 0]);
        assert_eq!(rows[0].group, Some(Vec::new()));
        assert_eq!(rows[0].built, 5);
    }
    for faction in [Some(0), Some((u64::from(u32::MAX) << 32) | 88), None] {
        capture.reserved.row.faction = faction;
        let [
            TableView::Complete(replay_data::rows::TableRows::CountryAppearance(positive)),
            TableView::Complete(replay_data::rows::TableRows::ReservedCountryNaming(rows)),
            ..,
        ] = capture.tables()
        else {
            panic!("independent faction refusal preserves both naming and positive appearance");
        };
        assert_eq!(positive[0].country, 7);
        assert_eq!(rows[0].faction, replay_data::rows::Recorded::Value(faction));
        assert_eq!(rows[0].tag, [0xff, 0]);
        assert_eq!(rows[0].built, 5);
    }
    capture.reserved.row.group = Some(Name { start: 4, len: 1 });
    assert!(matches!(
        capture.tables(),
        [TableView::Complete(_), TableView::Unavailable { .. }, ..]
    ));
}

#[test]
fn unreadable_census_is_unavailable_and_raw_colors_preserve_primary_values() {
    let memory = SliceMemory {
        base: 0,
        bytes: &[],
    };
    let mut capture = Captured::with_capacities(&Capacities {
        rows: 1,
        subjects: 1,
        naming_bytes: 0,
        ..Default::default()
    });
    capture.capture(&memory, 0, 0);
    assert!(matches!(capture.tables()[0], TableView::Unavailable { .. }));
    capture.reset();
    capture.rows.rows[0].country = 17;
    capture.rows.rows[0].rgba = [0x7fc01234, 0x80000000, 0xffffffff, 0x3f800000];
    capture.rows.rows[0].ui_rgba = [0x3f000000, 0x00000001, 0x7f800000, 0xbf800000];
    capture.rows.rows[0].cosmetic_tag.bytes.resize(3, 0);
    capture.rows.rows[0].cosmetic_tag.bytes[..3].copy_from_slice(&[65, 0, 255]);
    capture.rows.rows[0].cosmetic_tag.len = 3;
    capture.rows.rows[0].leader_character = Some(0);
    capture.rows.outcome = Outcome::Rows { len: 1 };
    let [
        TableView::Complete(replay_data::rows::TableRows::CountryAppearance(rows)),
        _,
        ..,
    ] = capture.tables()
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
    use engine_native_memory::{cpdx_array::Array, memory::{CountedMemory, ReadCounts}};
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
        naming_bytes: 0,
        ..Default::default()
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
    let [
        TableView::Complete(replay_data::rows::TableRows::CountryAppearance(rows)),
        _,
        ..,
    ] = capture.tables()
    else {
        panic!("complete subject roster");
    };
    assert_eq!(*rows[0].subjects.require("subjects").unwrap(), [9, -1, 9]);
    assert!(rows[1].subjects.require("subjects").unwrap().is_empty());
}

#[test]
fn positive_original_tag_projection_preserves_binary_absence_and_local_refusal() {
    use engine_native::crelation::subject::Name;
    let mut capture = Captured::with_capacities(&Capacities {
        rows: 1,
        subjects: 0,
        naming_bytes: 2,
        ..Default::default()
    });
    capture.rows.rows[0].country = 7;
    capture.rows.rows[0].original_tag_reference = i32::MAX;
    capture.rows.outcome = Outcome::Rows { len: 1 };
    capture.naming.first[..2].copy_from_slice(&[0xff, 0]);
    capture.naming.len = 2;
    for (span, expected) in [
        (Some(Name { start: 0, len: 2 }), Some(vec![0xff, 0])),
        (Some(Name { start: 2, len: 0 }), Some(Vec::new())),
        (None, None),
        (Some(Name { start: 2, len: 1 }), None),
    ] {
        capture.rows.rows[0].original_tag = span;
        let [
            TableView::Complete(replay_data::rows::TableRows::CountryAppearance(rows)),
            _,
            ..,
        ] = capture.tables()
        else {
            panic!("optional original-tag failures preserve the appearance prefix");
        };
        assert_eq!(rows[0].country, 7);
        assert_eq!(
            rows[0].original_tag,
            replay_data::rows::Recorded::Value(expected)
        );
        assert_eq!(rows[0].name_tag, Vec::<u8>::new());
        assert_eq!(
            rows[0].original_tag_reference,
            replay_data::rows::Recorded::Value(Some(i32::MAX))
        );
    }
}

#[test]
fn actual_original_tag_buckets_project_order_and_refuse_independently() {
    let mut capacities = Capacities {
        rows: 0,
        subjects: 0,
        naming_bytes: 4,
        original_tag_buckets: 3,
        original_tag_members: 4,
        country_array: 4,
        ..Default::default()
    };
    let mut capture = Captured::with_capacities(&capacities);
    capture.reserved.outcome = Outcome::Rows { len: 1 };
    capture.reserved.row.original_tag_reference = 7;
    capture.reserved.row.original_tag =
        Some(engine_native::crelation::subject::Name { start: 0, len: 0 });
    capture.naming.len = 0;
    for (index, (start, len)) in [(0, 0), (0, 4), (4, 0)].into_iter().enumerate() {
        capture.original_tag_buckets.first[index].start = start;
        capture.original_tag_buckets.first[index].len = len;
    }
    capture.original_tag_members.first[..4].copy_from_slice(&[9, 0, 9, u32::MAX]);
    capture.original_tag_members.len = 4;
    capture.original_tag_buckets.len = 3;
    capture.original_tag_bucket_len = Ok(3);
    capture.country_array.first[..4].copy_from_slice(&[0, 9, 0, 9]);
    capture.country_array_len = Ok(4);
    capture.country_array.len = 4;
    let [
        _,
        TableView::Complete(replay_data::rows::TableRows::ReservedCountryNaming(rows)),
        ..,
    ] = capture.tables()
    else {
        panic!("complete singleton")
    };
    assert_eq!(
        rows[0].original_tag_buckets,
        replay_data::rows::Recorded::Value(Some(vec![vec![], vec![9, 0, 9, u32::MAX], vec![]]))
    );
    assert_eq!(
        rows[0].original_tag_reference,
        replay_data::rows::Recorded::Value(Some(7))
    );
    assert_eq!(
        rows[0].original_tag,
        replay_data::rows::Recorded::Value(Some(vec![]))
    );
    assert_eq!(
        rows[0].country_array,
        replay_data::rows::Recorded::Value(Some(vec![0, 9, 0, 9]))
    );
    for count in [
        Some(i32::MIN),
        Some(-1),
        Some(0),
        Some(7),
        Some(i32::MAX),
        None,
    ] {
        capture.reserved.row.owned_state_count = count;
        let [
            _,
            TableView::Complete(replay_data::rows::TableRows::ReservedCountryNaming(rows)),
            ..,
        ] = capture.tables()
        else {
            panic!("count refusal preserves singleton")
        };
        assert_eq!(
            rows[0].owned_state_count,
            replay_data::rows::Recorded::Value(count)
        );
        assert_eq!(
            rows[0].original_tag_reference,
            replay_data::rows::Recorded::Value(Some(7))
        );
        assert_eq!(
            rows[0].original_tag,
            replay_data::rows::Recorded::Value(Some(vec![]))
        );
        assert_eq!(
            rows[0].original_tag_buckets,
            replay_data::rows::Recorded::Value(Some(vec![vec![], vec![9, 0, 9, u32::MAX], vec![]]))
        );
        assert_eq!(
            rows[0].country_array,
            replay_data::rows::Recorded::Value(Some(vec![0, 9, 0, 9]))
        );
    }

    for invalid in [Err("injected bucket refusal"), Ok(4)] {
        capture.original_tag_bucket_len = invalid;
        let [
            _,
            TableView::Complete(replay_data::rows::TableRows::ReservedCountryNaming(rows)),
            ..,
        ] = capture.tables()
        else {
            panic!("bucket refusal preserves prefix")
        };
        assert_eq!(
            rows[0].original_tag_buckets,
            replay_data::rows::Recorded::Value(None)
        );
        assert_eq!(
            rows[0].original_tag_reference,
            replay_data::rows::Recorded::Value(Some(7))
        );
        assert_eq!(
            rows[0].original_tag,
            replay_data::rows::Recorded::Value(Some(vec![]))
        );
        assert_eq!(
            rows[0].country_array,
            replay_data::rows::Recorded::Value(Some(vec![0, 9, 0, 9]))
        );
    }
    capture.original_tag_bucket_len = Ok(3);
    capture.original_tag_buckets.first[1].len = 5;
    assert_eq!(capture.original_tag_lists(), None);
    capture.original_tag_buckets.first[1].len = 4;
    for invalid in [Err("injected physical array refusal"), Ok(5)] {
        capture.country_array_len = invalid;
        let [
            _,
            TableView::Complete(replay_data::rows::TableRows::ReservedCountryNaming(rows)),
            ..,
        ] = capture.tables()
        else {
            panic!("array refusal preserves other fields")
        };
        assert_eq!(
            rows[0].country_array,
            replay_data::rows::Recorded::Value(None)
        );
        assert_eq!(
            rows[0].original_tag_buckets,
            replay_data::rows::Recorded::Value(Some(vec![vec![], vec![9, 0, 9, u32::MAX], vec![]]))
        );
        assert_eq!(
            rows[0].original_tag_reference,
            replay_data::rows::Recorded::Value(Some(7))
        );
    }
    capture.original_tag_buckets.required = 5000;
    capture.original_tag_members.required = 10000;
    capture.country_array.required = 5001;
    capture.grow(&mut capacities);
    assert!(capture.original_tag_buckets.first.len() >= 5000);
    assert!(capture.original_tag_members.first.len() >= 10000);
    assert!(capture.country_array.first.len() >= 5001);
    capture.reset();
    assert!(capture.original_tag_bucket_len.is_err());
    assert_eq!(capture.original_tag_members.len, 0);
    assert!(capture.country_array_len.is_err());
    assert_eq!(capture.country_array.len, 0);
}

#[test]
fn subject_growth_keeps_all_rows_and_payload_fault_omits_only_affected_countries() {
    use engine_native_memory::cpdx_array::Array;
    let bytes = [9_i32, -1, 9, 17]
        .into_iter()
        .flat_map(i32::to_le_bytes)
        .collect::<Vec<_>>();
    for payload_fault in [false, true] {
        let mut capacity = Capacities {
            rows: 3,
            subjects: if payload_fault { 4 } else { 1 },
            naming_bytes: 0,
            ..Default::default()
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
        capture.rows.outcome = Outcome::Truncated {
            len: 3,
            unread_rows: 1,
        };
        capture.capture_subjects(&SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        });
        let missing = 1 + u32::from(payload_fault);
        assert_eq!(
            capture.rows.outcome,
            Outcome::Truncated {
                len: 4 - missing as usize,
                unread_rows: missing
            }
        );
        assert_eq!(capture.subjects.required, 4);
        let [
            TableView::Truncated {
                rows: replay_data::rows::TableRows::CountryAppearance(rows),
                missing: actual_missing,
            },
            _,
            ..,
        ] = capture.tables()
        else {
            panic!("unaffected countries must remain partial rows");
        };
        assert_eq!(actual_missing, missing);
        assert_eq!(
            rows.iter().map(|row| row.country).collect::<Vec<_>>(),
            if payload_fault {
                vec![7, 11]
            } else {
                vec![3, 7, 11]
            }
        );
        if !payload_fault {
            assert_eq!(*rows[0].subjects.require("subjects").unwrap(), [9, -1, 9]);
        }
        assert!(
            rows[rows.len() - 2]
                .subjects
                .require("subjects")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            *rows.last().unwrap().subjects.require("subjects").unwrap(),
            [17]
        );
        capture.grow(&mut capacity);
        assert!(capture.subjects.first.len() >= 4);
        assert_eq!(capture.subject_starts.len(), capture.rows.rows.len());
    }
}

#[test]
fn subject_arena_presizes_from_live_country_slots_without_a_cap() {
    use engine_native_memory::cpdx_array::Array;
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

