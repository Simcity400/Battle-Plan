use super::fixture::*;
use super::*;

fn with_hour<R>(run: impl FnOnce(&Hour<'_>) -> R) -> R {
    let content = content();
    let frame = frame();
    let hour = Hour::new(&frame, &content).expect("the fixture joins");
    run(&hour)
}

#[test]
fn every_resource_with_a_listing_pool_gate_grant_or_cost_gives_its_state_an_icon() {
    with_hour(|hour| {
        let has = |state| hour.has_icon(state).unwrap();
        assert!(has(MIXED));
        assert!(has(COSTED));
        assert!(has(GATED));
        assert!(has(GRANTED));
        assert!(has(ORPHAN));
        assert!(!has(EMPTY));
        assert_eq!(hour.icon(EMPTY).unwrap(), None);
        let refused = hour.icon(ORPHAN).unwrap_err();
        assert!(refused.contains("holds no resource origin"), "{refused}");
        for state in [GATED, COSTED] {
            assert_eq!(
                hour.icon(state).unwrap(),
                Some(StateIcon {
                    rows: Vec::new(),
                    flag: None
                })
            );
        }
    });
}

#[test]
fn rows_read_the_rights_holders_origin_with_damage_temporary_oil_and_efficiency() {
    with_hour(|hour| {
        let oil = hour.amounts(MIXED, 0).unwrap();
        assert_eq!(oil.holder, Some(AFG));
        assert_eq!((oil.origin.country, oil.origin.list), (AFG, 1));
        assert_eq!(oil.operands.oil, 150_000);
        assert_eq!(oil.operands.infrastructure, 160_000);
        assert_eq!(oil.operands.damage, 30_000);
        assert_eq!(oil.damaged, 2);
        assert_eq!(oil.operands.grants, 100_000);
        assert_eq!(oil.nominal, 100_000 * 150_000 / ONE + 320_000);
        assert_eq!(oil.delivered, 470_000 - 60_000 + 90_000);
        let steel = hour.amounts(MIXED, 1).unwrap();
        assert_eq!(steel.holder, None);
        assert_eq!((steel.origin.country, steel.origin.list), (GER, 0));
        assert_eq!(steel.nominal, 640_000 * 110_000 / ONE);
        assert_eq!(steel.delivered, (704_000 - 132_000) * 80_000 / ONE);
        let rubber = hour.amounts(MIXED, 2).unwrap();
        assert_eq!((rubber.nominal, rubber.delivered), (-30_000, -30_000));
        let icon = hour.icon(MIXED).unwrap().unwrap();
        assert_eq!(
            icon,
            StateIcon {
                rows: vec![
                    IconRow {
                        definition: 0,
                        nominal: 470_000,
                        delivered: 500_000,
                        flag: Some(AFG),
                    },
                    IconRow {
                        definition: 1,
                        nominal: 704_000,
                        delivered: 457_600,
                        flag: None,
                    },
                ],
                flag: None,
            }
        );
        assert_eq!(
            icon.rows
                .iter()
                .map(|row| (
                    whole(row.delivered),
                    whole(row.nominal),
                    RowColour::of(row.delivered, row.nominal)
                ))
                .collect::<Vec<_>>(),
            [(5, 5, RowColour::Green), (5, 7, RowColour::Red)]
        );
    });
}

#[test]
fn one_foreign_holder_of_every_row_takes_the_windows_flag() {
    with_hour(|hour| {
        let given = hour.icon(GIVEN).unwrap().unwrap();
        assert_eq!(given.flag, Some(AFG));
        assert_eq!(given.rows.len(), 1);
        assert_eq!(given.rows[0].flag, None);
        assert_eq!(
            RowColour::of(given.rows[0].delivered, given.rows[0].nominal),
            RowColour::Plain
        );
        let aliased = hour.icon(ALIASED).unwrap().unwrap();
        assert_eq!(aliased.flag, None);
        assert_eq!(aliased.rows[0].flag, None);
        assert_eq!(hour.amounts(ALIASED, 1).unwrap().origin.country, GER);
        let granted = hour.icon(GRANTED).unwrap().unwrap();
        assert_eq!(granted.rows[0].nominal, 300_000);
        assert_eq!(granted.rows[0].delivered, 300_000);
    });
}

#[test]
fn a_rights_holder_without_an_origin_is_refused_by_name() {
    let content = content();
    let mut frame = frame();
    put(
        &mut frame,
        replay_data::rows::TableRows::ResourceRights(vec![replay_data::rows::ResourceRightRow {
            ordinal: 0,
            state: GRANTED,
            resource: 1,
            holder: AFG,
        }]),
    );
    let hour = Hour::new(&frame, &content).unwrap();
    let refused = hour.icon(GRANTED).unwrap_err();
    assert!(refused.contains("rights holder"), "{refused}");
}

#[test]
fn delivered_clamps_before_efficiency_and_subtracts_the_cost_after() {
    let operands = Operands {
        full: 100_000,
        grants: 0,
        oil: ONE,
        infrastructure: ONE,
        route: 120_000,
        local: ONE,
        cost: 50_000,
        damage: 200_000,
        temporary: 0,
        efficiency: 50_000,
    };
    assert_eq!(delivered(&operands), -50_000);
    assert_eq!(nominal(&operands), 70_000);
    let negative = Operands {
        full: -100_000,
        damage: 0,
        ..operands
    };
    assert_eq!(nominal(&negative), -170_000);
    assert_eq!(delivered(&negative), -50_000);
}

#[test]
fn values_print_rounded_half_away_from_zero_and_colour_against_the_nominal() {
    assert_eq!(whole(250_000), 3);
    assert_eq!(whole(249_999), 2);
    assert_eq!(whole(-250_000), -3);
    assert_eq!(RowColour::of(-1, -5), RowColour::Red);
    assert_eq!(RowColour::of(4, 5), RowColour::Red);
    assert_eq!(RowColour::of(6, 5), RowColour::Green);
    assert_eq!(RowColour::of(5, 5), RowColour::Plain);
    assert_eq!(RowColour::Red.code(), Some('R'));
    assert_eq!(RowColour::Plain.code(), None);
}

#[test]
fn the_origin_route_kind_is_its_list_owners_class_zero_route() {
    with_hour(|hour| {
        let own = hour.amounts(MIXED, 1).unwrap().origin;
        assert_eq!(hour.route_kind(own), Ok(2));
        let rights = hour.amounts(MIXED, 0).unwrap().origin;
        assert!(hour.route_kind(rights).is_err());
    });
}

#[test]
fn a_recorded_resource_must_match_its_declaration() {
    let mut content = content();
    content.resources.swap(0, 1);
    let frame = frame();
    let refused = Hour::new(&frame, &content).err().unwrap();
    assert!(refused.contains("declaration 1"), "{refused}");
}
