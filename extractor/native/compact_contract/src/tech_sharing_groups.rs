//! Domain 69 `tech_sharing_groups_live` payload contract and encoder.

/// One technology-sharing group in the domain 69 snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechSharingGroupRow {
    pub id_token: u32,
    pub member_tag_db_ids: Vec<u32>,
}

const ROW_LEN: usize = 12;

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
