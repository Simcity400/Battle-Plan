use super::*;

fn links(count: u32) -> TableRows {
    TableRows::CountryLinks(
        (1..=count)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    )
}

fn slot_change(key: u128, by: i64) -> DeltaRow {
    let mut values = Vec::new();
    put_varint(&mut values, zigzag(i128::from(by)));
    DeltaRow {
        format: crate::FORMAT_VERSION,
        key,
        mask: 1,
        recorded_fields: TableId::CountryLinks.field_count(),
        values,
    }
}

fn zigzag(value: i128) -> u128 {
    ((value << 1) ^ (value >> 127)) as u128
}

fn validated(apply: impl FnOnce() -> io::Result<TableRows>) -> (io::Result<TableRows>, usize) {
    VALIDATED_ROWS.with(|count| count.set(0));
    let rows = apply();
    (rows, VALIDATED_ROWS.with(std::cell::Cell::get))
}

#[test]
fn validated_in_place_deltas_check_only_their_rows() {
    let changed = [slot_change(500, 1), slot_change(501, -1)];
    let (rows, checked) = validated(|| links(1000).apply_delta_validated(&changed, &[]));
    let rows = rows.unwrap();
    assert_eq!(checked, 2);
    let expected = links(1000).apply_delta(&changed, &[]).unwrap();
    expected.validate_keys().unwrap();
    assert_eq!(rows, expected);
    let slots = rows.rows::<tables::CountryLinks>().unwrap();
    assert_eq!(
        (slots[499].canonical_slot, slots[500].canonical_slot),
        (501, 500)
    );
}

fn merged(rows: &TableRows, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<TableRows> {
    let rows = rows.apply_delta_shared(changed, deleted)?;
    rows.validate_keys()?;
    Ok(rows)
}

#[test]
fn shared_deltas_leave_the_baseline_and_check_only_their_rows() {
    let baseline = links(1000);
    let changed = [
        slot_change(500, 1),
        slot_change(501, -1),
        slot_change(1500, 7),
    ];
    let deleted = [3, 5, 7];
    let (rows, checked) = validated(|| baseline.apply_delta_shared_validated(&changed, &deleted));
    assert_eq!(
        rows.unwrap(),
        merged(&baseline, &changed, &deleted).unwrap()
    );
    assert_eq!(checked, changed.len());
    assert_eq!(baseline, links(1000));
}

#[test]
fn many_insertions_and_deletions_merge_and_check_only_changed_rows() {
    let changed = [slot_change(500, 1), slot_change(1500, 7)];
    let deleted = [3, 5, 7];
    let (rows, checked) = validated(|| links(1000).apply_delta_validated(&changed, &deleted));
    let rows = rows.unwrap();
    assert_eq!(rows, merged(&links(1000), &changed, &deleted).unwrap());
    assert_eq!(checked, changed.len());
}

#[test]
fn every_mix_of_changes_insertions_and_deletions_matches_the_merge() {
    let table = || {
        TableRows::CountryLinks(
            (1..=20)
                .map(|id| CountryLinkRow {
                    id: id * 2,
                    canonical_slot: id,
                })
                .collect(),
        )
    };
    let mut state = 0x9e37_79b9_u32;
    let mut random = |bound: u32| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state % bound
    };
    let (mut kept, mut edited) = (0, 0);
    for _ in 0..4000 {
        let spread = 6 + random(60);
        let (mut changed, mut deleted) = (Vec::new(), Vec::new());
        for key in 0..=42_u128 {
            match random(spread) {
                0 | 1 => changed.push(slot_change(key, i64::from(random(5)) - 2)),
                2 => deleted.push(key),
                _ => {}
            }
        }
        let expected = merged(&table(), &changed, &deleted).map_err(|error| error.to_string());
        let actual = table()
            .apply_delta_validated(&changed, &deleted)
            .map_err(|error| error.to_string());
        assert_eq!(actual, expected, "changed {changed:?} deleted {deleted:?}");
        let shared = table()
            .apply_delta_shared_validated(&changed, &deleted)
            .map_err(|error| error.to_string());
        assert_eq!(shared, expected, "changed {changed:?} deleted {deleted:?}");
        if let Ok(rows) = actual {
            kept += 1;
            edited += usize::from(rows.len() != 20 || !deleted.is_empty());
        }
    }
    assert!(
        kept > 1000 && edited > 500,
        "{kept} applied, {edited} with edits"
    );
}

#[test]
fn bulk_merge_every_hour_matches_the_original_merge_including_deletion_only_hours() {
    let mut actual = links(1000);
    let mut reference = actual.clone();
    for hour in 0..32_u128 {
        let deleted: Vec<_> = (hour * 3 + 1..=hour * 3 + 3).collect();
        let changed = if hour % 4 == 0 {
            vec![]
        } else {
            vec![slot_change(500 + hour, 1), slot_change(1500 + hour, 7)]
        };
        let expected = merged(&reference, &changed, &deleted).unwrap();
        let (next, checked) = validated(|| actual.apply_delta_validated(&changed, &deleted));
        actual = next.unwrap();
        reference = expected;
        assert_eq!(actual, reference, "hour {hour}");
        assert_eq!(checked, changed.len(), "hour {hour}");
        let (mut actual_bytes, mut reference_bytes) = (Vec::new(), Vec::new());
        actual.encode_rows(&mut actual_bytes).unwrap();
        reference.encode_rows(&mut reference_bytes).unwrap();
        assert_eq!(actual_bytes, reference_bytes, "hour {hour}");
    }
}

#[test]
fn bulk_merge_keeps_apply_before_final_validation_and_absent_delete_precedence() {
    let baseline = TableRows::LandCombatDivisions(
        (0..10)
            .map(|ordinal| LandCombatDivisionRow {
                combat: 7,
                ordinal,
                ..Default::default()
            })
            .collect(),
    );
    let key = |ordinal| (7_u128 << 33) | ordinal;
    let mut bad_value = Vec::new();
    encode_native_change(
        &Some(LandCombatDivisionRow::SPEED_SHARE_MAXIMUM + 1),
        Some(&None),
        None,
        &mut bad_value,
    )
    .unwrap();
    let semantic = DeltaRow {
        key: key(4),
        mask: 1 << 13,
        format: crate::FORMAT_VERSION,
        recorded_fields: TableId::LandCombatDivisions.field_count(),
        values: bad_value,
    };
    let malformed = DeltaRow {
        key: key(8),
        mask: 1 << 9,
        format: crate::FORMAT_VERSION,
        recorded_fields: TableId::LandCombatDivisions.field_count(),
        values: vec![],
    };
    for changed in [
        vec![semantic.clone()],
        vec![semantic.clone(), malformed.clone()],
    ] {
        for deleted in [
            vec![key(0), key(1), key(2)],
            vec![key(0), key(1), key(2), key(11)],
        ] {
            let expected = merged(&baseline, &changed, &deleted).unwrap_err();
            let actual = baseline
                .clone()
                .apply_delta_validated(&changed, &deleted)
                .unwrap_err();
            assert_eq!(
                (actual.kind(), actual.to_string()),
                (expected.kind(), expected.to_string()),
                "{changed:?} {deleted:?}"
            );
            let shared = baseline
                .apply_delta_shared_validated(&changed, &deleted)
                .unwrap_err();
            assert_eq!(
                (shared.kind(), shared.to_string()),
                (expected.kind(), expected.to_string()),
                "{changed:?} {deleted:?}"
            );
        }
    }
}

#[test]
fn row_changes_rebuild_every_later_version() {
    let mut state = 0x2545_f491_u32;
    let mut random = |bound: u32| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state % bound
    };
    let mut table = |keep: u32| {
        TableRows::CountryLinks(
            (1..=40)
                .filter(|_| random(keep) != 0)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: id,
                })
                .collect(),
        )
    };
    for round in 0..2000 {
        let previous = table(2 + round % 7);
        let mut later = table(2 + round % 5);
        if let TableRows::CountryLinks(rows) = &mut later {
            for row in rows.iter_mut().filter(|row| row.id % 3 == round % 3) {
                row.canonical_slot += 1;
            }
        }
        let changes = later.changes_since(&previous).unwrap().unwrap();
        assert_eq!(previous.clone().apply_changes(changes).unwrap(), later);
        let unchanged = later.changes_since(&later).unwrap().unwrap();
        assert_eq!(unchanged.heap_bytes(), 0);
        assert_eq!(later.clone().apply_changes(unchanged).unwrap(), later);
    }
    assert!(
        links(3)
            .changes_since(&TableRows::Countries(Vec::new()))
            .unwrap()
            .is_none()
    );
    let error = links(3)
        .apply_changes(RowChanges {
            rows: TableRows::Countries(Vec::new()),
            deleted: Vec::new(),
        })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "row changes of another table");
}
