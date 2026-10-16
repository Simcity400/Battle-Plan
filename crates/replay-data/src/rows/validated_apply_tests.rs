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

/// Division-free row change: `key`'s canonical slot moves by `by`.
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

/// The row-by-row merge every delta used, as the reference result.
fn merged(rows: &TableRows, changed: &[DeltaRow], deleted: &[u128]) -> io::Result<TableRows> {
    let rows = rows.apply_delta_shared(changed, deleted)?;
    rows.validate_keys()?;
    Ok(rows)
}

#[test]
fn a_few_insertions_and_deletions_apply_in_place_checking_only_their_rows() {
    let changed = [slot_change(500, 1), slot_change(1500, 7)];
    let (rows, checked) = validated(|| links(1000).apply_delta_validated(&changed, &[3]));
    assert_eq!(rows.unwrap(), merged(&links(1000), &changed, &[3]).unwrap());
    assert_eq!(checked, 2);
}

#[test]
fn many_insertions_and_deletions_merge_and_check_the_whole_table() {
    let changed = [slot_change(500, 1), slot_change(1500, 7)];
    let deleted = [3, 5, 7];
    let (rows, checked) = validated(|| links(1000).apply_delta_validated(&changed, &deleted));
    let rows = rows.unwrap();
    assert_eq!(rows, merged(&links(1000), &changed, &deleted).unwrap());
    assert_eq!(checked, rows.len());
}

#[test]
fn every_mix_of_changes_insertions_and_deletions_matches_the_merge() {
    // Rows 2, 4, .., 40; odd keys are absent.
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
        // Sparse to dense deltas, so both in-place edits and merges succeed.
        let spread = 6 + random(60);
        let (mut changed, mut deleted) = (Vec::new(), Vec::new());
        for key in 0..=42_u128 {
            match random(spread) {
                // A present row changes, an absent one is inserted.
                0 | 1 => changed.push(slot_change(key, i64::from(random(5)) - 2)),
                // Deleting an absent key fails exactly as the merge fails.
                2 => deleted.push(key),
                _ => {}
            }
        }
        let expected = merged(&table(), &changed, &deleted).map_err(|error| error.to_string());
        let actual = table()
            .apply_delta_validated(&changed, &deleted)
            .map_err(|error| error.to_string());
        assert_eq!(actual, expected, "changed {changed:?} deleted {deleted:?}");
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
