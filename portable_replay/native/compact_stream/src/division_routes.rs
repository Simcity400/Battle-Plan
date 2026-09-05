pub const FORMAT_MARKER: u32 = u32::from_le_bytes(*b"D2R4");
pub const ROW_LEN: usize = 184;
pub const MAX_ROUTE_PROVINCES: usize = 65_536;
pub const MAX_MODEL_ENTITY_BYTES: usize = 4096;
pub const MAX_ACCLIMATIZATION_CATEGORIES: usize = 65_536;
pub const ACCLIMATIZATION_SCALE: i64 = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcclimatizationCategory {
    pub category: String,
    pub weight_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acclimatization {
    pub maximum_raw: i64,
    pub categories: Vec<AcclimatizationCategory>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateModelPreference {
    pub template_id: u32,
    pub entity: String,
    pub category: Option<u64>,
}

pub fn append_inputs<'a>(
    payload: &mut Vec<u8>,
    routes: impl ExactSizeIterator<Item = Option<&'a [u32]>>,
    preferences: impl ExactSizeIterator<Item = Option<&'a TemplateModelPreference>>,
    acclimatization: impl ExactSizeIterator<Item = Option<&'a Acclimatization>>,
) {
    let rows = routes.len();
    assert_eq!(rows, preferences.len());
    assert_eq!(rows, acclimatization.len());
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
    let mut cursor = 0u32;
    for (index, preference) in preferences.enumerate() {
        let Some(preference) = preference else {
            continue;
        };
        assert!(preference.entity.len() <= MAX_MODEL_ENTITY_BYTES);
        let base = 32 + index * ROW_LEN;
        assert!(preference.template_id > 0 && preference.template_id <= i32::MAX as u32);
        payload[base + 144..base + 148].copy_from_slice(&preference.template_id.to_le_bytes());
        payload[base + 148..base + 152]
            .copy_from_slice(&u32::from(preference.category.is_some()).to_le_bytes());
        payload[base + 152..base + 160]
            .copy_from_slice(&preference.category.unwrap_or(0).to_le_bytes());
        payload[base + 160..base + 164].copy_from_slice(&cursor.to_le_bytes());
        payload[base + 164..base + 168]
            .copy_from_slice(&(preference.entity.len() as u32 + 1).to_le_bytes());
        payload.extend_from_slice(preference.entity.as_bytes());
        cursor = cursor
            .checked_add(preference.entity.len() as u32)
            .expect("bounded template entity pool");
    }
    let mut cursor = 0u32;
    for (index, value) in acclimatization.enumerate() {
        let Some(value) = value else { continue };
        assert!(value.categories.len() <= MAX_ACCLIMATIZATION_CATEGORIES);
        let base = 32 + index * ROW_LEN;
        payload[base + 168..base + 172].copy_from_slice(&cursor.to_le_bytes());
        payload[base + 172..base + 176]
            .copy_from_slice(&(value.categories.len() as u32 + 1).to_le_bytes());
        payload[base + 176..base + 184].copy_from_slice(&value.maximum_raw.to_le_bytes());
        for entry in &value.categories {
            assert!(!entry.category.is_empty() && entry.category.len() <= MAX_MODEL_ENTITY_BYTES);
            payload.extend_from_slice(&(entry.category.len() as u16).to_le_bytes());
            payload.extend_from_slice(entry.category.as_bytes());
            payload.extend_from_slice(&entry.weight_raw.to_le_bytes());
            cursor = cursor
                .checked_add(10 + entry.category.len() as u32)
                .expect("bounded acclimatization pool");
        }
    }
}

pub fn write_current_template(payload: &mut [u8], index: usize, value: Option<Option<u32>>) {
    let base = 32 + index * ROW_LEN;
    let (status, id) = match value {
        None => (0u32, 0),
        Some(None) => (1, 0),
        Some(Some(id)) => {
            assert!(id > 0 && id <= i32::MAX as u32);
            (2, id)
        }
    };
    payload[base + 128..base + 132].copy_from_slice(&status.to_le_bytes());
    payload[base + 132..base + 136].copy_from_slice(&id.to_le_bytes());
}

pub fn write_route_head(payload: &mut [u8], index: usize, value: Option<Option<u32>>) {
    let base = 32 + index * ROW_LEN;
    let (status, id) = match value {
        None => (0u32, 0),
        Some(None) => (1, 0),
        Some(Some(id)) => {
            assert!(id > 0 && id <= i32::MAX as u32);
            (2, id)
        }
    };
    payload[base + 136..base + 140].copy_from_slice(&status.to_le_bytes());
    payload[base + 140..base + 144].copy_from_slice(&id.to_le_bytes());
}

pub struct DivisionRoutes<'a> {
    payload: &'a [u8],
    rows: usize,
    pool: usize,
    entity_pool: usize,
    acclimatization_pool: usize,
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
        if pool > payload.len() {
            return Err("division route row or pool length invalid".into());
        }
        let mut consumed = 0usize;
        for index in 0..rows {
            let base = 32 + index * ROW_LEN;
            let template_status = word(payload, base + 128);
            let template_id = word(payload, base + 132);
            if !matches!((template_status, template_id), (0 | 1, 0))
                && !(template_status == 2 && template_id > 0 && template_id <= i32::MAX as u32)
            {
                return Err("division current template has invalid status or identity".into());
            }
            if word(payload, base + 8) & (1 << 12) != 0 && template_status != 0 {
                return Err("deleted division carries current template".into());
            }
            let head_status = word(payload, base + 136);
            let head_id = word(payload, base + 140);
            if !matches!((head_status, head_id), (0 | 1, 0))
                && !(head_status == 2 && head_id > 0 && head_id <= i32::MAX as u32)
            {
                return Err("division route head has invalid status or identity".into());
            }
            if word(payload, base + 8) & (1 << 12) != 0 && head_status != 0 {
                return Err("deleted division carries route head".into());
            }
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
        let entity_pool = pool + consumed * 4;
        let mut text_consumed = 0usize;
        for index in 0..rows {
            let base = 32 + index * ROW_LEN;
            let template = word(payload, base + 144);
            let present = word(payload, base + 148);
            let category = u64::from_le_bytes(payload[base + 152..base + 160].try_into().unwrap());
            let start = word(payload, base + 160) as usize;
            let count = word(payload, base + 164) as usize;
            if template == 0 {
                if present != 0 || category != 0 || start != 0 || count != 0 {
                    return Err("unavailable template preference carries values".into());
                }
                continue;
            }
            if word(payload, base + 8) & (1 << 12) != 0
                || word(payload, base + 128) != 2
                || word(payload, base + 132) != template
            {
                return Err("template preference current-template custody mismatch".into());
            }
            if present > 1 || (present == 0 && category != 0) {
                return Err("template preference category presence invalid".into());
            }
            if count == 0 || count - 1 > MAX_MODEL_ENTITY_BYTES || start != text_consumed {
                return Err("template preference entity length or pool custody invalid".into());
            }
            text_consumed = text_consumed
                .checked_add(count - 1)
                .ok_or("template entity pool overflow")?;
            let bytes = payload
                .get(entity_pool + start..entity_pool + text_consumed)
                .ok_or("template preference entity truncated")?;
            std::str::from_utf8(bytes).map_err(|_| "template preference entity invalid UTF-8")?;
        }
        let acclimatization_pool = entity_pool + text_consumed;
        let mut acclimatization_consumed = 0usize;
        for index in 0..rows {
            let base = 32 + index * ROW_LEN;
            let start = word(payload, base + 168) as usize;
            let count = word(payload, base + 172) as usize;
            if count == 0 {
                if start != 0 || payload[base + 176..base + 184] != [0; 8] {
                    return Err("unavailable acclimatization carries values".into());
                }
                continue;
            }
            if word(payload, base + 8) & (1 << 12) != 0
                || count - 1 > MAX_ACCLIMATIZATION_CATEGORIES
                || start != acclimatization_consumed
            {
                return Err("acclimatization census or pool custody invalid".into());
            }
            let mut cursor = acclimatization_pool + start;
            let mut keys = std::collections::BTreeSet::new();
            for _ in 0..count - 1 {
                let (key, _) = read_acclimatization_category(payload, &mut cursor)?;
                if !keys.insert(key) {
                    return Err("acclimatization repeats a category".into());
                }
            }
            acclimatization_consumed = cursor - acclimatization_pool;
        }
        if acclimatization_pool + acclimatization_consumed != payload.len() {
            return Err("division input pool contains unowned bytes".into());
        }
        for province in payload[pool..entity_pool].chunks_exact(4) {
            let id = u32::from_le_bytes(province.try_into().unwrap());
            if id == 0 || id >= i32::MAX as u32 {
                return Err("division route contains invalid province identity".into());
            }
        }
        Ok(Self {
            payload,
            rows,
            pool,
            entity_pool,
            acclimatization_pool,
        })
    }

    pub fn current_template(&self, index: usize) -> Option<Option<u32>> {
        assert!(index < self.rows);
        let base = 32 + index * ROW_LEN;
        match word(self.payload, base + 128) {
            0 => None,
            1 => Some(None),
            2 => Some(Some(word(self.payload, base + 132))),
            _ => unreachable!("validated template status"),
        }
    }

    pub fn route_head(&self, index: usize) -> Option<Option<u32>> {
        assert!(index < self.rows);
        let base = 32 + index * ROW_LEN;
        match word(self.payload, base + 136) {
            0 => None,
            1 => Some(None),
            2 => Some(Some(word(self.payload, base + 140))),
            _ => unreachable!("validated route head status"),
        }
    }

    pub fn template_preference(&self, index: usize) -> Option<TemplateModelPreference> {
        assert!(index < self.rows);
        let base = 32 + index * ROW_LEN;
        let template_id = word(self.payload, base + 144);
        if template_id == 0 {
            return None;
        }
        let category = (word(self.payload, base + 148) == 1)
            .then(|| u64::from_le_bytes(self.payload[base + 152..base + 160].try_into().unwrap()));
        let start = self.entity_pool + word(self.payload, base + 160) as usize;
        let len = word(self.payload, base + 164) as usize - 1;
        let entity = std::str::from_utf8(&self.payload[start..start + len])
            .unwrap()
            .to_owned();
        Some(TemplateModelPreference {
            template_id,
            entity,
            category,
        })
    }

    pub fn acclimatization(&self, index: usize) -> Option<Acclimatization> {
        assert!(index < self.rows);
        let base = 32 + index * ROW_LEN;
        let count = word(self.payload, base + 172) as usize;
        if count == 0 {
            return None;
        }
        let maximum_raw =
            i64::from_le_bytes(self.payload[base + 176..base + 184].try_into().unwrap());
        let mut cursor = self.acclimatization_pool + word(self.payload, base + 168) as usize;
        let categories = (0..count - 1)
            .map(|_| {
                let (category, weight_raw) =
                    read_acclimatization_category(self.payload, &mut cursor).unwrap();
                AcclimatizationCategory {
                    category: category.to_owned(),
                    weight_raw,
                }
            })
            .collect();
        Some(Acclimatization {
            maximum_raw,
            categories,
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

fn read_acclimatization_category<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
) -> Result<(&'a str, i64), String> {
    let length = bytes
        .get(
            *cursor
                ..cursor
                    .checked_add(2)
                    .ok_or("acclimatization length overflow")?,
        )
        .ok_or("acclimatization length truncated")?;
    let length = u16::from_le_bytes(length.try_into().unwrap()) as usize;
    if length == 0 || length > MAX_MODEL_ENTITY_BYTES {
        return Err("acclimatization category length invalid".into());
    }
    *cursor += 2;
    let end = cursor
        .checked_add(length)
        .ok_or("acclimatization key overflow")?;
    let key = std::str::from_utf8(
        bytes
            .get(*cursor..end)
            .ok_or("acclimatization key truncated")?,
    )
    .map_err(|_| "acclimatization category invalid UTF-8")?;
    *cursor = end;
    let end = cursor
        .checked_add(8)
        .ok_or("acclimatization weight overflow")?;
    let raw = i64::from_le_bytes(
        bytes
            .get(*cursor..end)
            .ok_or("acclimatization weight truncated")?
            .try_into()
            .unwrap(),
    );
    *cursor = end;
    Ok((key, raw))
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
        append_inputs(
            &mut bytes,
            routes.iter().map(|route| route.as_deref()),
            std::iter::repeat_n(None, routes.len()),
            std::iter::repeat_n(None, routes.len()),
        );
        bytes
    }

    #[test]
    fn legacy_row_followed_by_zero_words_cannot_alias_route_layout() {
        let mut legacy = vec![0; 32 + 120];
        legacy[8..12].copy_from_slice(&1u32.to_le_bytes());
        legacy[32..36].copy_from_slice(&7u32.to_le_bytes());
        legacy[36..40].copy_from_slice(&4713u32.to_le_bytes());
        legacy.extend_from_slice(&[0; 64]);
        assert_eq!(
            DivisionRoutes::parse(&legacy).err().unwrap(),
            "unsupported division route format marker"
        );
        legacy[28..32].copy_from_slice(&FORMAT_MARKER.to_le_bytes());
        assert_eq!(DivisionRoutes::parse(&legacy).unwrap().route(0), None);
    }

    #[test]
    fn current_template_preserves_unknown_absence_and_identity_and_refuses_previous_grammar() {
        let mut bytes = payload(&[None, None, None]);
        for (index, value) in [None, Some(None), Some(Some(73))].into_iter().enumerate() {
            write_current_template(&mut bytes, index, value);
        }
        let parsed = DivisionRoutes::parse(&bytes).unwrap();
        assert_eq!(parsed.current_template(0), None);
        assert_eq!(parsed.current_template(1), Some(None));
        assert_eq!(parsed.current_template(2), Some(Some(73)));
        for (offset, value) in [(32 + 128, 3), (32 + 132, 1), (32 + 2 * ROW_LEN + 132, 0)] {
            let mut invalid = bytes.clone();
            invalid[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
            assert!(DivisionRoutes::parse(&invalid).is_err());
        }
        bytes[28..32].copy_from_slice(b"D2R1");
        assert_eq!(
            DivisionRoutes::parse(&bytes).err().unwrap(),
            "unsupported division route format marker"
        );
    }

    #[test]
    fn template_preferences_share_pool_without_aliasing_routes_or_current_identity() {
        let preferences = [
            None,
            Some(TemplateModelPreference {
                template_id: 73,
                entity: String::new(),
                category: None,
            }),
            Some(TemplateModelPreference {
                template_id: 74,
                entity: "armour_entity".into(),
                category: Some(0),
            }),
        ];
        let routes = [None, Some(vec![]), Some(vec![19, 17, 19])];
        let mut bytes = vec![0; 32 + 3 * ROW_LEN];
        bytes[8..12].copy_from_slice(&3u32.to_le_bytes());
        for (index, preference) in preferences.iter().enumerate() {
            if let Some(preference) = preference {
                write_current_template(&mut bytes, index, Some(Some(preference.template_id)));
            }
            write_route_head(&mut bytes, index, [None, Some(None), Some(Some(23))][index]);
        }
        append_inputs(
            &mut bytes,
            routes.iter().map(|r| r.as_deref()),
            preferences.iter().map(|p| p.as_ref()),
            std::iter::repeat_n(None, routes.len()),
        );
        let parsed = DivisionRoutes::parse(&bytes).unwrap();
        for index in 0..3 {
            assert_eq!(parsed.template_preference(index), preferences[index]);
            assert_eq!(parsed.route(index), routes[index]);
        }
        assert_eq!(parsed.route_head(0), None);
        assert_eq!(parsed.route_head(1), Some(None));
        assert_eq!(parsed.route_head(2), Some(Some(23)));
        for (offset, value) in [
            (32 + 136, 3),
            (32 + 140, 1),
            (32 + ROW_LEN + 132, 74),
            (32 + ROW_LEN + 148, 2),
            (32 + ROW_LEN + 152, 1),
            (32 + 2 * ROW_LEN + 160, 1),
            (32 + 2 * ROW_LEN + 164, MAX_MODEL_ENTITY_BYTES as u32 + 2),
            (32 + 2 * ROW_LEN + 8, 1 << 12),
        ] {
            let mut invalid = bytes.clone();
            invalid[offset..offset + 4].copy_from_slice(&u32::to_le_bytes(value));
            assert!(DivisionRoutes::parse(&invalid).is_err(), "offset {offset}");
        }
        let mut invalid = bytes.clone();
        *invalid.last_mut().unwrap() = 0xff;
        assert!(DivisionRoutes::parse(&invalid).is_err());
        assert!(DivisionRoutes::parse(&bytes[..bytes.len() - 1]).is_err());
        bytes[28..32].copy_from_slice(b"D2R2");
        assert!(DivisionRoutes::parse(&bytes)
            .err()
            .unwrap()
            .contains("unsupported"));
    }

    #[test]
    fn acclimatization_pool_preserves_unknown_empty_signed_order_and_refuses_aliases() {
        let values = [
            None,
            Some(Acclimatization {
                maximum_raw: 0,
                categories: vec![],
            }),
            Some(Acclimatization {
                maximum_raw: 10_000_000,
                categories: vec![
                    AcclimatizationCategory {
                        category: "cold".into(),
                        weight_raw: -1,
                    },
                    AcclimatizationCategory {
                        category: "hot".into(),
                        weight_raw: 123_456,
                    },
                ],
            }),
        ];
        let mut bytes = vec![0; 32 + 3 * ROW_LEN];
        bytes[8..12].copy_from_slice(&3u32.to_le_bytes());
        append_inputs(
            &mut bytes,
            std::iter::repeat_n(None, 3),
            std::iter::repeat_n(None, 3),
            values.iter().map(|v| v.as_ref()),
        );
        let parsed = DivisionRoutes::parse(&bytes).unwrap();
        for (index, expected) in values.iter().enumerate() {
            assert_eq!(&parsed.acclimatization(index), expected);
        }
        for (at, value) in [
            (32 + 168, 1),
            (32 + 176, 1),
            (32 + 2 * ROW_LEN + 168, 1),
            (
                32 + 2 * ROW_LEN + 172,
                MAX_ACCLIMATIZATION_CATEGORIES as u32 + 2,
            ),
            (32 + ROW_LEN + 8, 1 << 12),
        ] {
            let mut invalid = bytes.clone();
            invalid[at..at + 4].copy_from_slice(&u32::to_le_bytes(value));
            assert!(DivisionRoutes::parse(&invalid).is_err(), "offset {at}");
        }
        let mut invalid = bytes.clone();
        invalid[32 + 3 * ROW_LEN + 2] = 0xff;
        assert!(DivisionRoutes::parse(&invalid).is_err());
        assert!(DivisionRoutes::parse(&bytes[..bytes.len() - 1]).is_err());
        bytes[28..32].copy_from_slice(b"D2R3");
        assert!(DivisionRoutes::parse(&bytes).is_err());
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
