use super::*;
use replay_data::rows::ShipRow;
use viewer_runtime::msvc_stable_sort::sort;

fn member(slot: u32, ship: u64, live_ship: u8, live_convoy: u8) -> NavalCombatShipRow {
    NavalCombatShipRow {
        combat: 71,
        side: 0,
        battle_slot: slot,
        member_source: 0,
        group_index: 0,
        member_index: slot,
        ship,
        live_ship,
        live_convoy,
        ..Default::default()
    }
}

fn sources<'a>(
    combat_ships: &'a [NavalCombatShipRow],
    ships: &'a [ShipRow],
    task_forces: &'a [TaskForceRow],
) -> Sources<'a> {
    Sources {
        combat_ships: Some(combat_ships),
        ships: Some(ShipCensus::from_rows(ships)),
        task_forces: Some(task_forces),
        counter_inputs: None,
        aggression: None,
    }
}

fn grid<'a>(
    source: Sources<'a>,
    vision: &Vision,
    owner: &impl Fn(u32) -> Option<u32>,
    allegiances: &BTreeMap<u32, Allegiance>,
) -> Result<[SideGrid<'a>; 2], &'static str> {
    derive(&Index::new(source), 71, vision, owner, allegiances)
}

fn ids(side: &SideGrid<'_>) -> Vec<Vec<u64>> {
    side.groups
        .iter()
        .map(|group| {
            group
                .task_forces
                .iter()
                .map(|force| force.force.id)
                .collect()
        })
        .collect()
}

fn force(id: u64, country: u32, visibility: u16) -> Candidate<'static> {
    Candidate {
        force: GridTaskForce {
            force: Box::leak(Box::new(TaskForceRow {
                id,
                ..Default::default()
            })),
            country,
            ships: 1,
            visibility,
        },
        allegiance: if country == 0 {
            Allegiance::Ours
        } else {
            Allegiance::Other
        },
        refused: Some(false),
    }
}

fn with(candidate: &Candidate<'static>, fleet: u64, mission: u32) -> Candidate<'static> {
    Candidate {
        force: GridTaskForce {
            force: Box::leak(Box::new(TaskForceRow {
                fleet,
                mission,
                ..candidate.force.force.clone()
            })),
            ..candidate.force.clone()
        },
        ..candidate.clone()
    }
}

#[test]
fn primary_live_ships_deduplicate_task_force_and_count_convoys_separately() {
    let combat_ships = vec![
        member(2, 202, 1, 0),
        member(0, 201, 1, 0),
        member(1, 0, 0, 1),
        NavalCombatShipRow {
            member_source: 1,
            ..member(3, 999, 1, 0)
        },
    ];
    let ships = vec![
        ShipRow {
            id: 201,
            task_force: 101,
            ..Default::default()
        },
        ShipRow {
            id: 202,
            task_force: 101,
            ..Default::default()
        },
    ];
    let task_forces = vec![TaskForceRow {
        id: 101,
        roster_owner: 1,
        mission: 2,
        ..Default::default()
    }];
    let sides = grid(
        sources(&combat_ships, &ships, &task_forces),
        &Vision::default(),
        &|raw| (raw == 1).then_some(0),
        &BTreeMap::from([(0, Allegiance::Ours)]),
    )
    .unwrap();
    assert_eq!(sides[0].convoys, 1);
    assert_eq!(ids(&sides[0]), vec![vec![101]]);
    assert_eq!(
        sides[0].groups[0]
            .task_forces
            .iter()
            .map(|force| force.ships)
            .collect::<Vec<_>>(),
        vec![2]
    );
    assert_eq!(sides[1], SideGrid::default());
}

#[test]
fn paired_only_groups_are_not_walked() {
    let paired = |slot, ship, live_ship, live_convoy, group_member| NavalCombatShipRow {
        group_index: PAIRED_ONLY_GROUP,
        member_index: group_member,
        ..member(slot, ship, live_ship, live_convoy)
    };
    let combat_ships = vec![
        member(0, 201, 1, 0),
        paired(1, 202, 1, 0, 0),
        paired(2, 0, 0, 1, 0),
    ];
    let ships: Vec<_> = [(201, 101), (202, 102)]
        .into_iter()
        .map(|(id, task_force)| ShipRow {
            id,
            task_force,
            ..Default::default()
        })
        .collect();
    let task_forces: Vec<_> = [101, 102]
        .into_iter()
        .map(|id| TaskForceRow {
            id,
            roster_owner: 1,
            ..Default::default()
        })
        .collect();
    let sides = grid(
        sources(&combat_ships, &ships, &task_forces),
        &Vision::default(),
        &Some,
        &BTreeMap::from([(1, Allegiance::Allied)]),
    )
    .unwrap();
    assert_eq!(ids(&sides[0]), vec![vec![101]]);
    assert_eq!(sides[0].convoys, 0);
    let sides = grid(
        sources(&combat_ships, &ships[..1], &task_forces[..1]),
        &Vision::default(),
        &Some,
        &BTreeMap::from([(1, Allegiance::Allied)]),
    )
    .unwrap();
    assert_eq!(ids(&sides[0]), vec![vec![101]]);
}

#[test]
fn incomplete_census_and_missing_live_join_do_not_turn_into_empty_grid() {
    let member = [member(0, 201, 1, 0)];
    let ship = [ShipRow {
        id: 201,
        task_force: 101,
        ..Default::default()
    }];
    let force = [TaskForceRow {
        id: 101,
        roster_owner: 1,
        ..Default::default()
    }];
    let run = |source| {
        grid(
            source,
            &Vision::default(),
            &|_| Some(0),
            &BTreeMap::from([(0, Allegiance::Ours)]),
        )
        .unwrap_err()
    };
    assert_eq!(
        run(Sources {
            ships: None,
            ..sources(&member, &ship, &force)
        }),
        "ship census is incomplete"
    );
    assert_eq!(
        run(sources(&member, &[], &force)),
        "live naval combat ship is absent from Ships"
    );
    assert_eq!(
        run(sources(&member, &ship, &[])),
        "live naval combat ship's task force is absent"
    );
    let repeated = [ship[0].clone(), ship[0].clone()];
    assert_eq!(
        run(sources(&member, &repeated, &force)),
        "ship census repeats a handle"
    );
}

#[test]
fn a_truncated_intel_census_refuses_an_unlisted_foreign_task_force() {
    let combat_ships = [member(0, 201, 1, 0), member(1, 202, 1, 0)];
    let ships: Vec<_> = [(201, 101), (202, 102)]
        .into_iter()
        .map(|(id, task_force)| ShipRow {
            id,
            task_force,
            ..Default::default()
        })
        .collect();
    let task_forces: Vec<_> = [(101, 0), (102, 2)]
        .into_iter()
        .map(|(id, roster_owner)| TaskForceRow {
            id,
            roster_owner,
            ..Default::default()
        })
        .collect();
    let allegiances = BTreeMap::from([(0, Allegiance::Ours), (2, Allegiance::Enemy)]);
    let run = |vision: &Vision| {
        grid(
            sources(&combat_ships, &ships, &task_forces),
            vision,
            &Some,
            &allegiances,
        )
    };
    let omitted = Vision::sharing(0, []).with_truncated_unit_intel([]);
    assert_eq!(
        run(&omitted).unwrap_err(),
        "a foreign task force's intel is unknown (the unit intel census is truncated)"
    );
    let listed = Vision::sharing(0, []).with_truncated_unit_intel([(102, 3)]);
    let sides = run(&listed).unwrap();
    assert_eq!(ids(&sides[0]), vec![vec![101], vec![102]]);
    assert_eq!(sides[0].groups[1].task_forces[0].visibility, 250);
    let shared = Vision::sharing(0, [2]).with_truncated_unit_intel([]);
    assert!(run(&shared).is_ok());
    let own = grid(
        sources(&combat_ships[..1], &ships, &task_forces),
        &omitted,
        &Some,
        &allegiances,
    )
    .unwrap();
    assert_eq!(ids(&own[0]), vec![vec![101]]);
}

#[test]
fn native_sort_puts_own_then_foreign_full_and_preserves_equal_priority() {
    assert!(compare(&force(1, 0, 255), &force(2, 2, 255), 0) < 0);
    assert!(compare(&force(1, 1, 250), &force(2, 2, 150), 0) < 0);
    assert_eq!(compare(&force(1, 1, 250), &force(2, 2, 255), 0), 0);
    let mut a = force(1, 0, 255);
    let b = force(2, 0, 255);
    a.refused = Some(true);
    assert!(compare(&a, &b, 0) < 0);
    a.refused = Some(false);
    assert!(compare(&with(&a, 0, 8), &b, 0) > 0);
    assert!(
        compare(
            &with(&a, (2_u64 << 32) | 1, 0),
            &with(&b, (1_u64 << 32) | 2, 0),
            0
        ) < 0
    );
    assert!(
        compare(
            &with(&a, u64::from(i32::MAX as u32), 0),
            &with(&b, u64::from(i32::MIN as u32), 0),
            0
        ) < 0
    );
}

#[test]
fn wrapping_fleet_handles_sort_as_the_game_sorts_them() {
    let handle = |low: i32| u64::from(low as u32) | (1 << 32);
    let pool: Vec<Candidate<'_>> = [0, i32::MAX, i32::MIN]
        .into_iter()
        .enumerate()
        .map(|(index, low)| {
            let mut candidate = with(&force(index as u64, 0, 255), handle(low), 2);
            candidate.refused = Some(false);
            candidate
        })
        .collect();
    let mut order: Vec<usize> = (0..pool.len()).collect();
    sort(&mut order, |&a, &b| compare(&pool[a], &pool[b], 0) < 0);
    assert_eq!(order, [2, 0, 1]);
}

#[test]
fn multiple_foreign_forces_form_contiguous_allegiance_runs_in_stable_order() {
    let combat_ships: Vec<_> = (1..=4)
        .map(|id| member(id, 200 + u64::from(id), 1, 0))
        .collect();
    let ships: Vec<_> = (1..=4)
        .map(|id| ShipRow {
            id: 200 + id,
            task_force: 100 + id,
            ..Default::default()
        })
        .collect();
    let task_forces: Vec<_> = (1..=4)
        .map(|id| TaskForceRow {
            id: 100 + id,
            roster_owner: id as u32,
            ..Default::default()
        })
        .collect();
    let sides = grid(
        sources(&combat_ships, &ships, &task_forces),
        &Vision::default(),
        &Some,
        &BTreeMap::from([
            (1, Allegiance::Allied),
            (2, Allegiance::Allied),
            (3, Allegiance::Enemy),
            (4, Allegiance::Other),
        ]),
    )
    .unwrap();
    assert_eq!(ids(&sides[0]), vec![vec![101, 102], vec![103], vec![104]]);
    assert_eq!(
        sides[0].groups[0]
            .task_forces
            .iter()
            .map(|force| force.ships)
            .collect::<Vec<_>>(),
        vec![1, 1]
    );
}

#[test]
fn native_run_split_is_asymmetric_when_coarse_sort_ties() {
    let combat_ships = [member(0, 201, 1, 0), member(1, 202, 1, 0)];
    let ships = [
        ShipRow {
            id: 201,
            task_force: 101,
            ..Default::default()
        },
        ShipRow {
            id: 202,
            task_force: 102,
            ..Default::default()
        },
    ];
    let task_forces = [
        TaskForceRow {
            id: 101,
            roster_owner: 1,
            ..Default::default()
        },
        TaskForceRow {
            id: 102,
            roster_owner: 2,
            ..Default::default()
        },
    ];
    let sides = grid(
        sources(&combat_ships, &ships, &task_forces),
        &Vision::default(),
        &Some,
        &BTreeMap::from([(1, Allegiance::Other), (2, Allegiance::Allied)]),
    )
    .unwrap();
    assert_eq!(ids(&sides[0]), vec![vec![101, 102]]);
}

#[test]
fn own_forces_sort_by_native_engagement_predicate_and_need_its_operands() {
    let combat_ships = [member(0, 202, 1, 0), member(1, 201, 1, 0)];
    let ships = [
        ShipRow {
            id: 201,
            task_force: 101,
            ..Default::default()
        },
        ShipRow {
            id: 202,
            task_force: 102,
            ..Default::default()
        },
    ];
    let task_forces = [
        TaskForceRow {
            id: 101,
            roster_owner: 1,
            mission: 2,
            ..Default::default()
        },
        TaskForceRow {
            id: 102,
            roster_owner: 1,
            mission: 2,
            ..Default::default()
        },
    ];
    let counter_inputs = [
        TaskForceCounterInputRow {
            task_force: 101,
            engagement_rule: 0,
            ..Default::default()
        },
        TaskForceCounterInputRow {
            task_force: 102,
            engagement_rule: 1,
            ..Default::default()
        },
    ];
    let vision = Vision::default();
    let owner = |raw| (raw == 1).then_some(0);
    let allegiances = BTreeMap::from([(0, Allegiance::Ours)]);
    assert_eq!(
        grid(
            sources(&combat_ships, &ships, &task_forces),
            &vision,
            &owner,
            &allegiances,
        )
        .unwrap_err(),
        "own naval sort needs complete engagement operands"
    );
    let sides = grid(
        Sources {
            counter_inputs: Some(&counter_inputs),
            aggression: Some(&[0, 60_000]),
            ..sources(&combat_ships, &ships, &task_forces)
        },
        &vision,
        &owner,
        &allegiances,
    )
    .unwrap();
    assert_eq!(ids(&sides[0]), vec![vec![101, 102]]);
}

#[test]
fn unknown_allegiance_is_not_assumed_other() {
    let combat_ships = [member(0, 201, 1, 0)];
    let ships = [ShipRow {
        id: 201,
        task_force: 101,
        ..Default::default()
    }];
    let task_forces = [TaskForceRow {
        id: 101,
        roster_owner: 1,
        ..Default::default()
    }];
    assert_eq!(
        grid(
            sources(&combat_ships, &ships, &task_forces),
            &Vision::default(),
            &Some,
            &BTreeMap::new(),
        )
        .unwrap_err(),
        "naval combat task-force allegiance is unresolved"
    );
}

#[test]
fn countries_share_one_index_across_combats() {
    const COUNTRIES: u32 = 40;
    const COMBATS: u64 = 12;
    let mut combat_ships = Vec::new();
    let mut ships = Vec::new();
    let mut task_forces = Vec::new();
    let mut counter_inputs = Vec::new();
    for country in 0..COUNTRIES {
        for (side, offset) in [(0_u8, 0_u64), (1, 1)] {
            let id = u64::from(country) * 2 + offset;
            let combat = if side == 0 {
                COMBATS
            } else {
                u64::from(country) % COMBATS
            };
            combat_ships.push(NavalCombatShipRow {
                combat,
                side,
                member_source: 0,
                group_index: country,
                member_index: 0,
                ship: 10_000_000 + id,
                live_ship: 1,
                ..Default::default()
            });
            ships.push(ShipRow {
                id: 10_000_000 + id,
                task_force: id,
                ..Default::default()
            });
            task_forces.push(TaskForceRow {
                id,
                fleet: (u64::from(country) << 32) | u64::from(country % 7),
                roster_owner: if side == 0 { 0 } else { country },
                mission: 2,
                ..Default::default()
            });
            counter_inputs.push(TaskForceCounterInputRow {
                task_force: id,
                ..Default::default()
            });
        }
    }
    let census = ShipCensus::from_rows(&ships);
    let index = Index::new(Sources {
        combat_ships: Some(&combat_ships),
        ships: Some(ShipCensus {
            task_forces: Cow::Borrowed(&census.task_forces),
            counts: Cow::Borrowed(&census.counts),
            rows: census.rows,
        }),
        task_forces: Some(&task_forces),
        counter_inputs: Some(&counter_inputs),
        aggression: Some(&[0, 60_000]),
    });
    let allegiances: BTreeMap<u32, Allegiance> = (0..COUNTRIES)
        .map(|country| {
            (
                country,
                if country == 0 {
                    Allegiance::Ours
                } else {
                    Allegiance::Enemy
                },
            )
        })
        .collect();
    let vision = Vision::default();
    let own = Index::new(Sources {
        combat_ships: Some(&combat_ships),
        ships: Some(ShipCensus::from_rows(&ships)),
        task_forces: Some(&task_forces),
        counter_inputs: Some(&counter_inputs),
        aggression: Some(&[0, 60_000]),
    });
    let mut drawn = 0;
    for combat in 0..=COMBATS {
        let sides = derive(&index, combat, &vision, &Some, &allegiances).unwrap();
        assert_eq!(
            derive(&own, combat, &vision, &Some, &allegiances).unwrap(),
            sides
        );
        drawn += sides
            .iter()
            .flat_map(|side| &side.groups)
            .map(|group| group.task_forces.len())
            .sum::<usize>();
    }
    assert_eq!(drawn, 2 * COUNTRIES as usize);
    let crowded = derive(&index, COMBATS, &vision, &Some, &allegiances).unwrap();
    let fleets: Vec<u64> = crowded[0]
        .groups
        .iter()
        .flat_map(|group| &group.task_forces)
        .map(|force| force.force.fleet)
        .collect();
    assert_eq!(fleets.len(), COUNTRIES as usize);
    assert!(fleets.is_sorted_by_key(|&fleet| signed_handle(fleet)));
}
