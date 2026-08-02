//! Frame-level refusal record (`H4FR`).
//!
//! A refused frame is ONE event with a list of offenders, not 73 domains each
//! having a gap. Forcing it into the per-domain gap vocabulary was modelling it
//! wrong before it was a naming problem, which is why none of the five
//! `DOMAIN_GAP` reasons fitted: Torn, Overrun, Backpressure, ClockUnreadable and
//! WorkerPanic all describe a throughput or acquisition outcome for a single
//! domain, and a refusal is neither.
//!
//! What this record has to make true for a consumer:
//!
//! * the refused hour is ACCOUNTED FOR. Reading the stream must show that hour
//!   N was attempted and refused, not that it never existed. That was the right
//!   instinct behind the per-domain gaps and it survives the change.
//! * a refusal is DISTINGUISHABLE from a throughput loss without special
//!   knowledge. A consumer that sees `H4FR` knows the collector declined to
//!   publish because named domains could not read; a consumer that sees
//!   `H4DG`/BACKPRESSURE knows a queue overflowed. Those are different facts and
//!   the stream now says which.
//! * the offenders are named with the SAME detail the refusal was decided on --
//!   domain, the raw status bits, the non-tolerated subset, and the reader's own
//!   typed reason -- rather than a count.
//!
//! Layout, all little-endian. Header is fixed; entries follow contiguously.
//!
//! ```text
//! magic          [4]  b"H4FR"
//! version        u16  = FRAME_REFUSAL_VERSION
//! header_len     u16  = FRAME_REFUSAL_HEADER_LEN
//! refused_hour   i32
//! offender_count u16
//! reserved       u16
//! -- 16 byte header --
//! per offender:
//!   domain_id       u16
//!   reserved        u16
//!   status_flags    u32   raw PERF status for that domain
//!   non_tolerated   u32   the subset that actually forced the refusal
//!   reason_len      u16
//!   reserved        u16
//!   reason          [reason_len]  UTF-8, the reader's typed diagnostic units
//! ```

pub const FRAME_REFUSAL_MAGIC: &[u8; 4] = b"H4FR";
pub const FRAME_REFUSAL_VERSION: u16 = 1;
pub const FRAME_REFUSAL_HEADER_LEN: usize = 16;
pub const FRAME_REFUSAL_ENTRY_FIXED_LEN: usize = 16;
/// Bound on one reason string, so a pathological reader cannot make a refusal
/// record unbounded.
pub const FRAME_REFUSAL_MAX_REASON_LEN: usize = 512;
/// Bound on offenders. The scheduled roster is 73; this leaves headroom without
/// admitting an absurd record.
pub const FRAME_REFUSAL_MAX_OFFENDERS: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRefusalOffender {
    pub domain_id: u16,
    pub status_flags: u32,
    pub non_tolerated_flags: u32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRefusalRecordV1 {
    pub refused_hour: i32,
    pub offenders: Vec<FrameRefusalOffender>,
}

impl FrameRefusalRecordV1 {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.offenders.is_empty() {
            // A refusal with no offenders is incoherent: something refused the
            // frame and this record would not say what. Fail closed rather than
            // emit a record that asserts a refusal it cannot substantiate.
            return Err("frame refusal names no offending domain".to_string());
        }
        if self.offenders.len() > FRAME_REFUSAL_MAX_OFFENDERS {
            return Err(format!(
                "frame refusal names {} offenders, above the {FRAME_REFUSAL_MAX_OFFENDERS} bound",
                self.offenders.len()
            ));
        }
        let mut out = Vec::with_capacity(FRAME_REFUSAL_HEADER_LEN + self.offenders.len() * 64);
        out.extend_from_slice(FRAME_REFUSAL_MAGIC);
        out.extend_from_slice(&FRAME_REFUSAL_VERSION.to_le_bytes());
        out.extend_from_slice(&(FRAME_REFUSAL_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&self.refused_hour.to_le_bytes());
        out.extend_from_slice(&(self.offenders.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        for offender in &self.offenders {
            if offender.non_tolerated_flags == 0 {
                // An offender with nothing non-tolerated did not force the
                // refusal, so listing it would misattribute the cause.
                return Err(format!(
                    "frame refusal lists domain {} with no non-tolerated bits",
                    offender.domain_id
                ));
            }
            let reason = offender.reason.as_bytes();
            if reason.is_empty() {
                return Err(format!(
                    "frame refusal gives domain {} no reason",
                    offender.domain_id
                ));
            }
            if reason.len() > FRAME_REFUSAL_MAX_REASON_LEN {
                return Err(format!(
                    "frame refusal reason for domain {} is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_REASON_LEN} bound",
                    offender.domain_id,
                    reason.len()
                ));
            }
            out.extend_from_slice(&offender.domain_id.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&offender.status_flags.to_le_bytes());
            out.extend_from_slice(&offender.non_tolerated_flags.to_le_bytes());
            out.extend_from_slice(&(reason.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(reason);
        }
        Ok(out)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        validate_frame_refusal_payload(payload)?;
        let refused_hour = i32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
        let offender_count = u16::from_le_bytes([payload[12], payload[13]]) as usize;
        let mut offenders = Vec::with_capacity(offender_count);
        let mut offset = FRAME_REFUSAL_HEADER_LEN;
        for _ in 0..offender_count {
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
            let start = offset + FRAME_REFUSAL_ENTRY_FIXED_LEN;
            let end = start + reason_len;
            let reason = core::str::from_utf8(&payload[start..end])
                .map_err(|error| format!("frame refusal reason is not UTF-8: {error}"))?
                .to_string();
            offenders.push(FrameRefusalOffender {
                domain_id,
                status_flags,
                non_tolerated_flags,
                reason,
            });
            offset = end;
        }
        Ok(Self {
            refused_hour,
            offenders,
        })
    }
}

/// Complete grammar check. Total over arbitrary bytes; never panics.
pub fn validate_frame_refusal_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() < FRAME_REFUSAL_HEADER_LEN {
        return Err(format!(
            "frame refusal payload {} byte(s), below the {FRAME_REFUSAL_HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    if &payload[0..4] != FRAME_REFUSAL_MAGIC {
        return Err("frame refusal magic is not H4FR".to_string());
    }
    let version = u16::from_le_bytes([payload[4], payload[5]]);
    if version != FRAME_REFUSAL_VERSION {
        return Err(format!(
            "frame refusal version {version} is not {FRAME_REFUSAL_VERSION}"
        ));
    }
    let header_len = u16::from_le_bytes([payload[6], payload[7]]) as usize;
    if header_len != FRAME_REFUSAL_HEADER_LEN {
        return Err(format!(
            "frame refusal header length {header_len} is not {FRAME_REFUSAL_HEADER_LEN}"
        ));
    }
    let offender_count = u16::from_le_bytes([payload[12], payload[13]]) as usize;
    if offender_count == 0 {
        return Err("frame refusal names no offending domain".to_string());
    }
    if offender_count > FRAME_REFUSAL_MAX_OFFENDERS {
        return Err(format!(
            "frame refusal names {offender_count} offenders, above the \
             {FRAME_REFUSAL_MAX_OFFENDERS} bound"
        ));
    }
    let mut offset = FRAME_REFUSAL_HEADER_LEN;
    for index in 0..offender_count {
        if offset
            .checked_add(FRAME_REFUSAL_ENTRY_FIXED_LEN)
            .is_none_or(|end| end > payload.len())
        {
            return Err(format!(
                "frame refusal entry {index} is truncated at offset {offset}"
            ));
        }
        let non_tolerated = u32::from_le_bytes([
            payload[offset + 8],
            payload[offset + 9],
            payload[offset + 10],
            payload[offset + 11],
        ]);
        if non_tolerated == 0 {
            let domain_id = u16::from_le_bytes([payload[offset], payload[offset + 1]]);
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has no non-tolerated bits"
            ));
        }
        let reason_len = u16::from_le_bytes([payload[offset + 12], payload[offset + 13]]) as usize;
        if reason_len == 0 {
            return Err(format!("frame refusal entry {index} gives no reason"));
        }
        if reason_len > FRAME_REFUSAL_MAX_REASON_LEN {
            return Err(format!(
                "frame refusal entry {index} reason is {reason_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_REASON_LEN} bound"
            ));
        }
        let start = offset + FRAME_REFUSAL_ENTRY_FIXED_LEN;
        let Some(end) = start.checked_add(reason_len) else {
            return Err(format!("frame refusal entry {index} reason length overflows"));
        };
        if end > payload.len() {
            return Err(format!(
                "frame refusal entry {index} reason runs past the payload"
            ));
        }
        if core::str::from_utf8(&payload[start..end]).is_err() {
            return Err(format!("frame refusal entry {index} reason is not UTF-8"));
        }
        offset = end;
    }
    if offset != payload.len() {
        return Err(format!(
            "frame refusal payload has {} trailing byte(s) after {offender_count} entries",
            payload.len() - offset
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FrameRefusalRecordV1 {
        FrameRefusalRecordV1 {
            refused_hour: 60831436,
            offenders: vec![
                FrameRefusalOffender {
                    domain_id: 47,
                    status_flags: 0x0000_0300,
                    non_tolerated_flags: 0x0000_0300,
                    reason: "domain47_manager_failure_kind=7".to_string(),
                },
                FrameRefusalOffender {
                    domain_id: 91,
                    status_flags: 0x0000_0100,
                    non_tolerated_flags: 0x0000_0100,
                    reason: "d91_exact_read_failure_code=22".to_string(),
                },
            ],
        }
    }

    #[test]
    fn round_trips_with_every_offender_field_preserved() {
        let value = sample();
        let wire = value.encode().unwrap();
        validate_frame_refusal_payload(&wire).unwrap();
        assert_eq!(FrameRefusalRecordV1::decode(&wire).unwrap(), value);
    }

    #[test]
    fn a_refusal_that_names_nobody_is_refused() {
        let empty = FrameRefusalRecordV1 {
            refused_hour: 1,
            offenders: Vec::new(),
        };
        assert!(empty.encode().is_err());
        // And on the wire: a header claiming zero offenders is rejected rather
        // than decoded as "refused for no reason".
        let mut wire = sample().encode().unwrap();
        wire[12] = 0;
        wire[13] = 0;
        assert!(validate_frame_refusal_payload(&wire).is_err());
    }

    #[test]
    fn an_offender_with_no_non_tolerated_bits_is_refused() {
        let mut value = sample();
        value.offenders[0].non_tolerated_flags = 0;
        assert!(value.encode().is_err());
    }

    #[test]
    fn an_offender_with_no_reason_is_refused() {
        let mut value = sample();
        value.offenders[0].reason = String::new();
        assert!(value.encode().is_err());
    }

    /// A consumer that has not been taught this record type must FAIL, not skip.
    ///
    /// The canonical validator's catch-all returns "unknown compact record type",
    /// which is the loud failure a new type needs. This asserts both halves: the
    /// new type is accepted through the real dispatch (so the contract is
    /// actually wired, not just defined), and a type nobody has defined is still
    /// rejected (so the loud failure has not been softened into a skip).
    ///
    /// Two out-of-tree roster parsers were silently stale for weeks on this
    /// project. A record type that a reader quietly ignores is the same failure
    /// with a different shape.
    #[test]
    fn an_untaught_consumer_fails_loudly_on_an_unknown_record_type() {
        use crate::record_semantics::{
            validate_compact_record_semantics, COMPACT_DOMAIN_FRAME, COMPACT_RECORD_TYPE_FRAME_REFUSED,
        };
        let wire = sample().encode().unwrap();

        validate_compact_record_semantics(COMPACT_DOMAIN_FRAME, COMPACT_RECORD_TYPE_FRAME_REFUSED, 0, &wire)
            .expect("the new type must be wired into the canonical dispatch");

        // Wrong envelope domain is refused: this is a FRAME-level record.
        assert!(
            validate_compact_record_semantics(47, COMPACT_RECORD_TYPE_FRAME_REFUSED, 0, &wire).is_err(),
            "a frame-level record carried on a domain envelope must be refused"
        );

        // And an undefined type still fails loudly rather than being skipped.
        //
        // This probe used to be type 7, which is now FRAME_DEGRADED. The test's
        // meaning is "a type nobody has defined must fail loudly", not "type 7
        // must fail", so the probe moves to a value the contract does not
        // assign rather than the assertion being softened. If a future type
        // claims this value, this probe moves again -- it must never be the
        // assertion that gives way.
        let unassigned_record_type = u16::MAX;
        let unknown = validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            unassigned_record_type,
            0,
            &wire,
        );
        assert!(
            unknown
                .as_ref()
                .err()
                .is_some_and(|error| error.contains("unknown compact record type")),
            "an undefined record type must fail loudly, got {unknown:?}"
        );
    }

    #[test]
    fn every_truncation_and_corruption_fails_closed() {
        let wire = sample().encode().unwrap();
        for len in 0..wire.len() {
            assert!(
                validate_frame_refusal_payload(&wire[..len]).is_err(),
                "prefix of {len} byte(s) must not validate"
            );
        }
        let mut trailing = wire.clone();
        trailing.push(0);
        assert!(validate_frame_refusal_payload(&trailing).is_err());
        for (index, label) in [(0usize, "magic"), (4, "version"), (6, "header_len")] {
            let mut bad = wire.clone();
            bad[index] ^= 0xFF;
            assert!(
                validate_frame_refusal_payload(&bad).is_err(),
                "corrupt {label} must fail"
            );
        }
    }
}
