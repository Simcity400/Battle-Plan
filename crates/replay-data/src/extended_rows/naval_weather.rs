use super::*;

/// Hourly unit movement operands of every task force, keyed by handle:
/// movement target (zero for null), first combat's role (0 without a
/// combat, 1 attacking, else 2), and both routes in stored order. The viewer
/// derives stack kind and settled route point from these and content;
/// nothing of the map entity is recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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
    /// Ordered unit combat-vector roots: first full reference, then first reached native
    /// virtual-type-2 reference. Zero proves absence; None is a local read/authentication
    /// refusal. Per-unit order is not saved.
    pub combat_links: Recorded<Option<[u64; 2]>>,
    /// Counter ETA and range operands acquired at this frame's hour. None is
    /// outside the ordinary-route/target-null native demand or a missing clock;
    /// each packet component is independently unavailable after a read refusal.
    pub fresh: Recorded<Option<FreshTaskForceCounterEta>>,
}
opaque_value!(Option<[u64; 2]>);
/// Current simulation context for a freshly searched task-force counter route.
/// Require `sample_hour.fresh_at(frame.hour)`; never carry these through another hour.
/// All speeds/ranges are native signed fixed5. `speed_modifier` is the
/// unclamped common actual-location plus fuel-deficit value, before departure
/// terrain. `modified_range` precedes playback's content distance scale
/// and guarded half. Raw simulation bytes are not booleans.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct FreshTaskForceCounterEta {
    pub sample_hour: PacketHour,
    pub base_speed: Option<i64>,
    pub speed_modifier: Option<i64>,
    pub terrain_class: Option<i32>,
    pub modified_range: Option<i64>,
    pub exiled: Option<u8>,
    pub strategic_redeployment: Option<u8>,
}
value_struct!(FreshTaskForceCounterEta {
    sample_hour: PacketHour, base_speed: Option<i64>, speed_modifier: Option<i64>,
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
pub struct NavalRegionAccessRow {
    pub country: u32,
    pub region: u32,
    pub mode: u8,
}
typed_codec!(NavalRegionAccessRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (mode): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
pub struct NavalProvinceCombatRow {
    pub province: u32,
    pub combats: Vec<u64>,
    /// One of [`Self::LAND`], [`Self::BORDER_WAR`] or [`Self::NAVAL`] per entry of `combats`.
    pub kinds: Vec<u8>,
}
impl NavalProvinceCombatRow {
    /// A `land combat` entry.
    pub const LAND: u8 = 0;
    /// A `border war combat` entry.
    pub const BORDER_WAR: u8 = 1;
    /// A `naval combat` entry.
    pub const NAVAL: u8 = 2;
}
typed_codec!(NavalProvinceCombatRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (combats): U64List, 1 => (kinds): Bytes }, |row| if row.kinds.len() != row.combats.len() || row.kinds.iter().any(|&kind| kind > Self::NAVAL) { Err(invalid("invalid province combat kinds")) } else { Ok(()) });

/// A naval combat's transfer handles in stored order, including zero and
/// stale handles, walked for the spillover pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatTransferRow {
    pub combat: u64,
    pub transfers: Vec<u64>,
}
typed_codec!(NavalCombatTransferRow, 64, |row| row.combat as u128, |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (transfers): U64List }, |_row| Ok(()));

/// A live transfer's live unit handles in stored order: nonzero references
/// that resolve to a unit and are saved. Stale handles are left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
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
#[derive(replay_spool::Spooled)]
pub struct WeatherRegionRow {
    pub id: u32,
    pub max_snow: i64,
    pub light_rain: u8,
    pub heavy_rain: u8,
    pub snow: u8,
    pub blizzard: u8,
    pub sandstorm: u8,
    pub arctic_water: u8,
    /// The effective region attack and breakthrough values recordings
    /// through format 101 captured ([`WeatherRegionAttackInputs`]); current
    /// writers leave it empty ([`Witnessed`]) and record [`Self::combat`].
    pub attack: Recorded<Witnessed<WeatherRegionAttackInputs>>,
    /// The region weather modifier's raw `army_attack_factor` and
    /// `breakthrough_factor` entries ([`RegionCombatWeather`]), which the
    /// selected army's attack list scales by acclimatization. None is a read
    /// refusal; older formats use NotRecorded.
    pub combat: Recorded<Option<RegionCombatWeather>>,
    /// Raw regional Air detection modifier. Outer absence refuses the accumulator;
    /// inner absence is a known missing key, evaluated as zero.
    pub air_detection: Recorded<Option<Option<i64>>>,
    /// Complete raw regional accumulator observation for the independent map terms below.
    /// False refuses them; true with an absent scalar proves a missing key.
    pub map_modifiers_known: Recorded<bool>,
    pub naval_detection: Recorded<Option<i64>>,
    pub air_attack: Recorded<Option<i64>>,
    pub air_defence: Recorded<Option<i64>>,
    pub air_efficiency: Recorded<Option<std::sync::Arc<Vec<ModifierBreakdownNode>>>>,
}
typed_codec!(WeatherRegionRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
{ 0 => (max_snow): I64, 1 => (light_rain): U8, 2 => (heavy_rain): U8, 3 => (snow): U8, 4 => (blizzard): U8, 5 => (sandstorm): U8, 6 => (arctic_water): U8, 7 => (attack): Opaque, 8 => (combat): Opaque, 9 => (air_detection): Opaque, 10 => (map_modifiers_known): Bool, 11 => (naval_detection): OptionalI64, 12 => (air_attack): OptionalI64, 13 => (air_defence): OptionalI64, 14 => (air_efficiency): Opaque }, |row| {
   if let Recorded::Value(Some(nodes)) = &row.air_efficiency { check_tree(nodes)?; }
   Ok(())
});

/// Live display boundary, carried only by WeatherProvinces slot zero.
/// These cells are excluded from dirty updates and rebuilt separately.
/// Sampling current snow/mud planes each acquisition needs no startup/load
/// witness, including attachment in the middle of a game.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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

/// Forecast direction from the shared native day/night manager. A clear
/// update gate proves the direction from the hour; an independently failed
/// held observation never becomes a calendar-derived value.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct WeatherDaylight {
    pub updated: bool,
    pub held: Option<[i64; 3]>,
    pub held_feather: Option<[i64; 2]>,
}
value_struct!(WeatherDaylight { updated: bool, held: Option<[i64; 3]>, held_feather: Option<[i64; 2]> });
opaque_value!(Option<WeatherDaylight>);

impl WeatherDaylight {
    pub fn direction(self, hour: i32) -> Result<[i64; 3], &'static str> {
        if self.updated {
            Ok(crate::day_night::seasonal_direction(hour))
        } else {
            self.held
                .ok_or("held weather daylight direction unavailable")
        }
    }
    pub fn feather(self, defines: &crate::day_night::Defines) -> Result<[i64; 2], &'static str> {
        if self.updated {
            Ok(crate::day_night::forecast_feather(
                defines.day_night_feather,
            ))
        } else {
            self.held_feather
                .ok_or("held weather daylight feather unavailable")
        }
    }
}

/// WeatherProvinces: the snow word the province weather record holds (winter grading, movement
/// snow, combat terrain), its permanent snow (set at load; movement snow adds it) and the two
/// display bytes the weather manager keeps for the province, which the snow/mud map texture
/// averages. The political province tooltip reads the full fixed5 temperature
/// ([`ProvinceTemperature`]) and raw mud byte. Display mud cannot substitute: the weather display
/// ORs raw and custom mud before writing that byte. The province's day/night record is
/// [`DayNightClock`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct WeatherProvinceRow {
    pub id: u32,
    pub snow: i64,
    pub permanent_snow: i64,
    pub display_snow: u8,
    pub display_mud: u8,
    pub raw_mud: Recorded<u8>,
    pub temperature: ProvinceTemperature,
    pub clock: DayNightClock,
    /// Slot zero: Some(live boundary), None on an independent guarded refusal.
    /// Other native slots: known None. Historical NotRecorded is distinct.
    pub display: Recorded<Option<WeatherDisplay>>,
    /// Slot zero: the hourly temperature pass's inputs since the rows'
    /// anchor, None when the collector could not read the hour whole. Other
    /// native slots: known None. Recordings before format 102 hold none.
    pub temperature_pass: Recorded<Option<TemperaturePass>>,
    /// The province weather record's modifier accumulator, every entry
    /// ([`ProvinceWeatherModifiers`]), read with the record (the window's provinces
    /// afresh, the rest carried). None is this province's read refusal; older formats
    /// use NotRecorded.
    pub modifiers: Recorded<Option<ProvinceWeatherModifiers>>,
    /// Slot zero's seasonal forecast direction. Updated direction derives
    /// from the frame's hour; held direction retains suspension/startup state.
    pub daylight: Recorded<Option<WeatherDaylight>>,
}
typed_codec!(WeatherProvinceRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
{ 0 => (snow): I64, 1 => (permanent_snow): I64, 2 => (display_snow): U8, 3 => (display_mud): U8, 4 => (temperature.temperature): I64, 5 => (raw_mud): U8, 6 => (clock.local_hour): I32, 7 => (clock.night_factor): I64, 8 => (display): Opaque, 9 => (clock.source): U8, 10 => (temperature.drift): I64, 11 => (temperature.staged): I64, 12 => (temperature.working): I64, 13 => (temperature_pass): Opaque, 14 => (modifiers): Opaque, 15 => (daylight): Opaque },
|row| {
   row.clock.canonical().map_err(invalid)?;
   row.temperature.record().map(|_| ()).map_err(invalid)?;
   match &row.modifiers {
       Recorded::Value(Some(modifiers)) => modifiers.canonical().map_err(invalid),
       _ => Ok(()),
   }
});

/// One entry of a province weather record's modifier accumulator: a modifier definition
/// key and its raw fixed5 value, the sum of that key over the static modifiers the record
/// holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct ProvinceModifierEntry {
    pub key: u32,
    pub value: i64,
}
value_struct!(ProvinceModifierEntry {
    key: u32,
    value: i64
});

/// A province weather record's modifier accumulator: every entry, keys ascending. The game
/// rebuilds it from the province's mud, its temperature range's and ground snow's static
/// modifiers and the modifiers script and building damage add to the province, and keeps
/// it until the next rebuild. The terrain map mode's hover describes it whole; the movement
/// speed adds `army_speed_factor` and, for a move into a province held by an enemy,
/// `army_attack_speed_factor`, each scaled by the division's acclimatization factor; a
/// moving division at war takes `attrition`, and `heat_attrition` and `winter_attrition`
/// scaled by acclimatization and the country's and commander's thermal factors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct ProvinceWeatherModifiers {
    pub entries: Vec<ProvinceModifierEntry>,
}
value_struct!(ProvinceWeatherModifiers { entries: Vec<ProvinceModifierEntry> });

impl ProvinceWeatherModifiers {
    /// The game's accumulator holds each key once, positive and ascending, as the
    /// collector reads it: a duplicate, unordered or zero key is malformed, so no
    /// consumer's search or map collapses or misses an entry.
    pub fn canonical(&self) -> Result<(), &'static str> {
        let mut previous = 0;
        for entry in &self.entries {
            if entry.key <= previous {
                return Err("province weather modifier keys not positive and increasing");
            }
            previous = entry.key;
        }
        Ok(())
    }

    /// The raw entry of `key`; `None` when the accumulator has none.
    pub fn get(&self, key: u32) -> Option<i64> {
        self.entries
            .binary_search_by_key(&key, |entry| entry.key)
            .ok()
            .map(|at| self.entries[at].value)
    }
}
opaque_value!(Option<ProvinceWeatherModifiers>);

/// A province record's temperature words, the `province weather` record's committed temperature,
/// committed drift, staged temperature and working drift.
///
/// From format 102 a row holds the record's words at its frame's anchor
/// ([`TemperaturePass::anchor`]): committed temperature and drift in fields 4 and 10, and the
/// staged words as their wrapping differences from those two in fields 11 and 12, zero after every
/// commit. The hour's temperature is the hourly pass's ([`crate::temperature`]) from the anchor
/// through the recorded hours. Formats 93 to 101 recorded the committed temperature at the row's
/// hour in field 4 alone; earlier formats none. Format-102 captures written before the anchored
/// words share the version but not the layout, so their blocks read as malformed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct ProvinceTemperature {
    temperature: Recorded<i64>,
    drift: Recorded<i64>,
    staged: Recorded<i64>,
    working: Recorded<i64>,
}

/// What a [`ProvinceTemperature`] establishes about its record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemperatureRecord {
    /// The record's words at the frame's anchor.
    Anchored(crate::temperature::State),
    /// The temperature the record held at the row's hour (formats 93 to 101).
    AtHour(i64),
    NotRecorded,
}

impl ProvinceTemperature {
    /// A record's words at its frame's anchor.
    pub const fn anchored(state: crate::temperature::State) -> Self {
        Self {
            temperature: Recorded::Value(state.temperature),
            drift: Recorded::Value(state.drift),
            staged: Recorded::Value(state.staged.wrapping_sub(state.temperature)),
            working: Recorded::Value(state.working.wrapping_sub(state.drift)),
        }
    }

    /// A temperature recorded at the row's hour (formats 93 to 101).
    pub const fn historical(temperature: i64) -> Self {
        Self {
            temperature: Recorded::Value(temperature),
            drift: Recorded::NotRecorded,
            staged: Recorded::NotRecorded,
            working: Recorded::NotRecorded,
        }
    }

    /// Recordings before format 93.
    pub const NOT_RECORDED: Self = Self {
        temperature: Recorded::NotRecorded,
        drift: Recorded::NotRecorded,
        staged: Recorded::NotRecorded,
        working: Recorded::NotRecorded,
    };

    /// The stored words in their only encodings: all four, field 4 alone, or none.
    pub fn record(&self) -> Result<TemperatureRecord, &'static str> {
        use Recorded::{NotRecorded, Value};
        match (self.temperature, self.drift, self.staged, self.working) {
            (Value(temperature), Value(drift), Value(staged), Value(working)) => {
                Ok(TemperatureRecord::Anchored(crate::temperature::State {
                    temperature,
                    drift,
                    staged: staged.wrapping_add(temperature),
                    working: working.wrapping_add(drift),
                }))
            }
            (Value(temperature), NotRecorded, NotRecorded, NotRecorded) => {
                Ok(TemperatureRecord::AtHour(temperature))
            }
            (NotRecorded, NotRecorded, NotRecorded, NotRecorded) => {
                Ok(TemperatureRecord::NotRecorded)
            }
            _ => Err("WeatherProvinces temperature words are malformed"),
        }
    }
}

/// What the hourly temperature pass read since a frame's anchor, carried by WeatherProvinces slot
/// zero. The rows hold every record's words read at `anchor`, after that hour's update, with the
/// weather manager's update cursor then. `hours` has one entry for each later hour through the
/// frame's: the manager's seed after the hourly weather update added a synchronized 31-bit random
/// integer, when both suspension gates were clear and the cursor and records moved as one ordinary
/// pass moves them, or None for an hour whose suspended manager moved nothing. The collector starts
/// a new anchor at any other hour and every two `KEYFRAME_INTERVAL`s of hours. `anchor_seed` is the
/// anchor hour's own seed when the hour before proves that hour's ordinary pass, which lets an
/// anchor be compared with its derivation; `witness` is [`crate::temperature::witness`] of every
/// record's committed temperature read at the frame's hour.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct TemperaturePass {
    pub anchor: i32,
    pub cursor: i32,
    pub anchor_seed: Option<u32>,
    pub hours: Vec<Option<u32>>,
    pub witness: TemperatureWitness,
}
value_struct!(TemperaturePass { anchor: i32, cursor: i32, anchor_seed: Option<u32>,
    hours: Vec<Option<u32>>, witness: TemperatureWitness });
opaque_value!(Option<TemperaturePass>);

/// A [`TemperaturePass`]'s [`crate::temperature::witness`], eight little-endian bytes. Recordings
/// before [`PROVINCE_WITNESS_FORMAT`](crate::rows::compatibility::PROVINCE_WITNESS_FORMAT) hold an
/// order-dependent checksum in those bytes, which decodes as no witness, so their derived
/// temperatures are refused by name.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct TemperatureWitness(Option<u64>);

impl TemperatureWitness {
    pub const fn new(witness: u64) -> Self {
        Self(Some(witness))
    }

    /// None where the recording predates the witness.
    pub const fn get(self) -> Option<u64> {
        self.0
    }
}

impl Value for TemperatureWitness {
    const MINIMUM_ENCODED_BYTES: usize = size_of::<u64>();
    fn heap_bytes(&self) -> usize {
        0
    }
    fn encoded_bytes(&self) -> io::Result<usize> {
        Ok(size_of::<u64>())
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }
    fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
        let witness = self
            .0
            .ok_or_else(|| invalid("a temperature pass without its witness"))?;
        output.extend_from_slice(&witness.to_le_bytes());
        Ok(())
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        let witness = <u64 as Value>::decode(input)?;
        Ok(Self(
            (input.format >= crate::rows::compatibility::PROVINCE_WITNESS_FORMAT)
                .then_some(witness),
        ))
    }
}
// Checkpoints keep a decoded older pass's absent witness, which recordings never write.
impl crate::checkpoint::values::CheckpointValue for TemperatureWitness {
    const MINIMUM_BYTES: usize =
        <Option<u64> as crate::checkpoint::values::CheckpointValue>::MINIMUM_BYTES;
    fn encode(&self, output: &mut Vec<u8>) -> io::Result<()> {
        crate::checkpoint::values::CheckpointValue::encode(&self.0, output)
    }
    fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
        <Option<u64> as crate::checkpoint::values::CheckpointValue>::decode(input).map(Self)
    }
}

/// The clock a province slot's day/night record holds at the boundary: the manager's dense
/// day/night array (stride 0x330), the local hour (i32) and night factor (fixed5), which the
/// getters return to the province tooltip, land combat and air spotting. Neither is written to text
/// saves.
///
/// The hourly weather update runs the day/night update whenever it finds both suspension gates
/// clear, and that writes every slot from 1 as a function of the date and the content
/// ([`crate::day_night`]). Such a row records only that the update ran ([`ClockState::Updated`]).
/// Slot 0, which no update writes, every slot of a suspended manager (the console weather toggle,
/// which keeps the existing rows) and a row whose gates were not read record the clock they hold
/// ([`ClockState::Held`]). A newly allocated record holds hour 0 and night 100000 until its first
/// update.
///
/// Format 102 adds the source word (field 9); fields 6 and 7 then hold a
/// held row's clock and zero for an updated one. Format 101 recorded every
/// row's clock in fields 6 and 7, which read as held; earlier formats none.
/// The default is [`Self::UPDATED`], whose words are all zero.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct DayNightClock {
    local_hour: Recorded<i32>,
    night_factor: Recorded<i64>,
    source: Recorded<u8>,
}

/// What a [`DayNightClock`] establishes about the record at its row's hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockState {
    /// The native writer updated the record at this hour: derive the clock
    /// from the hour and the content ([`crate::day_night::ProvinceClock`]).
    Updated,
    /// The record holds this clock, which no hour determines.
    Held(crate::day_night::Clock),
    /// Recordings before format 101 hold neither.
    NotRecorded,
}

impl DayNightClock {
    const UPDATED_SOURCE: u8 = 0;
    const HELD_SOURCE: u8 = 1;

    /// A record the native writer updated at the row's hour.
    pub const UPDATED: Self = Self {
        local_hour: Recorded::Value(0),
        night_factor: Recorded::Value(0),
        source: Recorded::Value(Self::UPDATED_SOURCE),
    };

    /// A record holding `clock`, which no update at the row's hour wrote.
    pub const fn held(clock: crate::day_night::Clock) -> Self {
        Self {
            local_hour: Recorded::Value(clock.local_hour),
            night_factor: Recorded::Value(clock.night_factor),
            source: Recorded::Value(Self::HELD_SOURCE),
        }
    }

    /// Format 101's recorded clock, which every row held in fields 6 and 7.
    pub const fn historical(clock: crate::day_night::Clock) -> Self {
        Self {
            local_hour: Recorded::Value(clock.local_hour),
            night_factor: Recorded::Value(clock.night_factor),
            source: Recorded::NotRecorded,
        }
    }

    /// Recordings before format 101.
    pub const NOT_RECORDED: Self = Self {
        local_hour: Recorded::NotRecorded,
        night_factor: Recorded::NotRecorded,
        source: Recorded::NotRecorded,
    };

    /// The record of province slot `province`. Slot 0 is never updated (the day/night
    /// pass starts at 1), so an updated slot 0 is refused.
    pub fn state(&self, province: u32) -> Result<ClockState, &'static str> {
        match self.canonical()? {
            ClockState::Updated if province == 0 => {
                Err("WeatherProvinces slot 0 records an update no native writer makes")
            }
            state => Ok(state),
        }
    }

    /// The stored words in their only encodings: an updated record carries
    /// no clock words, and the source is updated (0) or held (1).
    pub fn canonical(&self) -> Result<ClockState, &'static str> {
        match (self.source, self.local_hour, self.night_factor) {
            (Recorded::Value(Self::UPDATED_SOURCE), Recorded::Value(0), Recorded::Value(0)) => {
                Ok(ClockState::Updated)
            }
            (
                Recorded::Value(Self::HELD_SOURCE) | Recorded::NotRecorded,
                Recorded::Value(local_hour),
                Recorded::Value(night_factor),
            ) => Ok(ClockState::Held(crate::day_night::Clock {
                local_hour,
                night_factor,
            })),
            (Recorded::NotRecorded, Recorded::NotRecorded, Recorded::NotRecorded) => {
                Ok(ClockState::NotRecorded)
            }
            _ => Err("WeatherProvinces day/night clock is malformed"),
        }
    }
}

#[cfg(test)]
#[path = "naval_weather/tests.rs"]
mod tests;

/// NavalSupremacy: key region:country (64 bits), 7 payload mask bits.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
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
