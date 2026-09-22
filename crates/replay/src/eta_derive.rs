//! The native ETA walk, replayed from recorded operands with no memory access.
//!
//! Port of the native walk (`140BECF90`) over the `DivisionEtaInputs` row.
//! Per-pair operands are leg-aligned vectors on the division row. The duration
//! and speed evaluators are the engine's own pure functions. The recording
//! stores these operands, not a second independently acquired duration, so
//! replay alone cannot certify capture equivalence to the native walk.

use crate::rows::{DivisionEtaInputsRow, DivisionMovementRow, MovementDefinesRow};
use engine::carmy::{MovementSpeedInputs, NormalEdgeOperands, PreparedNormalSpeed};
use engine::cunit::{EtaInputs, EtaLeg, EtaSpeed, eta_remaining};

/// The destination-side province operands `normal_movement_edge` reads.
/// The river relation term is recorded per leg, so the destination
/// controller is not needed here.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProvinceOperands {
    /// Raw province_definition +0xd2; selects the transport branch.
    pub terrain_flags: u8,
    /// One-based CTerrainType +0x88 index; indexes `terrain_speed`.
    pub terrain_index: u8,
    pub fort_positive: bool,
    /// Owning state id, or zero for a province with no state.
    pub state: u32,
}

/// Recompute `(next_remaining, destination_remaining)` for one moving division.
///
/// `location` is `Divisions.province`, the walk's start. `province` and
/// `state_infra` resolve this same hour's rows; a missing one is a refusal.
/// The caller validates the weather census before `snow` follows the native
/// predicate, where a province without a weather record has no snow. `None`
/// means the row cannot be derived exactly.
///
/// `adjacency_complete` is not consulted. The recorder stores each leg's
/// distance directly, including the native 100000 no-adjacency fallback, so
/// missing adjacency does not prevent derivation. The flag is diagnostic.
pub fn derive(
    inputs: &DivisionEtaInputsRow,
    movement: &DivisionMovementRow,
    location: u32,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    state_infra: impl Fn(u32) -> Option<i64>,
    snow: impl Fn(u32) -> bool,
    defines: &MovementDefinesRow,
) -> Option<(i64, i64)> {
    if location == 0 {
        return None;
    }
    // A route the recorder's arena truncated cannot be walked.
    if movement.path.len() != movement.path_total as usize {
        return None;
    }
    if movement.path.is_empty() {
        return Some((0, 0)); // cunit.rs:889-891
    }
    let mut legs: Vec<EtaLeg> = Vec::with_capacity(movement.path.len());
    let mut next_leg_count = 0;
    let mut from = location;
    for (index, &to) in movement.path.iter().enumerate() {
        if to == 0 {
            return None; // cunit.rs:632-635
        }
        if from != to {
            // Self edges emit no leg, so the leg vectors advance only here.
            let leg = legs.len();
            let distance = *inputs.leg_distance.get(leg)?;
            let speed = edge_speed(
                inputs,
                movement,
                defines,
                &province,
                &state_infra,
                &snow,
                from,
                to,
                leg,
            )?;
            legs.push(EtaLeg {
                distance,
                speed: EtaSpeed::Fixed(speed),
            });
            if index == 0 {
                next_leg_count = 1; // cunit.rs:659-661
            }
            // `is_zero` only short-circuits a proof that the speed is positive
            // (carmy.rs:1600-1610), so the evaluated value decides identically.
            if speed == 0 {
                break; // cunit.rs:663-666
            }
        }
        from = to;
    }
    let input = EtaInputs {
        division: inputs.division,
        next: movement.path[0],
        destination: movement.path[movement.path.len() - 1],
        path_total: movement.path_total,
        leg_start: 0,
        leg_len: legs.len(),
        next_leg_count,
        numerator: movement.numerator,
        // Recorded directly, including the native null-destination branch
        // this call alone can take.
        current_speed: EtaSpeed::Fixed(inputs.current_speed),
        normal: None,
    };
    eta_remaining(&input, &legs).ok()
}

/// One leg's speed, in the native branch order of
/// `movement_speed_inputs_internal` (carmy.rs:2395-2504).
#[allow(clippy::too_many_arguments)]
fn edge_speed(
    inputs: &DivisionEtaInputsRow,
    movement: &DivisionMovementRow,
    defines: &MovementDefinesRow,
    province: impl Fn(u32) -> Option<ProvinceOperands>,
    state_infra: impl Fn(u32) -> Option<i64>,
    snow: impl Fn(u32) -> bool,
    from: u32,
    to: u32,
    leg: usize,
) -> Option<i64> {
    // A branch vector may be empty only when no leg took it. An evaluated
    // branch requires its recorded operand, including an explicitly recorded zero.
    let recorded = |values: &[i64]| values.get(leg).copied();
    let source = province(from)?;
    let target = province(to)?;
    if movement.naval_transfer != 0
        || source.terrain_flags & 1 == 0
        || target.terrain_flags & 1 == 0
    {
        return Some(
            MovementSpeedInputs::Scaled {
                speed: recorded(&inputs.leg_transport_speed)?,
                scale: defines.transport_scale,
            }
            .evaluate(),
        );
    }
    if inputs.invasion != 0 {
        return Some(defines.invasion_speed);
    }
    if movement.strategic_redeployment != 0 && inputs.rail_blocked == 0 {
        return Some(
            MovementSpeedInputs::Scaled {
                speed: recorded(&inputs.leg_rail_speed)?,
                scale: defines.outer_scale,
            }
            .evaluate(),
        );
    }
    // normal_movement_edge, carmy.rs:3030-3147. The sub-unit and leader terrain
    // terms are already composed into terrain_speed.
    let terrain = usize::from(target.terrain_index).checked_sub(1)?;
    let mut additive = *inputs.terrain_speed.get(terrain)?;
    if target.fort_positive {
        additive = additive.wrapping_add(inputs.fort_penalty);
    }
    if snow(to) {
        additive = additive.wrapping_add(inputs.snow_penalty);
    }
    if target.state != 0 {
        additive = additive.wrapping_add(state_infra(target.state)?);
    }
    let crossing = *inputs.leg_crossing.get(leg)?;
    let river = if (1..=3).contains(&crossing) {
        Some((
            recorded(&inputs.leg_river_combined)?.wrapping_add(inputs.river_leader),
            inputs.river_adjustment,
            match crossing {
                2 => defines.river_crossing_two,
                3 => defines.river_crossing_three,
                _ => 0,
            },
        ))
    } else {
        None
    };
    Some(
        PreparedNormalSpeed {
            modifier: inputs.speed_modifier,
            base_speed: inputs.speed_base,
            organisation: inputs.speed_org,
            fuel: inputs.speed_fuel,
            outer_scale: inputs.speed_outer_scale,
        }
        .evaluate(NormalEdgeOperands { additive, river }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAND: u8 = 1;
    const SEA: u8 = 0;

    fn defines() -> MovementDefinesRow {
        MovementDefinesRow {
            transport_scale: 200_000,
            invasion_speed: 700_000,
            outer_scale: 100_000,
            river_crossing_two: -20_000,
            river_crossing_three: -50_000,
            embark_distance: 1_000,
        }
    }

    /// Land, terrain index 1, no fort, state 1; id zero has no row.
    fn land(id: u32) -> Option<ProvinceOperands> {
        (id != 0).then_some(ProvinceOperands {
            terrain_flags: LAND,
            terrain_index: 1,
            fort_positive: false,
            state: 1,
        })
    }

    /// modifier 1.0 with terrain zero, so a plain normal leg runs at speed_base.
    fn inputs(legs: usize) -> DivisionEtaInputsRow {
        DivisionEtaInputsRow {
            division: 51,
            speed_modifier: 100_000,
            speed_base: 400_000,
            speed_outer_scale: 100_000,
            current_speed: 400_000,
            terrain_speed: vec![0],
            adjacency_complete: 1,
            leg_distance: vec![100_000; legs],
            leg_crossing: vec![0; legs],
            ..Default::default()
        }
    }

    fn movement(path: Vec<u32>) -> DivisionMovementRow {
        DivisionMovementRow {
            division: 51,
            next: path[0],
            destination: path[path.len() - 1],
            path_total: path.len() as u32,
            path,
            ..Default::default()
        }
    }

    fn run(
        inputs: &DivisionEtaInputsRow,
        movement: &DivisionMovementRow,
        location: u32,
    ) -> Option<(i64, i64)> {
        derive(
            inputs,
            movement,
            location,
            land,
            |_| Some(0),
            |_| false,
            &defines(),
        )
    }

    #[test]
    fn a_normal_route_truncates_each_leg_and_subtracts_progress() {
        let mut row = inputs(2);
        row.leg_distance = vec![100_000, 250_000];
        let mut path = movement(vec![2, 3]);
        path.numerator = 50_000;
        // Speed 400000: legs are 25000 and 62500 fixed5 hours, progress 12500.
        assert_eq!(run(&row, &path, 1), Some((12_500, 75_000)));
    }

    #[test]
    fn a_self_first_edge_emits_no_leg_so_next_remaining_is_minus_progress() {
        let row = inputs(1);
        let mut path = movement(vec![1, 2]);
        path.numerator = 50_000;
        // cunit.rs:637 and :916 - route[0] == location.
        assert_eq!(run(&row, &path, 1), Some((-12_500, 12_500)));
    }

    #[test]
    fn a_zero_speed_leg_stops_the_walk_and_zeroes_the_duration() {
        // Leg zero at zero speed: both results are zero, progress unevaluated.
        let mut row = inputs(2);
        row.speed_outer_scale = 0;
        assert_eq!(run(&row, &movement(vec![2, 3]), 1), Some((0, 0)));
        // A later zero leg keeps next_remaining and zeroes the destination.
        let mut row = inputs(2);
        row.leg_transport_speed = vec![0, 0];
        let water = |id: u32| {
            land(id).map(|p| ProvinceOperands {
                terrain_flags: if id == 3 { SEA } else { LAND },
                ..p
            })
        };
        let derived = derive(
            &row,
            &movement(vec![2, 3]),
            1,
            water,
            |_| Some(0),
            |_| false,
            &defines(),
        );
        assert_eq!(derived, Some((25_000, 0)));
    }

    /// The recorder stops its scan at the first zero-speed leg, so the leg
    /// vectors are a prefix of the route. The walk stops there too, and must
    /// never reach for an operand of a leg neither side evaluates.
    #[test]
    fn a_route_recorded_only_up_to_its_stop_still_derives() {
        // Provinces 1..=3 are recorded and 4 is not, so touching 4 refuses.
        let known = |sea: u32| {
            move |id: u32| {
                (1..=3).contains(&id).then_some(ProvinceOperands {
                    terrain_flags: if id == sea { SEA } else { LAND },
                    terrain_index: 1,
                    fort_positive: false,
                    state: 1,
                })
            }
        };
        let evaluate = |row: &DivisionEtaInputsRow, sea: u32| {
            derive(
                row,
                &movement(vec![2, 3, 4]),
                1,
                known(sea),
                |_| Some(0),
                |_| false,
                &defines(),
            )
        };
        // Stopping at leg zero: one recorded leg for a three-leg route.
        let mut row = inputs(1);
        row.leg_transport_speed = vec![0];
        assert_eq!(evaluate(&row, 2), Some((0, 0)));
        // Stopping mid route: two recorded legs, and leg two is never reached.
        let mut row = inputs(2);
        row.leg_transport_speed = vec![0, 0];
        assert_eq!(evaluate(&row, 3), Some((25_000, 0)));
        // A route that never stops still needs a leg for every route edge.
        assert_eq!(evaluate(&inputs(2), 0), None);
    }

    #[test]
    fn every_branch_takes_its_own_recorded_leg_value() {
        // Transport: either endpoint not land, or the naval transfer bit.
        let mut row = inputs(1);
        row.leg_transport_speed = vec![50_000];
        let water = |id: u32| {
            land(id).map(|p| ProvinceOperands {
                terrain_flags: if id == 2 { SEA } else { LAND },
                ..p
            })
        };
        let transport = derive(
            &row,
            &movement(vec![2]),
            1,
            water,
            |_| Some(0),
            |_| false,
            &defines(),
        );
        // 50000 scaled by 200000 is speed 100000 over distance 100000.
        assert_eq!(transport, Some((100_000, 100_000)));
        let mut transfer = movement(vec![2]);
        transfer.naval_transfer = 1;
        assert_eq!(run(&row, &transfer, 1), Some((100_000, 100_000)));
        // Invasion: a fixed define, with no leg vector at all.
        let mut row = inputs(1);
        row.invasion = 1;
        assert_eq!(run(&row, &movement(vec![2]), 1), Some((14_285, 14_285)));
        // Rail: strategic redeployment with the rail branch unblocked.
        let mut row = inputs(1);
        row.leg_rail_speed = vec![250_000];
        let mut rail = movement(vec![2]);
        rail.strategic_redeployment = 1;
        assert_eq!(run(&row, &rail, 1), Some((40_000, 40_000)));
        // Blocked rail falls through to the normal branch.
        row.rail_blocked = 1;
        assert_eq!(run(&row, &rail, 1), Some((25_000, 25_000)));
    }

    #[test]
    fn evaluated_branches_require_their_operands_but_preserve_recorded_zero() {
        let mut row = inputs(1);
        let mut route = movement(vec![2]);
        route.naval_transfer = 1;
        assert_eq!(run(&row, &route, 1), None);
        row.leg_transport_speed = vec![0];
        assert_eq!(run(&row, &route, 1), Some((0, 0)));

        route.naval_transfer = 0;
        route.strategic_redeployment = 1;
        assert_eq!(run(&row, &route, 1), None);
        row.leg_rail_speed = vec![0];
        assert_eq!(run(&row, &route, 1), Some((0, 0)));

        route.strategic_redeployment = 0;
        row.leg_crossing = vec![1];
        assert_eq!(run(&row, &route, 1), None);
        row.leg_river_combined = vec![0];
        assert_eq!(run(&row, &route, 1), Some((25_000, 25_000)));
        row.leg_crossing.clear();
        assert_eq!(run(&row, &route, 1), None);
    }

    #[test]
    fn the_normal_edge_adds_fort_snow_infrastructure_and_the_river_terms() {
        let mut row = inputs(1);
        row.speed_modifier = 200_000;
        row.terrain_speed = vec![-10_000];
        row.fort_penalty = -20_000;
        row.snow_penalty = -30_000;
        row.river_adjustment = -40_000;
        row.river_leader = 30_000;
        row.leg_crossing = vec![2];
        row.leg_river_combined = vec![50_000];
        let fortified = |id: u32| {
            land(id).map(|p| ProvinceOperands {
                fort_positive: true,
                ..p
            })
        };
        let derived = derive(
            &row,
            &movement(vec![2]),
            1,
            fortified,
            |_| Some(5_000),
            |_| true,
            &defines(),
        );
        let speed = PreparedNormalSpeed {
            modifier: 200_000,
            base_speed: 400_000,
            organisation: None,
            fuel: None,
            outer_scale: 100_000,
        }
        .evaluate(NormalEdgeOperands {
            additive: -10_000 - 20_000 - 30_000 + 5_000,
            river: Some((80_000, -40_000, -20_000)),
        });
        assert_eq!(
            derived,
            Some((10_000_000_000 / speed, 10_000_000_000 / speed))
        );
    }

    #[test]
    fn the_organisation_and_fuel_factors_apply_when_present() {
        let mut row = inputs(1);
        row.speed_org = Some(50_000);
        row.speed_fuel = Some(80_000);
        // 400000 halved and then scaled by 0.8 is 160000.
        assert_eq!(run(&row, &movement(vec![2]), 1), Some((62_500, 62_500)));
    }

    #[test]
    fn a_missing_operand_refuses_the_row_but_an_incomplete_adjacency_does_not() {
        // leg_distance already holds what the native no-adjacency call returned.
        let mut row = inputs(1);
        row.adjacency_complete = 0;
        assert_eq!(run(&row, &movement(vec![2]), 1), Some((25_000, 25_000)));
        let row = inputs(1);
        let refuse = |_: u32| None;
        let none = |_: u32| None;
        assert_eq!(
            derive(
                &row,
                &movement(vec![2]),
                1,
                refuse,
                |_| Some(0),
                |_| false,
                &defines()
            ),
            None,
            "a province with no recorded row"
        );
        assert_eq!(
            derive(
                &row,
                &movement(vec![2]),
                1,
                land,
                none,
                |_| false,
                &defines()
            ),
            None,
            "a state with no recorded infrastructure"
        );
        let deep = |id: u32| {
            land(id).map(|p| ProvinceOperands {
                terrain_index: 9,
                ..p
            })
        };
        assert_eq!(
            derive(
                &row,
                &movement(vec![2]),
                1,
                deep,
                |_| Some(0),
                |_| false,
                &defines()
            ),
            None,
            "a terrain index outside the recorded vector"
        );
        let mut short = inputs(1);
        short.leg_distance = Vec::new();
        short.leg_crossing = Vec::new();
        assert_eq!(
            run(&short, &movement(vec![2]), 1),
            None,
            "fewer legs than legs walked"
        );
        let mut truncated = movement(vec![2, 3]);
        truncated.path_total = 3;
        assert_eq!(run(&inputs(2), &truncated, 1), None, "a truncated path");
        assert_eq!(
            run(&inputs(2), &movement(vec![2, 0]), 1),
            None,
            "a null route province"
        );
        assert_eq!(
            run(&inputs(1), &movement(vec![2]), 0),
            None,
            "a null location"
        );
    }
}
