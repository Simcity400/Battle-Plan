//! Class-1 fresh counter duration: speed and integration.
//! Arrival does not enter the task-force speed getter.

use super::fresh_search::{Inputs, available};
use crate::{
    map::raids::reach::Moves,
    table_access::{TokenError, TokenKeys},
};
use engine::cunit::eta_ratio;
use installed_content::{
    regions::StrategicRegion,
    terrain::TerrainDefinitions,
    unit_path::{CounterRequest, Fact, MoveWorld},
};
use replay::rows::{FreshTaskForceCounterEta, TaskForceMapMotionRow, TaskForceRow};
use std::{collections::BTreeMap, sync::Arc};

const P: i64 = 100_000;

/// Shares the existing typed terrain database. Mission/range defines cannot
/// make a task-force departure adjuster unavailable.
#[derive(Debug)]
pub(crate) struct TerrainSpeedContent {
    terrain: Arc<TerrainDefinitions>,
    regions: BTreeMap<u32, Option<usize>>,
    subunits: Fact<TokenKeys>,
}

impl TerrainSpeedContent {
    pub(crate) fn new(
        terrain: Arc<TerrainDefinitions>,
        regions: &BTreeMap<u32, StrategicRegion>,
        subunits: Fact<TokenKeys>,
    ) -> Self {
        // The generic terrain-validity byte is 1 for real categories.
        // Unknown or absent authored terrain is null and uses the five-byte fallback key.
        // The authored naval_terrain boolean is independent of that validity byte.
        let fallback = terrain.category(b"ocean");
        let regions = regions
            .iter()
            .map(|(&id, region)| {
                let selected = region
                    .naval_terrain
                    .as_ref()
                    .and_then(|name| terrain.category(name.as_bytes()))
                    .or(fallback);
                (id, selected)
            })
            .collect();
        Self {
            terrain,
            regions,
            subunits,
        }
    }

    pub(crate) fn movement<'a>(
        &self,
        region: u32,
        class_name: impl FnOnce() -> Fact<Option<&'a str>>,
    ) -> Fact<i64> {
        let selected = self
            .regions
            .get(&region)
            .ok_or_else(|| format!("fresh naval ETA region {region} has no content template"))?;
        let Some(at) = *selected else {
            // The terrain database supplies a null terrain; its common unit adjuster
            // initializes all three values to zero.
            return Ok(0);
        };
        let terrain = &self.terrain.categories[at];
        let common = terrain.common_adjuster.movement;
        if terrain.classes.is_empty() {
            return Ok(common);
        }
        let Some(name) = class_name()? else {
            return Ok(common);
        };
        let Some(at) = terrain
            .classes
            .iter()
            .position(|(key, _)| key.eq_ignore_ascii_case(name.as_bytes()))
        else {
            return Ok(common);
        };
        // Skip an authored class whose sub-unit resolves to null.
        // Registered repeated class tokens keep their first bucket.
        if available(&self.subunits)?.resolve(name)?.is_none() {
            return Ok(common);
        }
        let own = terrain
            .class_adjusters
            .get(at)
            .ok_or("fresh naval ETA terrain class adjuster is unavailable")?
            .movement;
        // Merge common values into each class after load.
        Ok(common.wrapping_add(own))
    }
}

pub(super) fn derive(
    inputs: &Inputs<'_>,
    moves: &Moves<'_, '_>,
    motion: &TaskForceMapMotionRow,
    ours: bool,
    local_option: bool,
) -> Fact<i64> {
    let naval = inputs.naval();
    let motion = naval.motion(motion.task_force)?;
    let force = naval.tf(motion.task_force)?;
    let target = endpoint(force, motion, ours)?;
    derive_to(inputs, moves, force.id, target, local_option).and_then(|preview| preview.duration)
}

pub(super) fn derive_to(
    inputs: &Inputs<'_>,
    moves: &Moves<'_, '_>,
    handle: u64,
    target: u32,
    local_option: bool,
) -> Fact<super::CounterPreview> {
    let naval = inputs.naval();
    let motion = naval.motion(handle)?;
    let force = naval.tf(handle)?;
    // The profile's exile and strategic bytes are native SEARCH operands.
    // They cannot be omitted merely because its duration later is zero.
    let profile = naval.fresh(handle)?;
    naval.route_identities()?;
    let strategic = profile
        .strategic_redeployment
        .ok_or("fresh naval ETA strategic byte was refused")?
        != 0;
    let route = moves.task_force_counter_route(
        force,
        profile,
        target,
        CounterRequest {
            strategic,
            local_option,
            class: Default::default(),
        },
    )?;
    let duration = if route.is_empty() {
        Ok(0)
    } else {
        remaining(inputs, force, motion, profile, &route, |from| {
            moves
                .strategic_region(from)
                .ok_or_else(|| format!("fresh naval ETA departure {from} has no strategic region"))
        })
    };
    Ok(super::CounterPreview { duration, route })
}

fn endpoint(force: &TaskForceRow, motion: &TaskForceMapMotionRow, ours: bool) -> Fact<u32> {
    if force.route_count <= 0 {
        return Err("fresh naval ETA task force has no ordinary route".into());
    }
    if ours && motion.path.len() != force.route_count as usize {
        return Err("fresh naval ETA destination route is incomplete".into());
    }
    let target = if ours {
        motion.path.last()
    } else {
        motion.path.first()
    };
    target
        .copied()
        .filter(|&id| id != 0)
        .ok_or_else(|| "fresh naval ETA ordinary route endpoint is unavailable".into())
}

fn speed(
    inputs: &Inputs<'_>,
    force: &TaskForceRow,
    fresh: &FreshTaskForceCounterEta,
    region: u32,
) -> Fact<i64> {
    replay::fixed_layout::validate_recording(
        inputs.frame(),
        replay::fixed_layout::Family {
            keys: &[
                engine::modifier_keys::NAVAL_SPEED_FACTOR,
                engine::modifier_keys::NAVY_WEATHER_PENALTY,
            ],
            ..replay::fixed_layout::TASK_FORCE_COUNTER_ETA
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
                // A complete reverse dictionary certifies no registered bucket.
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
            // Ship class is a raw lexer token, including signed values.
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

/// The fixed5 product wraps before the division; sum/count alone changes
/// extreme operands. The zero roster does not reach a division.
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
            return Err("fresh naval ETA searched route contains a null province".into());
        }
        if from == to {
            continue;
        }
        // Apply embark distance only to classes 0 and 13.
        let distance =
            inputs.content.map.distance(from, to).ok_or_else(|| {
                format!("fresh naval ETA edge {from}->{to} distance is unavailable")
            })?;
        let speed = speed(inputs, force, fresh, source_region(from)?)?;
        // Native returns immediately, before later edges or actual progress.
        if speed == 0 {
            return Ok(0);
        }
        if from == force.province && actual_speed.is_none() {
            actual_speed = Some(speed);
        }
        // Native divides each leg before reaching the next speed. A later
        // zero cannot hide an earlier signed-division overflow refusal.
        duration = duration.wrapping_add(eta_ratio(distance, speed).map_err(str::to_owned)?);
        from = to;
    }
    if !route.is_empty() && route.first() == motion.path.first() {
        let numerator = motion
            .movement_numerator
            .require("TaskForceMapMotion.movement_numerator")?
            .ok_or("fresh naval ETA movement numerator was refused")?;
        // The speed getter ignores destination, so the first searched edge already
        // evaluates actual-departure speed from this fresh packet.
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
    use crate::map::division_eta::MovementContent;
    use installed_content::movement::{MovementDefines, MovementMap, Neighbour, ProvinceTerrain};
    use replay::{
        frame::{DecodedFrame, TableView},
        rows::{LexicalTokenRow, Recorded, ShipRow, TableId, TableRows, tables},
    };

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
        metadata(
            &mut frame,
            &[
                engine::modifier_keys::NAVAL_SPEED_FACTOR,
                engine::modifier_keys::NAVY_WEATHER_PENALTY,
            ],
        );
        frame
    }

    fn metadata(frame: &mut DecodedFrame, keys: &[u32]) {
        engine::executable_tables::initialize_for_tests()
            .expect("modifier-name tests require BATTLE_PLAN_GAME_DIR with the supported installation");
        let mut definitions = Vec::new();
        let mut tokens = Vec::new();
        for key in engine::modifier_keys::all().filter(|key| keys.contains(&key.index)) {
            definitions.push(replay::rows::ModifierDefinitionRow {
                index: key.index,
                token: key.index + 10_000,
                ..Default::default()
            });
            tokens.push(LexicalTokenRow {
                token: key.index + 10_000,
                literal: key.name.as_bytes().to_vec(),
            });
        }
        definitions.sort_by_key(|row| row.index);
        tokens.sort_by_key(|row| row.token);
        put(frame, TableRows::ModifierDefinitions(definitions));
        put(frame, TableRows::LexicalTokens(tokens));
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

    fn terrain(source: &str) -> TerrainSpeedContent {
        let terrain = Arc::new(installed_content::terrain::parse_definitions(source.as_bytes()).unwrap());
        let regions = BTreeMap::from([
            (
                9,
                StrategicRegion {
                    id: 9,
                    naval_terrain: Some("custom_water".into()),
                    ..Default::default()
                },
            ),
            (
                10,
                StrategicRegion {
                    id: 10,
                    naval_terrain: Some("other_water".into()),
                    ..Default::default()
                },
            ),
            (
                11,
                StrategicRegion {
                    id: 11,
                    naval_terrain: Some("unknown_water".into()),
                    ..Default::default()
                },
            ),
        ]);
        let keys = ["custom_hull".to_owned(), "other_hull".to_owned()];
        TerrainSpeedContent::new(terrain, &regions, Ok(TokenKeys::new(&keys)))
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
            sample_hour: 20,
            base_speed: Some(200_000),
            speed_modifier: Some(0),
            terrain_class: Some(selector),
            ..Default::default()
        }
    }

    #[test]
    fn idle_equal_endpoint_search_needs_range_but_not_speed_metadata_or_operands() {
        use crate::map::raids::{reach::Cells, script::World};
        let (assets, mut frame) = crate::map::raids::counter_fixture();
        let mut content = content();
        content.naval = Ok(installed_content::movement::NavalMovementDefines {
            range_scale: P,
            use_home_base: Err("unreached nonpositive-range policy".into()),
        });
        let range_keys = [
            engine::modifier_keys::NAVAL_RANGE,
            engine::modifier_keys::NAVAL_RANGE_FACTOR,
            engine::modifier_keys::NAVY_WEATHER_PENALTY,
            engine::modifier_keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR,
        ];
        metadata(&mut frame, &range_keys);
        let force = TaskForceRow {
            id: 7,
            province: 1,
            roster_owner: 11,
            logical_owner: 11,
            route_count: 0,
            ..Default::default()
        };
        let profile = FreshTaskForceCounterEta {
            sample_hour: frame.hour,
            modified_range: Some(0),
            strategic_redeployment: Some(0),
            exiled: Some(0),
            // Base speed, modifier and class selector are deliberately absent.
            ..Default::default()
        };
        put(&mut frame, TableRows::TaskForces(vec![force.clone()]));
        put(
            &mut frame,
            TableRows::TaskForceMapMotion(vec![TaskForceMapMotionRow {
                task_force: 7,
                fresh: Recorded::Value(Some(profile)),
                ..Default::default()
            }]),
        );
        {
            let inputs = Inputs::new(&frame, &content);
            let world = World::read(&frame, assets.facts());
            let cells = Cells::default();
            let moves = Moves::for_counter(&world, &assets, &cells, &inputs);
            let preview = derive_to(&inputs, &moves, 7, 1, false).unwrap();
            assert!(preview.route.is_empty());
            assert_eq!(preview.duration, Ok(0));
            // A reached speed call still authenticates its missing own key.
            assert!(
                speed(&inputs, &force, &profile, 9)
                    .unwrap_err()
                    .contains("naval_speed_factor")
            );
        }
        // The same empty search cannot bypass its genuine range metadata.
        metadata(&mut frame, &range_keys[1..]);
        let inputs = Inputs::new(&frame, &content);
        let world = World::read(&frame, assets.facts());
        let cells = Cells::default();
        let moves = Moves::for_counter(&world, &assets, &cells, &inputs);
        assert_eq!(
            derive_to(&inputs, &moves, 7, 1, false).unwrap_err(),
            "fresh task-force counter ETA refused: navy_max_range (0x8a): definition row missing"
        );
    }

    #[test]
    fn destination_requires_complete_route_but_next_keeps_a_prefix() {
        let force = force();
        let motion = TaskForceMapMotionRow {
            path: vec![2],
            ..motion()
        };
        assert_eq!(endpoint(&force, &motion, false), Ok(2));
        assert!(
            endpoint(&force, &motion, true)
                .unwrap_err()
                .contains("incomplete")
        );
        for count in [0, -1] {
            assert!(
                endpoint(
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
            endpoint(
                &force,
                &TaskForceMapMotionRow {
                    path: vec![0, 3],
                    ..motion.clone()
                },
                false
            )
            .is_err()
        );
    }

    #[test]
    fn terrain_uses_first_registered_class_plus_common_and_native_fallback() {
        let mut terrain = terrain(
            "categories={custom_water={naval_terrain=no units={movement=0.1} custom_hull={units={movement=0.2}} custom_hull={units={movement=0.8}} ghost_hull={units={movement=2}}} ocean={units={movement=-0.4}}}",
        );
        assert_eq!(terrain.movement(9, || Ok(Some("CUSTOM_HULL"))), Ok(30_000));
        assert_eq!(terrain.movement(9, || Ok(Some("ghost_hull"))), Ok(10_000));
        assert_eq!(
            terrain.movement(11, || Err("unreached class".into())),
            Ok(-40_000)
        );
        assert!(
            terrain
                .movement(99, || Ok(None))
                .unwrap_err()
                .contains("no content template")
        );
        terrain.subunits = Err("sub-unit domain refused".into());
        assert_eq!(terrain.movement(9, || Ok(Some("no_bucket"))), Ok(10_000));
        assert!(terrain.movement(9, || Ok(Some("custom_hull"))).is_err());
    }

    #[test]
    fn missing_ocean_is_native_null_terrain_with_zero_adjuster() {
        let terrain = terrain("categories={custom_water={units={movement=0.2}}}");
        assert_eq!(
            terrain.movement(11, || Err("unreached class".into())),
            Ok(0)
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
        // The sea->land leg keeps 400000, not the content embark value9000000.
        // Per-leg durations are 153846 + 200000; progress is 38461.
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
