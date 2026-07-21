//! Offline A20 gate for the retained domain-24 weather stream.
//!
//! This binary is owned by the `weather_live_lane` package. It
//! opens either raw or Zstandard-stored H4CR through the production compact
//! stream codec and streams the decoded bytes, so multi-gigabyte captures do
//! not need to be mapped or buffered in memory.

use compact_contract::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::io::{self, Read};
use std::path::Path;

const DOMAIN_WEATHER: u16 = 24;
const RECORD_TYPE_DATA: u16 = 3;
const HEADER_LEN: usize = 48;
const REGION_ROW_LEN: usize = 32;
const PROVINCE_ROW_LEN: usize = 32;
const REGION_COUNT: usize = 305;
const KNOWN_REGION_FLAGS: u32 = 0xff;
const KNOWN_PROVINCE_FLAGS: u32 = 0x3ff;
const RAIN_LIGHT: u32 = 1;
const RAIN_HEAVY: u32 = 1 << 1;
const SNOW: u32 = 1 << 2;
const BLIZZARD: u32 = 1 << 3;
const ARCTIC_WATER: u32 = 1 << 4;
const ACTIVE_SIGNAL: u32 = 1 << 5;
const PHENOMENON_ACTIVE: u32 = 1 << 6;
const SANDSTORM: u32 = 1 << 7;
const NAMED_AIRBORNE_FLAGS: u32 = RAIN_LIGHT | RAIN_HEAVY | SNOW | BLIZZARD | SANDSTORM;
const PROVINCE_TEMP_PRESENT: u32 = 1;
const PROVINCE_TEMP_CHANGE_PRESENT: u32 = 1 << 1;
const PROVINCE_WATER_PRESENT: u32 = 1 << 2;
const PROVINCE_SNOW_PRESENT: u32 = 1 << 3;
const PROVINCE_MUD_PRESENT: u32 = 1 << 4;
const PROVINCE_MUD_ACTIVE: u32 = 1 << 5;
const PROVINCE_CUSTOM_MODIFIER_PRESENT: u32 = 1 << 6;
const PROVINCE_STRUCT_VALIDATED: u32 = 1 << 7;
const PROVINCE_MAP_RECOLOR_DRIVER: u32 = 1 << 8;
const PROVINCE_DYNAMIC_LUT_DRIVER: u32 = 1 << 9;
// Raw +0x78 is retained by compact-v1 as a bounded activity diagnostic. RE
// proves it is not a stable weather-kind enum; all kind/transition proof below
// is derived from the named flags.
const MAX_RAW_ACTIVITY_CANDIDATE: u32 = 16;
const MAX_TEMPERATURE_RAW: i32 = 10_000_000;
const MAX_TEMP_CHANGE_RAW: i32 = 10_000_000;
const MAX_PRECIP_RAW: i32 = 100_000_000;
const MAX_CUSTOM_MODIFIER_COUNT: u32 = 4_096;
const MAX_ACTIVE_MODIFIER_COUNT: u32 = 4_096;
const FAILURE_MASK: u32 = COMPACT_STATUS_STATE_UNAVAILABLE
    | COMPACT_STATUS_PROFILE_MISMATCH
    | COMPACT_STATUS_NO_CLOCK
    | COMPACT_STATUS_NO_SCALAR_SOURCE
    | COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_CONTROL_INVALID;

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("missing u32 at payload offset {offset}"))?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()))
}

fn i32_at(bytes: &[u8], offset: usize) -> Result<i32, String> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("missing i32 at payload offset {offset}"))?;
    Ok(i32::from_le_bytes(raw.try_into().unwrap()))
}

/// Exact CAirInterface::GetWeatherEffectForAirRegion (0x14166ec50) named-flag
/// priority. Arctic water is an independent non-airborne phenomenon.
fn airborne_code(flags: u32) -> u8 {
    if flags & RAIN_LIGHT != 0 {
        2
    } else if flags & RAIN_HEAVY != 0 {
        3
    } else if flags & SNOW != 0 {
        4
    } else if flags & BLIZZARD != 0 {
        5
    } else if flags & SANDSTORM != 0 {
        7
    } else {
        0
    }
}

#[derive(Debug, Default)]
struct Gate {
    weather_records: u64,
    baselines: u64,
    deltas: u64,
    first_hour: Option<i32>,
    last_hour: Option<i32>,
    previous_hour: Option<i32>,
    consecutive_pairs: u64,
    nonconsecutive_pairs: u64,
    duplicate_hours: u64,
    hours: BTreeSet<i32>,
    observed_province_count: Option<usize>,
    flags_by_region: BTreeMap<u32, u32>,
    expiry_by_region: BTreeMap<u32, i32>,
    provinces_by_id: BTreeMap<u32, [u32; 7]>,
    rain_heavy_transitions: u64,
    weather_kind_transitions: u64,
    expiry_refreshes: u64,
    total_flag_transitions: u64,
    rain_heavy_min: usize,
    rain_heavy_max: usize,
    arctic_water_min: usize,
    arctic_water_max: usize,
}

impl Gate {
    fn new() -> Self {
        Self {
            rain_heavy_min: usize::MAX,
            arctic_water_min: usize::MAX,
            ..Self::default()
        }
    }

    fn consume(&mut self, hour: i32, payload: &[u8]) -> Result<(), String> {
        if payload.len() < HEADER_LEN {
            return Err(format!("weather payload has {} bytes, expected at least {HEADER_LEN}", payload.len()));
        }
        let region_rows = u32_at(payload, 8)? as usize;
        let changed_rows = u32_at(payload, 12)? as usize;
        let status = u32_at(payload, 16)?;
        let province_rows = u32_at(payload, 20)? as usize;
        let observed_regions = u32_at(payload, 24)? as usize;
        let observed_provinces = u32_at(payload, 28)? as usize;
        let scale = u32_at(payload, 32)?;
        let active_regions = u32_at(payload, 36)? as usize;
        let reserved0 = u32_at(payload, 40)?;
        let reserved1 = u32_at(payload, 44)?;
        let delta = status & COMPACT_STATUS_DELTA_DATA != 0;

        if status & !COMPACT_STATUS_V1_KNOWN_MASK != 0 {
            return Err(format!("weather status has unknown bits {status:#x}"));
        }
        if status & FAILURE_MASK != 0 {
            return Err(format!("weather status has failure bits {status:#x}"));
        }
        if observed_regions != REGION_COUNT {
            return Err(format!("observed region count {observed_regions} != {REGION_COUNT}"));
        }
        if let Some(expected) = self.observed_province_count {
            if observed_provinces != expected {
                return Err(format!(
                    "observed province count {observed_provinces} != prior {expected}"
                ));
            }
        } else {
            self.observed_province_count = Some(observed_provinces);
        }
        if scale != 100_000 {
            return Err(format!("weather scale {scale} != 100000"));
        }
        if reserved0 != 0 || reserved1 != 0 {
            return Err(format!("nonzero weather reserved words {reserved0}, {reserved1}"));
        }
        if observed_provinces == 0 && province_rows != 0 {
            return Err("province rows emitted with zero observed province rows".into());
        }
        if province_rows > observed_provinces {
            return Err(format!("province rows {province_rows} > observed {observed_provinces}"));
        }
        let expected = HEADER_LEN
            .checked_add(region_rows.checked_mul(REGION_ROW_LEN).ok_or("region length overflow")?)
            .and_then(|v| v.checked_add(province_rows.checked_mul(PROVINCE_ROW_LEN)?))
            .ok_or("payload length overflow")?;
        if payload.len() != expected {
            return Err(format!("weather payload length {} != {expected}", payload.len()));
        }
        if !delta && region_rows != REGION_COUNT {
            return Err(format!("baseline region count {region_rows} != {REGION_COUNT}"));
        }
        if delta && self.baselines == 0 {
            return Err("weather delta precedes baseline".into());
        }
        if delta && changed_rows != region_rows + province_rows {
            return Err(format!("delta changed rows {changed_rows} != emitted {}", region_rows + province_rows));
        }
        if active_regions > region_rows {
            return Err(format!("active regions {active_regions} > emitted {region_rows}"));
        }

        let mut seen = BTreeSet::new();
        let mut computed_active = 0usize;
        for index in 0..region_rows {
            let base = HEADER_LEN + index * REGION_ROW_LEN;
            let id = u32_at(payload, base)?;
            let flags = u32_at(payload, base + 4)?;
            let temperature_raw = i32_at(payload, base + 8)?;
            let raw_activity_candidate = u32_at(payload, base + 12)?;
            let transition_expiry = i32_at(payload, base + 16)?;
            let active_modifier_count = u32_at(payload, base + 20)?;
            let intensity = u32_at(payload, base + 24)?;
            let reserved = u32_at(payload, base + 28)?;
            if id > 304 || !seen.insert(id) {
                return Err(format!("invalid or duplicate region id {id}"));
            }
            if flags & !KNOWN_REGION_FLAGS != 0 {
                return Err(format!("region {id} has unknown flags {flags:#x}"));
            }
            let active_signal = flags & ACTIVE_SIGNAL != 0;
            let phenomenon_active = flags & PHENOMENON_ACTIVE != 0;
            if active_signal != phenomenon_active {
                return Err(format!("region {id} has mismatched active flags {flags:#x}"));
            }
            if raw_activity_candidate > MAX_RAW_ACTIVITY_CANDIDATE {
                return Err(format!(
                    "region {id} has invalid raw activity candidate {raw_activity_candidate}"
                ));
            }
            if (raw_activity_candidate != 0) != active_signal {
                return Err(format!(
                    "region {id} raw activity candidate {raw_activity_candidate} disagrees with active flags {flags:#x}"
                ));
            }
            if (flags & NAMED_AIRBORNE_FLAGS).count_ones() > 1 {
                return Err(format!(
                    "region {id} overlaps mutually-exclusive named airborne flags {flags:#x}"
                ));
            }
            if transition_expiry < 0 {
                return Err(format!(
                    "region {id} has negative absolute transition expiry {transition_expiry}"
                ));
            }
            if !(-MAX_TEMPERATURE_RAW..=MAX_TEMPERATURE_RAW).contains(&temperature_raw) {
                return Err(format!("region {id} has invalid temperature {temperature_raw}"));
            }
            if active_modifier_count > MAX_ACTIVE_MODIFIER_COUNT {
                return Err(format!(
                    "region {id} has invalid active modifier count {active_modifier_count}"
                ));
            }
            if intensity > u8::MAX as u32 {
                return Err(format!("region {id} has invalid intensity {intensity}"));
            }
            if reserved != 0 {
                return Err(format!("region {id} has reserved value {reserved}"));
            }
            if raw_activity_candidate != 0 {
                computed_active += 1;
            }
            if let Some(old) = self.flags_by_region.insert(id, flags) {
                let changed = old ^ flags;
                if changed != 0 {
                    self.total_flag_transitions += changed.count_ones() as u64;
                }
                if changed & RAIN_HEAVY != 0 {
                    self.rain_heavy_transitions += 1;
                }
                if airborne_code(old) != airborne_code(flags) {
                    self.weather_kind_transitions += 1;
                    if self.expiry_by_region.get(&id).copied() == Some(transition_expiry) {
                        return Err(format!(
                            "region {id} changed named weather kind without refreshing expiry"
                        ));
                    }
                }
            }
            if let Some(old_expiry) = self.expiry_by_region.insert(id, transition_expiry) {
                if transition_expiry != old_expiry {
                    if old_expiry > hour {
                        return Err(format!(
                            "region {id} refreshed before prior expiry {old_expiry} at hour {hour}"
                        ));
                    }
                    if transition_expiry <= hour {
                        return Err(format!(
                            "region {id} refreshed to non-future expiry {transition_expiry} at hour {hour}"
                        ));
                    }
                    self.expiry_refreshes += 1;
                }
            }
        }
        if computed_active != active_regions {
            return Err(format!("active region header {active_regions} != computed {computed_active}"));
        }
        if !delta && seen.len() != REGION_COUNT {
            return Err(format!("baseline unique regions {} != {REGION_COUNT}", seen.len()));
        }

        let province_base = HEADER_LEN + region_rows * REGION_ROW_LEN;
        let mut seen_provinces = BTreeSet::new();
        for index in 0..province_rows {
            let base = province_base + index * PROVINCE_ROW_LEN;
            let id = u32_at(payload, base)?;
            let flags = u32_at(payload, base + 4)?;
            let temperature_raw = i32_at(payload, base + 8)?;
            let temperature_change_raw = i32_at(payload, base + 12)?;
            let water_raw = i32_at(payload, base + 16)?;
            let snow_raw = i32_at(payload, base + 20)?;
            let custom_modifier_count = u32_at(payload, base + 24)?;
            let reserved = u32_at(payload, base + 28)?;
            if id as usize >= observed_provinces || !seen_provinces.insert(id) {
                return Err(format!("invalid or duplicate province id {id}"));
            }
            if flags & !KNOWN_PROVINCE_FLAGS != 0 {
                return Err(format!("province {id} has unknown flags {flags:#x}"));
            }
            if flags & PROVINCE_STRUCT_VALIDATED == 0 {
                return Err(format!("province {id} lacks the struct-validated flag"));
            }
            let temp_present = flags & PROVINCE_TEMP_PRESENT != 0;
            let temp_change_present = flags & PROVINCE_TEMP_CHANGE_PRESENT != 0;
            let water_present = flags & PROVINCE_WATER_PRESENT != 0;
            let snow_present = flags & PROVINCE_SNOW_PRESENT != 0;
            let mud_present = flags & PROVINCE_MUD_PRESENT != 0;
            let mud_active = flags & PROVINCE_MUD_ACTIVE != 0;
            let custom_present = flags & PROVINCE_CUSTOM_MODIFIER_PRESENT != 0;
            let map_recolor_driver = flags & PROVINCE_MAP_RECOLOR_DRIVER != 0;
            let dynamic_lut_driver = flags & PROVINCE_DYNAMIC_LUT_DRIVER != 0;
            if mud_active && !mud_present {
                return Err(format!("province {id} has mud-active without mud-present"));
            }
            if map_recolor_driver != (temp_present || water_present || snow_present) {
                return Err(format!("province {id} has inconsistent map-recolor flags"));
            }
            if dynamic_lut_driver != (snow_present || mud_present) {
                return Err(format!("province {id} has inconsistent dynamic-LUT flags"));
            }
            if (temp_present
                && !(-MAX_TEMPERATURE_RAW..=MAX_TEMPERATURE_RAW).contains(&temperature_raw))
                || (!temp_present && temperature_raw != 0)
            {
                return Err(format!("province {id} has invalid temperature {temperature_raw}"));
            }
            if (temp_change_present
                && !(-MAX_TEMP_CHANGE_RAW..=MAX_TEMP_CHANGE_RAW)
                    .contains(&temperature_change_raw))
                || (!temp_change_present && temperature_change_raw != 0)
            {
                return Err(format!(
                    "province {id} has invalid temperature change {temperature_change_raw}"
                ));
            }
            if (water_present && !(0..=MAX_PRECIP_RAW).contains(&water_raw))
                || (!water_present && water_raw != 0)
            {
                return Err(format!("province {id} has invalid water value {water_raw}"));
            }
            if (snow_present && !(0..=MAX_PRECIP_RAW).contains(&snow_raw))
                || (!snow_present && snow_raw != 0)
            {
                return Err(format!("province {id} has invalid snow value {snow_raw}"));
            }
            if (custom_present && custom_modifier_count > MAX_CUSTOM_MODIFIER_COUNT)
                || (!custom_present && custom_modifier_count != 0)
            {
                return Err(format!(
                    "province {id} has invalid custom modifier count {custom_modifier_count}"
                ));
            }
            if reserved != 0 {
                return Err(format!("province {id} has reserved value {reserved}"));
            }
            self.provinces_by_id.insert(
                id,
                [
                    flags,
                    temperature_raw as u32,
                    temperature_change_raw as u32,
                    water_raw as u32,
                    snow_raw as u32,
                    custom_modifier_count,
                    reserved,
                ],
            );
        }
        if !delta && seen_provinces.len() != observed_provinces {
            return Err(format!(
                "baseline unique provinces {} != observed {observed_provinces}",
                seen_provinces.len()
            ));
        }

        self.weather_records += 1;
        if delta { self.deltas += 1; } else { self.baselines += 1; }
        self.first_hour.get_or_insert(hour);
        if let Some(previous) = self.previous_hour {
            if hour == previous + 1 { self.consecutive_pairs += 1; }
            else if hour != previous { self.nonconsecutive_pairs += 1; }
        }
        if !self.hours.insert(hour) {
            self.duplicate_hours += 1;
        }
        self.previous_hour = Some(hour);
        self.last_hour = Some(hour);
        let rain = self.flags_by_region.values().filter(|v| **v & RAIN_HEAVY != 0).count();
        let arctic = self.flags_by_region.values().filter(|v| **v & ARCTIC_WATER != 0).count();
        self.rain_heavy_min = self.rain_heavy_min.min(rain);
        self.rain_heavy_max = self.rain_heavy_max.max(rain);
        self.arctic_water_min = self.arctic_water_min.min(arctic);
        self.arctic_water_max = self.arctic_water_max.max(arctic);
        Ok(())
    }

    fn passed(&self) -> bool {
        self.weather_records >= 2
            && self.baselines >= 1
            && self.deltas >= 1
            && self.flags_by_region.len() == REGION_COUNT
            && self.provinces_by_id.len() == self.observed_province_count.unwrap_or_default()
            && self.hours.len() as u64 == self.weather_records
            && self.duplicate_hours == 0
            && self.skipped_hours() == 0
            && self.consecutive_pairs.saturating_add(1) == self.weather_records
            && self.nonconsecutive_pairs == 0
            && self.total_flag_transitions > 0
            && self.rain_heavy_transitions > 0
            && self.weather_kind_transitions > 0
            && self.expiry_refreshes > 0
            && self.rain_heavy_max < REGION_COUNT
            && self.arctic_water_min > 0
            && self.arctic_water_max < REGION_COUNT
    }

    fn skipped_hours(&self) -> u64 {
        match (self.hours.first(), self.hours.last()) {
            (Some(first), Some(last)) => {
                let span = i64::from(*last) - i64::from(*first) + 1;
                u64::try_from(span)
                    .unwrap_or_default()
                    .saturating_sub(self.hours.len() as u64)
            }
            _ => 0,
        }
    }

    fn json(&self) -> String {
        format!(
            concat!(
                "{{\n  \"status\": \"{}\",\n  \"weather_records\": {},\n",
                "  \"baselines\": {},\n  \"deltas\": {},\n  \"first_hour\": {},\n",
                "  \"last_hour\": {},\n  \"consecutive_pairs\": {},\n",
                "  \"nonconsecutive_pairs\": {},\n  \"duplicate_hours\": {},\n",
                "  \"unique_hours\": {},\n  \"skipped_hours\": {},\n",
                "  \"unique_region_ids\": {},\n",
                "  \"unique_province_ids\": {},\n",
                "  \"rain_heavy_count_min\": {},\n  \"rain_heavy_count_max\": {},\n",
                "  \"arctic_water_count_min\": {},\n  \"arctic_water_count_max\": {},\n",
                "  \"rain_heavy_transitions\": {},\n",
                "  \"weather_kind_transitions\": {},\n",
                "  \"expiry_refreshes\": {},\n  \"total_flag_transitions\": {}\n}}"
            ),
            if self.passed() { "PASS" } else { "FAIL" },
            self.weather_records, self.baselines, self.deltas,
            self.first_hour.unwrap_or_default(), self.last_hour.unwrap_or_default(),
            self.consecutive_pairs, self.nonconsecutive_pairs, self.duplicate_hours,
            self.hours.len(), self.skipped_hours(), self.flags_by_region.len(),
            self.provinces_by_id.len(),
            if self.rain_heavy_min == usize::MAX { 0 } else { self.rain_heavy_min },
            self.rain_heavy_max,
            if self.arctic_water_min == usize::MAX { 0 } else { self.arctic_water_min },
            self.arctic_water_max, self.rain_heavy_transitions,
            self.weather_kind_transitions, self.expiry_refreshes,
            self.total_flag_transitions,
        )
    }
}

#[derive(Debug)]
struct ValidatedStream {
    gate: Gate,
    compression: compact_stream::StreamCompression,
    encoded_bytes: u64,
    encoded_sha256: String,
    decoded_bytes: u64,
    decoded_sha256: String,
}

impl ValidatedStream {
    fn passed(&self) -> bool {
        self.gate.passed()
    }

    fn json(&self) -> String {
        format!(
            concat!(
                "{{\n  \"schema\": \"hoi4.a20-weather-temporal-gate.v2\",\n",
                "  \"status\": \"{}\",\n  \"storage_compression\": \"{}\",\n",
                "  \"encoded_bytes\": {},\n  \"encoded_sha256\": \"{}\",\n",
                "  \"decoded_bytes\": {},\n  \"decoded_sha256\": \"{}\",\n",
                "  \"weather_gate\": {}\n}}"
            ),
            if self.passed() { "PASS" } else { "FAIL" },
            self.compression.as_str(),
            self.encoded_bytes,
            self.encoded_sha256,
            self.decoded_bytes,
            self.decoded_sha256,
            self.gate.json(),
        )
    }
}

struct HashingReader<R> {
    inner: R,
    hasher: Sha256,
    bytes: u64,
}

impl<R> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }

    fn finish(self) -> (u64, String) {
        (self.bytes, format!("{:x}", self.hasher.finalize()))
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read]);
        self.bytes = self.bytes.saturating_add(read as u64);
        Ok(read)
    }
}

fn encoded_sha256(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("open encoded stream {}: {error}", path.display()))?;
    let mut reader = HashingReader::new(file);
    io::copy(&mut reader, &mut io::sink())
        .map_err(|error| format!("hash encoded stream {}: {error}", path.display()))?;
    Ok(reader.finish().1)
}

fn validate_file(path: &Path) -> Result<ValidatedStream, String> {
    let decoded = compact_stream::open(path)
        .map_err(|error| format!("open compact stream {}: {error}", path.display()))?;
    let compression = decoded.compression();
    let encoded_bytes = decoded.encoded_len();
    let encoded_sha256 = encoded_sha256(path)?;
    let mut file = HashingReader::new(decoded);
    let mut gate = Gate::new();
    // This evidence gate reads the retained pre-CID1 collector protocol. Legacy
    // streams have no sequence-zero identity header; their first ordinary H4CR
    // record is sequence one. Identity-v1 streams must be handled by an
    // explicitly selected StatefulStreamValidator instead of being sniffed here.
    let mut expected_sequence = 1u64;
    loop {
        let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
        match file.read(&mut header_bytes[..1]) {
            Ok(0) => break,
            Ok(1) => {}
            Ok(_) => unreachable!("one-byte header prefix read returned more than one byte"),
            Err(error) => return Err(format!("read H4CR header prefix: {error}")),
        }
        file.read_exact(&mut header_bytes[1..])
            .map_err(|error| format!("truncated H4CR header at EOF: {error}"))?;
        let header = parse_compact_record_header(&header_bytes).map_err(|e| e.to_string())?;
        if header.sequence != expected_sequence {
            return Err(format!(
                "sequence {} != expected {expected_sequence}",
                header.sequence
            ));
        }
        expected_sequence = header
            .sequence
            .checked_add(1)
            .ok_or("H4CR sequence overflow")?;
        let mut payload = vec![0u8; header.payload_len];
        file.read_exact(&mut payload).map_err(|e| format!("truncated payload at sequence {}: {e}", header.sequence))?;
        if header.domain_id == DOMAIN_WEATHER && header.record_type == RECORD_TYPE_DATA {
            if header.record_flags != 0 {
                return Err(format!(
                    "weather record {} has nonzero record flags {:#x}",
                    header.sequence, header.record_flags
                ));
            }
            gate.consume(header.game_hour_raw, &payload)?;
        }
    }
    let (decoded_bytes, decoded_sha256) = file.finish();
    Ok(ValidatedStream {
        gate,
        compression,
        encoded_bytes,
        encoded_sha256,
        decoded_bytes,
        decoded_sha256,
    })
}

fn main() {
    let Some(path) = env::args_os().nth(1) else {
        eprintln!("usage: weather_temporal_drift_gate <collector_stream.bin>");
        std::process::exit(2);
    };
    match validate_file(Path::new(&path)) {
        Ok(stream) => {
            println!("{}", stream.json());
            if !stream.passed() { std::process::exit(1); }
        }
        Err(error) => {
            eprintln!("weather temporal drift gate: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(delta: bool, changed_id: Option<u32>) -> Vec<u8> {
        let count = if delta { 1 } else { REGION_COUNT };
        let mut out = Vec::with_capacity(HEADER_LEN + count * REGION_ROW_LEN);
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&(if delta { COMPACT_STATUS_DELTA_DATA } else { 0 }).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(REGION_COUNT as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&100_000u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let ids: Vec<u32> = if delta { vec![changed_id.unwrap()] } else { (0..REGION_COUNT as u32).collect() };
        for id in ids {
            out.extend_from_slice(&id.to_le_bytes());
            let flags = if id == 0 { ARCTIC_WATER } else if id < 35 { RAIN_HEAVY } else { 0 };
            let flags = if delta { flags ^ RAIN_HEAVY } else { flags };
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&0i32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            let expiry = if delta { 200i32 } else { 100i32 };
            out.extend_from_slice(&expiry.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out
    }

    #[test]
    fn exact_sparse_flags_and_hourly_drift_pass() {
        let mut gate = Gate::new();
        gate.consume(100, &payload(false, None)).unwrap();
        gate.consume(101, &payload(true, Some(1))).unwrap();
        assert!(gate.passed(), "{}", gate.json());
    }

    #[test]
    fn stationary_capture_does_not_claim_temporal_proof() {
        let mut gate = Gate::new();
        gate.consume(100, &payload(false, None)).unwrap();
        assert!(!gate.passed());
    }

    #[test]
    fn duplicate_weather_hour_does_not_claim_zero_skip_temporal_proof() {
        let mut gate = Gate::new();
        gate.consume(100, &payload(false, None)).unwrap();
        gate.consume(100, &payload(true, Some(1))).unwrap();
        assert_eq!(gate.duplicate_hours, 1);
        assert!(!gate.passed());
    }

    #[test]
    fn malformed_observed_region_count_fails_closed() {
        let mut bytes = payload(false, None);
        bytes[24..28].copy_from_slice(&304u32.to_le_bytes());
        assert!(Gate::new().consume(100, &bytes).is_err());
    }

    #[test]
    fn negative_signed_transition_expiry_fails_closed() {
        let mut bytes = payload(false, None);
        let expiry = HEADER_LEN + 16;
        bytes[expiry..expiry + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Gate::new().consume(100, &bytes).is_err());
    }

    #[test]
    fn malformed_province_presence_contract_fails_closed() {
        let mut bytes = payload(false, None);
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        bytes[28..32].copy_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&PROVINCE_STRUCT_VALIDATED.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        assert!(Gate::new().consume(100, &bytes).is_err());
    }

    #[test]
    #[ignore = "external evidence: retained 423-hour a729 production corpus"]
    fn retained_a20_423_hour_stream_passes_exact_gate() {
        let run_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "../../runs/production_speed5_ShortSpeed5_20260712_000620",
        );
        let path = run_dir.join("collector_stream.bin");
        let manifest_path = run_dir.join("native_session_manifest.json");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&manifest_path).expect("retained A20 manifest must be readable"),
        )
        .expect("retained A20 manifest must be valid JSON");
        assert_eq!(manifest["schema"], "native_collector_session_v1");
        assert_eq!(manifest["raw_stream"], "collector_stream.bin");
        assert_eq!(manifest["stream_storage"]["compression"], "zstd");
        assert_eq!(manifest["stream_storage"]["stored_bytes"], 17_484_345);
        assert_eq!(manifest["stream_storage"]["decoded_bytes"], 1_488_548_730u64);
        assert_eq!(
            manifest["stream_storage"]["decoded_sha256"],
            "3857e2d0afa1c30350eb0f3cfcef0d3ee6643d282674ce75bd064a50c20db3a1"
        );
        assert_eq!(manifest["unique_game_hour_count"], 423);
        assert_eq!(manifest["skipped_game_hours"], 0);
        assert_eq!(manifest["first_game_hour_raw"], 60_794_379);
        assert_eq!(manifest["last_game_hour_raw"], 60_794_801);
        assert_eq!(manifest["compact_stream"]["ordered"], true);
        assert_eq!(manifest["compact_stream"]["record_magic"], "H4CR");
        assert_eq!(manifest["compact_stream"]["record_version"], 1);
        assert_eq!(
            manifest["compact_stream"]["per_domain"]["weather_live"]["domain_id"],
            DOMAIN_WEATHER
        );
        assert_eq!(
            manifest["compact_stream"]["per_domain"]["weather_live"]["data_record_count"],
            423
        );
        assert_eq!(
            manifest["compact_stream"]["per_domain"]["weather_live"]["status_flags_or"],
            COMPACT_STATUS_DELTA_DATA
        );
        assert_eq!(
            manifest["modset"]["game"]["exe_sha256"],
            "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
        );
        let stream = validate_file(&path).expect("retained A20 compact stream must decode");
        eprintln!("{}", stream.json());
        assert!(stream.passed(), "{}", stream.json());
        assert_eq!(stream.compression, compact_stream::StreamCompression::Zstd);
        assert_eq!(stream.encoded_bytes, 17_484_345);
        assert_eq!(
            stream.encoded_sha256,
            "25c020ba7f423759c945311f56fd78ecf27f7581835fbada13ffb1f542e986ee"
        );
        assert_eq!(stream.decoded_bytes, 1_488_548_730);
        assert_eq!(
            stream.decoded_sha256,
            "3857e2d0afa1c30350eb0f3cfcef0d3ee6643d282674ce75bd064a50c20db3a1"
        );
        assert_eq!(stream.gate.weather_records, 423);
        assert_eq!(stream.gate.baselines, 1);
        assert_eq!(stream.gate.deltas, 422);
        assert_eq!(stream.gate.first_hour, Some(60_794_379));
        assert_eq!(stream.gate.last_hour, Some(60_794_801));
        assert_eq!(stream.gate.consecutive_pairs, 422);
        assert_eq!(stream.gate.nonconsecutive_pairs, 0);
        assert_eq!(stream.gate.duplicate_hours, 0);
        assert_eq!(stream.gate.hours.len(), 423);
        assert_eq!(stream.gate.skipped_hours(), 0);
        assert_eq!(stream.gate.flags_by_region.len(), REGION_COUNT);
        assert_eq!(stream.gate.provinces_by_id.len(), 13_414);
        assert!(stream.gate.provinces_by_id.contains_key(&0));
        assert!(stream.gate.provinces_by_id.contains_key(&13_413));
    }
}
