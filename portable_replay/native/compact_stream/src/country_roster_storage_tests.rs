use super::*;

fn observation(count: u32) -> Observation {
    Observation {
        kind: Kind::Division,
        country_count: Some(1),
        failure: None,
        countries: vec![Country {
            physical_slot: 0,
            owner_tag_db_id: Some(7),
            acquisition: Acquisition::Bracketed,
            observed_count: Some(count),
            failure: None,
            slots: (0..count).map(|ordinal| Slot {
                ordinal,
                reference: Ok(MemberRef { native_type: 4713, id: ordinal + 1 }),
            }).collect(),
        }],
    }
}

#[test]
fn roster_append_matches_wire_bytes_and_preserves_funded_allocation() {
    let value = observation(4);
    let expected = value.encode().unwrap();
    assert_eq!(value.encoded_len().unwrap(), HEADER_LEN + COUNTRY_LEN + 4 * SLOT_LEN);
    let mut out = Vec::new();
    out.try_reserve_exact(3 + expected.len()).unwrap();
    out.extend_from_slice(&[1, 2, 3]);
    let allocation = (out.as_ptr(), out.capacity());
    value.append_to(&mut out).unwrap();
    assert_eq!((out.as_ptr(), out.capacity()), allocation);
    assert_eq!(&out[..3], &[1, 2, 3]);
    assert_eq!(&out[3..], expected);
    assert_eq!(Observation::parse(&out[3..], Kind::Division).unwrap(), value);
}

#[test]
fn roster_append_refuses_capacity_and_invalid_census_before_mutation() {
    let mut value = observation(1);
    let mut out = vec![1, 2, 3];
    let before = out.clone();
    assert!(value.append_to(&mut out).unwrap_err().contains("capacity"));
    assert_eq!(out, before);
    out.try_reserve_exact(value.encoded_len().unwrap()).unwrap();
    value.countries[0].slots[0].ordinal = 1;
    assert!(value.append_to(&mut out).unwrap_err().contains("slot order"));
    assert_eq!(out, before);
}

fn check_unique(values: impl Iterator<Item = u64> + Clone) -> bool {
    let count = values.clone().count();
    let mut out = Vec::new();
    out.try_reserve_exact(3 + count * 8).unwrap();
    out.extend_from_slice(&[1, 2, 3]);
    let allocation = (out.as_ptr(), out.capacity());
    let reads = std::cell::Cell::new(0);
    let unique = unique_in_spare(&mut out, count, values.map(|value| {
        reads.set(reads.get() + 1);
        value.to_le_bytes()
    }), |left, right| left.cmp(right)).unwrap();
    assert_eq!(reads.get(), count, "keys are consumed once regardless of order");
    assert_eq!(out, [1, 2, 3]);
    assert_eq!((out.as_ptr(), out.capacity()), allocation);
    unique
}

#[test]
fn funded_uniqueness_covers_bounds_orders_and_late_duplicates_in_one_scan() {
    for count in [0_u64, 1, 4096, 4097, 20_000, 65_536, 250_000] {
        assert!(check_unique(0..count), "ascending {count}");
        assert!(check_unique((0..count).rev()), "descending {count}");
        assert!(check_unique((0..count).map(|key| (key * 257) % count)), "permuted {count}");
    }
    assert!(check_unique((0..8192_u64).map(|word| word.rotate_left(37))));
    assert!(check_unique((u64::MAX - 8192)..u64::MAX));
    assert!(!check_unique(std::iter::repeat_n(7_u64, 4097)));
    for duplicate in [0_u64, 2047, 2048, 4095, 4096, 8191] {
        assert!(!check_unique((0..8192).chain(std::iter::once(duplicate))), "late {duplicate}");
    }
}

#[test]
fn roster_sizing_defers_duplicates_but_append_and_legacy_paths_reject_them() {
    let mut value = observation(8192);
    assert!(value.encoded_len().is_ok());
    value.countries[0].slots[8191].reference = value.countries[0].slots[0].reference;
    let mut out = vec![1, 2, 3];
    out.try_reserve_exact(value.encoded_len().unwrap()).unwrap();
    assert!(value.append_to(&mut out).unwrap_err().contains("ambiguous member identity"));
    assert_eq!(out, [1, 2, 3]);
    assert!(value.validate().unwrap_err().contains("ambiguous member identity"));
    assert!(value.encode().unwrap_err().contains("ambiguous member identity"));
    let mut empty = observation(0);
    empty.countries.push(empty.countries[0].clone());
    empty.countries[1].physical_slot = 1;
    empty.country_count = Some(2);
    assert!(empty.encoded_len().is_ok());
    assert!(empty.append_to(&mut out).unwrap_err().contains("ambiguous owner identity"));
    assert_eq!(out, [1, 2, 3]);
    assert!(empty.validate().unwrap_err().contains("ambiguous owner identity"));
}

#[test]
fn funded_uniqueness_refuses_scratch_capacity_and_cardinality_without_mutation() {
    let mut out = vec![1, 2, 3];
    assert!(unique_in_spare(&mut out, 1, [7_u64.to_le_bytes()].into_iter(), |left, right| left.cmp(right))
        .unwrap_err().contains("capacity"));
    assert_eq!(out, [1, 2, 3]);
    out.try_reserve_exact(8).unwrap();
    assert!(unique_in_spare(&mut out, 1, [7_u64.to_le_bytes(), 8_u64.to_le_bytes()].into_iter(), |left, right| left.cmp(right))
        .unwrap_err().contains("cardinality"));
    assert_eq!(out, [1, 2, 3]);
}
