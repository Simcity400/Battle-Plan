//! offsets and both row layouts are generated from the same tables the readers

use alloc::vec::Vec;

use crate::{HeaderWord, ProvinceField, RegionField, HEADER_LEN, PROVINCE_ROW_LEN, REGION_ROW_LEN};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeaderWords {
    pub region_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub province_count: u32,
    pub observed_region_rows: u32,
    pub observed_province_rows: u32,
    pub scale_raw: u32,
    pub active_region_count: u32,
    pub max_water_raw: u32,
    pub max_snow_raw: u32,
}

impl HeaderWords {
    pub const fn word(&self, which: HeaderWord) -> u32 {
        match which {
            HeaderWord::RegionCount => self.region_count,
            HeaderWord::ChangedRows => self.changed_rows,
            HeaderWord::StatusFlags => self.status_flags,
            HeaderWord::ProvinceCount => self.province_count,
            HeaderWord::ObservedRegionRows => self.observed_region_rows,
            HeaderWord::ObservedProvinceRows => self.observed_province_rows,
            HeaderWord::ScaleRaw => self.scale_raw,
            HeaderWord::ActiveRegionCount => self.active_region_count,
            HeaderWord::MaxWaterRaw => self.max_water_raw,
            HeaderWord::MaxSnowRaw => self.max_snow_raw,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionRow {
    pub region_id: u32,
    pub flags: u32,
    pub temperature_raw: i32,
    pub phenomenon: u32,
    pub next_weather_change_raw: u32,
    pub active_modifier_count: u32,
    pub intensity: u32,
}

impl RegionRow {
    const fn word(&self, field: RegionField) -> u32 {
        match field {
            RegionField::RegionId => self.region_id,
            RegionField::Flags => self.flags,
            RegionField::TemperatureRaw => self.temperature_raw as u32,
            RegionField::Phenomenon => self.phenomenon,
            RegionField::NextWeatherChangeRaw => self.next_weather_change_raw,
            RegionField::ActiveModifierCount => self.active_modifier_count,
            RegionField::Intensity => self.intensity,
            RegionField::Reserved => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProvinceRow {
    pub province_id: u32,
    pub flags: u32,
    pub temperature_raw: i32,
    pub temperature_change_raw: i32,
    pub water_raw: i32,
    pub snow_raw: i32,
    pub custom_modifier_count: u32,
}

impl ProvinceRow {
    const fn word(&self, field: ProvinceField) -> u32 {
        match field {
            ProvinceField::ProvinceId => self.province_id,
            ProvinceField::Flags => self.flags,
            ProvinceField::TemperatureRaw => self.temperature_raw as u32,
            ProvinceField::TemperatureChangeRaw => self.temperature_change_raw as u32,
            ProvinceField::WaterRaw => self.water_raw as u32,
            ProvinceField::SnowRaw => self.snow_raw as u32,
            ProvinceField::CustomModifierCount => self.custom_modifier_count,
            ProvinceField::Reserved => 0,
        }
    }
}

fn push_header(payload: &mut Vec<u8>, words: &HeaderWords, elapsed_wall_ms: u64) {
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    // `+8` onwards, in offset order, generated from the table.
    let mut written = 8usize;
    for which in HeaderWord::ALL {
        debug_assert_eq!(
            which.offset(),
            written,
            "HeaderWord::ALL is not in offset order"
        );
        payload.extend_from_slice(&words.word(which).to_le_bytes());
        written += 4;
    }
    debug_assert_eq!(payload.len(), HEADER_LEN);
}

pub fn encode_payload(
    words: &HeaderWords,
    elapsed_wall_ms: u64,
    regions: impl ExactSizeIterator<Item = RegionRow>,
    provinces: impl ExactSizeIterator<Item = ProvinceRow>,
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        HEADER_LEN + regions.len() * REGION_ROW_LEN + provinces.len() * PROVINCE_ROW_LEN,
    );
    push_header(&mut payload, words, elapsed_wall_ms);
    for row in regions {
        for field in RegionField::ALL {
            payload.extend_from_slice(&row.word(field).to_le_bytes());
        }
    }
    for row in provinces {
        for field in ProvinceField::ALL {
            payload.extend_from_slice(&row.word(field).to_le_bytes());
        }
    }
    payload
}
