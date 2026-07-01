#[path = "domain44_compact.rs"]
#[allow(dead_code)]
mod domain44_compact;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process;

const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
const COMPACT_RECORD_HEADER_LEN: usize = 32;
const COMPACT_RECORD_VERSION: u16 = 1;
const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

fn main() {
    let mut stream_mode = false;
    let mut input_path = None;
    for arg in env::args().skip(1) {
        if arg == "--stream" {
            stream_mode = true;
        } else if input_path.is_none() {
            input_path = Some(arg);
        } else {
            eprintln!("usage: domain44_gate [--stream] <payload-or-stream-path>");
            process::exit(64);
        }
    }

    let Some(input_path) = input_path else {
        eprintln!("usage: domain44_gate [--stream] <payload-or-stream-path>");
        process::exit(64);
    };

    let bytes = match fs::read(&input_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("failed to read {input_path}: {err}");
            process::exit(66);
        }
    };

    let result = if stream_mode {
        decode_stream(&bytes)
    } else {
        let summary = domain44_compact::decode_payload(0, &bytes);
        GateResult {
            records_seen: 1,
            domain_data_records: 1,
            rows: summary.data_rows_total,
            changed_rows: summary.data_changed_rows_total,
            units: summary.data_units_total,
            errors: summary.parse_errors,
        }
    };

    println!("expected_domain_id={}", domain44_compact::DOMAIN_ID);
    println!("records_seen={}", result.records_seen);
    println!("domain_data_records={}", result.domain_data_records);
    println!("rows={}", result.rows);
    println!("changed_rows={}", result.changed_rows);
    for (key, value) in &result.units {
        println!("{key}={value}");
    }

    if !result.errors.is_empty() {
        for error in result.errors {
            eprintln!("{error}");
        }
        process::exit(1);
    }
}

struct GateResult {
    records_seen: u64,
    domain_data_records: u64,
    rows: u64,
    changed_rows: u64,
    units: BTreeMap<&'static str, u64>,
    errors: Vec<String>,
}

fn decode_stream(bytes: &[u8]) -> GateResult {
    let mut cursor = 0usize;
    let mut records_seen = 0u64;
    let mut domain_data_records = 0u64;
    let mut rows = 0u64;
    let mut changed_rows = 0u64;
    let mut units = BTreeMap::new();
    let mut errors = Vec::new();

    while cursor < bytes.len() {
        let record_offset = cursor;
        if bytes.len().saturating_sub(cursor) < COMPACT_RECORD_HEADER_LEN {
            errors.push(format!(
                "offset {record_offset}: compact record has truncated header"
            ));
            break;
        }
        if &bytes[cursor..cursor + 4] != COMPACT_RECORD_MAGIC {
            errors.push(format!(
                "offset {record_offset}: compact record magic mismatch"
            ));
            break;
        }
        let version = read_u16(bytes, cursor + 4);
        let header_len = read_u16(bytes, cursor + 6) as usize;
        let domain_id = read_u16(bytes, cursor + 20);
        let record_type = read_u16(bytes, cursor + 22);
        let payload_len = read_u32(bytes, cursor + 24) as usize;
        let reserved = read_u32(bytes, cursor + 28);
        if version != COMPACT_RECORD_VERSION {
            errors.push(format!(
                "offset {record_offset}: compact record version {version} != {COMPACT_RECORD_VERSION}"
            ));
        }
        if header_len != COMPACT_RECORD_HEADER_LEN {
            errors.push(format!(
                "offset {record_offset}: compact record header_len {header_len} != {COMPACT_RECORD_HEADER_LEN}"
            ));
            break;
        }
        if reserved != 0 {
            errors.push(format!(
                "offset {record_offset}: compact record reserved field is nonzero"
            ));
        }
        cursor += header_len;
        let Some(payload_end) = cursor.checked_add(payload_len) else {
            errors.push(format!(
                "offset {record_offset}: compact record payload length overflows"
            ));
            break;
        };
        if payload_end > bytes.len() {
            errors.push(format!(
                "offset {record_offset}: compact record payload is truncated"
            ));
            break;
        }
        records_seen = records_seen.saturating_add(1);
        if domain_id == domain44_compact::DOMAIN_ID
            && record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
        {
            let summary = domain44_compact::decode_payload(cursor, &bytes[cursor..payload_end]);
            domain_data_records = domain_data_records.saturating_add(1);
            rows = rows.saturating_add(summary.data_rows_total);
            changed_rows = changed_rows.saturating_add(summary.data_changed_rows_total);
            for (key, value) in summary.data_units_total {
                *units.entry(key).or_default() += value;
            }
            errors.extend(summary.parse_errors);
        }
        cursor = payload_end;
    }

    if domain_data_records == 0 {
        errors.push(format!(
            "compact stream did not contain domain {} data records",
            domain44_compact::DOMAIN_ID
        ));
    }

    GateResult {
        records_seen,
        domain_data_records,
        rows,
        changed_rows,
        units,
        errors,
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
