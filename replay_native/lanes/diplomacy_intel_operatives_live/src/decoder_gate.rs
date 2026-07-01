use crate::compact::{decode_payload, Domain53GateSummary, DOMAIN_ID_PROPOSED};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactRecordHeader {
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub domain_id: u16,
    pub record_type: u16,
    pub payload_len: u32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain53RecordGateSummary {
    pub record_count: u64,
    pub payload_bytes_total: u64,
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub parse_errors: Vec<String>,
}

pub fn encode_domain53_data_record(sequence: u64, game_hour_raw: i32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&DOMAIN_ID_PROPOSED.to_le_bytes());
    out.extend_from_slice(&COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn decode_domain53_record(offset: usize, bytes: &[u8]) -> Domain53RecordGateSummary {
    let mut out = Domain53RecordGateSummary::default();
    let Some((header, payload)) = parse_record(offset, bytes, &mut out) else {
        return out;
    };
    out.record_count = 1;
    out.payload_bytes_total = payload.len() as u64;

    if header.domain_id != DOMAIN_ID_PROPOSED {
        out.parse_errors.push(format!(
            "offset {offset}: domain_id {} != {}",
            header.domain_id, DOMAIN_ID_PROPOSED
        ));
        return out;
    }
    if header.record_type != COMPACT_RECORD_TYPE_DOMAIN_DATA {
        out.parse_errors.push(format!(
            "offset {offset}: record_type {} != {}",
            header.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA
        ));
        return out;
    }

    merge_payload_summary(
        &mut out,
        decode_payload(offset + COMPACT_RECORD_HEADER_LEN, payload),
    );
    out
}

fn parse_record<'a>(
    offset: usize,
    bytes: &'a [u8],
    errors: &mut Domain53RecordGateSummary,
) -> Option<(CompactRecordHeader, &'a [u8])> {
    if bytes.len() < COMPACT_RECORD_HEADER_LEN {
        errors.parse_errors.push(format!(
            "offset {offset}: truncated H4CR header with {} byte(s)",
            bytes.len()
        ));
        return None;
    }
    if &bytes[0..4] != COMPACT_RECORD_MAGIC {
        errors
            .parse_errors
            .push(format!("offset {offset}: invalid H4CR magic"));
        return None;
    }
    let version = read_u16(bytes, 4);
    if version != COMPACT_RECORD_VERSION {
        errors.parse_errors.push(format!(
            "offset {offset}: unsupported H4CR version {version}"
        ));
    }
    let header_len = read_u16(bytes, 6) as usize;
    if header_len != COMPACT_RECORD_HEADER_LEN {
        errors.parse_errors.push(format!(
            "offset {offset}: unsupported H4CR header length {header_len}"
        ));
        return None;
    }
    let payload_len = read_u32(bytes, 24) as usize;
    let expected_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
    if bytes.len() != expected_len {
        errors.parse_errors.push(format!(
            "offset {offset}: record length {} != header {COMPACT_RECORD_HEADER_LEN} + payload {payload_len}",
            bytes.len()
        ));
        return None;
    }
    let header = CompactRecordHeader {
        sequence: read_u64(bytes, 8),
        game_hour_raw: read_i32(bytes, 16),
        domain_id: read_u16(bytes, 20),
        record_type: read_u16(bytes, 22),
        payload_len: payload_len as u32,
        flags: read_u32(bytes, 28),
    };
    Some((header, &bytes[COMPACT_RECORD_HEADER_LEN..]))
}

fn merge_payload_summary(out: &mut Domain53RecordGateSummary, payload: Domain53GateSummary) {
    out.status_flags_or |= payload.status_flags_or;
    out.data_rows_total = out.data_rows_total.saturating_add(payload.data_rows_total);
    out.data_changed_rows_total = out
        .data_changed_rows_total
        .saturating_add(payload.data_changed_rows_total);
    out.parse_errors.extend(payload.parse_errors);
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16"))
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32"))
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("i32"))
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("u64"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::{
        build_domain53_payload, proof_fixture_rows, CURRENT_VALIDATED_SOURCE_PROOFS,
    };

    #[test]
    fn wraps_and_decodes_domain53_record() {
        let extraction =
            build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        assert!(extraction.ok(), "{:?}", extraction.errors);

        let record = encode_domain53_data_record(7, 43_800_024, &extraction.payload);
        let summary = decode_domain53_record(0, &record);

        assert_eq!(summary.record_count, 1);
        assert_eq!(summary.data_rows_total, 9);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn rejects_wrong_domain_id() {
        let extraction =
            build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        let mut record = encode_domain53_data_record(7, 43_800_024, &extraction.payload);
        record[20..22].copy_from_slice(&52u16.to_le_bytes());

        let summary = decode_domain53_record(0, &record);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("domain_id 52 != 53")));
    }
}
