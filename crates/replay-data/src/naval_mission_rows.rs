//! Operands of the naval mission icon's strike/invasion active counts,
//! convoy efficiencies and navies' mission ledgers. Efficiencies are
//! computed when the icon refreshes, never stored, so these tables keep
//! the required operands ([`crate::naval_mission_derive`] evaluates them).
//! Modifier values are raw accumulator entries, `None` when absent;
//! ModifierDefinitions provides feature and clamp metadata for present
//! entries. Keys: `engine_native::naval_mission_inputs::COUNTRY_KEYS`.
use super::*;

/// NavalMissionTaskForces (hourly): one row per task force whose mission
/// kind (`TaskForces.mission`) is strike force (2), convoy raiding (3),
/// convoy escort (4) or invasion support (9). Fleet, owners, location and
/// kind are in TaskForces; ship counts and classes in Ships; `is_in_regions`
/// in TaskForceCounterInputs.
///
/// `raiding_factor` is the stored stat-block factor. `local_*` values come
/// from the task force's own accumulator, `weather_*` values from its
/// location region's weather modifier. Both are read only while located;
/// a region outside the weather array has every key absent, as null adds
/// nothing. `leader` is the fleet's leader reference (zero for none).
/// `headquarters` keeps one entry per attached naval headquarters in order:
/// the commander's character reference, zero without a commander.
/// `range_fleet` (zero when unset), `range_begin` and `range_end` are the
/// mission's region range; `convoys` is its stored convoy count.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalMissionTaskForceRow {
    pub task_force: u64,
    /// Raw mission owner, also needed when TaskForceMissions has no region row.
    pub mission_owner: u32,
    pub raiding_factor: i64,
    pub local_coordination: Option<i64>,
    pub local_raiding: Option<i64>,
    pub weather_coordination: Option<i64>,
    pub weather_raiding: Option<i64>,
    pub leader: u64,
    pub headquarters: Vec<u64>,
    pub range_fleet: u64,
    pub range_begin: i32,
    pub range_end: i32,
    pub convoys: i32,
    pub range_base: i64,
    pub fuel: i64,
    pub fuel_requested: i64,
    pub underway: u8,
    pub replenishment: i32,
    pub replenishment_limit: i32,
    pub local_range: Option<i64>,
    pub local_range_factor: Option<i64>,
    pub weather_range: Option<i64>,
    pub weather_range_factor: Option<i64>,
}
impl NavalMissionTaskForceRow {
    /// Raw accumulator entries for a native mission modifier key.
    pub fn value(&self, key: u32) -> (Option<i64>, Option<i64>) {
        use engine_executable::modifier_keys as keys;
        match key {
            keys::NAVAL_COORDINATION => (self.local_coordination, self.weather_coordination),
            keys::CONVOY_RAIDING_EFFICIENCY_FACTOR => (self.local_raiding, self.weather_raiding),
            keys::NAVAL_RANGE => (self.local_range, self.weather_range),
            keys::NAVAL_RANGE_FACTOR => (self.local_range_factor, self.weather_range_factor),
            _ => unreachable!("unexpected naval mission task-force modifier key"),
        }
    }
}
typed_codec!(NavalMissionTaskForceRow, 64, |row| row.task_force as u128, |key| Self { task_force: key as u64, ..Default::default() },
 { 0 => (raiding_factor): I64, 1 => (local_coordination): OptionalI64, 2 => (local_raiding): OptionalI64, 3 => (weather_coordination): OptionalI64, 4 => (weather_raiding): OptionalI64, 5 => (leader): U64, 6 => (headquarters): U64List, 7 => (range_fleet): U64, 8 => (range_begin): I32, 9 => (range_end): I32, 10 => (convoys): I32, 11 => (range_base): I64, 12 => (fuel): I64, 13 => (fuel_requested): I64, 14 => (underway): U8, 15 => (replenishment): I32, 16 => (replenishment_limit): I32, 17 => (local_range): OptionalI64, 18 => (local_range_factor): OptionalI64, 19 => (weather_range): OptionalI64, 20 => (weather_range_factor): OptionalI64, 21 => (mission_owner): U32 }, |_row| Ok(()));

/// NavalMissionLedgers (hourly): one navy's mission ledger for a strategic
/// region, for every nonempty live ledger. Refresh can register fleet
/// regions outside TaskForceMissions' accessible list. `owner` is the navy's
/// canonical country id; `task_forces` keeps registration task forces in
/// stored order, repeats kept. The pride marker goes to the kind of the
/// last counted registration holding the pride of the fleet; escort
/// efficiency counts only each fleet's first escort registration.
///
/// `ring` says whether the navy keeps a presence ring for the region. An
/// absent ring gives `-100000`; otherwise `positive` and `total` count
/// entries above and at least zero within its first 720. This is read for
/// every nonempty ledger, including an escort animating out. The presence
/// key is the region's runtime slot, equal to its id by construction.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalMissionLedgerRow {
    pub owner: u32,
    pub region: u32,
    pub task_forces: Vec<u64>,
    pub ring: bool,
    pub positive: i32,
    pub total: i32,
}
typed_codec!(NavalMissionLedgerRow, 64, |row| (row.owner as u128) << 32 | row.region as u128, |key| Self { owner: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (task_forces): U64List, 1 => (ring): Bool, 2 => (positive): I32, 3 => (total): I32 },
 |row| if row.task_forces.is_empty() || (!row.ring && (row.positive, row.total) != (0, 0)) { Err(invalid("invalid naval mission ledger")) } else { Ok(()) });

/// NavalMissionCountries (sliced by country): every roster or mission owner
/// named by NavalMissionTaskForces, keyed by canonical country id. Values
/// are its accumulator entries: combined task-force modifiers start from
/// the roster owner's value, weather reduction uses its `0x248`, and escort
/// efficiency uses the mission owner's `0x10E`. Hubs keep the navy's stored
/// list. Up to a day old: each country is read in its daily slice, or at once
/// when new to the family or its commander fingerprint changed (see NavalMissionCommanders).
///
/// SliceReads witnesses the held owner slice's read hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalMissionCountryRow {
    pub country: u32,
    pub coordination: Option<i64>,
    pub raiding: Option<i64>,
    pub escort: Option<i64>,
    pub weather_reduction: Option<i64>,
    pub range: Option<i64>,
    pub range_factor: Option<i64>,
    pub underway: Option<i64>,
    pub hubs: Vec<u32>,
}
impl NavalMissionCountryRow {
    /// Raw accumulator entries for a native mission modifier key.
    pub fn value(&self, key: u32) -> Option<i64> {
        use engine_executable::modifier_keys as keys;
        match key {
            keys::NAVAL_COORDINATION => self.coordination,
            keys::CONVOY_RAIDING_EFFICIENCY_FACTOR => self.raiding,
            keys::CONVOY_ESCORT_EFFICIENCY => self.escort,
            keys::NAVY_WEATHER_PENALTY => self.weather_reduction,
            keys::NAVAL_RANGE => self.range,
            keys::NAVAL_RANGE_FACTOR => self.range_factor,
            keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR => self.underway,
            _ => unreachable!("unexpected naval mission country modifier key"),
        }
    }
}
typed_codec!(NavalMissionCountryRow, 32, |row| row.country as u128, |key| Self { country: key as u32, ..Default::default() },
 { 0 => (coordination): OptionalI64, 1 => (raiding): OptionalI64, 2 => (escort): OptionalI64, 3 => (weather_reduction): OptionalI64, 4 => (range): OptionalI64, 5 => (range_factor): OptionalI64, 6 => (underway): OptionalI64, 7 => (hubs): U32List }, |_row| Ok(()));

/// NavalMissionCommanders (sliced by country): modifier sources added for
/// a country's NavalMissionTaskForces (owner is their roster country).
/// Source 0 is a fleet leader (`reference` as in `leader`), using its
/// navy-leader accumulator. Sources 1 to 4 are a headquarters commander
/// (`reference` as in `headquarters`), whose four accumulators are summed
/// for a kind-2 unit leader in stored order. An
/// unresolved reference or commander of another unit-leader kind has rows
/// with every value absent. A missing row means the reference was not read.
/// The family's fingerprint covers each country's leader and commander
/// references, so a new one is read within the hour; a changed accumulator
/// waits for the country's slice.
///
/// SliceReads witnesses the held owner slice's read hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalMissionCommanderRow {
    pub country: u32,
    pub reference: u64,
    pub source: u8,
    pub coordination: Option<i64>,
    pub raiding: Option<i64>,
    pub range: Option<i64>,
    pub range_factor: Option<i64>,
}
impl NavalMissionCommanderRow {
    /// Sources: the fleet leader, then the four headquarters accumulators.
    pub const SOURCES: u8 = 5;
}
impl NavalMissionCommanderRow {
    /// Raw accumulator entries for a native mission modifier key.
    pub fn value(&self, key: u32) -> Option<i64> {
        use engine_executable::modifier_keys as keys;
        match key {
            keys::NAVAL_COORDINATION => self.coordination,
            keys::CONVOY_RAIDING_EFFICIENCY_FACTOR => self.raiding,
            keys::NAVAL_RANGE => self.range,
            keys::NAVAL_RANGE_FACTOR => self.range_factor,
            _ => unreachable!("unexpected naval mission commander modifier key"),
        }
    }
}
typed_codec!(NavalMissionCommanderRow, 104, |row| (row.country as u128) << 72 | (row.reference as u128) << 8 | row.source as u128, |key| Self { country: (key >> 72) as u32, reference: (key >> 8) as u64, source: key as u8, ..Default::default() },
 { 0 => (coordination): OptionalI64, 1 => (raiding): OptionalI64, 2 => (range): OptionalI64, 3 => (range_factor): OptionalI64 }, |row| if row.source >= NavalMissionCommanderRow::SOURCES { Err(invalid("invalid naval mission commander source")) } else { Ok(()) });
