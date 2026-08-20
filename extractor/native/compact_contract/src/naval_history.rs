use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 70;
pub const PAYLOAD_MAGIC: u32 = u32::from_le_bytes(*b"SCH1");
pub const LEGACY_PAYLOAD_VERSION: u16 = 1;
pub const ORDERED_PAYLOAD_VERSION: u16 = 2;
pub const PAYLOAD_VERSION: u16 = 3;
pub const LEGACY_HEADER_LEN: usize = 32;
pub const HEADER_LEN: usize = 272;
pub const ROW_LEN: usize = 24;
pub const MAX_ROWS: usize = 65_536;
pub const PRODUCTION_SCHEDULING_ENABLED: bool = true;
pub const LIVE_FIXTURE_ACCEPTED: bool = false;
pub const LIVE_FIXTURE_RECEIPT_SHA256: [u8; 32] = [0u8; 32];

pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const OWNER_SERIALIZER_MANIFEST_SHA256: [u8; 32] =
    hex32(b"2fd64c0a861d38e252fe262d52e62cbb4a5efa7f94ee7ab374c33e8ac368827e");
pub const OWNER_SERIALIZER_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"bc679b4a175e75253a1fea1bfde625ccf8594ec06c562699aeaac75b9eaddf3a");
pub const ROW_MUTATION_MANIFEST_SHA256: [u8; 32] =
    hex32(b"b36682b2f942e7e4e7c8ae9dbfc05e2b52ee46dcf388fc9ad6e1fee24fdb802e");
pub const ROW_SERIALIZER_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"005235886a2d847b111df481ae948395828ec999e3cebe39612d36d364fefe98");
pub const MUTATION_ARTIFACT_SHA256: [u8; 32] =
    hex32(b"2fa2e8a326081f82735b3824dc8b693130605a956964a170f1b5538e4ac1ef14");

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut output = [0u8; 32];
    let mut index = 0;
    while index < output.len() {
        output[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    output
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SunkConvoyHistoryRow {
    pub month: i32,
    pub convoys: u32,
    pub killer_country_tag_db_id: u32,
    pub owner_tag_db_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SunkConvoyHistoryOrdering {
    CanonicalIdentity,
    FirstSeenAggregateIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SunkConvoyHistoryAuthority {
    Unavailable,
    LegacyUnbound,
    StaticExact20Receipt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedSunkConvoyHistoryPayload {
    pub schema_version: u16,
    pub elapsed_wall_ms: u64,
    pub frame_hour: Option<u64>,
    pub source_sequence: Option<u64>,
    pub status_flags: u32,
    pub observed_rows: u32,
    pub authority: SunkConvoyHistoryAuthority,
    pub ordering: SunkConvoyHistoryOrdering,
    pub rows: Vec<SunkConvoyHistoryRow>,
}

pub fn encode_legacy_ordered_payload(
    rows: &[SunkConvoyHistoryRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
    observed_rows: u32,
) -> Result<Vec<u8>, String> {
    validate_rows(rows, observed_rows, false)?;
    let row_count = u32::try_from(rows.len())
        .map_err(|_| "sunk-convoy history row count exceeds u32".to_string())?;
    let row_bytes = rows
        .len()
        .checked_mul(ROW_LEN)
        .ok_or_else(|| "sunk-convoy history row byte length overflow".to_string())?;
    let capacity = LEGACY_HEADER_LEN
        .checked_add(row_bytes)
        .ok_or_else(|| "sunk-convoy history payload byte length overflow".to_string())?;
    let mut payload = Vec::with_capacity(capacity);
    payload.extend_from_slice(&PAYLOAD_MAGIC.to_le_bytes());
    payload.extend_from_slice(&ORDERED_PAYLOAD_VERSION.to_le_bytes());
    payload.extend_from_slice(&(LEGACY_HEADER_LEN as u16).to_le_bytes());
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

pub fn encode_exact_payload(
    rows: &[SunkConvoyHistoryRow],
    elapsed_wall_ms: u64,
    frame_hour: u64,
    source_sequence: u64,
    status_flags: u32,
    observed_rows: u32,
) -> Result<Vec<u8>, String> {
    if source_sequence == 0 {
        return Err("exact sunk-convoy payload has zero source sequence".to_string());
    }
    if crate::compact_status_unknown_flags_v1(status_flags) != 0 {
        return Err("exact sunk-convoy payload carries unknown status bits".to_string());
    }
    if crate::compact_status_is_failure(status_flags) {
        return Err("exact sunk-convoy payload carries failure status".to_string());
    }
    encode_v3(
        rows,
        elapsed_wall_ms,
        frame_hour,
        source_sequence,
        status_flags,
        observed_rows,
        SunkConvoyHistoryAuthority::StaticExact20Receipt,
    )
}

pub fn encode_unavailable_payload(
    elapsed_wall_ms: u64,
    frame_hour: u64,
    source_sequence: u64,
    status_flags: u32,
) -> Result<Vec<u8>, String> {
    if source_sequence == 0 {
        return Err("unavailable sunk-convoy payload has zero source sequence".to_string());
    }
    if crate::compact_status_unknown_flags_v1(status_flags) != 0 {
        return Err("unavailable sunk-convoy payload carries unknown status bits".to_string());
    }
    if !crate::compact_status_is_failure(status_flags) {
        return Err("unavailable sunk-convoy payload lacks failure status".to_string());
    }
    encode_v3(
        &[],
        elapsed_wall_ms,
        frame_hour,
        source_sequence,
        status_flags,
        0,
        SunkConvoyHistoryAuthority::Unavailable,
    )
}

fn encode_v3(
    rows: &[SunkConvoyHistoryRow],
    elapsed_wall_ms: u64,
    frame_hour: u64,
    source_sequence: u64,
    status_flags: u32,
    observed_rows: u32,
    authority: SunkConvoyHistoryAuthority,
) -> Result<Vec<u8>, String> {
    validate_rows(rows, observed_rows, false)?;
    let authority_raw = match authority {
        SunkConvoyHistoryAuthority::Unavailable => 0u16,
        SunkConvoyHistoryAuthority::StaticExact20Receipt => 1u16,
        SunkConvoyHistoryAuthority::LegacyUnbound => {
            return Err("legacy authority cannot be emitted in SCH1/v3".to_string())
        }
    };
    if authority == SunkConvoyHistoryAuthority::Unavailable && !rows.is_empty() {
        return Err("unavailable sunk-convoy payload carries rows".to_string());
    }
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
    payload.extend_from_slice(&authority_raw.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&observed_rows.to_le_bytes());
    payload.extend_from_slice(&frame_hour.to_le_bytes());
    payload.extend_from_slice(&source_sequence.to_le_bytes());
    for hash in [
        EXECUTABLE_SHA256,
        OWNER_SERIALIZER_MANIFEST_SHA256,
        OWNER_SERIALIZER_ARTIFACT_SHA256,
        ROW_MUTATION_MANIFEST_SHA256,
        ROW_SERIALIZER_ARTIFACT_SHA256,
        MUTATION_ARTIFACT_SHA256,
        LIVE_FIXTURE_RECEIPT_SHA256,
    ] {
        if authority == SunkConvoyHistoryAuthority::Unavailable {
            payload.extend_from_slice(&[0u8; 32]);
        } else {
            payload.extend_from_slice(&hash);
        }
    }
    for row in rows {
        payload.extend_from_slice(&row.month.to_le_bytes());
        payload.extend_from_slice(&row.convoys.to_le_bytes());
        payload.extend_from_slice(&row.killer_country_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
    }
    Ok(payload)
}

/// Byte offset of `source_sequence` inside an SCH1/v3 header.
///
/// Derived from [`encode_v3`]'s own write order: magic (4) + version (2) + header_len (2) +
/// row_len (2) + authority (2) + status_flags (4) + elapsed_wall_ms (8) + row_count (4) +
/// observed_rows (4) + frame_hour (8) = 40.
pub const SOURCE_SEQUENCE_OFFSET: usize = 40;

/// Write the record's FINAL H4CR sequence into an already-encoded SCH1 payload.
///
/// The producer bakes this field during the frame build, but the build's numbering is not the
/// numbering the stream ships: `resequence_compact_records` renumbers a frame's survivors by
/// POSITION after the post-build passes drop the records of rejected or unresolved-hour domains.
/// Every payload sitting after a dropped record therefore ends up carrying a sequence that is
/// too high by exactly the number of records dropped ahead of it, and
/// `world_fold::fold_sunk_convoy_history` refuses the payload on
/// `source_sequence != record.sequence`. Measured on capture `ob_first_bmp`: 58 of 367 D70 data
/// records, by +2 (x47), +4 (x2), +6 (x8) and +22 (x1).
///
/// Stamping from the resequencer makes the two the same value by construction.
///
/// A payload whose shape does not carry the field -- a legacy v1/v2 header, or anything too
/// short -- is left untouched and reported as not stamped, never treated as an error: those
/// spellings have no `source_sequence` to correct.
pub fn stamp_source_sequence(payload: &mut [u8], source_sequence: u64) -> bool {
    if payload.len() < HEADER_LEN {
        return false;
    }
    if u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) != PAYLOAD_MAGIC {
        return false;
    }
    if u16::from_le_bytes([payload[4], payload[5]]) != PAYLOAD_VERSION {
        return false;
    }
    payload[SOURCE_SEQUENCE_OFFSET..SOURCE_SEQUENCE_OFFSET + 8]
        .copy_from_slice(&source_sequence.to_le_bytes());
    true
}

pub fn decode_payload(payload: &[u8]) -> Result<DecodedSunkConvoyHistoryPayload, String> {
    if payload.len() < LEGACY_HEADER_LEN {
        return Err("sunk-convoy history payload truncated before header".to_string());
    }
    let magic = read_u32(payload, 0)?;
    let version = read_u16(payload, 4)?;
    let header_len = read_u16(payload, 6)? as usize;
    let row_len = read_u16(payload, 8)? as usize;
    let authority_raw = read_u16(payload, 10)?;
    let status_flags = read_u32(payload, 12)?;
    let elapsed_wall_ms = read_u64(payload, 16)?;
    let row_count = read_u32(payload, 24)? as usize;
    let observed_rows = read_u32(payload, 28)?;
    if magic != PAYLOAD_MAGIC {
        return Err(format!(
            "sunk-convoy history payload magic {magic:#010x} is not SCH1"
        ));
    }
    if version != LEGACY_PAYLOAD_VERSION
        && version != ORDERED_PAYLOAD_VERSION
        && version != PAYLOAD_VERSION
    {
        return Err(format!(
            "sunk-convoy history payload version {version} is unsupported"
        ));
    }
    let expected_header_len = if version == PAYLOAD_VERSION {
        HEADER_LEN
    } else {
        LEGACY_HEADER_LEN
    };
    if header_len != expected_header_len || row_len != ROW_LEN {
        return Err(format!(
            "sunk-convoy history schema header has header_len={header_len}, row_len={row_len}"
        ));
    }
    let (authority, frame_hour, source_sequence) = if version == PAYLOAD_VERSION {
        let authority = match authority_raw {
            0 => SunkConvoyHistoryAuthority::Unavailable,
            1 => SunkConvoyHistoryAuthority::StaticExact20Receipt,
            other => {
                return Err(format!(
                    "sunk-convoy history authority state {other} is unsupported"
                ))
            }
        };
        let frame_hour = read_u64(payload, 32)?;
        let source_sequence = read_u64(payload, 40)?;
        if source_sequence == 0 {
            return Err("sunk-convoy history has zero source sequence".to_string());
        }
        let expected_hashes = [
            EXECUTABLE_SHA256,
            OWNER_SERIALIZER_MANIFEST_SHA256,
            OWNER_SERIALIZER_ARTIFACT_SHA256,
            ROW_MUTATION_MANIFEST_SHA256,
            ROW_SERIALIZER_ARTIFACT_SHA256,
            MUTATION_ARTIFACT_SHA256,
            LIVE_FIXTURE_RECEIPT_SHA256,
        ];
        for (index, expected) in expected_hashes.iter().enumerate() {
            let offset = 48 + index * 32;
            let actual: [u8; 32] = payload
                .get(offset..offset + 32)
                .ok_or_else(|| "sunk-convoy history authority hash truncated".to_string())?
                .try_into()
                .expect("32-byte slice");
            let wanted = if authority == SunkConvoyHistoryAuthority::Unavailable {
                [0u8; 32]
            } else {
                *expected
            };
            if actual != wanted {
                return Err(format!(
                    "sunk-convoy history authority hash {index} does not match its pinned receipt"
                ));
            }
        }
        if crate::compact_status_unknown_flags_v1(status_flags) != 0 {
            return Err("sunk-convoy history carries unknown status bits".to_string());
        }
        if authority == SunkConvoyHistoryAuthority::Unavailable {
            if row_count != 0 || observed_rows != 0 {
                return Err("unavailable sunk-convoy history carries rows".to_string());
            }
            if !crate::compact_status_is_failure(status_flags) {
                return Err("unavailable sunk-convoy history lacks failure status".to_string());
            }
        } else if crate::compact_status_is_failure(status_flags) {
            return Err("exact sunk-convoy history carries failure status".to_string());
        }
        (authority, Some(frame_hour), Some(source_sequence))
    } else {
        if authority_raw != 0 {
            return Err(format!(
                "legacy sunk-convoy history has nonzero reserved field {authority_raw}"
            ));
        }
        (SunkConvoyHistoryAuthority::LegacyUnbound, None, None)
    };
    if row_count > MAX_ROWS {
        return Err(format!(
            "sunk-convoy history row count {row_count} exceeds {MAX_ROWS}"
        ));
    }
    let expected_len = expected_header_len
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
        let base = expected_header_len + row_index * ROW_LEN;
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
    let ordering = if version == LEGACY_PAYLOAD_VERSION {
        SunkConvoyHistoryOrdering::CanonicalIdentity
    } else {
        SunkConvoyHistoryOrdering::FirstSeenAggregateIdentity
    };
    validate_rows(
        &rows,
        observed_rows,
        ordering == SunkConvoyHistoryOrdering::CanonicalIdentity,
    )?;
    Ok(DecodedSunkConvoyHistoryPayload {
        schema_version: version,
        elapsed_wall_ms,
        frame_hour,
        source_sequence,
        status_flags,
        observed_rows,
        authority,
        ordering,
        rows,
    })
}

fn validate_rows(
    rows: &[SunkConvoyHistoryRow],
    observed_rows: u32,
    require_canonical_order: bool,
) -> Result<(), String> {
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
        if row.convoys == 0 {
            return Err(format!(
                "sunk-convoy history row {row_index} has zero convoys"
            ));
        }
        for (label, value) in [
            ("killer_country_tag_db_id", row.killer_country_tag_db_id),
            ("owner_tag_db_id", row.owner_tag_db_id),
        ] {
            if value == 0 {
                return Err(format!(
                    "sunk-convoy history row {row_index} {label} is zero"
                ));
            }
        }
        let identity = (row.month, row.killer_country_tag_db_id, row.owner_tag_db_id);
        if !identities.insert(identity) {
            return Err(format!(
                "sunk-convoy history row {row_index} duplicates month/killer/owner identity"
            ));
        }
        if require_canonical_order && previous.is_some_and(|prior| prior >= identity) {
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
        let payload = encode_exact_payload(&rows, 77, 17_000_277, 41, 0, 2).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.schema_version, PAYLOAD_VERSION);
        assert_eq!(decoded.elapsed_wall_ms, 77);
        assert_eq!(decoded.frame_hour, Some(17_000_277));
        assert_eq!(decoded.source_sequence, Some(41));
        assert_eq!(decoded.status_flags, 0);
        assert_eq!(decoded.observed_rows, 2);
        assert_eq!(
            decoded.authority,
            SunkConvoyHistoryAuthority::StaticExact20Receipt
        );
        assert_eq!(
            decoded.ordering,
            SunkConvoyHistoryOrdering::FirstSeenAggregateIdentity
        );
        assert_eq!(decoded.rows, rows);
    }

    #[test]
    fn decoder_rejects_schema_and_coverage_corruption() {
        let payload = encode_exact_payload(&rows(), 77, 17_000_277, 41, 0, 2).unwrap();
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
    fn v2_preserves_unsorted_first_seen_order_and_rejects_duplicate_and_invalid_rows() {
        let mut unsorted = rows();
        unsorted.reverse();
        let decoded =
            decode_payload(&encode_legacy_ordered_payload(&unsorted, 0, 0, 2).unwrap()).unwrap();
        assert_eq!(decoded.rows, unsorted);
        assert_eq!(
            decoded.ordering,
            SunkConvoyHistoryOrdering::FirstSeenAggregateIdentity
        );

        let mut duplicate = rows();
        duplicate.push(duplicate[1]);
        assert!(encode_legacy_ordered_payload(&duplicate, 0, 0, 3)
            .unwrap_err()
            .contains("duplicates"));

        let mut invalid = rows();
        invalid[0].convoys = 0;
        assert!(encode_legacy_ordered_payload(&invalid, 0, 0, 2)
            .unwrap_err()
            .contains("convoys"));
    }

    #[test]
    fn v1_remains_readable_but_cannot_claim_engine_first_seen_order() {
        let mut payload = encode_legacy_ordered_payload(&rows(), 77, 0, 2).unwrap();
        payload[4..6].copy_from_slice(&LEGACY_PAYLOAD_VERSION.to_le_bytes());
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(
            decoded.ordering,
            SunkConvoyHistoryOrdering::CanonicalIdentity
        );

        let mut unsorted = rows();
        unsorted.reverse();
        let mut invalid_legacy = encode_legacy_ordered_payload(&unsorted, 77, 0, 2).unwrap();
        invalid_legacy[4..6].copy_from_slice(&LEGACY_PAYLOAD_VERSION.to_le_bytes());
        assert!(decode_payload(&invalid_legacy)
            .unwrap_err()
            .contains("not strictly sorted"));
    }

    #[test]
    fn v3_unavailable_is_explicit_and_cannot_carry_rows_or_receipt_hashes() {
        let status = crate::COMPACT_STATUS_SOURCE_UNAVAILABLE;
        let payload = encode_unavailable_payload(77, 17_000_277, 41, status).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.authority, SunkConvoyHistoryAuthority::Unavailable);
        assert_eq!(decoded.frame_hour, Some(17_000_277));
        assert_eq!(decoded.source_sequence, Some(41));
        assert!(decoded.rows.is_empty());

        let mut bad_hash = payload;
        bad_hash[48] = 1;
        assert!(decode_payload(&bad_hash)
            .unwrap_err()
            .contains("authority hash"));
    }

    #[test]
    fn v3_rejects_unknown_status_bits_before_classification() {
        let unknown = 1u32 << 31;
        assert!(encode_exact_payload(&rows(), 0, 17_000_277, 41, unknown, 2)
            .unwrap_err()
            .contains("unknown status"));
        assert!(encode_unavailable_payload(0, 17_000_277, 41, unknown)
            .unwrap_err()
            .contains("unknown status"));
    }
}
