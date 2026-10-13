//! Native state payloads. Fixed arrays and records retain independent field-mask bits.
use super::*;
use crate::value::value_struct;

/// Name generator operands, rather than the cached display name.
/// `custom` keeps +0x88 bytes shown by the generator, including its stored
/// programmatic fallback for a null group; `group` is a content key.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NameOperands {
    pub kind: i32,
    pub order: i32,
    pub ordered: u8,
    pub programmatic: u8,
    pub custom: Vec<u8>,
    pub group: Vec<u8>,
}
value_struct!(NameOperands { kind: i32, order: i32, ordered: u8, programmatic: u8, custom: Vec<u8>, group: Vec<u8> });
opaque_value!(Option<NameOperands>);

pub use air::*;
pub use air_ledger::*;
pub use attack_inputs::*;
pub use carrier_air::*;
pub use characters_intelligence::*;
pub use directed_war::*;
pub use economy_construction::*;
pub use equipment_destruction::*;
pub use fresh_eta::*;
pub use fuel_demand::*;
pub use game_score::*;
pub use map_presentation::*;
pub use movement::*;
pub use naval_weather::*;
pub use packet_hour::*;
pub use political_runtime::*;
pub use politics_research::*;
pub use raids_projects::*;
pub use reserved_split::*;
pub use selected_routes::*;
pub use template_display_name::*;
pub use transport_mission_inputs::*;
pub use unit_combat_references::*;
pub use war_overview::*;

#[path = "extended_rows/air.rs"]
mod air;
#[path = "extended_rows/air_ledger.rs"]
mod air_ledger;
#[path = "extended_rows/attack_inputs.rs"]
mod attack_inputs;
#[path = "extended_rows/carrier_air.rs"]
mod carrier_air;
#[path = "extended_rows/characters_intelligence.rs"]
mod characters_intelligence;
#[path = "extended_rows/directed_war.rs"]
mod directed_war;
#[path = "extended_rows/economy_construction.rs"]
mod economy_construction;
#[path = "extended_rows/equipment_destruction.rs"]
mod equipment_destruction;
#[path = "extended_rows/fresh_eta.rs"]
mod fresh_eta;
#[path = "extended_rows/fuel_demand.rs"]
mod fuel_demand;
#[path = "extended_rows/game_score.rs"]
mod game_score;
#[path = "extended_rows/map_presentation.rs"]
mod map_presentation;
#[path = "extended_rows/movement.rs"]
mod movement;
#[path = "extended_rows/naval_weather.rs"]
mod naval_weather;
#[path = "extended_rows/nuclear.rs"]
mod nuclear;
#[path = "extended_rows/packet_hour.rs"]
mod packet_hour;
#[path = "extended_rows/peace.rs"]
mod peace;
#[path = "extended_rows/political_runtime.rs"]
mod political_runtime;
#[path = "extended_rows/politics_research.rs"]
mod politics_research;
#[path = "extended_rows/raids_projects.rs"]
mod raids_projects;
#[path = "extended_rows/reserved_split.rs"]
mod reserved_split;
#[path = "extended_rows/selected_routes.rs"]
mod selected_routes;
#[path = "extended_rows/template_display_name.rs"]
mod template_display_name;
#[path = "extended_rows/transport_mission_inputs.rs"]
mod transport_mission_inputs;
#[path = "extended_rows/transport_spotting_context.rs"]
mod transport_spotting_context;
pub use transport_spotting_context::*;
#[path = "extended_rows/transport_air.rs"]
mod transport_air;
pub use transport_air::*;
#[path = "extended_rows/transport_air_normalization.rs"]
mod transport_air_normalization;
pub use transport_air_normalization::*;
#[path = "extended_rows/transport_risk_ships.rs"]
mod transport_risk_ships;
pub use transport_risk_ships::*;
#[path = "extended_rows/unit_combat_references.rs"]
mod unit_combat_references;
#[path = "extended_rows/war_overview.rs"]
mod war_overview;

#[cfg(test)]
mod country_appearance_tests {
    use super::CountryAppearanceRow;

    #[test]
    fn recorded_colour_is_truncated_without_reapplying_hsv() {
        let row = |[red, green, blue]: [f32; 3]| CountryAppearanceRow {
            red: red.to_bits(),
            green: green.to_bits(),
            blue: blue.to_bits(),
            ..Default::default()
        };
        assert_eq!(row([0.4, 0.6, 0.8]).map_color(), [102, 153, 204]);
        assert_eq!(row([0.5; 3]).map_color(), [127; 3]);
        // Out-of-range floats saturate rather than wrap.
        assert_eq!(row([1.5, -0.5, 0.0]).map_color(), [255, 0, 0]);
    }
}
// Variable records borrow their typed values during encoding and allocate only on decode.
opaque_value!(Option<Vec<u8>>);
opaque_value!(Option<Vec<u32>>);
opaque_value!(Option<Vec<Vec<u32>>>);
opaque_value!(Option<Vec<u64>>);
opaque_value!(Vec<i64>);
opaque_value!(Vec<i32>);
opaque_value!(Option<Option<i32>>);
opaque_value!(Option<i16>);

#[cfg(test)]
pub(crate) use fixtures::sample_extended_table;

#[cfg(test)]
#[path = "extended_rows/fixtures.rs"]
mod fixtures;

#[cfg(test)]
#[path = "extended_rows/border_effects/tests.rs"]
mod border_effect_tests;
