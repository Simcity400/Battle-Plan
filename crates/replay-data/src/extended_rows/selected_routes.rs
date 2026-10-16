//! Current simulation inputs for the route search's ordered enemy test and
//! controller-area graph. No local selection. The previous area packet
//! (`Provinces.selected_areas`) and unit selection operands retired at
//! [`crate::rows::compatibility::RETIRED_INPUTS_FORMAT`]; the areas returned as
//! `Provinces.controller_area`, `owner_area` and `area_node`
//! ([`crate::rows::compatibility::AREA_FORMAT`]).
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderedEnemyInputs {
    pub sample_hour: PacketHour,
    /// Authentic enemy-vector raw words, including repeats and zero.
    pub entries: Vec<u32>,
}

value_struct!(OrderedEnemyInputs { sample_hour: PacketHour, entries: Vec<u32> });
opaque_value!(Option<OrderedEnemyInputs>);

/// One controller area as the route search reads it, recorded once on the row of its label
/// province. Its land count is the number of provinces with its label (the capture refuses a node
/// whose count differs) and its first land's native nonsea index is static map content, so neither
/// is stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControllerAreaNode {
    /// The area's representative, set by its area updates: the province costed by its state owner
    /// and used to order the route search's heap. None is a refusal.
    pub first_land: Option<u32>,
    /// The contacts in native list order, as labels. None is a refusal, never an empty list.
    pub neighbours: Option<Vec<u32>>,
}

value_struct!(ControllerAreaNode { first_land: Option<u32>, neighbours: Option<Vec<u32>> });
opaque_value!(Option<ControllerAreaNode>);
