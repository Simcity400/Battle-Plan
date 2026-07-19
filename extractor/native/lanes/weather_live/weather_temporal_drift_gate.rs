//! Offline A20 gate for the retained domain-24 weather stream.
//!
//! Build without Cargo:
//! `rustc --edition=2021 weather_temporal_drift_gate.rs -o weather_temporal_drift_gate.exe`
//! The validator streams the H4CR file, so multi-gigabyte captures do not need
//! to be mapped or buffered in memory.

#[path = "../../compact_contract/src/record_header.rs"]
mod record_header;
#[path = "../../compact_contract/src/status_flags_base.rs"]
mod status_flags_base;

use record_header::{parse_compact_record_header, COMPACT_RECORD_HEADER_LEN};
use status_flags_base::*;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

const DOMAIN_WEATHER: u16 = 24;
const RECORD_TYPE_DATA: u16 = 3;
const HEADER_LEN: usize = 48;
const REGION_ROW_LEN: usize = 32;
const PROVINCE_ROW_LEN: usize = 32;
const REGION_COUNT: usize = 305;
const KNOWN_REGION_FLAGS: u32 = 0xff;
const RAIN_HEAVY: u32 = 1 << 1;
const ARCTIC_WATER: u32 = 1 << 4;
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
    flags_by_region: BTreeMap<u32, u32>,
    rain_heavy_transitions: u64,
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
            let reserved = u32_at(payload, base + 28)?;
            if id > 304 || !seen.insert(id) {
                return Err(format!("invalid or duplicate region id {id}"));
            }
            if flags & !KNOWN_REGION_FLAGS != 0 {
                return Err(format!("region {id} has unknown flags {flags:#x}"));
            }
            if reserved != 0 {
                return Err(format!("region {id} has reserved value {reserved}"));
            }
            if flags & (1 << 5) != 0 {
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
            }
        }
        if computed_active != active_regions {
            return Err(format!("active region header {active_regions} != computed {computed_active}"));
        }
        if !delta && seen.len() != REGION_COUNT {
            return Err(format!("baseline unique regions {} != {REGION_COUNT}", seen.len()));
        }

        self.weather_records += 1;
        if delta { self.deltas += 1; } else { self.baselines += 1; }
        self.first_hour.get_or_insert(hour);
        if let Some(previous) = self.previous_hour {
            if hour == previous + 1 { self.consecutive_pairs += 1; }
            else if hour != previous { self.nonconsecutive_pairs += 1; }
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
            && self.consecutive_pairs >= 1
            && self.nonconsecutive_pairs == 0
            && self.total_flag_transitions > 0
            && self.rain_heavy_transitions > 0
            && self.rain_heavy_max < REGION_COUNT
            && self.arctic_water_min > 0
            && self.arctic_water_max < REGION_COUNT
    }

    fn json(&self) -> String {
        format!(
            concat!(
                "{{\n  \"status\": \"{}\",\n  \"weather_records\": {},\n",
                "  \"baselines\": {},\n  \"deltas\": {},\n  \"first_hour\": {},\n",
                "  \"last_hour\": {},\n  \"consecutive_pairs\": {},\n",
                "  \"nonconsecutive_pairs\": {},\n  \"unique_region_ids\": {},\n",
                "  \"rain_heavy_count_min\": {},\n  \"rain_heavy_count_max\": {},\n",
                "  \"arctic_water_count_min\": {},\n  \"arctic_water_count_max\": {},\n",
                "  \"rain_heavy_transitions\": {},\n  \"total_flag_transitions\": {}\n}}"
            ),
            if self.passed() { "PASS" } else { "FAIL" },
            self.weather_records, self.baselines, self.deltas,
            self.first_hour.unwrap_or_default(), self.last_hour.unwrap_or_default(),
            self.consecutive_pairs, self.nonconsecutive_pairs, self.flags_by_region.len(),
            if self.rain_heavy_min == usize::MAX { 0 } else { self.rain_heavy_min },
            self.rain_heavy_max,
            if self.arctic_water_min == usize::MAX { 0 } else { self.arctic_water_min },
            self.arctic_water_max, self.rain_heavy_transitions, self.total_flag_transitions,
        )
    }
}

fn validate_file(path: &Path) -> Result<Gate, String> {
    let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut gate = Gate::new();
    let mut expected_sequence = None;
    loop {
        let mut header_bytes = [0u8; COMPACT_RECORD_HEADER_LEN];
        match file.read_exact(&mut header_bytes) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                let mut trailing = Vec::new();
                file.read_to_end(&mut trailing).map_err(|read| read.to_string())?;
                if trailing.is_empty() { break; }
                return Err("truncated H4CR header at EOF".into());
            }
            Err(e) => return Err(format!("read H4CR header: {e}")),
        }
        let header = parse_compact_record_header(&header_bytes).map_err(|e| e.to_string())?;
        if let Some(expected) = expected_sequence {
            if header.sequence != expected {
                return Err(format!("sequence {} != expected {expected}", header.sequence));
            }
        }
        expected_sequence = Some(header.sequence.saturating_add(1));
        let mut payload = vec![0u8; header.payload_len];
        file.read_exact(&mut payload).map_err(|e| format!("truncated payload at sequence {}: {e}", header.sequence))?;
        if header.domain_id == DOMAIN_WEATHER && header.record_type == RECORD_TYPE_DATA {
            gate.consume(header.game_hour_raw, &payload)?;
        }
    }
    Ok(gate)
}

fn main() {
    let Some(path) = env::args_os().nth(1) else {
        eprintln!("usage: weather_temporal_drift_gate <collector_stream.bin>");
        std::process::exit(2);
    };
    match validate_file(Path::new(&path)) {
        Ok(gate) => {
            println!("{}", gate.json());
            if !gate.passed() { std::process::exit(1); }
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
            out.extend_from_slice(&0u32.to_le_bytes());
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
    fn malformed_observed_region_count_fails_closed() {
        let mut bytes = payload(false, None);
        bytes[24..28].copy_from_slice(&304u32.to_le_bytes());
        assert!(Gate::new().consume(100, &bytes).is_err());
    }
}
