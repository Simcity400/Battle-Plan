use std::collections::{BTreeMap, BTreeSet};

use crate::preferred_tactic::PreferredTacticSnapshot;

pub const DOMAIN_ID: u16 = 96;
pub const MAGIC: [u8; 4] = *b"PTW1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 276;
pub const FIXED_SCALE: i64 = 100_000;
pub const FIXED_ROW_LEN: usize = 40;

pub const MAX_COUNTRIES: usize = i32::MAX as usize;

#[inline]
#[must_use]
pub const fn max_rows_in(remaining: usize) -> usize {
    remaining / (FIXED_ROW_LEN + 1)
}
pub const MAX_TOKEN_BYTES: usize = 256;

pub const COUNTRY_PREFERRED_TACTIC_OFFSET: usize = 0x15D0;
pub const COUNTRY_MODIFIER_ACCUMULATOR_OFFSET: usize = 0x5B8;
pub const MODIFIER_ACCUMULATOR_ARRAY_OFFSET: usize = 0x00;
pub const MODIFIER_ACCUMULATOR_CAPACITY_OFFSET: usize = 0x08;
pub const MODIFIER_ACCUMULATOR_COUNT_OFFSET: usize = 0x0C;
pub const MODIFIER_ACCUMULATOR_ENTRY_STRIDE: usize = 0x10;
pub const MODIFIER_ACCUMULATOR_ENTRY_KEY_OFFSET: usize = 0x00;
pub const MODIFIER_ACCUMULATOR_ENTRY_VALUE_OFFSET: usize = 0x08;
pub const TACTIC_MODIFIER_KEY_OFFSET: usize = 0x174;
pub const COUNTRY_FACTOR_RVA: usize = 0x331D370;
pub const MODIFIER_METADATA_REGISTRY_PTR_RVA: usize = 0x33169C0;
pub const MODIFIER_METADATA_STRIDE: usize = 0x78;
pub const MODIFIER_METADATA_FLAGS_OFFSET: usize = 0x60;
pub const MODIFIER_METADATA_GATE_OFFSET: usize = 0x68;
pub const MODIFIER_METADATA_CLAMP_BIT: u32 = 1 << 4;

pub const FLAG_ACCUMULATOR_ENTRY_PRESENT: u16 = 1 << 0;
const KNOWN_ROW_FLAGS: u16 = FLAG_ACCUMULATOR_ENTRY_PRESENT;

pub const EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];

pub const CONSUMER_EVIDENCE_SHA256: [u8; 32] = [
    0xff, 0xa7, 0xe5, 0xb1, 0x4d, 0x1e, 0x26, 0xed, 0x2a, 0x16, 0xfc, 0x66, 0x9a, 0x3a, 0x72, 0x93,
    0xec, 0x4a, 0x01, 0xcd, 0x4a, 0x22, 0x16, 0xf9, 0xaa, 0x90, 0xff, 0x91, 0xf3, 0x74, 0x9d, 0x73,
];

pub const DEFINE_EVIDENCE_SHA256: [u8; 32] = [
    0x3c, 0x7e, 0x36, 0xd6, 0x39, 0xbe, 0xba, 0x72, 0x5d, 0x94, 0x85, 0x23, 0xe4, 0xc2, 0xa3, 0x25,
    0x6a, 0xe0, 0xc5, 0x96, 0xcc, 0x2e, 0x38, 0xb6, 0x67, 0x55, 0xf7, 0xab, 0x2b, 0x2f, 0x88, 0xcd,
];

pub const LOOKUP_EVIDENCE_SHA256: [u8; 32] = [
    0x04, 0x22, 0xc6, 0xcd, 0x87, 0x1a, 0xde, 0xdb, 0xba, 0xed, 0xce, 0x53, 0x63, 0xe3, 0x16, 0x3b,
    0xbe, 0x03, 0xdd, 0x2f, 0x8e, 0xd0, 0xd7, 0x98, 0x79, 0xf4, 0xbb, 0x77, 0xe7, 0x1a, 0xce, 0xf8,
];

pub const RUNTIME_OBSERVATION_SHA256: [u8; 32] = [
    0x91, 0xa5, 0xf2, 0xb7, 0x5e, 0x86, 0xf3, 0xa7, 0xb1, 0xc2, 0x99, 0xa1, 0xfa, 0x07, 0xa8, 0xfe,
    0xf9, 0x52, 0x02, 0xb3, 0x03, 0x70, 0xa3, 0xa1, 0x42, 0x1c, 0xa6, 0x43, 0x64, 0x9f, 0x4a, 0x00,
];

pub const TACTIC_KEY_CENSUS_SHA256: [u8; 32] = [
    0x92, 0x03, 0xff, 0x38, 0xde, 0x7c, 0x3d, 0x7f, 0x29, 0x04, 0x3d, 0x2d, 0x42, 0x8f, 0xea, 0x44,
    0xd6, 0x07, 0xc7, 0x31, 0x33, 0xe9, 0x95, 0x6f, 0x31, 0x7d, 0xa3, 0xb8, 0xc4, 0xa4, 0x7d, 0xa1,
];

pub const NONZERO_OVERRIDE_OBSERVATION_SHA256: [u8; 32] = [
    0xf4, 0xd7, 0x1c, 0x3e, 0x5a, 0x4c, 0x73, 0xbb, 0x1a, 0x52, 0x88, 0x76, 0x79, 0xc8, 0xce, 0xcc,
    0xbc, 0xd6, 0x0c, 0x87, 0x27, 0x29, 0xd6, 0x04, 0xbc, 0x10, 0xa0, 0xea, 0x55, 0xd3, 0x07, 0x3c,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModifierAccumulatorEntry {
    pub key: u32,
    pub value_raw: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CountryPreferredTacticWeightRow {
    pub tag_db_id: u32,
    pub tactic_database_id: i32,
    pub tactic_id: String,
    pub modifier_key: u32,
    pub modifier_metadata_flags: u32,
    pub modifier_metadata_gate: i32,
    pub accumulator_entry_present: bool,
    pub accumulator_raw: i64,
    pub effective_bonus_raw: i64,
}

impl CountryPreferredTacticWeightRow {
    pub fn multiplier_numerator_raw(&self) -> Result<i64, String> {
        FIXED_SCALE
            .checked_add(self.effective_bonus_raw)
            .ok_or_else(|| "PTW1 multiplier numerator overflow".into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CountryPreferredTacticWeightSnapshot {
    pub read_time_us: u64,
    pub observed_country_count: u32,
    pub status_flags: u32,
    pub source_sequence: u64,
    pub country_factor_raw: i64,
    pub rows: Vec<CountryPreferredTacticWeightRow>,
}

fn take<const N: usize>(payload: &[u8], at: &mut usize) -> Result<[u8; N], String> {
    let end = at.checked_add(N).ok_or("PTW1 cursor overflow")?;
    let bytes = payload.get(*at..end).ok_or("PTW1 truncated")?;
    *at = end;
    Ok(bytes.try_into().expect("fixed-size slice"))
}

pub fn resolve_sorted_accumulator(
    entries: &[ModifierAccumulatorEntry],
    modifier_key: u32,
) -> Result<(i64, bool), String> {
    if modifier_key == 0 {
        return Err("PTW1 zero modifier key".into());
    }
    if entries.windows(2).any(|pair| pair[0].key >= pair[1].key) {
        return Err("PTW1 accumulator keys are not strictly ascending".into());
    }
    match entries.binary_search_by_key(&modifier_key, |entry| entry.key) {
        Ok(index) => Ok((entries[index].value_raw, true)),
        Err(_) => Ok((0, false)),
    }
}

pub fn encode_payload(snapshot: &CountryPreferredTacticWeightSnapshot) -> Result<Vec<u8>, String> {
    validate_snapshot(snapshot)?;
    let row_count = u32::try_from(snapshot.rows.len()).map_err(|_| "PTW1 too many rows")?;
    let mut out = Vec::with_capacity(HEADER_LEN + snapshot.rows.len() * 48);
    out.extend_from_slice(&snapshot.read_time_us.to_le_bytes());
    out.extend_from_slice(&snapshot.observed_country_count.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&snapshot.status_flags.to_le_bytes());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&snapshot.source_sequence.to_le_bytes());
    out.extend_from_slice(&(FIXED_SCALE as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&snapshot.country_factor_raw.to_le_bytes());
    out.extend_from_slice(&EXECUTABLE_SHA256);
    out.extend_from_slice(&CONSUMER_EVIDENCE_SHA256);
    out.extend_from_slice(&DEFINE_EVIDENCE_SHA256);
    out.extend_from_slice(&LOOKUP_EVIDENCE_SHA256);
    out.extend_from_slice(&RUNTIME_OBSERVATION_SHA256);
    out.extend_from_slice(&TACTIC_KEY_CENSUS_SHA256);
    out.extend_from_slice(&NONZERO_OVERRIDE_OBSERVATION_SHA256);
    debug_assert_eq!(out.len(), HEADER_LEN);
    for row in &snapshot.rows {
        let token = row.tactic_id.as_bytes();
        let flags = if row.accumulator_entry_present {
            FLAG_ACCUMULATOR_ENTRY_PRESENT
        } else {
            0
        };
        out.extend_from_slice(&row.tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.tactic_database_id.to_le_bytes());
        out.extend_from_slice(&row.modifier_key.to_le_bytes());
        out.extend_from_slice(&row.modifier_metadata_flags.to_le_bytes());
        out.extend_from_slice(&row.modifier_metadata_gate.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(token.len() as u16).to_le_bytes());
        out.extend_from_slice(&row.accumulator_raw.to_le_bytes());
        out.extend_from_slice(&row.effective_bonus_raw.to_le_bytes());
        out.extend_from_slice(token);
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<CountryPreferredTacticWeightSnapshot, String> {
    if payload.len() < HEADER_LEN {
        return Err("PTW1 header truncated".into());
    }
    let mut at = 0usize;
    let read_time_us = u64::from_le_bytes(take(payload, &mut at)?);
    let observed_country_count = u32::from_le_bytes(take(payload, &mut at)?);
    let row_count = u32::from_le_bytes(take(payload, &mut at)?) as usize;
    let status_flags = u32::from_le_bytes(take(payload, &mut at)?);
    if take::<4>(payload, &mut at)? != MAGIC
        || u16::from_le_bytes(take(payload, &mut at)?) != VERSION
        || u16::from_le_bytes(take(payload, &mut at)?) as usize != HEADER_LEN
    {
        return Err("PTW1 magic/version/header mismatch".into());
    }
    let source_sequence = u64::from_le_bytes(take(payload, &mut at)?);
    if u32::from_le_bytes(take(payload, &mut at)?) != FIXED_SCALE as u32
        || u32::from_le_bytes(take(payload, &mut at)?) != 0
    {
        return Err("PTW1 scale/reserved mismatch".into());
    }
    let country_factor_raw = i64::from_le_bytes(take(payload, &mut at)?);
    if take::<32>(payload, &mut at)? != EXECUTABLE_SHA256
        || take::<32>(payload, &mut at)? != CONSUMER_EVIDENCE_SHA256
        || take::<32>(payload, &mut at)? != DEFINE_EVIDENCE_SHA256
        || take::<32>(payload, &mut at)? != LOOKUP_EVIDENCE_SHA256
        || take::<32>(payload, &mut at)? != RUNTIME_OBSERVATION_SHA256
        || take::<32>(payload, &mut at)? != TACTIC_KEY_CENSUS_SHA256
        || take::<32>(payload, &mut at)? != NONZERO_OVERRIDE_OBSERVATION_SHA256
    {
        return Err("PTW1 authority receipt mismatch".into());
    }
    if row_count > MAX_COUNTRIES || row_count > observed_country_count as usize {
        return Err("PTW1 row count outside observed roster".into());
    }
    if row_count > max_rows_in(payload.len() - at) {
        return Err("PTW1 row count exceeds the payload's capacity".into());
    }
    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        let tag_db_id = u32::from_le_bytes(take(payload, &mut at)?);
        let tactic_database_id = i32::from_le_bytes(take(payload, &mut at)?);
        let modifier_key = u32::from_le_bytes(take(payload, &mut at)?);
        let modifier_metadata_flags = u32::from_le_bytes(take(payload, &mut at)?);
        let modifier_metadata_gate = i32::from_le_bytes(take(payload, &mut at)?);
        let flags = u16::from_le_bytes(take(payload, &mut at)?);
        let token_len = u16::from_le_bytes(take(payload, &mut at)?) as usize;
        let accumulator_raw = i64::from_le_bytes(take(payload, &mut at)?);
        let effective_bonus_raw = i64::from_le_bytes(take(payload, &mut at)?);
        if flags & !KNOWN_ROW_FLAGS != 0 || token_len == 0 || token_len > MAX_TOKEN_BYTES {
            return Err("PTW1 row flags/token length invalid".into());
        }
        let end = at
            .checked_add(token_len)
            .ok_or("PTW1 token cursor overflow")?;
        let tactic_id = std::str::from_utf8(payload.get(at..end).ok_or("PTW1 token truncated")?)
            .map_err(|_| "PTW1 tactic token is not UTF-8")?
            .to_owned();
        at = end;
        rows.push(CountryPreferredTacticWeightRow {
            tag_db_id,
            tactic_database_id,
            tactic_id,
            modifier_key,
            modifier_metadata_flags,
            modifier_metadata_gate,
            accumulator_entry_present: flags & FLAG_ACCUMULATOR_ENTRY_PRESENT != 0,
            accumulator_raw,
            effective_bonus_raw,
        });
    }
    if at != payload.len() {
        return Err("PTW1 trailing bytes".into());
    }
    let snapshot = CountryPreferredTacticWeightSnapshot {
        read_time_us,
        observed_country_count,
        status_flags,
        source_sequence,
        country_factor_raw,
        rows,
    };
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub fn validate_snapshot(snapshot: &CountryPreferredTacticWeightSnapshot) -> Result<(), String> {
    let failed = crate::compact_status_is_failure(snapshot.status_flags);
    if snapshot.observed_country_count as usize > MAX_COUNTRIES
        || snapshot.rows.len() > snapshot.observed_country_count as usize
    {
        return Err("PTW1 invalid observed roster count".into());
    }
    if failed {
        if !snapshot.rows.is_empty() {
            return Err("PTW1 failed snapshot contains selected rows".into());
        }
        return Ok(());
    }
    if snapshot.observed_country_count == 0 || snapshot.source_sequence == 0 {
        return Err("PTW1 clean snapshot lacks roster/source sequence".into());
    }
    let mut previous_tag = 0u32;
    let mut tags = BTreeSet::new();
    let mut by_id: BTreeMap<i32, &str> = BTreeMap::new();
    let mut by_token: BTreeMap<&str, i32> = BTreeMap::new();
    for row in &snapshot.rows {
        if row.tag_db_id == 0
            || row.tag_db_id <= previous_tag
            || !tags.insert(row.tag_db_id)
            || row.tactic_database_id <= 0
            || row.modifier_key == 0
            || row.tactic_id.is_empty()
            || row.tactic_id.len() > MAX_TOKEN_BYTES
            || row.tactic_id.chars().any(char::is_control)
        {
            return Err("PTW1 row identity/order invariant failed".into());
        }
        if row.modifier_metadata_gate != 0
            || row.modifier_metadata_flags & MODIFIER_METADATA_CLAMP_BIT != 0
        {
            return Err("PTW1 modifier key requires gated or clamped getter semantics".into());
        }
        if snapshot.country_factor_raw.checked_add(row.accumulator_raw)
            != Some(row.effective_bonus_raw)
            || (!row.accumulator_entry_present && row.accumulator_raw != 0)
            || row.multiplier_numerator_raw().is_err()
        {
            return Err("PTW1 derived fixed-five value mismatch/overflow".into());
        }
        if by_id
            .insert(row.tactic_database_id, row.tactic_id.as_str())
            .is_some_and(|old| old != row.tactic_id.as_str())
            || by_token
                .insert(row.tactic_id.as_str(), row.tactic_database_id)
                .is_some_and(|old| old != row.tactic_database_id)
        {
            return Err("PTW1 tactic id/token mapping is not injective".into());
        }
        previous_tag = row.tag_db_id;
    }
    Ok(())
}

pub fn validate_against_preferred_tactic(
    weights: &CountryPreferredTacticWeightSnapshot,
    preferred: &PreferredTacticSnapshot,
) -> Result<(), String> {
    validate_snapshot(weights)?;
    if weights.observed_country_count as usize != preferred.rows.len()
        || weights.source_sequence != preferred.source_sequence
    {
        return Err("PTW1/PTC1 roster or source-sequence mismatch".into());
    }
    let actual: BTreeMap<u32, &CountryPreferredTacticWeightRow> = weights
        .rows
        .iter()
        .map(|row| (row.tag_db_id, row))
        .collect();
    for selected in &preferred.rows {
        match (
            selected.tactic_database_id,
            selected.tactic_id.as_deref(),
            actual.get(&selected.tag_db_id),
        ) {
            (0, None, None) => {}
            (id, Some(token), Some(weight))
                if id == weight.tactic_database_id && token == weight.tactic_id.as_str() => {}
            _ => return Err("PTW1/PTC1 selected tactic identity mismatch".into()),
        }
    }
    Ok(())
}
