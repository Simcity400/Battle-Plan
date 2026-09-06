use std::collections::BTreeSet;

pub const D54_LINK_VERSION: u32 = 5;
pub const D54_LINK_MAGIC: [u8; 4] = *b"D54L";
pub const MAX_LINKS_PER_DIRECTION: usize = 4096;
const HEADER_BYTES: usize = 16;
const ROW_BYTES: usize = 52;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OrderFrontBinding {
    #[default]
    Unavailable,
    Absent([u32; 3]),
    Exact([u32; 3]),
}

impl OrderFrontBinding {
    pub fn status(self) -> u32 {
        match self { Self::Unavailable => 0, Self::Absent(_) => 1, Self::Exact(_) => 2 }
    }

    pub fn words(self) -> Option<[u32; 3]> {
        match self { Self::Unavailable => None, Self::Absent(words) | Self::Exact(words) => Some(words) }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OrderedOrderLinks {
    pub status: OrderLinkStatus,
    pub root_status: OrderLinkStatus,
    pub root_positions: [Option<u32>; 3],
    pub front_binding: OrderFrontBinding,
    pub predecessors: Vec<OrderLinkIdentity>,
    pub successors: Vec<OrderLinkIdentity>,
    pub virtual_predecessors: Vec<OrderLinkIdentity>,
    pub virtual_successors: Vec<OrderLinkIdentity>,
}

impl OrderedOrderLinks {
    pub fn unavailable() -> Self {
        Self { status: OrderLinkStatus::Unavailable, ..Self::default() }
    }

    pub fn validate(&self) -> Result<(), String> {
        if (self.root_status != OrderLinkStatus::Exact && self.root_positions != [None; 3])
            || self.root_positions.iter().flatten().any(|slot| *slot >= 256) {
            return Err("D54 root positions violate status or container bound".into());
        }
        if self.status != OrderLinkStatus::Exact
            && (!self.predecessors.is_empty() || !self.successors.is_empty()
                || !self.virtual_predecessors.is_empty() || !self.virtual_successors.is_empty()) {
            return Err("D54 non-exact order links contain identities".into());
        }
        for links in [&self.predecessors, &self.successors, &self.virtual_predecessors, &self.virtual_successors] {
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
    let count = rows.iter().map(|row| row.predecessors.len() + row.successors.len() + row.virtual_predecessors.len() + row.virtual_successors.len()).sum::<usize>();
    out.extend_from_slice(&(count as u32).to_le_bytes());
    for row in rows {
        let front = row.front_binding.words().unwrap_or([0; 3]);
        for value in [row.status as u32, row.predecessors.len() as u32, row.successors.len() as u32, row.virtual_predecessors.len() as u32, row.virtual_successors.len() as u32, row.root_status as u32,
            row.root_positions[0].unwrap_or(u32::MAX), row.root_positions[1].unwrap_or(u32::MAX),
            row.root_positions[2].unwrap_or(u32::MAX), row.front_binding.status(), front[0], front[1], front[2]] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    for row in rows {
        for identity in row.predecessors.iter().chain(&row.successors).chain(&row.virtual_predecessors).chain(&row.virtual_successors) {
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
        return Err("D54 requires its version-5 ordered semantic link/root/front block".into());
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
        let mut vectors: [Vec<OrderLinkIdentity>; 4] = std::array::from_fn(|_| Vec::new());
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
        let [predecessors, successors, virtual_predecessors, virtual_successors] = vectors;
        let root_status = match u32_at(at + 20) {
            0 => OrderLinkStatus::Absent, 1 => OrderLinkStatus::Exact, 2 => OrderLinkStatus::Unavailable,
            _ => return Err("D54 root status is invalid".into()),
        };
        let root_positions = std::array::from_fn(|i| match u32_at(at + 24 + i * 4) {
            u32::MAX => None, value => Some(value),
        });
        let words = std::array::from_fn(|i| u32_at(at + 40 + i * 4));
        let front_binding = match u32_at(at + 36) {
            0 if words == [0; 3] => OrderFrontBinding::Unavailable,
            1 => OrderFrontBinding::Absent(words),
            2 => OrderFrontBinding::Exact(words),
            _ => return Err("D54 front binding has invalid status/payload".into()),
        };
        let row = OrderedOrderLinks { status, root_status, root_positions, front_binding,
            predecessors, successors, virtual_predecessors, virtual_successors };
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
    fn version_five_preserves_links_roots_and_front_binding_truth() {
        let a = OrderLinkIdentity { owner_kind: 0, owner_id: 7, generation: 12, engine_order_instance_id: 3 };
        let b = OrderLinkIdentity { owner_kind: 1, owner_id: 7, generation: 18, engine_order_instance_id: 3 };
        let rows = vec![OrderedOrderLinks::default(), OrderedOrderLinks::unavailable(),
            OrderedOrderLinks { front_binding: Default::default(), root_status: OrderLinkStatus::Absent, root_positions: [None; 3], status: OrderLinkStatus::Exact, predecessors: vec![], successors: vec![], virtual_predecessors: vec![], virtual_successors: vec![] },
            OrderedOrderLinks { front_binding: Default::default(), root_status: OrderLinkStatus::Absent, root_positions: [None; 3], status: OrderLinkStatus::Exact, predecessors: vec![b, a], successors: vec![a], virtual_predecessors: vec![a, b], virtual_successors: vec![b, a] }];
        let mut rows = rows;
        rows[2].root_status = OrderLinkStatus::Exact;
        rows[3].root_status = OrderLinkStatus::Exact;
        rows[3].root_positions = [Some(0), None, Some(2)];
        rows[2].front_binding = OrderFrontBinding::Absent([0, 0, 81]);
        rows[3].front_binding = OrderFrontBinding::Exact([77, 1234567, 81]);
        let bytes = encode_order_links(&rows);
        for status in [OrderLinkStatus::Absent, OrderLinkStatus::Unavailable] {
            let mut malformed = rows[3].clone(); malformed.root_status = status;
            assert!(decode_order_links(&encode_order_links(&[malformed]), 1).is_err());
        }
        let mut malformed = rows[3].clone(); malformed.root_positions[1] = Some(256);
        assert!(decode_order_links(&encode_order_links(&[malformed]), 1).is_err());
        let mut changed_root = rows.clone(); changed_root[3].root_positions[2] = Some(1);
        assert_ne!(encode_order_links(&changed_root), bytes);
        let mut unread_links = OrderedOrderLinks::unavailable();
        unread_links.root_status = OrderLinkStatus::Exact;
        unread_links.root_positions = [None, Some(0), None];
        let encoded = encode_order_links(&[unread_links.clone()]);
        assert_eq!(decode_order_links(&encoded, 1).unwrap().0, [unread_links]);
        for field in 0..3 {
            let mut changed = rows.clone();
            let mut words = changed[3].front_binding.words().unwrap(); words[field] += 1;
            changed[3].front_binding = OrderFrontBinding::Exact(words);
            assert_ne!(encode_order_links(&changed), bytes);
            assert_eq!(decode_order_links(&encode_order_links(&changed), changed.len()).unwrap().0, changed);
        }
        let mut invalid = encode_order_links(&[OrderedOrderLinks::default()]);
        invalid[16 + 40..16 + 44].copy_from_slice(&1u32.to_le_bytes());
        assert!(decode_order_links(&invalid, 1).is_err());
        invalid[16 + 36..16 + 40].copy_from_slice(&3u32.to_le_bytes());
        assert!(decode_order_links(&invalid, 1).is_err());
        let mut changed = rows.clone();
        changed[3].virtual_successors.reverse();
        assert_ne!(encode_order_links(&changed), bytes);
        assert_eq!(changed[3].predecessors, rows[3].predecessors);
        assert_eq!(changed[3].successors, rows[3].successors);
        assert_eq!(decode_order_links(&bytes, rows.len()).unwrap(), (rows.clone(), bytes.len()));
        for len in 0..bytes.len() { assert!(decode_order_links(&bytes[..len], rows.len()).is_err()); }
        let mut old = bytes.clone(); old[4..8].copy_from_slice(&4u32.to_le_bytes());
        assert!(decode_order_links(&old, rows.len()).is_err());
        assert!(decode_order_links(&[], 0).is_err());
        assert!(decode_order_links(&bytes, rows.len() + 1).is_err());
        for direction in 0..2 {
            let mut malformed = rows[3].clone();
            if direction == 0 { malformed.virtual_predecessors = vec![a, a]; }
            else { malformed.virtual_successors = vec![a; MAX_LINKS_PER_DIRECTION + 1]; }
            assert!(decode_order_links(&encode_order_links(&[malformed]), 1).is_err());
            let mut nonexact = OrderedOrderLinks::unavailable();
            nonexact.virtual_successors = vec![a];
            assert!(decode_order_links(&encode_order_links(&[nonexact]), 1).is_err());
        }
        for malformed in [
            OrderedOrderLinks { front_binding: Default::default(), root_status: OrderLinkStatus::Absent, root_positions: [None; 3], status: OrderLinkStatus::Unavailable, predecessors: vec![a], successors: vec![], virtual_predecessors: vec![], virtual_successors: vec![] },
            OrderedOrderLinks { front_binding: Default::default(), root_status: OrderLinkStatus::Absent, root_positions: [None; 3], status: OrderLinkStatus::Exact, predecessors: vec![a, a], successors: vec![], virtual_predecessors: vec![], virtual_successors: vec![] },
            OrderedOrderLinks { front_binding: Default::default(), root_status: OrderLinkStatus::Absent, root_positions: [None; 3], status: OrderLinkStatus::Exact, predecessors: vec![OrderLinkIdentity { generation: 0, ..a }], successors: vec![], virtual_predecessors: vec![], virtual_successors: vec![] },
        ] { assert!(decode_order_links(&encode_order_links(&[malformed]), 1).is_err()); }
    }
}
