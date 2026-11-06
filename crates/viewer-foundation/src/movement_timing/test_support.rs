//! Neutral recorded speed terms for estimate tests: rows under which a division's edge
//! speed is its base speed times the movement scale.
use replay_data::rows::{
    CoreSupplyAccessRow, CountryCounterInputRow, CountryLinkRow, CountryMovementModifiers,
    CountryRow, DivisionMovementStats, DivisionStatusRow, DivisionTemplateIconRow, LexicalTokenRow,
    ModifierDefinitionRow, MovementDefines, MovementStateTermRow, Recorded, RiverRelationRow,
    SimulationFeatureRow, StateMovementTerms, TableRows, tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let id = rows.table();
    *frame.tables[id.index()] = TableView::Complete(rows);
}

/// A status row whose speed terms are all neutral: full supply, no commander, no fuel
/// capacity, not in combat and no movement on any of `terrains` terrain categories.
pub fn neutral_status(division: u64, terrains: usize) -> DivisionStatusRow {
    DivisionStatusRow {
        organisation_exact: Recorded::Value(None),
        maximum_organisation_exact: Recorded::Value(None),
        division,
        supply_ratio: 100_000,
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

/// The shared rows every estimate reads: `country` (its own canonical slot unless the
/// frame already links it) with no movement entry, the recorded defines, no weather,
/// template 0 with no movement priority sub-unit speed key, and the modifier metadata
/// the movement family validates. Over the frame's Provinces (put them first), the
/// country, as roster owner and owner, has no location state entry, core supply access
/// everywhere and no relation river crossing factor towards any controller; no
/// transport speed is recorded. Needs the installation's executable tables.
pub fn neutral_world(frame: &mut DecodedFrame, country: u32) {
    // A frame that already links the country keeps its own country rows.
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
    engine_executable::executable_tables::initialize_for_tests()
        .expect("modifier-name tests require BATTLE_PLAN_GAME_DIR with the supported installation");
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
