use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportReceiverShips {
    pub task_force: u64,
    pub count: Option<i32>,
    pub ships: Option<Vec<TransportRiskShip>>,
}
value_struct!(TransportReceiverShips { task_force: u64, count: Option<i32>, ships: Option<Vec<TransportRiskShip>> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TransportRiskShip {
    pub ship: Option<u64>,
    pub class: Option<u32>,
    pub formation_flags: Option<u64>,
    pub positive_equipment_flags: Option<u64>,
    pub carrier_air_base: Option<Option<u64>>,
}
value_struct!(TransportRiskShip { ship: Option<u64>, class: Option<u32>, formation_flags: Option<u64>, positive_equipment_flags: Option<u64>, carrier_air_base: Option<Option<u64>> });
