use super::*;
use crate::combat::binding::NoIcons;

fn loc() -> Localisation {
    [
        (
            NAVAL_COMBAT_TOOLTIP_ATTACKER,
            "Attack $TCOUNT$ [$COUNTRIES$], HQ $HQCOUNT$",
        ),
        (
            NAVAL_COMBAT_TOOLTIP_ATTACKER_WITH_CONVOYS,
            "Attack $TCOUNT$, convoys $CONVOY_COUNT$ [$COUNTRIES$], HQ $HQCOUNT$",
        ),
        (
            NAVAL_COMBAT_TOOLTIP_DEFENDER,
            "Defend $TCOUNT$ [$COUNTRIES$], HQ $HQCOUNT$",
        ),
        (
            NAVAL_COMBAT_TOOLTIP_DEFENDER_WITH_CONVOYS,
            "Defend $TCOUNT$, convoys $CONVOY_COUNT$ [$COUNTRIES$], HQ $HQCOUNT$",
        ),
        ("COMBAT_UNDECIDED", "Undecided"),
        ("COMBAT_LAST_LESS_THAN_DAY", "Less than a day"),
        (NAVAL_COMBAT_CLICK_DESC, "Click to open; right-click to pan"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

#[test]
fn naval_marker_hover_builds_both_rosters_convoys_countries_hq_and_native_tail() {
    let mut detail = NavalCombatDetailsRow::default();
    detail.sides[0].units = vec![10, 11, 12];
    detail.sides[0].summary.countries = vec![7, 3];
    detail.sides[1].units = vec![20, 21];
    detail.sides[1].summary.countries = vec![8];
    let members: Vec<_> = [1, 16, 32]
        .map(|state| NavalCombatShipRow {
            state,
            side: 0,
            live_convoy: 1,
            ..Default::default()
        })
        .into();
    assert_eq!(
        seen(
            &NoIcons,
            &loc(),
            &detail,
            &members.iter().collect::<Vec<_>>(),
            4,
            [true, false],
            &|raw| Ok(format!("Country {raw}"))
        )
        .unwrap(),
        "Attack 3, convoys 2 [\u{11}YCountry 7\u{11}!, \u{11}YCountry 3\u{11}!], HQ 4\nDefend 2 [\u{11}YCountry 8\u{11}!], HQ 4\nUndecided\nLess than a day\nClick to open; right-click to pan"
    );
    let hits = [super::super::Hit {
        selection: super::super::Selection::Naval(19),
        rect: [1., 2., 3., 4.],
        layer: 0,
        order: 0,
        supported: true,
    }];
    assert_eq!(hit(&hits, [2., 3.]), Some((19, [1., 2., 3., 4.])));
    assert_eq!(hit(&hits, [4., 3.]), None);
}

#[test]
fn naval_marker_hq_is_first_owned_forward_force_and_one_count_for_both_sides() {
    let mut detail = NavalCombatDetailsRow::default();
    detail.sides[0].summary.countries = vec![7];
    detail.sides[1].summary.countries = vec![8];
    let members: Vec<_> = [1, 2, 1, 3]
        .map(|ship| NavalCombatShipRow {
            ship,
            live_ship: 1,
            ..Default::default()
        })
        .into();
    let ship_rows: Vec<_> = [1, 2, 3]
        .map(|id| ShipRow {
            id,
            task_force: id * 10,
            ..Default::default()
        })
        .into();
    let force_rows: Vec<_> = [10, 20, 30]
        .map(|id| TaskForceRow {
            id,
            roster_owner: if id == 10 { 8 } else { 7 },
            logical_owner: 8,
            ..Default::default()
        })
        .into();
    let input_rows: Vec<_> = [(10, 9), (20, 2), (30, 5)]
        .map(
            |(task_force, attached_headquarters)| TaskForceCounterInputRow {
                task_force,
                attached_headquarters,
                ..Default::default()
            },
        )
        .into();
    let ships = ship_rows.iter().map(|row| (row.id, row)).collect();
    let forces = force_rows.iter().map(|row| (row.id, row)).collect();
    let inputs = input_rows.iter().map(|row| (row.task_force, row)).collect();
    assert_eq!(
        hq_count(
            &detail,
            &members.iter().collect::<Vec<_>>(),
            70,
            &[(7, 70), (8, 80)].into_iter().collect(),
            &ships,
            &forces,
            &inputs
        ),
        Ok(2)
    );
}

#[test]
fn naval_marker_incomplete_tables_keep_the_recorded_gap_reason() {
    let table: Available<'_, tables::NavalCombatShips> = Available::Truncated {
        rows: &[],
        missing: 3,
    };
    assert!(
        whole(&table, "NavalCombatShips")
            .unwrap_err()
            .contains("truncated (3 rows unread)")
    );
    let table: Available<'_, tables::Ships> = Available::Unavailable {
        reason: "capture read failed",
    };
    assert!(
        whole(&table, "Ships")
            .unwrap_err()
            .contains("capture read failed")
    );
}
