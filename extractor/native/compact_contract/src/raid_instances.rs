//! Domain 91 `raid_instances_live` pointer-free raid-system snapshot.
//!
//! The carrier preserves the exact `CRaidSystem` inline-parent order and each
//! `CCountryRaidStatus` child-vector order. It never canonicalises by country
//! or raid id: those are identities, not ordering authorities.

use std::collections::BTreeSet;

pub const DOMAIN_ID: u16 = 91;
pub const DOMAIN_NAME: &str = "raid_instances_live";
pub const PAYLOAD_MAGIC: u32 = u32::from_le_bytes(*b"RID1");
pub const PAYLOAD_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 40;
pub const PARENT_LEN: usize = 32;
pub const CHILD_LEN: usize = 80;
pub const MAX_PARENTS: usize = 4_096;
pub const MAX_CHILDREN: usize = 65_536;
pub const MAX_TAG_DB_ID: u32 = 4_096;
pub const RAID_REFERENCE_TYPE: u32 = 84;
pub const EXACT_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RaidParentClassification {
    MissingTagDummy = 0,
    Country = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RaidPhase {
    Assembling = 1,
    Preparing = 2,
    Prepared = 3,
    InProgress = 4,
    Ended = 5,
}

impl RaidPhase {
    pub fn from_raw(value: u32) -> Option<Self> {
        Some(match value {
            1 => Self::Assembling,
            2 => Self::Preparing,
            3 => Self::Prepared,
            4 => Self::InProgress,
            5 => Self::Ended,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Assembling => "ASSEMBLING",
            Self::Preparing => "PREPARING",
            Self::Prepared => "PREPARED",
            Self::InProgress => "IN_PROGRESS",
            Self::Ended => "ENDED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RaidOutcome {
    None = 0,
    Failure = 1,
    LimitedSuccess = 2,
    Success = 3,
    CriticalSuccess = 4,
    Canceled = 5,
}

impl RaidOutcome {
    pub fn from_raw(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::None,
            1 => Self::Failure,
            2 => Self::LimitedSuccess,
            3 => Self::Success,
            4 => Self::CriticalSuccess,
            5 => Self::Canceled,
            _ => return None,
        })
    }

    pub fn as_str(self) -> Option<&'static str> {
        Some(match self {
            Self::None => return None,
            Self::Failure => "FAILURE",
            Self::LimitedSuccess => "LIMITED_SUCCESS",
            Self::Success => "SUCCESS",
            Self::CriticalSuccess => "CRITICAL_SUCCESS",
            Self::Canceled => "CANCELED",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RaidParentRow {
    pub producer_ordinal: u32,
    pub country_tag_db_id: u32,
    pub priority: i32,
    pub classification: RaidParentClassification,
    pub first_child_ordinal: u32,
    pub child_count: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RaidInstanceRow {
    pub producer_ordinal: u32,
    pub parent_ordinal: u32,
    pub child_ordinal: u32,
    pub raid_id: u32,
    pub raid_id_type: u32,
    pub prep_time_hours: i32,
    pub phase: RaidPhase,
    pub outcome: RaidOutcome,
    /// Serializer-validity byte at CRaidInstance+0x1B0. The date object at
    /// +0x1A8 is polymorphic and is not reinterpreted as a scalar.
    pub end_date_present: bool,
    pub victim_country_tag_db_id: u32,
    /// Serializer presence byte at `raid_source+0x04`. A raw kind value is not
    /// semantic unless this marker is set.
    pub source_kind_present: bool,
    pub source_kind_raw: u32,
    pub source_province_id: u32,
    pub source_country_tag_db_id: u32,
    /// Exact two-dword `raid_source` location carrier. Semantics remain
    /// source-kind dependent and are not guessed by this transport.
    pub source_location_raw: [u32; 2],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedRaidInstancesPayload {
    pub frame_hour: u64,
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub observed_parents: u32,
    pub observed_children: u32,
    pub parents: Vec<RaidParentRow>,
    pub children: Vec<RaidInstanceRow>,
}

pub fn encode_payload(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
    parents: &[RaidParentRow],
    children: &[RaidInstanceRow],
    observed_parents: u32,
    observed_children: u32,
) -> Result<Vec<u8>, String> {
    validate_rows(
        frame_hour,
        status_flags,
        parents,
        children,
        observed_parents,
        observed_children,
    )?;
    let total = HEADER_LEN
        .checked_add(parents.len().checked_mul(PARENT_LEN).ok_or("RID1 parent bytes overflow")?)
        .and_then(|value| value.checked_add(children.len().checked_mul(CHILD_LEN)?))
        .ok_or("RID1 payload bytes overflow")?;
    let mut out = Vec::with_capacity(total);
    put_u32(&mut out, PAYLOAD_MAGIC);
    put_u16(&mut out, PAYLOAD_VERSION);
    put_u16(&mut out, HEADER_LEN as u16);
    put_u16(&mut out, PARENT_LEN as u16);
    put_u16(&mut out, CHILD_LEN as u16);
    put_u32(&mut out, status_flags);
    put_u64(&mut out, frame_hour);
    put_u64(&mut out, elapsed_wall_ms);
    put_u32(&mut out, observed_parents);
    put_u32(&mut out, observed_children);
    for row in parents {
        put_u32(&mut out, row.producer_ordinal);
        put_u32(&mut out, row.country_tag_db_id);
        put_i32(&mut out, row.priority);
        put_u32(&mut out, row.classification as u32);
        put_u32(&mut out, row.first_child_ordinal);
        put_u32(&mut out, row.child_count);
        put_u64(&mut out, 0);
    }
    for row in children {
        put_u32(&mut out, row.producer_ordinal);
        put_u32(&mut out, row.parent_ordinal);
        put_u32(&mut out, row.child_ordinal);
        put_u32(&mut out, row.raid_id);
        put_u32(&mut out, row.raid_id_type);
        put_i32(&mut out, row.prep_time_hours);
        put_u32(&mut out, row.phase as u32);
        put_u32(&mut out, row.outcome as u32);
        put_u32(&mut out, u32::from(row.end_date_present));
        put_u32(&mut out, row.victim_country_tag_db_id);
        put_u32(&mut out, row.source_kind_raw);
        put_u32(&mut out, row.source_province_id);
        put_u32(&mut out, row.source_country_tag_db_id);
        put_u32(&mut out, row.source_location_raw[0]);
        put_u32(&mut out, row.source_location_raw[1]);
        put_u32(&mut out, u32::from(row.source_kind_present));
        put_u64(&mut out, 0);
        put_u64(&mut out, 0);
    }
    debug_assert_eq!(out.len(), total);
    Ok(out)
}

/// Encode a typed, rowless current-hour observation failure. A failed owner
/// walk is never represented as a measured-empty raid system and never reuses
/// rows from an earlier hour.
pub fn encode_source_unavailable(
    frame_hour: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, String> {
    if status_flags == 0 {
        return Err("RID1 unavailable payload requires a nonzero status".into());
    }
    encode_payload(
        frame_hour,
        elapsed_wall_ms,
        status_flags,
        &[],
        &[],
        0,
        0,
    )
}

pub fn decode_payload(payload: &[u8]) -> Result<DecodedRaidInstancesPayload, String> {
    if payload.len() < HEADER_LEN {
        return Err("RID1 payload truncated before header".into());
    }
    if read_u32(payload, 0)? != PAYLOAD_MAGIC
        || read_u16(payload, 4)? != PAYLOAD_VERSION
        || read_u16(payload, 6)? as usize != HEADER_LEN
        || read_u16(payload, 8)? as usize != PARENT_LEN
        || read_u16(payload, 10)? as usize != CHILD_LEN
    {
        return Err("RID1 magic/schema/stride mismatch".into());
    }
    let status_flags = read_u32(payload, 12)?;
    let frame_hour = read_u64(payload, 16)?;
    let elapsed_wall_ms = read_u64(payload, 24)?;
    let parent_count = read_u32(payload, 32)? as usize;
    let child_count = read_u32(payload, 36)? as usize;
    if parent_count > MAX_PARENTS || child_count > MAX_CHILDREN {
        return Err("RID1 row count exceeds bound".into());
    }
    let expected = HEADER_LEN
        .checked_add(parent_count.checked_mul(PARENT_LEN).ok_or("RID1 parent bytes overflow")?)
        .and_then(|value| value.checked_add(child_count.checked_mul(CHILD_LEN)?))
        .ok_or("RID1 payload bytes overflow")?;
    if payload.len() != expected {
        return Err("RID1 payload does not exactly cover declared rows".into());
    }
    let mut parents = Vec::with_capacity(parent_count);
    for index in 0..parent_count {
        let at = HEADER_LEN + index * PARENT_LEN;
        let classification = match read_u32(payload, at + 12)? {
            0 => RaidParentClassification::MissingTagDummy,
            1 => RaidParentClassification::Country,
            _ => return Err(format!("RID1 parent {index} classification invalid")),
        };
        if read_u64(payload, at + 24)? != 0 {
            return Err(format!("RID1 parent {index} reserved tail nonzero"));
        }
        parents.push(RaidParentRow {
            producer_ordinal: read_u32(payload, at)?,
            country_tag_db_id: read_u32(payload, at + 4)?,
            priority: read_i32(payload, at + 8)?,
            classification,
            first_child_ordinal: read_u32(payload, at + 16)?,
            child_count: read_u32(payload, at + 20)?,
        });
    }
    let child_base = HEADER_LEN + parent_count * PARENT_LEN;
    let mut children = Vec::with_capacity(child_count);
    for index in 0..child_count {
        let at = child_base + index * CHILD_LEN;
        if read_u64(payload, at + 64)? != 0
            || read_u64(payload, at + 72)? != 0
        {
            return Err(format!("RID1 child {index} reserved tail nonzero"));
        }
        children.push(RaidInstanceRow {
            producer_ordinal: read_u32(payload, at)?,
            parent_ordinal: read_u32(payload, at + 4)?,
            child_ordinal: read_u32(payload, at + 8)?,
            raid_id: read_u32(payload, at + 12)?,
            raid_id_type: read_u32(payload, at + 16)?,
            prep_time_hours: read_i32(payload, at + 20)?,
            phase: RaidPhase::from_raw(read_u32(payload, at + 24)?)
                .ok_or_else(|| format!("RID1 child {index} phase invalid"))?,
            outcome: RaidOutcome::from_raw(read_u32(payload, at + 28)?)
                .ok_or_else(|| format!("RID1 child {index} outcome invalid"))?,
            end_date_present: match read_u32(payload, at + 32)? {
                0 => false,
                1 => true,
                _ => return Err(format!("RID1 child {index} end-date marker invalid")),
            },
            victim_country_tag_db_id: read_u32(payload, at + 36)?,
            source_kind_present: match read_u32(payload, at + 60)? {
                0 => false,
                1 => true,
                _ => return Err(format!("RID1 child {index} source-kind marker invalid")),
            },
            source_kind_raw: read_u32(payload, at + 40)?,
            source_province_id: read_u32(payload, at + 44)?,
            source_country_tag_db_id: read_u32(payload, at + 48)?,
            source_location_raw: [read_u32(payload, at + 52)?, read_u32(payload, at + 56)?],
        });
    }
    validate_rows(
        frame_hour,
        status_flags,
        &parents,
        &children,
        parent_count as u32,
        child_count as u32,
    )?;
    Ok(DecodedRaidInstancesPayload {
        frame_hour,
        elapsed_wall_ms,
        status_flags,
        observed_parents: parent_count as u32,
        observed_children: child_count as u32,
        parents,
        children,
    })
}

fn validate_rows(
    frame_hour: u64,
    status_flags: u32,
    parents: &[RaidParentRow],
    children: &[RaidInstanceRow],
    observed_parents: u32,
    observed_children: u32,
) -> Result<(), String> {
    if status_flags != 0 {
        if !parents.is_empty() || !children.is_empty() {
            return Err("RID1 unavailable payload carries rows".into());
        }
        return Ok(());
    }
    if frame_hour == 0
        || parents.len() > MAX_PARENTS
        || children.len() > MAX_CHILDREN
        || observed_parents as usize != parents.len()
        || observed_children as usize != children.len()
    {
        return Err("RID1 coverage/header is incomplete".into());
    }
    // A rowless successful frame is an exact measured-empty population, not
    // source-unavailable. The production reader may emit it only after the
    // CRaidSystem parent-vector header remains coherent across its complete
    // double-read. Keep one-sided populations impossible: without a complete
    // parent census there is no authority for any child row.
    if parents.is_empty() {
        if children.is_empty() {
            return Ok(());
        }
        return Err("RID1 measured-empty population carries orphan children".into());
    }
    let mut parent_tags = BTreeSet::new();
    let mut raid_ids = BTreeSet::new();
    let mut next_child = 0usize;
    let mut dummy_count = 0usize;
    for (parent_index, parent) in parents.iter().enumerate() {
        if parent.producer_ordinal as usize != parent_index
            || parent.first_child_ordinal as usize != next_child
            || parent.priority < 0
        {
            return Err(format!("RID1 parent {parent_index} order/priority invalid"));
        }
        match parent.classification {
            RaidParentClassification::MissingTagDummy => {
                dummy_count += 1;
                if parent.country_tag_db_id != 0 || parent_index != 0 {
                    return Err("RID1 missing-tag dummy is not unique ordinal-zero tag zero".into());
                }
            }
            RaidParentClassification::Country => {
                if parent.country_tag_db_id == 0
                    || parent.country_tag_db_id > MAX_TAG_DB_ID
                    || !parent_tags.insert(parent.country_tag_db_id)
                {
                    return Err(format!("RID1 parent {parent_index} country identity invalid"));
                }
            }
        }
        let count = parent.child_count as usize;
        let end = next_child.checked_add(count).ok_or("RID1 child span overflow")?;
        if end > children.len() {
            return Err(format!("RID1 parent {parent_index} child span out of range"));
        }
        for (child_index, child) in children[next_child..end].iter().enumerate() {
            if child.producer_ordinal as usize != next_child + child_index
                || child.parent_ordinal as usize != parent_index
                || child.child_ordinal as usize != child_index
            {
                return Err(format!("RID1 child {} producer hierarchy invalid", next_child + child_index));
            }
        }
        next_child = end;
    }
    if dummy_count != 1 || next_child != children.len() {
        return Err("RID1 parent/child population is not exact".into());
    }
    for (index, child) in children.iter().enumerate() {
        if child.raid_id == 0
            || child.raid_id_type != RAID_REFERENCE_TYPE
            || !raid_ids.insert((child.raid_id_type, child.raid_id))
            || child.prep_time_hours < 0
            || child.victim_country_tag_db_id > MAX_TAG_DB_ID
            || child.source_country_tag_db_id > MAX_TAG_DB_ID
            || child.source_kind_raw > 5
            || !child.source_kind_present
        {
            return Err(format!("RID1 child {index} identity/source scalar invalid"));
        }
        let ended = child.phase == RaidPhase::Ended;
        if ended != (child.outcome != RaidOutcome::None)
            || ended != child.end_date_present
        {
            return Err(format!("RID1 child {index} lifecycle tuple invalid"));
        }
    }
    Ok(())
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn read_u16(bytes: &[u8], at: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        bytes.get(at..at + 2).ok_or("RID1 u16 truncated")?.try_into().unwrap(),
    ))
}
fn read_u32(bytes: &[u8], at: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        bytes.get(at..at + 4).ok_or("RID1 u32 truncated")?.try_into().unwrap(),
    ))
}
fn read_i32(bytes: &[u8], at: usize) -> Result<i32, String> {
    Ok(i32::from_le_bytes(
        bytes.get(at..at + 4).ok_or("RID1 i32 truncated")?.try_into().unwrap(),
    ))
}
fn read_u64(bytes: &[u8], at: usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(
        bytes.get(at..at + 8).ok_or("RID1 u64 truncated")?.try_into().unwrap(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<RaidParentRow>, Vec<RaidInstanceRow>) {
        (
            vec![
                RaidParentRow {
                    producer_ordinal: 0,
                    country_tag_db_id: 0,
                    priority: 1,
                    classification: RaidParentClassification::MissingTagDummy,
                    first_child_ordinal: 0,
                    child_count: 0,
                },
                RaidParentRow {
                    producer_ordinal: 1,
                    country_tag_db_id: 1,
                    priority: 1,
                    classification: RaidParentClassification::Country,
                    first_child_ordinal: 0,
                    child_count: 2,
                },
            ],
            vec![
                RaidInstanceRow {
                    producer_ordinal: 0,
                    parent_ordinal: 1,
                    child_ordinal: 0,
                    raid_id: 509,
                    raid_id_type: 84,
                    prep_time_hours: 2160,
                    phase: RaidPhase::Ended,
                    outcome: RaidOutcome::Failure,
                    end_date_present: true,
                    victim_country_tag_db_id: 2,
                    source_kind_present: true,
                    source_kind_raw: 0,
                    source_province_id: 6570,
                    source_country_tag_db_id: 1,
                    source_location_raw: [0, 0],
                },
                RaidInstanceRow {
                    producer_ordinal: 1,
                    parent_ordinal: 1,
                    child_ordinal: 1,
                    raid_id: 513,
                    raid_id_type: 84,
                    prep_time_hours: 0,
                    phase: RaidPhase::Preparing,
                    outcome: RaidOutcome::None,
                    end_date_present: false,
                    victim_country_tag_db_id: 0,
                    source_kind_present: true,
                    source_kind_raw: 0,
                    source_province_id: 6570,
                    source_country_tag_db_id: 1,
                    source_location_raw: [0, 0],
                },
            ],
        )
    }

    #[test]
    fn roundtrip_preserves_parent_and_child_producer_order() {
        let (parents, children) = fixture();
        let payload = encode_payload(900, 7, 0, &parents, &children, 2, 2).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.parents, parents);
        assert_eq!(decoded.children, children);
    }

    #[test]
    fn dummy_identity_duplicates_and_lifecycle_contradictions_reject() {
        let (parents, mut children) = fixture();
        children[1].raid_id = children[0].raid_id;
        assert!(encode_payload(900, 0, 0, &parents, &children, 2, 2).is_err());
        children[1].raid_id = 513;
        children[1].outcome = RaidOutcome::Success;
        assert!(encode_payload(900, 0, 0, &parents, &children, 2, 2).is_err());
        let mut bad_parents = parents;
        bad_parents[0].classification = RaidParentClassification::Country;
        assert!(encode_payload(900, 0, 0, &bad_parents, &children, 2, 2).is_err());
    }

    #[test]
    fn source_unavailable_is_typed_rowless_and_cannot_claim_clean() {
        assert!(encode_source_unavailable(900, 7, 0).is_err());
        let decoded = decode_payload(&encode_source_unavailable(900, 7, 1).unwrap()).unwrap();
        assert_eq!(decoded.frame_hour, 900);
        assert_eq!(decoded.status_flags, 1);
        assert!(decoded.parents.is_empty());
        assert!(decoded.children.is_empty());
    }

    #[test]
    fn authenticated_measured_empty_roundtrips_but_one_sided_population_rejects() {
        let payload = encode_payload(900, 7, 0, &[], &[], 0, 0).unwrap();
        let decoded = decode_payload(&payload).unwrap();
        assert_eq!(decoded.frame_hour, 900);
        assert_eq!(decoded.status_flags, 0);
        assert_eq!(decoded.observed_parents, 0);
        assert_eq!(decoded.observed_children, 0);
        assert!(decoded.parents.is_empty());
        assert!(decoded.children.is_empty());

        let (_, children) = fixture();
        assert!(encode_payload(900, 7, 0, &[], &children[..1], 0, 1).is_err());
        assert!(encode_payload(900, 7, 0, &[], &[], 1, 0).is_err());
        assert!(encode_payload(900, 7, 0, &[], &[], 0, 1).is_err());
    }
}
