use std::sync::OnceLock;

use engine_executable::{build::FIXED_MODIFIER_GAP, executable_tables, modifier_keys};
use replay_data::rows::{
    CoreSupplyAccessRow, CountryCounterInputRow, CountryMovementModifiers, LexicalTokenRow,
    ModifierDefinitionRow, MovementDefines, MovementStateTermRow, Recorded, SimulationFeatureRow,
    StateMovementTerms, TableRows, tables,
};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::test_support::test_put;

fn fixed_modifier_names() -> executable_tables::Tables {
    static TABLES: OnceLock<executable_tables::Tables> = OnceLock::new();
    *TABLES.get_or_init(|| {
        let names = (0..FIXED_MODIFIER_GAP).map(|index| match index {
            modifier_keys::ATTRITION => "attrition".into(),
            modifier_keys::HEAT_ATTRITION => "heat_attrition".into(),
            modifier_keys::WINTER_ATTRITION => "winter_attrition".into(),
            modifier_keys::UNIT_UPKEEP_ATTRITION_FACTOR => "unit_upkeep_attrition_factor".into(),
            _ => format!("synthetic_modifier_{index}"),
        });
        synthetic_game::Executable::empty()
            .modifiers(names)
            .pinned()
            .build()
    })
}

pub(in crate::map) fn put_modifier_definitions(
    frame: &mut DecodedFrame,
    keys: impl IntoIterator<Item = u32>,
) {
    executable_tables::enter(Some(fixed_modifier_names()));
    let mut keys: Vec<_> = keys.into_iter().collect();
    keys.sort_unstable();
    keys.dedup();
    test_put(
        frame,
        TableRows::ModifierDefinitions(
            keys.iter()
                .map(|&index| ModifierDefinitionRow {
                    index,
                    token: index + 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    test_put(
        frame,
        TableRows::LexicalTokens(
            keys.into_iter()
                .map(|index| LexicalTokenRow {
                    token: index + 1,
                    literal: modifier_keys::fixed_name(index).unwrap().as_bytes().to_vec(),
                })
                .collect(),
        ),
    );
}

pub(in crate::map) fn neutral_land_movement(frame: &mut DecodedFrame, country: u32, source: u32) {
    put_modifier_definitions(
        frame,
        replay_derive::fixed_layout::MOVEMENT.keys.iter().copied(),
    );
    let state = frame
        .complete::<tables::Provinces>()
        .unwrap()
        .iter()
        .find(|row| row.id == source)
        .unwrap()
        .state;
    for rows in [
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers::default())),
            ..Default::default()
        }]),
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            movement_defines: Recorded::Value(MovementDefines {
                out_of_supply_speed: Some(0),
                fuel_capacity_scale: Some(100_000),
                supply_days: Some(10),
                supply_grace: Some(24),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        TableRows::WeatherProvinces(Vec::new()),
        TableRows::MovementStateTerms(vec![MovementStateTermRow {
            state,
            roster: country,
            owner: country,
            terms: Some(StateMovementTerms::default()),
        }]),
        TableRows::CoreSupplyAccess(vec![CoreSupplyAccessRow {
            owner: country,
            province: source,
            access: Some(true),
        }]),
    ] {
        test_put(frame, rows);
    }
}
