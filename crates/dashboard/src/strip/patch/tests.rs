use super::apply::apply;
use super::*;
use serde_json::json;

#[test]
fn patches_rebuild_values_both_ways_without_changing_the_input() {
    let cases = [
        (json!({"same": [1, null]}), json!({"same": [1, null]})),
        (json!(3), json!(7)),
        (json!({"gone": 1}), json!({"added": 2})),
        (json!([1, 4]), json!([1, 2, 3, 4])),
        (json!([1, 2, 3, 4]), json!([1, 4])),
        (json!([1, {"v": 2}, 3]), json!([1, {"v": 7}, 3])),
        (json!({"v": 1}), json!([1])),
        (
            json!({"answer": {"rows": []}}),
            json!({"refused": "Synthetic hour is not recorded"}),
        ),
        (
            json!({"0": {"": 1, "a/b~c": 2}}),
            json!({"0": {"": 2, "a/b~c": 3}}),
        ),
    ];
    for (before, after) in cases {
        let original = before.clone();
        let forward = diff(&before, &after);
        let decoded: Vec<Operation> = rmp_serde::from_slice(&encode(&forward).unwrap()).unwrap();
        assert_eq!(decoded, forward);
        assert_eq!(apply(&before, &decoded).unwrap(), after);
        assert_eq!(apply(&after, &diff(&after, &before)).unwrap(), before);
        assert_eq!(before, original);
        assert_eq!(forward.is_empty(), before == after);
    }
    assert_eq!(
        diff(&json!([1, 4]), &json!([1, 2, 3, 4])),
        [Operation::Splice(
            Vec::new(),
            1,
            0,
            vec![json!(2), json!(3)]
        )]
    );
}

#[test]
fn the_wire_uses_named_operations_and_typed_path_parts() {
    let patch = diff(
        &json!({"rows": [1, 2], "gone": 3}),
        &json!({"rows": [1, 4], "new": []}),
    );
    let wire: Value = rmp_serde::from_slice(&encode(&patch).unwrap()).unwrap();
    assert_eq!(
        wire,
        json!([
            {"remove": ["gone"]},
            {"replace": [["new"], []]},
            {"replace": [["rows", 1], 4]}
        ])
    );
    let invalid = [Operation::Splice(Vec::new(), 4, 1, Vec::new())];
    assert!(apply(&json!([1]), &invalid).is_err());
}

#[test]
fn a_hunk_pairs_rows_but_keeps_a_whole_replacement_when_it_is_smaller() {
    let before = json!([{"a": 19, "b": 23}]);
    let after = json!([{"a": 31, "b": 37}]);
    let patch = diff(&before, &after);
    assert_eq!(
        patch,
        [Operation::Replace(vec![Part::Index(0)], after[0].clone())]
    );
    assert_eq!(apply(&before, &patch).unwrap(), after);
}

#[test]
fn keyed_rows_scroll_and_keep_field_edits_at_their_destination_indices() {
    let before = json!([
        {"key": "a", "count": 1},
        {"key": "b", "count": 2},
        {"key": "gone", "count": 3},
    ]);
    let after = json!([
        {"key": "new-1", "count": 4},
        {"key": "new-2", "count": 5},
        {"key": "a", "count": 11},
        {"key": "b", "count": 12},
    ]);
    let patch = diff(&before, &after);
    assert_eq!(
        patch,
        [
            Operation::Splice(Vec::new(), 0, 0, vec![after[0].clone(), after[1].clone()]),
            Operation::Replace(vec![Part::Index(2), Part::Key("count".into())], json!(11)),
            Operation::Replace(vec![Part::Index(3), Part::Key("count".into())], json!(12)),
            Operation::Splice(Vec::new(), 4, 1, Vec::new()),
        ]
    );
    assert_eq!(apply(&before, &patch).unwrap(), after);
    assert_eq!(apply(&after, &diff(&after, &before)).unwrap(), before);
}

#[test]
fn reordered_keys_outside_the_longest_run_are_spliced_with_their_full_value() {
    let before = json!([
        {"key": "head", "count": 0},
        {"key": "a", "count": 1},
        {"key": "b", "count": 2},
        {"key": "c", "count": 3},
        {"key": "d", "count": 4},
        {"key": "tail", "count": 5},
    ]);
    let after = json!([
        {"key": "head", "count": 0},
        {"key": "b", "count": 2},
        {"key": "c", "count": 3},
        {"key": "d", "count": 4},
        {"key": "a", "count": 11},
        {"key": "tail", "count": 5},
    ]);
    let patch = diff(&before, &after);
    assert_eq!(
        patch,
        [
            Operation::Splice(Vec::new(), 1, 1, Vec::new()),
            Operation::Splice(Vec::new(), 4, 0, vec![after[4].clone()]),
        ]
    );
    assert_eq!(apply(&before, &patch).unwrap(), after);
    assert_eq!(apply(&after, &diff(&after, &before)).unwrap(), before);
}

#[test]
fn duplicate_keys_and_mixed_rows_match_by_value() {
    let first = json!({"key": "a", "count": 1});
    let changed = json!({"key": "a", "count": 11});
    for anchor in [json!({"key": "a", "count": 2}), json!(2)] {
        let before = json!([first, anchor]);
        let after = json!([anchor, changed]);
        let patch = diff(&before, &after);
        assert_eq!(
            patch,
            [
                Operation::Splice(Vec::new(), 0, 1, Vec::new()),
                Operation::Splice(Vec::new(), 1, 0, vec![changed.clone()]),
            ]
        );
        assert_eq!(apply(&before, &patch).unwrap(), after);
        assert_eq!(apply(&after, &diff(&after, &before)).unwrap(), before);
    }
}

#[test]
fn value_search_matches_across_more_than_thirty_two_edits() {
    let before = json!((0..511).collect::<Vec<_>>());
    let after = json!((20..531).collect::<Vec<_>>());
    let patch = diff(&before, &after);
    assert_eq!(
        patch,
        [
            Operation::Splice(Vec::new(), 0, 20, Vec::new()),
            Operation::Splice(Vec::new(), 491, 0, (511..531).map(|n| json!(n)).collect()),
        ]
    );
    assert_eq!(apply(&before, &patch).unwrap(), after);
    assert_eq!(apply(&after, &diff(&after, &before)).unwrap(), before);
}

#[test]
fn an_oversized_equal_length_search_edits_only_the_changed_elements() {
    let before: Vec<_> = (0..514).map(|index| json!(index)).collect();
    let mut after = before.clone();
    after.swap(1, 2);
    after[512] = json!(9_021);
    let (before, after) = (Value::Array(before), Value::Array(after));
    let patch = diff(&before, &after);
    assert_eq!(
        patch,
        [
            Operation::Replace(vec![Part::Index(1)], json!(2)),
            Operation::Replace(vec![Part::Index(2)], json!(1)),
            Operation::Replace(vec![Part::Index(512)], json!(9_021)),
        ]
    );
    assert_eq!(apply(&before, &patch).unwrap(), after);
    assert!(encode(&patch).unwrap().len() < rmp_serde::to_vec(&after).unwrap().len());
}

#[test]
fn an_equal_count_hunk_in_a_table_keeps_single_field_edits() {
    let rows = |offset: i32| {
        let rows = (0..100).map(|id| {
            json!({
                "ordinal": id,
                "displayName": format!("Parcel manifest line {id} for scheduled dispatch"),
                "warehouseIdentifier": "North quay consolidation office",
                "destination": "Harbor transfer depot",
                "state": "Awaiting reconciliation",
                "count": id * 3 + 17 + offset,
            })
        });
        Value::Array(rows.collect())
    };
    let (before, after) = (rows(0), rows(1));
    let patch = diff(&before, &after);
    assert_eq!(patch.len(), 100);
    assert!(patch.iter().all(|op| {
        let Operation::Replace(path, _) = op else {
            return false;
        };
        matches!(path.as_slice(), [Part::Index(_), Part::Key(key)] if key == "count")
    }));
    assert_eq!(apply(&before, &patch).unwrap(), after);
    assert!(encode(&patch).unwrap().len() * 5 < rmp_serde::to_vec(&after).unwrap().len());
}

#[test]
fn a_search_just_over_the_cell_budget_splices_only_the_unequal_middle() {
    let before = json!((0..514).collect::<Vec<_>>());
    let after = json!((0..1).chain(2..512).chain([900, 513]).collect::<Vec<_>>());
    let patch = diff(&before, &after);
    assert_eq!(
        patch,
        [Operation::Splice(
            Vec::new(),
            1,
            512,
            (2..512).chain([900]).map(|index| json!(index)).collect()
        )]
    );
    assert_eq!(apply(&before, &patch).unwrap(), after);
}
