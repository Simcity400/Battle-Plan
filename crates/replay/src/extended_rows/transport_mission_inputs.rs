//! Value types of the retired TransportSearchRegions and TransportReceivers
//! tables ([`super::RESERVED_SPLIT_TABLES`]): wire tombstones that keep the
//! tables' framing. Nothing reads or writes them.
use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportNavalRegion {
    pub position: [i32; 2],
    pub edges: Vec<TransportNavalEdge>,
}
value_struct!(TransportNavalRegion { position: [i32; 2], edges: Vec<TransportNavalEdge> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportNavalEdge {
    pub to: u32,
    pub flags: u8,
    pub unrestricted: u8,
    /// Original order and repeats. None is the native null rule.
    pub rules: Vec<Option<u32>>,
}
value_struct!(TransportNavalEdge { to: u32, flags: u8, unrestricted: u8, rules: Vec<Option<u32>> });

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportMissionReceiver {
    pub task_force: u64,
    /// Original embedded mission +10 country, independent of task logical owner.
    pub mission_owner: Option<u32>,
    /// Genuine same-pass raw logical owner, if its original graph reached it.
    pub logical_owner: Option<u32>,
    /// Nonboolean mission gate and signed original combat-header count.
    /// Their lazy native gate distinguishes unreached fields from refusals.
    pub in_regions: Option<u8>,
    pub combat_references: Option<i32>,
    /// Reached full NAVAL_DETECTION before the native unity addition.
    pub combined_detection: Option<i64>,
    /// One original sixteen-byte cache copy: mission +20 and +28, signed64.
    pub detection_cache: Option<[i64; 2]>,
    /// Borrowed original raw supply, independent of the other graph results.
    pub received_supply_ratio: Option<i64>,
    /// Independent leaves from the same original ETA/risk graph. Absence
    /// refuses this graph after the outer native mission/combat gates.
    pub risk: Option<TransportReceiverRisk>,
}
value_struct!(TransportMissionReceiver {
    task_force: u64,
    mission_owner: Option<u32>,
    logical_owner: Option<u32>,
    in_regions: Option<u8>,
    combat_references: Option<i32>,
    combined_detection: Option<i64>,
    detection_cache: Option<[i64; 2]>,
    received_supply_ratio: Option<i64>,
    risk: Option<TransportReceiverRisk>,
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportTaskForceStats {
    pub detection_138: Option<i64>,
    pub visibility_148: Option<i64>,
    pub visibility_150: Option<i64>,
    pub carrier_detection_160: Option<i64>,
}
value_struct!(TransportTaskForceStats {
    detection_138: Option<i64>, visibility_148: Option<i64>,
    visibility_150: Option<i64>, carrier_detection_160: Option<i64>,
});

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TransportReceiverRisk {
    /// Raw common modifier results before each caller's unity addition.
    pub coordination: Option<i64>,
    pub spotting_chance: Option<i64>,
    pub night_spotting_chance: Option<i64>,
    pub visibility: Option<i64>,
    pub speed_base: Option<i64>,
    pub speed_modifier: Option<i64>,
    pub terrain_class: Option<i32>,
    pub stats: Option<TransportTaskForceStats>,
    pub roster_owner: Option<u32>,
    pub location: Option<u32>,
    pub region: Option<u32>,
    pub location_controller: Option<u32>,
    pub detached_activity: Option<i32>,
    pub ship_count: Option<i32>,
    /// Original current roster/location-controller relation key1C5.
    pub targeted_spotting_modifier: Option<i64>,
    /// Native class terrain-speed fold from the original ship traversal.
    pub actual_terrain_speed: Option<i64>,
}
value_struct!(TransportReceiverRisk {
    coordination: Option<i64>, spotting_chance: Option<i64>,
    night_spotting_chance: Option<i64>, visibility: Option<i64>,
    speed_base: Option<i64>, speed_modifier: Option<i64>, terrain_class: Option<i32>,
    stats: Option<TransportTaskForceStats>,
    roster_owner: Option<u32>, location: Option<u32>, region: Option<u32>,
    location_controller: Option<u32>, detached_activity: Option<i32>, ship_count: Option<i32>,
    targeted_spotting_modifier: Option<i64>, actual_terrain_speed: Option<i64>,
});
