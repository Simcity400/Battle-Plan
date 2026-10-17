use super::*;

fn fixture(states: &str) -> (Save, PartialMapMembership) {
    let header = "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 countries={A={} B={}} provinces={} ";
    let save = super::super::parse(format!("{header}states={{{states}}}").as_bytes()).unwrap();
    let membership = PartialMapMembership {
        known_states: [1].into_iter().collect(),
        ..Default::default()
    };
    (save, membership)
}

#[test]
fn omission_is_a_separate_current_emission_and_strict_control_still_refuses() {
    let (save, membership) =
        fixture("1={name=label manpower_pool={available=-5 locked=0 total=1}}");
    assert_eq!(
        save.state_control_emissions(&membership).unwrap(),
        [StateControlEmission::NonPositiveOwner { id: 1 }]
    );
    assert!(
        save.known_state_control(&membership)
            .unwrap_err()
            .to_string()
            .contains("lacks its owner")
    );
    // Province control preserves the owner's nonpositive emission class.
    let mut membership = membership;
    membership
        .provinces
        .insert(2, ProvinceMembership::Known(Some(1)));
    let rows = save.province_control(&membership).unwrap();
    assert_eq!(rows[0].owner, None);
    assert_eq!(rows[0].controller, None);
}

#[test]
fn positive_default_and_explicit_controller_use_the_unchanged_policy() {
    for (state, controller, explicit) in [
        ("1={owner=A}", b"A".as_slice(), false),
        ("1={owner=A controller=B}", b"B".as_slice(), true),
    ] {
        let (save, membership) = fixture(state);
        let strict = save.known_state_control(&membership).unwrap();
        assert_eq!(
            save.state_control_emissions(&membership).unwrap(),
            strict
                .iter()
                .cloned()
                .map(StateControlEmission::Positive)
                .collect::<Vec<_>>()
        );
        assert_eq!(strict[0].owner, b"A");
        assert_eq!(strict[0].controller, controller);
        assert_eq!(strict[0].controller_explicit, explicit);
    }
    for state in [
        "1={owner=UNKNOWN}",
        "1={owner=A controller=UNKNOWN}",
        "1={owner=\"\"}",
    ] {
        let (save, membership) = fixture(state);
        assert!(save.state_control_emissions(&membership).is_err());
        assert!(save.known_state_control(&membership).is_err());
    }
}

#[test]
fn a_controller_without_owner_is_not_a_nonpositive_emission() {
    let (save, membership) = fixture("1={controller=A}");
    assert!(
        save.state_control_emissions(&membership)
            .unwrap_err()
            .to_string()
            .contains("controller without a positive owner")
    );
}

#[test]
fn omission_keeps_mandatory_saved_and_content_state_inventory() {
    let (mut save, mut membership) = fixture("1={}");
    membership.known_states.insert(2);
    assert!(
        save.state_control_emissions(&membership)
            .unwrap_err()
            .to_string()
            .contains("save lacks state 2")
    );
    membership.known_states.remove(&2);
    let mut extra = save.states[&1].clone();
    extra.id = 2;
    save.states.insert(2, extra);
    assert!(
        save.state_control_emissions(&membership)
            .unwrap_err()
            .to_string()
            .contains("no membership")
    );
    membership.excluded_states.insert(
        2,
        super::super::ExcludedState {
            file: "unreadable-state.txt".into(),
            reasons: vec!["source fixture".into()],
        },
    );
    assert_eq!(save.state_control_emissions(&membership).unwrap().len(), 2);
}

#[test]
fn unowned_state_provinces_keep_null_ownership_and_explicit_controllers() {
    let (mut save, mut membership) = fixture("1={}");
    for id in [2, 3, 4] {
        membership
            .provinces
            .insert(id, ProvinceMembership::Known(Some(1)));
    }
    save.provinces.insert(
        3,
        super::super::ProvinceOverride {
            id: 3,
            name: None,
            owner: None,
            controller: Some(b"B".to_vec()),
            victory_points: None,
            strategic_locations: Vec::new(),
        },
    );
    let rows = save.province_control(&membership).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .all(|row| row.state == Some(1) && row.owner.is_none())
    );
    assert_eq!(rows[0].controller, None);
    assert_eq!(rows[1].controller.as_deref(), Some(b"B".as_slice()));
    assert!(rows[1].controller_explicit);
    assert_eq!(rows[2].controller, None);
    save.states.remove(&1);
    assert!(
        save.province_control(&membership)
            .unwrap_err()
            .to_string()
            .contains("save lacks state 1")
    );
}

#[test]
fn unowned_state_control_scales_to_5000_synthetic_countries() {
    let (mut save, mut membership) = fixture("1={}");
    let country = save.countries[b"A".as_slice()].clone();
    let state = save.states[&1].clone();
    save.countries.clear();
    for id in 1..=5000 {
        let mut country = country.clone();
        country.tag = format!("Q{id:06}").into_bytes();
        let tag = country.tag.clone();
        save.countries.insert(tag.clone(), country);
        let mut state = state.clone();
        state.id = id;
        save.states.insert(id, state);
        membership.known_states.insert(id);
        membership
            .provinces
            .insert(id, ProvinceMembership::Known(Some(id)));
        if id % 2 == 0 {
            save.provinces.insert(
                id,
                super::super::ProvinceOverride {
                    id,
                    name: None,
                    owner: None,
                    controller: Some(tag),
                    victory_points: None,
                    strategic_locations: Vec::new(),
                },
            );
        }
    }
    let rows = save.province_control(&membership).unwrap();
    assert_eq!(rows.len(), 5000);
    assert!(rows.iter().all(|row| row.owner.is_none()));
    assert_eq!(
        rows.iter().filter(|row| row.controller.is_some()).count(),
        2500
    );
    assert_eq!(
        save.state_control_emissions(&membership).unwrap().len(),
        5000
    );
}
