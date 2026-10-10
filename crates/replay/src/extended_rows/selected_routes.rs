//! Current province-access, area-cost and retreat inputs. No local selection.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvinceAreaInputs {
    pub sample_hour: i32,
    /// Actual D0 pointer equality; zero is native null. Labels are pass-local.
    pub controller: u32,
    /// Opaque D8 equality/null; None is an independent read refusal.
    pub owner: Option<u32>,
    /// Stored once, in the province whose ID equals the controller label.
    /// None on that row refuses graph acquisition; other members need no copy.
    pub node: Option<SelectedAreaNode>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectedAreaNode {
    /// area+28 province ID and its native Map+238 index, not a viewer index.
    pub first_land: Option<u32>,
    pub nonsea_index: Option<i32>,
    /// Land-only simulation province count for temporary area records. The
    /// visited bitset instead uses the content-derived total province count.
    pub nonsea_count: Option<u32>,
    /// Native signed area+3C; the cost callback tests nonzero.
    pub land_count: Option<i32>,
    /// area+90 list order, with repeats. None is refusal, never an empty list.
    pub neighbours: Option<Vec<u32>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectedUnitInputs {
    pub sample_hour: i32,
    /// Access XORs the raw exile byte with one; Boolean normalization loses it.
    pub exiled: Option<u8>,
    /// Selected commander accumulator's effective keys 297/298, read separately.
    /// A null commander gives known zeros. None refuses only the reached key.
    pub commander: Option<bool>,
    pub retreat: [Option<i64>; 2],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderedEnemyInputs {
    pub sample_hour: i32,
    /// Authentic diplomacy+98 raw words, including repeats and zero.
    pub entries: Vec<u32>,
}

value_struct!(ProvinceAreaInputs { sample_hour: i32, controller: u32, owner: Option<u32>, node: Option<SelectedAreaNode> });
value_struct!(SelectedAreaNode { first_land: Option<u32>, nonsea_index: Option<i32>, nonsea_count: Option<u32>, land_count: Option<i32>, neighbours: Option<Vec<u32>> });
value_struct!(SelectedUnitInputs { sample_hour: i32, exiled: Option<u8>, commander: Option<bool>, retreat: [Option<i64>;2] });
value_struct!(OrderedEnemyInputs { sample_hour: i32, entries: Vec<u32> });
opaque_value!(Option<ProvinceAreaInputs>);
opaque_value!(Option<SelectedUnitInputs>);
opaque_value!(Option<OrderedEnemyInputs>);
