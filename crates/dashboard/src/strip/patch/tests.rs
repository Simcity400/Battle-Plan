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
fn an_oversized_search_splices_only_the_middle() {
    let bookend = json!("prepared by the eastern dispatch office".repeat(535));
    let mut before: Vec<_> = (0..4_100).map(|index| json!(index)).collect();
    before[0] = bookend.clone();
    before[4_099] = bookend;
    let mut after = before.clone();
    after[1] = json!(9_019);
    after[4_098] = json!(9_021);
    let (before, after) = (Value::Array(before), Value::Array(after));
    let patch = diff(&before, &after);
    assert!(matches!(patch.as_slice(), [Operation::Splice(path, 1, 4_098, _)] if path.is_empty()));
    assert_eq!(apply(&before, &patch).unwrap(), after);
    assert!(encode(&patch).unwrap().len() < rmp_serde::to_vec(&after).unwrap().len());
}
