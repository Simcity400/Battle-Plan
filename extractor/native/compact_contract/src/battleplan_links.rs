use std::collections::BTreeSet;

pub const D54_LINK_VERSION: u32 = 2;
pub const D54_LINK_MAGIC: [u8; 4] = *b"D54L";
pub const MAX_LINKS_PER_DIRECTION: usize = 4096;
const HEADER_BYTES: usize = 16;
const ROW_BYTES: usize = 12;
const IDENTITY_BYTES: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum OrderLinkStatus {
    #[default]
    Absent = 0,
    Exact = 1,
    Unavailable = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrderLinkIdentity {
    pub owner_kind: u32,
    pub owner_id: u32,
    pub generation: u64,
    pub engine_order_instance_id: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderedOrderLinks {
    pub status: OrderLinkStatus,
    pub predecessors: Vec<OrderLinkIdentity>,
    pub successors: Vec<OrderLinkIdentity>,
}

impl OrderedOrderLinks {
    pub fn unavailable() -> Self {
        Self { status: OrderLinkStatus::Unavailable, ..Self::default() }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.status != OrderLinkStatus::Exact
            && (!self.predecessors.is_empty() || !self.successors.is_empty()) {
            return Err("D54 non-exact order links contain identities".into());
        }
        for links in [&self.predecessors, &self.successors] {
            if links.len() > MAX_LINKS_PER_DIRECTION {
                return Err("D54 ordered link vector exceeds its bound".into());
            }
            let mut seen = BTreeSet::new();
            for identity in links {
                if !matches!(identity.owner_kind, 0 | 1)
                    || identity.owner_id == 0 || identity.owner_id > i32::MAX as u32
                    || identity.generation == 0 || identity.engine_order_instance_id == 0
                    || identity.engine_order_instance_id > 100_000_000
                    || !seen.insert(*identity) {
                    return Err("D54 ordered link has invalid or duplicate semantic identity".into());
                }
            }
        }
        Ok(())
    }
}

pub fn encode_order_links(rows: &[OrderedOrderLinks]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&D54_LINK_MAGIC);
    out.extend_from_slice(&D54_LINK_VERSION.to_le_bytes());
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    let count = rows.iter().map(|row| row.predecessors.len() + row.successors.len()).sum::<usize>();
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for row in rows {
        for value in [row.status as u32, row.predecessors.len() as u32, row.successors.len() as u32] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    for row in rows {
        for identity in row.predecessors.iter().chain(&row.successors) {
            out.extend_from_slice(&identity.owner_kind.to_le_bytes());
            out.extend_from_slice(&identity.owner_id.to_le_bytes());
            out.extend_from_slice(&identity.generation.to_le_bytes());
            out.extend_from_slice(&identity.engine_order_instance_id.to_le_bytes());
        }
    }
    out
}

pub fn decode_order_links(bytes: &[u8], expected_rows: usize) -> Result<(Vec<OrderedOrderLinks>, usize), String> {
    let u32_at = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    if bytes.len() < HEADER_BYTES || bytes[..4] != D54_LINK_MAGIC
        || u32_at(4) != D54_LINK_VERSION || u32_at(8) as usize != expected_rows {
        return Err("D54 requires its version-2 ordered semantic link block".into());
    }
    let count = u32_at(12) as usize;
    let rows_end = expected_rows.checked_mul(ROW_BYTES).and_then(|n| n.checked_add(HEADER_BYTES))
        .ok_or("D54 link row length overflow")?;
    let end = count.checked_mul(IDENTITY_BYTES).and_then(|n| n.checked_add(rows_end))
        .ok_or("D54 link identity length overflow")?;
    if end > bytes.len() { return Err("D54 ordered link block is truncated".into()); }
    let mut rows = Vec::with_capacity(expected_rows);
    let mut cursor = rows_end;
    for index in 0..expected_rows {
        let at = HEADER_BYTES + index * ROW_BYTES;
        let status = match u32_at(at) {
            0 => OrderLinkStatus::Absent,
            1 => OrderLinkStatus::Exact,
            2 => OrderLinkStatus::Unavailable,
            _ => return Err("D54 ordered link status is invalid".into()),
        };
        let mut vectors = [Vec::new(), Vec::new()];
        for (direction, vector) in vectors.iter_mut().enumerate() {
            let count = u32_at(at + 4 + direction * 4) as usize;
            if count > MAX_LINKS_PER_DIRECTION || count > (end - cursor) / IDENTITY_BYTES {
                return Err("D54 ordered link child count is invalid".into());
            }
            for _ in 0..count {
                vector.push(OrderLinkIdentity {
                    owner_kind: u32_at(cursor), owner_id: u32_at(cursor + 4),
                    generation: u64::from_le_bytes(bytes[cursor + 8..cursor + 16].try_into().unwrap()),
                    engine_order_instance_id: u32_at(cursor + 16),
                });
                cursor += IDENTITY_BYTES;
            }
        }
        let [predecessors, successors] = vectors;
        let row = OrderedOrderLinks { status, predecessors, successors };
        row.validate()?;
        rows.push(row);
    }
    if cursor != end { return Err("D54 ordered link child accounting differs".into()); }
    Ok((rows, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_two_preserves_order_typed_identity_and_three_truth_states() {
        let a = OrderLinkIdentity { owner_kind: 0, owner_id: 7, generation: 12, engine_order_instance_id: 3 };
        let b = OrderLinkIdentity { owner_kind: 1, owner_id: 7, generation: 18, engine_order_instance_id: 3 };
        let rows = vec![OrderedOrderLinks::default(), OrderedOrderLinks::unavailable(),
            OrderedOrderLinks { status: OrderLinkStatus::Exact, predecessors: vec![], successors: vec![] },
            OrderedOrderLinks { status: OrderLinkStatus::Exact, predecessors: vec![b, a], successors: vec![a] }];
        let bytes = encode_order_links(&rows);
        assert_eq!(decode_order_links(&bytes, rows.len()).unwrap(), (rows.clone(), bytes.len()));
        for len in 0..bytes.len() { assert!(decode_order_links(&bytes[..len], rows.len()).is_err()); }
        let mut old = bytes.clone(); old[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_order_links(&old, rows.len()).is_err());
        assert!(decode_order_links(&[], 0).is_err());
        assert!(decode_order_links(&bytes, rows.len() + 1).is_err());
        for malformed in [
            OrderedOrderLinks { status: OrderLinkStatus::Unavailable, predecessors: vec![a], successors: vec![] },
            OrderedOrderLinks { status: OrderLinkStatus::Exact, predecessors: vec![a, a], successors: vec![] },
            OrderedOrderLinks { status: OrderLinkStatus::Exact, predecessors: vec![OrderLinkIdentity { generation: 0, ..a }], successors: vec![] },
        ] { assert!(decode_order_links(&encode_order_links(&[malformed]), 1).is_err()); }
    }
}
