use super::*;

/// Same-hour numerical dictionary from the original transport SHIPS loops.
/// A repeated ship handle remains a repeated ordinal in its receiver's roster.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportRiskShips {
    pub sample_hour: i32,
    pub receivers: Option<Vec<TransportReceiverShips>>,
}
value_struct!(TransportRiskShips { sample_hour: i32, receivers: Option<Vec<TransportReceiverShips>> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportReceiverShips {
    pub task_force: u64,
    pub count: Option<i32>,
    /// Complete original ordered roster; None refuses its graph independently.
    pub ships: Option<Vec<TransportRiskShip>>,
}
value_struct!(TransportReceiverShips { task_force: u64, count: Option<i32>, ships: Option<Vec<TransportRiskShip>> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportRiskShip {
    pub ship: Option<u64>,
    pub class: Option<u32>,
    pub formation_flags: Option<u64>,
    pub positive_equipment_flags: Option<u64>,
    /// Some(None) is native null; None is a failed or unreached observation.
    pub carrier_air_base: Option<Option<u64>>,
}
value_struct!(TransportRiskShip { ship: Option<u64>, class: Option<u32>, formation_flags: Option<u64>, positive_equipment_flags: Option<u64>, carrier_air_base: Option<Option<u64>> });

#[cfg(test)]
#[path = "transport_risk_ships_tests.rs"]
mod tests;
