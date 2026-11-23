
use std::collections::BTreeMap;

use content_reader_map::movement::MovementMap;
use replay_data::{rows::DivisionRow, rows::Recorded, rows::tables};
use replay_derive::{
    eta_derive, eta_derive::Defines, eta_derive::OrganisationDefines, eta_derive::RailDefines,
    eta_derive::frame_terms::FrameTerms, eta_derive::frame_terms::Geography,
    eta_derive::frame_terms::SubUnitContent,
};
use replay_frame::frame::DecodedFrame;

pub struct Content {
    pub map: MovementMap,
    pub defines: Defines,
    pub terrain_movement: Vec<i64>,
    pub regions: BTreeMap<u32, u32>,
    pub sub_units: Result<content_reader_script::sub_units::SubUnits, String>,
}

impl Content {
    pub fn read(layers: &content_reader_layers::ResolvedLayers) -> std::io::Result<Self> {
        let defines = content_reader_defines::defines::read(layers)?;
        let movement = content_reader_map::movement::MovementDefines::read(&defines)?;
        let terrain = content_reader_key_names::terrain::read_definitions(layers)?;
        let regions = content_reader_regions::regions::read(layers)?;
        Ok(Self {
            map: MovementMap::read(layers, &defines)?,
            defines: Defines {
                embark_distance: movement.embark_distance,
                transport_scale: movement.transport_scale,
                invasion_speed: movement.invasion_speed,
                outer_scale: movement.outer_scale,
                river_crossing_two: movement.river_crossing_two,
                river_crossing_three: movement.river_crossing_three,
                organisation: movement
                    .organisation
                    .ok()
                    .map(|(threshold, penalty)| OrganisationDefines { threshold, penalty }),
                rail: movement.rail.as_ref().ok().map(|rail| RailDefines {
                    speed: rail.speed,
                    maximum_level: rail.maximum_level,
                }),
                noncore_supply: movement
                    .noncore_supply
                    .each_ref()
                    .map(|value| value.as_ref().ok().copied()),
                combat_movement: movement.combat_movement.as_ref().ok().copied(),
            },
            terrain_movement: terrain
                .categories
                .iter()
                .map(|category| category.common_adjuster.movement)
                .collect(),
            regions: regions
                .regions
                .iter()
                .flat_map(|(&id, region)| region.provinces.iter().map(move |&p| (p, id)))
                .collect(),
            sub_units: content_reader_key_names::unit_leader::sub_units(layers)
                .map_err(|error| error.to_string()),
        })
    }
}

impl SubUnitContent for Content {
    fn special_forces(&self, name: &[u8]) -> Result<bool, String> {
        self.sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .special_forces(name)
    }

    fn sub_unit_types(&self, name: &[u8]) -> Result<u64, String> {
        self.sub_units
            .as_ref()
            .map_err(Clone::clone)?
            .category(name)
    }
}

impl Geography for Content {
    fn terrain_index(&self, province: u32) -> Option<i32> {
        self.map.terrain(province).map(|terrain| terrain.index)
    }

    fn terrain_flags(&self, province: u32) -> Option<u8> {
        self.map.terrain(province).map(|terrain| terrain.flags)
    }

    fn terrain_movement(&self, index: i32) -> Option<i64> {
        usize::try_from(index)
            .ok()
            .and_then(|index| index.checked_sub(1))
            .and_then(|at| self.terrain_movement.get(at))
            .copied()
    }

    fn crossing_kind(&self, from: u32, to: u32) -> Option<u8> {
        self.map.crossing_kind(from, to)
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.regions.get(&province).copied()
    }
}

pub fn tables() -> impl Iterator<Item = replay_data::rows::TableId> {
    use replay_data::{rows::TableId::DivisionMovement, rows::TableId::Divisions};
    [Divisions, DivisionMovement]
        .into_iter()
        .chain(replay_derive::eta_derive::frame_terms::TABLES)
}

struct Edge {
    location: u32,
    first: u32,
    path: Vec<u32>,
    numerator: i64,
    speed: Result<i64, String>,
    distance: Option<i64>,
    key: MovementRowKey,
}

#[derive(Default)]
pub struct Checker {
    content: Option<Result<Content, String>>,
    previous: Option<(replay_data::BranchId, i32, BTreeMap<u64, Edge>)>,
    pub matched: u64,
    pub matched_later: u64,
    pub matched_clipped: u64,
    pub differed: u64,
    pub stalled: u64,
    pub refused: BTreeMap<String, u64>,
    listed: Vec<String>,
    differences: BTreeMap<String, u64>,
}

impl Checker {
    pub fn new(content: Result<Content, String>) -> Self {
        Self {
            content: Some(content),
            ..Self::default()
        }
    }

    pub fn observe(&mut self, frame: &DecodedFrame) {
        let Some(Ok(content)) = self.content.as_ref() else {
            return;
        };
        let current = edges(frame, content);
        if let Some((branch, hour, previous)) = self.previous.take()
            && branch == frame.branch
            && hour.checked_add(1) == Some(frame.hour)
        {
            for (division, before) in &previous {
                let Some(after) = current.get(division) else {
                    continue;
                };
                if after.location != before.location
                    || after.first != before.first
                    || after.path != before.path
                    || after.numerator < before.numerator
                {
                    continue;
                }
                let step = after.numerator.wrapping_sub(before.numerator);
                match &before.speed {
                    Err(reason) => *self.refused.entry(reason.clone()).or_default() += 1,
                    Ok(_) if step == 0 => self.stalled += 1,
                    Ok(speed)
                        if progress_matches(
                            before.numerator,
                            after.numerator,
                            *speed,
                            before.distance,
                        ) =>
                    {
                        self.matched += 1;
                        self.matched_clipped += u64::from(*speed != step);
                    }
                    Ok(_)
                        if after.speed.as_ref().is_ok_and(|speed| {
                            progress_matches(
                                before.numerator,
                                after.numerator,
                                *speed,
                                before.distance,
                            )
                        }) =>
                    {
                        self.matched_later += 1;
                        self.matched_clipped +=
                            u64::from(after.speed.as_ref().is_ok_and(|speed| *speed != step));
                    }
                    Ok(speed) => {
                        self.differed += 1;
                        let kind = classify(frame, *division, &before.key);
                        *self.differences.entry(kind.clone()).or_default() += 1;
                        if self.listed.len() < super::LISTED {
                            self.listed.push(format!(
                                "division {division:#x} hour {}: derived {speed}, stepped {step} ({kind})",
                                frame.hour - 1
                            ));
                        }
                    }
                }
            }
        }
        self.previous = Some((frame.branch, frame.hour, current));
    }

    pub fn print(&self) {
        match &self.content {
            Some(Err(reason)) => println!("division movement speed: content refused: {reason}"),
            _ => {
                println!(
                    "division movement speed against the next hour's numerator step: {} equal, {} equal with the next hour's terms only, {} differing, {} stalled, {} refused",
                    self.matched,
                    self.matched_later,
                    self.differed,
                    self.stalled,
                    self.refused.values().sum::<u64>()
                );
                for (kind, count) in &self.differences {
                    println!("  differing with {kind}: {count}");
                }
                println!(
                    "  equal steps clipped at the first-edge distance: {}",
                    self.matched_clipped
                );
                for (reason, count) in &self.refused {
                    println!("  refused ({reason}): {count}");
                }
                for line in &self.listed {
                    println!("  difference: {line}");
                }
            }
        }
    }

    pub fn failures(&self) -> [u64; 2] {
        match &self.content {
            Some(Err(_)) => [0, 1],
            _ => [0, 0],
        }
    }
}

fn progress_matches(before: i64, after: i64, speed: i64, distance: Option<i64>) -> bool {
    let advanced = before.wrapping_add(speed).max(0);
    after == advanced || distance.is_some_and(|distance| after == distance && advanced >= distance)
}

fn classify(frame: &DecodedFrame, division: u64, movement: &MovementRowKey) -> String {
    let status = frame
        .complete::<tables::DivisionStatus>()
        .ok()
        .and_then(|rows| {
            rows.binary_search_by_key(&division, |row| row.division)
                .ok()
                .map(|at| &rows[at])
        });
    let mut terms = Vec::new();
    match status {
        Some(status) => {
            if matches!(status.out_of_supply_days, Recorded::Value(Some(days)) if days != 0) {
                terms.push("days out of supply");
            }
            if status
                .recorded_supply_ratio_bounds()
                .is_some_and(|[_, high]| high < 100_000)
            {
                terms.push("partial supply");
            }
        }
        None => terms.push("no status"),
    }
    if movement.fuel_limited {
        terms.push("fuel");
    }
    if movement.retreat {
        terms.push("retreat");
    }
    if let Some(status) = status {
        match &status.commander_modifiers {
            Recorded::Value(Some(entries)) if entries[1].is_some() => {
                terms.push("commander armour speed entry");
            }
            Recorded::Value(Some(entries)) if entries.iter().any(Option::is_some) => {
                terms.push("commander");
            }
            _ => {}
        }
    }
    if terms.is_empty() {
        "none of these".into()
    } else {
        terms.join(", ")
    }
}

struct MovementRowKey {
    fuel_limited: bool,
    retreat: bool,
}

fn edges(frame: &DecodedFrame, content: &Content) -> BTreeMap<u64, Edge> {
    let mut edges = BTreeMap::new();
    let (Ok(movement), Ok(divisions)) = (
        frame.complete::<tables::DivisionMovement>(),
        frame.complete::<tables::Divisions>(),
    ) else {
        return edges;
    };
    let divisions: BTreeMap<u64, &DivisionRow> =
        divisions.iter().map(|row| (row.id, row)).collect();
    let terms = FrameTerms::new(frame);
    for row in movement {
        let (Some(&first), Some(division)) = (row.path.first(), divisions.get(&row.division))
        else {
            continue;
        };
        if row.path_total == 0 || division.province == 0 || first == 0 {
            continue;
        }
        let mut fuel_limited = false;
        let speed = terms.with_terms(division, row, content, &content.defines, |terms| {
            let speed = eta_derive::edge_speed_with(
                || eta_derive::base_speed(terms),
                division.province,
                first,
                terms,
                &content.defines,
            )?;
            if terms.fuel.is_some() {
                let without_fuel = eta_derive::SpeedTerms {
                    modifier: terms.modifier,
                    edge: terms.edge,
                    organisation: terms.organisation,
                    fuel: None,
                    base_speed: terms.base_speed,
                };
                fuel_limited = eta_derive::edge_speed_with(
                    || eta_derive::base_speed(&without_fuel),
                    division.province,
                    first,
                    &without_fuel,
                    &content.defines,
                )? != speed;
            }
            Ok(speed)
        });
        edges.insert(
            row.division,
            Edge {
                location: division.province,
                first,
                path: row.path.clone(),
                numerator: row.numerator,
                speed,
                distance: content
                    .map
                    .terrain(division.province)
                    .zip(content.map.terrain(first))
                    .and_then(|(from, to)| {
                        if from.flags & 3 == 0 && to.flags & 1 != 0 {
                            Some(content.defines.embark_distance)
                        } else {
                            content.map.distance(division.province, first)
                        }
                    }),
                key: MovementRowKey {
                    fuel_limited,
                    retreat: row.retreat != 0,
                },
            },
        );
    }
    edges
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cross_hour_progress_matches_native_clipping_without_a_tolerance() {
        assert!(progress_matches(
            1_016_240,
            1_016_666,
            5_000,
            Some(1_016_666)
        ));
        assert!(!progress_matches(
            1_016_240,
            1_016_665,
            5_000,
            Some(1_016_666)
        ));
        assert!(!progress_matches(
            1_010_000,
            1_016_666,
            5_000,
            Some(1_016_666)
        ));
        assert!(progress_matches(
            1_010_000,
            1_015_000,
            5_000,
            Some(1_016_666)
        ));
        assert!(!progress_matches(1_016_240, 1_016_666, 5_000, None));
    }
    fn checker() -> Checker {
        Checker::new(Ok(Content {
            map: MovementMap::default(),
            defines: Defines::default(),
            terrain_movement: Vec::new(),
            regions: BTreeMap::new(),
            sub_units: Err("unused".into()),
        }))
    }
    fn moving(branch: u64, hour: i32) -> DecodedFrame {
        use replay_data::rows::{DivisionMovementRow, TableId, TableRows};
        use replay_frame::frame::{Metrics, TableView};
        let mut frame = DecodedFrame {
            branch,
            hour,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                id: 7,
                province: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::DivisionMovement.index()] =
            TableView::Complete(TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: 7,
                path: vec![2],
                path_total: 1,
                numerator: 10,
                ..Default::default()
            }]));
        frame
    }
    #[test]
    fn a_step_pairs_only_consecutive_hours_of_one_branch() {
        let mut checker = checker();
        checker.observe(&moving(1, 10));
        checker.observe(&moving(2, 11));
        assert_eq!(checker.refused.values().sum::<u64>(), 0);
        checker.observe(&moving(2, 12));
        assert_eq!(checker.refused.values().sum::<u64>(), 1);
        checker.observe(&moving(2, i32::MAX));
        checker.observe(&moving(2, i32::MIN));
        assert_eq!(checker.refused.values().sum::<u64>(), 1);
    }
    #[test]
    fn a_naval_transfer_edge_is_derived_without_the_base_speed() {
        use replay_data::rows::{DivisionMovementRow, OwnerTransportSpeedRow, TableId, TableRows};
        use replay_frame::frame::TableView;
        let mut frame = moving(1, 10);
        {
            *frame.tables[TableId::DivisionMovement.index()] =
                TableView::Complete(TableRows::DivisionMovement(vec![DivisionMovementRow {
                    division: 7,
                    path: vec![2],
                    path_total: 1,
                    naval_transfer: 1,
                    ..Default::default()
                }]));
            *frame.tables[TableId::DivisionTransportSpeeds.index()] = TableView::Complete(
                TableRows::DivisionTransportSpeeds(vec![OwnerTransportSpeedRow {
                    owner: 0,
                    region: 70,
                    speed: Some(800_000),
                }]),
            );
            let content = Content {
                map: MovementMap::default(),
                defines: Defines {
                    transport_scale: 50_000,
                    ..Defines::default()
                },
                terrain_movement: Vec::new(),
                regions: BTreeMap::from([(1, 70)]),
                sub_units: Err("unused".into()),
            };
            assert_eq!(edges(&frame, &content)[&7].speed, Ok(400_000));
        }
    }
    #[test]
    fn cross_hour_progress_reports_without_failing_or_requiring_observations() {
        assert_eq!(Checker::default().failures(), [0, 0]);
        let mut checker = checker();
        assert_eq!(checker.failures(), [0, 0]);
        checker.matched = 1;
        assert_eq!(checker.failures(), [0, 0]);
        checker.matched_later = 2;
        checker.differed = 3;
        assert_eq!(checker.failures(), [0, 0]);
        checker.refused.insert("missing state terms".into(), 4);
        checker.stalled = 5;
        assert_eq!(checker.failures(), [0, 0]);
        checker.content = Some(Err("movement content unavailable".into()));
        assert_eq!(checker.failures(), [0, 1]);
    }

    fn land_fuel_fixture(
        fuel: Recorded<Option<i64>>,
        fuel_rate: i64,
        out_of_fuel_speed: Option<i64>,
        base_speed: i64,
    ) -> (DecodedFrame, Content) {
        use content_reader_map::movement::{Neighbour, ProvinceTerrain};
        use engine_executable::{build::FIXED_MODIFIER_GAP, executable_tables as exec_tables};
        use replay_data::rows::{
            CoreSupplyAccessRow, CountryCounterInputRow, CountryLinkRow, CountryMovementModifiers,
            CountryRow, DivisionMovementRow, DivisionMovementStats, DivisionPanelStatsRow,
            DivisionStatusRow, DivisionTemplateIconRow, LexicalTokenRow, ModifierDefinitionRow,
            MovementDefines, MovementStateTermRow, ProvinceRow, RiverRelationRow,
            SimulationFeatureRow, StateMovementTerms, StateRow, TableId, TableRows,
        };
        use replay_frame::frame::TableView;

        static TABLES: std::sync::OnceLock<exec_tables::Tables> = std::sync::OnceLock::new();
        if !exec_tables::is_initialized() {
            let tables = TABLES.get_or_init(|| {
                let defaults = content_reader_script::test_game::tables();
                let first = defaults.keywords.as_ref().unwrap().modifier_names.len();
                content_reader_script::test_game::executable()
                    .modifiers(
                        (first..FIXED_MODIFIER_GAP as usize)
                            .map(|index| format!("fixture_modifier_{index}")),
                    )
                    .pinned()
                    .build()
            });
            exec_tables::enter(Some(tables));
        }

        let mut frame = moving(1, 10);
        let put = |frame: &mut DecodedFrame, rows: TableRows| {
            let id = rows.table();
            *frame.tables[id.index()] = TableView::Complete(rows);
        };
        put(
            &mut frame,
            TableRows::States(vec![StateRow {
                id: 1,
                owner: 1,
                controller: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    state: 1,
                    controller: 1,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 2,
                    state: 1,
                    controller: 1,
                    ..Default::default()
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::Divisions(vec![DivisionRow {
                id: 7,
                province: 1,
                roster_owner: 1,
                logical_owner: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::DivisionMovement(vec![DivisionMovementRow {
                division: 7,
                path: vec![2],
                path_total: 1,
                numerator: 10,
                withdrawal: Recorded::Value(Some(0)),
                fuel,
                ..Default::default()
            }]),
        );
        let mut panel = DivisionPanelStatsRow {
            division: 7,
            ..Default::default()
        };
        panel.panel_stats[DivisionPanelStatsRow::MAXIMUM_SPEED] = Some(base_speed);
        panel.panel_stats[1] = Some(1_000_000);
        put(&mut frame, TableRows::DivisionPanelStats(vec![panel]));
        put(
            &mut frame,
            TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: 7,
                organisation_exact: Recorded::Value(None),
                maximum_organisation_exact: Recorded::Value(None),
                supply_ratio: Some(100_000),
                supply_grace_maximum: Recorded::Value(100_000),
                supply_grace_remaining: Recorded::Value(0),
                out_of_supply_days: Recorded::Value(Some(0)),
                commander_modifiers: Recorded::Value(Some([None; 5])),
                combat_movement: Recorded::Value(Some(None)),
                weather_factor: Recorded::Value(Some(100_000)),
                movement_stats: Recorded::Value(Some(DivisionMovementStats {
                    terrain: vec![0],
                    scalars: Some([0; 3]),
                    fuel_rate,
                    air_defense: 0,
                })),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]),
        );
        put(
            &mut frame,
            TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 1,
                movement_modifiers: Recorded::Value(Some(CountryMovementModifiers::default())),
                ..Default::default()
            }]),
        );
        put(
            &mut frame,
            TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                movement_defines: Recorded::Value(MovementDefines {
                    out_of_supply_speed: Some(-80_000),
                    retreat_speed: Some(25_000),
                    withdrawal_speed: Some(-50_000),
                    out_of_fuel_speed,
                    fuel_capacity_scale: Some(9_600_000),
                    supply_days: Some(30),
                    supply_grace: Some(72),
                    ..Default::default()
                }),
                ..Default::default()
            }]),
        );
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
            TableRows::MovementStateTerms(vec![MovementStateTermRow {
                state: 1,
                roster: 1,
                owner: 1,
                terms: Some(StateMovementTerms::default()),
            }]),
        );
        put(
            &mut frame,
            TableRows::CoreSupplyAccess(vec![
                CoreSupplyAccessRow {
                    owner: 1,
                    province: 1,
                    access: Some(true),
                },
                CoreSupplyAccessRow {
                    owner: 1,
                    province: 2,
                    access: Some(true),
                },
            ]),
        );
        put(
            &mut frame,
            TableRows::RiverRelations(vec![RiverRelationRow {
                owner: 1,
                controller: 1,
                entry: Some(None),
            }]),
        );
        let mut keys = engine_executable::modifier_keys::all()
            .unwrap()
            .collect::<Vec<_>>();
        keys.sort_unstable_by_key(|key| key.index);
        put(
            &mut frame,
            TableRows::ModifierDefinitions(
                keys.iter()
                    .map(|key| ModifierDefinitionRow {
                        index: key.index,
                        token: key.index + 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut frame,
            TableRows::LexicalTokens(
                keys.iter()
                    .map(|key| LexicalTokenRow {
                        token: key.index + 1,
                        literal: key.name.as_bytes().to_vec(),
                    })
                    .collect(),
            ),
        );
        let land = ProvinceTerrain { flags: 1, index: 1 };
        let map = MovementMap::from_parts(&[
            (
                1,
                land,
                vec![Neighbour {
                    to: 2,
                    distance: Some(1_000_000),
                    ..Default::default()
                }],
            ),
            (
                2,
                land,
                vec![Neighbour {
                    to: 1,
                    distance: Some(1_000_000),
                    ..Default::default()
                }],
            ),
        ]);
        let content = Content {
            map,
            defines: Defines {
                outer_scale: 100_000,
                ..Defines::default()
            },
            terrain_movement: vec![0],
            regions: BTreeMap::new(),
            sub_units: Err("unused".into()),
        };
        let _ = TableId::DivisionMovement;
        (frame, content)
    }

    #[test]
    fn fuel_diagnostic_reports_fuel_only_when_it_changes_movement_speed() {
        let (full, content) =
            land_fuel_fixture(Recorded::Value(Some(96_000_000)), 1_000_000, Some(40_000), 400_000);
        let full_edge = &edges(&full, &content)[&7];
        assert_eq!(full_edge.speed, Ok(400_000));
        assert!(!full_edge.key.fuel_limited);
        assert_eq!(classify(&full, 7, &full_edge.key), "none of these");

        let (empty, content) =
            land_fuel_fixture(Recorded::Value(Some(0)), 1_000_000, Some(40_000), 400_000);
        let empty_edge = &edges(&empty, &content)[&7];
        assert_eq!(empty_edge.speed, Ok(160_000));
        assert!(empty_edge.key.fuel_limited);
        assert_eq!(classify(&empty, 7, &empty_edge.key), "fuel");

        let (negative, content) =
            land_fuel_fixture(Recorded::Value(Some(-5)), 1_000_000, Some(40_000), 400_000);
        let negative_edge = &edges(&negative, &content)[&7];
        assert_eq!(negative_edge.speed, Ok(160_000));
        assert!(negative_edge.key.fuel_limited);
        assert_eq!(classify(&negative, 7, &negative_edge.key), "fuel");

        let mut checker_full = Checker::new(Ok(content));
        let mut next_full = full.clone();
        next_full.hour = 11;
        if let replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::DivisionMovement(rows),
        ) = &mut *next_full.tables[replay_data::rows::TableId::DivisionMovement.index()]
        {
            rows[0].numerator = 10 + 123_456;
        }
        checker_full.observe(&full);
        checker_full.observe(&next_full);
        assert_eq!(checker_full.differed, 1);
        assert_eq!(checker_full.differences.get("none of these"), Some(&1));
        assert!(!checker_full.differences.contains_key("fuel"));

        let (_, content) =
            land_fuel_fixture(Recorded::Value(Some(0)), 1_000_000, Some(40_000), 400_000);
        let mut checker_empty = Checker::new(Ok(content));
        let mut next_empty = empty.clone();
        next_empty.hour = 11;
        if let replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::DivisionMovement(rows),
        ) = &mut *next_empty.tables[replay_data::rows::TableId::DivisionMovement.index()]
        {
            rows[0].numerator = 10 + 123_456;
        }
        checker_empty.observe(&empty);
        checker_empty.observe(&next_empty);
        assert_eq!(checker_empty.differed, 1);
        assert_eq!(checker_empty.differences.get("fuel"), Some(&1));

        let (_, content) =
            land_fuel_fixture(Recorded::Value(Some(0)), 1_000_000, Some(40_000), 400_000);
        let mut checker_no_status = Checker::new(Ok(content));
        let mut next_no_status = next_empty.clone();
        *next_no_status.tables[replay_data::rows::TableId::DivisionStatus.index()] =
            replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::DivisionStatus(
                Vec::new(),
            ));
        checker_no_status.observe(&empty);
        checker_no_status.observe(&next_no_status);
        assert_eq!(checker_no_status.differed, 1);
        assert_eq!(
            checker_no_status.differences.get("no status, fuel"),
            Some(&1),
            "{:?}",
            checker_no_status.differences
        );

        let (partial_penalizing, content) =
            land_fuel_fixture(Recorded::Value(Some(24_000_000)), 1_000_000, Some(40_000), 400_000);
        let partial_edge = &edges(&partial_penalizing, &content)[&7];
        assert_eq!(partial_edge.speed, Ok(220_000));
        assert!(partial_edge.key.fuel_limited);
        assert_eq!(classify(&partial_penalizing, 7, &partial_edge.key), "fuel");

        let (partial_neutral_floor, content) =
            land_fuel_fixture(Recorded::Value(Some(24_000_000)), 1_000_000, Some(100_000), 400_000);
        let neutral_floor_edge = &edges(&partial_neutral_floor, &content)[&7];
        assert_eq!(neutral_floor_edge.speed, Ok(400_000));
        assert!(!neutral_floor_edge.key.fuel_limited);
        assert_eq!(
            classify(&partial_neutral_floor, 7, &neutral_floor_edge.key),
            "none of these"
        );

        let (partial_clamped_min_speed, content) =
            land_fuel_fixture(Recorded::Value(Some(24_000_000)), 1_000_000, Some(40_000), 100_000);
        let clamped_edge = &edges(&partial_clamped_min_speed, &content)[&7];
        assert_eq!(clamped_edge.speed, Ok(100_000));
        assert!(!clamped_edge.key.fuel_limited);

        let (zero_rate, content) =
            land_fuel_fixture(Recorded::Value(Some(500)), 0, Some(40_000), 400_000);
        let zero_rate_edge = &edges(&zero_rate, &content)[&7];
        assert_eq!(zero_rate_edge.speed, Ok(400_000));
        assert!(!zero_rate_edge.key.fuel_limited);

        for sentinel in [i64::MAX, i64::MIN] {
            let (frame, content) =
                land_fuel_fixture(Recorded::Value(Some(sentinel)), 1_000_000, Some(40_000), 400_000);
            let edge = &edges(&frame, &content)[&7];
            assert_eq!(edge.speed, Ok(400_000));
            assert!(!edge.key.fuel_limited, "sentinel {sentinel}");
        }

        for missing in [Recorded::NotRecorded, Recorded::Value(None)] {
            let (frame, content) =
                land_fuel_fixture(missing, 1_000_000, Some(40_000), 400_000);
            let edge = &edges(&frame, &content)[&7];
            assert!(edge.speed.is_err());
            assert!(!edge.key.fuel_limited);
        }
    }
}
