use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirWingRow {
    pub id: u64,
    pub owner: u32,
    pub air_base: u64,
    pub base_override: u64,
    pub country_override: i32,
    pub planes: i32,
    pub acquired_manpower: Recorded<Option<i32>>,
    pub exile_country: Recorded<Option<i32>>,
    pub mission_mask: u32,
    pub mission_region: Option<u32>,
    pub base_range_raw: i64,
    pub ace: u64,
    pub mission_active: u8,
    pub eligibility_veto: u8,
    pub eligibility_counter: i32,
    pub eligibility_limit: i32,
    pub attached_orders_group: Option<u64>,
    pub pool_ordinal: u32,
    pub name: Recorded<Option<Vec<u8>>>,
    pub pool_subunit: Recorded<Option<u32>>,
    pub raid_reference: Recorded<Option<AirWingRaidReference>>,
    pub strategic_bombing_raw: Recorded<Option<i64>>,
    pub replacement_setting: Recorded<Option<u8>>,
    pub map_flags: Recorded<Option<u32>>,
    pub fuel_base_raw: Recorded<Option<i64>>,
    pub fuel_current_type: Recorded<Option<i32>>,
    pub fuel_effective_aircraft: Recorded<Option<i32>>,
    pub fuel_context_mode: Recorded<Option<u32>>,
    pub fuel_capacity_class: Recorded<Option<i32>>,
    pub fuel_override_resolved: Recorded<Option<u32>>,
}

impl AirWingRow {
    pub fn fuel_inputs(&self) -> Result<AirWingFuelInputs, String> {
        Ok(AirWingFuelInputs {
            base_fuel: *self.fuel_base_raw.require("AirWings.fuel_base_raw")?,
            current_type: *self
                .fuel_current_type
                .require("AirWings.fuel_current_type")?,
            effective_aircraft: *self
                .fuel_effective_aircraft
                .require("AirWings.fuel_effective_aircraft")?,
            context_mode: self
                .fuel_context_mode
                .require("AirWings.fuel_context_mode")?
                .map(u8::try_from)
                .transpose()
                .map_err(|_| "Air fuel context is outside its stored byte".to_owned())?,
            capacity_class: *self
                .fuel_capacity_class
                .require("AirWings.fuel_capacity_class")?,
            override_resolved: match self
                .fuel_override_resolved
                .require("AirWings.fuel_override_resolved")?
            {
                Some(0) => Some(false),
                Some(1) => Some(true),
                None => None,
                _ => {
                    return Err(
                        "Air fuel override liveness is outside its native boolean domain".into(),
                    );
                }
            },
        })
    }
    pub fn training_stop(&self) -> Option<bool> {
        self.map_flag(0)
    }
    pub fn attachment_coverage(&self) -> Option<bool> {
        self.map_flag(2)
    }
    pub fn assigned_to_raid(&self) -> Option<bool> {
        self.map_flag(4)
    }
    fn map_flag(&self, bit: u32) -> Option<bool> {
        match self.map_flags {
            Recorded::Value(Some(flags)) if flags & (1 << bit) != 0 => {
                Some(flags & (2 << bit) != 0)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct AirWingRaidReference {
    pub raw: u64,
    pub resolves: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AirWingFuelInputs {
    pub base_fuel: Option<i64>,
    pub current_type: Option<i32>,
    pub effective_aircraft: Option<i32>,
    pub context_mode: Option<u8>,
    pub capacity_class: Option<i32>,
    pub override_resolved: Option<bool>,
}

row_codec!(AirWingRow, 64, |row| row.id as u128, |key| { id: key as u64 }, {0 => owner: U32, 1 => air_base: U64, 2 => base_override: U64, 3 => country_override: I32, 4 => planes: I32, 5 => mission_mask: U32, 6 => mission_region: OptionalU32, 7 => base_range_raw: I64, 8 => ace: U64, 9 => mission_active: U8, 10 => eligibility_veto: U8, 11 => eligibility_counter: I32, 12 => eligibility_limit: I32, 13 => attached_orders_group: OptionalU64, 14 => pool_ordinal: U32, 15 => name: Opaque, 16 => pool_subunit: OptionalU32, 17 => raid_reference: Opaque, 18 => strategic_bombing_raw: OptionalI64, 19 => acquired_manpower: OptionalI32, 20 => exile_country: OptionalI32, 21 => replacement_setting: Opaque, 22 => map_flags: OptionalU32, 23 => fuel_base_raw: OptionalI64, 24 => fuel_current_type: OptionalI32, 25 => fuel_effective_aircraft: OptionalI32, 26 => fuel_context_mode: OptionalU32, 27 => fuel_capacity_class: OptionalI32, 28 => fuel_override_resolved: OptionalU32}, |_row| Ok(()));
