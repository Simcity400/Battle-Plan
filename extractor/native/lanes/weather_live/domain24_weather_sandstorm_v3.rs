use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 24;
pub const DOMAIN_NAME: &str = "weather_live";

pub const HEADER_LEN: usize = 48;
pub const REGION_ROW_LEN: usize = 32;
pub const PROVINCE_ROW_LEN: usize = 32;

pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const FIXED_SCALE_RAW: u32 = 100_000;

pub const EXPECTED_REGION_ROWS: usize = 305;
pub const MAX_REGION_ID: u32 = 304;
pub const MAX_RAW_ACTIVITY_CANDIDATE: u32 = 16;
pub const MAX_TEMPERATURE_RAW: i32 = 10_000_000;
pub const MAX_TEMP_CHANGE_RAW: i32 = 10_000_000;
pub const MAX_PRECIP_RAW: i32 = 100_000_000;
pub const MIN_OBSERVED_PROVINCE_ROWS: usize = 10_000;
pub const MAX_OBSERVED_PROVINCE_ROWS: usize = 65_536;
pub const MAX_CUSTOM_MODIFIER_COUNT: u32 = 4_096;
pub const MAX_ACTIVE_MODIFIER_COUNT: u32 = 4_096;

pub const REGION_FLAG_RAIN_LIGHT: u32 = 1;
pub const REGION_FLAG_RAIN_HEAVY: u32 = 1 << 1;
pub const REGION_FLAG_SNOW: u32 = 1 << 2;
pub const REGION_FLAG_BLIZZARD: u32 = 1 << 3;
pub const REGION_FLAG_ARCTIC_WATER: u32 = 1 << 4;
pub const REGION_FLAG_ACTIVE_SIGNAL: u32 = 1 << 5;
pub const REGION_FLAG_PHENOMENON_ACTIVE: u32 = 1 << 6;
pub const REGION_FLAG_SANDSTORM: u32 = 1 << 7;
pub const REGION_KNOWN_FLAGS: u32 = REGION_FLAG_RAIN_LIGHT
    | REGION_FLAG_RAIN_HEAVY
    | REGION_FLAG_SNOW
    | REGION_FLAG_BLIZZARD
    | REGION_FLAG_ARCTIC_WATER
    | REGION_FLAG_ACTIVE_SIGNAL
    | REGION_FLAG_PHENOMENON_ACTIVE
    | REGION_FLAG_SANDSTORM;

pub const PROVINCE_FLAG_TEMP_PRESENT: u32 = 1;
pub const PROVINCE_FLAG_TEMP_CHANGE_PRESENT: u32 = 1 << 1;
pub const PROVINCE_FLAG_WATER_PRESENT: u32 = 1 << 2;
pub const PROVINCE_FLAG_SNOW_PRESENT: u32 = 1 << 3;
pub const PROVINCE_FLAG_MUD_PRESENT: u32 = 1 << 4;
pub const PROVINCE_FLAG_MUD_ACTIVE: u32 = 1 << 5;
pub const PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT: u32 = 1 << 6;
pub const PROVINCE_FLAG_STRUCT_VALIDATED: u32 = 1 << 7;
pub const PROVINCE_FLAG_MAP_RECOLOR_DRIVER: u32 = 1 << 8;
pub const PROVINCE_FLAG_DYNAMIC_LUT_DRIVER: u32 = 1 << 9;
pub const PROVINCE_KNOWN_FLAGS: u32 = PROVINCE_FLAG_TEMP_PRESENT
    | PROVINCE_FLAG_TEMP_CHANGE_PRESENT
    | PROVINCE_FLAG_WATER_PRESENT
    | PROVINCE_FLAG_SNOW_PRESENT
    | PROVINCE_FLAG_MUD_PRESENT
    | PROVINCE_FLAG_MUD_ACTIVE
    | PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT
    | PROVINCE_FLAG_STRUCT_VALIDATED
    | PROVINCE_FLAG_MAP_RECOLOR_DRIVER
    | PROVINCE_FLAG_DYNAMIC_LUT_DRIVER;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeatherRegionRow {
    pub region_id: u32,
    pub flags: u32,
    pub temperature_raw: i32,
    pub phenomenon: u32,
    pub next_weather_change_raw: u32,
    pub active_modifier_count: u32,
    pub intensity: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeatherProvinceVisualRow {
    pub province_id: u32,
    pub flags: u32,
    pub temperature_raw: i32,
    pub temperature_change_raw: i32,
    pub water_raw: i32,
    pub snow_raw: i32,
    pub custom_modifier_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeatherVisualHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub observed_region_rows: u32,
    pub observed_province_rows: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WeatherSandstormGateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

pub fn province_visual_flags(
    temperature_present: bool,
    temp_change_present: bool,
    water_present: bool,
    snow_present: bool,
    mud_present: bool,
    mud_active: bool,
    custom_modifier_count_present: bool,
) -> u32 {
    let mut flags = PROVINCE_FLAG_STRUCT_VALIDATED;
    if temperature_present {
        flags |= PROVINCE_FLAG_TEMP_PRESENT;
    }
    if temp_change_present {
        flags |= PROVINCE_FLAG_TEMP_CHANGE_PRESENT;
    }
    if water_present {
        flags |= PROVINCE_FLAG_WATER_PRESENT;
    }
    if snow_present {
        flags |= PROVINCE_FLAG_SNOW_PRESENT;
    }
    if mud_present {
        flags |= PROVINCE_FLAG_MUD_PRESENT;
    }
    if mud_active {
        flags |= PROVINCE_FLAG_MUD_ACTIVE;
    }
    if custom_modifier_count_present {
        flags |= PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT;
    }
    if temperature_present || water_present || snow_present {
        flags |= PROVINCE_FLAG_MAP_RECOLOR_DRIVER;
    }
    if snow_present || mud_present {
        flags |= PROVINCE_FLAG_DYNAMIC_LUT_DRIVER;
    }
    flags
}

pub fn encode_payload(
    header: &WeatherVisualHeader,
    region_rows: &[WeatherRegionRow],
    province_rows: &[WeatherProvinceVisualRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        HEADER_LEN
            .saturating_add(region_rows.len().saturating_mul(REGION_ROW_LEN))
            .saturating_add(province_rows.len().saturating_mul(PROVINCE_ROW_LEN)),
    );
    let active_region_count = region_rows
        .iter()
        .filter(|row| row.phenomenon >= 1)
        .count()
        .min(u32::MAX as usize) as u32;
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(region_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&(province_rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.observed_region_rows.to_le_bytes());
    payload.extend_from_slice(&header.observed_province_rows.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&active_region_count.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in region_rows {
        payload.extend_from_slice(&row.region_id.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.temperature_raw.to_le_bytes());
        payload.extend_from_slice(&row.phenomenon.to_le_bytes());
        payload.extend_from_slice(&row.next_weather_change_raw.to_le_bytes());
        payload.extend_from_slice(&row.active_modifier_count.to_le_bytes());
        payload.extend_from_slice(&row.intensity.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    for row in province_rows {
        payload.extend_from_slice(&row.province_id.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.temperature_raw.to_le_bytes());
        payload.extend_from_slice(&row.temperature_change_raw.to_le_bytes());
        payload.extend_from_slice(&row.water_raw.to_le_bytes());
        payload.extend_from_slice(&row.snow_raw.to_le_bytes());
        payload.extend_from_slice(&row.custom_modifier_count.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> WeatherSandstormGateSummary {
    let mut summary = WeatherSandstormGateSummary::default();
    if payload.len() < HEADER_LEN {
        summary
            .parse_errors
            .push(format!("offset {offset}: payload shorter than header"));
        return summary;
    }
    let region_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let province_count = read_u32_le(payload, 20) as usize;
    let observed_region_rows = read_u32_le(payload, 24) as usize;
    let observed_province_rows = read_u32_le(payload, 28) as usize;
    let scale_raw = read_u32_le(payload, 32);
    let active_region_count = read_u32_le(payload, 36) as usize;
    let reserved0 = read_u32_le(payload, 40);
    let reserved1 = read_u32_le(payload, 44);
    let delta_data = (status_flags & STATUS_DELTA_DATA) != 0;
    let emitted_rows = region_count.saturating_add(province_count);

    summary.status_flags_or |= status_flags;
    summary.data_rows_total = emitted_rows as u64;
    summary.data_changed_rows_total = changed_rows as u64;

    let expected_len = HEADER_LEN
        .saturating_add(region_count.saturating_mul(REGION_ROW_LEN))
        .saturating_add(province_count.saturating_mul(PROVINCE_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return summary;
    }
    if observed_region_rows != EXPECTED_REGION_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: observed_region_rows {observed_region_rows} != {EXPECTED_REGION_ROWS}"
        ));
    }
    if observed_province_rows != 0
        && !(MIN_OBSERVED_PROVINCE_ROWS..=MAX_OBSERVED_PROVINCE_ROWS)
            .contains(&observed_province_rows)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: observed_province_rows {observed_province_rows} outside bounds"
        ));
    }
    if !delta_data && region_count != EXPECTED_REGION_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: full snapshot region_count {region_count} != {EXPECTED_REGION_ROWS}"
        ));
    }
    if changed_rows > emitted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: changed_rows {changed_rows} > emitted_rows {emitted_rows}"
        ));
    }
    if delta_data && changed_rows != emitted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: delta changed_rows {changed_rows} != emitted_rows {emitted_rows}"
        ));
    }
    if scale_raw != FIXED_SCALE_RAW {
        summary
            .parse_errors
            .push(format!("offset {offset}: scale_raw {scale_raw} != {FIXED_SCALE_RAW}"));
    }
    if active_region_count > region_count {
        summary.parse_errors.push(format!(
            "offset {offset}: active_region_count {active_region_count} > region_count {region_count}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: reserved header fields are nonzero ({reserved0}, {reserved1})"
        ));
    }

    add_unit(&mut summary, "weather_region_rows", region_count);
    add_unit(&mut summary, "weather_observed_region_rows", observed_region_rows);
    add_unit(&mut summary, "weather_province_visual_rows", province_count);
    add_unit(
        &mut summary,
        "weather_observed_province_rows",
        observed_province_rows,
    );
    add_unit(
        &mut summary,
        "weather_active_region_rows",
        active_region_count,
    );

    let mut seen_regions = BTreeSet::new();
    let mut computed_active_regions = 0usize;
    let mut cursor = HEADER_LEN;
    for row_index in 0..region_count {
        let region_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let temperature_raw = read_i32_le(payload, cursor + 8);
        let phenomenon = read_u32_le(payload, cursor + 12);
        let next_weather_change_raw = read_u32_le(payload, cursor + 16);
        let row_reserved = read_u32_le(payload, cursor + 28);
        cursor += REGION_ROW_LEN;
        validate_region_row(
            offset,
            row_index,
            region_id,
            flags,
            temperature_raw,
            phenomenon,
            next_weather_change_raw,
            row_reserved,
            &mut seen_regions,
            &mut computed_active_regions,
            &mut summary,
        );
    }
    if computed_active_regions != active_region_count {
        summary.parse_errors.push(format!(
            "offset {offset}: active_region_count {active_region_count} != computed {computed_active_regions}"
        ));
    }

    let mut seen_provinces = BTreeSet::new();
    for row_index in 0..province_count {
        let province_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let temperature_raw = read_i32_le(payload, cursor + 8);
        let temperature_change_raw = read_i32_le(payload, cursor + 12);
        let water_raw = read_i32_le(payload, cursor + 16);
        let snow_raw = read_i32_le(payload, cursor + 20);
        let custom_modifier_count = read_u32_le(payload, cursor + 24);
        let row_reserved = read_u32_le(payload, cursor + 28);
        cursor += PROVINCE_ROW_LEN;
        validate_province_row(
            offset,
            row_index,
            province_id,
            flags,
            temperature_raw,
            temperature_change_raw,
            water_raw,
            snow_raw,
            custom_modifier_count,
            row_reserved,
            observed_province_rows,
            &mut seen_provinces,
            &mut summary,
        );
    }
    summary
}

fn validate_region_row(
    offset: usize,
    row_index: usize,
    region_id: u32,
    flags: u32,
    temperature_raw: i32,
    phenomenon: u32,
    next_weather_change_raw: u32,
    row_reserved: u32,
    seen_regions: &mut BTreeSet<u32>,
    computed_active_regions: &mut usize,
    summary: &mut WeatherSandstormGateSummary,
) {
    if region_id > MAX_REGION_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} has invalid region_id {region_id}"
        ));
    } else if !seen_regions.insert(region_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} duplicates region_id {region_id}"
        ));
    }
    if flags & !REGION_KNOWN_FLAGS != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} has unknown flags {flags}"
        ));
    }
    let named_weather_count = [
        REGION_FLAG_RAIN_LIGHT,
        REGION_FLAG_RAIN_HEAVY,
        REGION_FLAG_SNOW,
        REGION_FLAG_BLIZZARD,
        REGION_FLAG_SANDSTORM,
    ]
    .into_iter()
    .filter(|flag| flags & *flag != 0)
    .count();
    if named_weather_count > 1 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} overlaps mutually-exclusive named weather flags"
        ));
    }
    let active_flag = (flags & REGION_FLAG_ACTIVE_SIGNAL) != 0;
    let phenomenon_flag = (flags & REGION_FLAG_PHENOMENON_ACTIVE) != 0;
    if active_flag != phenomenon_flag {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} has mismatched active flags"
        ));
    }
    if phenomenon >= 1 {
        *computed_active_regions = computed_active_regions.saturating_add(1);
        if !active_flag {
            summary.parse_errors.push(format!(
                "offset {offset}: weather region row {row_index} has nonzero raw +0x78 activity candidate without active flag"
            ));
        }
    } else if active_flag {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} has active flag without raw +0x78 activity candidate"
        ));
    }
    if phenomenon > MAX_RAW_ACTIVITY_CANDIDATE {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} has invalid raw +0x78 activity candidate {phenomenon}"
        ));
    }
    if next_weather_change_raw > i32::MAX as u32 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} has invalid signed transition expiry {next_weather_change_raw}"
        ));
    }
    if !(-MAX_TEMPERATURE_RAW..=MAX_TEMPERATURE_RAW).contains(&temperature_raw) {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} has invalid temperature_raw {temperature_raw}"
        ));
    }
    if flags & REGION_FLAG_SANDSTORM != 0 {
        add_unit(summary, "weather_region_sandstorm_rows", 1);
    }
    if row_reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region row {row_index} reserved field is {row_reserved}"
        ));
    }
}

fn validate_province_row(
    offset: usize,
    row_index: usize,
    province_id: u32,
    flags: u32,
    temperature_raw: i32,
    temperature_change_raw: i32,
    water_raw: i32,
    snow_raw: i32,
    custom_modifier_count: u32,
    row_reserved: u32,
    observed_province_rows: usize,
    seen_provinces: &mut BTreeSet<u32>,
    summary: &mut WeatherSandstormGateSummary,
) {
    if observed_province_rows != 0 && province_id as usize >= observed_province_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has invalid province_id {province_id}"
        ));
    } else if !seen_provinces.insert(province_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} duplicates province_id {province_id}"
        ));
    }
    if flags & !PROVINCE_KNOWN_FLAGS != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & PROVINCE_FLAG_STRUCT_VALIDATED == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} missing struct validation flag"
        ));
    }
    let temp_present = flags & PROVINCE_FLAG_TEMP_PRESENT != 0;
    let temp_change_present = flags & PROVINCE_FLAG_TEMP_CHANGE_PRESENT != 0;
    let water_present = flags & PROVINCE_FLAG_WATER_PRESENT != 0;
    let snow_present = flags & PROVINCE_FLAG_SNOW_PRESENT != 0;
    let mud_present = flags & PROVINCE_FLAG_MUD_PRESENT != 0;
    let mud_active = flags & PROVINCE_FLAG_MUD_ACTIVE != 0;
    let custom_present = flags & PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT != 0;
    let map_recolor_driver = flags & PROVINCE_FLAG_MAP_RECOLOR_DRIVER != 0;
    let dynamic_lut_driver = flags & PROVINCE_FLAG_DYNAMIC_LUT_DRIVER != 0;

    if mud_active && !mud_present {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has mud_active without mud_present"
        ));
    }
    if map_recolor_driver != (temp_present || water_present || snow_present) {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has inconsistent map recolor flag"
        ));
    }
    if dynamic_lut_driver != (snow_present || mud_present) {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has inconsistent dynamic LUT flag"
        ));
    }
    if temp_present {
        add_unit(summary, "weather_province_temp_rows", 1);
        if !(-MAX_TEMPERATURE_RAW..=MAX_TEMPERATURE_RAW).contains(&temperature_raw) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has invalid temperature_raw {temperature_raw}"
            ));
        }
    } else if temperature_raw != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has temperature_raw without presence flag"
        ));
    }
    if temp_change_present {
        add_unit(summary, "weather_province_temp_change_rows", 1);
        if !(-MAX_TEMP_CHANGE_RAW..=MAX_TEMP_CHANGE_RAW).contains(&temperature_change_raw) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has invalid temperature_change_raw {temperature_change_raw}"
            ));
        }
    } else if temperature_change_raw != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has temperature_change_raw without presence flag"
        ));
    }
    if water_present {
        add_unit(summary, "weather_province_water_rows", 1);
        if !(0..=MAX_PRECIP_RAW).contains(&water_raw) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has invalid water_raw {water_raw}"
            ));
        }
    } else if water_raw != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has water_raw without presence flag"
        ));
    }
    if snow_present {
        add_unit(summary, "weather_province_snow_rows", 1);
        if !(0..=MAX_PRECIP_RAW).contains(&snow_raw) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has invalid snow_raw {snow_raw}"
            ));
        }
    } else if snow_raw != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has snow_raw without presence flag"
        ));
    }
    if mud_present {
        add_unit(summary, "weather_province_mud_measured_rows", 1);
    }
    if mud_active {
        add_unit(summary, "weather_province_mud_active_rows", 1);
    }
    if custom_present {
        add_unit(summary, "weather_province_custom_modifier_rows", 1);
        if custom_modifier_count > MAX_CUSTOM_MODIFIER_COUNT {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has invalid custom_modifier_count {custom_modifier_count}"
            ));
        }
    } else if custom_modifier_count != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} has custom_modifier_count without presence flag"
        ));
    }
    if map_recolor_driver {
        add_unit(summary, "weather_province_map_recolor_driver_rows", 1);
    }
    if dynamic_lut_driver {
        add_unit(summary, "weather_province_dynamic_lut_driver_rows", 1);
    }
    if row_reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province row {row_index} reserved field is {row_reserved}"
        ));
    }
}

fn add_unit(summary: &mut WeatherSandstormGateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&payload[offset..offset + 4]);
    u32::from_le_bytes(bytes)
}

fn read_i32_le(payload: &[u8], offset: usize) -> i32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&payload[offset..offset + 4]);
    i32::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_region_sandstorm_flag_without_unknown_flag_error() {
        let header = WeatherVisualHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 2,
            observed_region_rows: EXPECTED_REGION_ROWS as u32,
            observed_province_rows: 13_414,
        };
        let payload = encode_payload(
            &header,
            &[WeatherRegionRow {
                region_id: 42,
                flags: REGION_FLAG_SANDSTORM
                    | REGION_FLAG_ACTIVE_SIGNAL
                    | REGION_FLAG_PHENOMENON_ACTIVE,
                temperature_raw: 0,
                phenomenon: 1,
                next_weather_change_raw: 0,
                active_modifier_count: 0,
                intensity: 2,
            }],
            &[WeatherProvinceVisualRow {
                province_id: 1,
                flags: province_visual_flags(true, true, true, true, true, false, true),
                temperature_raw: 12,
                temperature_change_raw: 1,
                water_raw: 0,
                snow_raw: 0,
                custom_modifier_count: 0,
            }],
        );
        let summary = decode_payload(0, &payload);
        assert_eq!(summary.parse_errors, Vec::<String>::new());
        assert_eq!(
            summary
                .data_units_total
                .get("weather_region_sandstorm_rows"),
            Some(&1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("weather_province_visual_rows"),
            Some(&1)
        );
    }

    #[test]
    fn still_rejects_unknown_region_flags() {
        let header = WeatherVisualHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_region_rows: EXPECTED_REGION_ROWS as u32,
            observed_province_rows: 13_414,
        };
        let payload = encode_payload(
            &header,
            &[WeatherRegionRow {
                region_id: 1,
                flags: 1 << 31,
                temperature_raw: 0,
                phenomenon: 0,
                next_weather_change_raw: 0,
                active_modifier_count: 0,
                intensity: 0,
            }],
            &[],
        );
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("unknown flags")));
    }
}
