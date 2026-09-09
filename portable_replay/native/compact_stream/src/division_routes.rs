#[path = "division_routes/combat_membership.rs"]
mod combat_membership;
pub use combat_membership::{
    append_first_combat_countries, first_combat_countries_encoded_len,
    validate_first_combat_countries, CombatMembershipCause, CombatMembershipField,
    CombatMembershipRefusal, CombatMembershipStage, CountryWordsObservation,
    FirstCombatCountriesObservation, COMBAT_MEMBERSHIP_DESCRIPTOR_LEN, COMBAT_MEMBERSHIP_HEADER_LEN,
};

#[path = "division_routes/retained_visual_variant.rs"]
mod retained_visual_variant;
pub use retained_visual_variant::{
    write_retained_visual_variant, DivisionOwnerAuthority, RetainedVisualVariantObservation,
    RetainedVisualVariantRefusal,
};
pub use self::CombatMembershipField as RetainedVisualVariantField;

pub const FORMAT_MARKER: u32 = u32::from_le_bytes(*b"D2RB");
pub const ROW_LEN: usize = 328;
/// Underlying D20 training operands; no authored thresholds or selector results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum TrainingRefusal {
    NotObserved = 0,
    Unreadable = 1,
    InvalidIdentity = 2,
    Changed = 3,
    BoundExceeded = 4,
}
impl TrainingRefusal {
    pub fn from_raw(raw: u32) -> Result<Self, String> {
        match raw {
            0 => Ok(Self::NotObserved),
            1 => Ok(Self::Unreadable),
            2 => Ok(Self::InvalidIdentity),
            3 => Ok(Self::Changed),
            4 => Ok(Self::BoundExceeded),
            _ => Err("unknown training refusal".into()),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::NotObserved => "not_observed",
            Self::Unreadable => "unreadable",
            Self::InvalidIdentity => "invalid_identity",
            Self::Changed => "changed",
            Self::BoundExceeded => "bound_exceeded",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrainingObservation<T> {
    Measured(T),
    Unavailable(TrainingRefusal),
}
impl<T> Default for TrainingObservation<T> {
    fn default() -> Self {
        Self::Unavailable(TrainingRefusal::NotObserved)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainingCount {
    pub entry_count: i32,
    pub sum_raw: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainingArmy {
    pub kind: u32,
    pub id: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrainingScalars {
    pub experience: TrainingObservation<i64>,
    pub current_hp: TrainingObservation<i64>,
    pub maximum_hp: TrainingObservation<i64>,
    pub template_count: TrainingObservation<TrainingCount>,
    pub army: TrainingObservation<Option<TrainingArmy>>,
    pub country_operand: TrainingObservation<i32>,
}
impl TrainingScalars {
    pub fn json(&self) -> String {
        use TrainingObservation::*;
        fn unavailable(c: TrainingRefusal) -> String {
            format!("{{\"status\":\"unavailable\",\"reason\":\"{}\"}}", c.name())
        }
        fn scalar(v: TrainingObservation<i64>) -> String {
            match v {
                Measured(raw) => format!("{{\"status\":\"measured\",\"raw_i64\":{raw}}}"),
                Unavailable(c) => unavailable(c),
            }
        }
        let count = match self.template_count {
            Measured(v) => format!(
                "{{\"status\":\"measured\",\"entry_count\":{},\"sum_raw\":{}}}",
                v.entry_count, v.sum_raw
            ),
            Unavailable(c) => unavailable(c),
        };
        let army = match self.army {
            Measured(None) => "{\"status\":\"null\"}".into(),
            Measured(Some(v)) => format!(
                "{{\"status\":\"measured\",\"kind\":{},\"id\":{}}}",
                v.kind, v.id
            ),
            Unavailable(c) => unavailable(c),
        };
        let country = match self.country_operand {
            Measured(raw) => format!("{{\"status\":\"measured\",\"raw_i32\":{raw}}}"),
            Unavailable(c) => unavailable(c),
        };
        format!("{{\"schema\":1,\"experience\":{},\"current_hp\":{},\"maximum_hp\":{},\"template_count\":{},\"army\":{},\"country_operand\":{}}}",scalar(self.experience),scalar(self.current_hp),scalar(self.maximum_hp),count,army,country)
    }
    pub fn unavailable(cause: TrainingRefusal) -> Self {
        Self {
            experience: TrainingObservation::Unavailable(cause),
            current_hp: TrainingObservation::Unavailable(cause),
            maximum_hp: TrainingObservation::Unavailable(cause),
            template_count: TrainingObservation::Unavailable(cause),
            army: TrainingObservation::Unavailable(cause),
            country_operand: TrainingObservation::Unavailable(cause),
        }
    }
}
pub fn write_training_scalars(payload: &mut [u8], index: usize, v: TrainingScalars) {
    use TrainingObservation::*;
    let scalar = |v: TrainingObservation<i64>| match v {
        Measured(raw) => (1, 0, raw),
        Unavailable(c) => (0, c as u32, 0),
    };
    let count = match v.template_count {
        Measured(c) => {
            assert!(c.entry_count > 0 || c.sum_raw == 0);
            (1, c.entry_count as u32, i64::from(c.sum_raw))
        }
        Unavailable(c) => (0, c as u32, 0),
    };
    let army = match v.army {
        Measured(None) => (1, 0, 0),
        Measured(Some(a)) => {
            assert!(a.id != 0);
            (2, a.kind, i64::from(a.id))
        }
        Unavailable(c) => (0, c as u32, 0),
    };
    let country = match v.country_operand {
        Measured(raw) => (1, 0, i64::from(raw)),
        Unavailable(c) => (0, c as u32, 0),
    };
    for (n, (status, aux, raw)) in [
        scalar(v.experience),
        scalar(v.current_hp),
        scalar(v.maximum_hp),
        count,
        army,
        country,
    ]
    .into_iter()
    .enumerate()
    {
        let at = 32 + index * ROW_LEN + 216 + n * 16;
        payload[at..at + 4].copy_from_slice(&u32::to_le_bytes(status));
        payload[at + 4..at + 8].copy_from_slice(&aux.to_le_bytes());
        payload[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
    }
}
fn read_training_scalars(payload: &[u8], index: usize) -> Result<TrainingScalars, String> {
    use TrainingObservation::*;
    let cell = |n: usize| {
        let at = 32 + index * ROW_LEN + 216 + n * 16;
        (
            word(payload, at),
            word(payload, at + 4),
            i64::from_le_bytes(payload[at + 8..at + 16].try_into().unwrap()),
        )
    };
    fn absent<T>(aux: u32, raw: i64) -> Result<TrainingObservation<T>, String> {
        if raw != 0 {
            return Err("unavailable training operand has filler".into());
        }
        Ok(Unavailable(TrainingRefusal::from_raw(aux)?))
    }
    let scalar = |n| {
        let (status, aux, raw) = cell(n);
        match status {
            0 => absent(aux, raw),
            1 if aux == 0 => Ok(Measured(raw)),
            _ => Err("invalid training scalar".into()),
        }
    };
    let (status, aux, raw) = cell(3);
    let template_count = match status {
        0 => absent(aux, raw)?,
        1 if i32::try_from(raw).is_ok() && ((aux as i32) > 0 || raw == 0) => {
            Measured(TrainingCount {
                entry_count: aux as i32,
                sum_raw: raw as i32,
            })
        }
        _ => return Err("invalid training count".into()),
    };
    let (status, aux, raw) = cell(4);
    let army = match status {
        0 => absent(aux, raw)?,
        1 if aux == 0 && raw == 0 => Measured(None),
        2 if raw > 0 && u32::try_from(raw).is_ok() => Measured(Some(TrainingArmy {
            kind: aux,
            id: raw as u32,
        })),
        _ => return Err("invalid training army".into()),
    };
    let country_operand = match scalar(5)? {
        Measured(raw) => Measured(i32::try_from(raw).map_err(|_| "training country width")?),
        Unavailable(c) => Unavailable(c),
    };
    Ok(TrainingScalars {
        experience: scalar(0)?,
        current_hp: scalar(1)?,
        maximum_hp: scalar(2)?,
        template_count,
        army,
        country_operand,
    })
}

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

#[cfg(test)]
#[path = "division_storage_tests.rs"]
mod storage_tests;

/// Check structure and size the complete division payload prefix, including its header,
/// fixed rows, pools, and both country rosters. Combat membership is not included.
/// Duplicate roster identities and category keys are checked by `append_inputs`.
/// Iterators and their clones must yield the same items.
pub fn inputs_encoded_len<'a>(
    routes: impl ExactSizeIterator<Item = (Option<&'a [u32]>, Option<&'a [u32]>)> + Clone,
    preferences: impl ExactSizeIterator<Item = Option<&'a TemplateModelPreference>> + Clone,
    acclimatization: impl ExactSizeIterator<Item = Option<&'a Acclimatization>> + Clone,
    country_roster: &super::country_roster::Observation,
    railway_roster: &super::country_roster::Observation,
) -> Result<usize, String> {
    input_lengths(routes, preferences, acclimatization, country_roster, railway_roster).map(|lengths| lengths.total)
}

struct InputLengths {
    total: usize,
    country_roster: usize,
    railway_roster: usize,
}

fn input_lengths<'a>(
    routes: impl ExactSizeIterator<Item = (Option<&'a [u32]>, Option<&'a [u32]>)> + Clone,
    preferences: impl ExactSizeIterator<Item = Option<&'a TemplateModelPreference>> + Clone,
    acclimatization: impl ExactSizeIterator<Item = Option<&'a Acclimatization>> + Clone,
    country_roster: &super::country_roster::Observation,
    railway_roster: &super::country_roster::Observation,
) -> Result<InputLengths, String> {
    let rows = routes.len();
    u32::try_from(rows).map_err(|_| "division input row count overflow")?;
    if rows != preferences.len() || rows != acclimatization.len() {
        return Err("division input row cardinality".into());
    }
    if country_roster.kind != super::country_roster::Kind::Division
        || railway_roster.kind != super::country_roster::Kind::Railway {
        return Err("division input roster kind".into());
    }
    let mut route_words = 0_usize;
    let mut entity_bytes = 0_usize;
    let mut acclimatization_bytes = 0_usize;
    let mut exclusion_words = 0_usize;
    let mut seen = 0;
    for (ordinary, alternate) in routes {
        seen += 1;
        if seen > rows { return Err("division route row cardinality".into()); }
        for route in [ordinary, alternate].into_iter().flatten() {
            if route.len() > MAX_ROUTE_PROVINCES
                || route.iter().any(|id| *id == 0 || *id >= i32::MAX as u32) {
                return Err("division route length or province identity".into());
            }
            route_words = route_words.checked_add(route.len()).ok_or("division route pool overflow")?;
            u32::try_from(route_words).map_err(|_| "division route pool overflow")?;
        }
    }
    if seen != rows { return Err("division route row cardinality".into()); }
    let mut seen = 0;
    for preference in preferences {
        seen += 1;
        if seen > rows { return Err("division preference row cardinality".into()); }
        let Some(preference) = preference else { continue };
        if preference.template_id == 0 || preference.template_id > i32::MAX as u32
            || preference.entity.len() > MAX_MODEL_ENTITY_BYTES {
            return Err("division template preference identity or entity bound".into());
        }
        entity_bytes = entity_bytes.checked_add(preference.entity.len()).ok_or("template entity pool overflow")?;
        u32::try_from(entity_bytes).map_err(|_| "template entity pool overflow")?;
        if let Some(excluded) = &preference.excluded_variants {
            if excluded.len() > MAX_TEMPLATE_EXCLUSIONS
                || excluded.iter().flatten().any(|id| *id == 0 || *id > i32::MAX as u32) {
                return Err("template exclusion length or variant identity".into());
            }
            exclusion_words = exclusion_words.checked_add(excluded.len()).ok_or("template exclusions pool overflow")?;
            u32::try_from(exclusion_words).map_err(|_| "template exclusions pool overflow")?;
        }
    }
    if seen != rows { return Err("division preference row cardinality".into()); }
    let mut seen = 0;
    for value in acclimatization {
        seen += 1;
        if seen > rows { return Err("division acclimatization row cardinality".into()); }
        let Some(value) = value else { continue };
        if value.categories.len() > MAX_ACCLIMATIZATION_CATEGORIES {
            return Err("acclimatization category bound".into());
        }
        for entry in &value.categories {
            if entry.category.is_empty() || entry.category.len() > MAX_MODEL_ENTITY_BYTES {
                return Err("acclimatization category length".into());
            }
            acclimatization_bytes = acclimatization_bytes.checked_add(10 + entry.category.len())
                .ok_or("acclimatization pool overflow")?;
            u32::try_from(acclimatization_bytes).map_err(|_| "acclimatization pool overflow")?;
        }
    }
    if seen != rows { return Err("division acclimatization row cardinality".into()); }
    let roster_bytes = country_roster.encoded_len()?;
    let railway_bytes = railway_roster.encoded_len()?;
    let total = rows.checked_mul(ROW_LEN + 8).and_then(|n| n.checked_add(32))
        .and_then(|n| n.checked_add(route_words.checked_mul(4)?))
        .and_then(|n| n.checked_add(entity_bytes))
        .and_then(|n| n.checked_add(acclimatization_bytes))
        .and_then(|n| n.checked_add(exclusion_words.checked_mul(4)?))
        .and_then(|n| n.checked_add(roster_bytes))
        .and_then(|n| n.checked_add(railway_bytes))
        .ok_or("division input byte length overflow")?;
    Ok(InputLengths { total, country_roster: roster_bytes, railway_roster: railway_bytes })
}

/// Append into caller-funded capacity, with all input checks before mutation.
/// Iterators and their clones must yield the same items.
pub fn append_inputs<'a>(
    payload: &mut Vec<u8>,
    routes: impl ExactSizeIterator<Item = (Option<&'a [u32]>, Option<&'a [u32]>)> + Clone,
    preferences: impl ExactSizeIterator<Item = Option<&'a TemplateModelPreference>> + Clone,
    acclimatization: impl ExactSizeIterator<Item = Option<&'a Acclimatization>> + Clone,
    country_roster: &super::country_roster::Observation,
    railway_roster: &super::country_roster::Observation,
) -> Result<(), String> {
    let lengths = input_lengths(routes.clone(), preferences.clone(), acclimatization.clone(), country_roster, railway_roster)?;
    let rows = routes.len();
    let prefix_len = rows.checked_mul(ROW_LEN).and_then(|n| n.checked_add(32))
        .ok_or("division input row length overflow")?;
    if payload.len() != prefix_len || word(payload, 8) as usize != rows {
        return Err("division input payload row cardinality".into());
    }
    if payload.capacity() < lengths.total {
        return Err("division input output capacity unavailable".into());
    }
    for (index, (((ordinary, alternate), preference), value)) in routes.clone()
        .zip(preferences.clone()).zip(acclimatization.clone()).enumerate() {
        let base = 32 + index * ROW_LEN;
        if word(payload, base + 8) & (1 << 12) != 0
            && (ordinary.is_some() || alternate.is_some() || preference.is_some() || value.is_some()) {
            return Err("deleted division carries input pools".into());
        }
        if let Some(preference) = preference {
            if word(payload, base + 128) != 2 || word(payload, base + 132) != preference.template_id {
                return Err("template preference current-template custody mismatch".into());
            }
        }
        if let Some(value) = value {
            // Every category contributes at least eleven wire bytes, so its
            // eight-byte source index fits in the already-funded output tail.
            if !super::country_roster::unique_in_spare(payload, value.categories.len(),
                (0..value.categories.len()).map(|index| (index as u64).to_le_bytes()),
                |left, right| value.categories[u64::from_le_bytes(*left) as usize].category
                    .cmp(&value.categories[u64::from_le_bytes(*right) as usize].category))? {
                return Err("acclimatization repeats a category".into());
            }
        }
    }
    country_roster.validate_unique_in_spare(payload, lengths.country_roster)?;
    railway_roster.validate_unique_in_spare(payload, lengths.railway_roster)?;
    payload[28..32].copy_from_slice(&FORMAT_MARKER.to_le_bytes());
    for index in 0..rows {
        let base = 32 + index * ROW_LEN;
        payload[base + 120..base + 128].fill(0);
        payload[base + 144..base + 192].fill(0);
    }
    let mut cursor = 0u32;
    for (index, (ordinary, alternate)) in routes.enumerate() {
        for (offset, route) in [(120, ordinary), (184, alternate)] {
            let Some(route) = route else { continue };
            let descriptor = 32 + index * ROW_LEN + offset;
            payload[descriptor..descriptor + 4].copy_from_slice(&cursor.to_le_bytes());
            payload[descriptor + 4..descriptor + 8].copy_from_slice(&(route.len() as u32 + 1).to_le_bytes());
            for province in route { payload.extend_from_slice(&province.to_le_bytes()); }
            cursor += route.len() as u32;
        }
    }
    let mut cursor = 0u32;
    for (index, preference) in preferences.clone().enumerate() {
        let Some(preference) = preference else {
            continue;
        };
        let base = 32 + index * ROW_LEN;
        payload[base + 144..base + 148].copy_from_slice(&preference.template_id.to_le_bytes());
        payload[base + 148..base + 152]
            .copy_from_slice(&u32::from(preference.category.is_some()).to_le_bytes());
        payload[base + 152..base + 160]
            .copy_from_slice(&preference.category.unwrap_or(0).to_le_bytes());
        payload[base + 160..base + 164].copy_from_slice(&cursor.to_le_bytes());
        payload[base + 164..base + 168]
            .copy_from_slice(&(preference.entity.len() as u32 + 1).to_le_bytes());
        payload.extend_from_slice(preference.entity.as_bytes());
        cursor += preference.entity.len() as u32;
    }
    let mut cursor = 0u32;
    for (index, value) in acclimatization.enumerate() {
        let Some(value) = value else { continue };
        let base = 32 + index * ROW_LEN;
        payload[base + 168..base + 172].copy_from_slice(&cursor.to_le_bytes());
        payload[base + 172..base + 176]
            .copy_from_slice(&(value.categories.len() as u32 + 1).to_le_bytes());
        payload[base + 176..base + 184].copy_from_slice(&value.maximum_raw.to_le_bytes());
        for entry in &value.categories {
            payload.extend_from_slice(&(entry.category.len() as u16).to_le_bytes());
            payload.extend_from_slice(entry.category.as_bytes());
            payload.extend_from_slice(&entry.weight_raw.to_le_bytes());
            cursor += 10 + entry.category.len() as u32;
        }
    }
    let table = payload.len();
    payload.resize(table + rows * 8, 0);
    let mut consumed = 0_u32;
    for (index, preference) in preferences.enumerate() {
        let Some(excluded) = preference.and_then(|value| value.excluded_variants.as_ref()) else { continue };
        let at = table + index * 8;
        payload[at..at + 4].copy_from_slice(&consumed.to_le_bytes());
        payload[at + 4..at + 8].copy_from_slice(&(excluded.len() as u32 + 1).to_le_bytes());
        for id in excluded {
            payload.extend_from_slice(&id.unwrap_or(0).to_le_bytes());
        }
        consumed += excluded.len() as u32;
    }
    country_roster.append_validated(payload, lengths.country_roster);
    railway_roster.append_validated(payload, lengths.railway_roster);
    Ok(())
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
    membership_start: usize,
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
            retained_visual_variant::read(payload, index)?;
            let training = read_training_scalars(payload, index)?;
            if word(payload, base + 8) & (1 << 12) != 0 && training != TrainingScalars::default() {
                return Err("deleted division carries training operands".into());
            }
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
        let (_, railway_len) = super::country_roster::Observation::parse_prefix(&payload[railway_start..], super::country_roster::Kind::Railway)?;
        let membership_start = railway_start.checked_add(railway_len).ok_or("combat membership offset overflow")?;
        validate_first_combat_countries(&payload[membership_start..], rows, payload)?;
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
            membership_start,
        })
    }

    pub fn country_roster_bytes(&self) -> &[u8] { &self.payload[self.roster_start..self.railway_start] }

    pub fn railway_roster_bytes(&self) -> &[u8] { &self.payload[self.railway_start..self.membership_start] }

    pub fn first_combat_countries(&self, index: usize) -> Result<FirstCombatCountriesObservation, String> {
        combat_membership::read_first_combat_countries(&self.payload[self.membership_start..], self.rows, self.payload, index)
    }

    pub fn retained_visual_variant(&self, index: usize) -> (RetainedVisualVariantObservation, DivisionOwnerAuthority) {
        assert!(index < self.rows);
        retained_visual_variant::read(self.payload, index).expect("validated retained visual input")
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

    pub fn training_scalars(&self, index: usize) -> TrainingScalars {
        assert!(index < self.rows);
        read_training_scalars(self.payload, index).expect("validated training operands")
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
fn append_fixture_inputs<'a>(
    payload: &mut Vec<u8>,
    routes: impl ExactSizeIterator<Item = (Option<&'a [u32]>, Option<&'a [u32]>)> + Clone,
    preferences: impl ExactSizeIterator<Item = Option<&'a TemplateModelPreference>> + Clone,
    acclimatization: impl ExactSizeIterator<Item = Option<&'a Acclimatization>> + Clone,
    country_roster: &[u8],
    railway_roster: &[u8],
) {
    // These pre-existing fixtures never sampled countries. Preserve their
    // declared first identity, with an explicitly unobserved census.
    let observations: Vec<_> = (0..routes.len()).map(|index| {
        match combat_membership::row_projection(payload, index).unwrap() {
            None => FirstCombatCountriesObservation::default(),
            Some(None) => FirstCombatCountriesObservation::NoFirstCombat,
            Some(Some(combat)) => FirstCombatCountriesObservation::Present {
                combat, countries: CountryWordsObservation::Unavailable(Default::default()),
            },
        }
    }).collect();
    use super::country_roster::{Kind, Observation};
    let country_roster = Observation::parse(country_roster, Kind::Division).unwrap();
    let railway_roster = Observation::parse(railway_roster, Kind::Railway).unwrap();
    let prefix_len = inputs_encoded_len(routes.clone(), preferences.clone(), acclimatization.clone(), &country_roster, &railway_roster).unwrap();
    payload.try_reserve_exact(prefix_len - payload.len()).unwrap();
    append_inputs(payload, routes, preferences, acclimatization, &country_roster, &railway_roster).unwrap();
    payload.try_reserve_exact(first_combat_countries_encoded_len(observations.iter()).unwrap()).unwrap();
    append_first_combat_countries(payload, observations.iter()).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(routes: &[Option<Vec<u32>>]) -> Vec<u8> {
        let mut bytes = vec![0; 32 + routes.len() * ROW_LEN];
        bytes[8..12].copy_from_slice(&(routes.len() as u32).to_le_bytes());
        append_fixture_inputs(
            &mut bytes,
            routes.iter().map(|route| (route.as_deref(), None)),
            std::iter::repeat_n(None, routes.len()),
            std::iter::repeat_n(None, routes.len()),
         &unavailable_roster(), &unavailable_railway_roster());
        bytes
    }

    #[test]
    fn training_exact_widths_and_strict_states_survive_codec() {
        use TrainingObservation::*;
        for raw in [
            i64::MIN,
            i64::MAX,
            -1,
            0,
            1,
            -(1i64 << 32) - 1,
            (1i64 << 32) + 1,
            (1i64 << 53),
            (1i64 << 53) + 1,
        ] {
            let value = TrainingScalars {
                experience: Measured(raw),
                current_hp: Measured(raw),
                maximum_hp: Measured(raw),
                template_count: Measured(TrainingCount {
                    entry_count: 3,
                    sum_raw: i32::MIN,
                }),
                army: Measured(Some(TrainingArmy {
                    kind: u32::MAX,
                    id: u32::MAX,
                })),
                country_operand: Measured(i32::MIN),
            };
            let mut bytes = payload(&[None]);
            write_training_scalars(&mut bytes, 0, value);
            assert_eq!(
                DivisionRoutes::parse(&bytes).unwrap().training_scalars(0),
                value
            );
            bytes[28..32].copy_from_slice(b"D2R8");
            assert!(DivisionRoutes::parse(&bytes).is_err());
        }
        for army in [Measured(None), Unavailable(TrainingRefusal::Unreadable)] {
            let mut v = TrainingScalars::default();
            v.army = army;
            let mut bytes = payload(&[None]);
            write_training_scalars(&mut bytes, 0, v);
            assert_eq!(
                DivisionRoutes::parse(&bytes).unwrap().training_scalars(0),
                v
            );
        }
    }
    #[test]
    fn training_rejects_status_filler_width_and_deleted_measurements() {
        let clean = payload(&[None]);
        for (at, raw) in [
            (216, 9u32),
            (220, 9),
            (224, 1),
            (216 + 3 * 16, 2),
            (216 + 4 * 16, 3),
        ] {
            let mut bytes = clean.clone();
            bytes[32 + at..32 + at + 4].copy_from_slice(&raw.to_le_bytes());
            assert!(DivisionRoutes::parse(&bytes).is_err(), "{at}");
        }
        let mut value = TrainingScalars::default();
        value.experience = TrainingObservation::Measured(0);
        let mut bytes = clean;
        write_training_scalars(&mut bytes, 0, value);
        bytes[40..44].copy_from_slice(&(1u32 << 12).to_le_bytes());
        assert!(DivisionRoutes::parse(&bytes).is_err());
        let mut bytes = payload(&[None]);
        bytes[32 + 216 + 3 * 16..32 + 220 + 3 * 16].copy_from_slice(&1u32.to_le_bytes());
        bytes[32 + 224 + 3 * 16..32 + 232 + 3 * 16].copy_from_slice(&1i64.to_le_bytes());
        assert!(
            DivisionRoutes::parse(&bytes).is_err(),
            "nonpositive count cannot manufacture nonzero sum"
        );
    }

    fn set_training_cell(bytes: &mut [u8], cell: usize, status: u32, aux: u32, raw: i64) {
        let at = 32 + 216 + cell * 16;
        bytes[at..at + 4].copy_from_slice(&status.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&aux.to_le_bytes());
        bytes[at + 8..at + 16].copy_from_slice(&raw.to_le_bytes());
    }

    #[test]
    fn training_all_cells_reject_unknown_tags_refusals_and_unavailable_filler() {
        for cell in 0..6 {
            for (status, aux, raw) in [
                (3, 0, 0),
                (u32::MAX, 0, 0),
                (0, 5, 0),
                (0, u32::MAX, 0),
                (0, 0, 1),
                (0, 0, -1),
                (0, 4, 1i64 << 32),
            ] {
                let mut bytes = payload(&[None]);
                set_training_cell(&mut bytes, cell, status, aux, raw);
                assert!(
                    DivisionRoutes::parse(&bytes).is_err(),
                    "cell {cell}: {status}/{aux}/{raw}"
                );
            }
            if cell != 4 {
                let mut bytes = payload(&[None]);
                set_training_cell(&mut bytes, cell, 2, 0, 1);
                assert!(
                    DivisionRoutes::parse(&bytes).is_err(),
                    "identity tag on cell {cell}"
                );
            }
        }
        for cause in [
            TrainingRefusal::NotObserved,
            TrainingRefusal::Unreadable,
            TrainingRefusal::InvalidIdentity,
            TrainingRefusal::Changed,
            TrainingRefusal::BoundExceeded,
        ] {
            let value = TrainingScalars::unavailable(cause);
            let mut bytes = payload(&[None]);
            write_training_scalars(&mut bytes, 0, value);
            assert_eq!(
                DivisionRoutes::parse(&bytes).unwrap().training_scalars(0),
                value
            );
        }
    }

    #[test]
    fn training_measured_cells_enforce_auxiliary_and_integer_width_laws() {
        for cell in [0, 1, 2, 5] {
            let mut bytes = payload(&[None]);
            set_training_cell(&mut bytes, cell, 1, 1, 0);
            assert!(
                DivisionRoutes::parse(&bytes).is_err(),
                "scalar auxiliary cell {cell}"
            );
        }
        for cell in [3, 5] {
            for raw in [
                i64::MIN,
                i64::MAX,
                i64::from(i32::MIN) - 1,
                i64::from(i32::MAX) + 1,
            ] {
                let mut bytes = payload(&[None]);
                set_training_cell(&mut bytes, cell, 1, u32::from(cell == 3), raw);
                assert!(
                    DivisionRoutes::parse(&bytes).is_err(),
                    "i32 width cell {cell}: {raw}"
                );
            }
        }
        for count in [i32::MIN, -1, 0] {
            for raw in [i32::MIN, -1, 1, i32::MAX] {
                let mut bytes = payload(&[None]);
                set_training_cell(&mut bytes, 3, 1, count as u32, i64::from(raw));
                assert!(
                    DivisionRoutes::parse(&bytes).is_err(),
                    "nonpositive count {count}: {raw}"
                );
            }
        }
        for (status, aux, raw) in [
            (1, 1, 0),
            (1, 0, 1),
            (1, 0, -1),
            (2, 0, 0),
            (2, 0, -1),
            (2, 0, i64::from(u32::MAX) + 1),
        ] {
            let mut bytes = payload(&[None]);
            set_training_cell(&mut bytes, 4, status, aux, raw);
            assert!(
                DivisionRoutes::parse(&bytes).is_err(),
                "army {status}/{aux}/{raw}"
            );
        }
    }

    #[test]
    fn training_signed_count_country_and_null_army_remain_distinct_from_unavailable() {
        use TrainingObservation::Measured;
        for (entry_count, sum_raw) in [
            (i32::MIN, 0),
            (-1, 0),
            (0, 0),
            (1, i32::MIN),
            (i32::MAX, i32::MAX),
            (3, 0),
        ] {
            for country in [i32::MIN, -1, 0, i32::MAX] {
                let value = TrainingScalars {
                    experience: Measured(0),
                    current_hp: Measured(0),
                    maximum_hp: Measured(0),
                    template_count: Measured(TrainingCount {
                        entry_count,
                        sum_raw,
                    }),
                    army: Measured(None),
                    country_operand: Measured(country),
                };
                let mut bytes = payload(&[None]);
                write_training_scalars(&mut bytes, 0, value);
                assert_eq!(
                    DivisionRoutes::parse(&bytes).unwrap().training_scalars(0),
                    value
                );
                assert_ne!(value, TrainingScalars::default());
                assert!(value.json().contains("\"army\":{\"status\":\"null\"}"));
            }
        }
    }

    #[test]
    fn training_deleted_rows_require_every_cell_to_be_canonical_unobserved() {
        let mut clean = payload(&[None]);
        clean[40..44].copy_from_slice(&(1u32 << 12).to_le_bytes());
        assert_eq!(
            DivisionRoutes::parse(&clean).unwrap().training_scalars(0),
            TrainingScalars::default()
        );
        for at in 32 + 216..32 + ROW_LEN {
            let mut invalid = clean.clone();
            invalid[at] = 1;
            assert!(
                DivisionRoutes::parse(&invalid).is_err(),
                "deleted training byte {at}"
            );
        }
        for end in 32 + 216..32 + ROW_LEN {
            assert!(
                DivisionRoutes::parse(&clean[..end]).is_err(),
                "truncated training byte {end}"
            );
        }
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
        assert!(DivisionRoutes::parse(&legacy).is_err(), "both rosters require the membership trailer");
        let observations = [FirstCombatCountriesObservation::default()];
        legacy.try_reserve_exact(first_combat_countries_encoded_len(observations.iter()).unwrap()).unwrap();
        append_first_combat_countries(&mut legacy, observations.iter()).unwrap();
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
        append_fixture_inputs(&mut payload, [(None, None); 3].into_iter(), preferences.iter().map(Some), [None; 3].into_iter(), &unavailable_roster(), &unavailable_railway_roster());
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
        append_fixture_inputs(
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
        append_fixture_inputs(
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
        append_fixture_inputs(&mut bytes, ordinary.iter().zip(&alternate).map(|(a,b)| (a.as_deref(), b.as_deref())),
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
        assert!(DivisionRoutes::parse(&bytes).is_err(), "membership projection must also change");
        let membership = DivisionRoutes::parse(&valid).unwrap().membership_start + 8;
        bytes[membership..membership + 4].copy_from_slice(&3_u32.to_le_bytes());
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
        let mut bytes=vec![0;32];append_fixture_inputs(&mut bytes,std::iter::empty(),std::iter::empty(),std::iter::empty(),&unavailable_roster(),&railway);bytes
    }
    #[test]
    fn d2ra_owns_two_ordered_rosters_then_membership_and_rejects_malformed_bytes() {
        let bytes=payload();let parsed=DivisionRoutes::parse(&bytes).unwrap();
        assert_eq!(Observation::parse(parsed.railway_roster_bytes(),Kind::Railway).unwrap().countries[0].slots[0].reference.unwrap().id,0);
        for end in 32..bytes.len() { assert!(DivisionRoutes::parse(&bytes[..end]).is_err(),"{end}"); }
        let mut bad=bytes.clone();bad[28..32].copy_from_slice(b"D2R7");assert!(DivisionRoutes::parse(&bad).is_err());
        let mut bad=bytes.clone();bad.push(0);assert!(DivisionRoutes::parse(&bad).is_err());
        let mut swapped=bytes[..32].to_vec();swapped.extend_from_slice(parsed.railway_roster_bytes());swapped.extend_from_slice(parsed.country_roster_bytes());
        assert!(DivisionRoutes::parse(&swapped).is_err());
        let railway_start=parsed.railway_start;
        for offset in [railway_start+4,railway_start+6,railway_start+8,railway_start+28,railway_start+32+11] {
            let mut bad=bytes.clone();bad[offset]=255;assert!(DivisionRoutes::parse(&bad).is_err(),"{offset}");
        }
    }
}
