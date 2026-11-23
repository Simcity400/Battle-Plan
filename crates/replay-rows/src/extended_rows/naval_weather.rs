use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TaskForceMapMotionRow {
    pub task_force: u64,
    pub target: u32,
    pub combat_role: u8,
    pub path: Vec<u32>,
    pub alternate_path: Vec<u32>,
    pub movement_numerator: Recorded<Option<i64>>,
    pub retreat: Recorded<Option<u8>>,
    pub withdrawal: Recorded<Option<u8>>,
    pub naval_transfer: Recorded<Option<u8>>,
    pub combat_links: Recorded<Option<[u64; 2]>>,
    pub fresh: Recorded<Option<FreshTaskForceCounterEta>>,
}
opaque_value!(Option<[u64; 2]>);
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
impl OpaqueValue for FreshTaskForceCounterEta {}
typed_codec!(TaskForceMapMotionRow, 64, |r| r.task_force as u128, |k| Self { task_force: k as u64, ..Default::default() },
 { 0 => (target): U32, 1 => (combat_role): U8, 2 => (path): U32List, 3 => (alternate_path): U32List, 4 => (movement_numerator): OptionalI64, 5 => (retreat): Opaque, 6 => (withdrawal): Opaque, 7 => (naval_transfer): Opaque, 8 => (combat_links): Opaque, 9 => (fresh): Opaque }, |_r| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalMineLevelRow {
    pub country: u32,
    pub region: u32,
    pub raw_mines: i64,
}
typed_codec!(NavalMineLevelRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (raw_mines): I64 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalRegionAccessRow {
    pub country: u32,
    pub region: u32,
    pub mode: u8,
}
typed_codec!(NavalRegionAccessRow, 64, |row| (row.country as u128) << 32 | row.region as u128,
 |key| Self { country: (key >> 32) as u32, region: key as u32, ..Default::default() },
 { 0 => (mode): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SunkConvoyRow {
    pub month: i32,
    pub killer_country: u32,
    pub owner_country: u32,
    pub convoys: i32,
    pub ordinal: u32,
}
typed_codec!(SunkConvoyRow, 96, |row| (row.month as u32 as u128) << 64 | (row.killer_country as u128) << 32 | row.owner_country as u128, |key| Self { month: (key >> 64) as u32 as i32, killer_country: (key >> 32) as u32, owner_country: key as u32, convoys: Default::default(), ordinal: Default::default() },
 { 0 => (convoys): I32, 1 => (ordinal): U32 }, |_row| Ok(()));

impl SunkConvoyRow {
    pub fn month_of(hour: i32) -> Option<i32> {
        let elapsed = i64::from(hour) - i64::from(game_model::calendar::HOUR_EPOCH);
        if elapsed < 0 {
            return None;
        }
        let mut day = (elapsed / 24) % 365;
        let mut month = 0;
        for length in game_model::calendar::MONTH_LENGTHS {
            if day < i64::from(length) {
                break;
            }
            day -= i64::from(length);
            month += 1;
        }
        i32::try_from(elapsed / (365 * 24) * 12 + month).ok()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    pub province_ordinal: u32,
}
typed_codec!(NavalResultRow, 64, |row| row.id as u128, |key| Self { id: key as u64, province: Default::default(), result_hour: Default::default(), retire_hour: Default::default(), category: Default::default(), flag_a: Default::default(), flag_b: Default::default(), subtype: Default::default(), readiness: Default::default(), winning_side: Default::default(), losing_side: Default::default(), province_ordinal: Default::default() },
 { 0 => (province): U32, 1 => (result_hour): I32, 2 => (retire_hour): I32, 3 => (category): U32, 4 => (flag_a): U8, 5 => (flag_b): U8, 6 => (subtype): U8, 7 => (readiness): U8, 8 => (winning_side): U8, 9 => (losing_side): U8, 10 => (province_ordinal): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalResultSourceRow {
    pub id: u64,
    pub game_hour: i32,
    pub result: u64,
    pub combat: u64,
    pub merged: bool,
    pub transient: bool,
    pub combat_created_hour: Option<i32>,
}
typed_codec!(NavalResultSourceRow, 64, |row| row.id as u128,
 |key| Self { id: key as u64, ..Default::default() },
 { 0 => (game_hour): I32, 1 => (result): U64, 2 => (combat): U64, 3 => (merged): Bool, 4 => (transient): Bool, 5 => (combat_created_hour): OptionalI32 }, |row| {
    if row.transient && row.combat_created_hour != Some(row.game_hour) {
        Err(invalid("transient naval result source lacks same-hour construction"))
    } else { Ok(()) }
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalResultMemberRow {
    pub result: u64,
    pub member: u64,
    pub presence_value: u64,
    pub ordinal: u32,
    pub side: u8,
}
typed_codec!(NavalResultMemberRow, 128, |row| (row.result as u128) << 64 | row.member as u128, |key| Self { result: (key >> 64) as u64, member: key as u64, presence_value: Default::default(), ordinal: Default::default(), side: Default::default() },
 { 0 => (presence_value): U64, 1 => (ordinal): U32, 2 => (side): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalResultCountryRow {
    pub result: u64,
    pub country: u32,
    pub ordinal: u32,
    pub list: u8,
}
typed_codec!(NavalResultCountryRow, 104, |row| (row.result as u128) << 40 | (row.list as u128) << 32 | row.country as u128, |key| Self { result: (key >> 40) as u64, list: (key >> 32) as u8, country: key as u32, ordinal: Default::default() },
 { 0 => (ordinal): U32 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct SunkShipsRow {
    pub entries: Vec<SunkShipEntry>,
}
typed_codec!(SunkShipsRow, 0, |_row| 0, |_key| Self { entries: Default::default() },
 { 0 => (entries): Opaque }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalProvinceCombatRow {
    pub province: u32,
    pub combats: Vec<u64>,
    pub kinds: Vec<u8>,
}
impl NavalProvinceCombatRow {
    pub const LAND: u8 = 0;
    pub const BORDER_WAR: u8 = 1;
    pub const NAVAL: u8 = 2;
}
typed_codec!(NavalProvinceCombatRow, 32, |row| row.province as u128, |key| Self { province: key as u32, ..Default::default() },
 { 0 => (combats): U64List, 1 => (kinds): Bytes }, |row| if row.kinds.len() != row.combats.len() || row.kinds.iter().any(|&kind| kind > Self::NAVAL) { Err(invalid("invalid province combat kinds")) } else { Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatTransferRow {
    pub combat: u64,
    pub transfers: Vec<u64>,
}
typed_codec!(NavalCombatTransferRow, 64, |row| row.combat as u128, |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (transfers): U64List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalTransferUnitRow {
    pub transfer: u64,
    pub units: Vec<u64>,
}
typed_codec!(NavalTransferUnitRow, 64, |row| row.transfer as u128, |key| Self { transfer: key as u64, ..Default::default() },
 { 0 => (units): U64List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TaskForceMissionRow {
    pub task_force: u64,
    pub owner: u32,
    pub current_region: u32,
    pub regions: Vec<u32>,
}
typed_codec!(TaskForceMissionRow, 64, |row| row.task_force as u128, |key| Self { task_force: key as u64, ..Default::default() },
 { 0 => (owner): U32, 1 => (current_region): U32, 2 => (regions): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WeatherRegionRow {
    pub id: u32,
    pub max_snow: i64,
    pub light_rain: u8,
    pub heavy_rain: u8,
    pub snow: u8,
    pub blizzard: u8,
    pub sandstorm: u8,
    pub arctic_water: u8,
    pub attack: Recorded<Witnessed<WeatherRegionAttackInputs>>,
    pub combat: Recorded<Option<RegionCombatWeather>>,
    pub air_detection: Recorded<Option<Option<i64>>>,
    pub map_modifiers_known: Recorded<bool>,
    pub naval_detection: Recorded<Option<i64>>,
    pub air_attack: Recorded<Option<i64>>,
    pub air_defence: Recorded<Option<i64>>,
    pub air_efficiency: Recorded<Option<std::sync::Arc<Vec<ModifierBreakdownNode>>>>,
    pub custom_sea: Recorded<Option<Vec<i16>>>,
}
opaque_value!(Option<Vec<i16>>);
typed_codec!(WeatherRegionRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
{ 0 => (max_snow): I64, 1 => (light_rain): U8, 2 => (heavy_rain): U8, 3 => (snow): U8, 4 => (blizzard): U8, 5 => (sandstorm): U8, 6 => (arctic_water): U8, 7 => (attack): Opaque, 8 => (combat): Opaque, 9 => (air_detection): Opaque, 10 => (map_modifiers_known): Bool, 11 => (naval_detection): OptionalI64, 12 => (air_attack): OptionalI64, 13 => (air_defence): OptionalI64, 14 => (air_efficiency): Opaque, 15 => (custom_sea): Opaque }, |row| {
   if let Recorded::Value(Some(nodes)) = &row.air_efficiency { check_tree(nodes)?; }
   Ok(())
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WeatherDisplay {
    pub width: u32,
    pub height: u32,
    pub refresh_interval: i64,
    pub refresh_identity: u64,
    pub edges: Vec<u8>,
}
value_struct!(WeatherDisplay { width: u32, height: u32, refresh_interval: i64,
    refresh_identity: u64, edges: Vec<u8> });
impl OpaqueValue for WeatherDisplay {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WeatherDaylight {
    pub updated: bool,
    pub held: Option<[i64; 3]>,
    pub held_feather: Option<[i64; 2]>,
}
value_struct!(WeatherDaylight { updated: bool, held: Option<[i64; 3]>, held_feather: Option<[i64; 2]> });
impl OpaqueValue for WeatherDaylight {}

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WeatherProvinceRow {
    pub id: u32,
    pub snow: i64,
    pub permanent_snow: i64,
    pub display_snow: u8,
    pub display_mud: u8,
    pub raw_mud: Recorded<u8>,
    pub temperature: ProvinceTemperature,
    pub clock: DayNightClock,
    pub display: Recorded<Option<WeatherDisplay>>,
    pub temperature_pass: Recorded<Option<TemperaturePass>>,
    pub modifiers: Recorded<Option<ProvinceWeatherModifiers>>,
    pub daylight: Recorded<Option<WeatherDaylight>>,
    pub static_modifiers: Recorded<Option<ProvinceStaticModifiers>>,
}
typed_codec!(WeatherProvinceRow, 32, |row| row.id as u128, |key| Self { id: key as u32, ..Default::default() },
{ 0 => (snow): I64, 1 => (permanent_snow): I64, 2 => (display_snow): U8, 3 => (display_mud): U8, 4 => (temperature.temperature): I64, 5 => (raw_mud): U8, 6 => (clock.local_hour): I32, 7 => (clock.night_factor): I64, 8 => (display): Opaque, 9 => (clock.source): U8, 10 => (temperature.drift): I64, 11 => (temperature.staged): I64, 12 => (temperature.working): I64, 13 => (temperature_pass): Opaque, 14 => (modifiers): Opaque, 15 => (daylight): Opaque, 16 => (static_modifiers): Opaque },
|row| {
   row.clock.canonical().map_err(invalid)?;
   row.temperature.record().map(|_| ()).map_err(invalid)?;
   match &row.modifiers {
       Recorded::Value(Some(modifiers)) => modifiers.canonical().map_err(invalid),
       _ => Ok(()),
   }
});

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceModifierEntry {
    pub key: u32,
    pub value: i64,
}
value_struct!(ProvinceModifierEntry {
    key: u32,
    value: i64
});

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceWeatherModifiers {
    pub entries: Vec<ProvinceModifierEntry>,
}
value_struct!(ProvinceWeatherModifiers { entries: Vec<ProvinceModifierEntry> });

impl ProvinceWeatherModifiers {
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

    pub fn get(&self, key: u32) -> Option<i64> {
        self.entries
            .binary_search_by_key(&key, |entry| entry.key)
            .ok()
            .map(|at| self.entries[at].value)
    }
}
impl OpaqueValue for ProvinceWeatherModifiers {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceStaticModifier {
    pub name: Vec<u8>,
    pub days: i32,
}
value_struct!(ProvinceStaticModifier {
    name: Vec<u8>,
    days: i32
});

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceStaticModifiers {
    pub entries: Vec<ProvinceStaticModifier>,
}
value_struct!(ProvinceStaticModifiers { entries: Vec<ProvinceStaticModifier> });
impl OpaqueValue for ProvinceStaticModifiers {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProvinceTemperature {
    pub temperature: Recorded<i64>,
    pub drift: Recorded<i64>,
    pub staged: Recorded<i64>,
    pub working: Recorded<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemperatureRecord {
    Anchored(crate::temperature::State),
    AtHour(i64),
    NotRecorded,
}

impl ProvinceTemperature {
    pub const fn anchored(state: crate::temperature::State) -> Self {
        Self {
            temperature: Recorded::Value(state.temperature),
            drift: Recorded::Value(state.drift),
            staged: Recorded::Value(state.staged.wrapping_sub(state.temperature)),
            working: Recorded::Value(state.working.wrapping_sub(state.drift)),
        }
    }

    pub const fn historical(temperature: i64) -> Self {
        Self {
            temperature: Recorded::Value(temperature),
            drift: Recorded::NotRecorded,
            staged: Recorded::NotRecorded,
            working: Recorded::NotRecorded,
        }
    }

    pub const NOT_RECORDED: Self = Self {
        temperature: Recorded::NotRecorded,
        drift: Recorded::NotRecorded,
        staged: Recorded::NotRecorded,
        working: Recorded::NotRecorded,
    };

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemperaturePass {
    pub anchor: i32,
    pub cursor: i32,
    pub anchor_seed: Option<u32>,
    pub hours: Vec<Option<u32>>,
    pub witness: TemperatureWitness,
}
value_struct!(TemperaturePass { anchor: i32, cursor: i32, anchor_seed: Option<u32>,
    hours: Vec<Option<u32>>, witness: TemperatureWitness });
impl OpaqueValue for TemperaturePass {}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TemperatureWitness(Option<u64>);

impl TemperatureWitness {
    pub const fn new(witness: u64) -> Self {
        Self(Some(witness))
    }

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DayNightClock {
    pub local_hour: Recorded<i32>,
    pub night_factor: Recorded<i64>,
    pub source: Recorded<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockState {
    Updated,
    Held(crate::day_night::Clock),
    NotRecorded,
}

impl DayNightClock {
    const UPDATED_SOURCE: u8 = 0;
    const HELD_SOURCE: u8 = 1;

    pub const UPDATED: Self = Self {
        local_hour: Recorded::Value(0),
        night_factor: Recorded::Value(0),
        source: Recorded::Value(Self::UPDATED_SOURCE),
    };

    pub const fn held(clock: crate::day_night::Clock) -> Self {
        Self {
            local_hour: Recorded::Value(clock.local_hour),
            night_factor: Recorded::Value(clock.night_factor),
            source: Recorded::Value(Self::HELD_SOURCE),
        }
    }

    pub const fn historical(clock: crate::day_night::Clock) -> Self {
        Self {
            local_hour: Recorded::Value(clock.local_hour),
            night_factor: Recorded::Value(clock.night_factor),
            source: Recorded::NotRecorded,
        }
    }

    pub const NOT_RECORDED: Self = Self {
        local_hour: Recorded::NotRecorded,
        night_factor: Recorded::NotRecorded,
        source: Recorded::NotRecorded,
    };

    pub fn state(&self, province: u32) -> Result<ClockState, &'static str> {
        match self.canonical()? {
            ClockState::Updated if province == 0 => {
                Err("WeatherProvinces slot 0 records an update no native writer makes")
            }
            state => Ok(state),
        }
    }

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


#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

impl OpaqueValue for SunkShipEntry {}
