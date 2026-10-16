//! Neutral recorded speed terms for estimate tests: rows under which a division's edge
//! speed is its base speed times the movement scale.
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{
        CountryCounterInputRow, CountryLinkRow, CountryMovementModifiers, CountryRow,
        DivisionMovementStats, DivisionStatusRow, LexicalTokenRow, ModifierDefinitionRow,
        MovementDefines, Recorded, SimulationFeatureRow, TableRows, tables,
    },
};

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let id = rows.table();
    *frame.tables[id.index()] = TableView::Complete(rows);
}

/// A status row whose speed terms are all neutral: full supply, no commander, no fuel
/// capacity and no movement on any of `terrains` terrain categories.
pub fn neutral_status(division: u64, terrains: usize) -> DivisionStatusRow {
    DivisionStatusRow {
        division,
        supply_ratio: 100_000,
        supply_grace_maximum: Recorded::Value(100_000),
        supply_grace_remaining: Recorded::Value(0),
        out_of_supply_days: Recorded::Value(Some(0)),
        commander_modifiers: Recorded::Value(Some([None; 5])),
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
/// and the modifier metadata the movement family validates. Needs the installation's
/// executable tables.
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
    engine::executable_tables::initialize_for_tests()
        .expect("modifier-name tests require BATTLE_PLAN_GAME_DIR with the supported installation");
    let mut keys = engine::modifier_keys::all().collect::<Vec<_>>();
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
