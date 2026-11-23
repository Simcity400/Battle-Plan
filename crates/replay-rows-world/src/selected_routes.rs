use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct OrderedEnemyInputs {
    pub sample_hour: PacketHour,
    pub entries: Vec<u32>,
}

value_struct!(OrderedEnemyInputs { sample_hour: PacketHour, entries: Vec<u32> });
impl OpaqueValue for OrderedEnemyInputs {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ControllerAreaNode {
    pub first_land: Option<u32>,
    pub neighbours: Option<Vec<u32>>,
}

value_struct!(ControllerAreaNode { first_land: Option<u32>, neighbours: Option<Vec<u32>> });
impl OpaqueValue for ControllerAreaNode {}
