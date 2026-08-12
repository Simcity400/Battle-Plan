use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 42;
pub const DOMAIN_NAME: &str = "air_combat_losses_live";
pub const HEADER_LEN: usize = 48;
// This row is 76 bytes: the fixed u32/i32 fields are followed by three i64
// equipment amount fields plus the wave-3 per-wing cumulative scalars
// (air_accidents u32 @68, experience_raw i32 @72).
pub const ROW_LEN: usize = 76;
pub const STALE_ROW_LEN_64_REJECTION_GUARD: usize = 64;
pub const EQUIPMENT_AMOUNT_PER_PLANE: i64 = 100_000;

pub const STATUS_DELTA_DATA: u32 = 1 << 10;

pub const ROW_FLAG_LOSS: u32 = 1;
pub const ROW_FLAG_REINFORCEMENT: u32 = 1 << 1;
pub const ROW_FLAG_DELETED: u32 = 1 << 2;
pub const ROW_FLAG_MISSION_REGION: u32 = 1 << 3;
pub const ROW_FLAG_EQUIPMENT_SCALE_OK: u32 = 1 << 4;
/// air_accidents/experience_raw were measured for this row (absent != 0).
pub const ROW_FLAG_WING_SCALARS: u32 = 1 << 5;
/// First observation of this wing body: a state baseline row, not an event.
pub const ROW_FLAG_STATE_BASELINE: u32 = 1 << 6;
/// air_accidents or experience_raw changed vs the previous sample.
pub const ROW_FLAG_WING_SCALAR_CHANGE: u32 = 1 << 7;

/// Was `1_024`, a guess at the vanilla-plus-DLC country count (vanilla defines
/// 439 tags). ENGINE-STRUCTURAL now, by the same derivation `MAX_WING_ID` below
/// already uses: a country database id is admissible iff
/// `id < *(int32*)(db + 0x0C)` and that size is a signed `int`.
pub const MAX_OWNER_TAG_DB_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_WING_ID: u32 = i32::MAX as u32;
pub const MAX_PLANES: u32 = 10_000_000;
pub const MAX_MISSION_REGION_ID: u32 = 65_535;
// Wing experience is cumulative XP, fixed-x100000 (not a 0..1 ratio).
pub const MAX_EXPERIENCE_RAW: i32 = 1_000_000_000;
pub const REGION_STATS_TRAILER_MAGIC: u32 = 0x3152_3441;
pub const REGION_STATS_TRAILER_VERSION: u16 = 2;
pub const REGION_STATS_TRAILER_LEGACY_VERSION: u16 = 1;
pub const REGION_STATS_TRAILER_HEADER_LEN: usize = 32;
pub const REGION_STATS_ROW_LEN: usize = 40;
pub const REGION_DAMAGE_ROW_LEN: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirStrengthDeltaHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub observed_air_wing_rows: u32,
    pub observed_equipment_rows: u32,
    pub loss_rows: u32,
    pub reinforcement_rows: u32,
    pub deleted_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirStrengthDeltaRow {
    pub owner_tag_db_id: u32,
    pub wing_id: u32,
    pub generation: u64,
    pub deleted: bool,
    pub baseline: bool,
    pub scalar_change: bool,
    pub mission_mask: u32,
    pub mission_region: Option<u32>,
    pub old_count: u32,
    pub new_count: u32,
    pub count_delta: i32,
    pub old_equipment_amount_sum: i64,
    pub new_equipment_amount_sum: i64,
    pub equipment_amount_delta: i64,
    pub air_accidents: u32,
    pub experience_raw: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain42GateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

pub fn encode_payload(header: &AirStrengthDeltaHeader, rows: &[AirStrengthDeltaRow]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(HEADER_LEN.saturating_add(rows.len().saturating_mul(ROW_LEN)));
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&header.observed_air_wing_rows.to_le_bytes());
    payload.extend_from_slice(&header.observed_equipment_rows.to_le_bytes());
    payload.extend_from_slice(&header.loss_rows.to_le_bytes());
    payload.extend_from_slice(&header.reinforcement_rows.to_le_bytes());
    payload.extend_from_slice(&header.deleted_rows.to_le_bytes());
    payload.extend_from_slice(&(EQUIPMENT_AMOUNT_PER_PLANE as u32).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        let mut flags = ROW_FLAG_WING_SCALARS;
        if row.count_delta < 0 || row.equipment_amount_delta < 0 {
            flags |= ROW_FLAG_LOSS;
        }
        if row.count_delta > 0 || row.equipment_amount_delta > 0 {
            flags |= ROW_FLAG_REINFORCEMENT;
        }
        if row.deleted {
            flags |= ROW_FLAG_DELETED;
        }
        if row.baseline {
            flags |= ROW_FLAG_STATE_BASELINE;
        }
        if row.scalar_change {
            flags |= ROW_FLAG_WING_SCALAR_CHANGE;
        }
        if row.mission_region.is_some() {
            flags |= ROW_FLAG_MISSION_REGION;
        }
        if row.equipment_amount_delta == (row.count_delta as i64).saturating_mul(EQUIPMENT_AMOUNT_PER_PLANE) {
            flags |= ROW_FLAG_EQUIPMENT_SCALE_OK;
        }
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.wing_id.to_le_bytes());
        payload.extend_from_slice(&row.generation.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.mission_mask.to_le_bytes());
        payload.extend_from_slice(&row.mission_region.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.old_count.to_le_bytes());
        payload.extend_from_slice(&row.new_count.to_le_bytes());
        payload.extend_from_slice(&row.count_delta.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&row.old_equipment_amount_sum.to_le_bytes());
        payload.extend_from_slice(&row.new_equipment_amount_sum.to_le_bytes());
        payload.extend_from_slice(&row.equipment_amount_delta.to_le_bytes());
        payload.extend_from_slice(&row.air_accidents.to_le_bytes());
        payload.extend_from_slice(&row.experience_raw.to_le_bytes());
    }
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain42GateSummary {
    let mut summary = Domain42GateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live data payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let observed_equipment_rows = read_u32_le(payload, 24) as usize;
    let loss_rows = read_u32_le(payload, 28) as usize;
    let reinforcement_rows = read_u32_le(payload, 32) as usize;
    let deleted_rows = read_u32_le(payload, 36) as usize;
    let scale_raw = read_u32_le(payload, 40);
    let reserved0 = read_u32_le(payload, 44);
    summary.status_flags_or |= status_flags;

    if changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live changed_rows {changed_rows} != row_count {row_count}"
        ));
    }
    if loss_rows > row_count || reinforcement_rows > row_count || deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live counters exceed row_count {row_count}"
        ));
    }
    if scale_raw != EQUIPMENT_AMOUNT_PER_PLANE as u32 {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live scale_raw {scale_raw} != {}",
            EQUIPMENT_AMOUNT_PER_PLANE
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live header reserved field is nonzero"
        ));
    }
    let legacy_len = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if payload.len() < legacy_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live data payload is {} byte(s), expected at least {legacy_len}",
            payload.len()
        ));
        return summary;
    }

    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "air_strength_delta_rows", row_count);
    add_unit(
        &mut summary,
        "air_strength_observed_wing_rows",
        observed_air_wing_rows,
    );
    add_unit(
        &mut summary,
        "air_strength_observed_equipment_rows",
        observed_equipment_rows,
    );
    add_unit(&mut summary, "air_strength_loss_rows", loss_rows);
    add_unit(
        &mut summary,
        "air_strength_reinforcement_rows",
        reinforcement_rows,
    );
    add_unit(&mut summary, "air_strength_deleted_rows", deleted_rows);

    let mut cursor = HEADER_LEN;
    let mut seen_rows = BTreeSet::new();
    let mut decoded_loss_rows = 0usize;
    let mut decoded_reinforcement_rows = 0usize;
    let mut decoded_deleted_rows = 0usize;
    let mut mission_region_rows = 0usize;
    for row_index in 0..row_count {
        let owner_tag_db_id = read_u32_le(payload, cursor);
        let wing_id = read_u32_le(payload, cursor + 4);
        let generation = read_u64_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 16);
        let mission_mask = read_u32_le(payload, cursor + 20);
        let mission_region = read_u32_le(payload, cursor + 24);
        let old_count = read_u32_le(payload, cursor + 28);
        let new_count = read_u32_le(payload, cursor + 32);
        let count_delta = read_i32_le(payload, cursor + 36);
        let reserved1 = read_u32_le(payload, cursor + 40);
        let old_equipment_amount_sum = read_i64_le(payload, cursor + 44);
        let new_equipment_amount_sum = read_i64_le(payload, cursor + 52);
        let equipment_amount_delta = read_i64_le(payload, cursor + 60);
        let air_accidents = read_u32_le(payload, cursor + 68);
        let experience_raw = read_i32_le(payload, cursor + 72);
        cursor += ROW_LEN;

        let deleted = flags & ROW_FLAG_DELETED != 0;
        let loss = flags & ROW_FLAG_LOSS != 0;
        let reinforcement = flags & ROW_FLAG_REINFORCEMENT != 0;
        let baseline = flags & ROW_FLAG_STATE_BASELINE != 0;
        let wing_scalars = flags & ROW_FLAG_WING_SCALARS != 0;
        let scalar_change = flags & ROW_FLAG_WING_SCALAR_CHANGE != 0;
        if baseline && (loss || reinforcement || deleted || scalar_change) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} marks baseline with event flags"
            ));
        }
        if baseline
            && (count_delta != 0
                || old_count != new_count
                || equipment_amount_delta != 0
                || old_equipment_amount_sum != new_equipment_amount_sum)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live baseline row {row_index} has nonzero deltas"
            ));
        }
        if wing_scalars {
            if air_accidents > MAX_PLANES {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} has invalid air_accidents {air_accidents}"
                ));
            }
            if !(0..=MAX_EXPERIENCE_RAW).contains(&experience_raw) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} has invalid experience_raw {experience_raw}"
                ));
            }
        } else if air_accidents != 0 || experience_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has wing scalar values without measured flag"
            ));
        }
        if scalar_change && !wing_scalars {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} marks scalar change without measured scalars"
            ));
        }
        if owner_tag_db_id == 0 || owner_tag_db_id > MAX_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if wing_id == 0 || wing_id > MAX_WING_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid wing_id {wing_id}"
            ));
        } else if !seen_rows.insert((owner_tag_db_id, wing_id, generation, deleted, old_count, new_count)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} duplicates owner/wing/generation/counts"
            ));
        }
        if flags
            & !(ROW_FLAG_LOSS
                | ROW_FLAG_REINFORCEMENT
                | ROW_FLAG_DELETED
                | ROW_FLAG_MISSION_REGION
                | ROW_FLAG_EQUIPMENT_SCALE_OK
                | ROW_FLAG_WING_SCALARS
                | ROW_FLAG_STATE_BASELINE
                | ROW_FLAG_WING_SCALAR_CHANGE)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has unknown flags {flags}"
            ));
        }
        if !loss && !reinforcement && !deleted && !baseline && !scalar_change {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has no event flag"
            ));
        }
        if flags & ROW_FLAG_MISSION_REGION != 0 {
            mission_region_rows = mission_region_rows.saturating_add(1);
            if mission_region == 0 || mission_region > MAX_MISSION_REGION_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} has invalid mission_region {mission_region}"
                ));
            }
        } else if mission_region != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has mission_region without flag"
            ));
        }
        if old_count > MAX_PLANES || new_count > MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid counts {old_count}->{new_count}"
            ));
        }
        if count_delta as i64 != (new_count as i64) - (old_count as i64) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} count_delta {count_delta} does not match counts {old_count}->{new_count}"
            ));
        }
        if old_equipment_amount_sum < 0 || new_equipment_amount_sum < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has negative equipment amount sum"
            ));
        }
        if equipment_amount_delta != new_equipment_amount_sum.saturating_sub(old_equipment_amount_sum) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} equipment delta does not match equipment sums"
            ));
        }
        let expected_equipment_delta = (count_delta as i64).saturating_mul(EQUIPMENT_AMOUNT_PER_PLANE);
        if equipment_amount_delta == expected_equipment_delta {
            if flags & ROW_FLAG_EQUIPMENT_SCALE_OK == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} missing equipment scale flag"
                ));
            }
        } else if flags & ROW_FLAG_EQUIPMENT_SCALE_OK != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid equipment scale flag"
            ));
        }
        if loss != (count_delta < 0 || equipment_amount_delta < 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} loss flag does not match deltas"
            ));
        }
        if reinforcement != (count_delta > 0 || equipment_amount_delta > 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} reinforcement flag does not match deltas"
            ));
        }
        if deleted && (new_count != 0 || new_equipment_amount_sum != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live deleted row {row_index} has live new fields"
            ));
        }
        if reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} reserved field is nonzero"
            ));
        }
        if loss {
            decoded_loss_rows = decoded_loss_rows.saturating_add(1);
        }
        if reinforcement {
            decoded_reinforcement_rows = decoded_reinforcement_rows.saturating_add(1);
        }
        if deleted {
            decoded_deleted_rows = decoded_deleted_rows.saturating_add(1);
        }
        let _ = mission_mask;
    }
    if decoded_loss_rows != loss_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live loss_rows {loss_rows} != decoded {decoded_loss_rows}"
        ));
    }
    if decoded_reinforcement_rows != reinforcement_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live reinforcement_rows {reinforcement_rows} != decoded {decoded_reinforcement_rows}"
        ));
    }
    if decoded_deleted_rows != deleted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live deleted_rows {deleted_rows} != decoded {decoded_deleted_rows}"
        ));
    }
    add_unit(
        &mut summary,
        "air_strength_mission_region_rows",
        mission_region_rows,
    );
    if payload.len() != legacy_len {
        let trailer = legacy_len;
        if payload.len() < trailer.saturating_add(REGION_STATS_TRAILER_HEADER_LEN) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat regional stats trailer is truncated"
            ));
            return summary;
        }
        let magic = read_u32_le(payload, trailer);
        let version = read_u16_le(payload, trailer + 4);
        let header_len = read_u16_le(payload, trailer + 6) as usize;
        let region_rows = read_u32_le(payload, trailer + 8) as usize;
        let damage_rows = read_u32_le(payload, trailer + 12) as usize;
        let observed_region_objects = read_u32_le(payload, trailer + 16) as usize;
        let invalid_wing_rows = read_u32_le(payload, trailer + 20) as usize;
        let scale_mismatch_rows = read_u32_le(payload, trailer + 24) as usize;
        let first_invalid = read_u32_le(payload, trailer + 28);
        if magic != REGION_STATS_TRAILER_MAGIC
            || (version != REGION_STATS_TRAILER_VERSION
                && version != REGION_STATS_TRAILER_LEGACY_VERSION)
            || header_len != REGION_STATS_TRAILER_HEADER_LEN
        {
            summary.parse_errors.push(format!(
                "offset {offset}: invalid air combat regional stats trailer identity"
            ));
            return summary;
        }
        if version == REGION_STATS_TRAILER_LEGACY_VERSION
            && (invalid_wing_rows != 0 || scale_mismatch_rows != 0 || first_invalid != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat regional stats legacy reserved field is nonzero"
            ));
        }
        let first_invalid_reason = (first_invalid & 0xFFFF) as usize;
        let first_invalid_ordinal = (first_invalid >> 16) as usize;
        if version == REGION_STATS_TRAILER_VERSION
            && ((invalid_wing_rows == 0) != (first_invalid_reason == 0)
                || (7..10).contains(&first_invalid_reason)
                || first_invalid_reason > 32)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat invalid-wing diagnostic mismatch"
            ));
        }
        let expected_len = trailer
            .saturating_add(header_len)
            .saturating_add(region_rows.saturating_mul(REGION_STATS_ROW_LEN))
            .saturating_add(damage_rows.saturating_mul(REGION_DAMAGE_ROW_LEN));
        if payload.len() != expected_len {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat regional stats payload is {} byte(s), expected {expected_len}",
                payload.len()
            ));
            return summary;
        }
        add_unit(&mut summary, "air_region_combat_stat_rows", region_rows);
        add_unit(&mut summary, "air_region_damage_rows", damage_rows);
        add_unit(
            &mut summary,
            "air_region_combat_objects",
            observed_region_objects,
        );
        add_unit(
            &mut summary,
            "air_strength_invalid_wing_rows",
            invalid_wing_rows,
        );
        add_unit(
            &mut summary,
            "air_strength_scale_mismatch_rows",
            scale_mismatch_rows,
        );
        add_unit(
            &mut summary,
            "air_strength_first_invalid_ordinal",
            first_invalid_ordinal,
        );
        add_unit(
            &mut summary,
            "air_strength_first_invalid_reason",
            first_invalid_reason,
        );
    }
    summary
}

fn add_unit(summary: &mut Domain42GateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32_le(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().expect("i32 range"))
}

fn read_u16_le(payload: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(payload[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u64_le(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(payload[offset..offset + 8].try_into().expect("u64 range"))
}

fn read_i64_le(payload: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(payload[offset..offset + 8].try_into().expect("i64 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loss_row() -> AirStrengthDeltaRow {
        AirStrengthDeltaRow {
            owner_tag_db_id: 1,
            wing_id: 10,
            generation: 0,
            deleted: false,
            baseline: false,
            scalar_change: false,
            mission_mask: 4,
            mission_region: Some(42),
            old_count: 100,
            new_count: 97,
            count_delta: -3,
            old_equipment_amount_sum: 10_000_000,
            new_equipment_amount_sum: 9_700_000,
            equipment_amount_delta: -300_000,
            air_accidents: 7,
            experience_raw: 45_000,
        }
    }

    fn single_loss_header() -> AirStrengthDeltaHeader {
        AirStrengthDeltaHeader {
            elapsed_wall_ms: 100,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_air_wing_rows: 500,
            observed_equipment_rows: 900,
            loss_rows: 1,
            reinforcement_rows: 0,
            deleted_rows: 0,
        }
    }

    #[test]
    fn encoded_payload_uses_76_byte_rows_not_64() {
        assert_eq!(ROW_LEN, 76);
        assert_ne!(ROW_LEN, STALE_ROW_LEN_64_REJECTION_GUARD);
        let rows = vec![loss_row(), loss_row()];
        let header = AirStrengthDeltaHeader {
            changed_rows: rows.len() as u32,
            loss_rows: rows.len() as u32,
            ..single_loss_header()
        };
        let payload = encode_payload(&header, &rows);
        assert_eq!(payload.len(), HEADER_LEN + rows.len() * ROW_LEN);
        assert_ne!(
            payload.len(),
            HEADER_LEN + rows.len() * STALE_ROW_LEN_64_REJECTION_GUARD
        );
    }

    #[test]
    fn rejects_stale_64_byte_row_payload_shape() {
        let rows = vec![loss_row()];
        let payload = encode_payload(&single_loss_header(), &rows);
        let stale_payload_len = HEADER_LEN + STALE_ROW_LEN_64_REJECTION_GUARD;
        let summary = decode_payload(64, &payload[..stale_payload_len]);
        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("expected 124") && error.contains("112 byte")
        }));
    }

    #[test]
    fn decodes_loss_reinforcement_and_deleted_rows() {
        let rows = vec![
            loss_row(),
            AirStrengthDeltaRow {
                owner_tag_db_id: 1,
                wing_id: 11,
                generation: 1,
                deleted: false,
                baseline: false,
                scalar_change: false,
                mission_mask: 8,
                mission_region: None,
                old_count: 80,
                new_count: 84,
                count_delta: 4,
                old_equipment_amount_sum: 8_000_000,
                new_equipment_amount_sum: 8_400_000,
                equipment_amount_delta: 400_000,
                air_accidents: 0,
                experience_raw: 12_000,
            },
            AirStrengthDeltaRow {
                owner_tag_db_id: 2,
                wing_id: 20,
                generation: 2,
                deleted: true,
                baseline: false,
                scalar_change: false,
                mission_mask: 4,
                mission_region: Some(43),
                old_count: 12,
                new_count: 0,
                count_delta: -12,
                old_equipment_amount_sum: 1_200_000,
                new_equipment_amount_sum: 0,
                equipment_amount_delta: -1_200_000,
                air_accidents: 2,
                experience_raw: 30_000,
            },
        ];
        let header = AirStrengthDeltaHeader {
            elapsed_wall_ms: 100,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: rows.len() as u32,
            observed_air_wing_rows: 500,
            observed_equipment_rows: 900,
            loss_rows: 2,
            reinforcement_rows: 1,
            deleted_rows: 1,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(32, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(summary.status_flags_or, STATUS_DELTA_DATA);
        assert_eq!(summary.data_rows_total, 3);
        assert_eq!(summary.data_changed_rows_total, 3);
        assert_eq!(
            summary.data_units_total.get("air_strength_loss_rows").copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_strength_reinforcement_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_strength_mission_region_rows")
                .copied(),
            Some(2)
        );
    }

    #[test]
    fn accepts_zero_delta_baseline_data_record() {
        let header = AirStrengthDeltaHeader {
            elapsed_wall_ms: 100,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 0,
            observed_air_wing_rows: 500,
            observed_equipment_rows: 900,
            loss_rows: 0,
            reinforcement_rows: 0,
            deleted_rows: 0,
        };
        let payload = encode_payload(&header, &[]);
        let summary = decode_payload(0, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(summary.data_rows_total, 0);
        assert_eq!(
            summary
                .data_units_total
                .get("air_strength_observed_wing_rows")
                .copied(),
            Some(500)
        );
    }

    #[test]
    fn rejects_bad_scale_and_delta_mismatch() {
        let rows = vec![AirStrengthDeltaRow {
            owner_tag_db_id: 1,
            wing_id: 10,
            generation: 0,
            deleted: false,
            baseline: false,
            scalar_change: false,
            mission_mask: 4,
            mission_region: None,
            old_count: 100,
            new_count: 97,
            count_delta: -2,
            old_equipment_amount_sum: 10_000_000,
            new_equipment_amount_sum: 9_700_000,
            equipment_amount_delta: -300_000,
            air_accidents: 0,
            experience_raw: 0,
        }];
        let header = AirStrengthDeltaHeader {
            elapsed_wall_ms: 100,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_air_wing_rows: 500,
            observed_equipment_rows: 900,
            loss_rows: 1,
            reinforcement_rows: 0,
            deleted_rows: 0,
        };
        let mut payload = encode_payload(&header, &rows);
        payload[40..44].copy_from_slice(&1u32.to_le_bytes());
        let summary = decode_payload(7, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("scale_raw 1")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("count_delta -2")));
    }

    #[test]
    fn decodes_v2_failure_cause_telemetry_separately() {
        let header = AirStrengthDeltaHeader {
            elapsed_wall_ms: 100,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 0,
            observed_air_wing_rows: 10,
            observed_equipment_rows: 20,
            loss_rows: 0,
            reinforcement_rows: 0,
            deleted_rows: 0,
        };
        let mut payload = encode_payload(&header, &[]);
        payload.extend_from_slice(&REGION_STATS_TRAILER_MAGIC.to_le_bytes());
        payload.extend_from_slice(&REGION_STATS_TRAILER_VERSION.to_le_bytes());
        payload.extend_from_slice(&(REGION_STATS_TRAILER_HEADER_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&3u32.to_le_bytes());
        payload.extend_from_slice(&((1u32 << 16) | 31).to_le_bytes());

        let summary = decode_payload(0, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(
            summary
                .data_units_total
                .get("air_strength_invalid_wing_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_strength_scale_mismatch_rows")
                .copied(),
            Some(3)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_strength_first_invalid_ordinal")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_strength_first_invalid_reason")
                .copied(),
            Some(31)
        );
    }
}
