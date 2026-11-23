
use super::fresh_search::{Inputs, available};
use game_model::movement::unit::eta_ratio;
use content_reader_unit_path::unit_path::Fact;
use replay_data::{
    rows::FreshTaskForceCounterEta, rows::TaskForceMapMotionRow, rows::TaskForceRow,
};
use dashboard_text::table_access::TokenError;

const P: i64 = 100_000;

pub(super) const ADMISSION_UNRECORDED: &str = "task-force route admission inputs (owner navy range, theatres and nearest-base caches) are not recorded";

pub(super) fn derive(
    inputs: &Inputs<'_>,
    motion: &TaskForceMapMotionRow,
    ours: bool,
    source_region: impl Fn(u32) -> Fact<u32>,
) -> Fact<i64> {
    let naval = inputs.naval();
    let motion = naval.motion(motion.task_force)?;
    let force = naval.tf(motion.task_force)?;
    let route = stored_route(force, motion, ours)?;
    let profile = naval.fresh(force.id)?;
    remaining(inputs, force, motion, profile, route, source_region)
}

fn stored_route<'m>(
    force: &TaskForceRow,
    motion: &'m TaskForceMapMotionRow,
    ours: bool,
) -> Fact<&'m [u32]> {
    if force.route_count <= 0 {
        return Err("task force has no ordinary route".into());
    }
    if !ours {
        return motion
            .path
            .get(..1)
            .ok_or_else(|| "task force next province is not recorded".into());
    }
    if motion.path.len() != force.route_count as usize {
        return Err("task force destination route is incomplete".into());
    }
    Ok(&motion.path)
}

fn speed(
    inputs: &Inputs<'_>,
    force: &TaskForceRow,
    fresh: &FreshTaskForceCounterEta,
    region: u32,
) -> Fact<i64> {
    replay_derive::fixed_layout::validate_recording(
        inputs.frame(),
        replay_derive::fixed_layout::Family {
            keys: &[
                engine_executable::modifier_keys::NAVAL_SPEED_FACTOR,
                engine_executable::modifier_keys::NAVY_WEATHER_PENALTY,
            ],
            ..replay_derive::fixed_layout::TASK_FORCE_COUNTER_ETA
        },
    )?;
    let base = fresh
        .base_speed
        .ok_or("fresh naval ETA base speed was refused")?;
    let modifier = fresh
        .speed_modifier
        .ok_or("fresh naval ETA common speed modifier was refused")?;
    let selector = fresh
        .terrain_class
        .ok_or("fresh naval ETA terrain class selector was refused")?;
    let terrain = available(&inputs.content.task_force_terrain)?;
    let naval = inputs.naval();
    let adjustment = |token| {
        terrain.movement(region, || {
            match naval.tokens()?.name(token) {
                Ok(name) => Ok(Some(name)),
                Err(TokenError::Rejected | TokenError::Absent(_)) => Ok(None),
                Err(error) => Err(error.to_string()),
            }
        })
    };
    let adjustment = if selector >= 0 {
        adjustment(selector as u32)?
    } else {
        let ships = naval.ships(force.id)?;
        let count = i32::try_from(ships.len())
            .map_err(|_| "fresh naval ETA ship count exceeds the native signed representation")?;
        let sum = ships.iter().try_fold(0_i64, |sum, ship| {
            Ok::<_, String>(sum.wrapping_add(adjustment(ship.class_id as u32)?))
        })?;
        class_mean(sum, count)
    };
    Ok(evaluate_speed(
        base,
        modifier,
        adjustment,
        inputs.content.defines.transport_scale,
    ))
}

fn class_mean(sum: i64, count: i32) -> i64 {
    if count == 0 {
        return 0;
    }
    if (sum as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return 0xffff_ffff;
    }
    sum.wrapping_mul(P) / (i64::from(count) * P)
}

fn evaluate_speed(base: i64, modifier: i64, terrain: i64, outer: i64) -> i64 {
    let factor = P.wrapping_add(modifier).wrapping_add(terrain).max(10_000);
    (base.wrapping_mul(factor) / P).wrapping_mul(outer) / P
}

fn remaining(
    inputs: &Inputs<'_>,
    force: &TaskForceRow,
    motion: &TaskForceMapMotionRow,
    fresh: &FreshTaskForceCounterEta,
    route: &[u32],
    source_region: impl Fn(u32) -> Fact<u32>,
) -> Fact<i64> {
    if route.is_empty() {
        return Ok(0);
    }
    let mut from = force.province;
    let mut duration = 0_i64;
    let mut actual_speed = None;
    for &to in route {
        if to == 0 {
            return Err("fresh naval ETA route contains a null province".into());
        }
        if from == to {
            continue;
        }
        let distance =
            inputs.content.map.distance(from, to).ok_or_else(|| {
                format!("fresh naval ETA edge {from}->{to} distance is unavailable")
            })?;
        let speed = speed(inputs, force, fresh, source_region(from)?)?;
        if speed == 0 {
            return Ok(0);
        }
        if from == force.province && actual_speed.is_none() {
            actual_speed = Some(speed);
        }
        duration = duration.wrapping_add(eta_ratio(distance, speed).map_err(str::to_owned)?);
        from = to;
    }
    if !route.is_empty() && route.first() == motion.path.first() {
        let numerator = motion
            .movement_numerator
            .require("TaskForceMapMotion.movement_numerator")?
            .ok_or("fresh naval ETA movement numerator was refused")?;
        let actual_speed = match actual_speed {
            Some(speed) => speed,
            None => speed(inputs, force, fresh, source_region(force.province)?)?,
        };
        duration =
            duration.wrapping_sub(eta_ratio(numerator, actual_speed).map_err(str::to_owned)?);
    }
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_map::movement::{MovementDefines, MovementMap, Neighbour, ProvinceTerrain};
    use replay_data::{
        rows::LexicalTokenRow, rows::Recorded, rows::ShipRow, rows::TableId, rows::TableRows,
        rows::tables,
    };
    use replay_frame::{frame::DecodedFrame, frame::TableView};
    use viewer_foundation::test_support::movement_timing::terrain;
    use viewer_foundation::movement_timing::MovementContent;

    fn frame() -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 20,
            metrics: Default::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture domain unavailable".into(),
                }
                .into()
            }),
        };
        super::super::test_support::put_modifier_definitions(
            &mut frame,
            [
                engine_executable::modifier_keys::NAVAL_SPEED_FACTOR,
                engine_executable::modifier_keys::NAVY_WEATHER_PENALTY,
            ],
        );
        frame
    }

    fn put(frame: &mut DecodedFrame, rows: TableRows) {
        let id = rows.table();
        *frame.tables[id.index()] = TableView::Complete(rows);
    }

    fn tokens(frame: &mut DecodedFrame) {
        let mut tokens = frame.complete::<tables::LexicalTokens>().unwrap().to_vec();
        tokens.extend([
            LexicalTokenRow {
                token: 700,
                literal: b"custom_hull".to_vec(),
            },
            LexicalTokenRow {
                token: u32::MAX,
                literal: b"other_hull".to_vec(),
            },
        ]);
        tokens.sort_by_key(|row| row.token);
        put(frame, TableRows::LexicalTokens(tokens));
    }

    fn content() -> MovementContent {
        let sea = ProvinceTerrain { flags: 0, index: 1 };
        let land = ProvinceTerrain { flags: 1, index: 1 };
        let mut content = MovementContent::new(
            MovementMap::from_parts(&[
                (
                    1,
                    sea,
                    vec![Neighbour {
                        to: 2,
                        distance: Some(400_000),
                        ..Default::default()
                    }],
                ),
                (
                    2,
                    land,
                    vec![Neighbour {
                        to: 3,
                        distance: Some(600_000),
                        ..Default::default()
                    }],
                ),
                (3, sea, vec![]),
            ]),
            MovementDefines {
                embark_distance: 9_000_000,
                transport_scale: P,
                invasion_speed: 0,
                outer_scale: 0,
                river_crossing_two: 0,
                river_crossing_three: 0,
                organisation: Err("unrelated organisation defines unavailable".into()),
                noncore_supply: std::array::from_fn(|_| {
                    Err("unrelated noncore supply defines unavailable".into())
                }),
                combat_movement: Err("unrelated combat movement define unavailable".into()),
                rail: Err("unrelated railway defines unavailable".into()),
                naval: Err("unrelated range defines unavailable".into()),
            },
        );
        content.task_force_terrain = Ok(terrain(
            "categories={custom_water={units={movement=0.1} custom_hull={units={movement=0.2}} other_hull={units={movement=-0.1}}} other_water={units={movement=0.5}}}",
        ));
        content
    }

    fn force() -> TaskForceRow {
        TaskForceRow {
            id: 7,
            province: 1,
            route_count: 2,
            ..Default::default()
        }
    }

    fn motion() -> TaskForceMapMotionRow {
        TaskForceMapMotionRow {
            task_force: 7,
            path: vec![2, 3],
            movement_numerator: Recorded::Value(Some(100_000)),
            ..Default::default()
        }
    }

    fn fresh(selector: i32) -> FreshTaskForceCounterEta {
        FreshTaskForceCounterEta {
            sample_hour: replay_data::rows::PacketHour::Stamped(20),
            base_speed: Some(200_000),
            speed_modifier: Some(0),
            terrain_class: Some(selector),
            ..Default::default()
        }
    }

    #[test]
    fn arrival_walks_the_stored_route_to_the_destination_or_the_next_province() {
        let mut frame = frame();
        tokens(&mut frame);
        put(&mut frame, TableRows::TaskForces(vec![force()]));
        put(
            &mut frame,
            TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                fresh: Recorded::Value(Some(fresh(700))),
                ..motion()
            }]),
        );
        let content = content();
        let inputs = Inputs::new(&frame, &content);
        let region = |from| Ok(if from == 1 { 9 } else { 10 });
        assert_eq!(derive(&inputs, &motion(), true, region), Ok(315_385));
        assert_eq!(derive(&inputs, &motion(), false, region), Ok(115_385));
        let mut stale = frame.clone();
        put(
            &mut stale,
            TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                fresh: Recorded::Value(Some(FreshTaskForceCounterEta {
                    sample_hour: replay_data::rows::PacketHour::Stamped(19),
                    ..fresh(700)
                })),
                ..motion()
            }]),
        );
        assert!(
            derive(&Inputs::new(&stale, &content), &motion(), true, region)
                .unwrap_err()
                .contains("another hour")
        );
    }

    #[test]
    fn destination_requires_complete_route_but_next_keeps_a_prefix() {
        let force = force();
        let motion = TaskForceMapMotionRow {
            path: vec![2],
            ..motion()
        };
        assert_eq!(stored_route(&force, &motion, false), Ok(&[2][..]));
        assert!(
            stored_route(&force, &motion, true)
                .unwrap_err()
                .contains("incomplete")
        );
        assert_eq!(stored_route(&force, &self::motion(), true), Ok(&[2, 3][..]));
        for count in [0, -1] {
            assert!(
                stored_route(
                    &TaskForceRow {
                        route_count: count,
                        ..force.clone()
                    },
                    &motion,
                    false
                )
                .is_err()
            );
        }
        assert!(
            stored_route(
                &force,
                &TaskForceMapMotionRow {
                    path: vec![],
                    ..motion.clone()
                },
                false
            )
            .is_err()
        );
    }

    #[test]
    fn positive_selector_does_not_demand_ships_or_unrelated_domains() {
        let mut frame = frame();
        tokens(&mut frame);
        let content = content();
        let inputs = Inputs::new(&frame, &content);
        assert_eq!(speed(&inputs, &force(), &fresh(700), 9), Ok(260_000));
        assert_eq!(speed(&inputs, &force(), &fresh(700), 10), Ok(300_000));
    }

    #[test]
    fn negative_selector_averages_complete_raw_class_tokens() {
        let mut frame = frame();
        tokens(&mut frame);
        put(
            &mut frame,
            TableRows::Ships(vec![
                ShipRow {
                    id: 1,
                    task_force: 7,
                    class_id: 700,
                    ..Default::default()
                },
                ShipRow {
                    id: 2,
                    task_force: 7,
                    class_id: -1,
                    ..Default::default()
                },
            ]),
        );
        let content = content();
        assert_eq!(
            speed(&Inputs::new(&frame, &content), &force(), &fresh(-1), 9),
            Ok(230_000)
        );
        *frame.tables[TableId::Ships.index()] = TableView::Truncated {
            rows: TableRows::Ships(vec![]),
            missing: 1,
        };
        assert!(speed(&Inputs::new(&frame, &content), &force(), &fresh(-1), 9).is_err());
        put(&mut frame, TableRows::Ships(vec![]));
        assert_eq!(
            speed(&Inputs::new(&frame, &content), &force(), &fresh(-1), 9),
            Ok(200_000)
        );
    }

    #[test]
    fn class_average_keeps_fixed5_wrapping_and_exceptional_values() {
        assert_eq!(class_mean(30_001, 2), 15_000);
        assert_eq!(class_mean(i64::MAX, 2), 0xffff_ffff);
        assert_eq!(class_mean(i64::MIN, 2), 0xffff_ffff);
        assert_eq!(class_mean(100_000_000_000_000, 1), -84_467_440_737_095);
        assert_eq!(class_mean(i64::MAX, 0), 0);
    }

    #[test]
    fn speed_clamps_after_terrain_before_both_ordered_products() {
        assert_eq!(evaluate_speed(200_000, -100_000, 20_000, 50_000), 20_000);
        assert_eq!(evaluate_speed(200_000, -100_000, -20_000, 50_000), 10_000);
        assert_eq!(evaluate_speed(100_001, 1, 0, 50_001), 50_002);
        assert_eq!(
            evaluate_speed(100_000_000_000_000, 0, 0, P),
            -84_467_440_737_095
        );
    }

    #[test]
    fn route_uses_departure_terrain_and_class1_distance_then_signed_progress() {
        let mut frame = frame();
        tokens(&mut frame);
        let content = content();
        let inputs = Inputs::new(&frame, &content);
        assert_eq!(
            remaining(&inputs, &force(), &motion(), &fresh(700), &[2, 3], |from| {
                Ok(if from == 1 { 9 } else { 10 })
            }),
            Ok(315_385)
        );
        let motion = TaskForceMapMotionRow {
            movement_numerator: Recorded::Value(Some(-100_000)),
            ..motion()
        };
        assert_eq!(
            remaining(&inputs, &force(), &motion, &fresh(700), &[2], |_| Ok(9)),
            Ok(192_307)
        );
    }

    #[test]
    fn different_first_and_zero_speed_do_not_reach_progress() {
        let mut frame = frame();
        tokens(&mut frame);
        let content = content();
        let inputs = Inputs::new(&frame, &content);
        let motion = TaskForceMapMotionRow {
            movement_numerator: Recorded::NotRecorded,
            ..motion()
        };
        assert_eq!(
            remaining(&inputs, &force(), &motion, &fresh(700), &[3], |_| Ok(9)),
            Ok(38_461)
        );
        let fresh = FreshTaskForceCounterEta {
            base_speed: Some(0),
            ..fresh(700)
        };
        assert_eq!(
            remaining(&inputs, &force(), &motion, &fresh, &[2, 999], |_| Ok(9)),
            Ok(0)
        );
    }

    #[test]
    fn earlier_division_overflow_is_not_hidden_by_later_zero_speed() {
        let frame = frame();
        let mut content = content();
        let sea = ProvinceTerrain { flags: 0, index: 1 };
        content.map = MovementMap::from_parts(&[
            (
                1,
                sea,
                vec![Neighbour {
                    to: 2,
                    distance: Some(1_i64 << 58),
                    ..Default::default()
                }],
            ),
            (
                2,
                sea,
                vec![Neighbour {
                    to: 3,
                    distance: Some(P),
                    ..Default::default()
                }],
            ),
            (3, sea, vec![]),
        ]);
        content.task_force_terrain = Ok(terrain(
            "categories={custom_water={} other_water={units={movement=-1}}}",
        ));
        let fresh = FreshTaskForceCounterEta {
            base_speed: Some(-1),
            ..fresh(700)
        };
        let refusal = remaining(
            &Inputs::new(&frame, &content),
            &force(),
            &motion(),
            &fresh,
            &[2, 3],
            |from| Ok(if from == 1 { 9 } else { 10 }),
        )
        .unwrap_err();
        assert_eq!(refusal, "ETA native signed division overflows");
    }

    #[test]
    fn duplicate_tokens_and_absent_fresh_components_refuse_reached_speed() {
        let mut frame = frame();
        tokens(&mut frame);
        let content = content();
        for packet in [
            FreshTaskForceCounterEta {
                base_speed: None,
                ..fresh(700)
            },
            FreshTaskForceCounterEta {
                speed_modifier: None,
                ..fresh(700)
            },
            FreshTaskForceCounterEta {
                terrain_class: None,
                ..fresh(700)
            },
        ] {
            assert!(speed(&Inputs::new(&frame, &content), &force(), &packet, 9).is_err());
        }
        put(
            &mut frame,
            TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 700,
                    literal: b"custom_hull".to_vec(),
                },
                LexicalTokenRow {
                    token: 700,
                    literal: b"other_hull".to_vec(),
                },
            ]),
        );
        assert!(speed(&Inputs::new(&frame, &content), &force(), &fresh(700), 9).is_err());
    }
}
