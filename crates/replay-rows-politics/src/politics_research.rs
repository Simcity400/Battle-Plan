use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportSubjectAccess {
    pub sample_hour: PacketHour,
    pub complete_owner: bool,
    pub target_raw: Option<u32>,
    pub subject: Option<ZeroSubjectInputs>,
}
value_struct!(TransportSubjectAccess {
    sample_hour: PacketHour, complete_owner: bool, target_raw: Option<u32>, subject: Option<ZeroSubjectInputs>,
});
impl OpaqueValue for TransportSubjectAccess {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ScriptVariableEntry {
    pub name: Vec<u8>,
    pub value_raw: i64,
}
value_struct!(ScriptVariableEntry {name: Vec<u8>, value_raw: i64});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OccupationGroupLaw {
    pub occupied: u32,
    pub scope_country: Option<u32>,
    pub law: Option<Vec<u8>>,
}
value_struct!(OccupationGroupLaw { occupied: u32, scope_country: Option<u32>, law: Option<Vec<u8>> });
impl OpaqueValue for OccupationGroupLaw {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GarrisonEquipment {
    pub equipment_token: u32,
    pub archetype_token: u32,
    pub archetype_active: u8,
    pub required_raw: i64,
    pub required_lookup_raw: i64,
    pub allocated_raw: i64,
    pub weight_raw: i64,
}
value_struct!(GarrisonEquipment {
    equipment_token: u32,
    archetype_token: u32,
    archetype_active: u8,
    required_raw: i64,
    required_lookup_raw: i64,
    allocated_raw: i64,
    weight_raw: i64
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct GarrisonEquipmentInputs {
    pub rows: Vec<GarrisonEquipment>,
    pub failure: Option<Vec<u8>>,
}
value_struct!(GarrisonEquipmentInputs { rows: Vec<GarrisonEquipment>, failure: Option<Vec<u8>> });
opaque_value!(GarrisonEquipmentInputs);

impl OpaqueValue for ScriptVariableEntry {}
