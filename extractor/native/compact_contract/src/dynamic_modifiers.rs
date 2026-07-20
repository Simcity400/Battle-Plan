//! Strict pointer-free country dynamic-modifier instance carrier (domain 67 v2).
//!
//! The legacy domain-67 payload carries only active definition ids. `DMI2`
//! adds the exact fields established by the a729 serializer/decrement bodies:
//! enabled, the signed fixed-five value vector, and the signed remaining-day
//! counter (`-1` is encoded as `None`). The header also carries the exact-a729
//! executable and static-authority receipt hashes; decoders reject any other
//! producer authority. It deliberately carries no absolute expiry hour: the
//! counter advances on the country's daily tick, whose phase is not derivable
//! from an hourly observation.
//!
//! An H4CR record carrying this payload must set
//! `COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2`. The flag is the discriminator;
//! payload magic alone is never used to reinterpret a legacy elapsed-wall-ms
//! header.

use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 67;
pub const MAGIC: &[u8; 4] = b"DMI2";
pub const SCHEMA_VERSION: u16 = 2;
pub const HEADER_LEN: usize = 100;
pub const ROW_LEN: usize = 28;
pub const VALUE_SCALE: i64 = 100_000;
pub const MAX_INSTANCES: usize = 262_144;
pub const MAX_VALUES_PER_INSTANCE: usize = 256;
pub const MAX_MODIFIER_ID_LEN: usize = 128;
pub const AUTHORITY_KIND_EXACT_A729: u32 = 1;
pub const A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94,
    0x77, 0x75, 0xc4, 0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82,
    0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
pub const STATIC_AUTHORITY_RECEIPT_SHA256: [u8; 32] = [
    0xbd, 0xba, 0xc3, 0xcb, 0x11, 0x58, 0xc9, 0x71, 0xb4, 0x08, 0x58, 0x66, 0x11,
    0xd6, 0xab, 0xa6, 0x18, 0xec, 0xfe, 0x85, 0xb5, 0x26, 0x5e, 0x3b, 0x9e, 0x8c,
    0x2c, 0x0b, 0xf5, 0x45, 0xe4, 0x15,
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
    /// `None` is the engine's exact `-1` permanent sentinel. `Some(0)` is a
    /// measured zero and must not be collapsed into absence.
    pub remaining_days: Option<u32>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    TooShort,
    BadMagic,
    UnsupportedVersion(u16),
    BadHeaderLength(u16),
    UnsupportedAuthorityKind(u32),
    AuthorityMismatch,
    CountOverflow,
    PayloadTooLarge(usize),
    TooManyInstances(u32),
    TooManyValues(u32),
    LengthMismatch,
    InvalidOwner { row: usize, owner: u32 },
    InvalidFlags { row: usize, flags: u16 },
    InvalidRemainingDays { row: usize, value: u32 },
    InvalidModifierId { row: usize },
    InvalidIdWindow { row: usize },
    InvalidValueWindow { row: usize },
    DuplicateIdentity { owner: u32, modifier_id: String },
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("checked payload"))
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("checked payload"))
}

fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("checked payload"))
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
    let authority_kind = u32_at(payload, 32);
    if authority_kind != AUTHORITY_KIND_EXACT_A729 {
        return Err(CodecError::UnsupportedAuthorityKind(authority_kind));
    }
    let authority = DynamicModifierAuthority {
        executable_sha256: payload[36..68].try_into().expect("checked authority"),
        static_authority_receipt_sha256: payload[68..100]
            .try_into()
            .expect("checked authority"),
    };
    if !authority.is_exact_a729() {
        return Err(CodecError::AuthorityMismatch);
    }
    let instance_count = usize::try_from(instance_count_u32).map_err(|_| CodecError::CountOverflow)?;
    if instance_count > MAX_INSTANCES {
        return Err(CodecError::TooManyInstances(instance_count_u32));
    }
    let id_blob_bytes = usize::try_from(id_blob_bytes_u32).map_err(|_| CodecError::CountOverflow)?;
    let total_value_count = usize::try_from(total_value_count_u32).map_err(|_| CodecError::CountOverflow)?;
    let maximum_values = instance_count
        .checked_mul(MAX_VALUES_PER_INSTANCE)
        .ok_or(CodecError::CountOverflow)?;
    if total_value_count > maximum_values {
        return Err(CodecError::TooManyValues(total_value_count_u32));
    }
    let rows_end = HEADER_LEN
        .checked_add(instance_count.checked_mul(ROW_LEN).ok_or(CodecError::CountOverflow)?)
        .ok_or(CodecError::CountOverflow)?;
    let ids_end = rows_end.checked_add(id_blob_bytes).ok_or(CodecError::CountOverflow)?;
    let values_bytes = total_value_count.checked_mul(8).ok_or(CodecError::CountOverflow)?;
    let expected_len = ids_end.checked_add(values_bytes).ok_or(CodecError::CountOverflow)?;
    if payload.len() != expected_len {
        return Err(CodecError::LengthMismatch);
    }

    let ids = &payload[rows_end..ids_end];
    let values = &payload[ids_end..];
    let mut expected_id_offset = 0usize;
    let mut expected_value_offset = 0usize;
    let mut identities = BTreeSet::new();
    let mut instances = Vec::with_capacity(instance_count);
    for row in 0..instance_count {
        let base = HEADER_LEN + row * ROW_LEN;
        let owner = u32_at(payload, base);
        if owner == 0 || owner > crate::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID {
            return Err(CodecError::InvalidOwner { row, owner });
        }
        let id_offset = usize::try_from(u32_at(payload, base + 4)).map_err(|_| CodecError::CountOverflow)?;
        let id_len = usize::from(u16_at(payload, base + 8));
        let flags = u16_at(payload, base + 10);
        if flags & !KNOWN_FLAGS != 0 {
            return Err(CodecError::InvalidFlags { row, flags });
        }
        let remaining_raw = u32_at(payload, base + 12);
        let value_offset = usize::try_from(u32_at(payload, base + 16)).map_err(|_| CodecError::CountOverflow)?;
        let value_count = usize::try_from(u32_at(payload, base + 20)).map_err(|_| CodecError::CountOverflow)?;
        if u32_at(payload, base + 24) != 0 {
            return Err(CodecError::InvalidFlags { row, flags });
        }
        if id_offset != expected_id_offset || id_len == 0 || id_len > MAX_MODIFIER_ID_LEN {
            return Err(CodecError::InvalidIdWindow { row });
        }
        let id_end = id_offset.checked_add(id_len).ok_or(CodecError::CountOverflow)?;
        let id_bytes = ids.get(id_offset..id_end).ok_or(CodecError::InvalidIdWindow { row })?;
        let modifier_id = std::str::from_utf8(id_bytes)
            .ok()
            .filter(|id| !id.as_bytes().contains(&0))
            .ok_or(CodecError::InvalidModifierId { row })?
            .to_string();
        if value_offset != expected_value_offset || value_count > MAX_VALUES_PER_INSTANCE {
            return Err(CodecError::InvalidValueWindow { row });
        }
        let value_end = value_offset.checked_add(value_count).ok_or(CodecError::CountOverflow)?;
        if value_end > total_value_count {
            return Err(CodecError::InvalidValueWindow { row });
        }
        let remaining_days = if flags & FLAG_REMAINING_DAYS != 0 {
            if remaining_raw > i32::MAX as u32 {
                return Err(CodecError::InvalidRemainingDays { row, value: remaining_raw });
            }
            Some(remaining_raw)
        } else {
            if remaining_raw != 0 {
                return Err(CodecError::InvalidRemainingDays { row, value: remaining_raw });
            }
            None
        };
        let mut values_raw = Vec::with_capacity(value_count);
        for value_index in value_offset..value_end {
            let offset = value_index * 8;
            values_raw.push(i64::from_le_bytes(
                values[offset..offset + 8].try_into().expect("checked value window"),
            ));
        }
        if !identities.insert((owner, modifier_id.clone())) {
            return Err(CodecError::DuplicateIdentity { owner, modifier_id });
        }
        instances.push(DynamicModifierInstance {
            owner_tag_db_id: owner,
            modifier_id,
            enabled: flags & FLAG_ENABLED != 0,
            values_raw,
            remaining_days,
        });
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
    })
}

pub fn encode_payload(frame: &DynamicModifierFrame) -> Result<Vec<u8>, CodecError> {
    if !frame.authority.is_exact_a729() {
        return Err(CodecError::AuthorityMismatch);
    }
    if frame.instances.len() > MAX_INSTANCES {
        return Err(CodecError::TooManyInstances(u32::MAX));
    }
    let mut identities = BTreeSet::new();
    let mut id_bytes = 0usize;
    let mut value_count = 0usize;
    for (row, instance) in frame.instances.iter().enumerate() {
        if instance.owner_tag_db_id == 0
            || instance.owner_tag_db_id > crate::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID
        {
            return Err(CodecError::InvalidOwner { row, owner: instance.owner_tag_db_id });
        }
        if instance.modifier_id.is_empty()
            || instance.modifier_id.len() > MAX_MODIFIER_ID_LEN
            || instance.modifier_id.as_bytes().contains(&0)
        {
            return Err(CodecError::InvalidModifierId { row });
        }
        if instance.values_raw.len() > MAX_VALUES_PER_INSTANCE {
            return Err(CodecError::InvalidValueWindow { row });
        }
        if instance.remaining_days.is_some_and(|days| days > i32::MAX as u32) {
            return Err(CodecError::InvalidRemainingDays {
                row,
                value: instance.remaining_days.unwrap_or_default(),
            });
        }
        if !identities.insert((instance.owner_tag_db_id, instance.modifier_id.as_str())) {
            return Err(CodecError::DuplicateIdentity {
                owner: instance.owner_tag_db_id,
                modifier_id: instance.modifier_id.clone(),
            });
        }
        id_bytes = id_bytes.checked_add(instance.modifier_id.len()).ok_or(CodecError::CountOverflow)?;
        value_count = value_count.checked_add(instance.values_raw.len()).ok_or(CodecError::CountOverflow)?;
    }
    let payload_len = HEADER_LEN
        .checked_add(frame.instances.len().checked_mul(ROW_LEN).ok_or(CodecError::CountOverflow)?)
        .and_then(|len| len.checked_add(id_bytes))
        .and_then(|len| len.checked_add(value_count.checked_mul(8)?))
        .ok_or(CodecError::CountOverflow)?;
    if payload_len > crate::COMPACT_MAX_PAYLOAD_LEN as usize {
        return Err(CodecError::PayloadTooLarge(payload_len));
    }
    let instance_count = u32::try_from(frame.instances.len()).map_err(|_| CodecError::CountOverflow)?;
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
    out.extend_from_slice(&AUTHORITY_KIND_EXACT_A729.to_le_bytes());
    out.extend_from_slice(&frame.authority.executable_sha256);
    out.extend_from_slice(&frame.authority.static_authority_receipt_sha256);
    let mut id_offset = 0u32;
    let mut value_offset = 0u32;
    for instance in &frame.instances {
        let mut flags = if instance.enabled { FLAG_ENABLED } else { 0 };
        if instance.remaining_days.is_some() {
            flags |= FLAG_REMAINING_DAYS;
        }
        out.extend_from_slice(&instance.owner_tag_db_id.to_le_bytes());
        out.extend_from_slice(&id_offset.to_le_bytes());
        out.extend_from_slice(&(instance.modifier_id.len() as u16).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&instance.remaining_days.unwrap_or_default().to_le_bytes());
        out.extend_from_slice(&value_offset.to_le_bytes());
        out.extend_from_slice(&(instance.values_raw.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        id_offset += instance.modifier_id.len() as u32;
        value_offset += instance.values_raw.len() as u32;
    }
    for instance in &frame.instances {
        out.extend_from_slice(instance.modifier_id.as_bytes());
    }
    for instance in &frame.instances {
        for value in &instance.values_raw {
            out.extend_from_slice(&value.to_le_bytes());
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

/// Stateful, gap-sensitive transition classifier. `invalidate_continuity` must
/// be called for every missing, failed, malformed, or legacy-only frame. The
/// next good v2 frame then becomes a fresh baseline; it can never turn a gap
/// into an inferred expiry.
#[derive(Debug, Default, Clone)]
pub struct LifecycleTracker {
    prior: BTreeMap<(u32, String), DynamicModifierInstance>,
    continuous: bool,
}

impl LifecycleTracker {
    pub fn invalidate_continuity(&mut self) {
        self.continuous = false;
    }

    pub fn observe(&mut self, instances: &[DynamicModifierInstance]) -> LifecycleObservation {
        let current = instances
            .iter()
            .cloned()
            .map(|instance| {
                ((instance.owner_tag_db_id, instance.modifier_id.clone()), instance)
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
                match self.prior.get(key) {
                    None => transitions.push(LifecycleTransition {
                        owner_tag_db_id: instance.owner_tag_db_id,
                        modifier_id: instance.modifier_id.clone(),
                        kind: LifecycleTransitionKind::Added,
                        previous_remaining_days: None,
                        current_remaining_days: instance.remaining_days,
                    }),
                    Some(previous) if previous.remaining_days.is_none() != instance.remaining_days.is_none() => {
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
        self.prior = current;
        self.continuous = true;
        LifecycleObservation { baseline, transitions }
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
            elapsed_wall_ms: 77,
            status_flags: 0,
            authority: DynamicModifierAuthority::exact_a729(),
            instances: vec![row("permanent", None), row("timed", Some(7)), row("zero", Some(0))],
        };
        let payload = encode_payload(&frame).unwrap();
        assert_eq!(decode_payload(&payload), Ok(frame));
    }

    #[test]
    fn removal_without_observed_zero_is_not_classified_as_expiry() {
        let mut tracker = LifecycleTracker::default();
        tracker.observe(&[row("timed", Some(2))]);
        let observation = tracker.observe(&[]);
        assert_eq!(observation.transitions[0].kind, LifecycleTransitionKind::RemovedWithoutObservedZero);
    }

    #[test]
    fn exact_zero_then_removal_is_distinct() {
        let mut tracker = LifecycleTracker::default();
        tracker.observe(&[row("timed", Some(1))]);
        let zero = tracker.observe(&[row("timed", Some(0))]);
        assert_eq!(zero.transitions[0].kind, LifecycleTransitionKind::ZeroObserved);
        let removed = tracker.observe(&[]);
        assert_eq!(removed.transitions[0].kind, LifecycleTransitionKind::RemovedAfterObservedZero);
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
            elapsed_wall_ms: 1,
            status_flags: 0,
            authority: DynamicModifierAuthority::exact_a729(),
            instances: vec![row("same", Some(1)), row("same", Some(1))],
        };
        assert!(matches!(encode_payload(&duplicate), Err(CodecError::DuplicateIdentity { .. })));

        let good = DynamicModifierFrame {
            elapsed_wall_ms: 1,
            status_flags: 0,
            authority: DynamicModifierAuthority::exact_a729(),
            instances: vec![row("one", Some(1))],
        };
        let mut payload = encode_payload(&good).unwrap();
        payload[HEADER_LEN + 4..HEADER_LEN + 8].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(decode_payload(&payload), Err(CodecError::InvalidIdWindow { row: 0 }));

        let mut forged = good;
        forged.authority.static_authority_receipt_sha256[0] ^= 0xff;
        assert_eq!(encode_payload(&forged), Err(CodecError::AuthorityMismatch));
    }
}
