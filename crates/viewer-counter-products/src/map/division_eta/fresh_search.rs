
use super::MovementContent;
use engine_railways::crailway_manager::fresh_inputs;
use content_reader_unit_path::{unit_path::CounterRequest, unit_path::Fact};
use replay_data::{
    rows::DivisionMovementRow, rows::DivisionRow, rows::ProvinceRow, rows::RailCooldownRow,
    rows::TableSchema, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use viewer_unit_reads::movement_reads::Moves;
use viewer_foundation::movement_timing::{HourTerms, operands};

pub(super) struct Inputs<'a> {
    pub(super) frame: &'a DecodedFrame,
    pub(super) hour: i32,
    pub(super) content: &'a MovementContent,
    naval: OnceLock<super::naval::NavalInputs<'a>>,
    railway: OnceLock<Fact<BTreeMap<u64, &'a replay_data::rows::RailwayGunCounterInputRow>>>,
    movement: OnceLock<Fact<BTreeMap<u64, &'a DivisionMovementRow>>>,
    divisions: Fact<BTreeMap<u64, &'a DivisionRow>>,
    provinces: Fact<BTreeMap<u32, &'a ProvinceRow>>,
    links: Fact<BTreeMap<u32, u32>>,
    countries: Fact<BTreeMap<u32, u32>>,
    cooldowns: Fact<BTreeMap<u32, &'a RailCooldownRow>>,
    edges: Fact<BTreeMap<(u32, u32), i32>>,
}

pub(super) fn index<T: TableSchema, K: Ord>(
    frame: &DecodedFrame,
    key: impl Fn(&T::Row) -> K,
) -> Fact<BTreeMap<K, &T::Row>> {
    let rows = frame.complete::<T>().map_err(|error| error.to_string())?;
    let mut indexed = BTreeMap::new();
    for row in rows {
        if indexed.insert(key(row), row).is_some() {
            return Err(format!("fresh ETA {:?} contains duplicate keys", T::ID));
        }
    }
    Ok(indexed)
}

pub(super) fn available<T>(domain: &Fact<T>) -> Fact<&T> {
    domain.as_ref().map_err(Clone::clone)
}

impl<'a> Inputs<'a> {
    pub(super) fn frame(&self) -> &'a DecodedFrame {
        self.frame
    }
    pub(super) fn new(frame: &'a DecodedFrame, content: &'a MovementContent) -> Self {
        let links = index::<tables::CountryLinks, _>(frame, |row| row.id).map(|rows| {
            rows.into_iter()
                .map(|(id, row)| (id, row.canonical_slot))
                .collect()
        });
        let countries = index::<tables::Countries, _>(frame, |row| row.id).and_then(|rows| {
            let mut slots = BTreeMap::new();
            for (id, row) in rows {
                if slots.insert(row.canonical_slot, id).is_some() {
                    return Err("fresh ETA Countries contains duplicate keys".into());
                }
            }
            Ok(slots)
        });
        let edges = index::<tables::RailEdges, _>(frame, |row| (row.from, row.to)).map(|rows| {
            rows.into_iter()
                .map(|(edge, row)| (edge, row.level))
                .collect()
        });
        Self {
            frame,
            hour: frame.hour,
            content,
            naval: OnceLock::new(),
            railway: OnceLock::new(),
            movement: OnceLock::new(),
            divisions: index::<tables::Divisions, _>(frame, |row| row.id),
            provinces: index::<tables::Provinces, _>(frame, |row| row.id),
            links,
            countries,
            cooldowns: index::<tables::RailCooldowns, _>(frame, |row| row.province),
            edges,
        }
    }

    pub(super) fn route_identities(&self) -> Fact<()> {
        available(&self.provinces)?;
        available(&self.links)?;
        available(&self.countries)?;
        Ok(())
    }

    pub(super) fn directed_rail_percentage(&self, from: u32, to: u32) -> Fact<i64> {
        let defines = available(&self.content.rail)?;
        if let Some(row) = available(&self.cooldowns)?.get(&from) {
            let edge = if row.cooldown_ticks <= 0 {
                available(&self.edges)?.get(&(from, to)).copied()
            } else {
                None
            };
            let health = if row.cooldown_ticks <= 0 && edge.is_some() {
                (*row
                    .building
                    .require("RailCooldowns.building")?)
                .map(|state| fresh_inputs::RailwayBuildingState {
                    building: state
                        .building
                        .map(|health| fresh_inputs::RailwayBuildingHealth {
                            level: health.level,
                            healthy: health.healthy,
                            partial: health.partial,
                        }),
                })
            } else {
                None
            };
            fresh_inputs::directed_percentage(
                row.cooldown_ticks,
                edge,
                health,
                defines.maximum_level,
            )
            .map_err(str::to_owned)
        } else {
            Ok(0)
        }
    }

    pub(super) fn naval(&self) -> &super::naval::NavalInputs<'a> {
        self.naval
            .get_or_init(|| super::naval::NavalInputs::new(self.frame))
    }

    pub(super) fn railway(
        &self,
        handle: u64,
    ) -> Fact<&'a replay_data::rows::RailwayGunCounterInputRow> {
        available(self.railway.get_or_init(|| {
            index::<tables::RailwayGunCounterInputs, _>(self.frame, |row| row.gun)
        }))?
        .get(&handle)
        .copied()
        .ok_or_else(|| format!("fresh ETA railway gun {handle} is not recorded"))
    }
}

impl viewer_unit_reads::movement_reads::CounterFacts for Inputs<'_> {
    fn rail_connected(&self, from: u32, to: u32) -> Fact<bool> {
        for province in [from, to] {
            if self
                .content
                .map
                .terrain(province)
                .ok_or_else(|| format!("fresh ETA province {province} has no content terrain"))?
                .flags
                & 1
                == 0
            {
                return Ok(false);
            }
        }
        let nodes = available(&self.cooldowns)?;
        let (Some(source), Some(target)) = (nodes.get(&from), nodes.get(&to)) else {
            return Ok(false);
        };
        for row in [source, target] {
            if row.cooldown_ticks != 0 {
                return Ok(false);
            }
            let selected = row
                .building
                .require("RailCooldowns.building")?
                .as_ref()
                .ok_or("fresh ETA railway selected building was refused")?;
            if selected.building.is_none_or(|health| health.level <= 0) {
                return Ok(false);
            }
        }
        Ok(available(&self.edges)?.contains_key(&(from, to)))
    }

    fn rail_orders_group(&self, handle: u64) -> Fact<u64> {
        let fresh = self
            .railway(handle)?
            .fresh
            .require("RailwayGunCounterInputs.fresh")?
            .as_ref()
            .ok_or("fresh ETA railway inputs were refused")?;
        if !fresh.sample_hour.fresh_at(self.hour) {
            return Err("fresh ETA railway group is not from the frame hour".into());
        }
        fresh
            .orders_group
            .ok_or_else(|| "fresh ETA railway orders group was refused".into())
    }

    fn naval_access(&self, _: &Moves<'_, '_>, _: u64, _: u32) -> Fact<bool> {
        Err(super::task_force::ADMISSION_UNRECORDED.into())
    }
    fn all_submarine(&self, _: u64) -> Fact<bool> {
        Err(super::task_force::ADMISSION_UNRECORDED.into())
    }
    fn hostile_allows(&self, _: &Moves<'_, '_>, _: u64, _: u32) -> Fact<bool> {
        Err(super::task_force::ADMISSION_UNRECORDED.into())
    }
}

impl<'a> Inputs<'a> {
    pub(super) fn movement(&self, handle: u64) -> Fact<&'a DivisionMovementRow> {
        available(
            self.movement.get_or_init(|| {
                index::<tables::DivisionMovement, _>(self.frame, |row| row.division)
            }),
        )?
        .get(&handle)
        .copied()
        .ok_or_else(|| format!("fresh ETA division {handle:#x} has no DivisionMovement row"))
    }

    pub(super) fn derive_to(
        &self,
        moves: &Moves<'_, '_>,
        handle: u64,
        target: u32,
        local_option: bool,
    ) -> Fact<super::CounterPreview> {
        let movement = self.movement(handle)?;
        let division = *available(&self.divisions)?
            .get(&handle)
            .ok_or_else(|| format!("fresh ETA division {handle:#x} is not recorded"))?;
        self.route_identities()?;
        let route = moves.counter_route(
            handle,
            target,
            CounterRequest {
                strategic: movement.strategic_redeployment != 0,
                local_option,
                class: Default::default(),
            },
        )?;
        let duration = if route.is_empty() {
            Ok(0)
        } else {
            let map = &self.content.map;
            let defines = &self.content.defines;
            HourTerms::new(self.frame).with_route_terms(
                division,
                movement,
                &route,
                self.content,
                defines,
                |terms| {
                    replay_derive::eta_derive::estimate_route(
                        movement,
                        division.province,
                        &route,
                        |id| operands(map, id),
                        |from, to| map.distance(from, to),
                        terms,
                        defines,
                    )
                },
            )
        };
        Ok(super::CounterPreview { duration, route })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_map::movement::{MovementMap, Neighbour, ProvinceTerrain, RailMovementDefines};
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::RailEdgeRow, rows::RailwayBuildingHealth,
        rows::RailwayBuildingState, rows::Recorded, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 20,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture domain unavailable".into(),
                }
                .into()
            }),
            metrics: Metrics::default(),
        }
    }

    fn put(frame: &mut DecodedFrame, rows: TableRows) {
        let table = rows.table();
        *frame.tables[table.index()] = TableView::Complete(rows);
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
                        distance: Some(400_000),
                        ..Default::default()
                    }],
                ),
                (2, ProvinceTerrain { flags: 1, index: 2 }, vec![]),
            ]),
            defines: replay_derive::eta_derive::Defines {
                embark_distance: 400_000,
                transport_scale: 100_000,
                outer_scale: 100_000,
                ..Default::default()
            },
            rail: Ok(RailMovementDefines {
                speed: [300_000, 0, 0, 0],
                maximum_level: 5,
            }),
            task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
            land: Err("unrelated land movement terrain is unavailable".into()),
        }
    }

    #[test]
    fn idle_explicit_target_times_the_searched_route_at_the_edge_speed() {
        use content_reader_map::province_graph::ProvinceGraph;
        use content_reader_unit_path::unit_path::MoveGraph;
        use dashboard_text::triggers::World;
        use replay_data::rows::DivisionTemplateIconRow;
        use viewer_unit_reads::movement_reads::{
            Cells,
            test_support::{positive_frame, unnamed_assets},
        };

        let mut assets = unnamed_assets();
        assets.geometry.land = [3, 4].into();
        let mut graph = ProvinceGraph::default();
        graph.edges.insert(3, vec![(4, Some(50_000))]);
        graph.edges.insert(4, vec![(3, Some(50_000))]);
        assets.moves = Ok(MoveGraph::new(&graph, &assets.geometry));
        let mut frame = positive_frame();
        let mut content = content(1);
        content.map = MovementMap::from_parts(&[
            (
                3,
                ProvinceTerrain { flags: 1, index: 1 },
                vec![Neighbour {
                    to: 4,
                    distance: Some(50_000),
                    ..Default::default()
                }],
            ),
            (4, ProvinceTerrain { flags: 1, index: 2 }, vec![]),
        ]);
        content.land = Ok(viewer_foundation::movement_timing::LandSpeedContent {
            terrain_movement: vec![0, 0],
            regions: Default::default(),
            ..Default::default()
        });
        super::super::test_support::neutral_land_movement(&mut frame, 11, 3);
        put(
            &mut frame,
            TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
                template: 0,
                speed_keys: Recorded::Value(Some(Vec::new())),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 7,
                province: 3,
                logical_owner: 11,
                roster_owner: 11,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::DivisionStatus(vec![
                viewer_foundation::movement_timing::test_support::neutral_status(7, 2),
            ]),
        );
        let idle = DivisionMovementRow {
            division: 7,
            source: 3,
            target: 3,
            withdrawal: Recorded::Value(Some(0)),
            fuel: Recorded::Value(Some(0)),
            ..Default::default()
        };
        for (base_speed, expected) in [
            (
                None,
                Err("the division's base speed is not recorded".to_owned()),
            ),
            (Some(200_000), Ok(25_000)),
        ] {
            put(
                &mut frame,
                TableRows::DivisionMovement(vec![DivisionMovementRow {
                    own_base_speed: Some(base_speed),
                    ..idle.clone()
                }]),
            );
            let inputs = Inputs::new(&frame, &content);
            let world = World::read(&frame, assets.trigger_content.facts());
            let cells = Cells::default();
            let moves = Moves::for_counter(&world, assets.movement_reads(), &cells, &inputs);
            let preview = inputs.derive_to(&moves, 7, 4, false).unwrap();
            assert_eq!(preview.route, [4]);
            assert_eq!(preview.duration, expected);
            let same = inputs.derive_to(&moves, 7, 3, false).unwrap();
            assert!(same.route.is_empty());
            assert_eq!(same.duration, Ok(0));
        }
    }

    fn country_frame() -> DecodedFrame {
        let mut frame = frame();
        put(&mut frame, TableRows::Provinces(vec![]));
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 99,
                canonical_slot: 1,
            }]),
        );
        put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 101,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        frame
    }

    #[test]
    fn rail_admission_requires_zero_ticks_selected_positive_levels_and_direction() {
        let mut frame = frame();
        let content = content(1);
        put(
            &mut frame,
            TableRows::RailEdges(vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: 1,
            }]),
        );
        for (ticks, level, expected) in [(0, 1, true), (-1, 1, false), (1, 1, false), (0, 0, false)]
        {
            put(
                &mut frame,
                TableRows::RailCooldowns(
                    [1, 2]
                        .map(|province| RailCooldownRow {
                            province,
                            cooldown_ticks: ticks,
                            building: Recorded::Value(Some(RailwayBuildingState {
                                building: Some(RailwayBuildingHealth {
                                    level,
                                    healthy: level,
                                    partial: None,
                                }),
                            })),
                            selection: Default::default(),
                            selection_hour: Default::default(),
                            building_bound: Default::default(),
                        })
                        .to_vec(),
                ),
            );
            let inputs = Inputs::new(&frame, &content);
            assert_eq!(
                viewer_unit_reads::movement_reads::CounterFacts::rail_connected(&inputs, 1, 2),
                Ok(expected)
            );
            assert_eq!(
                viewer_unit_reads::movement_reads::CounterFacts::rail_connected(&inputs, 2, 1),
                Ok(false)
            );
        }
    }

    #[test]
    fn rail_admission_does_not_read_nodes_for_sea_or_health_without_both_nodes() {
        let mut frame = frame();
        let sea = content(0);
        assert_eq!(
            viewer_unit_reads::movement_reads::CounterFacts::rail_connected(
                &Inputs::new(&frame, &sea),
                1,
                2
            ),
            Ok(false),
        );
        put(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 0,
                building: Recorded::NotRecorded,
                selection: Default::default(),
                selection_hour: Default::default(),
                building_bound: Default::default(),
            }]),
        );
        let land = content(1);
        assert_eq!(
            viewer_unit_reads::movement_reads::CounterFacts::rail_connected(
                &Inputs::new(&frame, &land),
                1,
                2
            ),
            Ok(false),
        );
    }

    #[test]
    fn task_force_admission_facts_refuse_by_name() {
        use viewer_unit_reads::movement_reads::CounterFacts;
        let frame = frame();
        let content = content(0);
        let inputs = Inputs::new(&frame, &content);
        assert_eq!(
            inputs.all_submarine(7),
            Err(super::super::task_force::ADMISSION_UNRECORDED.into())
        );
    }

    #[test]
    fn rail_health_absence_is_consumed_only_on_an_open_directed_edge() {
        let mut frame = frame();
        put(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 1,
                building: Recorded::NotRecorded,
                selection: Default::default(),
                selection_hour: Default::default(),
                building_bound: Default::default(),
            }]),
        );
        let content = content(1);
        assert_eq!(
            Inputs::new(&frame, &content).directed_rail_percentage(1, 2),
            Ok(0)
        );
        *frame.tables[TableId::RailCooldowns.index()] =
            TableView::Complete(TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 0,
                building: Recorded::NotRecorded,
                selection: Default::default(),
                selection_hour: Default::default(),
                building_bound: Default::default(),
            }]));
        put(
            &mut frame,
            TableRows::RailEdges(vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: 5,
            }]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).directed_rail_percentage(1, 2),
            Err("RailCooldowns.building not recorded".into())
        );
    }

    #[test]
    fn observed_null_rail_building_is_zero_but_missing_selection_is_refused() {
        let mut frame = frame();
        put(
            &mut frame,
            TableRows::RailEdges(vec![RailEdgeRow {
                from: 1,
                to: 2,
                level: 5,
            }]),
        );
        let content = content(1);
        for (building, expected) in [
            (Some(RailwayBuildingState { building: None }), Ok(0)),
            (
                None,
                Err("fresh railway building health unavailable".into()),
            ),
        ] {
            put(
                &mut frame,
                TableRows::RailCooldowns(vec![RailCooldownRow {
                    province: 1,
                    cooldown_ticks: 0,
                    building: Recorded::Value(building),
                    selection: Default::default(),
                    selection_hour: Default::default(),
                    building_bound: Default::default(),
                }]),
            );
            assert_eq!(
                Inputs::new(&frame, &content).directed_rail_percentage(1, 2),
                expected
            );
        }
    }

    #[test]
    fn row_indexes_refuse_duplicate_aliases_slots_and_directed_edges() {
        synthetic_game::enter();
        let content = content(0);
        let mut frame = country_frame();
        put(
            &mut frame,
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 2,
                },
            ]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).route_identities(),
            Err("fresh ETA CountryLinks contains duplicate keys".into())
        );

        let mut frame = country_frame();
        put(
            &mut frame,
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    ..Default::default()
                },
                CountryRow {
                    id: 102,
                    canonical_slot: 1,
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).route_identities(),
            Err("fresh ETA Countries contains duplicate keys".into())
        );

        put(
            &mut frame,
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    ..Default::default()
                },
                CountryRow {
                    id: 101,
                    canonical_slot: 2,
                    ..Default::default()
                },
            ]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).countries,
            Err("fresh ETA Countries contains duplicate keys".into())
        );

        let mut frame = self::frame();
        put(
            &mut frame,
            TableRows::RailCooldowns(vec![RailCooldownRow {
                province: 1,
                cooldown_ticks: 0,
                building: Recorded::NotRecorded,
                selection: Default::default(),
                selection_hour: Default::default(),
                building_bound: Default::default(),
            }]),
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
                    from: 1,
                    to: 2,
                    level: 5,
                },
            ]),
        );
        assert_eq!(
            Inputs::new(&frame, &content).directed_rail_percentage(1, 2),
            Err("fresh ETA RailEdges contains duplicate keys".into())
        );
    }

    #[test]
    fn route_identity_gate_refuses_incomplete_and_duplicate_core_domains_only() {
        let content = content(1);
        let mut frame = country_frame();
        put(&mut frame, TableRows::Provinces(vec![]));
        assert_eq!(Inputs::new(&frame, &content).route_identities(), Ok(()));
        for rows in [
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 2,
                    ..Default::default()
                };
                2
            ]),
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 99,
                    canonical_slot: 1
                };
                2
            ]),
            TableRows::Countries(vec![
                CountryRow {
                    id: 101,
                    canonical_slot: 1,
                    ..Default::default()
                };
                2
            ]),
        ] {
            let table = rows.table();
            let original = frame.tables[table.index()].clone();
            put(&mut frame, rows);
            assert_eq!(
                Inputs::new(&frame, &content).route_identities(),
                Err(format!("fresh ETA {table:?} contains duplicate keys"))
            );
            frame.tables[table.index()] = TableView::Unavailable {
                reason: "fixture core gap".into(),
            }
            .into();
            assert!(
                Inputs::new(&frame, &content)
                    .route_identities()
                    .unwrap_err()
                    .contains("fixture core gap")
            );
            frame.tables[table.index()] = original;
        }
    }
}
