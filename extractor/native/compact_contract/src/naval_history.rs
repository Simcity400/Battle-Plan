//! Domain 70 `sunk_convoy_history_live` payload contract.
//!
//! This domain is a full snapshot of the global `CGameState` sunk-convoy
//! history. It is deliberately separate from domain 49 active naval combats
//! and from per-ship cumulative sunk-convoy counters.

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 70;
pub const PAYLOAD_MAGIC: u32 = u32::from_le_bytes(*b"SCH1");
pub const PAYLOAD_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 32;
pub const ROW_LEN: usize = 24;
pub const MAX_ROWS: usize = 65_536;
pub const MAX_TAG_DB_ID: u32 = 2_048;
pub const MAX_CONVOYS_PER_ROW: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SunkConvoyHistoryRow {
    pub month: i32,
    pub convoys: u32,
    pub killer_country_tag_db_id: u32,
    pub owner_tag_db_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedSunkConvoyHistoryPayload {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub observed_rows: u32,
    pub rows: Vec<SunkConvoyHistoryRow>,
}

pub fn encode_payload(
    rows: &[SunkConvoyHistoryRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_rows: u32,
) -> Result<Vec<u8>, String> {
    validate_rows(rows, observed_rows)?;
    let row_count = u32::try_from(rows.len())
        .map_err(|_| "sunk-convoy history row count exceeds u32".to_string())?;
    let row_bytes = rows
        .len()
        .checked_mul(ROW_LEN)
        .ok_or_else(|| "sunk-convoy history row byte length overflow".to_string())?;
    let capacity = HEADER_LEN
        .checked_add(row_bytes)
        .ok_or_else(|| "sunk-convoy history payload byte length overflow".to_string())?;
    let mut payload = Vec::with_capacity(capacity);
    payload.extend_from_slice(&PAYLOAD_MAGIC.to_le_bytes());
    payload.extend_from_slice(&PAYLOAD_VERSION.to_le_bytes());
    payload.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&observed_rows.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.month.to_le_bytes());
        payload.extend_from_slice(&row.convoys.to_le_bytes());
        payload.extend_from_slice(&row.killer_country_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
    }
    Ok(payload)
}

pub fn decode_payload(payload: &[u8]) -> Result<DecodedSunkConvoyHistoryPayload, String> {
    if payload.len() < HEADER_LEN {
        return Err("sunk-convoy history payload truncated before header".to_string());
    }
    let magic = read_u32(payload, 0)?;
    let version = read_u16(payload, 4)?;
    let header_len = read_u16(payload, 6)? as usize;
    let row_len = read_u16(payload, 8)? as usize;
    let reserved = read_u16(payload, 10)?;
    let status_flags = read_u32(payload, 12)?;
    let elapsed_wall_ms = read_u64(payload, 16)?;
    let row_count = read_u32(payload, 24)? as usize;
    let observed_rows = read_u32(payload, 28)?;
    if magic != PAYLOAD_MAGIC {
        return Err(format!(
            "sunk-convoy history payload magic {magic:#010x} is not SCH1"
        ));
    }
    if version != PAYLOAD_VERSION {
        return Err(format!(
            "sunk-convoy history payload version {version} is unsupported"
        ));
    }
    if header_len != HEADER_LEN || row_len != ROW_LEN || reserved != 0 {
        return Err(format!(
            "sunk-convoy history schema header has header_len={header_len}, row_len={row_len}, reserved={reserved}"
        ));
    }
    if row_count > MAX_ROWS {
        return Err(format!(
            "sunk-convoy history row count {row_count} exceeds {MAX_ROWS}"
        ));
    }
    let expected_len = HEADER_LEN
        .checked_add(
            row_count
                .checked_mul(ROW_LEN)
                .ok_or_else(|| "sunk-convoy history row byte length overflow".to_string())?,
        )
        .ok_or_else(|| "sunk-convoy history payload byte length overflow".to_string())?;
    if payload.len() != expected_len {
        return Err(format!(
            "sunk-convoy history payload length {} does not exactly cover {row_count} rows ({expected_len})",
            payload.len()
        ));
    }
    let mut rows = Vec::with_capacity(row_count);
    for row_index in 0..row_count {
        let base = HEADER_LEN + row_index * ROW_LEN;
        let reserved_tail = read_u64(payload, base + 16)?;
        if reserved_tail != 0 {
            return Err(format!(
                "sunk-convoy history row {row_index} has nonzero reserved tail"
            ));
        }
        rows.push(SunkConvoyHistoryRow {
            month: read_i32(payload, base)?,
            convoys: read_u32(payload, base + 4)?,
            killer_country_tag_db_id: read_u32(payload, base + 8)?,
            owner_tag_db_id: read_u32(payload, base + 12)?,
        });
    }
    validate_rows(&rows, observed_rows)?;
    Ok(DecodedSunkConvoyHistoryPayload {
        elapsed_wall_ms,
        status_flags,
        observed_rows,
        rows,
    })
}

fn validate_rows(rows: &[SunkConvoyHistoryRow], observed_rows: u32) -> Result<(), String> {
    if rows.len() > MAX_ROWS {
        return Err(format!(
            "sunk-convoy history row count {} exceeds {MAX_ROWS}",
            rows.len()
        ));
    }
    let row_count = u32::try_from(rows.len())
        .map_err(|_| "sunk-convoy history row count exceeds u32".to_string())?;
    if observed_rows != row_count {
        return Err(format!(
            "sunk-convoy history observed_rows {observed_rows} does not equal emitted row count {row_count}"
        ));
    }
    let mut identities = BTreeSet::new();
    let mut previous = None;
    for (row_index, row) in rows.iter().enumerate() {
        if row.convoys == 0 || row.convoys > MAX_CONVOYS_PER_ROW {
            return Err(format!(
                "sunk-convoy history row {row_index} convoys {} out of range",
                row.convoys
            ));
        }
        for (label, value) in [
            ("killer_country_tag_db_id", row.killer_country_tag_db_id),
            ("owner_tag_db_id", row.owner_tag_db_id),
        ] {
            if value == 0 || value > MAX_TAG_DB_ID {
                return Err(format!(
                    "sunk-convoy history row {row_index} {label} {value} out of range"
                ));
            }
        }
        let identity = (row.month, row.killer_country_tag_db_id, row.owner_tag_db_id);
        if !identities.insert(identity) {
            return Err(format!(
                "sunk-convoy history row {row_index} duplicates month/killer/owner identity"
            ));
        }
        if previous.is_some_and(|prior| prior >= identity) {
            return Err(format!(
                "sunk-convoy history row {row_index} is not strictly sorted by month/killer/owner"
            ));
        }
        previous = Some(identity);
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let raw = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| format!("sunk-convoy history u16 at {offset} is truncated"))?;
    Ok(u16::from_le_bytes(raw.try_into().expect("two-byte slice")))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("sunk-convoy history u32 at {offset} is truncated"))?;
    Ok(u32::from_le_bytes(raw.try_into().expect("four-byte slice")))
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, String> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| format!("sunk-convoy history i32 at {offset} is truncated"))?;
    Ok(i32::from_le_bytes(raw.try_into().expect("four-byte slice")))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let raw = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| format!("sunk-convoy history u64 at {offset} is truncated"))?;
    Ok(u64::from_le_bytes(
        raw.try_into().expect("eight-byte slice"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<SunkConvoyHistoryRow> {
        vec![
            SunkConvoyHistoryRow {
                month: 10,
                convoys: 2,
                killer_country_tag_db_id: 4,
                owner_tag_db_id: 9,
            },
            SunkConvoyHistoryRow {
                month: 11,
                convoys: 3,
                killer_country_tag_db_id: 8,
                owner_tag_db_id: 9,
            },
        ]
    }

    #[test]
    fn payload_round_trips_exact_rows() {
        let rows = rows();
        let payload = encode_payload(&rows, 77, 0, 2).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.elapsed_wall_ms, 77);
        assert_eq!(decoded.status_flags, 0);
        assert_eq!(decoded.observed_rows, 2);
        assert_eq!(decoded.rows, rows);
    }

    #[test]
    fn decoder_rejects_schema_and_coverage_corruption() {
        let payload = encode_payload(&rows(), 77, 0, 2).unwrap();
        let mut bad_magic = payload.clone();
        bad_magic[0..4].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode_payload(&bad_magic).unwrap_err().contains("magic"));

        let mut bad_reserved = payload.clone();
        bad_reserved[HEADER_LEN + 16..HEADER_LEN + 24].copy_from_slice(&1u64.to_le_bytes());
        assert!(decode_payload(&bad_reserved)
            .unwrap_err()
            .contains("reserved tail"));

        let mut trailing = payload;
        trailing.push(0);
        assert!(decode_payload(&trailing)
            .unwrap_err()
            .contains("does not exactly cover"));
    }

    #[test]
    fn encoder_rejects_unsorted_duplicate_and_invalid_rows() {
        let mut unsorted = rows();
        unsorted.reverse();
        assert!(encode_payload(&unsorted, 0, 0, 2)
            .unwrap_err()
            .contains("not strictly sorted"));

        let mut duplicate = rows();
        duplicate.push(duplicate[1]);
        assert!(encode_payload(&duplicate, 0, 0, 3)
            .unwrap_err()
            .contains("duplicates"));

        let mut invalid = rows();
        invalid[0].convoys = 0;
        assert!(encode_payload(&invalid, 0, 0, 2)
            .unwrap_err()
            .contains("convoys"));
    }
}
