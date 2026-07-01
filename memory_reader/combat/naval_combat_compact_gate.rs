// Lane-local Rust decoder/gate for naval_combat_live compact records.
//
// This file intentionally does not modify or depend on shared recorder_host
// entrypoints. It mirrors the proposed compact schema so the decoder/gate can
// be copied into recorder_host when an integration slot is granted.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::PathBuf;

const MAGIC: &[u8; 4] = b"H4CR";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 32;
const RECORD_TYPE_DOMAIN_DATA: u16 = 3;

const DOMAIN_NAVAL_COMBAT_LIFECYCLE: u16 = 49;
const DOMAIN_NAVAL_COMBAT_MEMBERS: u16 = 50;
const SCHEMA_VERSION: u16 = 1;
const COMBAT_ROW_LEN: usize = 72;
const MEMBER_ROW_LEN: usize = 72;
const PAYLOAD_HEADER_LEN: usize = 28;

const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
const STATUS_PARTIAL_DATA: u32 = 1 << 9;

const COMBAT_ROW_DELETED: u32 = 1;
const MEMBER_ROW_DELETED: u32 = 1;
const SIDE_ATTACKER: u32 = 1;
const SIDE_DEFENDER: u32 = 2;

#[derive(Debug, Clone)]
struct CompactRecord<'a> {
    offset: usize,
    sequence: u64,
    domain_id: u16,
    record_type: u16,
    payload: &'a [u8],
}

#[derive(Debug, Default)]
struct GateSummary {
    record_count: usize,
    combat_row_count: usize,
    member_row_count: usize,
    active_combat_count: usize,
    two_sided_combat_count: usize,
    status_flags: u32,
    domains: Vec<&'static str>,
}

#[derive(Debug)]
struct GateResult {
    ok: bool,
    errors: Vec<String>,
    summary: GateSummary,
}

#[derive(Debug)]
struct PayloadHeader {
    status_flags: u32,
    row_count: usize,
    string_count: usize,
}

fn main() {
    let mut args = env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("usage: naval_combat_compact_gate <compact.bin> [--json-out path]");
        std::process::exit(2);
    };
    let mut json_out: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        if arg == "--json-out" {
            let Some(path) = args.next() else {
                eprintln!("--json-out requires a path");
                std::process::exit(2);
            };
            json_out = Some(PathBuf::from(path));
        } else {
            eprintln!("unknown argument: {arg}");
            std::process::exit(2);
        }
    }

    let data = match fs::read(&input) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("read {input} failed: {err}");
            std::process::exit(1);
        }
    };
    let result = gate_stream(&data);
    let json = result_json(&result);
    if let Some(path) = json_out {
        if let Err(err) = fs::write(&path, &json) {
            eprintln!("write {} failed: {err}", path.display());
            std::process::exit(1);
        }
    }
    println!("{json}");
    std::process::exit(if result.ok { 0 } else { 1 });
}

fn gate_stream(data: &[u8]) -> GateResult {
    let mut errors = Vec::new();
    let records = match decode_records(data) {
        Ok(records) => records,
        Err(err) => {
            return GateResult {
                ok: false,
                errors: vec![err],
                summary: GateSummary::default(),
            };
        }
    };
    let mut summary = GateSummary {
        record_count: records.len(),
        ..GateSummary::default()
    };
    let mut last_sequence: Option<u64> = None;
    let mut lifecycle_keys: BTreeSet<(u32, u64)> = BTreeSet::new();
    let mut member_sides: BTreeMap<(u32, u64), BTreeSet<u32>> = BTreeMap::new();
    let mut seen_domains: BTreeSet<u16> = BTreeSet::new();

    for record in records {
        if let Some(previous) = last_sequence {
            if record.sequence <= previous {
                errors.push("record sequence is not strictly increasing".to_string());
            }
        }
        last_sequence = Some(record.sequence);
        seen_domains.insert(record.domain_id);
        match record.domain_id {
            DOMAIN_NAVAL_COMBAT_LIFECYCLE => match decode_combat_payload(record.payload) {
                Ok((header, rows)) => {
                    summary.status_flags |= header.status_flags;
                    summary.combat_row_count += rows.len();
                    for row in rows {
                        if row.reserved != 0 {
                            errors.push(format!(
                                "offset {}: combat row reserved field is nonzero",
                                record.offset
                            ));
                        }
                        if row.flags & COMBAT_ROW_DELETED == 0 {
                            lifecycle_keys.insert((row.combat_id, row.generation));
                        }
                    }
                }
                Err(err) => errors.push(format!("offset {}: {err}", record.offset)),
            },
            DOMAIN_NAVAL_COMBAT_MEMBERS => match decode_member_payload(record.payload) {
                Ok((header, rows)) => {
                    summary.status_flags |= header.status_flags;
                    summary.member_row_count += rows.len();
                    for row in rows {
                        if row.reserved != 0 {
                            errors.push(format!(
                                "offset {}: member row reserved field is nonzero",
                                record.offset
                            ));
                        }
                        if row.flags & MEMBER_ROW_DELETED == 0 {
                            member_sides
                                .entry((row.combat_id, row.generation))
                                .or_default()
                                .insert(row.side);
                        }
                    }
                }
                Err(err) => errors.push(format!("offset {}: {err}", record.offset)),
            },
            other => errors.push(format!("offset {}: unexpected domain {other}", record.offset)),
        }
        if record.record_type != RECORD_TYPE_DOMAIN_DATA {
            errors.push(format!("offset {}: unexpected record type {}", record.offset, record.record_type));
        }
    }

    summary.domains = seen_domains.iter().map(|domain| domain_name(*domain)).collect();
    summary.active_combat_count = lifecycle_keys.len();
    summary.two_sided_combat_count = lifecycle_keys
        .iter()
        .filter(|key| {
            member_sides
                .get(key)
                .map(|sides| sides.contains(&SIDE_ATTACKER) && sides.contains(&SIDE_DEFENDER))
                .unwrap_or(false)
        })
        .count();

    if !seen_domains.contains(&DOMAIN_NAVAL_COMBAT_LIFECYCLE) {
        errors.push("missing naval combat lifecycle domain".to_string());
    }
    if !seen_domains.contains(&DOMAIN_NAVAL_COMBAT_MEMBERS) {
        errors.push("missing naval combat member domain".to_string());
    }
    if summary.status_flags & STATUS_SOURCE_UNAVAILABLE != 0 {
        errors.push("source unavailable status flag set".to_string());
    }
    if summary.status_flags & STATUS_PARTIAL_DATA != 0 {
        errors.push("partial data status flag set".to_string());
    }
    if lifecycle_keys.is_empty() {
        errors.push("no active lifecycle rows".to_string());
    }
    for key in lifecycle_keys {
        let two_sided = member_sides
            .get(&key)
            .map(|sides| sides.contains(&SIDE_ATTACKER) && sides.contains(&SIDE_DEFENDER))
            .unwrap_or(false);
        if !two_sided {
            errors.push(format!("combat {}:{} lacks two-sided member rows", key.0, key.1));
        }
    }

    GateResult {
        ok: errors.is_empty(),
        errors,
        summary,
    }
}

fn decode_records(data: &[u8]) -> Result<Vec<CompactRecord<'_>>, String> {
    let mut records = Vec::new();
    let mut cursor = 0usize;
    while cursor < data.len() {
        require(data, cursor, HEADER_LEN, "compact header")?;
        if &data[cursor..cursor + 4] != MAGIC {
            return Err(format!("bad compact magic at offset {cursor}"));
        }
        let version = read_u16(data, cursor + 4);
        let header_len = read_u16(data, cursor + 6) as usize;
        let sequence = read_u64(data, cursor + 8);
        let domain_id = read_u16(data, cursor + 20);
        let record_type = read_u16(data, cursor + 22);
        let payload_len = read_u32(data, cursor + 24) as usize;
        let reserved = read_u32(data, cursor + 28);
        if version != VERSION {
            return Err(format!("bad compact version {version} at offset {cursor}"));
        }
        if header_len != HEADER_LEN {
            return Err(format!("bad compact header length {header_len} at offset {cursor}"));
        }
        if reserved != 0 {
            return Err(format!("nonzero compact header reserved at offset {cursor}"));
        }
        let payload_start = cursor + HEADER_LEN;
        require(data, payload_start, payload_len, "compact payload")?;
        records.push(CompactRecord {
            offset: cursor,
            sequence,
            domain_id,
            record_type,
            payload: &data[payload_start..payload_start + payload_len],
        });
        cursor = payload_start + payload_len;
    }
    Ok(records)
}

#[derive(Debug)]
struct CombatRow {
    combat_id: u32,
    generation: u64,
    flags: u32,
    reserved: u32,
}

#[derive(Debug)]
struct MemberRow {
    combat_id: u32,
    generation: u64,
    flags: u32,
    side: u32,
    reserved: u32,
}

fn decode_combat_payload(payload: &[u8]) -> Result<(PayloadHeader, Vec<CombatRow>), String> {
    let header = decode_payload_header(payload, COMBAT_ROW_LEN)?;
    let mut cursor = PAYLOAD_HEADER_LEN;
    let mut rows = Vec::with_capacity(header.row_count);
    for _ in 0..header.row_count {
        require(payload, cursor, COMBAT_ROW_LEN, "combat row")?;
        rows.push(CombatRow {
            combat_id: read_u32(payload, cursor),
            generation: read_u64(payload, cursor + 4),
            flags: read_u32(payload, cursor + 20),
            reserved: read_u32(payload, cursor + 68),
        });
        cursor += COMBAT_ROW_LEN;
    }
    validate_string_table(payload, cursor, header.string_count)?;
    Ok((header, rows))
}

fn decode_member_payload(payload: &[u8]) -> Result<(PayloadHeader, Vec<MemberRow>), String> {
    let header = decode_payload_header(payload, MEMBER_ROW_LEN)?;
    let mut cursor = PAYLOAD_HEADER_LEN;
    let mut rows = Vec::with_capacity(header.row_count);
    for _ in 0..header.row_count {
        require(payload, cursor, MEMBER_ROW_LEN, "member row")?;
        rows.push(MemberRow {
            combat_id: read_u32(payload, cursor),
            generation: read_u64(payload, cursor + 4),
            flags: read_u32(payload, cursor + 12),
            side: read_u32(payload, cursor + 16),
            reserved: read_u32(payload, cursor + 68),
        });
        cursor += MEMBER_ROW_LEN;
    }
    validate_string_table(payload, cursor, header.string_count)?;
    Ok((header, rows))
}

fn decode_payload_header(payload: &[u8], expected_row_len: usize) -> Result<PayloadHeader, String> {
    require(payload, 0, PAYLOAD_HEADER_LEN, "payload header")?;
    let version = read_u16(payload, 0);
    let row_len = read_u16(payload, 2) as usize;
    let status_flags = read_u32(payload, 12);
    let row_count = read_u32(payload, 16) as usize;
    let string_count = read_u32(payload, 20) as usize;
    let reserved = read_u32(payload, 24);
    if version != SCHEMA_VERSION {
        return Err(format!("unsupported schema version {version}"));
    }
    if row_len != expected_row_len {
        return Err(format!("unexpected row length {row_len}"));
    }
    if reserved != 0 {
        return Err("payload reserved field is nonzero".to_string());
    }
    if row_count > 100_000 {
        return Err("row count exceeds limit".to_string());
    }
    if string_count > 4096 {
        return Err("string count exceeds limit".to_string());
    }
    Ok(PayloadHeader {
        status_flags,
        row_count,
        string_count,
    })
}

fn validate_string_table(payload: &[u8], mut cursor: usize, string_count: usize) -> Result<(), String> {
    for index in 0..string_count {
        require(payload, cursor, 2, "string length")?;
        let len = read_u16(payload, cursor) as usize;
        cursor += 2;
        if len > 512 {
            return Err("string entry exceeds limit".to_string());
        }
        require(payload, cursor, len, "string entry")?;
        if index == 0 && len != 0 {
            return Err("string table index 0 must be empty".to_string());
        }
        cursor += len;
    }
    if cursor != payload.len() {
        return Err("trailing bytes after string table".to_string());
    }
    Ok(())
}

fn require(data: &[u8], offset: usize, needed: usize, label: &str) -> Result<(), String> {
    if offset.saturating_add(needed) > data.len() {
        Err(format!(
            "{label} overruns buffer at offset {offset}: need {needed}, remaining {}",
            data.len().saturating_sub(offset)
        ))
    } else {
        Ok(())
    }
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]])
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

fn domain_name(domain_id: u16) -> &'static str {
    match domain_id {
        DOMAIN_NAVAL_COMBAT_LIFECYCLE => "naval_combat_lifecycle_live",
        DOMAIN_NAVAL_COMBAT_MEMBERS => "naval_combat_members_live",
        _ => "unknown",
    }
}

fn result_json(result: &GateResult) -> String {
    format!(
        "{{\n  \"ok\": {},\n  \"errors\": {},\n  \"summary\": {{\n    \"record_count\": {},\n    \"domains\": {},\n    \"combat_row_count\": {},\n    \"member_row_count\": {},\n    \"active_combat_count\": {},\n    \"two_sided_combat_count\": {},\n    \"status_flags\": {}\n  }}\n}}\n",
        result.ok,
        json_string_array(&result.errors),
        result.summary.record_count,
        json_str_array(&result.summary.domains),
        result.summary.combat_row_count,
        result.summary.member_row_count,
        result.summary.active_combat_count,
        result.summary.two_sided_combat_count,
        result.summary.status_flags,
    )
}

fn json_string_array(values: &[String]) -> String {
    let parts = values.iter().map(|value| json_str(value)).collect::<Vec<_>>();
    format!("[{}]", parts.join(", "))
}

fn json_str_array(values: &[&str]) -> String {
    let parts = values.iter().map(|value| json_str(value)).collect::<Vec<_>>();
    format!("[{}]", parts.join(", "))
}

fn json_str(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn append_record(out: &mut Vec<u8>, sequence: u64, domain: u16, payload: &[u8]) {
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&42i32.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn payload(row_len: usize, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
        out.extend_from_slice(&(row_len as u16).to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&(1u32 << 10).to_le_bytes());
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for row in rows {
            out.extend_from_slice(row);
        }
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn combat_row(combat_id: u32, generation: u64) -> Vec<u8> {
        let mut row = vec![0u8; COMBAT_ROW_LEN];
        row[0..4].copy_from_slice(&combat_id.to_le_bytes());
        row[4..12].copy_from_slice(&generation.to_le_bytes());
        row
    }

    fn member_row(combat_id: u32, generation: u64, side: u32) -> Vec<u8> {
        let mut row = vec![0u8; MEMBER_ROW_LEN];
        row[0..4].copy_from_slice(&combat_id.to_le_bytes());
        row[4..12].copy_from_slice(&generation.to_le_bytes());
        row[16..20].copy_from_slice(&side.to_le_bytes());
        row
    }

    #[test]
    fn two_sided_stream_passes_gate() {
        let mut stream = Vec::new();
        append_record(&mut stream, 1, DOMAIN_NAVAL_COMBAT_LIFECYCLE, &payload(COMBAT_ROW_LEN, &[combat_row(7, 1)]));
        append_record(
            &mut stream,
            2,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            &payload(MEMBER_ROW_LEN, &[member_row(7, 1, SIDE_ATTACKER), member_row(7, 1, SIDE_DEFENDER)]),
        );
        let result = gate_stream(&stream);
        assert!(result.ok, "{:?}", result.errors);
        assert_eq!(result.summary.two_sided_combat_count, 1);
    }

    #[test]
    fn missing_defender_fails_gate() {
        let mut stream = Vec::new();
        append_record(&mut stream, 1, DOMAIN_NAVAL_COMBAT_LIFECYCLE, &payload(COMBAT_ROW_LEN, &[combat_row(7, 1)]));
        append_record(
            &mut stream,
            2,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            &payload(MEMBER_ROW_LEN, &[member_row(7, 1, SIDE_ATTACKER)]),
        );
        let result = gate_stream(&stream);
        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("lacks two-sided member rows")));
    }
}
