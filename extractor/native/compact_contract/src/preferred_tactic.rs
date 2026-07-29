//! Pointer-free country preferred-tactic snapshot (PTC1 / compact domain 92).
//!
//! The exact a729 producer reads `CCountry+0x15D0`.  The pointee is either the
//! exact no-tactic sentinel class or an authenticated combat-tactic definition;
//! process addresses never enter this payload. A null field is not an
//! authenticated no-selection witness and must remain source-unavailable.

use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 92;
pub const MAGIC: [u8; 4] = *b"PTC1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 164;
pub const MAX_ROWS: usize = 1024;
pub const MAX_TOKEN_BYTES: usize = 256;

pub const FLAG_SELECTED: u16 = 1 << 0;
pub const FLAG_TOKEN_PRESENT: u16 = 1 << 1;
const KNOWN_FLAGS: u16 = FLAG_SELECTED | FLAG_TOKEN_PRESENT;

pub const EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];

/// Ghidra manifest `430665f0...`: country root/consumer and the explicit
/// separation from the `CCountryLeader+0x10B0` army-leader field.
pub const CONSUMER_EVIDENCE_SHA256: [u8; 32] = [
    0x43, 0x06, 0x65, 0xf0, 0x5f, 0xec, 0x12, 0x94, 0x79, 0x0c, 0xb7, 0xe6, 0x17, 0x34, 0x26, 0x48,
    0xb7, 0x47, 0x57, 0x31, 0xec, 0x5b, 0xd0, 0xc4, 0xa1, 0x9a, 0x40, 0x12, 0x3a, 0xf2, 0x4d, 0xb3,
];

/// Ghidra manifest `d6327e0e...`: `CSetPreferredTacticCommand` validation and
/// execute writer, including the exact `*(country+0x15D0)=selected_definition`.
pub const WRITER_EVIDENCE_SHA256: [u8; 32] = [
    0xd6, 0x32, 0x7e, 0x0e, 0x86, 0x22, 0x9e, 0x5c, 0x1e, 0xc6, 0xbe, 0x0c, 0x13, 0x1b, 0xd9, 0x5d,
    0xcb, 0x31, 0x89, 0x71, 0x9d, 0x68, 0x26, 0x4e, 0x7b, 0xb5, 0x95, 0x99, 0x54, 0xd4, 0x3a, 0x49,
];

/// Ghidra manifest `75d225d3...`: exact tactic registry population and stable
/// definition-identity surface.
pub const IDENTITY_EVIDENCE_SHA256: [u8; 32] = [
    0x75, 0xd2, 0x25, 0xd3, 0x08, 0x98, 0x5d, 0xad, 0x5e, 0xfe, 0x74, 0x33, 0xef, 0xad, 0x10, 0xff,
    0xae, 0xe1, 0xbc, 0x48, 0x43, 0x26, 0x74, 0x7a, 0x7c, 0x96, 0xb9, 0x6e, 0x3b, 0x49, 0xac, 0xa8,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreferredTacticRow {
    pub tag_db_id: u32,
    pub tactic_database_id: i32,
    pub tactic_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreferredTacticSnapshot {
    pub read_time_us: u64,
    pub status_flags: u32,
    pub source_sequence: u64,
    pub rows: Vec<PreferredTacticRow>,
}

fn take<const N: usize>(payload: &[u8], at: &mut usize) -> Result<[u8; N], String> {
    let end = at.checked_add(N).ok_or("PTC1 cursor overflow")?;
    let bytes = payload.get(*at..end).ok_or("PTC1 truncated")?;
    *at = end;
    Ok(bytes.try_into().expect("fixed-size slice"))
}

pub fn encode_payload(snapshot: &PreferredTacticSnapshot) -> Result<Vec<u8>, String> {
    validate_rows(&snapshot.rows)?;
    let selected_count = snapshot
        .rows
        .iter()
        .filter(|row| row.tactic_database_id > 0)
        .count();
    let row_count = u32::try_from(snapshot.rows.len()).map_err(|_| "PTC1 too many rows")?;
    let selected_count =
        u32::try_from(selected_count).map_err(|_| "PTC1 too many selected rows")?;
    let mut out = Vec::with_capacity(HEADER_LEN + snapshot.rows.len() * 16);
    out.extend_from_slice(&snapshot.read_time_us.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&selected_count.to_le_bytes());
    out.extend_from_slice(&snapshot.status_flags.to_le_bytes());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&snapshot.source_sequence.to_le_bytes());
    out.extend_from_slice(&EXECUTABLE_SHA256);
    out.extend_from_slice(&CONSUMER_EVIDENCE_SHA256);
    out.extend_from_slice(&WRITER_EVIDENCE_SHA256);
    out.extend_from_slice(&IDENTITY_EVIDENCE_SHA256);
    for row in &snapshot.rows {
        let token = row.tactic_id.as_deref().unwrap_or("").as_bytes();
        let flags = if row.tactic_database_id == 0 {
            0
        } else {
            FLAG_SELECTED | FLAG_TOKEN_PRESENT
        };
        out.extend_from_slice(&row.tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.tactic_database_id.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(token.len() as u16).to_le_bytes());
        out.extend_from_slice(token);
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<PreferredTacticSnapshot, String> {
    if payload.len() < HEADER_LEN {
        return Err("PTC1 header truncated".into());
    }
    let mut at = 0usize;
    let read_time_us = u64::from_le_bytes(take(payload, &mut at)?);
    let row_count = u32::from_le_bytes(take(payload, &mut at)?) as usize;
    let selected_count = u32::from_le_bytes(take(payload, &mut at)?) as usize;
    let status_flags = u32::from_le_bytes(take(payload, &mut at)?);
    if take::<4>(payload, &mut at)? != MAGIC
        || u16::from_le_bytes(take(payload, &mut at)?) != VERSION
        || u16::from_le_bytes(take(payload, &mut at)?) as usize != HEADER_LEN
    {
        return Err("PTC1 magic/version/header mismatch".into());
    }
    let source_sequence = u64::from_le_bytes(take(payload, &mut at)?);
    if take::<32>(payload, &mut at)? != EXECUTABLE_SHA256
        || take::<32>(payload, &mut at)? != CONSUMER_EVIDENCE_SHA256
        || take::<32>(payload, &mut at)? != WRITER_EVIDENCE_SHA256
        || take::<32>(payload, &mut at)? != IDENTITY_EVIDENCE_SHA256
    {
        return Err("PTC1 authority receipt mismatch".into());
    }
    if row_count > MAX_ROWS || selected_count > row_count {
        return Err("PTC1 row count outside bounds".into());
    }
    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        let tag_db_id = u32::from_le_bytes(take(payload, &mut at)?);
        let tactic_database_id = i32::from_le_bytes(take(payload, &mut at)?);
        let flags = u16::from_le_bytes(take(payload, &mut at)?);
        let token_len = u16::from_le_bytes(take(payload, &mut at)?) as usize;
        if flags & !KNOWN_FLAGS != 0 || token_len > MAX_TOKEN_BYTES {
            return Err("PTC1 row flags/token length invalid".into());
        }
        let end = at
            .checked_add(token_len)
            .ok_or("PTC1 token cursor overflow")?;
        let token = payload.get(at..end).ok_or("PTC1 token truncated")?;
        at = end;
        let tactic_id = if token.is_empty() {
            None
        } else {
            Some(
                std::str::from_utf8(token)
                    .map_err(|_| "PTC1 tactic token is not UTF-8")?
                    .to_owned(),
            )
        };
        let selected = tactic_database_id > 0;
        if tactic_database_id < 0
            || (selected != (flags == (FLAG_SELECTED | FLAG_TOKEN_PRESENT) && tactic_id.is_some()))
            || (!selected && (flags != 0 || tactic_id.is_some()))
        {
            return Err("PTC1 none/selected identity invariant failed".into());
        }
        rows.push(PreferredTacticRow {
            tag_db_id,
            tactic_database_id,
            tactic_id,
        });
    }
    if at != payload.len() {
        return Err("PTC1 trailing bytes".into());
    }
    validate_rows(&rows)?;
    if rows.iter().filter(|row| row.tactic_database_id > 0).count() != selected_count {
        return Err("PTC1 selected count mismatch".into());
    }
    Ok(PreferredTacticSnapshot {
        read_time_us,
        status_flags,
        source_sequence,
        rows,
    })
}

fn validate_rows(rows: &[PreferredTacticRow]) -> Result<(), String> {
    if rows.len() > MAX_ROWS {
        return Err("PTC1 too many rows".into());
    }
    let mut previous = 0u32;
    let mut by_id: BTreeMap<i32, &str> = BTreeMap::new();
    let mut by_token: BTreeMap<&str, i32> = BTreeMap::new();
    let mut tags = BTreeSet::new();
    for row in rows {
        if row.tag_db_id == 0 || row.tag_db_id <= previous || !tags.insert(row.tag_db_id) {
            return Err("PTC1 rows must have unique ascending nonzero tag ids".into());
        }
        previous = row.tag_db_id;
        match (row.tactic_database_id, row.tactic_id.as_deref()) {
            (0, None) => {}
            (id, Some(token)) if id > 0 => {
                if token.is_empty()
                    || token.len() > MAX_TOKEN_BYTES
                    || token.chars().any(char::is_control)
                {
                    return Err("PTC1 tactic token invalid".into());
                }
                if by_id.insert(id, token).is_some_and(|old| old != token)
                    || by_token.insert(token, id).is_some_and(|old| old != id)
                {
                    return Err("PTC1 tactic id/token mapping is not injective".into());
                }
            }
            _ => return Err("PTC1 invalid tactic id/token pair".into()),
        }
    }
    Ok(())
}
