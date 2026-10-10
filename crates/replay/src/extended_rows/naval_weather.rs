use super::*;

/// Hourly unit movement operands of every task force, keyed by handle:
/// movement target (zero for null), first combat's role (0 without a
/// combat, 1 attacking, else 2), and both routes in stored order. The viewer
/// derives stack kind and settled route point from these and content;
/// nothing of the map entity is recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TaskForceMapMotionRow {
    pub task_force: u64,
    pub target: u32,
    pub combat_role: u8,
    pub path: Vec<u32>,
    pub alternate_path: Vec<u32>,
    /// Fresh raw fixed5 movement numerator for arrow fill.
    pub movement_numerator: Recorded<Option<i64>>,
    /// Raw saved retreat/withdraw bytes, never UI state.
    pub retreat: Recorded<Option<u8>>,
    pub withdrawal: Recorded<Option<u8>>,
    /// Movement-target pointer presence, encoded 0/1; None is a local read refusal.
    pub naval_transfer: Recorded<Option<u8>>,
    /// Ordered CUnit combat-vector roots: first full reference, then first
    /// reached native virtual-type-2 reference. Zero proves absence; None is
    /// a local read/authentication refusal. Per-unit order is not saved.
    pub combat_links: Recorded<Option<[u64; 2]>>,
    /// Counter ETA and range operands acquired at this frame's hour. None is
    /// outside the ordinary-route/target-null native demand or a missing clock;
    /// each packet component is independently unavailable after a read refusal.
    pub fresh: Recorded<Option<FreshTaskForceCounterEta>>,
}
opaque_value!(Option<[u64; 2]>);
/// Current simulation context for a freshly searched task-force counter route.
/// Require `sample_hour == frame.hour`; never carry these through another hour.
/// All speeds/ranges are native signed fixed5. `speed_modifier` is the
/// unclamped common actual-location plus fuel-deficit value, before departure
/// terrain. `modified_range` precedes playback's content distance scale
/// and guarded half. Raw simulation bytes are not booleans.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FreshTaskForceCounterEta {
    pub sample_hour: i32,
    pub base_speed: Option<i64>,
    pub speed_modifier: Option<i64>,
    pub terrain_class: Option<i32>,
    pub modified_range: Option<i64>,
    pub exiled: Option<u8>,
    pub strategic_redeployment: Option<u8>,
}
value_struct!(FreshTaskForceCounterEta {
    sample_hour: i32, base_speed: Option<i64>, speed_modifier: Option<i64>,
    terrain_class: Option<i32>, modified_range: Option<i64>, exiled: Option<u8>,
    strategic_redeployment: Option<u8>,
});
opaque_value!(Option<FreshTaskForceCounterEta>);
typed_codec!(TaskForceMapMotionRow, 64, |r| r.task_force as u128, |k| Self { task_force: k as u64, ..Default::default() },
 { 0 => (target): U32, 1 => (combat_role): U8, 2 => (path): U32List, 3 => (alternate_path): U32List, 4 => (movement_numerator): OptionalI64, 5 => (retreat): Opaque, 6 => (withdrawal): Opaque, 7 => (naval_transfer): Opaque, 8 => (combat_links): Opaque, 9 => (fresh): Opaque }, |_r| Ok(()));

/// NavalMineLevels: the wing's effective country's stored mine amount for
/// the mission region, compared with `NNavy.NAVAL_MINES_IN_REGION_MAX`,
/// raw fixed5. One row per pair a registered mine-laying mission needs;
/// zero is kept.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalMineLevelRow {
    /// Canonical country slot.
    pub country: u32,
    pub region: u32,
    pub raw_mines: i64,
}
typed_codec!(NavalMineLevelRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (raw_mines): I64 }, |_row| Ok(()));

/// NavalRegionAccess: key raw country reference:strategic region id (64 bits);
/// one row per nonzero mode of the country's strategic navy, indexed by
/// strategic region id from `map/strategicregions`. `mode` is the stored
/// byte: 1 avoid, 2 block; no row means 0, allowed. Raid army searches use
/// the navy of the unit's positive expeditionary owner, else roster owner.
/// Entering a sea province of a different strategic region is rejected at
/// mode 2, and at mode 1 when the request avoids restricted regions (raids
/// do). Player UI and AI daily commands update the modes. Navy reset zeroes
/// them when a country is reused (civil war, dynamic countries); construction,
/// setup resizing and save loading build them. This set is not proven
/// complete: navy pointers held by other objects have not all been tracked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalRegionAccessRow {
    pub country: u32,
    pub region: u32,
    pub mode: u8,
}
typed_codec!(NavalRegionAccessRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (mode): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SunkShipEntry {
    pub ship_name: Vec<u8>,
    pub attribution_name: Vec<u8>,
    pub class_name: Vec<u8>,
    pub attribution_class_name: Vec<u8>,
    pub victim_country: u32,
    pub attacker_country: u32,
    pub sink_hour: i32,
    pub class_token: u32,
    pub attribution_class_token: u32,
    pub attribution_class_present: u8,
    pub model_level: i32,
    pub victim_equipment_variant: u64,
    pub damage_source_reference: u64,
    pub province: u32,
    pub province_present: u8,
    pub battle: u64,
    pub cause_record: u8,
    pub assisted_kill: u8,
}
value_struct!(SunkShipEntry {ship_name: Vec<u8>, attribution_name: Vec<u8>, class_name: Vec<u8>, attribution_class_name: Vec<u8>, victim_country: u32, attacker_country: u32, sink_hour: i32, class_token: u32, attribution_class_token: u32, attribution_class_present: u8, model_level: i32, victim_equipment_variant: u64, damage_source_reference: u64, province: u32, province_present: u8, battle: u64, cause_record: u8, assisted_kill: u8});

/// SunkConvoys: key month:killer_country:owner_country (96 bits), 2 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SunkConvoyRow {
    pub month: i32,
    pub killer_country: u32,
    pub owner_country: u32,
    pub convoys: i32,
    pub ordinal: u32,
}
typed_codec!(SunkConvoyRow, 96, |row| (row.month as u32 as u128) << 64 | (row.killer_country as u128) << 32 | row.owner_country as u128, |key| Self { month: (key >> 64) as u32 as i32, killer_country: (key >> 32) as u32, owner_country: key as u32, convoys: Default::default(), ordinal: Default::default() },
 { 0 => (convoys): I32, 1 => (ordinal): U32 }, |_row| Ok(()));

/// NavalResults: key id (64 bits), 10 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultRow {
    pub id: u64,
    pub province: u32,
    pub result_hour: i32,
    pub retire_hour: i32,
    pub category: u32,
    pub flag_a: u8,
    pub flag_b: u8,
    pub subtype: u8,
    pub readiness: u8,
    pub winning_side: u8,
    pub losing_side: u8,
    /// The record's place in its province's bucket, the order the map icon keeps.
    pub province_ordinal: u32,
}
typed_codec!(NavalResultRow, 64, |row| row.id as u128, |key| Self { id: key as u64, province: Default::default(), result_hour: Default::default(), retire_hour: Default::default(), category: Default::default(), flag_a: Default::default(), flag_b: Default::default(), subtype: Default::default(), readiness: Default::default(), winning_side: Default::default(), losing_side: Default::default(), province_ordinal: Default::default() },
 { 0 => (province): U32, 1 => (result_hour): I32, 2 => (retire_hour): I32, 3 => (category): U32, 4 => (flag_a): U8, 5 => (flag_b): U8, 6 => (subtype): U8, 7 => (readiness): U8, 8 => (winning_side): U8, 9 => (losing_side): U8, 10 => (province_ordinal): U32 }, |_row| Ok(()));

/// One combat-to-result transition. New results receive an independent
/// reference; flagged port/naval strikes can merge into a retained result.
/// `result` is the full NavalResults.id handle. Persistent `combat` is the
/// full NavalCombats.id handle; transient strikes preserve the invalid
/// handle and must not join by it. `id` orders events within the recording
/// branch; `game_hour` dates the transition, not the hourly drain. Each
/// persistent combat maps to at most one result; only transient strikes
/// merge. A merge overwrites result dates. Skipped creation contributes
/// no row; an incomplete interval is unavailable.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultSourceRow {
    pub id: u64,
    pub game_hour: i32,
    pub result: u64,
    pub combat: u64,
    pub merged: bool,
    /// Transient strikes never register their combat. Each event remains
    /// distinct even though its handle is invalid.
    pub transient: bool,
    /// Construction hour for a witnessed fresh birth; None for an unwitnessed
    /// persistent birth. Save loading is excluded. Transient construction and
    /// result creation occur in the same game call.
    pub combat_created_hour: Option<i32>,
}
typed_codec!(NavalResultSourceRow, 64, |row| row.id as u128,
 |key| Self { id: key as u64, ..Default::default() },
 { 0 => (game_hour): I32, 1 => (result): U64, 2 => (combat): U64, 3 => (merged): Bool, 4 => (transient): Bool, 5 => (combat_created_hour): OptionalI32 }, |row| {
    if row.transient && row.combat_created_hour != Some(row.game_hour) {
        Err(invalid("transient naval result source lacks same-hour construction"))
    } else { Ok(()) }
});

/// NavalResultMembers: key result:member (128 bits), 3 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultMemberRow {
    pub result: u64,
    pub member: u64,
    pub presence_value: u64,
    pub ordinal: u32,
    pub side: u8,
}
typed_codec!(NavalResultMemberRow, 128, |row| (row.result as u128) << 64 | row.member as u128, |key| Self { result: (key >> 64) as u64, member: key as u64, presence_value: Default::default(), ordinal: Default::default(), side: Default::default() },
 { 0 => (presence_value): U64, 1 => (ordinal): U32, 2 => (side): U8 }, |_row| Ok(()));

/// NavalResultCountries: key result:list:country (104 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalResultCountryRow {
    pub result: u64,
    pub country: u32,
    pub ordinal: u32,
    pub list: u8,
}
typed_codec!(NavalResultCountryRow, 104, |row| (row.result as u128) << 40 | (row.list as u128) << 32 | row.country as u128, |key| Self { result: (key >> 40) as u64, list: (key >> 32) as u8, country: key as u32, ordinal: Default::default() },
 { 0 => (ordinal): U32 }, |_row| Ok(()));

/// SunkShips: key singleton zero (0 bits), 1 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SunkShipsRow {
    pub entries: Vec<SunkShipEntry>,
}
typed_codec!(SunkShipsRow, 0, |_row| 0, |_key| Self { entries: Default::default() },
 { 0 => (entries): Opaque }, |_row| Ok(()));

/// A province with a nonempty combat vector: each entry's handle and
/// class in vector order. Naval-icon admission visits naval entries before
/// registry spillovers; icon rotation counts engaged entries of every
/// class. A nonempty vector raises a sea province's retained naval-result marker.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalProvinceCombatRow {
    pub province: u32,
    pub combats: Vec<u64>,
    /// One of [`Self::LAND`], [`Self::BORDER_WAR`] or [`Self::NAVAL`] per entry of `combats`.
    pub kinds: Vec<u8>,
}
impl NavalProvinceCombatRow {
    /// A `CLandCombat` entry.
    pub const LAND: u8 = 0;
    /// A `CLandBorderWarCombat` entry.
    pub const BORDER_WAR: u8 = 1;
    /// A `CNavalCombat` entry.
    pub const NAVAL: u8 = 2;
}
typed_codec!(NavalProvinceCombatRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (combats): U64List, 1 => (kinds): Bytes }, |row| if row.kinds.len() != row.combats.len() || row.kinds.iter().any(|&kind| kind > Self::NAVAL) { Err(invalid("invalid province combat kinds")) } else { Ok(()) });

/// A naval combat's transfer handles in stored order, including zero and
/// stale handles, walked for the spillover pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalCombatTransferRow {
    pub combat: u64,
    pub transfers: Vec<u64>,
}
typed_codec!(NavalCombatTransferRow, 64, |row| row.combat as u128, |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (transfers): U64List }, |_row| Ok(()));

/// A live transfer's live unit handles in stored order: nonzero references
/// that resolve to a unit and are saved. Stale handles are left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalTransferUnitRow {
    pub transfer: u64,
    pub units: Vec<u64>,
}
typed_codec!(NavalTransferUnitRow, 64, |row| row.transfer as u128, |key| Self { transfer: key as u64, ..Default::default() },
 { 0 => (units): U64List }, |_row| Ok(()));

/// One task force's registered naval mission regions, for task forces with
/// a current or accessible region. `owner` is the mission's raw country,
/// whose strategic navy registers it; `regions` is its accessible-region
/// list in order, repeats kept, as registered in the navy. `current_region`
/// is the region being patrolled, zero for none.
///
/// A task force with neither current nor accessible regions has no row, so a
/// row is never empty. Resolve owner through CountryLinks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TaskForceMissionRow {
    pub task_force: u64,
    pub owner: u32,
    pub current_region: u32,
    pub regions: Vec<u32>,
}
typed_codec!(TaskForceMissionRow, 64, |row| row.task_force as u128, |key| Self { task_force: key as u64, ..Default::default() },
 { 0 => (owner): U32, 1 => (current_region): U32, 2 => (regions): U32List }, |_row| Ok(()));

/// WeatherRegions: stored weather flags selecting map particles and naval
/// combat window icons, plus the manager's snow ceiling (set at load),
/// repeated on every row so it delta-encodes to nothing.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WeatherRegionRow {
    pub id: u32,
    pub max_snow: i64,
    pub light_rain: u8,
    pub heavy_rain: u8,
    pub snow: u8,
    pub blizzard: u8,
    pub sandstorm: u8,
    pub arctic_water: u8,
    /// Fresh target-region effective attack and breakthrough values.
    pub attack: Recorded<WeatherRegionAttackInputs>,
}
typed_codec!(WeatherRegionRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
 { 0 => (max_snow): I64, 1 => (light_rain): U8, 2 => (heavy_rain): U8, 3 => (snow): U8, 4 => (blizzard): U8, 5 => (sandstorm): U8, 6 => (arctic_water): U8, 7 => (attack): Opaque }, |_row| Ok(()));

/// Live display boundary, carried only by WeatherProvinces slot zero.
/// These cells are excluded from dirty updates and rebuilt separately.
/// Sampling current snow/mud planes each acquisition needs no startup/load
/// witness, including attachment in the middle of a game.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WeatherDisplay {
    pub width: u32,
    pub height: u32,
    /// Content-loaded fixed5 seconds.
    pub refresh_interval: i64,
    /// Stored refresh f64 bits: equality only, never animation time.
    pub refresh_identity: u64,
    /// Planar snow then mud; right column south first without the northern
    /// corner, then the full northern row. Exactly 2*(width+height-1) bytes.
    pub edges: Vec<u8>,
}
value_struct!(WeatherDisplay { width: u32, height: u32, refresh_interval: i64,
    refresh_identity: u64, edges: Vec<u8> });
opaque_value!(Option<WeatherDisplay>);

/// WeatherProvinces: stored snow (winter grading, movement snow, combat
/// terrain), permanent snow (set at load and added to movement snow), and
/// the manager's two per-province display bytes averaged by the snow/mud
/// map texture. The political province tooltip reads full fixed5 temperature
/// and the raw mud byte. Display mud cannot substitute: raw and custom mud
/// are ORed before writing it. The tooltip also reads held local hour and
/// night from the manager's dense day-night array (i32 and fixed5 i64).
/// Weather suspension skips their writer, so these hourly stored values
/// remain authoritative after a seek or suspended reset. A newly allocated
/// row starts with local_hour zero; neither field is written to text saves.
/// A weather reset preserves existing day-night rows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WeatherProvinceRow {
    pub id: u32,
    pub snow: i64,
    pub permanent_snow: i64,
    pub display_snow: u8,
    pub display_mud: u8,
    pub raw_mud: Recorded<u8>,
    pub temperature: Recorded<i64>,
    pub local_hour: Recorded<i32>,
    pub night_factor: Recorded<i64>,
    /// Slot zero: Some(live boundary), None on an independent guarded refusal.
    /// Other native slots: known None. Historical NotRecorded is distinct.
    pub display: Recorded<Option<WeatherDisplay>>,
}
typed_codec!(WeatherProvinceRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
 { 0 => (snow): I64, 1 => (permanent_snow): I64, 2 => (display_snow): U8, 3 => (display_mud): U8, 4 => (temperature): I64, 5 => (raw_mud): U8, 6 => (local_hour): I32, 7 => (night_factor): I64, 8 => (display): Opaque }, |_row| Ok(()));

#[cfg(test)]
#[path = "naval_weather/tests.rs"]
mod tests;

/// NavalSupremacy: key region:country (64 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NavalSupremacyRow {
    pub region: u32,
    pub region_slot: u32,
    pub country: u32,
    pub current: i64,
    pub base_target: i64,
    pub target: i64,
    pub individual_ratio: i64,
    pub previous: i64,
    pub decline_from: i64,
}
typed_codec!(NavalSupremacyRow, 64, |row| (row.region as u128) << 32 | row.country as u128, |key| Self { region: (key >> 32) as u32, country: key as u32, region_slot: Default::default(), current: Default::default(), base_target: Default::default(), target: Default::default(), individual_ratio: Default::default(), previous: Default::default(), decline_from: Default::default() },
 { 0 => (region_slot): U32, 1 => (current): I64, 2 => (base_target): I64, 3 => (target): I64, 4 => (individual_ratio): I64, 5 => (previous): I64, 6 => (decline_from): I64 }, |_row| Ok(()));

opaque_value!(Vec<SunkShipEntry>);
