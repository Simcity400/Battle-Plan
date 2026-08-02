//! Frame-level DEGRADED notice (`H4FD`) and the three-state domain
//! completeness vocabulary it belongs to.
//!
//! # Why this record exists
//!
//! Frame completeness was all-or-nothing across the whole scheduled roster: one
//! domain whose non-tolerated status flags fired refused the entire frame
//! (`H4FR`). With 73 scheduled domains the probability of all of them being
//! simultaneously clean is not a design margin, it is a deadlock -- and the
//! decisive consequence is not the lost hours, it is that NO domain fix can be
//! verified end to end, because nothing is ever published to verify against.
//!
//! This record adds the other outcome: the frame IS published, and the domains
//! that failed are named, in band, as UNKNOWN for that hour.
//!
//! # The distinction this record has to protect
//!
//! There are exactly three things a domain can be in a published frame, and a
//! consumer must be able to tell them apart from the wire alone:
//!
//! | state              | PERF status                                   | named in `H4FD` | fold |
//! |--------------------|-----------------------------------------------|-----------------|------|
//! | complete           | no failure-class bits                         | no              | applied |
//! | expected-partial   | failure bits, all inside the domain's `COMPACT_EXPECTED_PARTIAL_TOLERANCES` entry | no | applied as a sparse patch |
//! | degraded-unknown   | failure bits OUTSIDE any tolerance            | YES             | discarded, accumulator retained |
//!
//! An expected-partial entry asserts something specific and reviewed: *this
//! partial is expected, and the written integrity argument for why the sparse
//! patch is safe is in the table*. A degraded domain asserts the opposite: *this
//! failed and nobody signed off on it; here is the reason, do not use this
//! hour*. Labelling a defect as expected would be worse than the deadlock this
//! record solves, because the deadlock is loud and a mislabel is silent.
//!
//! So the two are kept apart by construction rather than by convention:
//!
//! * a degraded entry's `non_tolerated_flags` must equal exactly what
//!   [`crate::compact_status_failure_flags_after_expected_partial_tolerance`]
//!   computes for that domain and status word -- the SAME function the fold and
//!   the certifier classify with. A status the tolerance table fully covers
//!   yields zero there, and a zero entry is rejected. It is therefore not
//!   possible to encode an expected-partial as a degradation, or to encode a
//!   degradation while understating which bits forced it.
//! * conversely a tolerated domain is never listed here, so "named in `H4FD`"
//!   and "tolerated" are disjoint on the wire without the consumer having to
//!   diff two tables.
//!
//! `H4FD` is NOT a frame boundary record. It rides inside the published frame,
//! before its `FRAME_END`, because it describes that frame. `H4FR` closes an
//! hour that published nothing; this one annotates an hour that published.
//!
//! Layout, all little-endian. Header is fixed; entries follow contiguously and
//! are ordered by strictly increasing `domain_id` so one domain cannot appear
//! twice with disagreeing bits.
//!
//! ```text
//! magic          [4]  b"H4FD"
//! version        u16  = FRAME_DEGRADED_VERSION
//! header_len     u16  = FRAME_DEGRADED_HEADER_LEN
//! degraded_hour  i32
//! domain_count   u16
//! reserved       u16
//! -- 16 byte header --
//! per degraded domain:
//!   domain_id       u16
//!   reserved        u16
//!   status_flags    u32   raw PERF status for that domain
//!   non_tolerated   u32   the subset that made it unknown
//!   reason_len      u16
//!   reserved        u16
//!   reason          [reason_len]  UTF-8, the reader's typed diagnostic units
//! ```

use crate::{
    compact_status_failure_flags_after_expected_partial_tolerance, expected_partial_tolerance,
    expected_partial_tolerance_applies, CompactDomainId,
};

pub const FRAME_DEGRADED_MAGIC: &[u8; 4] = b"H4FD";
pub const FRAME_DEGRADED_VERSION: u16 = 1;
pub const FRAME_DEGRADED_HEADER_LEN: usize = 16;
pub const FRAME_DEGRADED_ENTRY_FIXED_LEN: usize = 16;
/// Bound on one reason string, so a pathological reader cannot make the record
/// unbounded. Same bound as `H4FR`; the reasons come from the same place.
pub const FRAME_DEGRADED_MAX_REASON_LEN: usize = 512;
/// Bound on named domains. The scheduled roster is 73; this leaves headroom
/// without admitting an absurd record.
pub const FRAME_DEGRADED_MAX_DOMAINS: usize = 1024;

/// The shared reason for a domain that failed and did NOT say why.
///
/// Some scheduled domains attach no diagnostic units at all, so their failures
/// arrive as a bare status bit with no typed cause. That is a defect in those
/// domains -- but it must not become a defect a consumer cannot SEE. An entry
/// with an empty reason is refused by this codec, so without one shared name
/// for the condition a silent failure would either be unpublishable or
/// laundered into a blank string.
///
/// This names it, identically for every domain. It is a work item, never an
/// excuse: the number of entries carrying this reason is the exact size of the
/// silent-domain problem, and it is now countable from any captured stream
/// instead of being invisible.
pub const FRAME_DEGRADED_REASON_UNDIAGNOSED: &str = "undiagnosed_failure";

/// What one domain's status says about that domain in a published frame.
///
/// This is the single classifier. The collector decides what to publish with
/// it, the fold decides what to apply with it, and the certifier decides what
/// to accept with it, so the three cannot drift into disagreeing about whether
/// an hour is usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactDomainCompleteness {
    /// No failure-class bit is set. The domain's rows are complete for the hour.
    Complete,
    /// Failure-class bits are set and are entirely covered by this domain's
    /// `COMPACT_EXPECTED_PARTIAL_TOLERANCES` entry: a reviewed partial with a
    /// written integrity argument. Rows are a sparse patch, omissions are known
    /// and NOT fabricated as deletions.
    ExpectedPartial,
    /// Failure-class bits are set that no tolerance covers. The domain/hour is
    /// UNKNOWN. Its rows must not be applied and its accumulator must be
    /// retained. `non_tolerated_flags` is the subset that made it so.
    Degraded { non_tolerated_flags: u32 },
}

impl CompactDomainCompleteness {
    pub const fn is_degraded(self) -> bool {
        matches!(self, Self::Degraded { .. })
    }
}

/// Classify one domain's raw PERF status word.
///
/// Lane-local status bits are stripped first (a lane badge is not a global
/// failure), which is why this goes through the shared normalizer rather than
/// masking directly.
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
pub struct FrameDegradedDomain {
    pub domain_id: u16,
    pub status_flags: u32,
    pub non_tolerated_flags: u32,
    pub reason: String,
}

impl FrameDegradedDomain {
    /// Build an entry for a domain, or `None` when the domain is not degraded.
    ///
    /// This is the only sanctioned way to produce an entry: `non_tolerated_flags`
    /// is DERIVED from the shared classifier rather than supplied, so a caller
    /// cannot list a complete or expected-partial domain here even by mistake.
    pub fn from_status(
        domain_id: CompactDomainId,
        status_flags: u32,
        reason: impl Into<String>,
    ) -> Option<Self> {
        match compact_domain_completeness(domain_id, status_flags) {
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags,
            } => Some(Self {
                domain_id,
                status_flags,
                non_tolerated_flags,
                reason: reason.into(),
            }),
            CompactDomainCompleteness::Complete | CompactDomainCompleteness::ExpectedPartial => {
                None
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDegradedRecordV1 {
    pub degraded_hour: i32,
    pub domains: Vec<FrameDegradedDomain>,
}

/// The one place an entry's self-consistency is decided, shared by `encode`
/// and `validate_frame_degraded_payload` so a record cannot be writable and
/// unreadable (or the reverse).
fn check_entry(
    index: usize,
    domain_id: u16,
    status_flags: u32,
    non_tolerated_flags: u32,
    reason_len: usize,
) -> Result<(), String> {
    if non_tolerated_flags == 0 {
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) claims no non-tolerated bits, so it \
             is not degraded"
        ));
    }
    let expected =
        compact_status_failure_flags_after_expected_partial_tolerance(domain_id, status_flags);
    if expected != non_tolerated_flags {
        // The mislabel guard. If the status is fully covered by the domain's
        // tolerance entry, `expected` is zero and this fires -- a defect can
        // never be written down as an expected partial, and an expected partial
        // can never be written down as a defect.
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

impl FrameDegradedRecordV1 {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.domains.is_empty() {
            // A degraded notice naming nobody would assert a defect it cannot
            // substantiate, and a consumer seeing it could not tell which hour
            // data to distrust. Fail closed.
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
                    // Two entries for one domain could disagree about its bits,
                    // and a consumer would have no rule for which wins.
                    return Err(format!(
                        "frame degraded entry {index} domain {} does not follow {previous} in \
                         strictly increasing order",
                        domain.domain_id
                    ));
                }
            }
            previous_domain_id = Some(domain.domain_id);
            let reason = domain.reason.as_bytes();
            check_entry(
                index,
                domain.domain_id,
                domain.status_flags,
                domain.non_tolerated_flags,
                reason.len(),
            )?;
            out.extend_from_slice(&domain.domain_id.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&domain.status_flags.to_le_bytes());
            out.extend_from_slice(&domain.non_tolerated_flags.to_le_bytes());
            out.extend_from_slice(&(reason.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(reason);
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
            let reason_len =
                u16::from_le_bytes([payload[offset + 12], payload[offset + 13]]) as usize;
            let start = offset + FRAME_DEGRADED_ENTRY_FIXED_LEN;
            let end = start + reason_len;
            let reason = core::str::from_utf8(&payload[start..end])
                .map_err(|error| format!("frame degraded reason is not UTF-8: {error}"))?
                .to_string();
            domains.push(FrameDegradedDomain {
                domain_id,
                status_flags,
                non_tolerated_flags,
                reason,
            });
            offset = end;
        }
        Ok(Self {
            degraded_hour,
            domains,
        })
    }

    /// The domains this notice declares unknown, for a consumer cross-checking
    /// the frame's PERF records against the notice.
    pub fn declared_domain_ids(&self) -> Vec<u16> {
        self.domains.iter().map(|domain| domain.domain_id).collect()
    }
}

/// Complete grammar check. Total over arbitrary bytes; never panics.
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
        let reason_len = u16::from_le_bytes([payload[offset + 12], payload[offset + 13]]) as usize;
        check_entry(index, domain_id, status_flags, non_tolerated, reason_len)?;
        let start = offset + FRAME_DEGRADED_ENTRY_FIXED_LEN;
        let Some(end) = start.checked_add(reason_len) else {
            return Err(format!(
                "frame degraded entry {index} reason length overflows"
            ));
        };
        if end > payload.len() {
            return Err(format!(
                "frame degraded entry {index} reason runs past the payload"
            ));
        }
        if core::str::from_utf8(&payload[start..end]).is_err() {
            return Err(format!("frame degraded entry {index} reason is not UTF-8"));
        }
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
        COMPACT_STATUS_PARTIAL_DATA, COMPACT_STATUS_SOURCE_UNAVAILABLE,
        COMPACT_STATUS_STATE_UNAVAILABLE,
    };

    fn sample() -> FrameDegradedRecordV1 {
        FrameDegradedRecordV1 {
            degraded_hour: 60831436,
            domains: vec![
                FrameDegradedDomain::from_status(
                    47,
                    COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
                    "domain47_manager_failure_kind=7",
                )
                .expect("domain 47 has no tolerance, so this status is degraded"),
                FrameDegradedDomain::from_status(
                    91,
                    COMPACT_STATUS_SOURCE_UNAVAILABLE,
                    "d91_exact_read_failure_code=22",
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
        assert_eq!(FrameDegradedRecordV1::decode(&wire).unwrap(), value);
    }

    /// The three states are DISJOINT and every failure status lands in exactly
    /// one of them.
    #[test]
    fn the_three_completeness_states_are_disjoint_and_total() {
        // Complete: no failure-class bit at all.
        assert_eq!(
            compact_domain_completeness(26, 0),
            CompactDomainCompleteness::Complete
        );
        // Expected-partial: domain 26's tolerance covers exactly PARTIAL_DATA.
        assert_eq!(
            compact_domain_completeness(26, COMPACT_STATUS_PARTIAL_DATA),
            CompactDomainCompleteness::ExpectedPartial
        );
        // Degraded: the same domain with a bit the tolerance does NOT cover.
        //
        // Note WHICH bits come back. A tolerance is all-or-nothing per domain:
        // `expected_partial_tolerance_applies` requires the failure set to sit
        // ENTIRELY inside the tolerated set, so once one uncovered bit is
        // present the tolerance stops applying and NOTHING is tolerated -- the
        // residual is the whole failure set, PARTIAL_DATA included. That is the
        // conservative reading and the right one: the written integrity
        // argument for d26's partial was made about a sampled-root shortfall
        // alone, and it does not carry over to a partial that happened
        // alongside an unavailable state.
        assert_eq!(
            compact_domain_completeness(
                26,
                COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_STATE_UNAVAILABLE
            ),
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags: COMPACT_STATUS_PARTIAL_DATA
                    | COMPACT_STATUS_STATE_UNAVAILABLE
            }
        );
        // And a domain with NO tolerance entry: the same PARTIAL_DATA is a
        // degradation, not an expected partial.
        assert_eq!(
            compact_domain_completeness(47, COMPACT_STATUS_PARTIAL_DATA),
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags: COMPACT_STATUS_PARTIAL_DATA
            }
        );
    }

    /// The constraint this whole record exists to protect: a tolerated partial
    /// can NEVER be written down as a degradation, and the reverse.
    #[test]
    fn a_tolerated_partial_cannot_be_encoded_as_a_degradation() {
        // Every tolerance entry, exercised against its own tolerated bits.
        for tolerance in crate::COMPACT_EXPECTED_PARTIAL_TOLERANCES {
            assert!(
                FrameDegradedDomain::from_status(
                    tolerance.domain_id,
                    tolerance.tolerated_flags,
                    "some reason",
                )
                .is_none(),
                "domain {} tolerated status must not build a degraded entry",
                tolerance.domain_id
            );
            // And a hand-built entry that lies about it is refused by the codec,
            // not merely discouraged by the constructor.
            let forced = FrameDegradedRecordV1 {
                degraded_hour: 7,
                domains: vec![FrameDegradedDomain {
                    domain_id: tolerance.domain_id,
                    status_flags: tolerance.tolerated_flags,
                    non_tolerated_flags: tolerance.tolerated_flags,
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
            // And the same lie cannot be smuggled in on the wire either.
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

    /// A complete domain cannot be listed either: zero failure bits classifies
    /// as zero non-tolerated, and a zero entry is not a degradation.
    #[test]
    fn a_complete_domain_cannot_be_encoded_as_degraded() {
        assert!(FrameDegradedDomain::from_status(47, 0, "nothing wrong").is_none());
        let forced = FrameDegradedRecordV1 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 47,
                status_flags: 0,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                reason: "fabricated".to_string(),
            }],
        };
        assert!(forced.encode().is_err());
    }

    /// An entry may not understate which bits made it unknown: the record must
    /// agree with the shared classifier exactly.
    #[test]
    fn an_entry_that_understates_its_non_tolerated_bits_is_refused() {
        let mut value = sample();
        value.domains[0].non_tolerated_flags = COMPACT_STATUS_SOURCE_UNAVAILABLE;
        let error = value.encode().expect_err("understated bits must be refused");
        assert!(error.contains("classifies as"), "{error}");
    }

    #[test]
    fn a_notice_that_names_nobody_is_refused() {
        let empty = FrameDegradedRecordV1 {
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
        let error = value.encode().expect_err("duplicate domain must be refused");
        assert!(error.contains("strictly increasing"), "{error}");
    }

    #[test]
    fn a_domain_with_no_reason_is_refused() {
        let mut value = sample();
        value.domains[0].reason = String::new();
        assert!(value.encode().is_err());
    }

    /// A consumer that has not been taught this record type must FAIL, not skip.
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

        // Wrong envelope domain is refused: this is a FRAME-level record.
        assert!(
            validate_compact_record_semantics(47, COMPACT_RECORD_TYPE_FRAME_DEGRADED, 0, &wire)
                .is_err(),
            "a frame-level record carried on a domain envelope must be refused"
        );

        // A malformed payload is refused through the same dispatch.
        assert!(validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_DEGRADED,
            0,
            &wire[..wire.len() - 1],
        )
        .is_err());
    }

    /// `H4FD` and `H4FR` must not be confusable for one another. They share a
    /// header shape, so only the magic separates them -- and it must.
    #[test]
    fn a_refusal_payload_is_not_a_degraded_notice_and_the_reverse() {
        let degraded = sample().encode().unwrap();
        let refusal = crate::frame_refusal::FrameRefusalRecordV1 {
            refused_hour: 60831436,
            offenders: vec![crate::frame_refusal::FrameRefusalOffender {
                domain_id: 47,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                reason: "domain47_manager_failure_kind=7".to_string(),
            }],
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
