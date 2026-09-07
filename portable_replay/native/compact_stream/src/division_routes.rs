pub const FORMAT_MARKER: u32 = u32::from_le_bytes(*b"D2R8");
pub const ROW_LEN: usize = 216;
pub const MAX_ROUTE_PROVINCES: usize = 65_536;
pub const MAX_TEMPLATE_EXCLUSIONS: usize = 65_536;
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
    pub excluded_variants: Option<Vec<Option<u32>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CombatClass { Land, BorderWar }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatIdentity {
    pub class: CombatClass,
    pub kind: u32,
    pub id: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MovementScalars {
    pub numerator_raw: Option<i64>,
    pub naval_transfer_membership: Option<bool>,
    pub movement_target_present: Option<bool>,
    pub retreat_raw: Option<u8>,
    pub first_combat: Option<Option<CombatIdentity>>,
}

pub fn write_movement_scalars(payload: &mut [u8], index: usize, value: MovementScalars) {
    let base = 32 + index * ROW_LEN;
    let flags = word(payload, base + 8) & !3;
    payload[base + 8..base + 12].copy_from_slice(&(flags | (u32::from(value.numerator_raw.is_some()) << 1) | u32::from(value.numerator_raw.is_some_and(|raw| raw != 0))).to_le_bytes());
    payload[base + 24..base + 32].copy_from_slice(&value.numerator_raw.unwrap_or(0).to_le_bytes());
    for (at, value) in [(192, value.naval_transfer_membership), (196, value.movement_target_present)] {
        let raw = value.map_or(0, |value| 1 + u32::from(value));
        payload[base + at..base + at + 4].copy_from_slice(&raw.to_le_bytes());
    }
    payload[base + 200..base + 204].copy_from_slice(&value.retreat_raw.map_or(0, |value| u32::from(value) + 1).to_le_bytes());
    let (status, kind, id) = match value.first_combat {
        None => (0_u32, 0, 0),
        Some(None) => (1, 0, 0),
        Some(Some(identity)) => {
            assert!(identity.id > 0 && identity.id <= i32::MAX as u32);
            assert!(identity.class != CombatClass::Land || identity.kind == 62);
            (if identity.class == CombatClass::Land { 2 } else { 3 }, identity.kind, identity.id)
        }
    };
    for (at, value) in [(204, status), (208, kind), (212, id)] {
        payload[base + at..base + at + 4].copy_from_slice(&value.to_le_bytes());
    }
}

pub fn append_inputs<'a>(
    payload: &mut Vec<u8>,
    routes: impl ExactSizeIterator<Item = (Option<&'a [u32]>, Option<&'a [u32]>)>,
    preferences: impl ExactSizeIterator<Item = Option<&'a TemplateModelPreference>>,
    acclimatization: impl ExactSizeIterator<Item = Option<&'a Acclimatization>>,
    country_roster: &[u8],
    railway_roster: &[u8],
) {
    let rows = routes.len();
    let preferences = preferences.collect::<Vec<_>>();
    assert_eq!(rows, preferences.len());
    assert_eq!(rows, acclimatization.len());
    assert_eq!(payload.len(), 32 + rows * ROW_LEN);
    payload[28..32].copy_from_slice(&FORMAT_MARKER.to_le_bytes());
    let mut cursor = 0u32;
    for (index, (ordinary, alternate)) in routes.enumerate() {
        for (offset, route) in [(120, ordinary), (184, alternate)] {
            let Some(route) = route else { continue };
            assert!(route.len() <= MAX_ROUTE_PROVINCES);
            let descriptor = 32 + index * ROW_LEN + offset;
            payload[descriptor..descriptor + 4].copy_from_slice(&cursor.to_le_bytes());
            payload[descriptor + 4..descriptor + 8].copy_from_slice(&(route.len() as u32 + 1).to_le_bytes());
            for province in route { payload.extend_from_slice(&province.to_le_bytes()); }
            cursor = cursor.checked_add(route.len() as u32).expect("bounded division route pool");
        }
    }
    let mut cursor = 0u32;
    for (index, preference) in preferences.iter().copied().enumerate() {
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
    let table = payload.len();
    payload.resize(table + rows * 8, 0);
    let mut consumed = 0_u32;
    for (index, preference) in preferences.into_iter().enumerate() {
        let Some(excluded) = preference.and_then(|value| value.excluded_variants.as_ref()) else { continue };
        assert!(excluded.len() <= MAX_TEMPLATE_EXCLUSIONS);
        let at = table + index * 8;
        payload[at..at + 4].copy_from_slice(&consumed.to_le_bytes());
        payload[at + 4..at + 8].copy_from_slice(&(excluded.len() as u32 + 1).to_le_bytes());
        for id in excluded {
            assert!(id.is_none_or(|id| id > 0 && id <= i32::MAX as u32));
            payload.extend_from_slice(&id.unwrap_or(0).to_le_bytes());
        }
        consumed += excluded.len() as u32;
    }
    super::country_roster::Observation::parse(country_roster, super::country_roster::Kind::Division).expect("validated division country roster");
    payload.extend_from_slice(country_roster);
    super::country_roster::Observation::parse(railway_roster, super::country_roster::Kind::Railway).expect("validated railway country roster");
    payload.extend_from_slice(railway_roster);
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
    exclusions_table: usize,
    roster_start: usize,
    railway_start: usize,
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
            let flags = word(payload, base + 8);
            let numerator = i64::from_le_bytes(payload[base + 24..base + 32].try_into().unwrap());
            if flags & (1 << 1) == 0 && numerator != 0 {
                return Err("unavailable division movement numerator carries bytes".into());
            }
            if (flags & 1 != 0) != (flags & 2 != 0 && numerator != 0) {
                return Err("division moving flag disagrees with exact numerator".into());
            }
            if word(payload, base + 192) > 2 || word(payload, base + 196) > 2
                || word(payload, base + 200) > 256 {
                return Err("division movement scalar status invalid".into());
            }
            let status = word(payload, base + 204);
            let kind = word(payload, base + 208);
            let id = word(payload, base + 212);
            let count = word(payload, base + 48);
            if !match status {
                0 => kind == 0 && id == 0,
                1 => kind == 0 && id == 0 && count == 0,
                2 => kind == 62 && id > 0 && id <= i32::MAX as u32 && count > 0,
                3 => id > 0 && id <= i32::MAX as u32 && count > 0,
                _ => false,
            } {
                return Err("division first combat identity or count custody invalid".into());
            }
            if flags & (1 << 12) != 0 && (flags & (1 << 1) != 0
                || payload[base + 192..base + ROW_LEN].iter().any(|value| *value != 0)) {
                return Err("deleted division carries movement scalars".into());
            }
            for offset in [120, 184] {
                let start = word(payload, base + offset) as usize;
                let count = word(payload, base + offset + 4) as usize;
                if count == 0 {
                    if start != 0 { return Err("unavailable division route has pool offset".into()); }
                    continue;
                }
                if flags & (1 << 12) != 0 { return Err("deleted division carries a route".into()); }
                let len = count - 1;
                if len > MAX_ROUTE_PROVINCES || start != consumed {
                    return Err("division route exceeds bound or breaks ordered pool custody".into());
                }
                consumed = consumed.checked_add(len).ok_or("division route pool overflow")?;
                if consumed > (payload.len() - pool) / 4 { return Err("division route extends beyond pool".into()); }
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
        let exclusions_table = acclimatization_pool + acclimatization_consumed;
        let exclusions_pool = exclusions_table.checked_add(rows.checked_mul(8).ok_or("template exclusions table overflow")?).ok_or("template exclusions table overflow")?;
        if exclusions_pool > payload.len() {
            return Err("template exclusions table truncated".into());
        }
        let mut exclusions_consumed = 0_usize;
        for index in 0..rows {
            let at = exclusions_table + index * 8;
            let start = word(payload, at) as usize;
            let count = word(payload, at + 4) as usize;
            if count == 0 {
                if start != 0 { return Err("unavailable template exclusions have offset".into()); }
                continue;
            }
            if word(payload, 32 + index * ROW_LEN + 144) == 0
                || count - 1 > MAX_TEMPLATE_EXCLUSIONS || start != exclusions_consumed {
                return Err("template exclusions have invalid identity or pool custody".into());
            }
            exclusions_consumed = exclusions_consumed.checked_add(count - 1).ok_or("template exclusions count overflow")?;
        }
        let roster_start = exclusions_pool.checked_add(exclusions_consumed.checked_mul(4)
            .ok_or("division exclusion bytes overflow")?).ok_or("division roster offset overflow")?;
        let roster = payload.get(roster_start..).ok_or("division roster missing")?;
        let (_, division_len) = super::country_roster::Observation::parse_prefix(roster, super::country_roster::Kind::Division)?;
        let railway_start = roster_start.checked_add(division_len).ok_or("railway roster offset overflow")?;
        super::country_roster::Observation::parse(&payload[railway_start..], super::country_roster::Kind::Railway)?;
        for entry in payload[exclusions_pool..roster_start].chunks_exact(4) {
            if u32::from_le_bytes(entry.try_into().unwrap()) > i32::MAX as u32 {
                return Err("template exclusion variant identity is invalid".into());
            }
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
            exclusions_table,
            roster_start,
            railway_start,
        })
    }

    pub fn country_roster_bytes(&self) -> &[u8] { &self.payload[self.roster_start..self.railway_start] }

    pub fn railway_roster_bytes(&self) -> &[u8] { &self.payload[self.railway_start..] }

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
        let at = self.exclusions_table + index * 8;
        let start = word(self.payload, at) as usize;
        let count = word(self.payload, at + 4) as usize;
        let excluded_variants = (count != 0).then(|| {
            let pool = self.exclusions_table + self.rows * 8;
            (0..count - 1).map(|index| {
                let id = word(self.payload, pool + (start + index) * 4);
                (id != 0).then_some(id)
            }).collect()
        });
        Some(TemplateModelPreference {
            template_id,
            entity,
            category,
            excluded_variants,
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

    pub fn ordinary_route(&self, index: usize) -> Option<Vec<u32>> { self.route_at(index, 120) }
    pub fn alternate_route(&self, index: usize) -> Option<Vec<u32>> { self.route_at(index, 184) }

    pub fn movement_scalars(&self, index: usize) -> MovementScalars {
        assert!(index < self.rows);
        let base = 32 + index * ROW_LEN;
        let boolean = |at| match word(self.payload, base + at) { 0 => None, 1 => Some(false), 2 => Some(true), _ => unreachable!("validated boolean") };
        let first_combat = match word(self.payload, base + 204) {
            0 => None,
            1 => Some(None),
            status => Some(Some(CombatIdentity {
                class: if status == 2 { CombatClass::Land } else { CombatClass::BorderWar },
                kind: word(self.payload, base + 208), id: word(self.payload, base + 212),
            })),
        };
        MovementScalars {
            numerator_raw: (word(self.payload, base + 8) & (1 << 1) != 0)
                .then(|| i64::from_le_bytes(self.payload[base + 24..base + 32].try_into().unwrap())),
            naval_transfer_membership: boolean(192), movement_target_present: boolean(196),
            retreat_raw: (word(self.payload, base + 200) != 0).then(|| (word(self.payload, base + 200) - 1) as u8),
            first_combat,
        }
    }

    fn route_at(&self, index: usize, offset: usize) -> Option<Vec<u32>> {
        assert!(index < self.rows);
        let base = 32 + index * ROW_LEN;
        let count = word(self.payload, base + offset + 4) as usize;
        if count == 0 {
            return None;
        }
        let start = self.pool + word(self.payload, base + offset) as usize * 4;
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
            routes.iter().map(|route| (route.as_deref(), None)),
            std::iter::repeat_n(None, routes.len()),
            std::iter::repeat_n(None, routes.len()),
         &unavailable_roster(), &unavailable_railway_roster());
        bytes
    }

    #[test]
    fn legacy_row_followed_by_zero_words_cannot_alias_route_layout() {
        let mut legacy = vec![0; 32 + 120];
        legacy[8..12].copy_from_slice(&1u32.to_le_bytes());
        legacy[32..36].copy_from_slice(&7u32.to_le_bytes());
        legacy[36..40].copy_from_slice(&4713u32.to_le_bytes());
        legacy.extend_from_slice(&[0; ROW_LEN - 120 + 8]);
        assert_eq!(
            DivisionRoutes::parse(&legacy).err().unwrap(),
            "unsupported division route format marker"
        );
        legacy[28..32].copy_from_slice(&FORMAT_MARKER.to_le_bytes());
        assert!(DivisionRoutes::parse(&legacy).is_err(), "marker alone does not supply CRO1");
        legacy.extend_from_slice(&unavailable_roster());
        assert!(DivisionRoutes::parse(&legacy).is_err(), "division tail alone does not supply railway custody");
        legacy.extend_from_slice(&unavailable_railway_roster());
        assert_eq!(DivisionRoutes::parse(&legacy).unwrap().ordinary_route(0), None);
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
    fn template_exclusions_roundtrip_null_slots_empty_unknown_and_require_current_custody() {
        let preferences = [
            TemplateModelPreference { template_id: 73, entity: String::new(), category: Some(u64::MAX), excluded_variants: None },
            TemplateModelPreference { template_id: 73, entity: String::new(), category: Some(0), excluded_variants: Some(vec![]) },
            TemplateModelPreference { template_id: 73, entity: "model".into(), category: None, excluded_variants: Some(vec![Some(11), None, Some(11)]) },
        ];
        let mut payload = vec![0; 32 + 3 * ROW_LEN];
        payload[8..12].copy_from_slice(&3_u32.to_le_bytes());
        for index in 0..3 { write_current_template(&mut payload, index, Some(Some(73))); }
        append_inputs(&mut payload, [(None, None); 3].into_iter(), preferences.iter().map(Some), [None; 3].into_iter(), &unavailable_roster(), &unavailable_railway_roster());
        let parsed = DivisionRoutes::parse(&payload).unwrap();
        for (index, expected) in preferences.iter().enumerate() {
            assert_eq!(parsed.template_preference(index).as_ref(), Some(expected));
        }
        let table = parsed.exclusions_table;
        let mut malformed = payload.clone();
        malformed[table + 16..table + 20].copy_from_slice(&1_u32.to_le_bytes());
        assert!(DivisionRoutes::parse(&malformed).is_err());
        let mut legacy = payload.clone();
        legacy[28..32].copy_from_slice(b"D2R4");
        assert!(DivisionRoutes::parse(&legacy).is_err());
        payload[32 + 2 * ROW_LEN + 132..32 + 2 * ROW_LEN + 136].copy_from_slice(&74_u32.to_le_bytes());
        assert!(DivisionRoutes::parse(&payload).is_err());
    }

    #[test]
    fn template_preferences_share_pool_without_aliasing_routes_or_current_identity() {
        let preferences = [
            None,
            Some(TemplateModelPreference {
                template_id: 73,
                entity: String::new(),
                category: None,
                excluded_variants: None,
            }),
            Some(TemplateModelPreference {
                template_id: 74,
                entity: "armour_entity".into(),
                category: Some(0),
                excluded_variants: None,
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
            routes.iter().map(|r| (r.as_deref(), None)),
            preferences.iter().map(|p| p.as_ref()),
            std::iter::repeat_n(None, routes.len()),
         &unavailable_roster(), &unavailable_railway_roster());
        let parsed = DivisionRoutes::parse(&bytes).unwrap();
        for index in 0..3 {
            assert_eq!(parsed.template_preference(index), preferences[index]);
            assert_eq!(parsed.ordinary_route(index), routes[index]);
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
            std::iter::repeat_n((None, None), 3),
            std::iter::repeat_n(None, 3),
            values.iter().map(|v| v.as_ref()),
         &unavailable_roster(), &unavailable_railway_roster());
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
            assert_eq!(&parsed.ordinary_route(index), route);
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
    #[test]
    fn movement_inputs_round_trip_signed_width_both_routes_and_every_truth_state() {
        let ordinary = [None, Some(vec![]), Some(vec![19, 17, 19])];
        let alternate = [Some(vec![]), None, Some(vec![22, 21])];
        let scalars = [MovementScalars::default(), MovementScalars {
            numerator_raw: Some(0), naval_transfer_membership: Some(false), movement_target_present: Some(false),
            retreat_raw: Some(0), first_combat: Some(None),
        }, MovementScalars {
            numerator_raw: Some(i64::MIN + 1), naval_transfer_membership: Some(true), movement_target_present: Some(true),
            retreat_raw: Some(255), first_combat: Some(Some(CombatIdentity { class: CombatClass::Land, kind: 62, id: 91 })),
        }];
        let mut bytes = vec![0; 32 + 3 * ROW_LEN];
        bytes[8..12].copy_from_slice(&3_u32.to_le_bytes());
        bytes[32 + 2 * ROW_LEN + 48..32 + 2 * ROW_LEN + 52].copy_from_slice(&2_u32.to_le_bytes());
        for (index, value) in scalars.iter().enumerate() { write_movement_scalars(&mut bytes, index, *value); }
        append_inputs(&mut bytes, ordinary.iter().zip(&alternate).map(|(a,b)| (a.as_deref(), b.as_deref())),
            [None; 3].into_iter(), [None; 3].into_iter(), &unavailable_roster(), &unavailable_railway_roster());
        let parsed = DivisionRoutes::parse(&bytes).unwrap();
        for index in 0..3 {
            assert_eq!(parsed.ordinary_route(index), ordinary[index]);
            assert_eq!(parsed.alternate_route(index), alternate[index]);
            assert_eq!(parsed.movement_scalars(index), scalars[index]);
        }
        for raw in [i64::MIN, i64::MAX, -1, 0, 0x1_0000_0000] {
            write_movement_scalars(&mut bytes, 2, MovementScalars { numerator_raw: Some(raw), ..scalars[2] });
            assert_eq!(DivisionRoutes::parse(&bytes).unwrap().movement_scalars(2).numerator_raw, Some(raw));
        }
    }

    #[test]
    fn movement_inputs_refuse_old_marker_and_invalid_scalar_or_reference_custody() {
        let valid = payload(&[None]);
        for marker in [u32::from_le_bytes(*b"D2R5"), 0] {
            let mut bytes = valid.clone(); bytes[28..32].copy_from_slice(&marker.to_le_bytes());
            assert!(DivisionRoutes::parse(&bytes).err().unwrap().contains("unsupported division route format"));
        }
        for (offset, raw) in [(192, 3_u32), (196, 3), (200, 257), (204, 4), (208, 62), (212, 91), (24, 1)] {
            let mut bytes = valid.clone(); bytes[32 + offset..36 + offset].copy_from_slice(&raw.to_le_bytes());
            assert!(DivisionRoutes::parse(&bytes).is_err(), "offset {offset}");
        }
        for numerator in [None, Some(0), Some(-1), Some(1), Some(i64::MIN), Some(i64::MAX)] {
            let mut bytes = valid.clone();
            write_movement_scalars(&mut bytes, 0, MovementScalars { numerator_raw: numerator, ..Default::default() });
            assert_eq!(word(&bytes, 40) & 1 != 0, numerator.is_some_and(|raw| raw != 0));
            assert!(DivisionRoutes::parse(&bytes).is_ok());
            bytes[40] ^= 1;
            assert!(DivisionRoutes::parse(&bytes).err().unwrap().contains("moving flag disagrees"));
        }
        let mut bytes = valid.clone();
        write_movement_scalars(&mut bytes, 0, MovementScalars { first_combat: Some(Some(CombatIdentity {
            class: CombatClass::Land, kind: 62, id: 91,
        })), ..Default::default() });
        assert!(DivisionRoutes::parse(&bytes).is_err());
        bytes[32 + 48..32 + 52].copy_from_slice(&1_u32.to_le_bytes());
        assert!(DivisionRoutes::parse(&bytes).is_ok());
        bytes[32 + 8..32 + 12].copy_from_slice(&(1_u32 << 12).to_le_bytes());
        assert!(DivisionRoutes::parse(&bytes).is_err());
    }

}

pub fn unavailable_roster() -> Vec<u8> {
    super::country_roster::Observation::unavailable(
        super::country_roster::Kind::Division, super::country_roster::Cause::ExternalBridge,
    ).encode().expect("bounded unavailable roster")
}

pub fn unavailable_railway_roster() -> Vec<u8> {
    super::country_roster::Observation::unavailable(
        super::country_roster::Kind::Railway, super::country_roster::Cause::ExternalBridge,
    ).encode().expect("unavailable railway roster")
}

#[cfg(test)]
mod railway_tail_tests {
    use super::*;
    use super::super::country_roster::{Acquisition,Country,Kind,MemberRef,Observation,Slot};
    fn payload() -> Vec<u8> {
        let railway=Observation {kind:Kind::Railway,country_count:Some(1),failure:None,countries:vec![Country {
            physical_slot:0,owner_tag_db_id:Some(7),acquisition:Acquisition::Bracketed,
            observed_count:Some(1),failure:None,slots:vec![Slot {ordinal:0,reference:Ok(MemberRef {native_type:51,id:0})}],
        }]}.encode().unwrap();
        let mut bytes=vec![0;32];append_inputs(&mut bytes,std::iter::empty(),std::iter::empty(),std::iter::empty(),&unavailable_roster(),&railway);bytes
    }
    #[test]
    fn d2r8_owns_exactly_two_ordered_tails_and_rejects_old_missing_extra_or_malformed_bytes() {
        let bytes=payload();let parsed=DivisionRoutes::parse(&bytes).unwrap();
        assert_eq!(Observation::parse(parsed.railway_roster_bytes(),Kind::Railway).unwrap().countries[0].slots[0].reference.unwrap().id,0);
        for end in 32..bytes.len() { assert!(DivisionRoutes::parse(&bytes[..end]).is_err(),"{end}"); }
        let mut bad=bytes.clone();bad[28..32].copy_from_slice(b"D2R7");assert!(DivisionRoutes::parse(&bad).is_err());
        let mut bad=bytes.clone();bad.push(0);assert!(DivisionRoutes::parse(&bad).is_err());
        let mut swapped=bytes[..32].to_vec();swapped.extend_from_slice(parsed.railway_roster_bytes());swapped.extend_from_slice(parsed.country_roster_bytes());
        assert!(DivisionRoutes::parse(&swapped).is_err());
        let railway_start=bytes.len()-parsed.railway_roster_bytes().len();
        for offset in [railway_start+4,railway_start+6,railway_start+8,railway_start+28,railway_start+32+11] {
            let mut bad=bytes.clone();bad[offset]=255;assert!(DivisionRoutes::parse(&bad).is_err(),"{offset}");
        }
    }
}
