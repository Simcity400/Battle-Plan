//! Native map-table conversion on the spool thread. Arena bytes and ordered
//! references stay exact; bad bounds or duplicate native identities fail a table.

use crate::{
    map_capture::MapCaptured,
    projection::{Projection, project_table, scalar_table},
    spooling::unavailable,
    table_capture::CaptureRows,
};
use engine::{
    cair_wing, cfleet, chuman, cland_combat, cnaval_combat, cnaval_combatant, corder_instance,
    cship, ctask_force, walk::Outcome,
};
#[cfg(test)]
use replay::frame::TableView;
use replay::{
    frame::TableInput,
    rows::{
        AirWingRow, ArmyOrderRow, FleetRow, HumanControllerRow, LandCombatRow,
        NavalCombatParticipantRow, NavalCombatRow, PlayerRow, ShipRow, TaskForceRow, tables,
    },
};

pub(crate) fn table_inputs(
    captured: &MapCaptured,
    mut projection: Option<&mut Projection<'_>>,
) -> [TableInput; 10] {
    let _phase = crate::allocation::enter(crate::allocation::Phase::Map);
    [
        players(
            &captured.players,
            &captured.player_names,
            captured.player_names_len,
            projection.as_deref_mut(),
        ),
        controllers(&captured.human_controllers, projection.as_deref_mut()),
        land_combats(
            &captured.land_combats,
            &captured.land_participants,
            &captured.land_classes,
            captured.land_participants_len,
            projection.as_deref_mut(),
        ),
        fleets(
            &captured.naval.fleets,
            &captured.naval.fleet_names,
            projection.as_deref_mut(),
        ),
        task_forces(&captured.naval.task_forces, projection.as_deref_mut()),
        ships(&captured.naval.ships, projection.as_deref_mut()),
        naval_combats(&captured.naval_combats, projection.as_deref_mut()),
        naval_participants(
            &captured.naval_combat_participants,
            projection.as_deref_mut(),
        ),
        air_wings(
            &captured.air_wings,
            &captured.air_wing_names,
            captured.air_wing_names_len,
            projection.as_deref_mut(),
        ),
        army_orders(
            &captured.orders.rows,
            &captured.orders.geometry,
            captured.orders.geometry_len,
            projection,
        ),
    ]
}

fn source<T>(captured: &CaptureRows<T>, outcome: Outcome) -> Result<&[T], TableInput> {
    match outcome {
        Outcome::Unavailable { reason } => Err(unavailable(reason).into()),
        Outcome::Rows { len } | Outcome::Truncated { len, .. } => captured
            .rows
            .get(..len)
            .ok_or_else(|| unavailable("walker row count exceeds allocated buffer").into()),
    }
}

fn range<T>(arena: &[T], start: usize, len: usize) -> Option<&[T]> {
    arena.get(start..start.checked_add(len)?)
}

fn players(
    captured: &CaptureRows<chuman::Row>,
    names: &[u8],
    names_len: usize,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    let outcome = captured.outcome;
    let rows = match source(captured, outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    let Some(names) = names.get(..names_len) else {
        return unavailable("player bytes lie outside the captured byte arena").into();
    };
    if rows.iter().any(|row| {
        range(names, row.user_start, row.user_len).is_none()
            || range(names, row.name_start, row.name_len).is_none()
    }) {
        return unavailable("player bytes lie outside the captured byte arena").into();
    }
    project_table::<tables::Players, _>(
        projection,
        outcome,
        rows,
        |row| u128::from(row.id as u32),
        |row, previous| {
            row.id == previous.id
                && row.country == previous.country
                && row.flags == previous.flags
                && names[row.user_start..row.user_start + row.user_len] == previous.user
                && names[row.name_start..row.name_start + row.name_len] == previous.name
        },
        |row| PlayerRow {
            id: row.id,
            country: row.country,
            flags: row.flags,
            user: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &names[row.user_start..row.user_start + row.user_len],
            )),
            name: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &names[row.name_start..row.name_start + row.name_len],
            )),
        },
    )
}

fn controllers(
    captured: &CaptureRows<chuman::ControllerRow>,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    scalar_table::<tables::HumanControllers, _>(
        projection,
        captured.outcome,
        &captured.rows,
        |row| u128::from(row.country),
        |row| HumanControllerRow {
            country: row.country,
            count: row.count,
        },
    )
}

fn land_combats(
    captured: &CaptureRows<cland_combat::Row>,
    participants: &[u64],
    classes: &[u32],
    participants_len: usize,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    let outcome = captured.outcome;
    let rows = match source(captured, outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    let (Some(participants), Some(classes)) = (
        participants.get(..participants_len),
        classes.get(..participants_len),
    ) else {
        return unavailable("land combat members lie outside the captured reference arena").into();
    };
    let mut scratch = Vec::new();
    let maximum = rows
        .iter()
        .map(|row| row.attacker_len.max(row.defender_len))
        .max()
        .unwrap_or(0);
    if engine::allocation::reserve_exact(&mut scratch, maximum).is_err() {
        return unavailable("collector allocation failed").into();
    }
    for row in rows {
        for (start, len) in [
            (row.attacker_start, row.attacker_len),
            (row.defender_start, row.defender_len),
        ] {
            let Some(members) = range(participants, start, len) else {
                return unavailable("land combat members lie outside the captured reference arena")
                    .into();
            };
            // Each native side vector keeps unique members. Cross-side or
            // cross-combat exclusion is not established and is not imposed.
            scratch.clear();
            scratch.extend_from_slice(members);
            scratch.sort_unstable();
            if scratch.windows(2).any(|pair| pair[0] == pair[1]) {
                return unavailable("land combat side contains duplicate army references").into();
            }
        }
    }
    project_table::<tables::LandCombats, _>(
        projection,
        outcome,
        rows,
        |row| u128::from(row.id),
        |row, previous| {
            row.id == previous.id
                && row.province == previous.province
                && row.duration == previous.duration
                && row.day == previous.day
                && participants[row.attacker_start..row.attacker_start + row.attacker_len]
                    == previous.attackers
                && participants[row.defender_start..row.defender_start + row.defender_len]
                    == previous.defenders
                && classes[row.attacker_start..row.attacker_start + row.attacker_len]
                    == previous.attacker_classes
                && classes[row.defender_start..row.defender_start + row.defender_len]
                    == previous.defender_classes
        },
        |row| LandCombatRow {
            id: row.id,
            province: row.province,
            duration: row.duration,
            day: row.day,
            attackers: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &participants[row.attacker_start..row.attacker_start + row.attacker_len],
            )),
            defenders: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &participants[row.defender_start..row.defender_start + row.defender_len],
            )),
            attacker_classes: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &classes[row.attacker_start..row.attacker_start + row.attacker_len],
            )),
            defender_classes: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &classes[row.defender_start..row.defender_start + row.defender_len],
            )),
        },
    )
}

fn fleets(
    captured: &CaptureRows<cfleet::Row>,
    names: &[u8],
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    scalar_table::<tables::Fleets, _>(
        projection,
        captured.outcome,
        &captured.rows,
        |row| u128::from(row.id),
        |row| FleetRow {
            id: row.id,
            owner: row.owner,
            home_base: row.home_base,
            name: if row.name_known {
                range(names, row.name_start, row.name_len)
                    .map(|bytes| engine::allocation::owned_vec(engine::allocation::copy_vec(bytes)))
            } else {
                None
            }
            .into(),
            leader: row.leader.into(),
        },
    )
}

fn task_forces(
    captured: &CaptureRows<ctask_force::Row>,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    scalar_table::<tables::TaskForces, _>(
        projection,
        captured.outcome,
        &captured.rows,
        |row| u128::from(row.id),
        |row| TaskForceRow {
            id: row.id,
            fleet: row.fleet,
            roster_owner: row.roster_owner,
            logical_owner: row.logical_owner,
            expeditionary_owner: row.expeditionary_owner,
            province: row.province,
            mission: row.mission,
            route_count: row.route_count,
            combat_references: row.combat_references,
        },
    )
}

fn ships(
    captured: &CaptureRows<cship::Row>,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    scalar_table::<tables::Ships, _>(
        projection,
        captured.outcome,
        &captured.rows,
        |row| u128::from(row.id),
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

fn naval_combats(
    captured: &CaptureRows<cnaval_combat::Row>,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    scalar_table::<tables::NavalCombats, _>(
        projection,
        captured.outcome,
        &captured.rows,
        |row| u128::from(row.id),
        |row| NavalCombatRow {
            id: row.id,
            province: row.province,
            attacker_positioning: row.attacker_positioning,
            defender_positioning: row.defender_positioning,
            attacker_anti_air: row.attacker_anti_air,
            defender_anti_air: row.defender_anti_air,
            registry_ordinal: row.registry_ordinal,
        },
    )
}

fn naval_participants(
    captured: &CaptureRows<cnaval_combatant::Row>,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    let outcome = captured.outcome;
    let rows = match source(captured, outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    // Side is the selected native vector, not an arbitrary integer field. The
    // composite key reserves exactly one bit for this authenticated selection.
    if rows.iter().any(|row| row.side > 1) {
        return unavailable("naval combat participant has an invalid side").into();
    }
    scalar_table::<tables::NavalCombatParticipants, _>(
        projection,
        outcome,
        rows,
        |row| {
            (u128::from(row.combat) << 33)
                | (u128::from(row.side) << 32)
                | u128::from(row.battle_slot)
        },
        |row| NavalCombatParticipantRow {
            combat: row.combat,
            side: row.side,
            battle_slot: row.battle_slot,
            ship: row.ship,
            owner: row.owner,
            state: row.state,
        },
    )
}

fn air_wings(
    captured: &CaptureRows<cair_wing::Row>,
    names: &[u8],
    names_len: usize,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    let rows = match source(captured, captured.outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    let names = names.get(..names_len);
    // Invalid name spans refuse this field only, retaining the old columns.
    let name = |row: &cair_wing::Row| {
        names.and_then(|bytes| row.name.and_then(|(start, len)| range(bytes, start, len)))
    };
    let scalar = |row: &cair_wing::Row| AirWingRow {
        id: row.id,
        owner: row.owner,
        country_override: row.country_override,
        air_base: row.air_base,
        base_override: row.base_override,
        planes: row.planes,
        mission_mask: row.mission_mask,
        mission_region: row.mission_region,
        base_range_raw: row.base_range_raw,
        ace: row.ace,
        mission_active: row.mission_active,
        eligibility_veto: row.eligibility_veto,
        eligibility_counter: row.eligibility_counter,
        eligibility_limit: row.eligibility_limit,
        attached_orders_group: row.attached_orders_group,
        pool_ordinal: row.pool_ordinal,
        name: replay::rows::Recorded::Value(None),
        strategic_bombing_raw: row.strategic_bombing_raw.into(),
        pool_subunit: row.pool_subunit.into(),
        raid_reference: replay::rows::Recorded::Value(row.raid_reference.map(|value| {
            replay::rows::AirWingRaidReference {
                raw: value.raw,
                resolves: value.resolves,
            }
        })),
    };
    project_table::<tables::AirWings, _>(
        projection,
        captured.outcome,
        rows,
        |row| u128::from(row.id),
        |row, previous| {
            row.id == previous.id
                && row.owner == previous.owner
                && row.country_override == previous.country_override
                && row.air_base == previous.air_base
                && row.base_override == previous.base_override
                && row.planes == previous.planes
                && row.mission_mask == previous.mission_mask
                && row.mission_region == previous.mission_region
                && row.base_range_raw == previous.base_range_raw
                && row.ace == previous.ace
                && row.mission_active == previous.mission_active
                && row.eligibility_veto == previous.eligibility_veto
                && row.eligibility_counter == previous.eligibility_counter
                && row.eligibility_limit == previous.eligibility_limit
                && row.attached_orders_group == previous.attached_orders_group
                && row.pool_ordinal == previous.pool_ordinal
                && previous.strategic_bombing_raw
                    == replay::rows::Recorded::Value(row.strategic_bombing_raw)
                && previous.pool_subunit == replay::rows::Recorded::Value(row.pool_subunit)
                && previous.raid_reference
                    == replay::rows::Recorded::Value(row.raid_reference.map(|value| {
                        replay::rows::AirWingRaidReference {
                            raw: value.raw,
                            resolves: value.resolves,
                        }
                    }))
                && matches!(&previous.name, replay::rows::Recorded::Value(value) if value.as_deref() == name(row))
        },
        |row| AirWingRow {
            name: replay::rows::Recorded::Value(
                name(row).map(|bytes| {
                    engine::allocation::owned_vec(engine::allocation::copy_vec(bytes))
                }),
            ),
            ..scalar(row)
        },
    )
}

fn army_orders(
    captured: &CaptureRows<corder_instance::Row>,
    geometry: &[u32],
    geometry_len: usize,
    projection: Option<&mut Projection<'_>>,
) -> TableInput {
    let outcome = captured.outcome;
    let rows = match source(captured, outcome) {
        Ok(rows) => rows,
        Err(failure) => return failure,
    };
    let Some(geometry) = geometry.get(..geometry_len) else {
        return unavailable("army order geometry lies outside the captured arena").into();
    };
    if rows.iter().any(|row| {
        range(geometry, row.path_start, row.path_len).is_none()
            || range(geometry, row.target_start, row.target_len).is_none()
    }) {
        return unavailable("army order geometry lies outside the captured arena").into();
    }
    project_table::<tables::ArmyOrders, _>(
        projection,
        outcome,
        rows,
        |row| (u128::from(row.owner_group) << 32) | u128::from(row.instance),
        |row, previous| {
            row.owner_group == previous.owner_group
                && row.instance == previous.instance
                && row.country == previous.country
                && row.order_type == previous.order_type
                && row.active == previous.active
                && row.paradrop_target == previous.paradrop_target
                && geometry[row.path_start..row.path_start + row.path_len] == previous.path
                && geometry[row.target_start..row.target_start + row.target_len]
                    == previous.target_states
        },
        |row| ArmyOrderRow {
            owner_group: row.owner_group,
            instance: row.instance,
            country: row.country,
            order_type: row.order_type,
            active: row.active,
            path: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &geometry[row.path_start..row.path_start + row.path_len],
            )),
            target_states: engine::allocation::owned_vec(engine::allocation::copy_vec(
                &geometry[row.target_start..row.target_start + row.target_len],
            )),
            paradrop_target: row.paradrop_target,
        },
    )
}

#[cfg(test)]
fn sampled(input: TableInput) -> TableView {
    match input {
        TableInput::Sampled(view) => view,
        _ => panic!("fixture has no comparison baseline"),
    }
}

#[cfg(test)]
fn tables(captured: &MapCaptured) -> [TableView; 10] {
    table_inputs(captured, None).map(sampled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::TableRows;

    #[test]
    fn stored_air_wing_name_projection_preserves_bytes_and_refuses_only_bad_spans() {
        let mut captured = fixture();
        captured.air_wing_names = vec![0xff, 0, b'W'];
        captured.air_wing_names_len = 3;
        for (span, expected) in [
            (Some((0, 3)), Some(vec![0xff, 0, b'W'])),
            (Some((3, 0)), Some(Vec::new())),
            (Some((2, 2)), None),
            (None, None),
        ] {
            captured.air_wings.rows[0].name = span;
            let TableView::Complete(TableRows::AirWings(rows)) = sampled(air_wings(
                &captured.air_wings,
                &captured.air_wing_names,
                captured.air_wing_names_len,
                None,
            )) else {
                panic!("complete wing census")
            };
            assert_eq!(rows[0].name, replay::rows::Recorded::Value(expected));
            assert_eq!(
                (rows[0].planes, rows[0].owner, rows[0].id),
                (i32::MIN, u32::MAX, u64::MAX)
            );
        }
    }

    #[test]
    fn native_projection_preserves_all_map_fields_and_current_arena_failures() {
        use crate::projection::ProjectionScratch;
        use replay::{
            frame::{FrameEncoder, FrameInput, Metrics},
            rows::TableId,
        };
        let map_tables = [
            TableId::Players,
            TableId::HumanControllers,
            TableId::LandCombats,
            TableId::Fleets,
            TableId::TaskForces,
            TableId::Ships,
            TableId::NavalCombats,
            TableId::NavalCombatParticipants,
            TableId::AirWings,
            TableId::ArmyOrders,
        ];
        let frame = |hour, map: [TableInput; 10]| {
            let mut input = FrameInput {
                branch: 1,
                hour,
                metrics: Metrics::default(),
                tables: std::array::from_fn(|_| unavailable("fixture table").into()),
            };
            for (id, rows) in map_tables.into_iter().zip(map) {
                input.tables[id.index()] = rows;
            }
            input
        };
        let mut captured = fixture();
        let mut encoder = FrameEncoder::new();
        let mut ordinary = FrameEncoder::new();
        let scratch = ProjectionScratch::default();
        for hour in 0..3 {
            if hour == 2 {
                captured.player_names[0] ^= 1;
                captured.orders.geometry[0] ^= 1;
                captured.air_wings.rows[0].strategic_bombing_raw = Some(0);
            }
            let current = table_inputs(
                &captured,
                Some(&mut Projection::new(&encoder, 1, hour, false, &scratch)),
            );
            for (index, table) in current.iter().enumerate() {
                let reused = hour == 1 || (hour == 2 && !matches!(index, 0 | 8 | 9));
                assert_eq!(
                    matches!(table, TableInput::ComparedEqual(_)),
                    reused,
                    "map table {index} hour {hour}"
                );
            }
            let expected = table_inputs(&captured, None);
            assert_eq!(
                encoder.encode_input(frame(hour, current), false).unwrap(),
                ordinary.encode_input(frame(hour, expected), false).unwrap()
            );
        }
        captured.player_names_len = captured.player_names.len() + 1;
        captured.orders.geometry_len = captured.orders.geometry.len() + 1;
        captured.naval.ships.outcome = Outcome::Unavailable {
            reason: "native ship unreadable",
        };
        let rejected = table_inputs(
            &captured,
            Some(&mut Projection::new(&encoder, 1, 3, false, &scratch)),
        );
        for index in [0, 5, 9] {
            assert!(matches!(
                rejected[index],
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
    }

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
        captured.land_classes = vec![0, 13, 1];
        captured.land_participants_len = 3;
        captured.naval.fleet_names = b"Fleet".to_vec();
        set(
            &mut captured.naval.fleets,
            vec![cfleet::Row {
                id: u64::MAX,
                owner: u32::MAX,
                home_base: u32::MAX - 1,
                leader: None,
                name_start: 0,
                name_len: 5,
                name_known: true,
            }],
        );
        set(
            &mut captured.naval.task_forces,
            vec![ctask_force::Row {
                id: u64::MAX,
                fleet: u64::MAX - 1,
                roster_owner: 1,
                logical_owner: 2,
                expeditionary_owner: 3,
                province: u32::MAX,
                mission: u32::MAX - 1,
                route_count: -1,
                combat_references: i32::MIN,
            }],
        );
        set(
            &mut captured.naval.ships,
            vec![cship::Row {
                roster_ordinal: 0,
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
                registry_ordinal: u32::MAX,
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
                pool_ordinal: u32::MAX - 3,
                country_override: i32::MIN,
                air_base: u64::MAX - 1,
                base_override: u64::MAX - 2,
                planes: i32::MIN,
                mission_mask: u32::MAX,
                mission_region: Some(0),
                base_range_raw: i64::MIN,
                ace: u64::MAX,
                mission_active: 1,
                eligibility_veto: u8::MAX,
                eligibility_counter: i32::MIN,
                eligibility_limit: i32::MAX,
                attached_orders_group: Some(u64::MAX),
                name: None,
                strategic_bombing_raw: Some(i64::MIN),
                pool_subunit: Some(u32::MAX),
                raid_reference: Some(cair_wing::transport_operands::RaidReference {
                    raw: u64::MAX,
                    resolves: false,
                }),
            }],
        );
        set(
            &mut captured.orders.rows,
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
        captured.orders.geometry = vec![9, 2, 9, 0, u32::MAX];
        captured.orders.geometry_len = 5;
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
                attacker_classes: vec![0, 13],
                defender_classes: vec![1],
            }]),
            TableRows::Fleets(vec![FleetRow {
                id: u64::MAX,
                owner: u32::MAX,
                home_base: u32::MAX - 1,
                name: replay::rows::Recorded::Value(Some(b"Fleet".to_vec())),
                leader: replay::rows::Recorded::Value(None),
            }]),
            TableRows::TaskForces(vec![TaskForceRow {
                id: u64::MAX,
                fleet: u64::MAX - 1,
                roster_owner: 1,
                logical_owner: 2,
                expeditionary_owner: 3,
                province: u32::MAX,
                mission: u32::MAX - 1,
                route_count: -1,
                combat_references: i32::MIN,
            }]),
            TableRows::Ships(vec![ShipRow {
                id: u64::MAX,
                task_force: u64::MAX - 1,
                strength: i64::MIN,
                organisation: i64::MAX,
                // Stored at whole hundredths (ShipRow::EXPERIENCE_STEP),
                // truncated toward zero.
                experience: -123_456_789_012_000,
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
                registry_ordinal: u32::MAX,
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
                pool_ordinal: u32::MAX - 3,
                country_override: i32::MIN,
                air_base: u64::MAX - 1,
                base_override: u64::MAX - 2,
                planes: i32::MIN,
                mission_mask: u32::MAX,
                mission_region: Some(0),
                base_range_raw: i64::MIN,
                ace: u64::MAX,
                mission_active: 1,
                eligibility_veto: u8::MAX,
                eligibility_counter: i32::MIN,
                eligibility_limit: i32::MAX,
                attached_orders_group: Some(u64::MAX),
                name: replay::rows::Recorded::Value(None),
                strategic_bombing_raw: replay::rows::Recorded::Value(Some(i64::MIN)),
                pool_subunit: replay::rows::Recorded::Value(Some(u32::MAX)),
                raid_reference: replay::rows::Recorded::Value(Some(
                    replay::rows::AirWingRaidReference {
                        raw: u64::MAX,
                        resolves: false,
                    },
                )),
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
            sampled(players(&captured.players, &captured.player_names, 3, None)),
            TableView::Unavailable { .. }
        ));
        captured.players.rows[0].name_start = usize::MAX;
        assert!(matches!(
            sampled(players(&captured.players, &captured.player_names, 3, None)),
            TableView::Unavailable { .. }
        ));
        captured.players.rows[0].name_start = 3;
        captured.players.rows[0].name_len = 0;
        assert!(matches!(
            sampled(players(&captured.players, &captured.player_names, 3, None)),
            TableView::Complete(_)
        ));
        assert!(matches!(
            sampled(players(&captured.players, &captured.player_names, 5, None)),
            TableView::Unavailable { .. }
        ));

        captured.land_combats.rows[0].defender_start = usize::MAX;
        assert!(matches!(
            sampled(land_combats(
                &captured.land_combats,
                &captured.land_participants,
                &captured.land_classes,
                3,
                None
            )),
            TableView::Unavailable { .. }
        ));
        captured.land_combats.rows[0].defender_start = 2;
        captured.land_classes.pop();
        assert!(matches!(
            sampled(land_combats(
                &captured.land_combats,
                &captured.land_participants,
                &captured.land_classes,
                3,
                None
            )),
            TableView::Unavailable { .. }
        ));
        captured.land_classes.push(0);
        assert!(matches!(
            sampled(land_combats(
                &captured.land_combats,
                &captured.land_participants,
                &captured.land_classes,
                2,
                None
            )),
            TableView::Unavailable { .. }
        ));
        assert!(matches!(
            sampled(land_combats(
                &captured.land_combats,
                &captured.land_participants,
                &captured.land_classes,
                4,
                None
            )),
            TableView::Unavailable { .. }
        ));

        captured.orders.rows.rows[0].target_start = usize::MAX;
        assert!(matches!(
            sampled(army_orders(
                &captured.orders.rows,
                &captured.orders.geometry,
                5,
                None
            )),
            TableView::Unavailable { .. }
        ));
        captured.orders.rows.rows[0].target_start = 3;
        assert!(matches!(
            sampled(army_orders(
                &captured.orders.rows,
                &captured.orders.geometry,
                4,
                None
            )),
            TableView::Unavailable { .. }
        ));
        assert!(matches!(
            sampled(army_orders(
                &captured.orders.rows,
                &captured.orders.geometry,
                6,
                None
            )),
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
        captured.orders.rows.rows.push(corder_instance::Row {
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
        captured.orders.rows.outcome = truncated;
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
        captured.orders.rows.outcome = Outcome::Rows { len: 3 };
        assert_eq!(
            sampled(army_orders(
                &captured.orders.rows,
                &captured.orders.geometry,
                5,
                None
            )),
            unavailable("walker row count exceeds allocated buffer")
        );
    }

    #[test]
    fn land_reference_uniqueness_is_per_side_and_uses_all_handle_bits() {
        let mut captured = fixture();
        // Distinct full references with the same numeric id remain distinct.
        captured.land_participants = vec![0x100000002, 0x100000003, 0x100000002];
        assert!(matches!(
            sampled(land_combats(
                &captured.land_combats,
                &captured.land_participants,
                &captured.land_classes,
                3,
                None
            )),
            TableView::Complete(_)
        ));
        captured.land_participants[1] = captured.land_participants[0];
        assert_eq!(
            sampled(land_combats(
                &captured.land_combats,
                &captured.land_participants,
                &captured.land_classes,
                3,
                None
            )),
            unavailable("land combat side contains duplicate army references")
        );
        captured.land_combats.rows[0].attacker_len = 1;
        captured.land_combats.rows[0].defender_start = 0;
        captured.land_combats.rows[0].defender_len = 2;
        assert_eq!(
            sampled(land_combats(
                &captured.land_combats,
                &captured.land_participants,
                &captured.land_classes,
                3,
                None
            )),
            unavailable("land combat side contains duplicate army references")
        );
    }

    #[test]
    fn primary_failures_are_table_local_and_precede_arena_validation() {
        let mut captured = fixture();
        let before = tables(&captured);
        captured.naval.ships.outcome = Outcome::Unavailable {
            reason: "native ship unreadable",
        };
        let after = tables(&captured);
        for index in 0..10 {
            if index == 5 {
                assert_eq!(after[index], unavailable("native ship unreadable"));
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
        let TableView::Complete(rows) = sampled(naval_participants(
            &captured.naval_combat_participants,
            None,
        )) else {
            panic!("valid composite keys rejected")
        };
        assert_eq!(rows.len(), 4);
        let rows = rows.rows::<tables::NavalCombatParticipants>().unwrap();
        assert!(rows.windows(2).all(|pair| pair[0].key() < pair[1].key()));
        captured.naval_combat_participants.rows[1] = member;
        assert_eq!(
            sampled(naval_participants(
                &captured.naval_combat_participants,
                None
            )),
            unavailable("walker produced duplicate engine ids")
        );
        captured.naval_combat_participants.rows[1].side = 2;
        assert_eq!(
            sampled(naval_participants(
                &captured.naval_combat_participants,
                None
            )),
            unavailable("naval combat participant has an invalid side")
        );

        let order = captured.orders.rows.rows[0];
        set(
            &mut captured.orders.rows,
            vec![
                order,
                corder_instance::Row {
                    owner_group: order.owner_group - 1,
                    ..order
                },
            ],
        );
        assert!(
            matches!(sampled(army_orders(&captured.orders.rows, &captured.orders.geometry, 5, None)), TableView::Complete(rows) if rows.len() == 2)
        );
        captured.orders.rows.rows[1] = order;
        assert_eq!(
            sampled(army_orders(
                &captured.orders.rows,
                &captured.orders.geometry,
                5,
                None
            )),
            unavailable("walker produced duplicate engine ids")
        );
    }
}
