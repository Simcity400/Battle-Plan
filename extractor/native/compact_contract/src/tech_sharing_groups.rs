//! Domain 69 `tech_sharing_groups_live` payload contract and encoder.

use std::collections::BTreeSet;

/// One technology-sharing group in the domain 69 snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechSharingGroupRow {
    pub id_token: u32,
    pub member_tag_db_ids: Vec<u32>,
}

const ROW_LEN: usize = 12;

/// Exact engine scale of the signed fixed-point dynamic bonus values.
///
/// Static RE for build 870ebf proves that the scalar parser stores five
/// decimal places in a signed i64 and that `modify_tech_sharing_bonus` adds the
/// parsed value to the member-paired slot.  This is the dynamic adjustment,
/// not a pre-computed total including the static definition's base bonus.
pub const TECH_SHARING_BONUS_FIXED5_SCALE: i64 = 100_000;

/// One logical member and its same-index dynamic technology-sharing bonus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechSharingMemberBonus {
    pub member_tag_db_id: u32,
    pub dynamic_bonus_raw_fixed5: i64,
}

/// One technology-sharing group in the paired-member/bonus schema.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechSharingGroupBonusRow {
    pub id_token: u32,
    pub members: Vec<TechSharingMemberBonus>,
}

/// Decoded domain 69 payload with the proven paired bonus semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedTechSharingGroupsBonusPayload {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub rows: Vec<TechSharingGroupBonusRow>,
}

/// `TSG2` encoded little-endian in the paired-bonus payload header.
pub const TECH_SHARING_BONUS_PAYLOAD_MAGIC: u32 = u32::from_le_bytes(*b"TSG2");
/// Explicit paired-bonus payload schema version.
pub const TECH_SHARING_BONUS_PAYLOAD_VERSION: u32 = 2;

const BONUS_HEADER_LEN: usize = 24;
const MEMBER_BONUS_LEN: usize = 12;

/// Encode the production domain 69 DATA payload.
///
/// Wire layout:
/// - header: `elapsed_wall_ms(u64) group_count(u32) status_flags(u32)`
/// - one fixed 12-byte row per group:
///   `id_token(u32) member_count(u32) member_blob_offset(u32)`
/// - one concatenated member blob of `u32` country tag database IDs
pub fn encode_payload(
    rows: &[TechSharingGroupRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Vec<u8> {
    let group_count = rows.len().min(u32::MAX as usize) as u32;
    let mut payload = Vec::with_capacity(16usize.saturating_add(rows.len() * ROW_LEN));
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&group_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());

    let mut member_cursor = 0u32;
    for row in rows {
        let member_count = row.member_tag_db_ids.len().min(u32::MAX as usize) as u32;
        payload.extend_from_slice(&row.id_token.to_le_bytes());
        payload.extend_from_slice(&member_count.to_le_bytes());
        payload.extend_from_slice(&member_cursor.to_le_bytes());
        member_cursor = member_cursor.saturating_add(member_count);
    }
    for row in rows {
        for tag in &row.member_tag_db_ids {
            payload.extend_from_slice(&tag.to_le_bytes());
        }
    }
    payload
}

/// Encode the paired-member/bonus domain 69 DATA payload.
///
/// Wire layout:
/// - header: `elapsed_wall_ms(u64) group_count(u32) status_flags(u32)` then
///   `magic="TSG2"(u32) schema_version=2(u32)`
/// - one fixed 12-byte row per group:
///   `id_token(u32) member_count(u32) pair_blob_offset(u32)`
/// - one concatenated pair blob in row/member order; each 12-byte pair is
///   `member_tag_db_id(u32) dynamic_bonus_raw_fixed5(i64)`
///
/// Consumers must require the magic and schema version before interpreting the
/// row table. The extra header and wider pair blob are also incompatible with
/// the legacy member-only decoder's exact-coverage checks, including for zero
/// groups/members.
pub fn encode_payload_with_bonuses(
    rows: &[TechSharingGroupBonusRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, String> {
    let group_count = u32::try_from(rows.len())
        .map_err(|_| "tech-sharing group count exceeds u32".to_string())?;
    let mut pair_total = 0usize;
    let mut group_ids = BTreeSet::new();
    for (row_index, row) in rows.iter().enumerate() {
        if row.id_token == 0 || (row.id_token & 0x8000_0000) != 0 {
            return Err(format!(
                "tech-sharing row {row_index} has invalid id token {}",
                row.id_token
            ));
        }
        if !group_ids.insert(row.id_token) {
            return Err(format!(
                "tech-sharing row {row_index} duplicates id token {}",
                row.id_token
            ));
        }
        u32::try_from(row.members.len())
            .map_err(|_| format!("tech-sharing row {row_index} member count exceeds u32"))?;
        pair_total = pair_total
            .checked_add(row.members.len())
            .ok_or_else(|| "tech-sharing pair count overflow".to_string())?;
        let mut member_ids = BTreeSet::new();
        for (member_index, member) in row.members.iter().enumerate() {
            if member.member_tag_db_id == 0 || (member.member_tag_db_id & 0x8000_0000) != 0 {
                return Err(format!(
                    "tech-sharing row {row_index} member {member_index} has invalid tag_db_id {}",
                    member.member_tag_db_id
                ));
            }
            if !member_ids.insert(member.member_tag_db_id) {
                return Err(format!(
                    "tech-sharing row {row_index} duplicates member tag_db_id {}",
                    member.member_tag_db_id
                ));
            }
        }
    }

    // The wire offset is u32. Reject an unrepresentable aggregate before byte
    // capacity calculation/allocation rather than discovering it while writing
    // the row table after a potentially huge allocation.
    checked_pair_total_u32(pair_total)?;

    let row_bytes = rows
        .len()
        .checked_mul(ROW_LEN)
        .ok_or_else(|| "tech-sharing row byte length overflow".to_string())?;
    let pair_bytes = pair_total
        .checked_mul(MEMBER_BONUS_LEN)
        .ok_or_else(|| "tech-sharing pair byte length overflow".to_string())?;
    let capacity = BONUS_HEADER_LEN
        .checked_add(row_bytes)
        .and_then(|value| value.checked_add(pair_bytes))
        .ok_or_else(|| "tech-sharing payload byte length overflow".to_string())?;
    let mut payload = Vec::with_capacity(capacity);
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&group_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&TECH_SHARING_BONUS_PAYLOAD_MAGIC.to_le_bytes());
    payload.extend_from_slice(&TECH_SHARING_BONUS_PAYLOAD_VERSION.to_le_bytes());

    let mut pair_cursor = 0u32;
    for row in rows {
        let member_count = u32::try_from(row.members.len())
            .map_err(|_| "tech-sharing member count exceeds u32".to_string())?;
        payload.extend_from_slice(&row.id_token.to_le_bytes());
        payload.extend_from_slice(&member_count.to_le_bytes());
        payload.extend_from_slice(&pair_cursor.to_le_bytes());
        pair_cursor = pair_cursor
            .checked_add(member_count)
            .ok_or_else(|| "tech-sharing pair offset overflow".to_string())?;
    }
    for row in rows {
        for member in &row.members {
            payload.extend_from_slice(&member.member_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&member.dynamic_bonus_raw_fixed5.to_le_bytes());
        }
    }
    Ok(payload)
}

/// Decode and structurally validate the paired-member/bonus domain 69 payload.
pub fn decode_payload_with_bonuses(
    payload: &[u8],
) -> Result<DecodedTechSharingGroupsBonusPayload, String> {
    if payload.len() < BONUS_HEADER_LEN {
        return Err("tech-sharing bonus payload truncated before header".to_string());
    }
    let elapsed_wall_ms = read_u64(payload, 0)?;
    let group_count = read_u32(payload, 8)? as usize;
    let status_flags = read_u32(payload, 12)?;
    let magic = read_u32(payload, 16)?;
    let schema_version = read_u32(payload, 20)?;
    if magic != TECH_SHARING_BONUS_PAYLOAD_MAGIC {
        return Err(format!(
            "tech-sharing bonus payload magic {magic:#010x} is not TSG2"
        ));
    }
    if schema_version != TECH_SHARING_BONUS_PAYLOAD_VERSION {
        return Err(format!(
            "tech-sharing bonus payload schema version {schema_version} is unsupported"
        ));
    }
    let rows_len = group_count
        .checked_mul(ROW_LEN)
        .ok_or_else(|| "tech-sharing bonus row length overflow".to_string())?;
    let blob_start = BONUS_HEADER_LEN
        .checked_add(rows_len)
        .ok_or_else(|| "tech-sharing bonus row boundary overflow".to_string())?;
    if blob_start > payload.len() {
        return Err("tech-sharing bonus payload truncated in row table".to_string());
    }
    let blob = &payload[blob_start..];
    if blob.len() % MEMBER_BONUS_LEN != 0 {
        return Err("tech-sharing bonus pair blob has a partial row".to_string());
    }
    let pair_total = blob.len() / MEMBER_BONUS_LEN;
    let mut expected_offset = 0usize;
    let mut rows = Vec::with_capacity(group_count);
    let mut group_ids = BTreeSet::new();
    for row_index in 0..group_count {
        let base = BONUS_HEADER_LEN + row_index * ROW_LEN;
        let id_token = read_u32(payload, base)?;
        let member_count = read_u32(payload, base + 4)? as usize;
        let pair_offset = read_u32(payload, base + 8)? as usize;
        if id_token == 0 || (id_token & 0x8000_0000) != 0 {
            return Err(format!(
                "tech-sharing row {row_index} has invalid id token {id_token}"
            ));
        }
        if !group_ids.insert(id_token) {
            return Err(format!(
                "tech-sharing row {row_index} duplicates id token {id_token}"
            ));
        }
        if pair_offset != expected_offset {
            return Err(format!(
                "tech-sharing row {row_index} pair offset {pair_offset} is not contiguous at {expected_offset}"
            ));
        }
        let pair_end = pair_offset
            .checked_add(member_count)
            .ok_or_else(|| format!("tech-sharing row {row_index} pair extent overflow"))?;
        if pair_end > pair_total {
            return Err(format!(
                "tech-sharing row {row_index} pair extent exceeds blob"
            ));
        }
        let mut members = Vec::with_capacity(member_count);
        let mut member_ids = BTreeSet::new();
        for member_index in 0..member_count {
            let pair_index = pair_offset + member_index;
            let pair_base = pair_index * MEMBER_BONUS_LEN;
            let member_tag_db_id = read_u32(blob, pair_base)?;
            let dynamic_bonus_raw_fixed5 = read_i64(blob, pair_base + 4)?;
            if member_tag_db_id == 0 || (member_tag_db_id & 0x8000_0000) != 0 {
                return Err(format!(
                    "tech-sharing row {row_index} member {member_index} has invalid tag_db_id {member_tag_db_id}"
                ));
            }
            if !member_ids.insert(member_tag_db_id) {
                return Err(format!(
                    "tech-sharing row {row_index} duplicates member tag_db_id {member_tag_db_id}"
                ));
            }
            members.push(TechSharingMemberBonus {
                member_tag_db_id,
                dynamic_bonus_raw_fixed5,
            });
        }
        expected_offset = pair_end;
        rows.push(TechSharingGroupBonusRow { id_token, members });
    }
    if expected_offset != pair_total {
        return Err(format!(
            "tech-sharing bonus pair blob has {} unclaimed rows",
            pair_total - expected_offset
        ));
    }
    Ok(DecodedTechSharingGroupsBonusPayload {
        elapsed_wall_ms,
        status_flags,
        rows,
    })
}

fn checked_pair_total_u32(pair_total: usize) -> Result<u32, String> {
    u32::try_from(pair_total)
        .map_err(|_| "tech-sharing aggregate pair count exceeds u32".to_string())
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("tech-sharing u32 at {offset} is truncated"))?;
    Ok(u32::from_le_bytes(raw.try_into().expect("four-byte slice")))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let raw = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| format!("tech-sharing u64 at {offset} is truncated"))?;
    Ok(u64::from_le_bytes(
        raw.try_into().expect("eight-byte slice"),
    ))
}

fn read_i64(bytes: &[u8], offset: usize) -> Result<i64, String> {
    let raw = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| format!("tech-sharing i64 at {offset} is truncated"))?;
    Ok(i64::from_le_bytes(
        raw.try_into().expect("eight-byte slice"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<TechSharingGroupBonusRow> {
        vec![
            TechSharingGroupBonusRow {
                id_token: 10,
                members: vec![
                    TechSharingMemberBonus {
                        member_tag_db_id: 1,
                        dynamic_bonus_raw_fixed5: 10_000,
                    },
                    TechSharingMemberBonus {
                        member_tag_db_id: 2,
                        dynamic_bonus_raw_fixed5: -5_000,
                    },
                ],
            },
            TechSharingGroupBonusRow {
                id_token: 20,
                members: Vec::new(),
            },
        ]
    }

    #[test]
    fn paired_bonus_payload_round_trips_and_preserves_empty_groups() {
        let rows = fixture();
        let bytes = encode_payload_with_bonuses(&rows, 77, 0x40).unwrap();
        let decoded = decode_payload_with_bonuses(&bytes).unwrap();
        assert_eq!(decoded.elapsed_wall_ms, 77);
        assert_eq!(decoded.status_flags, 0x40);
        assert_eq!(decoded.rows, rows);
    }

    #[test]
    fn v2_discriminator_is_explicit_even_for_zero_rows() {
        let bytes = encode_payload_with_bonuses(&[], 77, 0).unwrap();
        assert_eq!(bytes.len(), BONUS_HEADER_LEN);
        assert_eq!(
            read_u32(&bytes, 16).unwrap(),
            TECH_SHARING_BONUS_PAYLOAD_MAGIC
        );
        assert_eq!(
            read_u32(&bytes, 20).unwrap(),
            TECH_SHARING_BONUS_PAYLOAD_VERSION
        );
        assert_ne!(bytes.len(), 16, "legacy empty payload was only 16 bytes");

        let mut bad_magic = bytes.clone();
        bad_magic[16..20].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_payload_with_bonuses(&bad_magic)
            .unwrap_err()
            .contains("magic"));

        let mut bad_version = bytes;
        bad_version[20..24].copy_from_slice(&3u32.to_le_bytes());
        assert!(decode_payload_with_bonuses(&bad_version)
            .unwrap_err()
            .contains("schema version"));
    }

    #[test]
    fn paired_blob_uses_explicitly_wider_rows_than_legacy_members() {
        let rows = fixture();
        let bytes = encode_payload_with_bonuses(&rows, 77, 0).unwrap();
        let blob_start = BONUS_HEADER_LEN + rows.len() * ROW_LEN;
        let declared_members: usize = rows.iter().map(|row| row.members.len()).sum();
        assert_eq!(
            bytes.len() - blob_start,
            declared_members * MEMBER_BONUS_LEN
        );
        assert_ne!(bytes.len() - blob_start, declared_members * 4);
    }

    #[test]
    fn bonus_decoder_rejects_noncontiguous_offsets_and_trailing_pairs() {
        let rows = fixture();
        let bytes = encode_payload_with_bonuses(&rows, 77, 0).unwrap();

        let mut bad_offset = bytes.clone();
        bad_offset[BONUS_HEADER_LEN + 8..BONUS_HEADER_LEN + 12]
            .copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_payload_with_bonuses(&bad_offset)
            .unwrap_err()
            .contains("not contiguous"));

        let mut trailing = bytes;
        trailing.extend_from_slice(&3u32.to_le_bytes());
        trailing.extend_from_slice(&0i64.to_le_bytes());
        assert!(decode_payload_with_bonuses(&trailing)
            .unwrap_err()
            .contains("unclaimed"));
    }

    #[test]
    fn paired_schema_rejects_invalid_or_duplicate_identities() {
        let mut rows = fixture();
        rows[0].members[1].member_tag_db_id = 1;
        assert!(encode_payload_with_bonuses(&rows, 0, 0)
            .unwrap_err()
            .contains("duplicates member"));

        let mut rows = fixture();
        rows[1].id_token = rows[0].id_token;
        assert!(encode_payload_with_bonuses(&rows, 0, 0)
            .unwrap_err()
            .contains("duplicates id"));

        let mut rows = fixture();
        rows[0].members[0].member_tag_db_id = 0x8000_0001;
        assert!(encode_payload_with_bonuses(&rows, 0, 0)
            .unwrap_err()
            .contains("invalid tag_db_id"));

        let mut rows = fixture();
        rows[0].id_token = 0x8000_0001;
        assert!(encode_payload_with_bonuses(&rows, 0, 0)
            .unwrap_err()
            .contains("invalid id token"));
    }

    #[test]
    fn aggregate_pair_count_is_rejected_before_allocation() {
        #[cfg(target_pointer_width = "64")]
        assert!(checked_pair_total_u32(u32::MAX as usize + 1)
            .unwrap_err()
            .contains("aggregate pair count"));
    }
}
