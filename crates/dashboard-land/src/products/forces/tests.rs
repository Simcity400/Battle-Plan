use super::*;

fn part(first: i32, last: i32, starts: bool) -> Part {
    Part { run: 0, key: 7, first, last, slot: 7, id: 70, segment_first: first, starts }
}

#[test]
fn parts_split_a_total_only_where_their_rows_add_up_to_it() {
    let held = [(0, Some(30)), (0, Some(20)), (1, Some(10))];
    assert_eq!(split(Some(60), &held, 2), vec![Some(50), Some(10)]);
    assert_eq!(split(Some(50), &[(0, Some(50)), (1, None)], 2), vec![Some(50), Some(0)]);
    assert_eq!(split(Some(60), &[(0, Some(50)), (1, None)], 2), vec![None, None]);
    assert_eq!(split(Some(40), &held, 2), vec![None, None]);
    assert_eq!(split(None, &held, 2), vec![None, None]);
    assert_eq!(split(Some(0), &[], 3), vec![Some(0), Some(0), Some(0)]);
}

#[test]
fn a_holders_runs_cut_where_its_total_reference_or_a_part_moves_and_join_where_nothing_does() {
    let totals = vec![(0, 9, Some(10)), (10, 19, None), (20, 29, Some(12))];
    let reference = vec![(0, 14, Some(100)), (15, 29, Some(120))];
    let first = vec![(0, 4, Some(10)), (5, 29, Some(4))];
    let second = vec![(0, 4, None), (5, 29, Some(6)), ];
    let owned: Owned<'_> = vec![(0, first.as_slice()), (1, second.as_slice())];
    let mut out = Vec::new();
    part_runs(&part(0, 29, true), 0, 29, &totals, Some(&reference), &owned, 2, &mut out);
    let shape: Vec<_> = out.iter().map(|run| (run.first_hour, run.last_hour, run.starts_segment, run.total, run.values.clone(), run.reference_total)).collect();
    assert_eq!(
        shape,
        vec![
            (0, 4, true, Some(10), vec![Some(10), Some(0)], Some(Some(100))),
            (5, 9, false, Some(10), vec![Some(4), Some(6)], Some(Some(100))),
            (10, 14, false, None, vec![None, None], Some(Some(100))),
            (15, 19, false, None, vec![None, None], Some(Some(120))),
            (20, 29, false, Some(12), vec![None, None], Some(Some(120))),
        ]
    );
    let mut cut = Vec::new();
    part_runs(&part(0, 29, true), 6, 8, &totals, None, &owned, 2, &mut cut);
    assert_eq!(cut, vec![ForceRun { first_hour: 6, last_hour: 8, starts_segment: false, total: Some(10), values: vec![Some(4), Some(6)], reference_total: None }]);
}

#[test]
fn recorded_parts_group_under_their_reply_part_in_key_order() {
    let resolved = BTreeMap::from([
        (3u128, ForcePart { key: "fighter".into(), name: Some("Fighter".into()), unresolved: false }),
        (1u128, ForcePart { key: "cas".into(), name: None, unresolved: false }),
        (2u128, ForcePart { key: "fighter".into(), name: Some("Fighter".into()), unresolved: false }),
    ]);
    let named = Named::group(resolved);
    assert_eq!(named.parts.iter().map(|part| part.key.as_str()).collect::<Vec<_>>(), vec!["cas", "fighter"]);
    assert_eq!((named.of[&1], named.of[&2], named.of[&3]), (0, 1, 1));
}

