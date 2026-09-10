//! Native map-table conversion on the spool thread. Arena bytes and ordered
//! references stay exact; bad bounds or duplicate native identities fail a table.

use crate::{
    map_capture::MapCaptured,
    spooling::{table, unavailable},
    table_capture::CaptureRows,
};
use engine::{
    cair_wing, cfleet, chuman, cland_combat, cnaval_combat, cnaval_combatant, corder_instance,
    cship, ctask_force, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{
        AirWingRow, ArmyOrderRow, FleetRow, HumanControllerRow, LandCombatRow,
        NavalCombatParticipantRow, NavalCombatRow, PlayerRow, ShipRow, TaskForceRow, tables,
    },
};

pub(crate) fn tables(captured: &MapCaptured) -> [TableView; 10] {
    [
        players(
            &captured.players,
            &captured.player_names,
            captured.player_names_len,
        ),
        controllers(&captured.human_controllers),
        land_combats(
            &captured.land_combats,
            &captured.land_participants,
            captured.land_participants_len,
        ),
        fleets(&captured.fleets),
        task_forces(&captured.task_forces),
        ships(&captured.ships),
        naval_combats(&captured.naval_combats),
        naval_participants(&captured.naval_combat_participants),
        air_wings(&captured.air_wings),
        army_orders(
            &captured.army_orders,
            &captured.order_geometry,
            captured.order_geometry_len,
        ),
    ]
}

fn outcome<T>(captured: &CaptureRows<T>, mismatch_reason: &'static str) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: mismatch_reason,
        }
    } else {
        captured.outcome
    }
}

fn source<T>(captured: &CaptureRows<T>, outcome: Outcome) -> Result<&[T], TableView> {
    match outcome {
        Outcome::Unavailable { reason } => Err(unavailable(reason)),
        Outcome::Rows { len } | Outcome::Truncated { len, .. } => captured
            .rows
            .get(..len)
            .ok_or_else(|| unavailable("walker row count exceeds allocated buffer")),
    }
}

fn range<T>(arena: &[T], start: usize, len: usize) -> Option<&[T]> {
    arena.get(start..start.checked_add(len)?)
}

fn players(captured: &CaptureRows<chuman::Row>, names: &[u8], names_len: usize) -> TableView {
    let outcome = outcome(captured, "players changed during double read");
    let rows = match source(captured, outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    let Some(names) = names.get(..names_len) else {
        return unavailable("player bytes lie outside the captured byte arena");
    };
    if rows.iter().any(|row| {
        range(names, row.user_start, row.user_len).is_none()
            || range(names, row.name_start, row.name_len).is_none()
    }) {
        return unavailable("player bytes lie outside the captured byte arena");
    }
    table::<tables::Players, _>(outcome, rows, |row| PlayerRow {
        id: row.id,
        country: row.country,
        flags: row.flags,
        user: names[row.user_start..row.user_start + row.user_len].to_vec(),
        name: names[row.name_start..row.name_start + row.name_len].to_vec(),
    })
}

fn controllers(captured: &CaptureRows<chuman::ControllerRow>) -> TableView {
    table::<tables::HumanControllers, _>(
        outcome(captured, "human controllers changed during double read"),
        &captured.rows,
        |row| HumanControllerRow {
            country: row.country,
            count: row.count,
        },
    )
}

fn land_combats(
    captured: &CaptureRows<cland_combat::Row>,
    participants: &[u64],
    participants_len: usize,
) -> TableView {
    let outcome = outcome(captured, "land combats changed during double read");
    let rows = match source(captured, outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    let Some(participants) = participants.get(..participants_len) else {
        return unavailable("land combat members lie outside the captured reference arena");
    };
    let mut scratch = Vec::new();
    for row in rows {
        for (start, len) in [
            (row.attacker_start, row.attacker_len),
            (row.defender_start, row.defender_len),
        ] {
            let Some(members) = range(participants, start, len) else {
                return unavailable("land combat members lie outside the captured reference arena");
            };
            // 1413CC2B0 keeps each native side vector unique. Cross-side or
            // cross-combat exclusion is not established and is not imposed.
            scratch.clear();
            scratch.extend_from_slice(members);
            scratch.sort_unstable();
            if scratch.windows(2).any(|pair| pair[0] == pair[1]) {
                return unavailable("land combat side contains duplicate army references");
            }
        }
    }
    table::<tables::LandCombats, _>(outcome, rows, |row| LandCombatRow {
        id: row.id,
        province: row.province,
        duration: row.duration,
        day: row.day,
        attackers: participants[row.attacker_start..row.attacker_start + row.attacker_len].to_vec(),
        defenders: participants[row.defender_start..row.defender_start + row.defender_len].to_vec(),
    })
}

fn fleets(captured: &CaptureRows<cfleet::Row>) -> TableView {
    table::<tables::Fleets, _>(
        outcome(captured, "fleets changed during double read"),
        &captured.rows,
        |row| FleetRow {
            id: row.id,
            owner: row.owner,
            home_base: row.home_base,
        },
    )
}

fn task_forces(captured: &CaptureRows<ctask_force::Row>) -> TableView {
    table::<tables::TaskForces, _>(
        outcome(captured, "task forces changed during double read"),
        &captured.rows,
        |row| TaskForceRow {
            id: row.id,
            fleet: row.fleet,
            roster_owner: row.roster_owner,
            logical_owner: row.logical_owner,
            expeditionary_owner: row.expeditionary_owner,
            province: row.province,
            mission: row.mission,
        },
    )
}

fn ships(captured: &CaptureRows<cship::Row>) -> TableView {
    table::<tables::Ships, _>(
        outcome(captured, "ships changed during double read"),
        &captured.rows,
        |row| ShipRow {
            id: row.id,
            task_force: row.task_force,
            strength: row.strength,
            organisation: row.organisation,
            experience: row.experience,
            category: row.category,
            sunk_convoys: row.sunk_convoys,
            class_id: row.class_id,
        },
    )
}

fn naval_combats(captured: &CaptureRows<cnaval_combat::Row>) -> TableView {
    table::<tables::NavalCombats, _>(
        outcome(captured, "naval combats changed during double read"),
        &captured.rows,
        |row| NavalCombatRow {
            id: row.id,
            province: row.province,
            attacker_positioning: row.attacker_positioning,
            defender_positioning: row.defender_positioning,
            attacker_anti_air: row.attacker_anti_air,
            defender_anti_air: row.defender_anti_air,
        },
    )
}

fn naval_participants(captured: &CaptureRows<cnaval_combatant::Row>) -> TableView {
    let outcome = outcome(
        captured,
        "naval combat participants changed during double read",
    );
    let rows = match source(captured, outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    // Side is the selected native vector, not an arbitrary integer field. The
    // composite key reserves exactly one bit for this authenticated selection.
    if rows.iter().any(|row| row.side > 1) {
        return unavailable("naval combat participant has an invalid side");
    }
    table::<tables::NavalCombatParticipants, _>(outcome, rows, |row| NavalCombatParticipantRow {
        combat: row.combat,
        side: row.side,
        battle_slot: row.battle_slot,
        ship: row.ship,
        owner: row.owner,
        state: row.state,
    })
}

fn air_wings(captured: &CaptureRows<cair_wing::Row>) -> TableView {
    table::<tables::AirWings, _>(
        outcome(captured, "air wings changed during double read"),
        &captured.rows,
        |row| AirWingRow {
            id: row.id,
            owner: row.owner,
            country_override: row.country_override,
            air_base: row.air_base,
            base_override: row.base_override,
            planes: row.planes,
            effective_planes: row.effective_planes,
            mission_mask: row.mission_mask,
            mission_region: row.mission_region,
        },
    )
}

fn army_orders(
    captured: &CaptureRows<corder_instance::Row>,
    geometry: &[u32],
    geometry_len: usize,
) -> TableView {
    let outcome = outcome(captured, "army orders changed during double read");
    let rows = match source(captured, outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    let Some(geometry) = geometry.get(..geometry_len) else {
        return unavailable("army order geometry lies outside the captured arena");
    };
    if rows.iter().any(|row| {
        range(geometry, row.path_start, row.path_len).is_none()
            || range(geometry, row.target_start, row.target_len).is_none()
    }) {
        return unavailable("army order geometry lies outside the captured arena");
    }
    table::<tables::ArmyOrders, _>(outcome, rows, |row| ArmyOrderRow {
        owner_group: row.owner_group,
        instance: row.instance,
        country: row.country,
        order_type: row.order_type,
        active: row.active,
        path: geometry[row.path_start..row.path_start + row.path_len].to_vec(),
        target_states: geometry[row.target_start..row.target_start + row.target_len].to_vec(),
        paradrop_target: row.paradrop_target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    fn set<R>(captured: &mut CaptureRows<R>, rows: Vec<R>) {
        captured.outcome = Outcome::Rows { len: rows.len() };
        captured.rows = rows;
    }

    fn fixture() -> MapCaptured {
        let mut captured = MapCaptured::new();
        set(
            &mut captured.players,
            vec![chuman::Row {
                id: i32::MIN,
                country: u32::MAX,
                flags: 0xa5,
                user_start: 0,
                user_len: 2,
                name_start: 2,
                name_len: 1,
            }],
        );
        captured.player_names = vec![0, 0xff, 0xfe, 0x42];
        captured.player_names_len = 3;
        set(
            &mut captured.human_controllers,
            vec![chuman::ControllerRow {
                country: u32::MAX,
                count: i8::MIN,
            }],
        );
        set(
            &mut captured.land_combats,
            vec![cland_combat::Row {
                id: u64::MAX,
                province: u32::MAX,
                duration: i32::MIN,
                day: -1,
                attacker_start: 0,
                attacker_len: 2,
                defender_start: 2,
                defender_len: 1,
            }],
        );
        captured.land_participants = vec![u64::MAX, 0x100000002, u64::MAX];
        captured.land_participants_len = 3;
        set(
            &mut captured.fleets,
            vec![cfleet::Row {
                id: u64::MAX,
                owner: u32::MAX,
                home_base: u32::MAX - 1,
            }],
        );
        set(
            &mut captured.task_forces,
            vec![ctask_force::Row {
                id: u64::MAX,
                fleet: u64::MAX - 1,
                roster_owner: 1,
                logical_owner: 2,
                expeditionary_owner: 3,
                province: u32::MAX,
                mission: u32::MAX - 1,
            }],
        );
        set(
            &mut captured.ships,
            vec![cship::Row {
                id: u64::MAX,
                task_force: u64::MAX - 1,
                strength: i64::MIN,
                organisation: i64::MAX,
                experience: -123456789012345,
                category: u32::MAX,
                sunk_convoys: i32::MIN + 1,
                class_id: i32::MIN,
            }],
        );
        set(
            &mut captured.naval_combats,
            vec![cnaval_combat::Row {
                id: u64::MAX,
                province: u32::MAX,
                attacker_positioning: i64::MIN,
                defender_positioning: i64::MAX,
                attacker_anti_air: i64::MIN + 1,
                defender_anti_air: i64::MAX - 1,
            }],
        );
        set(
            &mut captured.naval_combat_participants,
            vec![cnaval_combatant::Row {
                combat: u64::MAX,
                side: 1,
                battle_slot: u32::MAX,
                ship: 0,
                owner: u32::MAX,
                state: u32::MAX - 1,
            }],
        );
        set(
            &mut captured.air_wings,
            vec![cair_wing::Row {
                id: u64::MAX,
                owner: u32::MAX,
                country_override: i32::MIN,
                air_base: u64::MAX - 1,
                base_override: u64::MAX - 2,
                planes: i32::MIN,
                effective_planes: i32::MAX,
                mission_mask: u32::MAX,
                mission_region: Some(0),
            }],
        );
        set(
            &mut captured.army_orders,
            vec![corder_instance::Row {
                owner_group: u64::MAX,
                instance: u32::MAX,
                country: u32::MAX,
                order_type: i32::MIN,
                active: 0xff,
                paradrop_target: u32::MAX - 1,
                path_start: 0,
                path_len: 3,
                target_start: 3,
                target_len: 2,
            }],
        );
        captured.order_geometry = vec![9, 2, 9, 0, u32::MAX];
        captured.order_geometry_len = 5;
        captured
    }

    #[test]
    fn all_map_tables_preserve_native_extremes_and_ordered_arena_values() {
        let expected = [
            TableRows::Players(vec![PlayerRow {
                id: i32::MIN,
                country: u32::MAX,
                flags: 0xa5,
                user: vec![0, 0xff],
                name: vec![0xfe],
            }]),
            TableRows::HumanControllers(vec![HumanControllerRow {
                country: u32::MAX,
                count: i8::MIN,
            }]),
            TableRows::LandCombats(vec![LandCombatRow {
                id: u64::MAX,
                province: u32::MAX,
                duration: i32::MIN,
                day: -1,
                attackers: vec![u64::MAX, 0x100000002],
                defenders: vec![u64::MAX],
            }]),
            TableRows::Fleets(vec![FleetRow {
                id: u64::MAX,
                owner: u32::MAX,
                home_base: u32::MAX - 1,
            }]),
            TableRows::TaskForces(vec![TaskForceRow {
                id: u64::MAX,
                fleet: u64::MAX - 1,
                roster_owner: 1,
                logical_owner: 2,
                expeditionary_owner: 3,
                province: u32::MAX,
                mission: u32::MAX - 1,
            }]),
            TableRows::Ships(vec![ShipRow {
                id: u64::MAX,
                task_force: u64::MAX - 1,
                strength: i64::MIN,
                organisation: i64::MAX,
                experience: -123456789012345,
                category: u32::MAX,
                sunk_convoys: i32::MIN + 1,
                class_id: i32::MIN,
            }]),
            TableRows::NavalCombats(vec![NavalCombatRow {
                id: u64::MAX,
                province: u32::MAX,
                attacker_positioning: i64::MIN,
                defender_positioning: i64::MAX,
                attacker_anti_air: i64::MIN + 1,
                defender_anti_air: i64::MAX - 1,
            }]),
            TableRows::NavalCombatParticipants(vec![NavalCombatParticipantRow {
                combat: u64::MAX,
                side: 1,
                battle_slot: u32::MAX,
                ship: 0,
                owner: u32::MAX,
                state: u32::MAX - 1,
            }]),
            TableRows::AirWings(vec![AirWingRow {
                id: u64::MAX,
                owner: u32::MAX,
                country_override: i32::MIN,
                air_base: u64::MAX - 1,
                base_override: u64::MAX - 2,
                planes: i32::MIN,
                effective_planes: i32::MAX,
                mission_mask: u32::MAX,
                mission_region: Some(0),
            }]),
            TableRows::ArmyOrders(vec![ArmyOrderRow {
                owner_group: u64::MAX,
                instance: u32::MAX,
                country: u32::MAX,
                order_type: i32::MIN,
                active: 0xff,
                path: vec![9, 2, 9],
                target_states: vec![0, u32::MAX],
                paradrop_target: u32::MAX - 1,
            }]),
        ]
        .map(TableView::Complete);
        assert_eq!(tables(&fixture()), expected);
    }

    #[test]
    fn arena_validation_uses_published_lengths_and_rejects_overflow() {
        let mut captured = fixture();
        captured.players.rows[0].name_start = 3;
        assert!(matches!(
            players(&captured.players, &captured.player_names, 3),
            TableView::Unavailable { .. }
        ));
        captured.players.rows[0].name_start = usize::MAX;
        assert!(matches!(
            players(&captured.players, &captured.player_names, 3),
            TableView::Unavailable { .. }
        ));
        captured.players.rows[0].name_start = 3;
        captured.players.rows[0].name_len = 0;
        assert!(matches!(
            players(&captured.players, &captured.player_names, 3),
            TableView::Complete(_)
        ));
        assert!(matches!(
            players(&captured.players, &captured.player_names, 5),
            TableView::Unavailable { .. }
        ));

        captured.land_combats.rows[0].defender_start = usize::MAX;
        assert!(matches!(
            land_combats(&captured.land_combats, &captured.land_participants, 3),
            TableView::Unavailable { .. }
        ));
        captured.land_combats.rows[0].defender_start = 2;
        assert!(matches!(
            land_combats(&captured.land_combats, &captured.land_participants, 2),
            TableView::Unavailable { .. }
        ));
        assert!(matches!(
            land_combats(&captured.land_combats, &captured.land_participants, 4),
            TableView::Unavailable { .. }
        ));

        captured.army_orders.rows[0].target_start = usize::MAX;
        assert!(matches!(
            army_orders(&captured.army_orders, &captured.order_geometry, 5),
            TableView::Unavailable { .. }
        ));
        captured.army_orders.rows[0].target_start = 3;
        assert!(matches!(
            army_orders(&captured.army_orders, &captured.order_geometry, 4),
            TableView::Unavailable { .. }
        ));
        assert!(matches!(
            army_orders(&captured.army_orders, &captured.order_geometry, 6),
            TableView::Unavailable { .. }
        ));
    }

    #[test]
    fn truncated_rows_keep_complete_arrays_and_ignore_unpublished_buffer_slots() {
        let mut captured = fixture();
        let complete = tables(&captured);
        captured.players.rows.push(chuman::Row {
            name_start: usize::MAX,
            name_len: 2,
            ..Default::default()
        });
        captured.land_combats.rows.push(cland_combat::Row {
            attacker_start: usize::MAX,
            attacker_len: 2,
            ..Default::default()
        });
        captured.army_orders.rows.push(corder_instance::Row {
            path_start: usize::MAX,
            path_len: 2,
            ..Default::default()
        });
        let truncated = Outcome::Truncated {
            len: 1,
            unread_rows: 17,
        };
        captured.players.outcome = truncated;
        captured.land_combats.outcome = truncated;
        captured.army_orders.outcome = truncated;
        let converted = tables(&captured);
        for index in [0, 2, 9] {
            let TableView::Complete(rows) = &complete[index] else {
                panic!("fixture incomplete")
            };
            assert_eq!(
                converted[index],
                TableView::Truncated {
                    rows: rows.clone(),
                    missing: 17
                }
            );
        }
        captured.army_orders.outcome = Outcome::Rows { len: 3 };
        assert_eq!(
            army_orders(&captured.army_orders, &captured.order_geometry, 5),
            unavailable("walker row count exceeds allocated buffer")
        );
    }

    #[test]
    fn land_reference_uniqueness_is_per_side_and_uses_all_handle_bits() {
        let mut captured = fixture();
        // Distinct full references with the same numeric id remain distinct.
        captured.land_participants = vec![0x100000002, 0x100000003, 0x100000002];
        assert!(matches!(
            land_combats(&captured.land_combats, &captured.land_participants, 3),
            TableView::Complete(_)
        ));
        captured.land_participants[1] = captured.land_participants[0];
        assert_eq!(
            land_combats(&captured.land_combats, &captured.land_participants, 3),
            unavailable("land combat side contains duplicate army references")
        );
        captured.land_combats.rows[0].attacker_len = 1;
        captured.land_combats.rows[0].defender_start = 0;
        captured.land_combats.rows[0].defender_len = 2;
        assert_eq!(
            land_combats(&captured.land_combats, &captured.land_participants, 3),
            unavailable("land combat side contains duplicate army references")
        );
    }

    #[test]
    fn consistency_failures_are_table_local_and_stale_mismatch_counts_are_ignored() {
        let mut captured = fixture();
        let before = tables(&captured);
        captured.ships.checked = true;
        captured.ships.mismatches = 1;
        captured.human_controllers.mismatches = usize::MAX;
        let after = tables(&captured);
        for index in 0..10 {
            if index == 5 {
                assert_eq!(
                    after[index],
                    unavailable("ships changed during double read")
                );
            } else {
                assert_eq!(after[index], before[index]);
            }
        }
        captured.players.outcome = Outcome::Unavailable {
            reason: "native player string unreadable",
        };
        captured.player_names_len = usize::MAX;
        assert_eq!(
            tables(&captured)[0],
            unavailable("native player string unreadable")
        );
    }

    #[test]
    fn composite_native_keys_retain_full_width_and_reject_duplicates_or_invalid_sides() {
        let mut captured = fixture();
        let member = captured.naval_combat_participants.rows[0];
        set(
            &mut captured.naval_combat_participants,
            vec![
                member,
                cnaval_combatant::Row { side: 0, ..member },
                cnaval_combatant::Row {
                    combat: member.combat - 1,
                    ..member
                },
                cnaval_combatant::Row {
                    battle_slot: member.battle_slot - 1,
                    ..member
                },
            ],
        );
        let TableView::Complete(rows) = naval_participants(&captured.naval_combat_participants)
        else {
            panic!("valid composite keys rejected")
        };
        assert_eq!(rows.len(), 4);
        let rows = rows.rows::<tables::NavalCombatParticipants>().unwrap();
        assert!(rows.windows(2).all(|pair| pair[0].key() < pair[1].key()));
        captured.naval_combat_participants.rows[1] = member;
        assert_eq!(
            naval_participants(&captured.naval_combat_participants),
            unavailable("walker produced duplicate engine ids")
        );
        captured.naval_combat_participants.rows[1].side = 2;
        assert_eq!(
            naval_participants(&captured.naval_combat_participants),
            unavailable("naval combat participant has an invalid side")
        );

        let order = captured.army_orders.rows[0];
        set(
            &mut captured.army_orders,
            vec![
                order,
                corder_instance::Row {
                    owner_group: order.owner_group - 1,
                    ..order
                },
            ],
        );
        assert!(
            matches!(army_orders(&captured.army_orders, &captured.order_geometry, 5), TableView::Complete(rows) if rows.len() == 2)
        );
        captured.army_orders.rows[1] = order;
        assert_eq!(
            army_orders(&captured.army_orders, &captured.order_geometry, 5),
            unavailable("walker produced duplicate engine ids")
        );
    }
}
