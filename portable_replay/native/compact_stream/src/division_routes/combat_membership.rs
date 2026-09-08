//! D2RA complete first-combat country observations. The fixed row owns the
//! combat identity; the trailer owns only its observation and ordered words.
use super::{word, CombatClass, CombatIdentity, FORMAT_MARKER, ROW_LEN};

pub const COMBAT_MEMBERSHIP_HEADER_LEN: usize = 8;
pub const COMBAT_MEMBERSHIP_DESCRIPTOR_LEN: usize = 40;

macro_rules! coded_enum {
    ($name:ident { $($variant:ident = $code:literal => $text:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn from_raw(raw: u32) -> Result<Self, String> {
                match raw { $($code => Ok(Self::$variant),)+
                    _ => Err(concat!("unknown ", stringify!($name)).into()) }
            }
            pub fn code(self) -> u32 { match self { $(Self::$variant => $code),+ } }
            pub fn name(self) -> &'static str { match self { $(Self::$variant => $text),+ } }
        }
    }
}

coded_enum!(CombatMembershipCause {
    NotObserved = 0 => "not_observed",
    Unreadable = 1 => "unreadable",
    Changed = 2 => "changed",
    InvalidIdentity = 3 => "invalid_identity",
    NullSide = 4 => "null_side",
    Cycle = 5 => "cycle",
    VerificationBudgetExceeded = 6 => "verification_budget_exceeded",
    RetainedBudgetExceeded = 7 => "retained_budget_exceeded",
    AllocationFailed = 8 => "allocation_failed",
    AddressOverflow = 9 => "address_overflow",
    InvalidAddress = 10 => "invalid_address",
    RootChanged = 11 => "root_changed",
    HourChanged = 12 => "hour_changed",
    HourUnreadable = 13 => "hour_unreadable",
});
coded_enum!(CombatMembershipField {
    FrameRoot = 0 => "frame_root",
    Hour = 1 => "hour",
    DivisionIdentity = 2 => "division_identity",
    FirstCombatVector = 3 => "first_combat_vector",
    FirstCombatPointer = 4 => "first_combat_pointer",
    CombatIdentity = 5 => "combat_identity",
    SidePointer = 6 => "side_pointer",
    SideClass = 7 => "side_class",
    SideBacklink = 8 => "side_backlink",
    Head = 9 => "head",
    Country = 10 => "country",
    Next = 11 => "next",
    OrdinaryRoute = 12 => "ordinary_route",
    AlternateRoute = 13 => "alternate_route",
    RouteHead = 14 => "route_head",
    Numerator = 15 => "numerator",
    Transfer = 16 => "transfer",
    Target = 17 => "target",
    Retreat = 18 => "retreat",
    DivisionCountryOperand = 19 => "division_country_operand",
    AttachmentIdentity = 20 => "attachment_identity",
    CampaignIdentity = 21 => "campaign_identity",
    ProducerBinding = 22 => "producer_binding",
    Strength = 23 => "strength",
    Organisation = 24 => "organisation",
});
coded_enum!(CombatMembershipStage {
    Admission = 0 => "admission",
    Open = 1 => "open",
    Close = 2 => "close",
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CombatMembershipRefusal {
    pub cause: CombatMembershipCause,
    pub field: CombatMembershipField,
    pub stage: CombatMembershipStage,
    pub ordinal: Option<u32>,
    pub original_ordinal: Option<u32>,
}

impl Default for CombatMembershipRefusal {
    fn default() -> Self {
        Self {
            cause: CombatMembershipCause::NotObserved,
            field: CombatMembershipField::FrameRoot,
            stage: CombatMembershipStage::Admission,
            ordinal: None,
            original_ordinal: None,
        }
    }
}

impl CombatMembershipRefusal {
    pub fn validate(&self) -> Result<(), String> {
        if self.cause == CombatMembershipCause::Cycle
            && !matches!((self.original_ordinal, self.ordinal), (Some(original), Some(repeat)) if original < repeat)
        {
            return Err("combat membership cycle lacks ordered original/repeat ordinals".into());
        }
        Ok(())
    }

    pub fn json(&self) -> String {
        fn ordinal(value: Option<u32>) -> String {
            value.map_or_else(|| "null".into(), |value| value.to_string())
        }
        format!(
            "{{\"cause\":\"{}\",\"field\":\"{}\",\"stage\":\"{}\",\"ordinal\":{},\"original_ordinal\":{}}}",
            self.cause.name(), self.field.name(), self.stage.name(),
            ordinal(self.ordinal), ordinal(self.original_ordinal),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountryWordsObservation {
    Measured(Vec<i32>),
    Unavailable(CombatMembershipRefusal),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirstCombatCountriesObservation {
    NoFirstCombat,
    Present {
        combat: CombatIdentity,
        countries: CountryWordsObservation,
    },
    Unavailable(CombatMembershipRefusal),
}

impl Default for FirstCombatCountriesObservation {
    fn default() -> Self {
        Self::Unavailable(CombatMembershipRefusal::default())
    }
}

impl FirstCombatCountriesObservation {
    pub fn projection(&self) -> Option<Option<CombatIdentity>> {
        match self {
            Self::NoFirstCombat => Some(None),
            Self::Present { combat, .. } => Some(Some(*combat)),
            Self::Unavailable(_) => None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::NoFirstCombat => Ok(()),
            Self::Unavailable(refusal) => refusal.validate(),
            Self::Present { combat, countries } => {
                validate_identity(*combat)?;
                match countries {
                    CountryWordsObservation::Measured(words) => {
                        u32::try_from(words.len()).map_err(|_| "combat country count overflow")?;
                        Ok(())
                    }
                    CountryWordsObservation::Unavailable(refusal) => refusal.validate(),
                }
            }
        }
    }

    pub fn json(&self) -> String {
        match self {
            Self::NoFirstCombat => "{\"schema\":1,\"status\":\"no_first_combat\"}".into(),
            Self::Unavailable(refusal) => format!(
                "{{\"schema\":1,\"status\":\"unavailable\",\"refusal\":{}}}",
                refusal.json()
            ),
            Self::Present { combat, countries } => {
                let countries = match countries {
                    CountryWordsObservation::Measured(words) => {
                        use std::fmt::Write;
                        let mut result = String::from("{\"status\":\"measured\",\"raw_i32\":[");
                        for (index, raw) in words.iter().enumerate() {
                            if index != 0 {
                                result.push(',');
                            }
                            write!(&mut result, "{raw}").expect("writing to String");
                        }
                        result.push_str("]}");
                        result
                    }
                    CountryWordsObservation::Unavailable(refusal) => format!(
                        "{{\"status\":\"unavailable\",\"refusal\":{}}}",
                        refusal.json()
                    ),
                };
                let class = match combat.class {
                    CombatClass::Land => "land",
                    CombatClass::BorderWar => "border_war",
                };
                format!("{{\"schema\":1,\"status\":\"present\",\"combat\":{{\"class\":\"{class}\",\"type\":{},\"id\":{}}},\"countries\":{countries}}}", combat.kind, combat.id)
            }
        }
    }
}

fn validate_identity(combat: CombatIdentity) -> Result<(), String> {
    if combat.id == 0
        || combat.id > i32::MAX as u32
        || (combat.class == CombatClass::Land && combat.kind != 62)
    {
        return Err("invalid combat membership identity".into());
    }
    Ok(())
}

fn words(observation: &FirstCombatCountriesObservation) -> &[i32] {
    match observation {
        FirstCombatCountriesObservation::Present {
            countries: CountryWordsObservation::Measured(words),
            ..
        } => words,
        _ => &[],
    }
}

fn encoded_len(rows: usize, words: usize) -> Result<usize, String> {
    u32::try_from(rows).map_err(|_| "combat membership row count overflow")?;
    u32::try_from(words).map_err(|_| "combat membership word count overflow")?;
    rows.checked_mul(COMBAT_MEMBERSHIP_DESCRIPTOR_LEN)
        .and_then(|size| size.checked_add(COMBAT_MEMBERSHIP_HEADER_LEN))
        .and_then(|size| words.checked_mul(4).and_then(|pool| size.checked_add(pool)))
        .ok_or_else(|| "combat membership byte length overflow".into())
}

/// Exact additional bytes. The caller funds and fallibly reserves this storage
/// before append; this routine does not allocate a census copy.
/// Iterators and their clones must yield the same items.
pub fn first_combat_countries_encoded_len<'a>(
    observations: impl ExactSizeIterator<Item = &'a FirstCombatCountriesObservation> + Clone,
) -> Result<usize, String> {
    let rows = observations.len();
    u32::try_from(rows).map_err(|_| "combat membership row count overflow")?;
    let mut count = 0usize;
    let mut seen = 0usize;
    for observation in observations {
        seen += 1;
        if seen > rows {
            return Err("combat membership row cardinality mismatch".into());
        }
        observation.validate()?;
        count = count
            .checked_add(words(observation).len())
            .ok_or("combat country count overflow")?;
    }
    if seen != rows {
        return Err("combat membership row cardinality mismatch".into());
    }
    encoded_len(rows, count)
}

pub(super) fn row_projection(
    payload: &[u8],
    index: usize,
) -> Result<Option<Option<CombatIdentity>>, String> {
    let base = index
        .checked_mul(ROW_LEN)
        .and_then(|offset| offset.checked_add(32))
        .ok_or("combat membership row offset overflow")?;
    let end = base
        .checked_add(ROW_LEN)
        .ok_or("combat membership row end overflow")?;
    let row = payload
        .get(base..end)
        .ok_or("combat membership fixed row truncated")?;
    let kind = word(row, 208);
    let id = word(row, 212);
    match word(row, 204) {
        0 if kind == 0 && id == 0 => Ok(None),
        1 if kind == 0 && id == 0 && word(row, 48) == 0 => Ok(Some(None)),
        status @ (2 | 3) if word(row, 48) != 0 => {
            let combat = CombatIdentity {
                class: if status == 2 {
                    CombatClass::Land
                } else {
                    CombatClass::BorderWar
                },
                kind,
                id,
            };
            validate_identity(combat)?;
            Ok(Some(Some(combat)))
        }
        _ => Err("combat membership fixed identity or count invalid".into()),
    }
}

fn refusal_cells(refusal: CombatMembershipRefusal) -> [u32; 7] {
    [
        refusal.cause.code(),
        refusal.field.code(),
        refusal.stage.code(),
        u32::from(refusal.ordinal.is_some()),
        refusal.ordinal.unwrap_or(0),
        u32::from(refusal.original_ordinal.is_some()),
        refusal.original_ordinal.unwrap_or(0),
    ]
}

/// Append into caller-funded capacity, without allocating output storage. Invalid observations,
/// projection disagreements, or insufficient capacity leave payload unchanged.
/// Call once, immediately after `append_inputs` has emitted both exact rosters.
/// Iterators and their clones must yield the same items.
pub fn append_first_combat_countries<'a>(
    payload: &mut Vec<u8>,
    observations: impl ExactSizeIterator<Item = &'a FirstCombatCountriesObservation> + Clone,
) -> Result<(), String> {
    let additional = first_combat_countries_encoded_len(observations.clone())?;
    if payload.len() < 32
        || word(payload, 28) != FORMAT_MARKER
        || word(payload, 8) as usize != observations.len()
    {
        return Err("combat membership header or row count mismatch".into());
    }
    for (index, observation) in observations.clone().enumerate() {
        if row_projection(payload, index)? != observation.projection() {
            return Err("combat membership disagrees with first combat projection".into());
        }
        if word(payload, 32 + index * ROW_LEN + 8) & (1 << 12) != 0
            && *observation != FirstCombatCountriesObservation::default()
        {
            return Err("deleted division carries combat membership".into());
        }
    }
    if payload.capacity() - payload.len() < additional {
        return Err("combat membership append requires caller-funded capacity".into());
    }
    let rows = observations.len();
    let descriptor_bytes = rows * COMBAT_MEMBERSHIP_DESCRIPTOR_LEN;
    let count = (additional - COMBAT_MEMBERSHIP_HEADER_LEN - descriptor_bytes) / 4;
    payload.extend_from_slice(&(rows as u32).to_le_bytes());
    payload.extend_from_slice(&(count as u32).to_le_bytes());
    let mut cursor = 0u32;
    for observation in observations.clone() {
        use FirstCombatCountriesObservation::*;
        let (state, refusal) = match observation {
            Unavailable(refusal) => (0u32, Some(*refusal)),
            NoFirstCombat => (1, None),
            Present {
                countries: CountryWordsObservation::Measured(_),
                ..
            } => (2, None),
            Present {
                countries: CountryWordsObservation::Unavailable(refusal),
                ..
            } => (3, Some(*refusal)),
        };
        payload.extend_from_slice(&state.to_le_bytes());
        for cell in refusal.map_or([0; 7], refusal_cells) {
            payload.extend_from_slice(&cell.to_le_bytes());
        }
        let count = words(observation).len() as u32;
        let start = if state == 2 { cursor } else { 0 };
        payload.extend_from_slice(&start.to_le_bytes());
        payload.extend_from_slice(&count.to_le_bytes());
        cursor += count;
    }
    for observation in observations {
        for raw in words(observation) {
            payload.extend_from_slice(&raw.to_le_bytes());
        }
    }
    Ok(())
}

fn read_refusal(descriptor: &[u8]) -> Result<CombatMembershipRefusal, String> {
    fn ordinal(present: u32, value: u32) -> Result<Option<u32>, String> {
        match (present, value) {
            (0, 0) => Ok(None),
            (1, value) => Ok(Some(value)),
            _ => Err("invalid combat membership ordinal presence or filler".into()),
        }
    }
    let refusal = CombatMembershipRefusal {
        cause: CombatMembershipCause::from_raw(word(descriptor, 4))?,
        field: CombatMembershipField::from_raw(word(descriptor, 8))?,
        stage: CombatMembershipStage::from_raw(word(descriptor, 12))?,
        ordinal: ordinal(word(descriptor, 16), word(descriptor, 20))?,
        original_ordinal: ordinal(word(descriptor, 24), word(descriptor, 28))?,
    };
    refusal.validate()?;
    Ok(refusal)
}

/// Validate every descriptor, exact pool ownership, and the complete fixed-row
/// identity projection before any consumer may trust an individual observation.
pub fn validate_first_combat_countries(
    trailer: &[u8],
    rows: usize,
    complete_payload: &[u8],
) -> Result<(), String> {
    if trailer.len() < COMBAT_MEMBERSHIP_HEADER_LEN || word(trailer, 0) as usize != rows {
        return Err("combat membership trailer missing or row count mismatch".into());
    }
    let total_words = word(trailer, 4) as usize;
    if encoded_len(rows, total_words)? != trailer.len() {
        return Err("combat membership trailer length mismatch".into());
    }
    let mut consumed = 0usize;
    for index in 0..rows {
        let at = COMBAT_MEMBERSHIP_HEADER_LEN + index * COMBAT_MEMBERSHIP_DESCRIPTOR_LEN;
        let descriptor = &trailer[at..at + COMBAT_MEMBERSHIP_DESCRIPTOR_LEN];
        let state = word(descriptor, 0);
        let start = word(descriptor, 32) as usize;
        let count = word(descriptor, 36) as usize;
        let projection = row_projection(complete_payload, index)?;
        match state {
            0 | 3 => {
                read_refusal(descriptor)?;
                if start != 0 || count != 0 {
                    return Err("unavailable combat membership carries country words".into());
                }
                if (state == 0 && projection.is_some())
                    || (state == 3 && !matches!(projection, Some(Some(_))))
                {
                    return Err("combat membership disagrees with first combat projection".into());
                }
            }
            1 | 2 => {
                if descriptor[4..32].iter().any(|byte| *byte != 0) {
                    return Err("measured combat membership carries refusal cells".into());
                }
                if state == 1 {
                    if start != 0 || count != 0 || projection != Some(None) {
                        return Err("no-first combat membership projection or pool invalid".into());
                    }
                } else {
                    if !matches!(projection, Some(Some(_))) || start != consumed {
                        return Err(
                            "combat membership identity or ordered pool custody invalid".into()
                        );
                    }
                    consumed = consumed
                        .checked_add(count)
                        .ok_or("combat country count overflow")?;
                    if consumed > total_words {
                        return Err("combat country pool truncated".into());
                    }
                }
            }
            _ => return Err("unknown combat membership state".into()),
        }
        if word(complete_payload, 32 + index * ROW_LEN + 8) & (1 << 12) != 0
            && descriptor.iter().any(|byte| *byte != 0)
        {
            return Err("deleted division carries combat membership".into());
        }
    }
    if consumed != total_words {
        return Err("combat country pool has unused words".into());
    }
    Ok(())
}

pub(super) fn read_first_combat_countries(
    trailer: &[u8],
    rows: usize,
    payload: &[u8],
    index: usize,
) -> Result<FirstCombatCountriesObservation, String> {
    if index >= rows {
        return Err("combat membership row index out of bounds".into());
    }
    let at = COMBAT_MEMBERSHIP_HEADER_LEN + index * COMBAT_MEMBERSHIP_DESCRIPTOR_LEN;
    let descriptor = &trailer[at..at + COMBAT_MEMBERSHIP_DESCRIPTOR_LEN];
    let state = word(descriptor, 0);
    use FirstCombatCountriesObservation::*;
    if state == 0 {
        return Ok(Unavailable(read_refusal(descriptor)?));
    }
    if state == 1 {
        return Ok(NoFirstCombat);
    }
    let Some(Some(combat)) = row_projection(payload, index)? else {
        return Err("combat membership identity missing".into());
    };
    let countries = if state == 3 {
        CountryWordsObservation::Unavailable(read_refusal(descriptor)?)
    } else {
        let pool = COMBAT_MEMBERSHIP_HEADER_LEN + rows * COMBAT_MEMBERSHIP_DESCRIPTOR_LEN;
        let start = pool + word(descriptor, 32) as usize * 4;
        let count = word(descriptor, 36) as usize;
        let mut words = Vec::new();
        words
            .try_reserve_exact(count)
            .map_err(|_| "combat country allocation failed")?;
        for bytes in trailer[start..start + count * 4].chunks_exact(4) {
            words.push(i32::from_le_bytes(bytes.try_into().unwrap()));
        }
        CountryWordsObservation::Measured(words)
    };
    Ok(Present { combat, countries })
}

#[cfg(test)]
mod tests {
    use super::super::{
        append_fixture_inputs, unavailable_railway_roster, unavailable_roster,
        write_movement_scalars, DivisionRoutes, MovementScalars,
    };
    use super::*;

    fn combat() -> CombatIdentity {
        CombatIdentity {
            class: CombatClass::Land,
            kind: 62,
            id: 79,
        }
    }

    fn measured(raw: Vec<i32>) -> FirstCombatCountriesObservation {
        FirstCombatCountriesObservation::Present {
            combat: combat(),
            countries: CountryWordsObservation::Measured(raw),
        }
    }

    fn cycle() -> CombatMembershipRefusal {
        CombatMembershipRefusal {
            cause: CombatMembershipCause::Cycle,
            field: CombatMembershipField::Next,
            stage: CombatMembershipStage::Admission,
            ordinal: Some(4),
            original_ordinal: Some(1),
        }
    }

    fn payload(observations: &[FirstCombatCountriesObservation]) -> Vec<u8> {
        let rows = observations.len();
        let mut bytes = vec![0; 32 + rows * ROW_LEN];
        bytes[8..12].copy_from_slice(&(rows as u32).to_le_bytes());
        for (index, observation) in observations.iter().enumerate() {
            if matches!(observation, FirstCombatCountriesObservation::Present { .. }) {
                let at = 32 + index * ROW_LEN + 48;
                bytes[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
            }
            write_movement_scalars(
                &mut bytes,
                index,
                MovementScalars {
                    first_combat: observation.projection(),
                    ..Default::default()
                },
            );
        }
        append_fixture_inputs(
            &mut bytes,
            std::iter::repeat_n((None, None), rows),
            std::iter::repeat_n(None, rows),
            std::iter::repeat_n(None, rows),
            &unavailable_roster(),
            &unavailable_railway_roster(),
        );
        let start = DivisionRoutes::parse(&bytes).unwrap().membership_start;
        bytes.truncate(start);
        bytes
            .try_reserve_exact(first_combat_countries_encoded_len(observations.iter()).unwrap())
            .unwrap();
        append_first_combat_countries(&mut bytes, observations.iter()).unwrap();
        bytes
    }

    fn put(bytes: &mut [u8], at: usize, value: u32) {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn complete_country_words_preserve_signed_order_duplicates_and_all_truth_states() {
        let observations = [
            FirstCombatCountriesObservation::default(),
            FirstCombatCountriesObservation::NoFirstCombat,
            measured(vec![]),
            measured(vec![i32::MIN, 0, -1, 0, i32::MAX]),
            FirstCombatCountriesObservation::Present {
                combat: CombatIdentity {
                    class: CombatClass::BorderWar,
                    kind: u32::MAX,
                    id: 79,
                },
                countries: CountryWordsObservation::Unavailable(cycle()),
            },
            FirstCombatCountriesObservation::Unavailable(CombatMembershipRefusal {
                cause: CombatMembershipCause::RootChanged,
                field: CombatMembershipField::FrameRoot,
                stage: CombatMembershipStage::Close,
                ordinal: None,
                original_ordinal: None,
            }),
        ];
        let bytes = payload(&observations);
        let parsed = DivisionRoutes::parse(&bytes).unwrap();
        for (index, observation) in observations.iter().enumerate() {
            assert_eq!(&parsed.first_combat_countries(index).unwrap(), observation);
            assert_eq!(
                parsed.movement_scalars(index).first_combat,
                observation.projection()
            );
        }
        assert!(parsed.first_combat_countries(observations.len()).is_err());
        assert_eq!(observations[2].json(), "{\"schema\":1,\"status\":\"present\",\"combat\":{\"class\":\"land\",\"type\":62,\"id\":79},\"countries\":{\"status\":\"measured\",\"raw_i32\":[]}}");
        assert!(observations[3]
            .json()
            .contains("[-2147483648,0,-1,0,2147483647]"));
        assert_eq!(cycle().json(), "{\"cause\":\"cycle\",\"field\":\"next\",\"stage\":\"admission\",\"ordinal\":4,\"original_ordinal\":1}");
    }

    #[test]
    fn exact_trailer_is_required_and_rejects_every_truncation_extra_and_old_marker() {
        let bytes = payload(&[measured(vec![i32::MIN, 1])]);
        let start = DivisionRoutes::parse(&bytes).unwrap().membership_start;
        for end in start..bytes.len() {
            assert!(
                DivisionRoutes::parse(&bytes[..end]).is_err(),
                "truncated at {end}"
            );
        }
        let mut bad = bytes.clone();
        bad.extend_from_slice(&0u32.to_le_bytes());
        assert!(DivisionRoutes::parse(&bad).is_err());
        for marker in [*b"D2R8", *b"D2R9"] {
            let mut bad = bytes.clone();
            bad[28..32].copy_from_slice(&marker);
            assert!(DivisionRoutes::parse(&bad).is_err());
        }
        for (at, value) in [
            (start, 0),
            (start, u32::MAX),
            (start + 4, u32::MAX),
            (start + 8 + 36, u32::MAX),
        ] {
            let mut bad = bytes.clone();
            put(&mut bad, at, value);
            assert!(DivisionRoutes::parse(&bad).is_err(), "count at {at}");
        }
        assert!(encoded_len(usize::MAX, 0).is_err());
        assert!(encoded_len(0, usize::MAX).is_err());
    }

    #[test]
    fn descriptors_reject_unknown_codes_refusal_filler_bad_ordinals_and_projection() {
        let bytes = payload(&[FirstCombatCountriesObservation::Unavailable(cycle())]);
        let at = DivisionRoutes::parse(&bytes).unwrap().membership_start + 8;
        for (offset, value) in [
            (0, 4),
            (4, 14),
            (8, 25),
            (12, 3),
            (16, 2),
            (16, 0),
            (24, 2),
            (24, 0),
            (20, 1),
            (28, 4),
            (32, 1),
            (36, 1),
        ] {
            let mut bad = bytes.clone();
            put(&mut bad, at + offset, value);
            assert!(
                DivisionRoutes::parse(&bad).is_err(),
                "descriptor cell {offset}={value}"
            );
        }
        let bytes = payload(&[measured(vec![])]);
        let at = DivisionRoutes::parse(&bytes).unwrap().membership_start + 8;
        for offset in (4..32).step_by(4) {
            let mut bad = bytes.clone();
            put(&mut bad, at + offset, 1);
            assert!(
                DivisionRoutes::parse(&bad).is_err(),
                "measured refusal cell {offset}"
            );
        }
        for (offset, value) in [
            (at, 0),
            (at, 1),
            (32 + 204, 0),
            (32 + 208, 61),
            (32 + 212, 0),
            (32 + 212, u32::MAX),
        ] {
            let mut bad = bytes.clone();
            put(&mut bad, offset, value);
            assert!(
                DivisionRoutes::parse(&bad).is_err(),
                "identity/state at {offset}"
            );
        }
        let bytes = payload(&[FirstCombatCountriesObservation::NoFirstCombat]);
        let at = DivisionRoutes::parse(&bytes).unwrap().membership_start + 8;
        for (offset, value) in [(at + 4, 1), (at + 32, 1), (at + 36, 1), (at, 2), (at, 3)] {
            let mut bad = bytes.clone();
            put(&mut bad, offset, value);
            assert!(DivisionRoutes::parse(&bad).is_err());
        }
    }

    #[test]
    fn ordered_pool_rejects_overlap_gaps_empty_alias_and_unused_words() {
        let bytes = payload(&[measured(vec![7, -7]), measured(vec![]), measured(vec![7])]);
        let start = DivisionRoutes::parse(&bytes).unwrap().membership_start;
        for (offset, value) in [
            (8 + 32, 1),
            (8 + 40 + 32, 0),
            (8 + 80 + 32, 0),
            (8 + 80 + 32, 3),
            (8 + 80 + 36, 0),
        ] {
            let mut bad = bytes.clone();
            put(&mut bad, start + offset, value);
            assert!(
                DivisionRoutes::parse(&bad).is_err(),
                "pool descriptor {offset}"
            );
        }
        let mut bad = bytes.clone();
        put(&mut bad, start + 4, 4);
        bad.extend_from_slice(&19i32.to_le_bytes());
        assert!(DivisionRoutes::parse(&bad).is_err(), "unused physical word");
    }

    #[test]
    fn deleted_descriptor_is_exactly_not_observed_and_never_measured_empty() {
        let mut bytes = payload(&[FirstCombatCountriesObservation::default()]);
        put(&mut bytes, 32 + 8, 1 << 12);
        let parsed = DivisionRoutes::parse(&bytes).unwrap();
        assert_eq!(
            parsed.first_combat_countries(0).unwrap(),
            FirstCombatCountriesObservation::default()
        );
        let at = parsed.membership_start + 8;
        for offset in 0..COMBAT_MEMBERSHIP_DESCRIPTOR_LEN {
            let mut bad = bytes.clone();
            bad[at + offset] = 1;
            assert!(
                DivisionRoutes::parse(&bad).is_err(),
                "deleted descriptor byte {offset}"
            );
        }
    }

    #[test]
    fn refusal_code_tables_and_cycle_order_are_explicit() {
        for raw in 0..14 {
            assert_eq!(CombatMembershipCause::from_raw(raw).unwrap().code(), raw);
        }
        for raw in 0..25 {
            assert_eq!(CombatMembershipField::from_raw(raw).unwrap().code(), raw);
        }
        for raw in 0..3 {
            assert_eq!(CombatMembershipStage::from_raw(raw).unwrap().code(), raw);
        }
        for (ordinal, original_ordinal) in [
            (None, Some(0)),
            (Some(0), None),
            (Some(0), Some(0)),
            (Some(1), Some(2)),
        ] {
            assert!(CombatMembershipRefusal {
                ordinal,
                original_ordinal,
                ..cycle()
            }
            .validate()
            .is_err());
        }
        assert!(CombatMembershipRefusal {
            ordinal: Some(u32::MAX),
            original_ordinal: Some(0),
            ..cycle()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn append_requires_funding_and_rejects_disagreement_before_mutating_bytes() {
        #[derive(Clone)]
        struct Miscounted<'a> {
            item: Option<&'a FirstCombatCountriesObservation>,
            announced: usize,
        }
        impl<'a> Iterator for Miscounted<'a> {
            type Item = &'a FirstCombatCountriesObservation;
            fn next(&mut self) -> Option<Self::Item> {
                self.item.take()
            }
            fn size_hint(&self) -> (usize, Option<usize>) {
                (self.announced, Some(self.announced))
            }
        }
        impl ExactSizeIterator for Miscounted<'_> {}
        let observation = measured(vec![i32::MIN, i32::MAX]);
        for announced in [0, 2, usize::MAX] {
            assert!(first_combat_countries_encoded_len(Miscounted {
                item: Some(&observation),
                announced
            })
            .is_err());
        }
        let valid = payload(std::slice::from_ref(&observation));
        let start = DivisionRoutes::parse(&valid).unwrap().membership_start;
        let mut bytes = valid[..start].to_vec().into_boxed_slice().into_vec();
        let prefix = bytes.clone();
        assert!(append_first_combat_countries(&mut bytes, std::iter::once(&observation)).is_err());
        assert_eq!(bytes, prefix);
        let additional = first_combat_countries_encoded_len(std::iter::once(&observation)).unwrap();
        assert_eq!(additional, 8 + 40 + 8);
        bytes.try_reserve_exact(additional).unwrap();
        let pointer = bytes.as_ptr();
        let capacity = bytes.capacity();
        assert!(append_first_combat_countries(
            &mut bytes,
            std::iter::once(&FirstCombatCountriesObservation::NoFirstCombat)
        )
        .is_err());
        assert_eq!(bytes, prefix);
        let other_combat = FirstCombatCountriesObservation::Present {
            combat: CombatIdentity {
                class: CombatClass::BorderWar,
                kind: 999,
                id: 79,
            },
            countries: CountryWordsObservation::Measured(vec![]),
        };
        assert!(append_first_combat_countries(&mut bytes, std::iter::once(&other_combat)).is_err());
        assert_eq!(bytes, prefix);
        append_first_combat_countries(&mut bytes, std::iter::once(&observation)).unwrap();
        assert_eq!(bytes.as_ptr(), pointer);
        assert_eq!(bytes.capacity(), capacity);
        assert_eq!(bytes, valid);
    }
}
