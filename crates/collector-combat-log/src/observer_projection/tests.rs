use super::*;
use replay_data::rows::CombatLogObserverKind as K;
use std::collections::BTreeMap;

fn row(kind: K, item: u64, values: [u64; 5]) -> CombatLogObserverRow {
    CombatLogObserverRow::from_fact(u64::MAX, true, kind as u8, item, values).unwrap()
}

fn sample() -> Vec<CombatLogObserverRow> {
    vec![
        row(K::State, 0, [u64::MAX, i64::MIN as u64, 3, 0, 0]),
        row(
            K::Counters,
            0,
            [0, 1, u64::MAX, i32::MIN as i64 as u64, i32::MAX as u64],
        ),
        row(K::Counters, 5, [5, 6, 7, 8, 9]),
        row(K::Side, 0, [0, 12_500, u64::MAX, 0, 0]),
        row(K::Pool, 0, [0, 1, 0, 0, 0]),
        row(K::Pool, 1, [1, 1, 0, 0, 0]),
        row(K::Equipment, 1, [0, u64::MAX, u64::MAX, 0, 0]),
        row(K::Equipment, 1, [1, u64::MAX, u64::MAX, 0, 0]),
        row(K::Pool, 2, [1, 0, 0, 0, 0]),
        row(K::Equipment, 2, [0, 71, 0, 0, 0]),
        row(K::Modifier, 0, [0, u64::MAX, u64::MAX, 0, 0]),
        row(K::Country, 0, [0, 13, 0, 0, 0]),
        row(K::Country, 0, [1, 13, 0, 0, 0]),
        row(K::Leader, 0, [u64::MAX, u64::MAX, 0, 0, 0]),
        row(K::Damage, 0, [1, 2, 3, i64::MIN as u64, 0]),
        row(K::Activity, 0, [u64::MAX, 1, 2, 0, 0]),
        row(K::ActivityTemplate, 0, [0, 72, 0, 0, 0]),
        row(K::ActivityDamage, 0, [0, 73, u64::MAX, 0, 0]),
        row(K::ActivityEquipment, 0, [u64::MAX, 1, 1, 0, 0]),
        row(K::ActivityEquipment, 0, [0, 71, i64::MIN as u64, 0, 0]),
        row(K::Activity, 1, [u64::MAX, 2, 1, 0, 0]),
        row(K::ActivityTemplate, 1, [0, 72, 0, 0, 0]),
        row(K::ActivityDamage, 1, [0, 73, 10, 0, 0]),
        row(K::ActivityEquipment, 1, [u64::MAX, 0, 1, 0, 0]),
    ]
}
fn keyed(rows: &[CombatLogObserverRow]) -> BTreeMap<(u64, bool, u8, u64, u64), u128> {
    rows.iter()
        .map(|row| (row.semantic_key(), row.key()))
        .collect()
}
#[test]
fn combat_log_collection_insertions_keep_other_parents_and_kinds_keyed() {
    let first = project(&sample(), &[]).unwrap();
    let mut changed = sample();
    // A leader added before damage and activities in the native traversal.
    changed.insert(14, row(K::Leader, 1, [17, 2, 0, 0, 0]));
    // An early activity child must not renumber a later child family or parent.
    changed.insert(18, row(K::ActivityTemplate, 0, [1, 74, 0, 0, 0]));
    let second = project(&changed, &first).unwrap();
    let first = keyed(&first);
    let second = keyed(&second);
    for (semantic, key) in first {
        assert_eq!(second[&semantic], key);
    }
}
#[test]
fn combat_log_ordered_reconstruction_keeps_duplicates_and_every_word() {
    let native = sample();
    let expected: Vec<_> = native.iter().map(CombatLogObserverRow::fact).collect();
    let mut captured = native.clone();
    captured.reverse();
    let mut actual = project(&captured, &[]).unwrap();
    actual.sort_unstable_by_key(CombatLogObserverRow::logical_order);
    assert_eq!(
        actual
            .iter()
            .map(CombatLogObserverRow::fact)
            .collect::<Vec<_>>(),
        expected
    );
    assert_ne!(
        actual[6].key(),
        actual[7].key(),
        "equal equipment occurrences stay distinct"
    );
    assert_ne!(
        actual[11].key(),
        actual[12].key(),
        "equal country occurrences stay distinct"
    );
    assert!(actual.iter().any(|row| row.identity == u64::MAX));
}
#[test]
fn combat_log_resets_removals_replacements_and_unbounded_native_indices() {
    let mut native = sample();
    let first = project(&native, &[]).unwrap();
    native.retain(|row| row.kind != K::Leader as u8);
    let removed = project(&native, &first).unwrap();
    native[0].a = 0; // A real counter/damage reset is not inferred or suppressed.
    let reset = project(&native, &removed).unwrap();
    assert_eq!(keyed(&removed), keyed(&reset));
    assert_ne!(removed, reset);
    // No packed-key width narrows either full native array ordinal.
    native.push(row(
        K::ActivityDamage,
        i32::MAX as u64 - 1,
        [i32::MAX as u64 - 1, u64::MAX, 0, 0, 0],
    ));
    let extended = project(&native, &reset).unwrap();
    assert_eq!(extended.len(), native.len());
    let mut replacement = native.clone();
    replacement[0].combat -= 1;
    replacement[1].attacker = false;
    let fresh = project(&replacement, &[]).unwrap();
    assert_eq!(fresh.len(), replacement.len());
    assert!(project(&[], &extended).unwrap().is_empty());
}
#[test]
fn combat_log_projection_refuses_duplicate_occurrences_and_unknown_shapes() {
    let native = sample();
    assert_eq!(
        project(&[native[0].clone(), native[0].clone()], &[]).unwrap_err(),
        "combat log observer occurrence repeated"
    );
    assert_eq!(
        project(&native, &[native[0].clone(), native[0].clone()]).unwrap_err(),
        "combat log observer baseline occurrence repeated"
    );
    assert!(CombatLogObserverRow::from_fact(1, true, 255, 0, [0; 5]).is_err());
    assert!(CombatLogObserverRow::from_fact(1, true, K::Side as u8, 0, [1, 0, 0, 0, 0]).is_err());
}
#[test]
fn combat_log_synthetic_hourly_identity_churn_comparison() {
    let mut old_reassignments = 0;
    let mut stable_reassignments = 0;
    let mut previous_native: Vec<CombatLogObserverRow> = Vec::new();
    let mut previous_stable: Vec<CombatLogObserverRow> = Vec::new();
    for hour in 0..48 {
        let mut native = sample();
        if hour % 2 == 1 {
            native.insert(14, row(K::Leader, 1, [18, 1, 0, 0, 0]));
        }
        for parent in 2..34 {
            native.push(row(K::Activity, parent, [parent, 1, 2, 0, 0]));
            for index in 0..16 {
                native.push(row(
                    K::ActivityDamage,
                    parent,
                    [index, index + 700, hour * 10, 0, 0],
                ));
            }
            native.push(row(K::ActivityEquipment, parent, [u64::MAX, 0, 0, 0, 0]));
        }
        for (ordinal, row) in native.iter_mut().enumerate() {
            row.ordinal = ordinal as u64;
        }
        let stable = project(&native, &previous_stable).unwrap();
        if hour > 0 {
            let before = keyed(&previous_native);
            old_reassignments += native
                .iter()
                .filter(|r| {
                    before
                        .get(&r.semantic_key())
                        .is_some_and(|key| *key != r.key())
                })
                .count();
            let before = keyed(&previous_stable);
            stable_reassignments += stable
                .iter()
                .filter(|r| {
                    before
                        .get(&r.semantic_key())
                        .is_some_and(|key| *key != r.key())
                })
                .count();
        }
        previous_native = native;
        previous_stable = stable;
    }
    assert_eq!(old_reassignments, 586 * 47);
    assert_eq!(stable_reassignments, 0);
    println!(
        "47 hourly transitions: flattened key reassignments {old_reassignments} (586/hour), semantic key reassignments {stable_reassignments}"
    );
}
