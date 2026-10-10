//! Class13's fresh counter duration (`140E74B20`, `140BECF90`).
//! The common route search owns admission. Land speed uses equipment maximum
//! plus the directed railway bonus, then land scale and the gun's own supply
//! factor; it never uses the army road-speed fallback.

use super::fresh_search::Inputs;
use crate::map::raids::reach::Moves;
use engine::{carmy::MovementSpeedInputs, cunit::eta_ratio};
use installed_content::unit_path::{CounterRequest, Fact, MoveWorld};
use replay::rows::{FreshRailwayCounterEta, RailwayGunCounterInputRow};

pub(super) fn derive(
    inputs: &Inputs<'_>,
    moves: &Moves<'_, '_>,
    gun: &RailwayGunCounterInputRow,
    ours: bool,
    local_option: bool,
) -> Fact<i64> {
    let gun = inputs.railway(gun.gun)?;
    let endpoint = target(gun, ours)?;
    derive_to(inputs, moves, gun.gun, endpoint, local_option).and_then(|preview| preview.duration)
}

pub(super) fn derive_to(
    inputs: &Inputs<'_>,
    moves: &Moves<'_, '_>,
    handle: u64,
    target: u32,
    local_option: bool,
) -> Fact<super::CounterPreview> {
    let gun = inputs.railway(handle)?;
    inputs.route_identities()?;
    let route = moves.railway_counter_route(
        gun,
        target,
        CounterRequest {
            strategic: gun.strategic_redeployment != 0,
            local_option,
            ..Default::default()
        },
    )?;
    let duration = if route.is_empty() {
        Ok(0)
    } else {
        fresh(gun, inputs.hour).and_then(|fresh| {
            remaining(inputs, gun, fresh, &route, |from| {
                moves.strategic_region(from).ok_or_else(|| {
                    format!("fresh railway ETA source {from} has no strategic region")
                })
            })
        })
    };
    Ok(super::CounterPreview { duration, route })
}

fn target(gun: &RailwayGunCounterInputRow, ours: bool) -> Fact<u32> {
    if ours {
        gun.ordinary_path.last()
    } else {
        gun.ordinary_path.first()
    }
    .copied()
    .filter(|&id| id != 0)
    .ok_or_else(|| "fresh railway ETA ordinary route endpoint is unavailable".into())
}

fn fresh(gun: &RailwayGunCounterInputRow, hour: i32) -> Fact<&FreshRailwayCounterEta> {
    let fresh = gun
        .fresh
        .require("RailwayGunCounterInputs.fresh")
        .map_err(|error| error.to_string())?
        .as_ref()
        .ok_or("fresh railway ETA inputs were refused")?;
    if fresh.sample_hour != hour {
        return Err(format!(
            "fresh railway ETA inputs sampled at {}, frame is {hour}",
            fresh.sample_hour
        ));
    }
    Ok(fresh)
}

fn remaining(
    inputs: &Inputs<'_>,
    gun: &RailwayGunCounterInputRow,
    fresh: &FreshRailwayCounterEta,
    route: &[u32],
    source_region: impl Fn(u32) -> Fact<u32>,
) -> Fact<i64> {
    if route.is_empty() {
        return Ok(0);
    }
    let map = &inputs.content.map;
    let defines = &inputs.content.defines;
    let terrain = |id| {
        map.terrain(id)
            .ok_or_else(|| format!("fresh railway ETA province {id} has no content terrain"))
    };
    let mut from = gun.province;
    let mut duration = 0_i64;
    for &to in route {
        if to == 0 {
            return Err("fresh railway ETA searched route contains a null province".into());
        }
        if from == to {
            continue;
        }
        let source = terrain(from)?;
        let target = terrain(to)?;
        let distance = if source.flags & 3 == 0 && target.flags & 1 != 0 {
            defines.embark_distance
        } else {
            map.distance(from, to).ok_or_else(|| {
                format!("fresh railway ETA edge {from}->{to} distance is unavailable")
            })?
        };
        let speed = if gun.naval_transfer != 0 || source.flags & 1 == 0 || target.flags & 1 == 0 {
            MovementSpeedInputs::Scaled {
                speed: inputs.transport(gun.logical_owner, source_region(from)?)?,
                scale: defines.transport_scale,
            }
            .evaluate()
        } else {
            let maximum = fresh
                .maximum_speed
                .ok_or("fresh railway ETA equipment maximum speed was refused")?;
            let percentage = inputs.directed_rail_percentage(from, to)?;
            let rail = inputs.content.rail.as_ref().map_err(Clone::clone)?;
            let bonus = MovementSpeedInputs::Scaled {
                speed: percentage,
                scale: rail.speed[3],
            }
            .evaluate();
            let scaled = MovementSpeedInputs::Scaled {
                speed: maximum.wrapping_add(bonus),
                scale: defines.outer_scale,
            }
            .evaluate();
            MovementSpeedInputs::Scaled {
                speed: scaled,
                scale: fresh
                    .land_supply_factor
                    .ok_or("fresh railway ETA land supply factor was refused")?,
            }
            .evaluate()
        };
        // BECF90 exits immediately on zero speed, before later edges or the
        // independently captured actual first-edge progress are demanded.
        if speed == 0 {
            return Ok(0);
        }
        duration = duration.wrapping_add(eta_ratio(distance, speed).map_err(str::to_owned)?);
        from = to;
    }
    if route.first() == gun.ordinary_path.first() {
        let actual = gun
            .eta
            .require("RailwayGunCounterInputs.eta")
            .map_err(|error| error.to_string())?
            .as_ref()
            .ok_or("fresh railway ETA actual first-edge inputs were refused")?;
        Ok(duration.wrapping_sub(
            eta_ratio(actual.numerator, actual.current_speed).map_err(str::to_owned)?,
        ))
    } else {
        Ok(duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::{division_eta::MovementContent, test_frame, test_put as put};
    use installed_content::movement::{MovementMap, Neighbour, ProvinceTerrain, RailMovementDefines};
    use replay::{
        frame::DecodedFrame,
        rows::{
            CountryCounterInputRow, CountryLinkRow, CountryMovementInputs, CountryRow,
            RailCooldownRow, RailEdgeRow, RailwayBuildingHealth, RailwayBuildingState,
            RailwayGunEtaOperands, Recorded, RegionTransportSpeed, TableRows,
        },
    };

    fn frame() -> DecodedFrame {
        test_frame(20)
    }

    fn content(source_flags: u8) -> MovementContent {
        MovementContent {
            map: MovementMap::from_parts(&[
                (
                    1,
                    ProvinceTerrain {
                        flags: source_flags,
                        index: 1,
                    },
                    vec![Neighbour {
                        to: 2,
                        distance: Some(600_000),
                        ..Default::default()
                    }],
                ),
                (
                    2,
                    ProvinceTerrain { flags: 1, index: 2 },
                    vec![Neighbour {
                        to: 3,
                        distance: Some(300_000),
                        ..Default::default()
                    }],
                ),
                (3, ProvinceTerrain { flags: 1, index: 3 }, vec![]),
            ]),
            defines: replay::eta_derive::Defines {
                embark_distance: 400_000,
                transport_scale: 100_000,
                outer_scale: 100_000,
                ..Default::default()
            },
            // Large road/base values must not enter the class13 formula.
            rail: Ok(RailMovementDefines {
                speed: [99_000_000, 98_000_000, 97_000_000, 200_000],
                maximum_level: 5,
            }),
            naval: Err("unrelated naval defines are unavailable".into()),
            task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
        }
    }

    fn gun() -> RailwayGunCounterInputRow {
        RailwayGunCounterInputRow {
            gun: 7,
            province: 1,
            logical_owner: 99,
            // Different first edge leaves old ETA inputs outside demand.
            ordinary_path: vec![4],
            ..Default::default()
        }
    }

    fn land() -> FreshRailwayCounterEta {
        FreshRailwayCounterEta {
            sample_hour: 20,
            maximum_speed: Some(200_000),
            land_supply_factor: Some(50_000),
            ..Default::default()
        }
    }

    fn rails(frame: &mut DecodedFrame) {
        put(
            frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                building: Recorded::Value(Some(RailwayBuildingState {
                    building: Some(RailwayBuildingHealth {
                        level: 5,
                        healthy: 5,
                        partial: None,
                    }),
                })),
                ..Default::default()
            }]),
        );
        put(
            frame,
            TableRows::RailEdges(vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: 5,
            }]),
        );
    }

    fn transport(frame: &mut DecodedFrame, speed: i64) {
        put(
            frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 99,
                canonical_slot: 1,
            }]),
        );
        put(
            frame,
            TableRows::Countries(vec![CountryRow {
                id: 101,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        put(
            frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                forward_visibility: replay::rows::Recorded::NotRecorded,
                country: 101,
                movement: Recorded::Value(Some(CountryMovementInputs {
                    sample_hour: 20,
                    river: None,
                    transport: Some(vec![RegionTransportSpeed {
                        region: 9,
                        speed: Some(speed),
                    }]),
                    ..Default::default()
                })),
                ..Default::default()
            }]),
        );
    }

    #[test]
    fn ordinary_endpoint_and_fresh_hour_are_exact() {
        let mut gun = gun();
        gun.ordinary_path = vec![2, 3];
        assert_eq!(target(&gun, false), Ok(2));
        assert_eq!(target(&gun, true), Ok(3));
        gun.fresh = Recorded::Value(Some(land()));
        assert_eq!(fresh(&gun, 20), Ok(&land()));
        assert!(
            fresh(&gun, 21)
                .unwrap_err()
                .contains("sampled at 20, frame is 21")
        );
        gun.fresh = Recorded::Value(None);
        assert!(fresh(&gun, 20).is_err());
        gun.fresh = Recorded::NotRecorded;
        assert!(fresh(&gun, 20).is_err());
        gun.ordinary_path.clear();
        assert!(target(&gun, true).is_err());
        gun.ordinary_path = vec![2, 0];
        assert_eq!(target(&gun, false), Ok(2));
        assert!(target(&gun, true).is_err());
    }

    #[test]
    fn duplicate_railway_rows_refuse_the_query_identity_domain() {
        let mut frame = frame();
        let mut other = gun();
        other.province = 2;
        put(
            &mut frame,
            TableRows::RailwayGunCounterInputs(vec![gun(), other]),
        );
        let content = content(1);
        let inputs = Inputs::new(&frame, &content);
        assert!(inputs.railway(7).unwrap_err().contains("duplicate keys"));
    }

    #[test]
    fn railway_bonus_is_directed_and_applies_supply_after_land_scale() {
        let mut frame = frame();
        rails(&mut frame);
        let mut content = content(1);
        content.defines.outer_scale = 150_000;
        let inputs = Inputs::new(&frame, &content);
        // 1->2: (2+2)*1.5*.5=3; 2->3: 2*1.5*.5=1.5.
        assert_eq!(
            remaining(&inputs, &gun(), &land(), &[2, 3], |_| Err(
                "transport reached".into()
            )),
            Ok(400_000)
        );
    }

    #[test]
    fn transport_skips_refused_land_domains_and_uses_embark_distance() {
        let mut frame = frame();
        transport(&mut frame, 200_000);
        let mut transport_content = content(0);
        transport_content.rail = Err("unreached rail content".into());
        let inputs = Inputs::new(&frame, &transport_content);
        assert_eq!(
            remaining(
                &inputs,
                &gun(),
                &FreshRailwayCounterEta::default(),
                &[2],
                |_| Ok(9)
            ),
            Ok(200_000)
        );
        let mut transfer = gun();
        transfer.naval_transfer = 2;
        let content = content(1);
        let inputs = Inputs::new(&frame, &content);
        assert_eq!(
            remaining(
                &inputs,
                &transfer,
                &FreshRailwayCounterEta::default(),
                &[2, 3],
                |_| Ok(9)
            ),
            Ok(450_000)
        );
    }

    #[test]
    fn actual_progress_is_demanded_only_for_the_same_first_edge() {
        let mut frame = frame();
        rails(&mut frame);
        let content = content(1);
        let inputs = Inputs::new(&frame, &content);
        let mut gun = gun();
        gun.ordinary_path = vec![2];
        assert!(remaining(&inputs, &gun, &land(), &[2], |_| Ok(9)).is_err());
        gun.eta = Recorded::Value(Some(RailwayGunEtaOperands {
            numerator: 50_000,
            current_speed: 100_000,
        }));
        assert_eq!(
            remaining(&inputs, &gun, &land(), &[2], |_| Ok(9)),
            Ok(250_000)
        );
        gun.ordinary_path = vec![4];
        gun.eta = Recorded::NotRecorded;
        assert_eq!(
            remaining(&inputs, &gun, &land(), &[2], |_| Ok(9)),
            Ok(300_000)
        );
    }

    #[test]
    fn zero_speed_stops_before_progress_or_later_land_operands() {
        let mut frame = frame();
        transport(&mut frame, 0);
        let content = content(0);
        let inputs = Inputs::new(&frame, &content);
        let mut gun = gun();
        gun.ordinary_path = vec![2];
        assert_eq!(
            remaining(
                &inputs,
                &gun,
                &FreshRailwayCounterEta::default(),
                &[2, 3],
                |_| Ok(9)
            ),
            Ok(0)
        );
        assert_eq!(
            remaining(
                &inputs,
                &gun,
                &FreshRailwayCounterEta::default(),
                &[],
                |_| Err("unreached region".into())
            ),
            Ok(0)
        );
    }

    #[test]
    fn earlier_edge_division_overflow_is_not_hidden_by_later_zero_speed() {
        let mut frame = frame();
        put(
            &mut frame,
            TableRows::RailCooldowns(
                [(1, 1, None), (2, 0, Some(0)), (3, 1, None)]
                    .into_iter()
                    .map(|(province, healthy, partial)| RailCooldownRow {
                        province,
                        building: Recorded::Value(Some(RailwayBuildingState {
                            building: Some(RailwayBuildingHealth {
                                level: 1,
                                healthy,
                                partial,
                            }),
                        })),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::RailEdges(vec![
                RailEdgeRow {
                    from: 1,
                    to: 2,
                    level: 1,
                },
                RailEdgeRow {
                    from: 2,
                    to: 3,
                    level: 1,
                },
            ]),
        );
        let mut content = content(1);
        content.map = MovementMap::from_parts(&[
            (
                1,
                ProvinceTerrain { flags: 1, index: 1 },
                vec![Neighbour {
                    to: 2,
                    distance: Some(1_i64 << 58),
                    ..Default::default()
                }],
            ),
            (
                2,
                ProvinceTerrain { flags: 1, index: 2 },
                vec![Neighbour {
                    to: 3,
                    distance: Some(300_000),
                    ..Default::default()
                }],
            ),
            (3, ProvinceTerrain { flags: 1, index: 3 }, vec![]),
        ]);
        content.rail = Ok(RailMovementDefines {
            speed: [0, 0, 0, -2],
            maximum_level: 1,
        });
        let fresh = FreshRailwayCounterEta {
            maximum_speed: Some(0),
            ..land()
        };
        let inputs = Inputs::new(&frame, &content);
        // First speed = -2*.5 = -1, so wrapping distance*100000 is
        // i64::MIN and native division traps. The damaged next departure
        // would produce speed0, but native has already reached the trap.
        assert_eq!(
            remaining(&inputs, &gun(), &fresh, &[2, 3], |_| Ok(9)),
            Err("ETA native signed division overflows".into()),
        );
    }

    #[test]
    fn land_common_refusals_never_become_an_army_fallback() {
        let mut frame = frame();
        rails(&mut frame);
        let content = content(1);
        let inputs = Inputs::new(&frame, &content);
        let mut fresh = land();
        fresh.maximum_speed = None;
        assert!(
            remaining(&inputs, &gun(), &fresh, &[2], |_| Ok(9))
                .unwrap_err()
                .contains("maximum speed")
        );
        fresh = land();
        fresh.land_supply_factor = None;
        assert!(
            remaining(&inputs, &gun(), &fresh, &[2], |_| Ok(9))
                .unwrap_err()
                .contains("supply factor")
        );
    }
}
