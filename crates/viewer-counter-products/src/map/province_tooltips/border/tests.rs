use super::*;
use replay_data::rows::*;
use replay_frame::{frame::Metrics, frame::TableView};

fn loc() -> Localisation {
    [
        ("BORDER_WAR_NOT_OURS", "Not ours"),
        ("BORDER_WAR_HEADER", "Border"),
        ("BORDER_WAR_POWER", "Our $VAL|5$"),
        ("BORDER_WAR_POWER_ENEMY", "Enemy $VAL|5$"),
        ("BORDER_WAR_POWER_RATIO", "Share $VAL|5$"),
        ("BORDER_WAR_WIN", "Win $VAL|5$"),
        ("BORDER_WAR_LOSS", "Loss $VAL|5$"),
        ("BORDER_WAR_HINT", "Hint"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}

fn fixture() -> (super::super::Content, DecodedFrame, BTreeMap<u32, u32>) {
    let content = super::super::Content {
        members: [
            (1, vec![101, 102, 103]),
            (2, vec![201, 202]),
            (3, vec![301]),
            (4, vec![401]),
        ]
        .into_iter()
        .collect(),
        labels: std::sync::Arc::new(
            viewer_runtime::recorded_labels::RecordedLabels::from_localisation(
                &Default::default(),
            ),
        ),
        impassable: Default::default(),
        neighbors: Ok([(1, vec![2, 3, 4])].into_iter().collect()),
        temperature: Err("unused".into()),
        pass: Err("unused".into()),
        derived: Default::default(),
        units: Err("unused".into()),
        border: Ok(Defines {
            org: 100_000,
            strength: 50_000,
            victory: 80_000,
        }),
        air_log_timeout: Err("unused".into()),
        air_detection: Err("unused".into()),
        air_range: Err("unused".into()),
        air_fuel: Err("unused".into()),
        navy: super::super::strategic_navy::Defines::default(),
        occupation: Err("unused".into()),
        network_base_target: Err("unused".into()),
        resources: Err("unused".into()),
    };
    let links = [(11, 1), (111, 1), (22, 2), (222, 2), (33, 3), (44, 4)]
        .into_iter()
        .collect();
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 60_000_000,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "unused".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    put(
        &mut frame,
        TableRows::Countries(
            [
                (11, 1, Some(1)),
                (22, 2, Some(2)),
                (33, 3, Some(3)),
                (44, 4, None),
            ]
            .into_iter()
            .map(
                |(id, canonical_slot, border_land_doctrine_level)| CountryRow {
                    id,
                    canonical_slot,
                    border_land_doctrine_level: replay_data::rows::Recorded::Value(
                        border_land_doctrine_level,
                    ),
                    ..Default::default()
                },
            )
            .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::States(
            [(1, 11), (2, 22), (3, 111), (4, 33)]
                .into_iter()
                .map(|(id, controller)| StateRow {
                    id,
                    controller,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::StateFlags(
            [1, 2, 3, 4]
                .into_iter()
                .map(|state| StateFlagRow {
                    state,
                    border_conflict_active: replay_data::rows::Recorded::Value(u8::from(
                        state != 4,
                    )),
                    border_conflict_countries: if state == 1 {
                        vec![[111, 22]].into()
                    } else {
                        vec![].into()
                    },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::Provinces(
            [
                (101, 1, 11),
                (102, 1, 111),
                (103, 1, 33),
                (201, 2, 22),
                (202, 2, 222),
                (301, 3, 111),
                (401, 4, 33),
            ]
            .into_iter()
            .map(|(id, state, controller)| ProvinceRow {
                id,
                state,
                controller,
                ..Default::default()
            })
            .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::Divisions(
            [
                (10, 11, 101, 100_000, 200_000, 7),
                (20, 111, 102, 0, 0, 0),
                (30, 22, 201, 200_000, 100_000, 0),
                (40, 22, 301, 0, 0, 0),
                (50, 22, 401, 0, 0, 0),
            ]
            .into_iter()
            .map(
                |(id, logical_owner, province, organisation, hit_points, orders_group)| {
                    DivisionRow {
                        id,
                        logical_owner,
                        province,
                        organisation,
                        hit_points,
                        orders_group,
                        ..Default::default()
                    }
                },
            )
            .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(
            [
                (10, Some([100_000; 3])),
                (20, Some([100_000, 0, 0])),
                (30, Some([200_000, 100_000, 0])),
                (40, None),
                (50, None),
            ]
            .into_iter()
            .map(|(division, border_combat_stats)| DivisionStatusRow {
                division,
                border_combat_stats: replay_data::rows::Recorded::Value(border_combat_stats),
                ..Default::default()
            })
            .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::ProvinceUnitOrder(
            [
                (101, vec![10, 999]),
                (102, vec![20]),
                (103, vec![]),
                (201, vec![30]),
                (202, vec![]),
                (301, vec![40]),
                (401, vec![50]),
            ]
            .into_iter()
            .map(|(province, mixed)| ProvinceUnitOrderRow {
                province,
                mixed,
                ..Default::default()
            })
            .collect(),
        ),
    );
    let leader = 5_u64 << 32 | 1;
    put(
        &mut frame,
        TableRows::OrdersGroups(vec![OrdersGroupRow {
            handle: 7,
            leader,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 5,
            reference_type: 1,
            skill_status: 1,
            skill_valid: 0,
            skill_level: 2,
            ..Default::default()
        }]),
    );
    (content, frame, links)
}

fn render(
    content: &super::super::Content,
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
) -> Result<String, String> {
    let flag = frame
        .complete::<tables::StateFlags>()
        .unwrap()
        .iter()
        .find(|flag| flag.state == 1)
        .unwrap();
    build(content, &loc(), frame, 1, links, 1, flag)
}

#[test]
fn native_sum_uses_raw_hp_skill_and_doctrine_on_whole_aggregates() {
    let (content, frame, links) = fixture();
    assert_eq!(
        render(&content, &frame, &links).unwrap(),
        "    Border        Our 0.03200\n        Enemy 0.12600\n\n        Share 0.20253\n        Win 0.80000\n        Loss 0.20000\n\n        Hint\n"
    );
}

#[test]
fn enemy_fraction_preserves_native_empty_and_overflow_branches() {
    assert_eq!(enemy_ratio([0, 0]), 50_000);
    assert_eq!(enemy_ratio([-2, 1]), 50_000);
    assert_eq!(enemy_ratio([i64::MAX, 0]), 100_000_i64 - 0xffff_ffff);
    assert_eq!(
        enemy_ratio([i64::MIN, i64::MIN + 1]),
        100_000_i64 - 0xffff_ffff
    );
}

#[test]
fn empty_provinces_demand_controller_doctrine_and_aliases_do_not_repeat_it() {
    let (mut content, mut frame, links) = fixture();
    let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
    countries
        .iter_mut()
        .find(|country| country.id == 33)
        .unwrap()
        .border_land_doctrine_level = replay_data::rows::Recorded::Value(None);
    put(&mut frame, TableRows::Countries(countries));
    assert!(
        render(&content, &frame, &links)
            .unwrap_err()
            .contains("selected controller 33 land-doctrine cache unavailable")
    );
    content.members.get_mut(&1).unwrap().retain(|&id| id != 103);
    let text = render(&content, &frame, &links).unwrap();
    assert!(text.contains("Our 0.03200"));
    assert!(text.contains("Enemy 0.03150"));
}

#[test]
fn demanded_stats_and_unread_skill_refuse_but_native_invalid_skill_is_read() {
    let (content, mut frame, links) = fixture();
    assert!(render(&content, &frame, &links).is_ok());
    let statuses = frame.complete::<tables::DivisionStatus>().unwrap().to_vec();
    let mut missing = statuses.clone();
    missing
        .iter_mut()
        .find(|status| status.division == 10)
        .unwrap()
        .border_combat_stats = replay_data::rows::Recorded::Value(None);
    put(&mut frame, TableRows::DivisionStatus(missing));
    assert!(
        render(&content, &frame, &links)
            .unwrap_err()
            .contains("selected border army 10 cached defense/soft/hard attack unavailable")
    );
    put(&mut frame, TableRows::DivisionStatus(statuses));
    let mut leaders = frame.complete::<tables::UnitLeaders>().unwrap().to_vec();
    leaders[0].skill_status = 0;
    put(&mut frame, TableRows::UnitLeaders(leaders));
    assert!(
        render(&content, &frame, &links)
            .unwrap_err()
            .contains("leader skill unavailable")
    );
}

#[test]
fn inactive_and_outsider_gates_do_not_require_power_operands() {
    let (content, mut frame, links) = fixture();
    let inactive = StateFlagRow {
        state: 1,
        ..Default::default()
    };
    frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "not selected".into(),
    }
    .into();
    assert_eq!(
        build(&content, &loc(), &frame, 1, &links, 1, &inactive).unwrap(),
        ""
    );
    let active = frame
        .complete::<tables::StateFlags>()
        .unwrap()
        .iter()
        .find(|flag| flag.state == 1)
        .unwrap();
    assert_eq!(
        build(&content, &loc(), &frame, 4, &links, 1, active).unwrap(),
        "    Not ours"
    );
}

#[test]
fn zero_total_uses_exact_half_share() {
    let (content, mut frame, links) = fixture();
    let mut rosters = frame
        .complete::<tables::ProvinceUnitOrder>()
        .unwrap()
        .to_vec();
    for roster in &mut rosters {
        roster.mixed.clear();
    }
    put(&mut frame, TableRows::ProvinceUnitOrder(rosters));
    let text = render(&content, &frame, &links).unwrap();
    assert!(text.contains("Our 0.00000\n        Enemy 0.00000"));
    assert!(text.contains("Share 0.50000"));
}

#[test]
fn missing_roster_is_not_a_successful_empty_province() {
    let (content, mut frame, links) = fixture();
    let mut rosters = frame
        .complete::<tables::ProvinceUnitOrder>()
        .unwrap()
        .to_vec();
    rosters.retain(|roster| roster.province != 103);
    put(&mut frame, TableRows::ProvinceUnitOrder(rosters));
    assert!(
        render(&content, &frame, &links)
            .unwrap_err()
            .contains("selected border province 103 unit roster unavailable")
    );
}
