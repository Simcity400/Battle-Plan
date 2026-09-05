pub const FORMAT_MARKER: u32 = u32::from_le_bytes(*b"D2R1");
pub const ROW_LEN: usize = 128;
pub const MAX_ROUTE_PROVINCES: usize = 65_536;

pub fn append_routes<'a>(
    payload: &mut Vec<u8>,
    routes: impl ExactSizeIterator<Item = Option<&'a [u32]>>,
) {
    let rows = routes.len();
    assert_eq!(payload.len(), 32 + rows * ROW_LEN);
    payload[28..32].copy_from_slice(&FORMAT_MARKER.to_le_bytes());
    let mut cursor = 0u32;
    for (index, route) in routes.enumerate() {
        let Some(route) = route else { continue };
        assert!(route.len() <= MAX_ROUTE_PROVINCES);
        let descriptor = 32 + index * ROW_LEN + 120;
        payload[descriptor..descriptor + 4].copy_from_slice(&cursor.to_le_bytes());
        payload[descriptor + 4..descriptor + 8]
            .copy_from_slice(&(route.len() as u32 + 1).to_le_bytes());
        for province in route {
            payload.extend_from_slice(&province.to_le_bytes());
        }
        cursor = cursor
            .checked_add(route.len() as u32)
            .expect("bounded division route pool");
    }
}

pub struct DivisionRoutes<'a> {
    payload: &'a [u8],
    rows: usize,
    pool: usize,
}

impl<'a> DivisionRoutes<'a> {
    pub fn parse(payload: &'a [u8]) -> Result<Self, String> {
        if payload.len() < 32 {
            return Err("division route header truncated".into());
        }
        if word(payload, 28) != FORMAT_MARKER {
            return Err("unsupported division route format marker".into());
        }
        let rows = word(payload, 8) as usize;
        let pool = rows
            .checked_mul(ROW_LEN)
            .and_then(|size| size.checked_add(32))
            .ok_or("division route row length overflow")?;
        if pool > payload.len() || (payload.len() - pool) % 4 != 0 {
            return Err("division route row or pool length invalid".into());
        }
        let mut consumed = 0usize;
        for index in 0..rows {
            let base = 32 + index * ROW_LEN;
            let start = word(payload, base + 120) as usize;
            let count = word(payload, base + 124) as usize;
            if count == 0 {
                if start != 0 {
                    return Err("unavailable division route has pool offset".into());
                }
                continue;
            }
            if word(payload, base + 8) & (1 << 12) != 0 {
                return Err("deleted division carries a route".into());
            }
            let len = count - 1;
            if len > MAX_ROUTE_PROVINCES || start != consumed {
                return Err("division route exceeds bound or breaks ordered pool custody".into());
            }
            consumed = consumed
                .checked_add(len)
                .ok_or("division route pool overflow")?;
            if consumed > (payload.len() - pool) / 4 {
                return Err("division route extends beyond pool".into());
            }
        }
        if consumed != (payload.len() - pool) / 4 {
            return Err("division route pool contains unowned provinces".into());
        }
        for province in payload[pool..].chunks_exact(4) {
            let id = u32::from_le_bytes(province.try_into().unwrap());
            if id == 0 || id >= i32::MAX as u32 {
                return Err("division route contains invalid province identity".into());
            }
        }
        Ok(Self {
            payload,
            rows,
            pool,
        })
    }

    pub fn route(&self, index: usize) -> Option<Vec<u32>> {
        assert!(index < self.rows);
        let base = 32 + index * ROW_LEN;
        let count = word(self.payload, base + 124) as usize;
        if count == 0 {
            return None;
        }
        let start = self.pool + word(self.payload, base + 120) as usize * 4;
        Some(
            self.payload[start..start + (count - 1) * 4]
                .chunks_exact(4)
                .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()))
                .collect(),
        )
    }
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(routes: &[Option<Vec<u32>>]) -> Vec<u8> {
        let mut bytes = vec![0; 32 + routes.len() * ROW_LEN];
        bytes[8..12].copy_from_slice(&(routes.len() as u32).to_le_bytes());
        append_routes(&mut bytes, routes.iter().map(|route| route.as_deref()));
        bytes
    }

    #[test]
    fn legacy_row_followed_by_zero_words_cannot_alias_route_layout() {
        let mut legacy = vec![0; 32 + 120];
        legacy[8..12].copy_from_slice(&1u32.to_le_bytes());
        legacy[32..36].copy_from_slice(&7u32.to_le_bytes());
        legacy[36..40].copy_from_slice(&4713u32.to_le_bytes());
        legacy.extend_from_slice(&[0; 8]);
        assert_eq!(
            DivisionRoutes::parse(&legacy).err().unwrap(),
            "unsupported division route format marker"
        );
        legacy[28..32].copy_from_slice(&FORMAT_MARKER.to_le_bytes());
        assert_eq!(DivisionRoutes::parse(&legacy).unwrap().route(0), None);
    }

    #[test]
    fn route_roundtrip_preserves_unknown_empty_order_and_repeated_provinces() {
        let routes = [None, Some(vec![]), Some(vec![9, 7, 9]), Some(vec![11])];
        let bytes = payload(&routes);
        let parsed = DivisionRoutes::parse(&bytes).unwrap();
        for (index, route) in routes.iter().enumerate() {
            assert_eq!(&parsed.route(index), route);
        }
    }

    #[test]
    fn route_pool_refuses_alias_truncation_unowned_invalid_and_deleted_rows() {
        let bytes = payload(&[Some(vec![9]), Some(vec![7])]);
        for (offset, value) in [
            (32 + ROW_LEN + 120, 0u32),
            (32 + 124, 4),
            (32 + 8, 1 << 12),
            (32 + 2 * ROW_LEN, 0),
        ] {
            let mut invalid = bytes.clone();
            invalid[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(DivisionRoutes::parse(&invalid).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.extend_from_slice(&1u32.to_le_bytes());
        assert!(DivisionRoutes::parse(&trailing).is_err());
        assert!(DivisionRoutes::parse(&bytes[..bytes.len() - 1]).is_err());
        let mut absent_offset = payload(&[None]);
        absent_offset[152..156].copy_from_slice(&1u32.to_le_bytes());
        assert!(DivisionRoutes::parse(&absent_offset).is_err());
    }
}
