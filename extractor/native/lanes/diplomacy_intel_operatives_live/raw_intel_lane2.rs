use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawIntelStatus {
    Measured,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawIntelSlotStatus {
    Accepted,
    Vacant,
    Unreadable,
    NonHeapPointer,
    VtableUnreadable,
    VtableMismatch,
    TagUnreadable,
    TagInvalid,
    AmbiguousDuplicateTag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RawIntelCause {
    CountriesEmpty,
    DenominatorsRootUnavailable,
    DenominatorsSpanUnreadable,
    CivilianDenominatorInvalid,
    ArmyDenominatorInvalid,
    NavyDenominatorInvalid,
    AirDenominatorInvalid,
    NoFogFlagUnreadable,
    FamilyOff,
    PairBudgetExceeded,
    NoFogSkip,
    ViewerIntelTableUnavailable,
    IntelTableDataUnavailable,
    IntelTableCountInvalid,
    IntelTableSpanUnreadable,
    CopyBudgetExceeded,
}

impl RawIntelCause {
    fn is_global(self) -> bool {
        matches!(
            self,
            Self::CountriesEmpty
                | Self::DenominatorsRootUnavailable
                | Self::DenominatorsSpanUnreadable
                | Self::CivilianDenominatorInvalid
                | Self::ArmyDenominatorInvalid
                | Self::NavyDenominatorInvalid
                | Self::AirDenominatorInvalid
                | Self::NoFogFlagUnreadable
                | Self::FamilyOff
                | Self::PairBudgetExceeded
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawIntelSlot {
    pub physical_slot: u32,
    pub status: RawIntelSlotStatus,
    #[serde(deserialize_with = "required_nullable")]
    pub tag_db_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawIntelCensus {
    pub country_count: u32,
    pub header_stable: bool,
    pub allocation_complete: bool,
    pub slots: Vec<RawIntelSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawIntelOwner {
    pub owner_physical_slot: u32,
    pub owner_tag_db_id: u32,
    #[serde(deserialize_with = "required_nullable")]
    pub table_row_count: Option<u32>,
    #[serde(deserialize_with = "required_nullable")]
    pub copied_prefix_slots: Option<u32>,
    pub status: RawIntelStatus,
    #[serde(deserialize_with = "required_nullable")]
    pub cause: Option<RawIntelCause>,
    pub lane2_raw: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawIntelLane2 {
    pub schema: u32,
    pub census: RawIntelCensus,
    pub status: RawIntelStatus,
    #[serde(deserialize_with = "required_nullable")]
    pub cause: Option<RawIntelCause>,
    pub owners: Vec<RawIntelOwner>,
}

impl RawIntelLane2 {
    pub fn validate(&self) -> Result<(), String> {
        use compact_contract::build_profile as profile;
        if self.schema != 1
            || self.census.country_count as usize != self.census.slots.len()
            || self.census.slots.len() > profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS as usize
        {
            return Err("D53 raw lane2 census shape".into());
        }
        let mut tags = BTreeSet::new();
        let mut accepted = Vec::new();
        for (ordinal, slot) in self.census.slots.iter().enumerate() {
            if slot.physical_slot as usize != ordinal {
                return Err("D53 raw lane2 physical slot order".into());
            }
            let needs_tag = matches!(
                slot.status,
                RawIntelSlotStatus::Accepted | RawIntelSlotStatus::AmbiguousDuplicateTag
            );
            if needs_tag != slot.tag_db_id.is_some()
                || slot
                    .tag_db_id
                    .is_some_and(|tag| tag == 0 || tag > super::MAX_TAG_DB_ID)
            {
                return Err("D53 raw lane2 slot identity".into());
            }
            if slot.status == RawIntelSlotStatus::Accepted {
                let tag = slot.tag_db_id.unwrap();
                if !tags.insert(tag) {
                    return Err("D53 raw lane2 duplicate accepted identity".into());
                }
                accepted.push((slot.physical_slot, tag));
            }
        }
        if self.census.slots.iter().any(|slot| {
            slot.status == RawIntelSlotStatus::AmbiguousDuplicateTag
                && slot.tag_db_id.is_some_and(|tag| tags.contains(&tag))
        }) {
            return Err("D53 raw lane2 ambiguous accepted identity".into());
        }
        match self.status {
            RawIntelStatus::Unavailable => {
                if !self.cause.is_some_and(RawIntelCause::is_global) || !self.owners.is_empty() {
                    return Err("D53 raw lane2 global refusal".into());
                }
                return Ok(());
            }
            RawIntelStatus::Measured => {
                if self.cause.is_some() || self.owners.len() != accepted.len() {
                    return Err("D53 raw lane2 owner census".into());
                }
            }
        }
        let mut raw_count = 0usize;
        for (owner, identity) in self.owners.iter().zip(accepted) {
            if (owner.owner_physical_slot, owner.owner_tag_db_id) != identity {
                return Err("D53 raw lane2 owner identity".into());
            }
            if owner
                .table_row_count
                .is_some_and(|count| count == 0 || count > profile::DOMAIN53_MAX_VECTOR_ROWS)
            {
                return Err("D53 raw lane2 table count".into());
            }
            match owner.status {
                RawIntelStatus::Measured => {
                    let Some((count, prefix)) =
                        owner.table_row_count.zip(owner.copied_prefix_slots)
                    else {
                        return Err("D53 raw lane2 measured coverage absent".into());
                    };
                    if owner.cause.is_some()
                        || prefix > count
                        || prefix as usize != owner.lane2_raw.len()
                    {
                        return Err("D53 raw lane2 measured coverage".into());
                    }
                }
                RawIntelStatus::Unavailable => {
                    let count_matches_cause = match owner.cause {
                        Some(RawIntelCause::NoFogSkip) => true,
                        Some(
                            RawIntelCause::IntelTableSpanUnreadable
                            | RawIntelCause::CopyBudgetExceeded,
                        ) => owner.table_row_count.is_some(),
                        Some(
                            RawIntelCause::ViewerIntelTableUnavailable
                            | RawIntelCause::IntelTableDataUnavailable
                            | RawIntelCause::IntelTableCountInvalid,
                        ) => owner.table_row_count.is_none(),
                        _ => false,
                    };
                    if !count_matches_cause
                        || !owner.cause.is_some_and(|cause| !cause.is_global())
                        || owner.copied_prefix_slots.is_some()
                        || !owner.lane2_raw.is_empty()
                    {
                        return Err("D53 raw lane2 owner refusal".into());
                    }
                }
            }
            raw_count = raw_count
                .checked_add(owner.lane2_raw.len())
                .ok_or("D53 raw lane2 count overflow")?;
            if raw_count
                > compact_contract::COMPACT_MAX_PAYLOAD_LEN as usize / super::ROW_INTEL_MAP_LEN
            {
                return Err("D53 raw lane2 copy bound".into());
            }
        }
        Ok(())
    }
}

fn required_nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

pub const MAGIC: &[u8; 4] = b"D53L";
const HEADER_LEN: usize = 16;
const SLOT_STATUSES: [RawIntelSlotStatus; 9] = [
    RawIntelSlotStatus::Accepted,
    RawIntelSlotStatus::Vacant,
    RawIntelSlotStatus::Unreadable,
    RawIntelSlotStatus::NonHeapPointer,
    RawIntelSlotStatus::VtableUnreadable,
    RawIntelSlotStatus::VtableMismatch,
    RawIntelSlotStatus::TagUnreadable,
    RawIntelSlotStatus::TagInvalid,
    RawIntelSlotStatus::AmbiguousDuplicateTag,
];
const CAUSES: [RawIntelCause; 16] = [
    RawIntelCause::CountriesEmpty,
    RawIntelCause::DenominatorsRootUnavailable,
    RawIntelCause::DenominatorsSpanUnreadable,
    RawIntelCause::CivilianDenominatorInvalid,
    RawIntelCause::ArmyDenominatorInvalid,
    RawIntelCause::NavyDenominatorInvalid,
    RawIntelCause::AirDenominatorInvalid,
    RawIntelCause::NoFogFlagUnreadable,
    RawIntelCause::FamilyOff,
    RawIntelCause::PairBudgetExceeded,
    RawIntelCause::NoFogSkip,
    RawIntelCause::ViewerIntelTableUnavailable,
    RawIntelCause::IntelTableDataUnavailable,
    RawIntelCause::IntelTableCountInvalid,
    RawIntelCause::IntelTableSpanUnreadable,
    RawIntelCause::CopyBudgetExceeded,
];

pub fn encoded_len(value: Option<&RawIntelLane2>) -> usize {
    HEADER_LEN
        + 4
        + value.map_or(0, |raw| {
            24usize
                .saturating_add(raw.census.slots.len().saturating_mul(16))
                .saturating_add(raw.owners.iter().fold(0usize, |sum, owner| {
                    sum.saturating_add(36)
                        .saturating_add(owner.lane2_raw.len().saturating_mul(8))
                }))
        })
}

pub fn append(out: &mut Vec<u8>, value: Option<&RawIntelLane2>) {
    fn word(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }
    fn nullable(out: &mut Vec<u8>, value: Option<u32>) {
        word(out, u32::from(value.is_some()));
        word(out, value.unwrap_or(0));
    }
    fn status(out: &mut Vec<u8>, value: RawIntelStatus, cause: Option<RawIntelCause>) {
        word(
            out,
            match value {
                RawIntelStatus::Measured => 0,
                RawIntelStatus::Unavailable => 1,
            },
        );
        word(
            out,
            cause.map_or(0, |cause| {
                CAUSES.iter().position(|c| *c == cause).unwrap() as u32 + 1
            }),
        );
    }
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    word(out, (encoded_len(value) - HEADER_LEN) as u32);
    word(out, 0);
    word(out, u32::from(value.is_some()));
    let Some(raw) = value else {
        return;
    };
    word(out, raw.schema);
    word(out, raw.census.country_count);
    word(
        out,
        u32::from(raw.census.header_stable) | (u32::from(raw.census.allocation_complete) << 1),
    );
    for slot in &raw.census.slots {
        word(out, slot.physical_slot);
        word(
            out,
            SLOT_STATUSES
                .iter()
                .position(|s| *s == slot.status)
                .unwrap() as u32,
        );
        nullable(out, slot.tag_db_id);
    }
    status(out, raw.status, raw.cause);
    word(out, raw.owners.len() as u32);
    for owner in &raw.owners {
        word(out, owner.owner_physical_slot);
        word(out, owner.owner_tag_db_id);
        nullable(out, owner.table_row_count);
        nullable(out, owner.copied_prefix_slots);
        status(out, owner.status, owner.cause);
        word(out, owner.lane2_raw.len() as u32);
        for value in &owner.lane2_raw {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
}

pub fn decode(input: &[u8]) -> Result<(Option<RawIntelLane2>, usize), String> {
    struct Cursor<'a>(&'a [u8]);
    impl Cursor<'_> {
        fn bytes(&mut self, len: usize) -> Result<&[u8], String> {
            if len > self.0.len() {
                return Err("D53L truncated body".into());
            }
            let (value, rest) = self.0.split_at(len);
            self.0 = rest;
            Ok(value)
        }
        fn word(&mut self) -> Result<u32, String> {
            Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
        }
        fn nullable(&mut self) -> Result<Option<u32>, String> {
            let presence = self.word()?;
            let value = self.word()?;
            match (presence, value) {
                (0, 0) => Ok(None),
                (1, value) => Ok(Some(value)),
                _ => Err("D53L nullable encoding".into()),
            }
        }
        fn status(&mut self) -> Result<(RawIntelStatus, Option<RawIntelCause>), String> {
            let status = match self.word()? {
                0 => RawIntelStatus::Measured,
                1 => RawIntelStatus::Unavailable,
                _ => return Err("D53L unknown status".into()),
            };
            let code = self.word()?;
            let cause = if code == 0 {
                None
            } else {
                Some(*CAUSES.get(code as usize - 1).ok_or("D53L unknown cause")?)
            };
            Ok((status, cause))
        }
        fn count(&mut self, limit: usize, width: usize) -> Result<usize, String> {
            let n = self.word()? as usize;
            if n > limit || n > self.0.len() / width {
                return Err("D53L bounded count/truncated rows".into());
            }
            Ok(n)
        }
    }
    if input.len() < HEADER_LEN
        || input.get(..4) != Some(MAGIC)
        || input[4..8] != [1, 0, 16, 0]
        || input[12..16] != [0; 4]
    {
        return Err("D53L header/version/reserved".into());
    }
    let size = u32::from_le_bytes(input[8..12].try_into().unwrap()) as usize;
    if size > compact_contract::COMPACT_MAX_PAYLOAD_LEN as usize || size > input.len() - HEADER_LEN
    {
        return Err("D53L truncated/oversized trailer".into());
    }
    let mut cursor = Cursor(&input[HEADER_LEN..HEADER_LEN + size]);
    let raw = match cursor.word()? {
        0 => None,
        1 => {
            let schema = cursor.word()?;
            let country_count = cursor.count(
                compact_contract::build_profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS as usize,
                16,
            )?;
            let flags = cursor.word()?;
            if flags & !3 != 0 {
                return Err("D53L census flags".into());
            }
            let mut slots = Vec::with_capacity(country_count);
            for _ in 0..country_count {
                let physical_slot = cursor.word()?;
                let status = *SLOT_STATUSES
                    .get(cursor.word()? as usize)
                    .ok_or("D53L slot status")?;
                slots.push(RawIntelSlot {
                    physical_slot,
                    status,
                    tag_db_id: cursor.nullable()?,
                });
            }
            let (status, cause) = cursor.status()?;
            let n = cursor.count(country_count, 36)?;
            let mut owners = Vec::with_capacity(n);
            for _ in 0..n {
                let owner_physical_slot = cursor.word()?;
                let owner_tag_db_id = cursor.word()?;
                let table_row_count = cursor.nullable()?;
                let copied_prefix_slots = cursor.nullable()?;
                let (status, cause) = cursor.status()?;
                let n = cursor.count(
                    compact_contract::build_profile::DOMAIN53_MAX_VECTOR_ROWS as usize,
                    8,
                )?;
                let mut lane2_raw = Vec::with_capacity(n);
                for _ in 0..n {
                    lane2_raw.push(i64::from_le_bytes(cursor.bytes(8)?.try_into().unwrap()));
                }
                owners.push(RawIntelOwner {
                    owner_physical_slot,
                    owner_tag_db_id,
                    table_row_count,
                    copied_prefix_slots,
                    status,
                    cause,
                    lane2_raw,
                });
            }
            let raw = RawIntelLane2 {
                schema,
                census: RawIntelCensus {
                    country_count: country_count as u32,
                    header_stable: flags & 1 != 0,
                    allocation_complete: flags & 2 != 0,
                    slots,
                },
                status,
                cause,
                owners,
            };
            raw.validate()?;
            Some(raw)
        }
        _ => return Err("D53L presence".into()),
    };
    if !cursor.0.is_empty() {
        return Err("D53L trailing body bytes".into());
    }
    Ok((raw, HEADER_LEN + size))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw() -> RawIntelLane2 {
        RawIntelLane2 {
            schema: 1,
            census: RawIntelCensus {
                country_count: 3,
                header_stable: false,
                allocation_complete: false,
                slots: vec![
                    RawIntelSlot {
                        physical_slot: 0,
                        status: RawIntelSlotStatus::Accepted,
                        tag_db_id: Some(9),
                    },
                    RawIntelSlot {
                        physical_slot: 1,
                        status: RawIntelSlotStatus::Vacant,
                        tag_db_id: None,
                    },
                    RawIntelSlot {
                        physical_slot: 2,
                        status: RawIntelSlotStatus::Accepted,
                        tag_db_id: Some(3),
                    },
                ],
            },
            status: RawIntelStatus::Measured,
            cause: None,
            owners: vec![
                RawIntelOwner {
                    owner_physical_slot: 0,
                    owner_tag_db_id: 9,
                    table_row_count: Some(4),
                    copied_prefix_slots: Some(3),
                    status: RawIntelStatus::Measured,
                    cause: None,
                    lane2_raw: vec![i64::MIN, -1, i64::MAX],
                },
                RawIntelOwner {
                    owner_physical_slot: 2,
                    owner_tag_db_id: 3,
                    table_row_count: None,
                    copied_prefix_slots: None,
                    status: RawIntelStatus::Unavailable,
                    cause: Some(RawIntelCause::ViewerIntelTableUnavailable),
                    lane2_raw: vec![],
                },
            ],
        }
    }
    #[test]
    fn mandatory_nullable_keys_and_ambiguous_identity_are_strict() {
        let value = serde_json::to_value(raw()).unwrap();
        let parsed: RawIntelLane2 = serde_json::from_value(value.clone()).unwrap();
        parsed.validate().unwrap();
        for path in [
            "/cause",
            "/census/slots/1/tag_db_id",
            "/owners/0/cause",
            "/owners/1/table_row_count",
            "/owners/1/copied_prefix_slots",
        ] {
            let mut missing = value.clone();
            let (parent, key) = path.rsplit_once('/').unwrap();
            missing
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(
                serde_json::from_value::<RawIntelLane2>(missing).is_err(),
                "{path}"
            );
        }
        let mut bad = raw();
        bad.census.slots[1].status = RawIntelSlotStatus::AmbiguousDuplicateTag;
        bad.census.slots[1].tag_db_id = Some(9);
        assert!(bad.validate().unwrap_err().contains("ambiguous"));
    }
    #[test]
    fn binary_roundtrip_preserves_holes_prefix_sign_and_unknown_counts() {
        for value in [None, Some(raw())] {
            let mut bytes = vec![];
            append(&mut bytes, value.as_ref());
            assert_eq!(bytes.len(), encoded_len(value.as_ref()));
            assert_eq!(decode(&bytes).unwrap(), (value, bytes.len()));
            for end in 0..bytes.len() {
                assert!(decode(&bytes[..end]).is_err(), "truncation {end}");
            }
        }
    }
    #[test]
    fn binary_refuses_unknown_header_status_cause_and_contradictions() {
        let valid = raw();
        let mut bytes = vec![];
        append(&mut bytes, Some(&valid));
        // Body: presence/schema/count/flags, three 16-byte slots, then status/cause.
        for (offset, value) in [(4, 2u32), (12, 1), (28, 4), (36, 99), (80, 2), (84, 99)] {
            let mut bad = bytes.clone();
            bad[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(decode(&bad).is_err(), "offset {offset}");
        }
        for mutate in 0..7 {
            let mut bad = raw();
            match mutate {
                0 => bad.cause = Some(RawIntelCause::NoFogSkip),
                1 => bad.owners[0].copied_prefix_slots = None,
                2 => bad.owners[0].copied_prefix_slots = Some(4),
                3 => bad.owners[1].table_row_count = Some(0),
                4 => bad.owners[1].copied_prefix_slots = Some(0),
                5 => {
                    bad.census.slots.remove(1);
                }
                _ => {
                    bad.owners[1].cause = Some(RawIntelCause::CopyBudgetExceeded);
                }
            };
            let mut encoded = vec![];
            append(&mut encoded, Some(&bad));
            assert!(decode(&encoded).is_err(), "contradiction {mutate}");
        }
    }
    #[test]
    fn owning_d53_payload_requires_one_valid_trailer_and_schema_four() {
        let snapshot = super::super::DiplomacyIntelSnapshot {
            status_flags: super::super::STATUS_INTEL_MAP_UNPROVEN,
            intel_raw_lane2: Some(raw()),
            ..Default::default()
        };
        let bytes = super::super::encode_payload(&snapshot);
        let summary = super::super::decode_payload(0, &bytes);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let decoded = super::super::decode_snapshot_payload(0, &bytes).unwrap();
        assert_eq!(decoded.intel_raw_lane2, snapshot.intel_raw_lane2);
        assert_eq!(&bytes[20..24], &4u32.to_le_bytes());
        let start = bytes.windows(4).position(|w| w == MAGIC).unwrap();
        let mut absent = bytes.clone();
        absent.drain(start..start + encoded_len(snapshot.intel_raw_lane2.as_ref()));
        assert!(super::super::decode_payload(0, &absent)
            .parse_errors
            .iter()
            .any(|e| e.contains("missing D53L")));
        let mut duplicate = bytes.clone();
        append(&mut duplicate, None);
        assert!(super::super::decode_payload(0, &duplicate)
            .parse_errors
            .iter()
            .any(|e| e.contains("duplicate D53L")));
        for end in start..bytes.len() {
            assert!(!super::super::decode_payload(0, &bytes[..end])
                .parse_errors
                .is_empty());
        }
        let mut old = bytes;
        old[20..24].copy_from_slice(&3u32.to_le_bytes());
        assert!(!super::super::decode_payload(0, &old)
            .parse_errors
            .is_empty());
    }
}
