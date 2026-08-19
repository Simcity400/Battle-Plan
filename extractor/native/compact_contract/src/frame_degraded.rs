//! simultaneously clean is not a design margin, it is a deadlock -- and the
//! hour*. Labelling a defect as expected would be worse than the deadlock this
//! record solves, because the deadlock is loud and a mislabel is silent.

use crate::diagnostic_channel::{Authority, Predicate, AUTHORITY_LEN, MAX_PREDICATE_NAME_LEN};
use crate::{
    compact_status_failure_flags_after_expected_partial_tolerance, expected_partial_tolerance,
    expected_partial_tolerance_applies, CompactDomainId,
};

pub const FRAME_DEGRADED_MAGIC: &[u8; 4] = b"H4FD";
pub const FRAME_DEGRADED_VERSION: u16 = 2;
pub const FRAME_DEGRADED_HEADER_LEN: usize = 16;
pub const FRAME_DEGRADED_ENTRY_FIXED_LEN: usize = 32;
pub const FRAME_DEGRADED_MAX_PREDICATE_NAME_LEN: usize = MAX_PREDICATE_NAME_LEN;
pub const FRAME_DEGRADED_MAX_REASON_LEN: usize = 512;
pub const FRAME_DEGRADED_MAX_DOMAINS: usize = 1024;

pub const FRAME_DEGRADED_REASON_UNDIAGNOSED: &str = "undiagnosed_failure";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactDomainCompleteness {
    Complete,
    ExpectedPartial,
    Degraded { non_tolerated_flags: u32 },
}

impl CompactDomainCompleteness {
    pub const fn is_degraded(self) -> bool {
        matches!(self, Self::Degraded { .. })
    }
}

pub fn compact_domain_completeness(
    domain_id: CompactDomainId,
    status_flags: u32,
) -> CompactDomainCompleteness {
    let non_tolerated =
        compact_status_failure_flags_after_expected_partial_tolerance(domain_id, status_flags);
    if non_tolerated != 0 {
        return CompactDomainCompleteness::Degraded {
            non_tolerated_flags: non_tolerated,
        };
    }
    if expected_partial_tolerance_applies(domain_id, status_flags).is_some() {
        return CompactDomainCompleteness::ExpectedPartial;
    }
    CompactDomainCompleteness::Complete
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDegradedDiagnosis {
    pub predicate: Predicate,
    pub authority: Authority,
    pub reason: String,
}

impl FrameDegradedDiagnosis {
    pub fn new(predicate: Predicate, authority: Authority, reason: impl Into<String>) -> Self {
        Self {
            predicate,
            authority,
            reason: reason.into(),
        }
    }

    pub fn undeclared(authority: Authority, reason: impl Into<String>) -> Self {
        Self::new(Predicate::undeclared(), authority, reason)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDegradedDomain {
    pub domain_id: u16,
    pub status_flags: u32,
    pub non_tolerated_flags: u32,
    pub predicate: Predicate,
    pub authority: Authority,
    pub reason: String,
}

impl FrameDegradedDomain {
    pub fn from_status(
        domain_id: CompactDomainId,
        status_flags: u32,
        diagnosis: FrameDegradedDiagnosis,
    ) -> Option<Self> {
        match compact_domain_completeness(domain_id, status_flags) {
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags,
            } => Some(Self {
                domain_id,
                status_flags,
                non_tolerated_flags,
                predicate: diagnosis.predicate,
                authority: diagnosis.authority,
                reason: diagnosis.reason,
            }),
            CompactDomainCompleteness::Complete | CompactDomainCompleteness::ExpectedPartial => {
                None
            }
        }
    }

    pub fn render(&self) -> String {
        format!(
            "d{} {}#{} {} reason={}",
            self.domain_id,
            self.predicate.name,
            self.predicate.code.get(),
            self.authority.render(),
            self.reason
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDegradedRecordV2 {
    pub degraded_hour: i32,
    pub domains: Vec<FrameDegradedDomain>,
}

#[allow(clippy::too_many_arguments)]
fn check_entry(
    index: usize,
    domain_id: u16,
    reserved: u16,
    status_flags: u32,
    non_tolerated_flags: u32,
    predicate_code: u32,
    predicate_name: &str,
    authority_bytes: &[u8],
    reason_len: usize,
) -> Result<(), String> {
    if reserved != 0 {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) sets its reserved field to \
             {reserved:#06x}, which this format does not define"
        ));
    }
    if domain_id == crate::record_semantics::COMPACT_DOMAIN_FRAME {
        return Err(format!(
            "frame degraded entry {index} names the frame envelope domain \
             {domain_id}, which is not a per-domain degradation"
        ));
    }
    if crate::assigned_descriptor_by_id(domain_id).is_none() {
        return Err(format!(
            "frame degraded entry {index} names domain {domain_id}, which this contract \
             does not assign"
        ));
    }
    let unassigned = crate::compact_status_unassigned_flags(domain_id, status_flags);
    if unassigned != 0 {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) status {status_flags:#010x} sets \
             unassigned bit(s) {:?}",
            crate::compact_status_unassigned_bit_indices(unassigned)
        ));
    }
    if status_flags & crate::COMPACT_STATUS_BASELINE_CLEAR != 0 {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) requests BASELINE_CLEAR on a \
             failure-class record: a read that did not complete cannot describe the whole \
             extent it asks the consumer to replace"
        ));
    }
    if predicate_code == 0 {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) carries predicate code 0, which \
             is reserved for 'not declared'"
        ));
    }
    if predicate_name.is_empty() {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) gives its predicate no name"
        ));
    }
    if predicate_name.len() > FRAME_DEGRADED_MAX_PREDICATE_NAME_LEN {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) predicate name is {} bytes, above \
             the {FRAME_DEGRADED_MAX_PREDICATE_NAME_LEN} bound",
            predicate_name.len()
        ));
    }
    if (predicate_code == crate::diagnostic_channel::PREDICATE_UNDECLARED)
        != (predicate_name == crate::diagnostic_channel::PREDICATE_UNDECLARED_NAME)
    {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) pairs predicate code \
             {predicate_code:#010x} with name {predicate_name:?}; the reserved code and \
             {:?} are only valid together",
            crate::diagnostic_channel::PREDICATE_UNDECLARED_NAME
        ));
    }
    let authority = Authority::decode(authority_bytes)?;
    if authority == Authority::Undeclared {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) declares no authority, which has \
             no wire encoding"
        ));
    }
    if authority.encode() != authority_bytes[..AUTHORITY_LEN] {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) authority bytes are not canonical \
             for {}",
            authority.render()
        ));
    }
    if let Authority::OtherDomain(other) = authority {
        if crate::assigned_descriptor_by_id(other).is_none() {
            return Err(format!(
                "frame degraded entry {index} (domain {domain_id}) names domain {other} as its \
                 authority, which this contract does not assign"
            ));
        }
        if other == crate::record_semantics::COMPACT_DOMAIN_FRAME {
            return Err(format!(
                "frame degraded entry {index} (domain {domain_id}) names the frame envelope as a \
                 peer domain authority; a frame-owned fault refuses the hour"
            ));
        }
    }
    if non_tolerated_flags == 0 {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) claims no non-tolerated bits, so it \
             is not degraded"
        ));
    }
    let expected =
        compact_status_failure_flags_after_expected_partial_tolerance(domain_id, status_flags);
    if expected != non_tolerated_flags {
        let tolerance_note = match expected_partial_tolerance(domain_id) {
            Some(tolerance) if expected == 0 => format!(
                "; domain {domain_id} status {status_flags:#010x} is covered by its expected-partial \
                 tolerance ({}) and a tolerated partial is not a degradation",
                tolerance.reason
            ),
            _ => String::new(),
        };
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) claims non-tolerated bits \
             {non_tolerated_flags:#010x} but status {status_flags:#010x} classifies as \
             {expected:#010x}{tolerance_note}"
        ));
    }
    if reason_len == 0 {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) gives no reason"
        ));
    }
    if reason_len > FRAME_DEGRADED_MAX_REASON_LEN {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) reason is {reason_len} bytes, above \
             the {FRAME_DEGRADED_MAX_REASON_LEN} bound"
        ));
    }
    Ok(())
}

impl FrameDegradedRecordV2 {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.domains.is_empty() {
            return Err("frame degraded notice names no domain".to_string());
        }
        if self.domains.len() > FRAME_DEGRADED_MAX_DOMAINS {
            return Err(format!(
                "frame degraded notice names {} domains, above the {FRAME_DEGRADED_MAX_DOMAINS} bound",
                self.domains.len()
            ));
        }
        let mut out = Vec::with_capacity(FRAME_DEGRADED_HEADER_LEN + self.domains.len() * 64);
        out.extend_from_slice(FRAME_DEGRADED_MAGIC);
        out.extend_from_slice(&FRAME_DEGRADED_VERSION.to_le_bytes());
        out.extend_from_slice(&(FRAME_DEGRADED_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&self.degraded_hour.to_le_bytes());
        out.extend_from_slice(&(self.domains.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        let mut previous_domain_id: Option<u16> = None;
        for (index, domain) in self.domains.iter().enumerate() {
            if let Some(previous) = previous_domain_id {
                if domain.domain_id <= previous {
                    return Err(format!(
                        "frame degraded entry {index} domain {} does not follow {previous} in \
                         strictly increasing order",
                        domain.domain_id
                    ));
                }
            }
            previous_domain_id = Some(domain.domain_id);
            let reason = domain.reason.as_bytes();
            let name = domain.predicate.name.as_bytes();
            let authority = domain.authority.encode();
            check_entry(
                index,
                domain.domain_id,
                0,
                domain.status_flags,
                domain.non_tolerated_flags,
                domain.predicate.code.get(),
                &domain.predicate.name,
                &authority,
                reason.len(),
            )?;
            out.extend_from_slice(&domain.domain_id.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&domain.status_flags.to_le_bytes());
            out.extend_from_slice(&domain.non_tolerated_flags.to_le_bytes());
            out.extend_from_slice(&domain.predicate.code.get().to_le_bytes());
            out.extend_from_slice(&authority);
            out.extend_from_slice(&(reason.len() as u16).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(reason);
            out.extend_from_slice(name);
        }
        Ok(out)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        validate_frame_degraded_payload(payload)?;
        let degraded_hour = i32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
        let domain_count = u16::from_le_bytes([payload[12], payload[13]]) as usize;
        let mut domains = Vec::with_capacity(domain_count);
        let mut offset = FRAME_DEGRADED_HEADER_LEN;
        for _ in 0..domain_count {
            let domain_id = u16::from_le_bytes([payload[offset], payload[offset + 1]]);
            let status_flags = u32::from_le_bytes([
                payload[offset + 4],
                payload[offset + 5],
                payload[offset + 6],
                payload[offset + 7],
            ]);
            let non_tolerated_flags = u32::from_le_bytes([
                payload[offset + 8],
                payload[offset + 9],
                payload[offset + 10],
                payload[offset + 11],
            ]);
            let predicate_code = u32::from_le_bytes([
                payload[offset + 12],
                payload[offset + 13],
                payload[offset + 14],
                payload[offset + 15],
            ]);
            let authority = Authority::decode(&payload[offset + 16..offset + 16 + AUTHORITY_LEN])?;
            let reason_len =
                u16::from_le_bytes([payload[offset + 28], payload[offset + 29]]) as usize;
            let name_len =
                u16::from_le_bytes([payload[offset + 30], payload[offset + 31]]) as usize;
            let start = offset + FRAME_DEGRADED_ENTRY_FIXED_LEN;
            let reason_end = start + reason_len;
            let end = reason_end + name_len;
            let reason = core::str::from_utf8(&payload[start..reason_end])
                .map_err(|error| format!("frame degraded reason is not UTF-8: {error}"))?
                .to_string();
            let name = core::str::from_utf8(&payload[reason_end..end])
                .map_err(|error| format!("frame degraded predicate name is not UTF-8: {error}"))?;
            domains.push(FrameDegradedDomain {
                domain_id,
                status_flags,
                non_tolerated_flags,
                predicate: Predicate::new(predicate_code, name)?,
                authority,
                reason,
            });
            offset = end;
        }
        Ok(Self {
            degraded_hour,
            domains,
        })
    }

    pub fn declared_domain_ids(&self) -> Vec<u16> {
        self.domains.iter().map(|domain| domain.domain_id).collect()
    }
}

pub fn validate_frame_degraded_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() < FRAME_DEGRADED_HEADER_LEN {
        return Err(format!(
            "frame degraded payload {} byte(s), below the {FRAME_DEGRADED_HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    if &payload[0..4] != FRAME_DEGRADED_MAGIC {
        return Err("frame degraded magic is not H4FD".to_string());
    }
    let version = u16::from_le_bytes([payload[4], payload[5]]);
    if version != FRAME_DEGRADED_VERSION {
        return Err(format!(
            "frame degraded version {version} is not {FRAME_DEGRADED_VERSION}"
        ));
    }
    let header_len = u16::from_le_bytes([payload[6], payload[7]]) as usize;
    if header_len != FRAME_DEGRADED_HEADER_LEN {
        return Err(format!(
            "frame degraded header length {header_len} is not {FRAME_DEGRADED_HEADER_LEN}"
        ));
    }
    let domain_count = u16::from_le_bytes([payload[12], payload[13]]) as usize;
    if domain_count == 0 {
        return Err("frame degraded notice names no domain".to_string());
    }
    if domain_count > FRAME_DEGRADED_MAX_DOMAINS {
        return Err(format!(
            "frame degraded notice names {domain_count} domains, above the \
             {FRAME_DEGRADED_MAX_DOMAINS} bound"
        ));
    }
    let header_reserved = u16::from_le_bytes([payload[14], payload[15]]);
    if header_reserved != 0 {
        return Err(format!(
            "frame degraded header sets its reserved field to {header_reserved:#06x}, which this \
             format does not define"
        ));
    }
    let mut offset = FRAME_DEGRADED_HEADER_LEN;
    let mut previous_domain_id: Option<u16> = None;
    for index in 0..domain_count {
        if offset
            .checked_add(FRAME_DEGRADED_ENTRY_FIXED_LEN)
            .is_none_or(|end| end > payload.len())
        {
            return Err(format!(
                "frame degraded entry {index} is truncated at offset {offset}"
            ));
        }
        let domain_id = u16::from_le_bytes([payload[offset], payload[offset + 1]]);
        if let Some(previous) = previous_domain_id {
            if domain_id <= previous {
                return Err(format!(
                    "frame degraded entry {index} domain {domain_id} does not follow {previous} in \
                     strictly increasing order"
                ));
            }
        }
        previous_domain_id = Some(domain_id);
        let status_flags = u32::from_le_bytes([
            payload[offset + 4],
            payload[offset + 5],
            payload[offset + 6],
            payload[offset + 7],
        ]);
        let non_tolerated = u32::from_le_bytes([
            payload[offset + 8],
            payload[offset + 9],
            payload[offset + 10],
            payload[offset + 11],
        ]);
        let reserved = u16::from_le_bytes([payload[offset + 2], payload[offset + 3]]);
        let predicate_code = u32::from_le_bytes([
            payload[offset + 12],
            payload[offset + 13],
            payload[offset + 14],
            payload[offset + 15],
        ]);
        let authority_bytes = &payload[offset + 16..offset + 16 + AUTHORITY_LEN];
        let reason_len = u16::from_le_bytes([payload[offset + 28], payload[offset + 29]]) as usize;
        let name_len = u16::from_le_bytes([payload[offset + 30], payload[offset + 31]]) as usize;
        let start = offset + FRAME_DEGRADED_ENTRY_FIXED_LEN;
        let Some(reason_end) = start.checked_add(reason_len) else {
            return Err(format!(
                "frame degraded entry {index} reason length overflows"
            ));
        };
        let Some(end) = reason_end.checked_add(name_len) else {
            return Err(format!(
                "frame degraded entry {index} predicate name length overflows"
            ));
        };
        if end > payload.len() {
            return Err(format!(
                "frame degraded entry {index} reason or predicate name runs past the payload"
            ));
        }
        if core::str::from_utf8(&payload[start..reason_end]).is_err() {
            return Err(format!("frame degraded entry {index} reason is not UTF-8"));
        }
        let Ok(name) = core::str::from_utf8(&payload[reason_end..end]) else {
            return Err(format!(
                "frame degraded entry {index} predicate name is not UTF-8"
            ));
        };
        check_entry(
            index,
            domain_id,
            reserved,
            status_flags,
            non_tolerated,
            predicate_code,
            name,
            authority_bytes,
            reason_len,
        )?;
        offset = end;
    }
    if offset != payload.len() {
        return Err(format!(
            "frame degraded payload has {} trailing byte(s) after {domain_count} entries",
            payload.len() - offset
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_SOURCE_EMPTY,
        COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_STATE_UNAVAILABLE,
    };

    #[test]
    fn a_measured_empty_domain_is_complete_and_degrades_no_frame() {
        assert_eq!(
            compact_domain_completeness(36, COMPACT_STATUS_SOURCE_EMPTY),
            CompactDomainCompleteness::Complete
        );
        assert!(FrameDegradedDomain::from_status(
            36,
            COMPACT_STATUS_SOURCE_EMPTY,
            producer_diagnosis(),
        )
        .is_none());
        let both = COMPACT_STATUS_SOURCE_EMPTY | COMPACT_STATUS_SOURCE_UNAVAILABLE;
        assert_eq!(
            compact_domain_completeness(36, both),
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
            }
        );
    }

    fn producer_diagnosis() -> FrameDegradedDiagnosis {
        FrameDegradedDiagnosis::new(
            Predicate::new(4701, "domain47_manager_is_readable").unwrap(),
            Authority::OtherDomain(18),
            "domain47_manager_failure_kind=7",
        )
    }

    fn substantiated_diagnosis() -> FrameDegradedDiagnosis {
        FrameDegradedDiagnosis::new(
            Predicate::new(
                crate::domain_outcome::READ_PREDICATE_OUTCOME_SUBSTANTIATED_FROM_RECORD,
                "outcome_substantiated_from_record",
            )
            .unwrap(),
            Authority::SelfDomain,
            "d91_exact_read_failure_code=22",
        )
    }

    fn sample() -> FrameDegradedRecordV2 {
        FrameDegradedRecordV2 {
            degraded_hour: 60831436,
            domains: vec![
                FrameDegradedDomain::from_status(
                    47,
                    COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
                    producer_diagnosis(),
                )
                .expect("domain 47 has no tolerance, so this status is degraded"),
                FrameDegradedDomain::from_status(
                    91,
                    COMPACT_STATUS_SOURCE_UNAVAILABLE,
                    substantiated_diagnosis(),
                )
                .expect("domain 91 has no tolerance, so this status is degraded"),
            ],
        }
    }

    #[test]
    fn round_trips_with_every_field_preserved() {
        let value = sample();
        let wire = value.encode().unwrap();
        validate_frame_degraded_payload(&wire).unwrap();
        assert_eq!(FrameDegradedRecordV2::decode(&wire).unwrap(), value);
    }

    #[test]
    fn the_three_completeness_states_are_disjoint_and_total() {
        assert_eq!(
            compact_domain_completeness(26, 0),
            CompactDomainCompleteness::Complete
        );
        assert_eq!(
            compact_domain_completeness(26, COMPACT_STATUS_PARTIAL_DATA),
            CompactDomainCompleteness::ExpectedPartial
        );
        assert_eq!(
            compact_domain_completeness(
                26,
                COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_STATE_UNAVAILABLE
            ),
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags: COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_STATE_UNAVAILABLE
            }
        );
        assert_eq!(
            compact_domain_completeness(29, COMPACT_STATUS_PARTIAL_DATA),
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags: COMPACT_STATUS_PARTIAL_DATA
            }
        );
        assert_eq!(
            compact_domain_completeness(47, COMPACT_STATUS_PARTIAL_DATA),
            CompactDomainCompleteness::ExpectedPartial
        );
        assert_eq!(
            compact_domain_completeness(
                47,
                COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE
            ),
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags: COMPACT_STATUS_PARTIAL_DATA
                    | COMPACT_STATUS_SOURCE_UNAVAILABLE
            }
        );
    }

    #[test]
    fn a_tolerated_partial_cannot_be_encoded_as_a_degradation() {
        for tolerance in crate::COMPACT_EXPECTED_PARTIAL_TOLERANCES {
            assert!(
                FrameDegradedDomain::from_status(
                    tolerance.domain_id,
                    tolerance.tolerated_flags,
                    producer_diagnosis(),
                )
                .is_none(),
                "domain {} tolerated status must not build a degraded entry",
                tolerance.domain_id
            );
            let forced = FrameDegradedRecordV2 {
                degraded_hour: 7,
                domains: vec![FrameDegradedDomain {
                    domain_id: tolerance.domain_id,
                    status_flags: tolerance.tolerated_flags,
                    non_tolerated_flags: tolerance.tolerated_flags,
                    predicate: producer_diagnosis().predicate,
                    authority: producer_diagnosis().authority,
                    reason: "mislabelled".to_string(),
                }],
            };
            let error = forced
                .encode()
                .expect_err("a tolerated partial must not encode as degraded");
            assert!(
                error.contains("expected-partial tolerance"),
                "the refusal must name the tolerance, got {error}"
            );
            let mut wire = sample().encode().unwrap();
            wire[FRAME_DEGRADED_HEADER_LEN..FRAME_DEGRADED_HEADER_LEN + 2]
                .copy_from_slice(&tolerance.domain_id.to_le_bytes());
            wire[FRAME_DEGRADED_HEADER_LEN + 4..FRAME_DEGRADED_HEADER_LEN + 8]
                .copy_from_slice(&tolerance.tolerated_flags.to_le_bytes());
            wire[FRAME_DEGRADED_HEADER_LEN + 8..FRAME_DEGRADED_HEADER_LEN + 12]
                .copy_from_slice(&tolerance.tolerated_flags.to_le_bytes());
            assert!(
                validate_frame_degraded_payload(&wire).is_err(),
                "a wire entry claiming domain {} tolerated bits are a degradation must fail",
                tolerance.domain_id
            );
        }
    }

    #[test]
    fn a_complete_domain_cannot_be_encoded_as_degraded() {
        assert!(FrameDegradedDomain::from_status(47, 0, producer_diagnosis()).is_none());
        let forced = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 47,
                status_flags: 0,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: producer_diagnosis().authority,
                reason: "fabricated".to_string(),
            }],
        };
        assert!(forced.encode().is_err());
    }

    #[test]
    fn an_entry_that_understates_its_non_tolerated_bits_is_refused() {
        let mut value = sample();
        value.domains[0].non_tolerated_flags = COMPACT_STATUS_SOURCE_UNAVAILABLE;
        let error = value
            .encode()
            .expect_err("understated bits must be refused");
        assert!(error.contains("classifies as"), "{error}");
    }

    #[test]
    fn one_entry_encodes_to_exactly_these_bytes() {
        let record = FrameDegradedRecordV2 {
            degraded_hour: 0x0102_0304,
            domains: vec![FrameDegradedDomain::from_status(
                91,
                COMPACT_STATUS_SOURCE_UNAVAILABLE,
                FrameDegradedDiagnosis::new(
                    Predicate::new(0x0002_0011, "outcome_substantiated_from_record").unwrap(),
                    Authority::OtherDomain(18),
                    "why",
                ),
            )
            .expect("domain 91 has no tolerance, so this status is degraded")],
        };

        let mut expected: Vec<u8> = Vec::new();
        expected.extend_from_slice(b"H4FD");
        expected.extend_from_slice(&2u16.to_le_bytes());
        expected.extend_from_slice(&16u16.to_le_bytes());
        expected.extend_from_slice(&0x0102_0304i32.to_le_bytes());
        expected.extend_from_slice(&1u16.to_le_bytes());
        expected.extend_from_slice(&0u16.to_le_bytes());
        expected.extend_from_slice(&91u16.to_le_bytes());
        expected.extend_from_slice(&0u16.to_le_bytes());
        expected.extend_from_slice(&COMPACT_STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        expected.extend_from_slice(&COMPACT_STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        expected.extend_from_slice(&0x0002_0011u32.to_le_bytes());
        expected.extend_from_slice(&2u16.to_le_bytes());
        expected.extend_from_slice(&18u16.to_le_bytes());
        expected.extend_from_slice(&0u64.to_le_bytes());
        expected.extend_from_slice(&3u16.to_le_bytes());
        expected.extend_from_slice(&33u16.to_le_bytes());
        expected.extend_from_slice(b"why");
        expected.extend_from_slice(b"outcome_substantiated_from_record");

        assert_eq!(record.encode().unwrap(), expected);
        assert_eq!(
            expected.len(),
            FRAME_DEGRADED_HEADER_LEN + FRAME_DEGRADED_ENTRY_FIXED_LEN + 3 + 33,
            "the fixed part is {FRAME_DEGRADED_ENTRY_FIXED_LEN} bytes and the two strings follow it"
        );
        validate_frame_degraded_payload(&expected).unwrap();
    }

    #[test]
    fn a_predicate_that_cannot_be_counted_is_refused() {
        let wire = sample().encode().unwrap();
        let code_at = FRAME_DEGRADED_HEADER_LEN + 12;

        let mut zero_code = wire.clone();
        zero_code[code_at..code_at + 4].copy_from_slice(&0u32.to_le_bytes());
        let error = validate_frame_degraded_payload(&zero_code)
            .expect_err("predicate code 0 must be refused");
        assert!(error.contains("reserved for 'not declared'"), "{error}");

        let mut lying_code = wire.clone();
        lying_code[code_at..code_at + 4]
            .copy_from_slice(&crate::diagnostic_channel::PREDICATE_UNDECLARED.to_le_bytes());
        let error = validate_frame_degraded_payload(&lying_code)
            .expect_err("the reserved code under a real check's name must be refused");
        assert!(error.contains("only valid together"), "{error}");

        let undeclared = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain::from_status(
                91,
                COMPACT_STATUS_SOURCE_UNAVAILABLE,
                FrameDegradedDiagnosis::undeclared(Authority::SelfDomain, "nobody said"),
            )
            .unwrap()],
        };
        let reserved_pair = undeclared
            .encode()
            .expect("the reserved code with the reserved name is the one legal undeclared entry");
        validate_frame_degraded_payload(&reserved_pair).unwrap();
        let mut declared_code = reserved_pair.clone();
        declared_code[code_at..code_at + 4].copy_from_slice(&0x0002_0011u32.to_le_bytes());
        assert!(
            validate_frame_degraded_payload(&declared_code).is_err(),
            "the reserved name under a declared code must be refused"
        );
    }

    #[test]
    fn a_non_canonical_or_undeclared_authority_is_refused() {
        let wire = sample().encode().unwrap();
        let authority_at = FRAME_DEGRADED_HEADER_LEN + 16;

        let mut undeclared = wire.clone();
        undeclared[authority_at..authority_at + 2].copy_from_slice(&0u16.to_le_bytes());
        assert!(validate_frame_degraded_payload(&undeclared).is_err());

        let mut unknown = wire.clone();
        unknown[authority_at..authority_at + 2].copy_from_slice(&9u16.to_le_bytes());
        assert!(validate_frame_degraded_payload(&unknown).is_err());

        let mut noisy_rva = wire.clone();
        noisy_rva[authority_at + 4..authority_at + 12].copy_from_slice(&0xDEADu64.to_le_bytes());
        let error = validate_frame_degraded_payload(&noisy_rva)
            .expect_err("non-canonical authority bytes must be refused");
        assert!(error.contains("not canonical"), "{error}");
    }

    #[test]
    fn a_reserved_field_that_is_not_zero_is_refused() {
        let wire = sample().encode().unwrap();

        let mut header = wire.clone();
        header[14] = 1;
        assert!(validate_frame_degraded_payload(&header).is_err());

        let mut entry = wire.clone();
        entry[FRAME_DEGRADED_HEADER_LEN + 2] = 1;
        assert!(validate_frame_degraded_payload(&entry).is_err());
    }

    #[test]
    fn an_undefined_or_contradictory_status_word_is_refused() {
        let unassigned = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 91,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE | (1 << 30),
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: producer_diagnosis().authority,
                reason: "carries a bit nobody defines".to_string(),
            }],
        };
        let error = unassigned
            .encode()
            .expect_err("an unassigned status bit must be refused");
        assert!(error.contains("unassigned bit"), "{error}");

        let baseline = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 91,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE
                    | crate::COMPACT_STATUS_BASELINE_CLEAR,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: producer_diagnosis().authority,
                reason: "asks for a replacement it cannot justify".to_string(),
            }],
        };
        let error = baseline
            .encode()
            .expect_err("BASELINE_CLEAR on a degraded entry must be refused");
        assert!(error.contains("BASELINE_CLEAR"), "{error}");
    }

    #[test]
    fn an_entry_naming_the_frame_envelope_is_refused() {
        let forced = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: crate::record_semantics::COMPACT_DOMAIN_FRAME,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: producer_diagnosis().authority,
                reason: "the frame is not a domain".to_string(),
            }],
        };
        let error = forced
            .encode()
            .expect_err("the frame envelope is not a per-domain degradation");
        assert!(error.contains("frame envelope"), "{error}");
    }

    #[test]
    fn an_entry_naming_an_unassigned_domain_is_refused() {
        let forced = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 250,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: producer_diagnosis().authority,
                reason: "a degradation of nothing".to_string(),
            }],
        };
        let error = forced
            .encode()
            .expect_err("an unassigned domain cannot be degraded");
        assert!(error.contains("does not assign"), "{error}");
        let mut wire = sample().encode().unwrap();
        wire[16] = 250u16.to_le_bytes()[0];
        wire[17] = 250u16.to_le_bytes()[1];
        assert!(validate_frame_degraded_payload(&wire).is_err());
    }

    #[test]
    fn an_authority_naming_an_unassigned_domain_is_refused() {
        let forced = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 47,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: Authority::OtherDomain(250),
                reason: "blamed a domain that does not exist".to_string(),
            }],
        };
        let error = forced
            .encode()
            .expect_err("an unassigned authority names nobody");
        assert!(error.contains("as its authority"), "{error}");
        assert!(
            crate::assigned_descriptor_by_id(crate::record_semantics::COMPACT_DOMAIN_FRAME)
                .is_some()
        );
        let framed = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 47,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: Authority::OtherDomain(crate::record_semantics::COMPACT_DOMAIN_FRAME),
                reason: "blamed the frame as a peer".to_string(),
            }],
        };
        let error = framed
            .encode()
            .expect_err("the frame envelope is not a peer domain");
        assert!(error.contains("frame envelope"), "{error}");
        let ok = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 47,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: Authority::OtherDomain(91),
                reason: "d91's roster was the authority".to_string(),
            }],
        };
        let wire = ok
            .encode()
            .expect("an assigned peer domain is a real authority");
        validate_frame_degraded_payload(&wire).unwrap();
    }

    #[test]
    fn a_notice_that_names_nobody_is_refused() {
        let empty = FrameDegradedRecordV2 {
            degraded_hour: 1,
            domains: Vec::new(),
        };
        assert!(empty.encode().is_err());
        let mut wire = sample().encode().unwrap();
        wire[12] = 0;
        wire[13] = 0;
        assert!(validate_frame_degraded_payload(&wire).is_err());
    }

    #[test]
    fn a_domain_named_twice_is_refused() {
        let mut value = sample();
        value.domains[1].domain_id = value.domains[0].domain_id;
        let error = value
            .encode()
            .expect_err("duplicate domain must be refused");
        assert!(error.contains("strictly increasing"), "{error}");
    }

    #[test]
    fn a_domain_with_no_reason_is_refused() {
        let mut value = sample();
        value.domains[0].reason = String::new();
        assert!(value.encode().is_err());
    }

    #[test]
    fn an_untaught_consumer_fails_loudly_on_an_unknown_record_type() {
        use crate::record_semantics::{
            validate_compact_record_semantics, COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_DEGRADED,
        };
        let wire = sample().encode().unwrap();

        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_DEGRADED,
            0,
            &wire,
        )
        .expect("the new type must be wired into the canonical dispatch");

        assert!(
            validate_compact_record_semantics(47, COMPACT_RECORD_TYPE_FRAME_DEGRADED, 0, &wire)
                .is_err(),
            "a frame-level record carried on a domain envelope must be refused"
        );

        assert!(validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_DEGRADED,
            0,
            &wire[..wire.len() - 1],
        )
        .is_err());
    }

    #[test]
    fn a_refusal_payload_is_not_a_degraded_notice_and_the_reverse() {
        let degraded = sample().encode().unwrap();
        let refusal = crate::frame_refusal::FrameRefusalRecordV2 {
            refused_hour: 60831436,
            offenders: vec![crate::frame_refusal::FrameRefusalOffenderV2 {
                domain_id: 47,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: crate::diagnostic_channel::Predicate::new(
                    4701,
                    "domain47_manager_is_readable",
                )
                .unwrap(),
                authority: crate::diagnostic_channel::Authority::SelfDomain,
                locator: Vec::new(),
                units: Vec::new(),
                truncation: crate::diagnostic_channel::TruncationAccount::complete(0),
                off_frame: None,
                reason: "domain47_manager_failure_kind=7".to_string(),
                reason_units_named: 0,
            }],
            frame_findings: Vec::new(),
        }
        .encode()
        .unwrap();

        assert!(
            crate::frame_refusal::validate_frame_refusal_payload(&degraded).is_err(),
            "a degraded notice must not validate as a refusal"
        );
        assert!(
            validate_frame_degraded_payload(&refusal).is_err(),
            "a refusal must not validate as a degraded notice"
        );
    }

    #[test]
    fn every_truncation_and_corruption_fails_closed() {
        let wire = sample().encode().unwrap();
        for len in 0..wire.len() {
            assert!(
                validate_frame_degraded_payload(&wire[..len]).is_err(),
                "prefix of {len} byte(s) must not validate"
            );
        }
        let mut trailing = wire.clone();
        trailing.push(0);
        assert!(validate_frame_degraded_payload(&trailing).is_err());
        for (index, label) in [(0usize, "magic"), (4, "version"), (6, "header_len")] {
            let mut bad = wire.clone();
            bad[index] ^= 0xFF;
            assert!(
                validate_frame_degraded_payload(&bad).is_err(),
                "corrupt {label} must fail"
            );
        }
    }
}
