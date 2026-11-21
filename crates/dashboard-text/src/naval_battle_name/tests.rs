use super::*;

#[test]
fn the_native_wrapped_distance_gives_fixed5_pixels() {
    let close = |value: i64, expected: i64| (value - expected).abs() * 10_000 <= expected;
    let wrapped = wrapped_distance(100, (1, [95, 0]), (2, [5, 4]));
    assert!(close(wrapped, 1_077_033), "{wrapped}");
    let plain = wrapped_distance(100, (1, [0, 0]), (2, [3, 4]));
    assert!(close(plain, 500_000), "{plain}");
    assert_eq!(wrapped_distance(100, (7, [0, 0]), (7, [3, 4])), 0);
    let large = wrapped_distance(10_000, (1, [0, 0]), (2, [2_000, 0]));
    assert!((199_900_000..=200_100_000).contains(&large), "{large}");
}

#[test]
fn the_first_candidate_is_taken_and_only_a_greater_score_replaces_it() {
    let positions = BTreeMap::from([(1, [0, 0]), (10, [3, 4]), (11, [6, 8]), (12, [0, 1])]);
    let points = BTreeMap::from([(10, 1), (11, 1), (12, 0)]);
    let position = |id: u32| positions.get(&id).copied().unwrap_or([0, 0]);
    let victory_points = |id: u32| {
        points
            .get(&id)
            .copied()
            .ok_or_else(|| format!("province {id} has no Provinces row"))
    };
    let picker = Picker {
        width: 1_000,
        factor: 10_000_000,
        cutoff: 100_000,
        position: &position,
        victory_points: &victory_points,
    };
    assert_eq!(picker.pick(1, &[11, 10]), Ok(Some(10)));
    assert_eq!(picker.pick(1, &[12]), Ok(None));
    assert_eq!(picker.pick(1, &[]), Ok(None));
    let own = BTreeMap::from([(1, 0)]);
    let own_points = |id: u32| Ok(own.get(&id).copied().unwrap_or(1));
    let picker_own = Picker {
        victory_points: &own_points,
        ..picker
    };
    assert_eq!(picker_own.pick(1, &[10, 1]), Ok(Some(1)));
    assert_eq!(
        picker.pick(1, &[99]),
        Err("province 99 has no Provinces row".into())
    );
}

#[test]
fn candidates_are_the_regions_states_in_first_appearance_order() {
    let region = |id, provinces: &[u32]| StrategicRegion {
        id,
        provinces: provinces.to_vec(),
        ..Default::default()
    };
    let state = |id, provinces: &[u32]| StateDefinition {
        id,
        provinces: provinces.to_vec(),
        ..Default::default()
    };
    let regions = BTreeMap::from([(1, region(1, &[30, 20, 21, 5, 404]))]);
    let states = BTreeMap::from([
        (7, state(7, &[20, 21, 22, 21, 404])),
        (8, state(8, &[31, 30])),
    ]);
    let valid = |province: u32| province != 404;
    assert_eq!(
        battle_name_candidates(&regions, &states, &valid)[&1],
        vec![31, 30, 20, 21, 22]
    );
    let states = BTreeMap::from([(7, state(7, &[20, 21])), (9, state(9, &[20, 23]))]);
    let regions = BTreeMap::from([(1, region(1, &[20, 21]))]);
    assert_eq!(
        battle_name_candidates(&regions, &states, &valid)[&1],
        vec![20, 23, 20, 21]
    );
}
