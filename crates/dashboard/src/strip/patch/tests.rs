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
fn an_oversized_equal_length_search_edits_only_the_changed_elements() {
    let before: Vec<_> = (0..4_100).map(|index| json!(index)).collect();
    let mut after = before.clone();
    after[1] = json!(9_019);
    after[4_098] = json!(9_021);
    let (before, after) = (Value::Array(before), Value::Array(after));
    let patch = diff(&before, &after);
    assert_eq!(
        patch,
        [
            Operation::Replace(vec![Part::Index(1)], json!(9_019)),
            Operation::Replace(vec![Part::Index(4_098)], json!(9_021)),
        ]
    );
    assert_eq!(apply(&before, &patch).unwrap(), after);
    assert!(encode(&patch).unwrap().len() < rmp_serde::to_vec(&after).unwrap().len());
}

#[test]
fn an_exhausted_search_in_a_table_keeps_single_field_edits() {
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
fn an_exhausted_unequal_length_search_splices_only_the_middle() {
    let before = json!((0..80).collect::<Vec<_>>());
    let after = json!((0..30).chain(110..129).chain(50..80).collect::<Vec<_>>());
    let patch = diff(&before, &after);
    assert_eq!(
        patch,
        [Operation::Splice(
            Vec::new(),
            30,
            20,
            (110..129).map(|index| json!(index)).collect()
        )]
    );
    assert_eq!(apply(&before, &patch).unwrap(), after);
}
