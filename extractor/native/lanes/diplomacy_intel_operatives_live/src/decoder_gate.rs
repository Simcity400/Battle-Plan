use crate::compact::{decode_payload, Domain53GateSummary, DOMAIN_ID_PROPOSED};
use std::collections::BTreeMap;

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = compact_contract::COMPACT_RECORD_MAGIC;
pub const COMPACT_RECORD_VERSION: u16 = compact_contract::COMPACT_RECORD_VERSION;
pub const COMPACT_RECORD_HEADER_LEN: usize = compact_contract::COMPACT_RECORD_HEADER_LEN;
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
    pub data_units_total: BTreeMap<&'static str, u64>,
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
    if let Err(error) = compact_contract::validate_compact_record_semantics(
        header.domain_id,
        header.record_type,
        header.flags,
        payload,
    ) {
        out.parse_errors.push(format!("offset {offset}: {error}"));
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
    let parsed = match compact_contract::parse_compact_record_header(bytes) {
        Ok(header) => header,
        Err(err) => {
            errors.parse_errors.push(format!("offset {offset}: {err}"));
            return None;
        }
    };
    let expected_len = parsed.record_len();
    if bytes.len() != expected_len {
        errors.parse_errors.push(format!(
            "offset {offset}: record length {} != header {COMPACT_RECORD_HEADER_LEN} + payload {}",
            bytes.len(),
            parsed.payload_len
        ));
        return None;
    }
    let header = CompactRecordHeader {
        sequence: parsed.sequence,
        game_hour_raw: parsed.game_hour_raw,
        domain_id: parsed.domain_id,
        record_type: parsed.record_type,
        payload_len: parsed.payload_len as u32,
        flags: parsed.record_flags,
    };
    Some((header, &bytes[COMPACT_RECORD_HEADER_LEN..]))
}

fn merge_payload_summary(out: &mut Domain53RecordGateSummary, payload: Domain53GateSummary) {
    out.status_flags_or |= payload.status_flags_or;
    out.data_rows_total = out.data_rows_total.saturating_add(payload.data_rows_total);
    out.data_changed_rows_total = out
        .data_changed_rows_total
        .saturating_add(payload.data_changed_rows_total);
    for (name, count) in payload.data_units_total {
        let entry = out.data_units_total.entry(name).or_default();
        *entry = entry.saturating_add(count);
    }
    out.parse_errors.extend(payload.parse_errors);
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
        assert_eq!(summary.data_rows_total, 18);
        assert_eq!(
            summary
                .data_units_total
                .get("agency_capacity_cluster_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("compact_state_event_rows")
                .copied(),
            Some(8)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("compact_state_event_relation_opinion_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("compact_state_event_country_faction_group_rows")
                .copied(),
            Some(1)
        );
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

    #[test]
    fn rejects_noncanonical_data_header_flags() {
        let extraction =
            build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        let mut record = encode_domain53_data_record(7, 43_800_024, &extraction.payload);
        record[28..32].copy_from_slice(
            &compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER.to_le_bytes(),
        );

        let summary = decode_domain53_record(0, &record);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("rejected bits")));
    }
}
