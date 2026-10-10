//! One hourly transport-risk source packet on the reserved singleton.
//! Capture owns the physical navy census and one receiver dictionary. Viewer
//! selection, hover targets and route queries never enter this packet.
use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportMissionInputs {
    pub sample_hour: i32,
    /// Complete physical manager order, including null slot zero when nonempty.
    /// Empty is an authenticated null/empty manager, which supplies null
    /// strategic navy for every requested slot. None refuses this domain
    /// independently of the receiver dictionary.
    pub navies: Option<Vec<TransportNavy>>,
    /// Unique increasing full handles, including genuine outside-census
    /// receivers. No pointer or ordinary motion-row membership is persisted.
    pub receivers: Option<Vec<TransportMissionReceiver>>,
    /// Original native strategic-region search graph, independent of navies.
    pub naval_search: Option<TransportNavalSearch>,
    /// Same-hour original country and strategic-region spotting observations.
    pub spotting_context: Option<TransportSpottingContext>,
    /// Same-hour original physical strategic-air roots and regional headers.
    pub air: Option<TransportAirInputs>,
    /// Original ordered SHIPS loans, including genuine fading receivers.
    pub risk_ships: Option<TransportRiskShips>,
}
value_struct!(TransportMissionInputs {
    sample_hour: i32,
    navies: Option<Vec<TransportNavy>>,
    receivers: Option<Vec<TransportMissionReceiver>>,
    naval_search: Option<TransportNavalSearch>,
    spotting_context: Option<TransportSpottingContext>,
    air: Option<TransportAirInputs>,
    risk_ships: Option<TransportRiskShips>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportNavalSearch {
    pub width: Option<i32>,
    /// Original physical region roster; None is an authenticated null slot.
    pub regions: Option<Vec<Option<TransportNavalRegion>>>,
    /// Exact definition name bytes of authenticated CAdjacencyRule objects.
    pub rules: Vec<Vec<u8>>,
}
value_struct!(TransportNavalSearch {
    width: Option<i32>, regions: Option<Vec<Option<TransportNavalRegion>>>, rules: Vec<Vec<u8>>,
});

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportNavy {
    pub slot: u32,
    pub present: bool,
    /// Original outer region count, including its empty region slots. A
    /// refused header retains None, never a fabricated zero count.
    pub region_slots: Option<u32>,
    /// Sparse nonempty regions in native increasing region order. A known
    /// empty region is omitted only within a complete original outer domain.
    pub ledgers: Option<Vec<TransportLedger>>,
    /// Original hourly +B0 byte array; no carried NavalRegionAccess join.
    pub access_modes: Option<Vec<u8>>,
}
value_struct!(TransportNavy {
    slot: u32,
    present: bool,
    region_slots: Option<u32>,
    ledgers: Option<Vec<TransportLedger>>,
    access_modes: Option<Vec<u8>>,
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TransportLedger {
    pub region: u32,
    /// Native order and repeats. Zero means a native null mission or null
    /// backlink. A nonnull identity/read refusal withholds this whole ledger.
    pub occurrences: Option<Vec<u64>>,
}
value_struct!(TransportLedger {
    region: u32,
    occurrences: Option<Vec<u64>>,
});

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

opaque_value!(Option<TransportMissionInputs>);

#[cfg(test)]
#[path = "transport_mission_inputs_tests.rs"]
mod tests;
