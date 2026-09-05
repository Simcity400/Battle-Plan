use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 67;
pub const MAGIC: &[u8; 4] = b"DMI2";
pub const SCHEMA_VERSION: u16 = 3;
pub const HEADER_LEN: usize = 104;
pub const ROW_LEN: usize = 28;
pub const VALUE_SCALE: i64 = 100_000;
pub const MAX_INSTANCES: usize = 262_144;
pub const MAX_VALUES_PER_INSTANCE: usize = 256;
pub const MAX_MODIFIER_ID_LEN: usize = 128;
pub const AUTHORITY_KIND_EXACT_A729: u32 = 1;
pub const A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
pub const STATIC_AUTHORITY_RECEIPT_SHA256: [u8; 32] = [
    0xbd, 0xba, 0xc3, 0xcb, 0x11, 0x58, 0xc9, 0x71, 0xb4, 0x08, 0x58, 0x66, 0x11, 0xd6, 0xab, 0xa6,
    0x18, 0xec, 0xfe, 0x85, 0xb5, 0x26, 0x5e, 0x3b, 0x9e, 0x8c, 0x2c, 0x0b, 0xf5, 0x45, 0xe4, 0x15,
];

const FLAG_ENABLED: u16 = 1;
const FLAG_REMAINING_DAYS: u16 = 1 << 1;
const KNOWN_FLAGS: u16 = FLAG_ENABLED | FLAG_REMAINING_DAYS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicModifierInstance {
    pub owner_tag_db_id: u32,
    pub modifier_id: String,
    pub enabled: bool,
    pub values_raw: Vec<i64>,
    pub remaining_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicModifierZeroRemoval {
    pub owner_tag_db_id: u32,
    pub modifier_id: String,
    pub enabled: bool,
    pub values_raw: Vec<i64>,
    pub observed_game_hour_raw: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicModifierAuthority {
    pub executable_sha256: [u8; 32],
    pub static_authority_receipt_sha256: [u8; 32],
}

impl DynamicModifierAuthority {
    pub const fn exact_a729() -> Self {
        Self {
            executable_sha256: A729_EXECUTABLE_SHA256,
            static_authority_receipt_sha256: STATIC_AUTHORITY_RECEIPT_SHA256,
        }
    }

    pub fn is_exact_a729(&self) -> bool {
        self == &Self::exact_a729()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicModifierFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub authority: DynamicModifierAuthority,
    pub instances: Vec<DynamicModifierInstance>,
    pub zero_removals: Vec<DynamicModifierZeroRemoval>,
    pub partiality: Vec<CountryPartiality>,
}

pub const PARTIALITY_MAGIC: &[u8; 4] = b"DMP1";
pub const PARTIALITY_ROW_LEN: usize = 24;
pub const PARTIALITY_HEADER_LEN: usize = 8;
pub const PARTIALITY_CAUSES: u32 = (1 << 18) - 1;
pub const SOURCE_UNAVAILABLE_CAUSES: u32 = (1 << 14) | (1 << 15) | (1 << 17);
pub const PREFIX_CAUSES: u32 = (1 << 9) | (1 << 12);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryPartiality {
    pub owner_tag_db_id: u32,
    pub causes: u32,
    pub first_tick: u64,
    pub last_tick: u64,
}

pub fn decode_partiality(bytes: &[u8]) -> Result<Vec<CountryPartiality>, CodecError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if bytes.len() < 8 || &bytes[..4] != PARTIALITY_MAGIC {
        return Err(CodecError::InvalidPartiality);
    }
    let count = u32_at(bytes, 4) as usize;
    if count == 0 || count > MAX_INSTANCES || bytes.len() != 8 + count * PARTIALITY_ROW_LEN {
        return Err(CodecError::InvalidPartiality);
    }
    let mut rows = Vec::with_capacity(count);
    let mut identities = BTreeSet::new();
    for row in bytes[8..].chunks_exact(PARTIALITY_ROW_LEN) {
        let gap = CountryPartiality {
            owner_tag_db_id: u32_at(row, 0),
            causes: u32_at(row, 4),
            first_tick: u64_at(row, 8),
            last_tick: u64_at(row, 16),
        };
        if gap.owner_tag_db_id > crate::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
            || gap.causes == 0 || gap.causes & !PARTIALITY_CAUSES != 0
            || gap.first_tick > gap.last_tick
            || !identities.insert((gap.owner_tag_db_id, gap.first_tick, gap.last_tick))
        {
            return Err(CodecError::InvalidPartiality);
        }
        rows.push(gap);
    }
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    InvalidPartiality,
    TooShort,
    BadMagic,
    UnsupportedVersion(u16),
    BadHeaderLength(u16),
    UnsupportedAuthorityKind(u32),
    AuthorityMismatch,
    CountOverflow,
    PayloadTooLarge(usize),
    TooManyInstances(u32),
    TooManyZeroRemovals(u32),
    TooManyValues(u32),
    LengthMismatch,
    InvalidOwner { row: usize, owner: u32 },
    InvalidFlags { row: usize, flags: u16 },
    InvalidRemainingDays { row: usize, value: u32 },
    InvalidModifierId { row: usize },
    InvalidIdWindow { row: usize },
    InvalidValueWindow { row: usize },
    DuplicateIdentity { owner: u32, modifier_id: String },
    ZeroRemovalNotMeasuredZero { row: usize },
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("checked payload"),
    )
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked payload"),
    )
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(
        bytes[offset..offset + 8]
            .try_into()
            .expect("checked payload"),
    )
}

pub fn is_v2_payload(payload: &[u8]) -> bool {
    payload.get(..MAGIC.len()) == Some(MAGIC.as_slice())
}

pub fn decode_payload(payload: &[u8]) -> Result<DynamicModifierFrame, CodecError> {
    if payload.len() < HEADER_LEN {
        return Err(CodecError::TooShort);
    }
    if payload.len() > crate::COMPACT_MAX_PAYLOAD_LEN as usize {
        return Err(CodecError::PayloadTooLarge(payload.len()));
    }
    if !is_v2_payload(payload) {
        return Err(CodecError::BadMagic);
    }
    let version = u16_at(payload, 4);
    if version != SCHEMA_VERSION {
        return Err(CodecError::UnsupportedVersion(version));
    }
    let header_len = u16_at(payload, 6);
    if usize::from(header_len) != HEADER_LEN {
        return Err(CodecError::BadHeaderLength(header_len));
    }
    let elapsed_wall_ms = u64_at(payload, 8);
    let instance_count_u32 = u32_at(payload, 16);
    let id_blob_bytes_u32 = u32_at(payload, 20);
    let total_value_count_u32 = u32_at(payload, 24);
    let status_flags = u32_at(payload, 28);
    let zero_removal_count_u32 = u32_at(payload, 32);
    let authority_kind = u32_at(payload, 36);
    if authority_kind != AUTHORITY_KIND_EXACT_A729 {
        return Err(CodecError::UnsupportedAuthorityKind(authority_kind));
    }
    let authority = DynamicModifierAuthority {
        executable_sha256: payload[40..72].try_into().expect("checked authority"),
        static_authority_receipt_sha256: payload[72..104].try_into().expect("checked authority"),
    };
    if !authority.is_exact_a729() {
        return Err(CodecError::AuthorityMismatch);
    }
    let instance_count =
        usize::try_from(instance_count_u32).map_err(|_| CodecError::CountOverflow)?;
    if instance_count > MAX_INSTANCES {
        return Err(CodecError::TooManyInstances(instance_count_u32));
    }
    let zero_removal_count =
        usize::try_from(zero_removal_count_u32).map_err(|_| CodecError::CountOverflow)?;
    if zero_removal_count > MAX_INSTANCES {
        return Err(CodecError::TooManyZeroRemovals(zero_removal_count_u32));
    }
    let total_row_count = instance_count
        .checked_add(zero_removal_count)
        .ok_or(CodecError::CountOverflow)?;
    if total_row_count > MAX_INSTANCES {
        return Err(CodecError::TooManyInstances(u32::MAX));
    }
    let id_blob_bytes =
        usize::try_from(id_blob_bytes_u32).map_err(|_| CodecError::CountOverflow)?;
    let total_value_count =
        usize::try_from(total_value_count_u32).map_err(|_| CodecError::CountOverflow)?;
    let maximum_values = total_row_count
        .checked_mul(MAX_VALUES_PER_INSTANCE)
        .ok_or(CodecError::CountOverflow)?;
    if total_value_count > maximum_values {
        return Err(CodecError::TooManyValues(total_value_count_u32));
    }
    let rows_end = HEADER_LEN
        .checked_add(
            total_row_count
                .checked_mul(ROW_LEN)
                .ok_or(CodecError::CountOverflow)?,
        )
        .ok_or(CodecError::CountOverflow)?;
    let ids_end = rows_end
        .checked_add(id_blob_bytes)
        .ok_or(CodecError::CountOverflow)?;
    let values_bytes = total_value_count
        .checked_mul(8)
        .ok_or(CodecError::CountOverflow)?;
    let expected_len = ids_end
        .checked_add(values_bytes)
        .ok_or(CodecError::CountOverflow)?;
    if payload.len() < expected_len {
        return Err(CodecError::LengthMismatch);
    }
    let partiality = decode_partiality(&payload[expected_len..])?;
    if partiality.is_empty() != (status_flags & crate::COMPACT_STATUS_PARTIAL_DATA == 0) {
        return Err(CodecError::InvalidPartiality);
    }

    let ids = &payload[rows_end..ids_end];
    let values = &payload[ids_end..];
    let mut expected_id_offset = 0usize;
    let mut expected_value_offset = 0usize;
    let mut current_identities = BTreeSet::new();
    let mut instances = Vec::with_capacity(instance_count);
    let mut zero_removals = Vec::with_capacity(zero_removal_count);
    for row in 0..total_row_count {
        let base = HEADER_LEN + row * ROW_LEN;
        let owner = u32_at(payload, base);
        if owner == 0 || owner > crate::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID {
            return Err(CodecError::InvalidOwner { row, owner });
        }
        let id_offset =
            usize::try_from(u32_at(payload, base + 4)).map_err(|_| CodecError::CountOverflow)?;
        let id_len = usize::from(u16_at(payload, base + 8));
        let flags = u16_at(payload, base + 10);
        if flags & !KNOWN_FLAGS != 0 {
            return Err(CodecError::InvalidFlags { row, flags });
        }
        let remaining_raw = u32_at(payload, base + 12);
        let value_offset =
            usize::try_from(u32_at(payload, base + 16)).map_err(|_| CodecError::CountOverflow)?;
        let value_count =
            usize::try_from(u32_at(payload, base + 20)).map_err(|_| CodecError::CountOverflow)?;
        let event_hour_raw = u32_at(payload, base + 24);
        let is_zero_removal = row >= instance_count;
        if !is_zero_removal && event_hour_raw != 0 {
            return Err(CodecError::InvalidFlags { row, flags });
        }
        if id_offset != expected_id_offset || id_len == 0 || id_len > MAX_MODIFIER_ID_LEN {
            return Err(CodecError::InvalidIdWindow { row });
        }
        let id_end = id_offset
            .checked_add(id_len)
            .ok_or(CodecError::CountOverflow)?;
        let id_bytes = ids
            .get(id_offset..id_end)
            .ok_or(CodecError::InvalidIdWindow { row })?;
        let modifier_id = std::str::from_utf8(id_bytes)
            .ok()
            .filter(|id| !id.as_bytes().contains(&0))
            .ok_or(CodecError::InvalidModifierId { row })?
            .to_string();
        if value_offset != expected_value_offset || value_count > MAX_VALUES_PER_INSTANCE {
            return Err(CodecError::InvalidValueWindow { row });
        }
        let value_end = value_offset
            .checked_add(value_count)
            .ok_or(CodecError::CountOverflow)?;
        if value_end > total_value_count {
            return Err(CodecError::InvalidValueWindow { row });
        }
        let remaining_days = if flags & FLAG_REMAINING_DAYS != 0 {
            if remaining_raw > i32::MAX as u32 {
                return Err(CodecError::InvalidRemainingDays {
                    row,
                    value: remaining_raw,
                });
            }
            Some(remaining_raw)
        } else {
            if remaining_raw != 0 {
                return Err(CodecError::InvalidRemainingDays {
                    row,
                    value: remaining_raw,
                });
            }
            None
        };
        let mut values_raw = Vec::with_capacity(value_count);
        for value_index in value_offset..value_end {
            let offset = value_index * 8;
            values_raw.push(i64::from_le_bytes(
                values[offset..offset + 8]
                    .try_into()
                    .expect("checked value window"),
            ));
        }
        if !is_zero_removal && !current_identities.insert((owner, modifier_id.clone())) {
            return Err(CodecError::DuplicateIdentity { owner, modifier_id });
        }
        if is_zero_removal {
            if remaining_days != Some(0) {
                return Err(CodecError::ZeroRemovalNotMeasuredZero { row });
            }
            zero_removals.push(DynamicModifierZeroRemoval {
                owner_tag_db_id: owner,
                modifier_id,
                enabled: flags & FLAG_ENABLED != 0,
                values_raw,
                observed_game_hour_raw: event_hour_raw as i32,
            });
        } else {
            instances.push(DynamicModifierInstance {
                owner_tag_db_id: owner,
                modifier_id,
                enabled: flags & FLAG_ENABLED != 0,
                values_raw,
                remaining_days,
            });
        }
        expected_id_offset = id_end;
        expected_value_offset = value_end;
    }
    if expected_id_offset != id_blob_bytes || expected_value_offset != total_value_count {
        return Err(CodecError::LengthMismatch);
    }
    Ok(DynamicModifierFrame {
        elapsed_wall_ms,
        status_flags,
        authority,
        instances,
        zero_removals,
        partiality,
    })
}

pub fn encode_payload(frame: &DynamicModifierFrame) -> Result<Vec<u8>, CodecError> {
    if frame.partiality.is_empty() != (frame.status_flags & crate::COMPACT_STATUS_PARTIAL_DATA == 0) {
        return Err(CodecError::InvalidPartiality);
    }
    if !frame.authority.is_exact_a729() {
        return Err(CodecError::AuthorityMismatch);
    }
    if frame.instances.len() > MAX_INSTANCES {
        return Err(CodecError::TooManyInstances(u32::MAX));
    }
    if frame.zero_removals.len() > MAX_INSTANCES {
        return Err(CodecError::TooManyZeroRemovals(u32::MAX));
    }
    let total_rows = frame
        .instances
        .len()
        .checked_add(frame.zero_removals.len())
        .ok_or(CodecError::CountOverflow)?;
    if total_rows > MAX_INSTANCES {
        return Err(CodecError::TooManyInstances(u32::MAX));
    }
    let rows = frame
        .instances
        .iter()
        .map(|instance| {
            (
                instance.owner_tag_db_id,
                instance.modifier_id.as_str(),
                instance.enabled,
                instance.values_raw.as_slice(),
                instance.remaining_days,
                0u32,
                false,
            )
        })
        .chain(frame.zero_removals.iter().map(|event| {
            (
                event.owner_tag_db_id,
                event.modifier_id.as_str(),
                event.enabled,
                event.values_raw.as_slice(),
                Some(0),
                event.observed_game_hour_raw as u32,
                true,
            )
        }))
        .collect::<Vec<_>>();
    let mut current_identities = BTreeSet::new();
    let mut id_bytes = 0usize;
    let mut value_count = 0usize;
    for (row, &(owner, modifier_id, _enabled, values_raw, remaining_days, _hour, is_zero)) in
        rows.iter().enumerate()
    {
        if owner == 0 || owner > crate::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID {
            return Err(CodecError::InvalidOwner { row, owner });
        }
        if modifier_id.is_empty()
            || modifier_id.len() > MAX_MODIFIER_ID_LEN
            || modifier_id.as_bytes().contains(&0)
        {
            return Err(CodecError::InvalidModifierId { row });
        }
        if values_raw.len() > MAX_VALUES_PER_INSTANCE {
            return Err(CodecError::InvalidValueWindow { row });
        }
        if remaining_days.is_some_and(|days| days > i32::MAX as u32) {
            return Err(CodecError::InvalidRemainingDays {
                row,
                value: remaining_days.unwrap_or_default(),
            });
        }
        if !is_zero && !current_identities.insert((owner, modifier_id)) {
            return Err(CodecError::DuplicateIdentity {
                owner,
                modifier_id: modifier_id.to_string(),
            });
        }
        id_bytes = id_bytes
            .checked_add(modifier_id.len())
            .ok_or(CodecError::CountOverflow)?;
        value_count = value_count
            .checked_add(values_raw.len())
            .ok_or(CodecError::CountOverflow)?;
    }
    let payload_len = HEADER_LEN
        .checked_add(
            total_rows
                .checked_mul(ROW_LEN)
                .ok_or(CodecError::CountOverflow)?,
        )
        .and_then(|len| len.checked_add(id_bytes))
        .and_then(|len| len.checked_add(value_count.checked_mul(8)?))
        .ok_or(CodecError::CountOverflow)?;
    let payload_len = payload_len.checked_add(if frame.partiality.is_empty() { 0 } else {
        8 + frame.partiality.len() * PARTIALITY_ROW_LEN
    }).ok_or(CodecError::CountOverflow)?;
    if payload_len > crate::COMPACT_MAX_PAYLOAD_LEN as usize {
        return Err(CodecError::PayloadTooLarge(payload_len));
    }
    let instance_count =
        u32::try_from(frame.instances.len()).map_err(|_| CodecError::CountOverflow)?;
    let zero_removal_count =
        u32::try_from(frame.zero_removals.len()).map_err(|_| CodecError::CountOverflow)?;
    let id_blob_bytes = u32::try_from(id_bytes).map_err(|_| CodecError::CountOverflow)?;
    let total_value_count = u32::try_from(value_count).map_err(|_| CodecError::CountOverflow)?;
    let mut out = Vec::with_capacity(payload_len);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&frame.elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&instance_count.to_le_bytes());
    out.extend_from_slice(&id_blob_bytes.to_le_bytes());
    out.extend_from_slice(&total_value_count.to_le_bytes());
    out.extend_from_slice(&frame.status_flags.to_le_bytes());
    out.extend_from_slice(&zero_removal_count.to_le_bytes());
    out.extend_from_slice(&AUTHORITY_KIND_EXACT_A729.to_le_bytes());
    out.extend_from_slice(&frame.authority.executable_sha256);
    out.extend_from_slice(&frame.authority.static_authority_receipt_sha256);
    let mut id_offset = 0u32;
    let mut value_offset = 0u32;
    for &(owner, modifier_id, enabled, values_raw, remaining_days, event_hour, _) in &rows {
        let mut flags = if enabled { FLAG_ENABLED } else { 0 };
        if remaining_days.is_some() {
            flags |= FLAG_REMAINING_DAYS;
        }
        out.extend_from_slice(&owner.to_le_bytes());
        out.extend_from_slice(&id_offset.to_le_bytes());
        out.extend_from_slice(&(modifier_id.len() as u16).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&remaining_days.unwrap_or_default().to_le_bytes());
        out.extend_from_slice(&value_offset.to_le_bytes());
        out.extend_from_slice(&(values_raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&event_hour.to_le_bytes());
        id_offset += modifier_id.len() as u32;
        value_offset += values_raw.len() as u32;
    }
    for &(_, modifier_id, _, _, _, _, _) in &rows {
        out.extend_from_slice(modifier_id.as_bytes());
    }
    for &(_, _, _, values_raw, _, _, _) in &rows {
        for value in values_raw {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    if !frame.partiality.is_empty() {
        let start = out.len();
        out.extend_from_slice(PARTIALITY_MAGIC);
        out.extend_from_slice(&(frame.partiality.len() as u32).to_le_bytes());
        for gap in &frame.partiality {
            out.extend_from_slice(&gap.owner_tag_db_id.to_le_bytes());
            out.extend_from_slice(&gap.causes.to_le_bytes());
            out.extend_from_slice(&gap.first_tick.to_le_bytes());
            out.extend_from_slice(&gap.last_tick.to_le_bytes());
        }
        decode_partiality(&out[start..])?;
        if frame.status_flags & crate::COMPACT_STATUS_PARTIAL_DATA == 0 {
            return Err(CodecError::InvalidPartiality);
        }
    }
    debug_assert_eq!(out.len(), payload_len);
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleTransitionKind {
    BaselinePresent,
    Added,
    ZeroObserved,
    TimedCounterChanged,
    DurationKindChanged,
    RemovedAfterObservedZero,
    RemovedWithoutObservedZero,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleTransition {
    pub owner_tag_db_id: u32,
    pub modifier_id: String,
    pub kind: LifecycleTransitionKind,
    pub previous_remaining_days: Option<u32>,
    pub current_remaining_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleObservation {
    pub baseline: bool,
    pub transitions: Vec<LifecycleTransition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvidenceError {
    ContinuityUnavailable,
    MissingPrior { owner: u32, modifier_id: String },
    PriorWasNotOne { owner: u32, modifier_id: String },
    ExactFieldsChanged { owner: u32, modifier_id: String },
}

#[derive(Debug, Default, Clone)]
pub struct LifecycleTracker {
    prior: BTreeMap<(u32, String), DynamicModifierInstance>,
    continuous: bool,
    partial_owners: BTreeSet<u32>,
}

impl LifecycleTracker {
    pub fn invalidate_continuity(&mut self) {
        self.continuous = false;
    }

    pub fn invalidate_countries(&mut self, owners: impl IntoIterator<Item = u32>) {
        self.partial_owners.extend(owners);
        self.prior.retain(|(owner, _), _| !self.partial_owners.contains(owner));
    }

    pub fn observe_zero_removals(
        &mut self,
        events: &[DynamicModifierZeroRemoval],
    ) -> Result<LifecycleObservation, LifecycleEvidenceError> {
        if events.is_empty() {
            return Ok(LifecycleObservation {
                baseline: false,
                transitions: Vec::new(),
            });
        }
        if !self.continuous {
            return Err(LifecycleEvidenceError::ContinuityUnavailable);
        }
        let mut transitions = Vec::with_capacity(events.len());
        for event in events {
            let key = (event.owner_tag_db_id, event.modifier_id.clone());
            if self.partial_owners.contains(&event.owner_tag_db_id) {
                continue;
            }
            let Some(previous) = self.prior.get(&key) else {
                return Err(LifecycleEvidenceError::MissingPrior {
                    owner: event.owner_tag_db_id,
                    modifier_id: event.modifier_id.clone(),
                });
            };
            if previous.remaining_days != Some(1) {
                return Err(LifecycleEvidenceError::PriorWasNotOne {
                    owner: event.owner_tag_db_id,
                    modifier_id: event.modifier_id.clone(),
                });
            }
            if previous.enabled != event.enabled || previous.values_raw != event.values_raw {
                return Err(LifecycleEvidenceError::ExactFieldsChanged {
                    owner: event.owner_tag_db_id,
                    modifier_id: event.modifier_id.clone(),
                });
            }
            self.prior.remove(&key);
            transitions.push(LifecycleTransition {
                owner_tag_db_id: event.owner_tag_db_id,
                modifier_id: event.modifier_id.clone(),
                kind: LifecycleTransitionKind::RemovedAfterObservedZero,
                previous_remaining_days: Some(0),
                current_remaining_days: None,
            });
        }
        Ok(LifecycleObservation {
            baseline: false,
            transitions,
        })
    }

    pub fn observe(&mut self, instances: &[DynamicModifierInstance]) -> LifecycleObservation {
        let current = instances
            .iter()
            .cloned()
            .map(|instance| {
                (
                    (instance.owner_tag_db_id, instance.modifier_id.clone()),
                    instance,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let baseline = !self.continuous;
        let mut transitions = Vec::new();
        if baseline {
            transitions.extend(current.values().map(|instance| LifecycleTransition {
                owner_tag_db_id: instance.owner_tag_db_id,
                modifier_id: instance.modifier_id.clone(),
                kind: LifecycleTransitionKind::BaselinePresent,
                previous_remaining_days: None,
                current_remaining_days: instance.remaining_days,
            }));
        } else {
            for (key, instance) in &current {
                if self.partial_owners.contains(&instance.owner_tag_db_id) {
                    transitions.push(LifecycleTransition {
                        owner_tag_db_id: instance.owner_tag_db_id,
                        modifier_id: instance.modifier_id.clone(),
                        kind: LifecycleTransitionKind::BaselinePresent,
                        previous_remaining_days: None,
                        current_remaining_days: instance.remaining_days,
                    });
                    continue;
                }
                match self.prior.get(key) {
                    None => transitions.push(LifecycleTransition {
                        owner_tag_db_id: instance.owner_tag_db_id,
                        modifier_id: instance.modifier_id.clone(),
                        kind: LifecycleTransitionKind::Added,
                        previous_remaining_days: None,
                        current_remaining_days: instance.remaining_days,
                    }),
                    Some(previous)
                        if previous.remaining_days.is_none()
                            != instance.remaining_days.is_none() =>
                    {
                        transitions.push(LifecycleTransition {
                            owner_tag_db_id: instance.owner_tag_db_id,
                            modifier_id: instance.modifier_id.clone(),
                            kind: LifecycleTransitionKind::DurationKindChanged,
                            previous_remaining_days: previous.remaining_days,
                            current_remaining_days: instance.remaining_days,
                        });
                    }
                    Some(previous) if previous.remaining_days != instance.remaining_days => {
                        transitions.push(LifecycleTransition {
                            owner_tag_db_id: instance.owner_tag_db_id,
                            modifier_id: instance.modifier_id.clone(),
                            kind: if instance.remaining_days == Some(0) {
                                LifecycleTransitionKind::ZeroObserved
                            } else {
                                LifecycleTransitionKind::TimedCounterChanged
                            },
                            previous_remaining_days: previous.remaining_days,
                            current_remaining_days: instance.remaining_days,
                        });
                    }
                    Some(_) => {}
                }
            }
            for (key, previous) in &self.prior {
                if current.contains_key(key) {
                    continue;
                }
                transitions.push(LifecycleTransition {
                    owner_tag_db_id: previous.owner_tag_db_id,
                    modifier_id: previous.modifier_id.clone(),
                    kind: if previous.remaining_days == Some(0) {
                        LifecycleTransitionKind::RemovedAfterObservedZero
                    } else {
                        LifecycleTransitionKind::RemovedWithoutObservedZero
                    },
                    previous_remaining_days: previous.remaining_days,
                    current_remaining_days: None,
                });
            }
        }
        self.partial_owners.clear();
        self.prior = current;
        self.continuous = true;
        LifecycleObservation {
            baseline,
            transitions,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, days: Option<u32>) -> DynamicModifierInstance {
        DynamicModifierInstance {
            owner_tag_db_id: 33,
            modifier_id: id.into(),
            enabled: true,
            values_raw: vec![15_000, -10_000, 0],
            remaining_days: days,
        }
    }

    #[test]
    fn codec_roundtrip_preserves_permanent_timed_and_measured_zero() {
        let frame = DynamicModifierFrame {
            partiality: Vec::new(),
            elapsed_wall_ms: 77,
            status_flags: 0,
            authority: DynamicModifierAuthority::exact_a729(),
            instances: vec![
                row("permanent", None),
                row("timed", Some(7)),
                row("zero", Some(0)),
            ],
            zero_removals: Vec::new(),
        };
        let payload = encode_payload(&frame).unwrap();
        assert_eq!(decode_payload(&payload), Ok(frame));
    }

    #[test]
    fn codec_roundtrip_keeps_synchronous_zero_removal_separate_from_current_set() {
        let frame = DynamicModifierFrame {
            partiality: Vec::new(),
            elapsed_wall_ms: 77,
            status_flags: 0,
            authority: DynamicModifierAuthority::exact_a729(),
            instances: vec![row("permanent", None)],
            zero_removals: vec![DynamicModifierZeroRemoval {
                owner_tag_db_id: 33,
                modifier_id: "timed".into(),
                enabled: true,
                values_raw: vec![15_000, -10_000, 0],
                observed_game_hour_raw: 60_800_000,
            }],
        };
        let payload = encode_payload(&frame).unwrap();
        assert_eq!(decode_payload(&payload), Ok(frame));
    }

    #[test]
    fn removal_without_observed_zero_is_not_classified_as_expiry() {
        let mut tracker = LifecycleTracker::default();
        tracker.observe(&[row("timed", Some(2))]);
        let observation = tracker.observe(&[]);
        assert_eq!(
            observation.transitions[0].kind,
            LifecycleTransitionKind::RemovedWithoutObservedZero
        );
    }

    #[test]
    fn exact_zero_then_removal_is_distinct() {
        let mut tracker = LifecycleTracker::default();
        tracker.observe(&[row("timed", Some(1))]);
        let zero = tracker.observe(&[row("timed", Some(0))]);
        assert_eq!(
            zero.transitions[0].kind,
            LifecycleTransitionKind::ZeroObserved
        );
        let removed = tracker.observe(&[]);
        assert_eq!(
            removed.transitions[0].kind,
            LifecycleTransitionKind::RemovedAfterObservedZero
        );
    }

    #[test]
    fn synchronous_zero_removal_event_closes_prior_one_without_a_fake_current_zero() {
        let mut tracker = LifecycleTracker::default();
        tracker.observe(&[row("timed", Some(1)), row("permanent", None)]);
        let removal = tracker
            .observe_zero_removals(&[DynamicModifierZeroRemoval {
                owner_tag_db_id: 33,
                modifier_id: "timed".into(),
                enabled: true,
                values_raw: vec![15_000, -10_000, 0],
                observed_game_hour_raw: 124,
            }])
            .unwrap();
        assert_eq!(
            removal.transitions[0].kind,
            LifecycleTransitionKind::RemovedAfterObservedZero
        );
        assert_eq!(removal.transitions[0].previous_remaining_days, Some(0));
        let current = tracker.observe(&[row("permanent", None)]);
        assert!(current.transitions.is_empty());
    }

    #[test]
    fn gap_forces_fresh_baseline_and_suppresses_removal_inference() {
        let mut tracker = LifecycleTracker::default();
        tracker.observe(&[row("timed", Some(0))]);
        tracker.invalidate_continuity();
        let next = tracker.observe(&[]);
        assert!(next.baseline);
        assert!(next.transitions.is_empty());
    }

    #[test]
    fn decoder_rejects_duplicate_identity_and_noncanonical_windows() {
        let duplicate = DynamicModifierFrame {
            partiality: Vec::new(),
            elapsed_wall_ms: 1,
            status_flags: 0,
            authority: DynamicModifierAuthority::exact_a729(),
            instances: vec![row("same", Some(1)), row("same", Some(1))],
            zero_removals: Vec::new(),
        };
        assert!(matches!(
            encode_payload(&duplicate),
            Err(CodecError::DuplicateIdentity { .. })
        ));

        let good = DynamicModifierFrame {
            partiality: Vec::new(),
            elapsed_wall_ms: 1,
            status_flags: 0,
            authority: DynamicModifierAuthority::exact_a729(),
            instances: vec![row("one", Some(1))],
            zero_removals: Vec::new(),
        };
        let mut payload = encode_payload(&good).unwrap();
        payload[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_payload(&payload),
            Err(CodecError::InvalidIdWindow { row: 0 })
        );

        let mut forged = good;
        forged.authority.static_authority_receipt_sha256[0] ^= 0xff;
        assert_eq!(encode_payload(&forged), Err(CodecError::AuthorityMismatch));
    }
}

#[cfg(test)]
mod typed_partiality_tests {
    use super::*;

    #[test]
    fn d67_partiality_roundtrip_and_country_continuity() {
        let row = |owner, days| DynamicModifierInstance {
            owner_tag_db_id: owner, modifier_id: "m".into(), enabled: true,
            values_raw: vec![-1, 0, i64::MAX], remaining_days: Some(days),
        };
        let frame = DynamicModifierFrame {
            elapsed_wall_ms: 1, status_flags: crate::COMPACT_STATUS_PARTIAL_DATA,
            authority: DynamicModifierAuthority::exact_a729(),
            instances: vec![row(1, 2), row(2, 3)], zero_removals: vec![],
            partiality: vec![CountryPartiality { owner_tag_db_id: 2, causes: 1 << 6, first_tick: 10, last_tick: 11 }],
        };
        let bytes = encode_payload(&frame).unwrap();
        assert_eq!(decode_payload(&bytes).unwrap(), frame);
        for cut in 1..24 { assert!(decode_payload(&bytes[..bytes.len()-cut]).is_err()); }
        let mut tracker = LifecycleTracker::default();
        tracker.observe(&[row(1, 3), row(2, 4)]);
        tracker.invalidate_countries([2]);
        let observation = tracker.observe(&frame.instances);
        assert!(!observation.baseline);
        assert_eq!(observation.transitions[0].kind, LifecycleTransitionKind::TimedCounterChanged);
        assert_eq!(observation.transitions[1].kind, LifecycleTransitionKind::BaselinePresent);
        tracker.invalidate_countries([2]);
        let next = tracker.observe(&[row(1, 1), row(2, 2)]);
        assert_eq!(next.transitions[0].kind, LifecycleTransitionKind::TimedCounterChanged);
        assert_eq!(next.transitions[1].kind, LifecycleTransitionKind::BaselinePresent);
    }
    #[test]
    fn d67_zero_event_multiplicity_and_partial_status_are_lossless() {
        let event = DynamicModifierZeroRemoval {
            owner_tag_db_id: 33, modifier_id: "readded".into(), enabled: true,
            values_raw: vec![1], observed_game_hour_raw: 60_832_992,
        };
        let mut frame = DynamicModifierFrame {
            elapsed_wall_ms: 1, status_flags: 0, authority: DynamicModifierAuthority::exact_a729(),
            instances: Vec::new(), zero_removals: vec![event.clone(), event], partiality: Vec::new(),
        };
        assert_eq!(decode_payload(&encode_payload(&frame).unwrap()).unwrap(), frame);
        frame.status_flags = crate::COMPACT_STATUS_PARTIAL_DATA;
        assert_eq!(encode_payload(&frame), Err(CodecError::InvalidPartiality));
        frame.partiality.push(CountryPartiality { owner_tag_db_id: 33, causes: 1 << 11, first_tick: 1, last_tick: 2 });
        let mut bytes = encode_payload(&frame).unwrap();
        let start = bytes.len() - PARTIALITY_ROW_LEN;
        bytes[start + 4..start + 8].copy_from_slice(&(1u32 << 18).to_le_bytes());
        assert_eq!(decode_payload(&bytes), Err(CodecError::InvalidPartiality));
    }

}
