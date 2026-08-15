//! Exact a729 province-supply tooltip carrier.
//!
//! The base D44 rows remain unchanged. `PST1` is an additive trailer carrying
//! only pointer-free observations paired from the exact province producer and
//! its exact numeric-formatter sink.

use std::collections::BTreeSet;

pub mod domain44_contract;

#[cfg(windows)]
pub mod a729_tooltip_hook;
#[cfg(all(windows, target_arch = "x86_64"))]
pub mod a729_attrition_hook;

pub const TRAILER_MAGIC: u32 = u32::from_le_bytes(*b"PST1");
pub const TRAILER_VERSION: u32 = 1;
pub const AUTHORITY_EXACT_A729_TOOLTIP: u32 = 0xA729_0044;
pub const FIXED_SCALE: u32 = 100_000;
pub const DISPLAY_DECIMALS: u32 = 2;
pub const TRAILER_HEADER_LEN: usize = 48;
pub const TRAILER_ROW_LEN: usize = 40;
// Both bounds are re-applied by the replay consumer when it reads these rows
// back, so they live in the crate both workspaces depend on rather than being
// written once here and once there.
pub const MAX_PROVINCE_ID: u32 = portable_admission::SUPPLY_MAX_PROVINCE_ID;
pub const MAX_OBSERVER_TAG_DB_ID: u32 = portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_ROWS: usize = 16_384;

pub const EXACT_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub const EXACT_A729_JOB_ID: &str =
    "20260721T064706Z-wave6-exact-supply-tooltip-producer-chai-82394c9c";
pub const EXACT_A729_JOB_JSON_SHA256: &str =
    "2fb7d61f19dc76ba05bfb2850b0d43ede16ca476f9aabd721f32dfc3dfd591da";
pub const EXACT_A729_MANIFEST_SHA256: &str =
    "3f346cfcaf37895c347002fcfbd0782bd15672342aa6ba3da56f2396016108fd";

/// Pointer-free, current-frame transient attrition component carrier. This is
/// separate from ATR1 because exact a729 persists only the cached total, not a
/// reason enum.
pub const ATTRITION_COMPONENT_TRAILER_MAGIC: u32 = u32::from_le_bytes(*b"ARC1");
pub const ATTRITION_COMPONENT_TRAILER_VERSION: u32 = 1;
pub const ATTRITION_COMPONENT_HEADER_LEN: usize = 40;
pub const ATTRITION_COMPONENT_ROW_LEN: usize = 176;
pub const ATTRITION_COMPONENT_MAX_ROWS: usize = 16_384;
pub const ATTRITION_REFRESH_CALLER_RVA: u64 = 0x00C7_9598;
pub const ATTRITION_COMPONENT_FLAG_DELETED: u32 = 1;
pub const ATTRITION_COMPONENT_FLAG_OWNER: u32 = 1 << 1;
pub const ATTRITION_COMPONENT_FLAG_COMPLETE_EXACT: u32 = 1 << 2;
pub const ATTRITION_COMPONENT_FLAG_CACHED_TOTAL_JOIN_EXACT: u32 = 1 << 3;
pub const ATTRITION_COMPONENT_FLAG_REASONS_TRANSIENT: u32 = 1 << 4;
pub const ATTRITION_COMPONENT_LIVE_FLAGS: u32 = ATTRITION_COMPONENT_FLAG_OWNER
    | ATTRITION_COMPONENT_FLAG_COMPLETE_EXACT
    | ATTRITION_COMPONENT_FLAG_CACHED_TOTAL_JOIN_EXACT
    | ATTRITION_COMPONENT_FLAG_REASONS_TRANSIENT;
pub const ATTRITION_COMPONENT_OBSERVATION_MASK: u64 = (1 << 10) - 1;
pub const ATTRITION_COMPONENT_FUNCTION_JOB_ID: &str =
    "20260721T075907Z-wave6-attrition-reason-branch-map-final-7b436fbc";
pub const ATTRITION_COMPONENT_FUNCTION_MANIFEST_SHA256: &str =
    "179542b926915626f37ad2642405fbacd8c688c45ea7be087815f52575748767";
pub const ATTRITION_COMPONENT_DECOMPILE_SHA256: &str =
    "495ace70251ace045f6c1e713492cd4a538da476906ba1b5e77ce81c2977db23";
pub const ATTRITION_COMPONENT_INSTRUCTIONS_SHA256: &str =
    "3d04cf46d88a71e0bcbc3086a6f0837bd30bd27693997b3df84664eb2fdd1c4f";
pub const ATTRITION_COMPONENT_PCODE_SHA256: &str =
    "6eaabb053f04833d0443b8527756be10f63c0289ce6b574fb4a23d59058ceb9a";
pub const ATTRITION_COMPONENT_LABEL_JOB_ID: &str =
    "20260721T080247Z-wave6-attrition-reason-label-xrefs-final-02cddf6d";
pub const ATTRITION_COMPONENT_LABEL_ARTIFACT_SHA256: &str =
    "592953e14186bf0d843056892f753a939e898035565155bcd8dbc4ef069bb07c";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExactDivisionAttritionComponentRow {
    pub raw_division_id: u32,
    pub province_id: u32,
    pub owner_tag_db_id: u32,
    pub flags: u32,
    pub observation_mask: u64,
    pub upkeep_total_raw: i64,
    pub terrain_raw: i64,
    pub equipment_raw: i64,
    pub state_raw: i64,
    pub supply_raw: i64,
    pub state_modifiers_raw: i64,
    pub weather_modifiers_raw: i64,
    pub derived_terms_raw: i64,
    pub training_raw: i64,
    pub border_war_raw: i64,
    pub pre_multiplier_total_raw: i64,
    pub final_multiplier_raw: i64,
    pub final_total_raw: i64,
    pub cached_total_raw: i64,
    pub tick: i64,
    pub sequence: u64,
    pub refresh_caller_rva: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactDivisionAttritionComponentBatch {
    pub hook_epoch: u64,
    pub drain_sequence: u64,
    pub dropped_rows: u32,
    pub rows: Vec<ExactDivisionAttritionComponentRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExactProvinceSupplyTooltipRow {
    pub province_id: u32,
    pub observer_tag_db_id: u32,
    pub remaining_supply_raw: i64,
    pub fixed_scale: u32,
    pub displayed_decimals: u32,
    pub tick: i64,
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactProvinceSupplyTooltipBatch {
    pub hook_epoch: u64,
    pub drain_sequence: u64,
    pub dropped_rows: u32,
    pub rows: Vec<ExactProvinceSupplyTooltipRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    Truncated,
    Invalid(&'static str),
    TrailingBytes,
}

pub fn validate_row(row: &ExactProvinceSupplyTooltipRow) -> Result<(), CodecError> {
    if row.province_id == 0 || row.province_id > MAX_PROVINCE_ID {
        return Err(CodecError::Invalid("province_id"));
    }
    if row.observer_tag_db_id == 0 || row.observer_tag_db_id > MAX_OBSERVER_TAG_DB_ID {
        return Err(CodecError::Invalid("observer_tag_db_id"));
    }
    if row.fixed_scale != FIXED_SCALE {
        return Err(CodecError::Invalid("fixed_scale"));
    }
    if row.displayed_decimals != DISPLAY_DECIMALS {
        return Err(CodecError::Invalid("displayed_decimals"));
    }
    if row.tick < 0 || row.sequence == 0 {
        return Err(CodecError::Invalid("tick_or_sequence"));
    }
    Ok(())
}

pub fn encode_trailer(batch: &ExactProvinceSupplyTooltipBatch) -> Result<Vec<u8>, CodecError> {
    if batch.hook_epoch == 0 || batch.drain_sequence == 0 || batch.rows.len() > MAX_ROWS {
        return Err(CodecError::Invalid("batch_header"));
    }
    let mut out = Vec::with_capacity(TRAILER_HEADER_LEN + batch.rows.len() * TRAILER_ROW_LEN);
    put_u32(&mut out, TRAILER_MAGIC);
    put_u32(&mut out, TRAILER_VERSION);
    put_u32(&mut out, AUTHORITY_EXACT_A729_TOOLTIP);
    put_u32(&mut out, 0);
    put_u64(&mut out, batch.hook_epoch);
    put_u64(&mut out, batch.drain_sequence);
    put_u32(&mut out, batch.rows.len() as u32);
    put_u32(&mut out, TRAILER_ROW_LEN as u32);
    put_u32(&mut out, batch.dropped_rows);
    put_u32(&mut out, 0);
    for row in &batch.rows {
        validate_row(row)?;
        put_u32(&mut out, row.province_id);
        put_u32(&mut out, row.observer_tag_db_id);
        put_i64(&mut out, row.remaining_supply_raw);
        put_u32(&mut out, row.fixed_scale);
        put_u32(&mut out, row.displayed_decimals);
        put_i64(&mut out, row.tick);
        put_u64(&mut out, row.sequence);
    }
    Ok(out)
}

pub fn decode_trailer(bytes: &[u8]) -> Result<ExactProvinceSupplyTooltipBatch, CodecError> {
    if bytes.len() < TRAILER_HEADER_LEN {
        return Err(CodecError::Truncated);
    }
    if get_u32(bytes, 0)? != TRAILER_MAGIC
        || get_u32(bytes, 4)? != TRAILER_VERSION
        || get_u32(bytes, 8)? != AUTHORITY_EXACT_A729_TOOLTIP
        || get_u32(bytes, 12)? != 0
        || get_u32(bytes, 36)? as usize != TRAILER_ROW_LEN
        || get_u32(bytes, 44)? != 0
    {
        return Err(CodecError::Invalid("trailer_header"));
    }
    let hook_epoch = get_u64(bytes, 16)?;
    let drain_sequence = get_u64(bytes, 24)?;
    let row_count = get_u32(bytes, 32)? as usize;
    let dropped_rows = get_u32(bytes, 40)?;
    if hook_epoch == 0 || drain_sequence == 0 || row_count > MAX_ROWS {
        return Err(CodecError::Invalid("batch_header"));
    }
    let expected = TRAILER_HEADER_LEN
        .checked_add(row_count.checked_mul(TRAILER_ROW_LEN).ok_or(CodecError::Invalid("size"))?)
        .ok_or(CodecError::Invalid("size"))?;
    if bytes.len() < expected {
        return Err(CodecError::Truncated);
    }
    if bytes.len() != expected {
        return Err(CodecError::TrailingBytes);
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut cursor = TRAILER_HEADER_LEN;
    let mut last_sequence = 0;
    for _ in 0..row_count {
        let row = ExactProvinceSupplyTooltipRow {
            province_id: get_u32(bytes, cursor)?,
            observer_tag_db_id: get_u32(bytes, cursor + 4)?,
            remaining_supply_raw: get_i64(bytes, cursor + 8)?,
            fixed_scale: get_u32(bytes, cursor + 16)?,
            displayed_decimals: get_u32(bytes, cursor + 20)?,
            tick: get_i64(bytes, cursor + 24)?,
            sequence: get_u64(bytes, cursor + 32)?,
        };
        validate_row(&row)?;
        if row.sequence <= last_sequence {
            return Err(CodecError::Invalid("row_sequence"));
        }
        last_sequence = row.sequence;
        rows.push(row);
        cursor += TRAILER_ROW_LEN;
    }
    Ok(ExactProvinceSupplyTooltipBatch {
        hook_epoch,
        drain_sequence,
        dropped_rows,
        rows,
    })
}

pub fn validate_attrition_component_row(
    row: &ExactDivisionAttritionComponentRow,
) -> Result<(), CodecError> {
    if row.raw_division_id == 0 || row.raw_division_id > i32::MAX as u32 {
        return Err(CodecError::Invalid("attrition_division_id"));
    }
    if row.flags != ATTRITION_COMPONENT_LIVE_FLAGS
        || row.province_id == 0
        || row.province_id > MAX_PROVINCE_ID
        || row.owner_tag_db_id == 0
        || row.owner_tag_db_id > MAX_OBSERVER_TAG_DB_ID
        || row.observation_mask != ATTRITION_COMPONENT_OBSERVATION_MASK
        || row.tick < 0
        || row.sequence == 0
        || row.refresh_caller_rva != ATTRITION_REFRESH_CALLER_RVA
    {
        return Err(CodecError::Invalid("attrition_identity_or_completeness"));
    }
    let recomposed = [
        row.upkeep_total_raw,
        row.terrain_raw,
        row.equipment_raw,
        row.state_raw,
        row.supply_raw,
        row.state_modifiers_raw,
        row.weather_modifiers_raw,
        row.derived_terms_raw,
        row.training_raw,
        row.border_war_raw,
    ]
    .into_iter()
    .fold(0i128, |sum, value| sum + i128::from(value));
    if recomposed != i128::from(row.pre_multiplier_total_raw) {
        return Err(CodecError::Invalid("attrition_component_sum"));
    }
    let final_total = i128::from(row.pre_multiplier_total_raw)
        .checked_mul(i128::from(row.final_multiplier_raw))
        .and_then(|value| value.checked_div(i128::from(FIXED_SCALE)))
        .ok_or(CodecError::Invalid("attrition_final_law"))?;
    if final_total != i128::from(row.final_total_raw)
        || row.final_total_raw != row.cached_total_raw
    {
        return Err(CodecError::Invalid("attrition_final_join"));
    }
    Ok(())
}

pub fn encode_attrition_component_trailer(
    batch: &ExactDivisionAttritionComponentBatch,
) -> Result<Vec<u8>, CodecError> {
    if batch.hook_epoch == 0
        || batch.drain_sequence == 0
        || batch.rows.len() > ATTRITION_COMPONENT_MAX_ROWS
    {
        return Err(CodecError::Invalid("attrition_batch_header"));
    }
    let size = ATTRITION_COMPONENT_HEADER_LEN
        .checked_add(
            batch
                .rows
                .len()
                .checked_mul(ATTRITION_COMPONENT_ROW_LEN)
                .ok_or(CodecError::Invalid("attrition_size"))?,
        )
        .ok_or(CodecError::Invalid("attrition_size"))?;
    let mut out = Vec::with_capacity(size);
    put_u32(&mut out, ATTRITION_COMPONENT_TRAILER_MAGIC);
    put_u32(&mut out, ATTRITION_COMPONENT_TRAILER_VERSION);
    put_u32(&mut out, ATTRITION_COMPONENT_ROW_LEN as u32);
    put_u32(&mut out, batch.rows.len() as u32);
    put_u64(&mut out, batch.hook_epoch);
    put_u64(&mut out, batch.drain_sequence);
    put_u32(&mut out, batch.dropped_rows);
    put_u32(&mut out, 0);
    let mut prior_id = 0;
    let mut seen_sequences = BTreeSet::new();
    for row in &batch.rows {
        validate_attrition_component_row(row)?;
        if row.raw_division_id <= prior_id {
            return Err(CodecError::Invalid("attrition_row_order"));
        }
        if !seen_sequences.insert(row.sequence) {
            return Err(CodecError::Invalid("attrition_sequence"));
        }
        prior_id = row.raw_division_id;
        put_u32(&mut out, row.raw_division_id);
        put_u32(&mut out, row.province_id);
        put_u32(&mut out, row.owner_tag_db_id);
        put_u32(&mut out, row.flags);
        put_u64(&mut out, row.observation_mask);
        put_u64(&mut out, 0);
        for value in [
            row.upkeep_total_raw,
            row.terrain_raw,
            row.equipment_raw,
            row.state_raw,
            row.supply_raw,
            row.state_modifiers_raw,
            row.weather_modifiers_raw,
            row.derived_terms_raw,
            row.training_raw,
            row.border_war_raw,
            row.pre_multiplier_total_raw,
            row.final_multiplier_raw,
            row.final_total_raw,
            row.cached_total_raw,
            row.tick,
        ] {
            put_i64(&mut out, value);
        }
        put_u64(&mut out, row.sequence);
        put_u64(&mut out, row.refresh_caller_rva);
        put_u64(&mut out, 0);
    }
    Ok(out)
}

pub fn decode_attrition_component_trailer_prefix(
    bytes: &[u8],
) -> Result<(ExactDivisionAttritionComponentBatch, usize), CodecError> {
    if bytes.len() < ATTRITION_COMPONENT_HEADER_LEN {
        return Err(CodecError::Truncated);
    }
    if get_u32(bytes, 0)? != ATTRITION_COMPONENT_TRAILER_MAGIC
        || get_u32(bytes, 4)? != ATTRITION_COMPONENT_TRAILER_VERSION
        || get_u32(bytes, 8)? as usize != ATTRITION_COMPONENT_ROW_LEN
        || get_u32(bytes, 36)? != 0
    {
        return Err(CodecError::Invalid("attrition_trailer_header"));
    }
    let row_count = get_u32(bytes, 12)? as usize;
    let hook_epoch = get_u64(bytes, 16)?;
    let drain_sequence = get_u64(bytes, 24)?;
    let dropped_rows = get_u32(bytes, 32)?;
    if hook_epoch == 0
        || drain_sequence == 0
        || row_count > ATTRITION_COMPONENT_MAX_ROWS
    {
        return Err(CodecError::Invalid("attrition_batch_header"));
    }
    let expected = ATTRITION_COMPONENT_HEADER_LEN
        .checked_add(
            row_count
                .checked_mul(ATTRITION_COMPONENT_ROW_LEN)
                .ok_or(CodecError::Invalid("attrition_size"))?,
        )
        .ok_or(CodecError::Invalid("attrition_size"))?;
    if bytes.len() < expected {
        return Err(CodecError::Truncated);
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut cursor = ATTRITION_COMPONENT_HEADER_LEN;
    let mut prior_id = 0;
    let mut seen_sequences = BTreeSet::new();
    for _ in 0..row_count {
        if get_u64(bytes, cursor + 24)? != 0 || get_u64(bytes, cursor + 168)? != 0 {
            return Err(CodecError::Invalid("attrition_row_reserved"));
        }
        let row = ExactDivisionAttritionComponentRow {
            raw_division_id: get_u32(bytes, cursor)?,
            province_id: get_u32(bytes, cursor + 4)?,
            owner_tag_db_id: get_u32(bytes, cursor + 8)?,
            flags: get_u32(bytes, cursor + 12)?,
            observation_mask: get_u64(bytes, cursor + 16)?,
            upkeep_total_raw: get_i64(bytes, cursor + 32)?,
            terrain_raw: get_i64(bytes, cursor + 40)?,
            equipment_raw: get_i64(bytes, cursor + 48)?,
            state_raw: get_i64(bytes, cursor + 56)?,
            supply_raw: get_i64(bytes, cursor + 64)?,
            state_modifiers_raw: get_i64(bytes, cursor + 72)?,
            weather_modifiers_raw: get_i64(bytes, cursor + 80)?,
            derived_terms_raw: get_i64(bytes, cursor + 88)?,
            training_raw: get_i64(bytes, cursor + 96)?,
            border_war_raw: get_i64(bytes, cursor + 104)?,
            pre_multiplier_total_raw: get_i64(bytes, cursor + 112)?,
            final_multiplier_raw: get_i64(bytes, cursor + 120)?,
            final_total_raw: get_i64(bytes, cursor + 128)?,
            cached_total_raw: get_i64(bytes, cursor + 136)?,
            tick: get_i64(bytes, cursor + 144)?,
            sequence: get_u64(bytes, cursor + 152)?,
            refresh_caller_rva: get_u64(bytes, cursor + 160)?,
        };
        validate_attrition_component_row(&row)?;
        if row.raw_division_id <= prior_id {
            return Err(CodecError::Invalid("attrition_row_order"));
        }
        if !seen_sequences.insert(row.sequence) {
            return Err(CodecError::Invalid("attrition_sequence"));
        }
        prior_id = row.raw_division_id;
        rows.push(row);
        cursor += ATTRITION_COMPONENT_ROW_LEN;
    }
    Ok((
        ExactDivisionAttritionComponentBatch {
            hook_epoch,
            drain_sequence,
            dropped_rows,
            rows,
        },
        expected,
    ))
}

pub fn decode_attrition_component_trailer(
    bytes: &[u8],
) -> Result<ExactDivisionAttritionComponentBatch, CodecError> {
    let (batch, consumed) = decode_attrition_component_trailer_prefix(bytes)?;
    if consumed != bytes.len() {
        return Err(CodecError::TrailingBytes);
    }
    Ok(batch)
}

fn put_u32(out: &mut Vec<u8>, value: u32) { out.extend_from_slice(&value.to_le_bytes()); }
fn put_u64(out: &mut Vec<u8>, value: u64) { out.extend_from_slice(&value.to_le_bytes()); }
fn put_i64(out: &mut Vec<u8>, value: i64) { out.extend_from_slice(&value.to_le_bytes()); }
fn get_u32(bytes: &[u8], at: usize) -> Result<u32, CodecError> {
    Ok(u32::from_le_bytes(bytes.get(at..at + 4).ok_or(CodecError::Truncated)?.try_into().unwrap()))
}
fn get_u64(bytes: &[u8], at: usize) -> Result<u64, CodecError> {
    Ok(u64::from_le_bytes(bytes.get(at..at + 8).ok_or(CodecError::Truncated)?.try_into().unwrap()))
}
fn get_i64(bytes: &[u8], at: usize) -> Result<i64, CodecError> {
    Ok(i64::from_le_bytes(bytes.get(at..at + 8).ok_or(CodecError::Truncated)?.try_into().unwrap()))
}

#[cfg(test)]
mod attrition_component_tests {
    use super::*;

    fn row(raw_division_id: u32, sequence: u64) -> ExactDivisionAttritionComponentRow {
        ExactDivisionAttritionComponentRow {
            raw_division_id,
            province_id: raw_division_id,
            owner_tag_db_id: 1,
            flags: ATTRITION_COMPONENT_LIVE_FLAGS,
            observation_mask: ATTRITION_COMPONENT_OBSERVATION_MASK,
            upkeep_total_raw: 100,
            terrain_raw: 0,
            equipment_raw: 0,
            state_raw: 0,
            supply_raw: 0,
            state_modifiers_raw: 0,
            weather_modifiers_raw: 0,
            derived_terms_raw: 0,
            training_raw: 0,
            border_war_raw: 0,
            pre_multiplier_total_raw: 100,
            final_multiplier_raw: FIXED_SCALE as i64,
            final_total_raw: 100,
            cached_total_raw: 100,
            tick: 1,
            sequence,
            refresh_caller_rva: ATTRITION_REFRESH_CALLER_RVA,
        }
    }

    #[test]
    fn arc1_roundtrip_preserves_canonical_ids_and_causal_sequences() {
        let batch = ExactDivisionAttritionComponentBatch {
            hook_epoch: 1,
            drain_sequence: 1,
            dropped_rows: 0,
            // Causal sequence is intentionally not division-id order.
            rows: vec![row(1, 20), row(2, 10)],
        };
        let encoded = encode_attrition_component_trailer(&batch).expect("encode ARC1");
        let decoded = decode_attrition_component_trailer(&encoded).expect("decode ARC1");
        assert_eq!(decoded, batch);
    }

    #[test]
    fn arc1_rejects_tombstone_semantics() {
        let mut invalid = row(1, 1);
        invalid.flags = ATTRITION_COMPONENT_FLAG_DELETED;
        assert_eq!(
            validate_attrition_component_row(&invalid),
            Err(CodecError::Invalid("attrition_identity_or_completeness"))
        );
    }
}

/// EVERY HELD-FREEZE WINDOW IN THIS CRATE, READ.
///
/// This crate takes `inline_detour::freeze_process()` directly, which stops
/// every other thread of the game. Until the guard drops, an allocation or a
/// lock here is a HANG and not a slow path: the lock may be held by a thread
/// that can no longer run, and the collector declares no `#[global_allocator]`,
/// so Rust's allocator is `HeapAlloc` on the PROCESS DEFAULT HEAP -- the same
/// heap the game's own threads reach through ordinary Win32 calls.
///
/// `inline_detour` has enforced this on its own windows for a long time. This
/// crate did not, and the audit found real defects the first time it ran.
///
/// What production change would make this fail? Building an error string,
/// collecting an iterator, taking a mutex, or growing a vector between the
/// freeze and its drop -- at any call depth inside this crate, in any file
/// under `src/`, including one added after this test was written.
#[cfg(test)]
mod frozen_window_audit_tests {
    use freeze_window_audit::{audit, crate_sources, fingerprint};

    /// How many findings this crate still carries, and WHICH ones.
    ///
    /// THE 19 ALONE IS NOT A RATCHET: fixing one finding while adding
    /// another leaves it unchanged. The fingerprint is over the sorted findings,
    /// so any change to which ones exist fails here even when there are the same
    /// number of them. Both may only move when a human decides they should.
    const OPEN_FINDINGS: usize = 19;
    const OPEN_4347730519296784527u64: u64 = 4347730519296784527u64;

    #[test]
    fn nothing_in_a_held_freeze_window_allocates_or_blocks() {
        // THE DIRECTORY IS THE FILE LIST. A hand-written one cannot see a
        // module added later, and an unscanned module is an unscanned window.
        let sources = crate_sources(std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        )));
        assert!(
            sources.len() >= 2,
            "the scan found only {} source file(s); it is not reaching the crate",
            sources.len()
        );
        let findings = audit(&sources, &["suspend_all_other_threads()"]);
        let listed = findings
            .iter()
            .map(|finding| format!("  {finding}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            (findings.len(), fingerprint(&findings)),
            (OPEN_FINDINGS, OPEN_4347730519296784527u64),
            "the held-freeze windows of this crate changed. If you FIXED one, \
             lower `OPEN_FINDINGS` and update `OPEN_4347730519296784527u64`. If this grew, \
             the change put new work inside a window where the whole game is \
             stopped:\n{listed}"
        );
    }
}
