pub const FORMAT_MARKER: [u8; 4] = *b"D26A";
pub const HEADER_LEN: usize = 44;
pub const ROOT_ROW_LEN: usize = 24;
pub const ENTRY_ROW_LEN: usize = 16;
pub const MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub variant_id: u32,
    pub source_ordinal: u32,
    pub amount_raw: i64,
}

pub fn validate_header(payload: &[u8]) -> Result<(), String> {
    if payload.len() < HEADER_LEN || payload[40..44] != FORMAT_MARKER {
        return Err("division_equipment unsupported allocation grammar".into());
    }
    Ok(())
}

pub fn parse_allocation(row: &[u8]) -> Result<Allocation, String> {
    if row.len() != ENTRY_ROW_LEN {
        return Err("division_equipment allocation length is invalid".into());
    }
    let variant_id = u32::from_le_bytes(row[..4].try_into().unwrap());
    let source_ordinal = u32::from_le_bytes(row[4..8].try_into().unwrap());
    let amount_raw = i64::from_le_bytes(row[8..].try_into().unwrap());
    if variant_id == 0 || variant_id > i32::MAX as u32 {
        return Err("division_equipment allocation variant identity is invalid".into());
    }
    if !(0..=MAX_AMOUNT_RAW).contains(&amount_raw) {
        return Err("division_equipment allocation amount is invalid".into());
    }
    Ok(Allocation { variant_id, source_ordinal, amount_raw })
}

pub fn append_allocation(out: &mut Vec<u8>, row: Allocation) {
    out.extend_from_slice(&row.variant_id.to_le_bytes());
    out.extend_from_slice(&row.source_ordinal.to_le_bytes());
    out.extend_from_slice(&row.amount_raw.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_preserves_zero_exact_amount_and_source_order() {
        let rows = [
            Allocation { variant_id: 17, source_ordinal: 0, amount_raw: 0 },
            Allocation { variant_id: 3, source_ordinal: 1, amount_raw: MAX_AMOUNT_RAW - 1 },
        ];
        let mut bytes = Vec::new();
        for row in rows { append_allocation(&mut bytes, row); }
        assert_eq!(bytes.chunks_exact(ENTRY_ROW_LEN).map(parse_allocation).collect::<Result<Vec<_>, _>>().unwrap(), rows);
        let mut invalid = bytes[..ENTRY_ROW_LEN].to_vec();
        invalid[8..].copy_from_slice(&(-1_i64).to_le_bytes());
        assert!(parse_allocation(&invalid).is_err());
        assert!(parse_allocation(&invalid[..15]).is_err());
    }

    #[test]
    fn legacy_allocation_header_cannot_claim_ordered_census() {
        let mut legacy = vec![0; 40 + ROOT_ROW_LEN + ENTRY_ROW_LEN];
        legacy[40..44].copy_from_slice(&7_u32.to_le_bytes());
        assert!(validate_header(&legacy).is_err());
        legacy.splice(40..40, FORMAT_MARKER);
        assert!(validate_header(&legacy).is_ok());
        for len in 0..HEADER_LEN { assert!(validate_header(&legacy[..len]).is_err()); }
    }
}
