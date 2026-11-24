use super::*;

#[test]
fn border_optional_values_and_pairs_survive_delta_clear_and_reintroduction() {
    let old = DivisionStatusRow {
        division: 1,
        border_combat_stats: crate::rows::Recorded::Value(Some([i64::MIN, -100_001, i64::MAX])),
        ..Default::default()
    };
    let absent = DivisionStatusRow {
        division: 1,
        ..Default::default()
    };
    let delta = absent.delta(Some(&old)).unwrap().unwrap();
    assert_eq!(
        delta.apply::<DivisionStatusRow>(Some(old.clone())).unwrap(),
        absent
    );
    assert_eq!(
        old.delta(Some(&absent))
            .unwrap()
            .unwrap()
            .apply::<DivisionStatusRow>(Some(absent))
            .unwrap(),
        old
    );
    let country = CountryRow {
        id: 1,
        border_land_doctrine_level: crate::rows::Recorded::Value(Some(i32::MIN)),
        ..Default::default()
    };
    let absent = CountryRow {
        id: 1,
        ..Default::default()
    };
    assert_eq!(
        absent
            .delta(Some(&country))
            .unwrap()
            .unwrap()
            .apply::<CountryRow>(Some(country))
            .unwrap(),
        absent
    );
    let state = StateFlagRow {
        state: 1,
        border_conflict_active: crate::rows::Recorded::Value(255),
        border_conflict_countries: vec![[i32::MIN, 0], [7, 7]].into(),
        ..Default::default()
    };
    let inactive = StateFlagRow {
        state: 1,
        ..Default::default()
    };
    assert_eq!(
        inactive
            .delta(Some(&state))
            .unwrap()
            .unwrap()
            .apply::<StateFlagRow>(Some(state))
            .unwrap(),
        inactive
    );
}
#[test]
fn inactive_state_pair_payload_is_refused() {
    let row = StateFlagRow {
        state: 1,
        border_conflict_countries: vec![[1, 2]].into(),
        ..Default::default()
    };
    assert!(row.delta(None).is_err());
}

