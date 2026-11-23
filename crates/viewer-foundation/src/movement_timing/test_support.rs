use super::{LandSpeedContent, MovementContent, MovementMap, RailMovementDefines};
use content_reader_map::movement::{Neighbour, ProvinceTerrain};
use replay_data::rows::{DivisionMovementRow, DivisionPanelStatsRow, DivisionRow, ProvinceRow};
use replay_derive::eta_derive::Defines;
use replay_frame::frame::Metrics;
use std::collections::BTreeMap;

use replay_data::rows::{
    CoreSupplyAccessRow, CountryCounterInputRow, CountryLinkRow, CountryMovementModifiers,
    CountryRow, DivisionMovementStats, DivisionStatusRow, DivisionTemplateIconRow, LexicalTokenRow,
    ModifierDefinitionRow, MovementDefines, MovementStateTermRow, Recorded, RiverRelationRow,
    SimulationFeatureRow, StateMovementTerms, TableRows, tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

pub fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let id = rows.table();
    *frame.tables[id.index()] = TableView::Complete(rows);
}

pub fn neutral_status(division: u64, terrains: usize) -> DivisionStatusRow {
    DivisionStatusRow {
        organisation_exact: Recorded::Value(None),
        maximum_organisation_exact: Recorded::Value(None),
        division,
        supply_ratio: Some(100_000),
        supply_grace_maximum: Recorded::Value(100_000),
        supply_grace_remaining: Recorded::Value(0),
        out_of_supply_days: Recorded::Value(Some(0)),
        commander_modifiers: Recorded::Value(Some([None; 5])),
        combat_movement: Recorded::Value(Some(None)),
        weather_factor: Recorded::Value(Some(100_000)),
        movement_stats: Recorded::Value(Some(DivisionMovementStats {
            terrain: vec![0; terrains],
            scalars: Some([0; 3]),
            fuel_rate: 0,
            air_defense: 0,
        })),
        ..Default::default()
    }
}

pub fn neutral_world(frame: &mut DecodedFrame, country: u32) {
    let linked = frame
        .complete::<tables::CountryLinks>()
        .is_ok_and(|rows| rows.iter().any(|row| row.id == country));
    if !linked {
        put(
            frame,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: country,
                canonical_slot: country,
            }]),
        );
        put(
            frame,
            TableRows::Countries(vec![CountryRow {
                id: country,
                canonical_slot: country,
                ..Default::default()
            }]),
        );
    }
    put(
        frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers::default())),
            ..Default::default()
        }]),
    );
    put(
        frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            movement_defines: Recorded::Value(MovementDefines {
                out_of_supply_speed: Some(-80_000),
                retreat_speed: Some(25_000),
                withdrawal_speed: Some(-50_000),
                out_of_fuel_speed: Some(40_000),
                fuel_capacity_scale: Some(9_600_000),
                supply_days: Some(30),
                supply_grace: Some(72),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    put(frame, TableRows::WeatherProvinces(vec![]));
    put(frame, TableRows::WeatherRegions(vec![]));
    put(
        frame,
        TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
            template: 0,
            speed_keys: Recorded::Value(Some(Vec::new())),
            ..Default::default()
        }]),
    );
    let provinces = frame
        .complete::<tables::Provinces>()
        .map(<[_]>::to_vec)
        .unwrap_or_default();
    let distinct = |values: Vec<u32>| {
        let mut values = values;
        values.sort_unstable();
        values.dedup();
        values
    };
    let states = distinct(
        provinces
            .iter()
            .map(|row| row.state)
            .filter(|&state| state != 0)
            .collect(),
    );
    put(
        frame,
        TableRows::MovementStateTerms(
            states
                .into_iter()
                .map(|state| MovementStateTermRow {
                    state,
                    roster: country,
                    owner: country,
                    terms: Some(StateMovementTerms::default()),
                })
                .collect(),
        ),
    );
    put(
        frame,
        TableRows::CoreSupplyAccess(
            provinces
                .iter()
                .map(|row| CoreSupplyAccessRow {
                    owner: country,
                    province: row.id,
                    access: Some(true),
                })
                .collect(),
        ),
    );
    let controllers = distinct(provinces.iter().map(|row| row.controller).collect());
    put(
        frame,
        TableRows::RiverRelations(
            controllers
                .into_iter()
                .map(|controller| RiverRelationRow {
                    owner: country,
                    controller,
                    entry: Some(None),
                })
                .collect(),
        ),
    );
    put(frame, TableRows::CountryUnitSpeedEntries(vec![]));
    put(frame, TableRows::DivisionTransportSpeeds(vec![]));
    if !engine_executable::executable_tables::is_initialized() {
        crate::test_support::enter_fixed_modifier_tables();
    }
    let mut keys = engine_executable::modifier_keys::all()
        .unwrap()
        .collect::<Vec<_>>();
    keys.sort_unstable_by_key(|key| key.index);
    put(
        frame,
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
        frame,
        TableRows::LexicalTokens(
            keys.iter()
                .map(|key| LexicalTokenRow {
                    token: key.index + 1,
                    literal: key.name.as_bytes().to_vec(),
                })
                .collect(),
        ),
    );
}

pub(super) const LAND: ProvinceTerrain = ProvinceTerrain { flags: 1, index: 1 };

pub const SPEED: i64 = 700_000;

pub const DIVISION: u64 = 7;

pub fn content(map: MovementMap) -> MovementContent {
    MovementContent {
        map,
        defines: Defines {
            outer_scale: 100_000,
            ..Default::default()
        },
        rail: Ok(RailMovementDefines {
            speed: [0; 4],
            maximum_level: 0,
        }),
        task_force_terrain: Err("unrelated task-force terrain is unavailable".into()),
        land: Ok(LandSpeedContent {
            terrain_movement: vec![0],
            regions: BTreeMap::new(),
            ..Default::default()
        }),
    }
}

pub(super) fn status_row(division: u64) -> DivisionStatusRow {
    neutral_status(division, 1)
}

pub(super) fn world(frame: &mut DecodedFrame) {
    neutral_world(frame, 1);
}

pub fn two_leg_map() -> MovementMap {
    let edge = |to, distance| Neighbour {
        to,
        distance: Some(distance),
        ..Default::default()
    };
    MovementMap::from_parts(&[
        (1, LAND, vec![edge(2, 350_000)]),
        (2, LAND, vec![edge(1, 350_000), edge(3, 700_000)]),
        (3, LAND, vec![edge(2, 700_000)]),
    ])
}

pub(super) fn frame() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 60_832_632,
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "not needed for the estimate".into(),
        })
        .map(Into::into),
        metrics: Metrics::default(),
    }
}

pub(super) fn moving() -> DivisionMovementRow {
    DivisionMovementRow {
        division: DIVISION,
        path: vec![2, 3],
        path_total: 2,
        numerator: 175_000,
        withdrawal: Recorded::Value(Some(0)),
        fuel: Recorded::Value(Some(0)),
        ..Default::default()
    }
}

pub(super) fn panel(division: u64, speed: Option<i64>) -> DivisionPanelStatsRow {
    let mut panel = DivisionPanelStatsRow {
        division,
        ..Default::default()
    };
    panel.panel_stats[DivisionPanelStatsRow::MAXIMUM_SPEED] = speed;
    panel.panel_stats[1] = Some(1_000_000);
    panel
}

pub(super) fn provinces(ids: impl IntoIterator<Item = u32>) -> TableRows {
    TableRows::Provinces(
        ids.into_iter()
            .map(|id| ProvinceRow {
                id,
                ..Default::default()
            })
            .collect(),
    )
}

pub fn hour(base_speed: Option<i64>) -> DecodedFrame {
    let mut frame = frame();
    put(&mut frame, provinces(1..=3));
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            province: 1,
            roster_owner: 1,
            logical_owner: 1,
            ..Default::default()
        }]),
    );
    put(&mut frame, TableRows::DivisionMovement(vec![moving()]));
    put(
        &mut frame,
        TableRows::DivisionPanelStats(vec![panel(DIVISION, base_speed)]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![status_row(DIVISION)]),
    );
    world(&mut frame);
    frame
}
