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
//!
//! # Version 2: the reason that a v1 record could not carry
//!
//! Version 1's `reason` is one comma-joined string, and the producer that
//! built it kept six units and appended a literal `...`. Measured on
//! `runs/postmerge_recovery_20260731/deflist-0{1,2}`, that truncation fired on
//! every single frame for six domains, and the units it dropped were the ones
//! that would have named the failure -- D44's deciding units did not reach the
//! wire in either capture. A frame refusal whose whole job is to say WHY was
//! deleting the why.
//!
//! Version 2 keeps the free-form reason -- it is the producer's own text and it
//! is usually already computed -- and adds the structure a diagnosis needs:
//!
//! * a **predicate**, non-zero code plus name, so a refusal names WHICH check
//!   failed rather than a walk stage that conflates three to seven of them;
//! * an **authority**, so "a different domain's roster was incomplete" is not
//!   reported as this domain's defect;
//! * a **locator path** of `(ordinal, extent)` levels, so "region 41" arrives
//!   as "region 41 of 96 in country container 3 of 12";
//! * **typed units** carrying a role and a presence, so a zero is a zero and a
//!   reset sentinel is not read as an index;
//! * **typed truncation accounting** -- offered, emitted, bytes dropped, kind
//!   -- instead of three dots that a real reason could also end with;
//! * an optional **off-frame outcome**, so a derivation that is still running
//!   reports how long it has been running and against what deadline.
//!
//! ## Version 2 also accounts a refusal that belongs to no domain
//!
//! Version 1's header reserved a `u16` and required at least one offender, and
//! the producer guarded its whole publication on `!offenders.is_empty()`. A
//! frame refused for a reason that is not attributable to any single domain --
//! its own clock, its identity, a framing fault in assembly -- therefore
//! published NOTHING, and the hour went entirely unaccounted: nothing in the
//! stream recorded that it had happened or why. That is the same defect this
//! record was created to fix, one level up.
//!
//! Version 2 spends the reserved `u16` on `frame_finding_count` and carries
//! frame-level findings after the offenders. A refusal must still substantiate
//! itself -- it must name at least one offender OR at least one frame finding
//! -- so nothing is weakened; what changes is that "the frame's own clock was
//! unreadable" is now sayable instead of unsayable.
//!
//! ```text
//! per offender (version 2):
//!   domain_id            u16
//!   reserved             u16   MUST BE ZERO, so a later version can spend it
//!   status_flags         u32
//!   non_tolerated        u32
//!   predicate_code       u32   NON-ZERO; PREDICATE_UNDECLARED if the producer
//!                              declared none
//!   truncation_kind      u8    NON-ZERO
//!   reason_units_named   u8    how many of `units`, from the front, the
//!                              reason names; the typed replacement for `...`
//!   predicate_name_len   u16
//!   units_offered        u32
//!   units_emitted        u32
//!   bytes_dropped        u32
//!   authority            [12]  kind u16 | domain_id u16 | rva u64
//!   entry_flags          u16   bit0 = an off-frame outcome follows
//!   reason_len           u16
//!   locator_len          u16
//!   units_len            u16
//!   -- 52 byte fixed entry head --
//!   predicate_name       [predicate_name_len]
//!   locator              [locator_len]
//!   units                [units_len]
//!   off_frame            [28] when entry_flags bit0
//!   reason               [reason_len]
//!
//! per frame finding (version 2), after all offenders:
//!   predicate_code       u32   NON-ZERO
//!   authority            [12]
//!   predicate_name_len   u16
//!   locator_len          u16
//!   reason_len           u16
//!   reserved             u16
//!   -- 24 byte fixed finding head --
//!   predicate_name       [predicate_name_len]
//!   locator              [locator_len]
//!   reason               [reason_len]
//! ```

use crate::diagnostic_channel::{
    decode_locator, decode_units, encode_locator, encode_units, Authority, DiagUnit, LocatorLevel,
    OffFrameOutcome, Predicate, TruncationAccount, TruncationKind, OFF_FRAME_LEN,
};

pub const FRAME_REFUSAL_MAGIC: &[u8; 4] = b"H4FR";
pub const FRAME_REFUSAL_VERSION: u16 = 1;
/// Version written by the current producer. Version 1 stays decodable so
/// retained telemetry from earlier recorder builds remains readable.
pub const FRAME_REFUSAL_VERSION_V2: u16 = 2;
pub const FRAME_REFUSAL_HEADER_LEN: usize = 16;
pub const FRAME_REFUSAL_ENTRY_FIXED_LEN: usize = 16;
pub const FRAME_REFUSAL_V2_ENTRY_FIXED_LEN: usize = 52;
/// Bound on one locator blob.
pub const FRAME_REFUSAL_MAX_LOCATOR_LEN: usize = 1024;
/// Bound on one units blob. Sized so the per-offender floor
/// ([`crate::diagnostic_channel::OFFENDER_FLOOR_BYTES`]) plus a generous
/// discretionary share always fits.
pub const FRAME_REFUSAL_MAX_UNITS_LEN: usize = 4096;
/// Bound on one predicate name.
pub const FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN: usize =
    crate::diagnostic_channel::MAX_PREDICATE_NAME_LEN;
const ENTRY_FLAG_OFF_FRAME: u16 = 1 << 0;
/// Bound on one reason string, so a pathological reader cannot make a refusal
/// record unbounded.
pub const FRAME_REFUSAL_MAX_REASON_LEN: usize = 512;

/// Separator a producer places between its rendered diagnostic units and the
/// original producer text, as in `domain 88: read_completion=1,answer_extent=1
/// | reason=D88 source refused`.
///
/// It lives HERE, in the contract both sides already depend on, because the
/// validator below must recognise it as a field boundary. When the producer
/// owned a private copy, the validator's boundary rule accepted only a comma or
/// end-of-string, so the final unit before this suffix was reported missing --
/// which failed H4FR encoding and aborted the whole capture loop with no wire
/// evidence of why. Two independently-maintained spellings of one delimiter is
/// the drift that caused that; there is now exactly one.
pub const FRAME_REFUSAL_REASON_SUFFIX_SEPARATOR: &str = " | reason=";
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

// ---------------------------------------------------------------------------
// Version 2
// ---------------------------------------------------------------------------

/// One offender, with the structure a diagnosis actually needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRefusalOffenderV2 {
    pub domain_id: u16,
    pub status_flags: u32,
    pub non_tolerated_flags: u32,
    /// WHICH check refused. Never absent: a producer that declared none is
    /// stamped [`Predicate::undeclared`], which is a statement rather than a
    /// silence and is countable in one pass over a stream.
    pub predicate: Predicate,
    /// WHO refused. Often not this domain.
    pub authority: Authority,
    /// Where, as a path of `(ordinal, extent)` levels from outermost in.
    pub locator: Vec<LocatorLevel>,
    /// The typed units that survived the budget, in role order.
    pub units: Vec<DiagUnit>,
    /// Exactly what the budget cost this offender.
    pub truncation: TruncationAccount,
    /// Present when the answer was owed by a derivation that does not run on
    /// the frame.
    pub off_frame: Option<OffFrameOutcome>,
    /// The producer's own text. This is the `Err(String)` that the old
    /// `Err(_) => status |= BIT` arms threw away.
    pub reason: String,
    /// How many of `units`, in role order from the front, the reason names.
    ///
    /// The reason is bounded at [`FRAME_REFUSAL_MAX_REASON_LEN`] and the units
    /// blob at [`FRAME_REFUSAL_MAX_UNITS_LEN`], four times larger, so a
    /// verbose domain's reason cannot name everything it carries. This is that
    /// shortfall as a NUMBER. It is the typed replacement for the literal
    /// `...`: `named < units.len()` is the loss, stated rather than punctuated,
    /// and a consumer that wants the rest reads the blob or calls
    /// [`FrameRefusalOffenderV2::render`].
    pub reason_units_named: u32,
}

impl FrameRefusalOffenderV2 {
    /// Render the offender the way a human reads it in a log line. Deliberately
    /// has no `...` branch: what was dropped is stated numerically.
    pub fn render(&self) -> String {
        let mut parts = vec![
            format!(
                "predicate={}#{}",
                self.predicate.name,
                self.predicate.code.get()
            ),
            self.authority.render(),
        ];
        if !self.locator.is_empty() {
            parts.push(format!(
                "at[{}]",
                self.locator
                    .iter()
                    .map(LocatorLevel::render)
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
        if let Some(off_frame) = &self.off_frame {
            parts.push(off_frame.render());
        }
        if !self.units.is_empty() {
            parts.push(
                self.units
                    .iter()
                    .map(DiagUnit::render)
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        parts.push(self.truncation.render());
        if !self.reason.is_empty() {
            parts.push(format!("reason={}", self.reason));
        }
        // Say how much of the carried detail the reason alone conveys, so a
        // consumer reading only the log line knows whether it is reading all
        // of it. `named=3/24` is the fact the literal `...` was standing in
        // for, and unlike `...` it is also true when nothing was lost.
        parts.push(format!(
            "reason_names={}/{}",
            self.reason_units_named,
            self.units.len()
        ));
        parts.join(" ")
    }
}

/// A reason a frame was refused that belongs to no single domain.
///
/// The frame's own clock, its identity, a framing fault discovered while
/// assembling it. Version 1 could not carry these, and the producer therefore
/// published nothing at all for such an hour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRefusalFinding {
    pub predicate: Predicate,
    pub authority: Authority,
    pub locator: Vec<LocatorLevel>,
    pub reason: String,
}

/// `predicate_code u32 | authority [12] | predicate_name_len u16 |
///  locator_len u16 | reason_len u16 | reserved u16`
pub const FRAME_REFUSAL_FINDING_FIXED_LEN: usize = 24;
/// Bound on frame-level findings. The completeness walk stops on the first
/// framing fault, so a real frame carries very few; this admits a pathological
/// one without admitting an unbounded record.
pub const FRAME_REFUSAL_MAX_FINDINGS: usize = 256;

impl FrameRefusalFinding {
    pub fn render(&self) -> String {
        let mut parts = vec![
            format!(
                "predicate={}#{}",
                self.predicate.name,
                self.predicate.code.get()
            ),
            self.authority.render(),
        ];
        if !self.locator.is_empty() {
            parts.push(format!(
                "at[{}]",
                self.locator
                    .iter()
                    .map(LocatorLevel::render)
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
        parts.push(format!("reason={}", self.reason));
        parts.join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRefusalRecordV2 {
    pub refused_hour: i32,
    pub offenders: Vec<FrameRefusalOffenderV2>,
    /// Reasons that belong to the frame rather than to any domain.
    pub frame_findings: Vec<FrameRefusalFinding>,
}

impl FrameRefusalRecordV2 {
    pub fn encode(&self) -> Result<Vec<u8>, String> {
        // A refusal must still SUBSTANTIATE itself. What changed from version 1
        // is only that a frame-level finding is now an acceptable
        // substantiation; a record that asserts a refusal and says nothing at
        // all is refused exactly as before.
        if self.offenders.is_empty() && self.frame_findings.is_empty() {
            return Err(
                "frame refusal names neither an offending domain nor a frame-level finding"
                    .to_string(),
            );
        }
        if self.offenders.len() > FRAME_REFUSAL_MAX_OFFENDERS {
            return Err(format!(
                "frame refusal names {} offenders, above the {FRAME_REFUSAL_MAX_OFFENDERS} bound",
                self.offenders.len()
            ));
        }
        if self.frame_findings.len() > FRAME_REFUSAL_MAX_FINDINGS {
            return Err(format!(
                "frame refusal carries {} frame findings, above the \
                 {FRAME_REFUSAL_MAX_FINDINGS} bound",
                self.frame_findings.len()
            ));
        }
        let mut out = Vec::with_capacity(FRAME_REFUSAL_HEADER_LEN + self.offenders.len() * 192);
        out.extend_from_slice(FRAME_REFUSAL_MAGIC);
        out.extend_from_slice(&FRAME_REFUSAL_VERSION_V2.to_le_bytes());
        out.extend_from_slice(&(FRAME_REFUSAL_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&self.refused_hour.to_le_bytes());
        out.extend_from_slice(&(self.offenders.len() as u16).to_le_bytes());
        out.extend_from_slice(&(self.frame_findings.len() as u16).to_le_bytes());
        for offender in &self.offenders {
            if offender.non_tolerated_flags == 0 {
                return Err(format!(
                    "frame refusal lists domain {} with no non-tolerated bits",
                    offender.domain_id
                ));
            }
            // A lifted v1 record is a read-side view, not a v2 record: encoding
            // it would stamp declarations (an authority, a truncation account)
            // that the original producer never made.
            if offender.authority == Authority::Undeclared
                || offender.truncation.kind == TruncationKind::Undeclared
            {
                return Err(format!(
                    "frame refusal for domain {} carries an undeclared authority or truncation \
                     account; a record lifted from version 1 cannot be re-encoded as version 2",
                    offender.domain_id
                ));
            }
            // The structural guarantee that a domain cannot report nothing.
            // `predicate.code` is a NonZeroU32 in the type system, so the only
            // way to reach here without a producer declaration is to have been
            // stamped `PREDICATE_UNDECLARED`, and that says so on the wire.
            if offender.predicate.name.is_empty() {
                return Err(format!(
                    "frame refusal gives domain {} a predicate with no name",
                    offender.domain_id
                ));
            }
            if offender.predicate.name.len() > FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN {
                return Err(format!(
                    "frame refusal predicate name for domain {} is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN} bound",
                    offender.domain_id,
                    offender.predicate.name.len()
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
            if offender.truncation.units_emitted > offender.truncation.units_offered {
                return Err(format!(
                    "frame refusal for domain {} claims {} of {} units emitted",
                    offender.domain_id,
                    offender.truncation.units_emitted,
                    offender.truncation.units_offered
                ));
            }
            if offender.units.len() != offender.truncation.units_emitted as usize {
                return Err(format!(
                    "frame refusal for domain {} carries {} units but accounts {}",
                    offender.domain_id,
                    offender.units.len(),
                    offender.truncation.units_emitted
                ));
            }
            // The structural guarantee that the DECIDING fact reaches the field
            // every consumer actually reads.
            //
            // Carrying a unit in the blob and leaving it out of the reason is
            // exactly the failure that started this: the units existed, the
            // reason was what got logged, and the reason had dropped them. So a
            // record that carries units and does not name the first one -- the
            // highest-priority one, because the budget hands units back in role
            // order -- does not encode at all. A producer cannot regress the
            // reason to a bare status word without this refusing.
            check_reason_names_its_units(
                &offender.reason,
                &offender.units,
                offender.reason_units_named,
                &format!("frame refusal for domain {}", offender.domain_id),
            )?;
            // An account that says "complete" while units are missing is the
            // exact lie the literal `...` used to tell by omission.
            if offender.truncation.kind == TruncationKind::None
                && (offender.truncation.units_emitted != offender.truncation.units_offered
                    || offender.truncation.bytes_dropped != 0)
            {
                return Err(format!(
                    "frame refusal for domain {} claims no truncation while dropping {} unit(s) \
                     and {} byte(s)",
                    offender.domain_id,
                    offender.truncation.units_dropped(),
                    offender.truncation.bytes_dropped
                ));
            }
            let locator = encode_locator(&offender.locator)?;
            if locator.len() > FRAME_REFUSAL_MAX_LOCATOR_LEN {
                return Err(format!(
                    "frame refusal locator for domain {} is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_LOCATOR_LEN} bound",
                    offender.domain_id,
                    locator.len()
                ));
            }
            let units = encode_units(&offender.units)?;
            if units.len() > FRAME_REFUSAL_MAX_UNITS_LEN {
                return Err(format!(
                    "frame refusal units for domain {} are {} bytes, above the \
                     {FRAME_REFUSAL_MAX_UNITS_LEN} bound",
                    offender.domain_id,
                    units.len()
                ));
            }
            let entry_flags = if offender.off_frame.is_some() {
                ENTRY_FLAG_OFF_FRAME
            } else {
                0
            };
            out.extend_from_slice(&offender.domain_id.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&offender.status_flags.to_le_bytes());
            out.extend_from_slice(&offender.non_tolerated_flags.to_le_bytes());
            out.extend_from_slice(&offender.predicate.code.get().to_le_bytes());
            out.push(offender.truncation.kind.code());
            // Version 2's spare byte, spent on the reason's unit count. A
            // 512-byte reason cannot name more than 128 units -- the shortest
            // legal render is `x=0` plus a separator -- so the byte always
            // holds it, and the check is here so `encode` stays total rather
            // than relying on that arithmetic staying true.
            let reason_units_named = u8::try_from(offender.reason_units_named).map_err(|_| {
                format!(
                    "frame refusal for domain {} says its reason names {} units, above the \
                     {} the field holds",
                    offender.domain_id,
                    offender.reason_units_named,
                    u8::MAX
                )
            })?;
            out.push(reason_units_named);
            out.extend_from_slice(&(offender.predicate.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&offender.truncation.units_offered.to_le_bytes());
            out.extend_from_slice(&offender.truncation.units_emitted.to_le_bytes());
            out.extend_from_slice(&offender.truncation.bytes_dropped.to_le_bytes());
            out.extend_from_slice(&offender.authority.encode());
            out.extend_from_slice(&entry_flags.to_le_bytes());
            out.extend_from_slice(&(reason.len() as u16).to_le_bytes());
            out.extend_from_slice(&(locator.len() as u16).to_le_bytes());
            out.extend_from_slice(&(units.len() as u16).to_le_bytes());
            out.extend_from_slice(offender.predicate.name.as_bytes());
            out.extend_from_slice(&locator);
            out.extend_from_slice(&units);
            if let Some(off_frame) = &offender.off_frame {
                out.extend_from_slice(&off_frame.encode());
            }
            out.extend_from_slice(reason);
        }
        for finding in &self.frame_findings {
            if finding.predicate.name.is_empty() {
                return Err("frame refusal finding has a predicate with no name".to_string());
            }
            if finding.predicate.name.len() > FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN {
                return Err(format!(
                    "frame refusal finding predicate name is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN} bound",
                    finding.predicate.name.len()
                ));
            }
            let reason = finding.reason.as_bytes();
            if reason.is_empty() {
                return Err("frame refusal finding gives no reason".to_string());
            }
            if reason.len() > FRAME_REFUSAL_MAX_REASON_LEN {
                return Err(format!(
                    "frame refusal finding reason is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_REASON_LEN} bound",
                    reason.len()
                ));
            }
            let locator = encode_locator(&finding.locator)?;
            if locator.len() > FRAME_REFUSAL_MAX_LOCATOR_LEN {
                return Err(format!(
                    "frame refusal finding locator is {} bytes, above the \
                     {FRAME_REFUSAL_MAX_LOCATOR_LEN} bound",
                    locator.len()
                ));
            }
            out.extend_from_slice(&finding.predicate.code.get().to_le_bytes());
            out.extend_from_slice(&finding.authority.encode());
            out.extend_from_slice(&(finding.predicate.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&(locator.len() as u16).to_le_bytes());
            out.extend_from_slice(&(reason.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(finding.predicate.name.as_bytes());
            out.extend_from_slice(&locator);
            out.extend_from_slice(reason);
        }
        Ok(out)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        validate_frame_refusal_v2_payload(payload)?;
        let refused_hour = i32::from_le_bytes([payload[8], payload[9], payload[10], payload[11]]);
        let offender_count = u16::from_le_bytes([payload[12], payload[13]]) as usize;
        let finding_count = u16::from_le_bytes([payload[14], payload[15]]) as usize;
        let mut offenders = Vec::with_capacity(offender_count);
        let mut offset = FRAME_REFUSAL_HEADER_LEN;
        for _ in 0..offender_count {
            let (offender, next) = decode_v2_entry(payload, offset)?;
            offenders.push(offender);
            offset = next;
        }
        let mut frame_findings = Vec::with_capacity(finding_count);
        for _ in 0..finding_count {
            let (finding, next) = decode_v2_finding(payload, offset)?;
            frame_findings.push(finding);
            offset = next;
        }
        Ok(Self {
            refused_hour,
            offenders,
            frame_findings,
        })
    }
}

fn decode_v2_finding(
    payload: &[u8],
    offset: usize,
) -> Result<(FrameRefusalFinding, usize), String> {
    // Bounds already proved by `validate_frame_refusal_v2_payload`.
    let predicate_code = read_u32(payload, offset);
    let authority = Authority::decode(&payload[offset + 4..offset + 16])?;
    let predicate_name_len = read_u16(payload, offset + 16) as usize;
    let locator_len = read_u16(payload, offset + 18) as usize;
    let reason_len = read_u16(payload, offset + 20) as usize;
    let mut cursor = offset + FRAME_REFUSAL_FINDING_FIXED_LEN;
    let predicate_name = core::str::from_utf8(&payload[cursor..cursor + predicate_name_len])
        .map_err(|error| format!("frame refusal finding predicate name is not UTF-8: {error}"))?
        .to_string();
    cursor += predicate_name_len;
    let (locator, locator_used) = decode_locator(&payload[cursor..cursor + locator_len])?;
    if locator_used != locator_len {
        return Err(format!(
            "frame refusal finding locator declares {locator_len} bytes but decodes {locator_used}"
        ));
    }
    cursor += locator_len;
    let reason = core::str::from_utf8(&payload[cursor..cursor + reason_len])
        .map_err(|error| format!("frame refusal finding reason is not UTF-8: {error}"))?
        .to_string();
    cursor += reason_len;
    Ok((
        FrameRefusalFinding {
            predicate: Predicate::new(predicate_code, predicate_name)?,
            authority,
            locator,
            reason,
        },
        cursor,
    ))
}

fn read_u16(payload: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([payload[at], payload[at + 1]])
}

fn read_u32(payload: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([
        payload[at],
        payload[at + 1],
        payload[at + 2],
        payload[at + 3],
    ])
}

/// Is `needle` present in `haystack` as a whole field rather than as the head
/// of a longer one?
///
/// `render_units_bounded` joins renderings with `,` after a producer prefix
/// that ends in a space, so a genuinely named unit is always bounded by one of
/// those on each side. Plain `contains` is not enough: `x=1` occurs inside
/// `x=1_suffix=2`, so a reason that named only the longer unit would pass a
/// substring test for the shorter one.
fn contains_as_a_field(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut from = 0usize;
    while let Some(at) = haystack[from..].find(needle) {
        let start = from + at;
        let end = start + needle.len();
        let before_ok = start == 0
            || haystack[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c == ',' || c == ' ');
        let after_ok = end == haystack.len()
            || haystack[end..].starts_with(',')
            || haystack[end..].starts_with(FRAME_REFUSAL_REASON_SUFFIX_SEPARATOR);
        if before_ok && after_ok {
            return true;
        }
        // Advance by one char boundary, never by one byte: `haystack` is UTF-8
        // and slicing mid-character panics.
        from = start + haystack[start..].chars().next().map_or(1, char::len_utf8);
        if from >= haystack.len() {
            break;
        }
    }
    false
}

/// The one rule both halves of the wire enforce: a reason that claims to name
/// `named` units really does name the first `named` of them, in order.
///
/// This exists as a single function on purpose. It was written twice -- once in
/// `encode`, once nowhere -- and an invariant enforced by the writer and not by
/// the reader is not enforced: a payload that no producer would emit could
/// still be decoded and believed. The `what` argument only shapes the message.
fn check_reason_names_its_units(
    reason: &str,
    units: &[DiagUnit],
    named: u32,
    what: &str,
) -> Result<(), String> {
    if named as usize > units.len() {
        return Err(format!(
            "{what} says its reason names {named} units but carries {}",
            units.len()
        ));
    }
    // No units carried: the count check above already proved `named` is 0,
    // because 0 is the only value not above `units.len()`.
    if units.is_empty() {
        return Ok(());
    }
    if named == 0 {
        return Err(format!(
            "{what} carries {} unit(s) and names none of them in its reason",
            units.len()
        ));
    }
    for (position, unit) in units.iter().take(named as usize).enumerate() {
        let rendered = unit.render();
        if !contains_as_a_field(reason, &rendered) {
            return Err(format!(
                "{what} claims its reason names {named} unit(s) but the reason does not \
                 contain unit {position} {rendered:?}"
            ));
        }
    }
    Ok(())
}

fn decode_v2_entry(
    payload: &[u8],
    offset: usize,
) -> Result<(FrameRefusalOffenderV2, usize), String> {
    // `validate_frame_refusal_v2_payload` has already proved every bound this
    // walk relies on, so the indexing below cannot panic on a payload that
    // reached here. Both entry points call it first.
    let domain_id = read_u16(payload, offset);
    let status_flags = read_u32(payload, offset + 4);
    let non_tolerated_flags = read_u32(payload, offset + 8);
    let predicate_code = read_u32(payload, offset + 12);
    let truncation_kind = TruncationKind::from_code(payload[offset + 16]).ok_or_else(|| {
        format!(
            "frame refusal truncation kind {} is not defined",
            payload[offset + 16]
        )
    })?;
    let predicate_name_len = read_u16(payload, offset + 18) as usize;
    let units_offered = read_u32(payload, offset + 20);
    let units_emitted = read_u32(payload, offset + 24);
    let bytes_dropped = read_u32(payload, offset + 28);
    let authority = Authority::decode(&payload[offset + 32..offset + 44])?;
    let entry_flags = read_u16(payload, offset + 44);
    let reason_len = read_u16(payload, offset + 46) as usize;
    let locator_len = read_u16(payload, offset + 48) as usize;
    let units_len = read_u16(payload, offset + 50) as usize;

    let mut cursor = offset + FRAME_REFUSAL_V2_ENTRY_FIXED_LEN;
    let predicate_name = core::str::from_utf8(&payload[cursor..cursor + predicate_name_len])
        .map_err(|error| format!("frame refusal predicate name is not UTF-8: {error}"))?
        .to_string();
    cursor += predicate_name_len;
    let (locator, locator_used) = decode_locator(&payload[cursor..cursor + locator_len])?;
    if locator_used != locator_len {
        return Err(format!(
            "frame refusal locator for domain {domain_id} declares {locator_len} bytes but \
             decodes {locator_used}"
        ));
    }
    cursor += locator_len;
    let (units, units_used) = decode_units(&payload[cursor..cursor + units_len])?;
    if units_used != units_len {
        return Err(format!(
            "frame refusal units for domain {domain_id} declare {units_len} bytes but decode \
             {units_used}"
        ));
    }
    cursor += units_len;
    let off_frame = if entry_flags & ENTRY_FLAG_OFF_FRAME != 0 {
        let value = OffFrameOutcome::decode(&payload[cursor..cursor + OFF_FRAME_LEN])?;
        cursor += OFF_FRAME_LEN;
        Some(value)
    } else {
        None
    };
    let reason = core::str::from_utf8(&payload[cursor..cursor + reason_len])
        .map_err(|error| format!("frame refusal reason is not UTF-8: {error}"))?
        .to_string();
    cursor += reason_len;

    let predicate = Predicate::new(predicate_code, predicate_name)?;
    Ok((
        FrameRefusalOffenderV2 {
            domain_id,
            status_flags,
            non_tolerated_flags,
            predicate,
            authority,
            locator,
            units,
            truncation: TruncationAccount {
                kind: truncation_kind,
                units_offered,
                units_emitted,
                bytes_dropped,
            },
            off_frame,
            reason,
            reason_units_named: u32::from(payload[offset + 17]),
        },
        cursor,
    ))
}

/// Complete grammar check for version 2. Total over arbitrary bytes; never
/// panics.
pub fn validate_frame_refusal_v2_payload(payload: &[u8]) -> Result<(), String> {
    if payload.len() < FRAME_REFUSAL_HEADER_LEN {
        return Err(format!(
            "frame refusal payload {} byte(s), below the {FRAME_REFUSAL_HEADER_LEN}-byte header",
            payload.len()
        ));
    }
    if &payload[0..4] != FRAME_REFUSAL_MAGIC {
        return Err("frame refusal magic is not H4FR".to_string());
    }
    let version = read_u16(payload, 4);
    if version != FRAME_REFUSAL_VERSION_V2 {
        return Err(format!(
            "frame refusal version {version} is not {FRAME_REFUSAL_VERSION_V2}"
        ));
    }
    let header_len = read_u16(payload, 6) as usize;
    if header_len != FRAME_REFUSAL_HEADER_LEN {
        return Err(format!(
            "frame refusal header length {header_len} is not {FRAME_REFUSAL_HEADER_LEN}"
        ));
    }
    let offender_count = read_u16(payload, 12) as usize;
    let finding_count = read_u16(payload, 14) as usize;
    if offender_count == 0 && finding_count == 0 {
        return Err(
            "frame refusal names neither an offending domain nor a frame-level finding".to_string(),
        );
    }
    if offender_count > FRAME_REFUSAL_MAX_OFFENDERS {
        return Err(format!(
            "frame refusal names {offender_count} offenders, above the \
             {FRAME_REFUSAL_MAX_OFFENDERS} bound"
        ));
    }
    if finding_count > FRAME_REFUSAL_MAX_FINDINGS {
        return Err(format!(
            "frame refusal carries {finding_count} frame findings, above the \
             {FRAME_REFUSAL_MAX_FINDINGS} bound"
        ));
    }
    let mut offset = FRAME_REFUSAL_HEADER_LEN;
    for index in 0..offender_count {
        if offset
            .checked_add(FRAME_REFUSAL_V2_ENTRY_FIXED_LEN)
            .is_none_or(|end| end > payload.len())
        {
            return Err(format!(
                "frame refusal entry {index} is truncated at offset {offset}"
            ));
        }
        let domain_id = read_u16(payload, offset);
        // The writer zeroes this. A reader that ignores it cannot later spend
        // it on anything, because retained streams would already carry
        // arbitrary values in it -- the finding walk below already refuses a
        // nonzero reserved field for exactly this reason.
        if read_u16(payload, offset + 2) != 0 {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has a nonzero reserved field"
            ));
        }
        if read_u32(payload, offset + 8) == 0 {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has no non-tolerated bits"
            ));
        }
        // Zero is reserved for "the producer did not declare", and an entry
        // that does not declare is exactly what this record exists to prevent.
        if read_u32(payload, offset + 12) == 0 {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) declares no predicate"
            ));
        }
        if TruncationKind::from_code(payload[offset + 16]).is_none() {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has truncation kind {}",
                payload[offset + 16]
            ));
        }
        let predicate_name_len = read_u16(payload, offset + 18) as usize;
        if predicate_name_len == 0 {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) has an unnamed predicate"
            ));
        }
        if predicate_name_len > FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN {
            return Err(format!(
                "frame refusal entry {index} predicate name is {predicate_name_len} bytes, above \
                 the {FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN} bound"
            ));
        }
        let units_offered = read_u32(payload, offset + 20);
        let units_emitted = read_u32(payload, offset + 24);
        if units_emitted > units_offered {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) emits {units_emitted} of \
                 {units_offered} units"
            ));
        }
        let bytes_dropped = read_u32(payload, offset + 28);
        if payload[offset + 16] == TruncationKind::None.code()
            && (units_emitted != units_offered || bytes_dropped != 0)
        {
            return Err(format!(
                "frame refusal entry {index} (domain {domain_id}) claims no truncation while \
                 dropping {} unit(s) and {bytes_dropped} byte(s)",
                units_offered.saturating_sub(units_emitted)
            ));
        }
        // Read here, checked further down against the DECODED units by the
        // same `check_reason_names_its_units` the encoder calls. The count
        // arithmetic deliberately is NOT repeated here: a second copy is a
        // second place for the reader's rule to drift out of step with the
        // writer's, which is the defect this seam exists to prevent.
        let reason_units_named = u32::from(payload[offset + 17]);
        Authority::decode(&payload[offset + 32..offset + 44])?;
        let entry_flags = read_u16(payload, offset + 44);
        if entry_flags & !ENTRY_FLAG_OFF_FRAME != 0 {
            return Err(format!(
                "frame refusal entry {index} has unknown entry flags {entry_flags:#06x}"
            ));
        }
        let reason_len = read_u16(payload, offset + 46) as usize;
        let locator_len = read_u16(payload, offset + 48) as usize;
        let units_len = read_u16(payload, offset + 50) as usize;
        if reason_len == 0 {
            return Err(format!("frame refusal entry {index} gives no reason"));
        }
        if reason_len > FRAME_REFUSAL_MAX_REASON_LEN {
            return Err(format!(
                "frame refusal entry {index} reason is {reason_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_REASON_LEN} bound"
            ));
        }
        if locator_len > FRAME_REFUSAL_MAX_LOCATOR_LEN {
            return Err(format!(
                "frame refusal entry {index} locator is {locator_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_LOCATOR_LEN} bound"
            ));
        }
        if units_len > FRAME_REFUSAL_MAX_UNITS_LEN {
            return Err(format!(
                "frame refusal entry {index} units are {units_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_UNITS_LEN} bound"
            ));
        }
        let off_frame_len = if entry_flags & ENTRY_FLAG_OFF_FRAME != 0 {
            OFF_FRAME_LEN
        } else {
            0
        };
        let Some(end) = offset
            .checked_add(FRAME_REFUSAL_V2_ENTRY_FIXED_LEN)
            .and_then(|value| value.checked_add(predicate_name_len))
            .and_then(|value| value.checked_add(locator_len))
            .and_then(|value| value.checked_add(units_len))
            .and_then(|value| value.checked_add(off_frame_len))
            .and_then(|value| value.checked_add(reason_len))
        else {
            return Err(format!("frame refusal entry {index} length overflows"));
        };
        if end > payload.len() {
            return Err(format!("frame refusal entry {index} runs past the payload"));
        }
        // Every variable-length section must decode against its declared
        // length before the entry decoder indexes into it.
        let mut cursor = offset + FRAME_REFUSAL_V2_ENTRY_FIXED_LEN;
        if core::str::from_utf8(&payload[cursor..cursor + predicate_name_len]).is_err() {
            return Err(format!(
                "frame refusal entry {index} predicate name is not UTF-8"
            ));
        }
        cursor += predicate_name_len;
        let (_, locator_used) = decode_locator(&payload[cursor..cursor + locator_len])?;
        if locator_used != locator_len {
            return Err(format!(
                "frame refusal entry {index} locator declares {locator_len} bytes but decodes \
                 {locator_used}"
            ));
        }
        cursor += locator_len;
        let (units, units_used) = decode_units(&payload[cursor..cursor + units_len])?;
        if units_used != units_len {
            return Err(format!(
                "frame refusal entry {index} units declare {units_len} bytes but decode \
                 {units_used}"
            ));
        }
        if units.len() != units_emitted as usize {
            return Err(format!(
                "frame refusal entry {index} carries {} units but accounts {units_emitted}",
                units.len()
            ));
        }
        cursor += units_len;
        if off_frame_len != 0 {
            OffFrameOutcome::decode(&payload[cursor..cursor + OFF_FRAME_LEN])?;
            cursor += OFF_FRAME_LEN;
        }
        let Ok(reason) = core::str::from_utf8(&payload[cursor..cursor + reason_len]) else {
            return Err(format!("frame refusal entry {index} reason is not UTF-8"));
        };
        // The same rule `encode` applies, applied to what actually arrived.
        // Without this the writer refuses a reason that names nothing while the
        // reader accepts one, and a hand-built or corrupted payload hands a
        // consumer a `reason_units_named` it has no way to check.
        check_reason_names_its_units(
            reason,
            &units,
            reason_units_named,
            &format!("frame refusal entry {index} (domain {domain_id})"),
        )?;
        offset = end;
    }
    for index in 0..finding_count {
        if offset
            .checked_add(FRAME_REFUSAL_FINDING_FIXED_LEN)
            .is_none_or(|end| end > payload.len())
        {
            return Err(format!(
                "frame refusal finding {index} is truncated at offset {offset}"
            ));
        }
        if read_u32(payload, offset) == 0 {
            return Err(format!(
                "frame refusal finding {index} declares no predicate"
            ));
        }
        Authority::decode(&payload[offset + 4..offset + 16])?;
        let predicate_name_len = read_u16(payload, offset + 16) as usize;
        let locator_len = read_u16(payload, offset + 18) as usize;
        let reason_len = read_u16(payload, offset + 20) as usize;
        if read_u16(payload, offset + 22) != 0 {
            return Err(format!(
                "frame refusal finding {index} has a nonzero reserved field"
            ));
        }
        if predicate_name_len == 0 {
            return Err(format!(
                "frame refusal finding {index} has an unnamed predicate"
            ));
        }
        if predicate_name_len > FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN {
            return Err(format!(
                "frame refusal finding {index} predicate name is {predicate_name_len} bytes, \
                 above the {FRAME_REFUSAL_MAX_PREDICATE_NAME_LEN} bound"
            ));
        }
        if reason_len == 0 {
            return Err(format!("frame refusal finding {index} gives no reason"));
        }
        if reason_len > FRAME_REFUSAL_MAX_REASON_LEN {
            return Err(format!(
                "frame refusal finding {index} reason is {reason_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_REASON_LEN} bound"
            ));
        }
        if locator_len > FRAME_REFUSAL_MAX_LOCATOR_LEN {
            return Err(format!(
                "frame refusal finding {index} locator is {locator_len} bytes, above the \
                 {FRAME_REFUSAL_MAX_LOCATOR_LEN} bound"
            ));
        }
        let Some(end) = offset
            .checked_add(FRAME_REFUSAL_FINDING_FIXED_LEN)
            .and_then(|value| value.checked_add(predicate_name_len))
            .and_then(|value| value.checked_add(locator_len))
            .and_then(|value| value.checked_add(reason_len))
        else {
            return Err(format!("frame refusal finding {index} length overflows"));
        };
        if end > payload.len() {
            return Err(format!(
                "frame refusal finding {index} runs past the payload"
            ));
        }
        let mut cursor = offset + FRAME_REFUSAL_FINDING_FIXED_LEN;
        if core::str::from_utf8(&payload[cursor..cursor + predicate_name_len]).is_err() {
            return Err(format!(
                "frame refusal finding {index} predicate name is not UTF-8"
            ));
        }
        cursor += predicate_name_len;
        let (_, locator_used) = decode_locator(&payload[cursor..cursor + locator_len])?;
        if locator_used != locator_len {
            return Err(format!(
                "frame refusal finding {index} locator declares {locator_len} bytes but decodes \
                 {locator_used}"
            ));
        }
        cursor += locator_len;
        if core::str::from_utf8(&payload[cursor..cursor + reason_len]).is_err() {
            return Err(format!("frame refusal finding {index} reason is not UTF-8"));
        }
        offset = end;
    }
    if offset != payload.len() {
        return Err(format!(
            "frame refusal payload has {} trailing byte(s) after {offender_count} entries and \
             {finding_count} finding(s)",
            payload.len() - offset
        ));
    }
    Ok(())
}

/// The version a well-formed-enough payload DECLARES, before any grammar check.
///
/// `None` means the bytes are not a refusal record at all -- too short to hold
/// a version, or not bearing the magic -- and the caller hands them to the v1
/// validator, whose magic/length errors are the right diagnosis for that.
///
/// Public because [`decode_frame_refusal_any_version`] deliberately erases the
/// distinction -- it lifts v1 into the v2 shape -- and a consumer that REPORTS
/// what it decoded needs it back. The lift fills `authority` and `truncation`
/// with `SelfDomain` and `complete(0)`, which render as the positive assertions
/// `authority=self` and `units=0/0 complete`; a version 1 record asserted
/// neither, so a report that shows that rendering without saying the record was
/// version 1 is attributing declarations to a producer that made none.
pub fn declared_frame_refusal_version(payload: &[u8]) -> Option<u16> {
    if payload.len() >= 6 && &payload[0..4] == FRAME_REFUSAL_MAGIC {
        Some(read_u16(payload, 4))
    } else {
        None
    }
}

/// The refusal a reader owes a version it was never taught.
///
/// A future version 3 is NOT a v1 record, so reporting it as "version 3 is not
/// 1" blames the wrong fact: it reads as corruption when what happened is that
/// the producer is newer than the reader. Naming every version this build
/// decodes turns that into an actionable statement, and it stays an `Err` --
/// an unknown shape is never skipped, never lifted to a default, and never
/// counted as a decoded refusal.
fn unknown_frame_refusal_version(version: u16) -> String {
    format!(
        "frame refusal version {version} is not a version this build decodes \
         ({FRAME_REFUSAL_VERSION} or {FRAME_REFUSAL_VERSION_V2}); the record was written by a \
         newer producer and cannot be read here"
    )
}

/// Version-dispatching grammar check.
///
/// A consumer that only ever called the v1 validator would report a v2 record
/// as corrupt; a consumer that only called v2 would report every retained v1
/// record as corrupt. Both versions are legal on the wire, and this is the one
/// entry point that says so.
pub fn validate_frame_refusal_payload_any_version(payload: &[u8]) -> Result<(), String> {
    match declared_frame_refusal_version(payload) {
        Some(version) if version == FRAME_REFUSAL_VERSION_V2 => {
            validate_frame_refusal_v2_payload(payload)
        }
        Some(version) if version != FRAME_REFUSAL_VERSION => {
            Err(unknown_frame_refusal_version(version))
        }
        _ => validate_frame_refusal_payload(payload),
    }
}

/// Either version, decoded into the v2 shape.
///
/// A v1 record has no predicate, no authority, no locator, no typed units and
/// no truncation accounting, so it lifts to `PREDICATE_UNDECLARED` /
/// [`Authority::Undeclared`] / empty / [`TruncationAccount::undeclared`] with
/// its whole reason string preserved. That is an honest lift: every absent
/// fact is stated as absent — [`FrameRefusalOffenderV2::render`] on a lifted
/// offender says `authority=undeclared` and `units=undeclared`, never a
/// default that looks like a declaration. A lifted record is a read-side view
/// only; [`FrameRefusalRecordV2::encode`] refuses it rather than manufacture
/// v2 declarations for facts v1 never carried.
pub fn decode_frame_refusal_any_version(payload: &[u8]) -> Result<FrameRefusalRecordV2, String> {
    match declared_frame_refusal_version(payload) {
        Some(version) if version == FRAME_REFUSAL_VERSION_V2 => {
            return FrameRefusalRecordV2::decode(payload)
        }
        Some(version) if version != FRAME_REFUSAL_VERSION => {
            return Err(unknown_frame_refusal_version(version))
        }
        _ => {}
    }
    let legacy = FrameRefusalRecordV1::decode(payload)?;
    Ok(FrameRefusalRecordV2 {
        refused_hour: legacy.refused_hour,
        offenders: legacy
            .offenders
            .into_iter()
            .map(|offender| FrameRefusalOffenderV2 {
                domain_id: offender.domain_id,
                status_flags: offender.status_flags,
                non_tolerated_flags: offender.non_tolerated_flags,
                predicate: Predicate::undeclared(),
                authority: Authority::Undeclared,
                locator: Vec::new(),
                units: Vec::new(),
                truncation: TruncationAccount::undeclared(),
                off_frame: None,
                reason: offender.reason,
                // A version 1 entry carried no units, so its reason names
                // none. That is a true statement about retained telemetry,
                // not a gap: the upgrade must not invent a count.
                reason_units_named: 0,
            })
            .collect(),
        frame_findings: Vec::new(),
    })
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
            return Err(format!(
                "frame refusal entry {index} reason length overflows"
            ));
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
            validate_compact_record_semantics, COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
        };
        let wire = sample().encode().unwrap();

        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &wire,
        )
        .expect("the new type must be wired into the canonical dispatch");

        // Wrong envelope domain is refused: this is a FRAME-level record.
        assert!(
            validate_compact_record_semantics(47, COMPACT_RECORD_TYPE_FRAME_REFUSED, 0, &wire)
                .is_err(),
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

    // -----------------------------------------------------------------------
    // Version 2
    // -----------------------------------------------------------------------

    use crate::diagnostic_channel::{
        allocate_unit_budget, render_units_bounded, BudgetRequest, DiagUnit, OffFrameTerminal,
        UnitPresence, UnitRole, PREDICATE_UNDECLARED,
    };

    /// D42's measured shape, as it would be reported through the new channel.
    fn d42_offender() -> FrameRefusalOffenderV2 {
        FrameRefusalOffenderV2 {
            domain_id: 42,
            status_flags: 0x0000_0100,
            non_tolerated_flags: 0x0000_0100,
            predicate: Predicate::new(4216, "region_object_vtable_matches").unwrap(),
            authority: Authority::Engine { rva: 0x2A19_2A8 },
            locator: vec![
                LocatorLevel::new("country_container", 3, Some(12)),
                LocatorLevel::new("region", 41, Some(96)).with_address(0x2CE6_1D8A_0FF0),
            ],
            units: vec![
                DiagUnit::deciding("d42_region_walk_predicate", 4216),
                DiagUnit::locator("d42_region_walk_address", 0x2CE6_1D8A_0FF0),
            ],
            truncation: TruncationAccount::complete(2),
            off_frame: None,
            reason: "region object vtable echo did not match the module base: \
                     d42_region_walk_predicate=4216,d42_region_walk_address=49366849687536"
                .to_string(),
            reason_units_named: 2,
        }
    }

    /// D44's measured shape: the deciding fields are ZERO and a sibling field
    /// still holds the reset sentinel.
    fn d44_offender() -> FrameRefusalOffenderV2 {
        FrameRefusalOffenderV2 {
            domain_id: 44,
            status_flags: 0x0000_0200,
            non_tolerated_flags: 0x0000_0200,
            predicate: Predicate::new(4401, "country_supply_slot_is_readable").unwrap(),
            authority: Authority::SelfDomain,
            locator: vec![LocatorLevel::new("country_iteration", 3, Some(350))],
            units: vec![
                DiagUnit::new(
                    "d44_country_supply_failure_stage",
                    0,
                    UnitRole::Deciding,
                    UnitPresence::Observed,
                ),
                DiagUnit::new(
                    "d44_country_supply_failure_branch",
                    0,
                    UnitRole::Deciding,
                    UnitPresence::Observed,
                ),
                DiagUnit::new(
                    "d44_country_supply_slot_index",
                    0xFFFF_FFFF,
                    UnitRole::Locator,
                    UnitPresence::ResetSentinel,
                ),
            ],
            truncation: TruncationAccount::complete(3),
            off_frame: Some(OffFrameOutcome {
                spawn_generation: 7,
                elapsed_us: 51_000_000,
                deadline_us: 2_000_000,
                terminal: OffFrameTerminal::Running,
            }),
            reason: "country supply slot read refused: \
                     d44_country_supply_failure_stage=0,d44_country_supply_failure_branch=0"
                .to_string(),
            // Three units carried, two named: the sentinel locator did not fit
            // the fixture's reason. That shortfall is a NUMBER on the wire, and
            // the assertions below read it as one.
            reason_units_named: 2,
        }
    }

    fn sample_v2() -> FrameRefusalRecordV2 {
        FrameRefusalRecordV2 {
            refused_hour: 60_831_436,
            offenders: vec![d42_offender(), d44_offender()],
            frame_findings: vec![FrameRefusalFinding {
                predicate: Predicate::new(0x0001_0002, "record_hour_label_matches_frame_hour")
                    .unwrap(),
                authority: Authority::Host,
                locator: vec![LocatorLevel::new("record", 12, Some(147))],
                reason: "domain 18 record_type 1: game hour 60830968 != frame hour 60831436"
                    .to_string(),
            }],
        }
    }

    #[test]
    fn v2_round_trips_with_predicate_authority_locator_units_and_truncation_preserved() {
        let value = sample_v2();
        let wire = value.encode().unwrap();
        validate_frame_refusal_v2_payload(&wire).unwrap();
        assert_eq!(FrameRefusalRecordV2::decode(&wire).unwrap(), value);
        // And through the version-dispatching entry points, which is what a
        // consumer actually calls.
        validate_frame_refusal_payload_any_version(&wire).unwrap();
        assert_eq!(decode_frame_refusal_any_version(&wire).unwrap(), value);
    }

    /// The zero-valued deciding units and the reset sentinel all survive the
    /// trip, and a consumer can tell all three apart.
    #[test]
    fn a_zero_deciding_value_and_a_reset_sentinel_survive_and_stay_distinguishable() {
        let wire = sample_v2().encode().unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        let d44 = back.offenders.iter().find(|o| o.domain_id == 44).unwrap();
        let stage = d44
            .units
            .iter()
            .find(|unit| unit.name == "d44_country_supply_failure_stage")
            .expect("a zero-valued deciding unit must reach the wire");
        assert_eq!(stage.value, 0);
        assert_eq!(stage.presence, UnitPresence::Observed);
        assert!(stage.presence.value_is_a_measurement());
        let slot = d44
            .units
            .iter()
            .find(|unit| unit.name == "d44_country_supply_slot_index")
            .unwrap();
        assert_eq!(slot.value, 0xFFFF_FFFF);
        assert_eq!(slot.presence, UnitPresence::ResetSentinel);
        assert!(
            !slot.presence.value_is_a_measurement(),
            "0xFFFFFFFF must not read as an index"
        );
        assert_eq!(
            slot.render(),
            "d44_country_supply_slot_index=sentinel(0xffffffff)"
        );
    }

    #[test]
    fn v2_names_the_predicate_the_authority_and_an_ordinal_with_its_extent() {
        let d42 = d42_offender();
        let rendered = d42.render();
        assert!(
            rendered.contains("predicate=region_object_vtable_matches#4216"),
            "{rendered}"
        );
        assert!(
            rendered.contains("authority=engine@rva=0x2a192a8"),
            "{rendered}"
        );
        assert!(rendered.contains("country_container 3 of 12"), "{rendered}");
        assert!(rendered.contains("region 41 of 96"), "{rendered}");
        assert!(!rendered.contains("..."), "no literal ellipsis: {rendered}");
        // The authority is not this domain when a different domain refused.
        let d91 = FrameRefusalOffenderV2 {
            authority: Authority::OtherDomain(69),
            ..d42
        };
        assert!(d91.render().contains("authority=domain69"));
    }

    #[test]
    fn an_off_frame_outcome_states_elapsed_against_its_deadline() {
        let d44 = d44_offender();
        let rendered = d44.render();
        assert!(rendered.contains("off_frame=running"), "{rendered}");
        assert!(rendered.contains("elapsed=51000000us"), "{rendered}");
        assert!(rendered.contains("deadline=2000000us"), "{rendered}");
        assert!(rendered.contains("OVERDUE"), "{rendered}");
        assert!(rendered.contains("gen=7"), "{rendered}");
    }

    /// The structural bar: a refusal cannot be encoded, and cannot be decoded,
    /// without naming a predicate.
    #[test]
    fn an_entry_that_declares_no_predicate_is_refused_on_the_wire() {
        let wire = sample_v2().encode().unwrap();
        let mut bad = wire.clone();
        // predicate_code sits at entry offset +12.
        for byte in 0..4 {
            bad[FRAME_REFUSAL_HEADER_LEN + 12 + byte] = 0;
        }
        let error = validate_frame_refusal_v2_payload(&bad).unwrap_err();
        assert!(error.contains("declares no predicate"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());
        // And the reserved "undeclared" code IS accepted, because saying "this
        // producer declared nothing" out loud is the point.
        let undeclared = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders: vec![FrameRefusalOffenderV2 {
                predicate: Predicate::undeclared(),
                ..d42_offender()
            }],
            frame_findings: Vec::new(),
        };
        let wire = undeclared.encode().unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        assert!(back.offenders[0].predicate.is_undeclared());
        assert_eq!(back.offenders[0].predicate.code.get(), PREDICATE_UNDECLARED);
    }

    /// An account that claims completeness while units are missing is the lie
    /// the literal `...` used to tell by omission. Both halves refuse it.
    #[test]
    fn a_truncation_account_that_contradicts_the_carried_units_is_refused() {
        let mut value = sample_v2();
        value.offenders[0].truncation.units_offered = 9;
        // kind is still None while 9 were offered and 2 emitted.
        let error = value.encode().unwrap_err();
        assert!(error.contains("claims no truncation"), "{error}");

        let mut value = sample_v2();
        value.offenders[0].truncation.units_emitted = 1;
        let error = value.encode().unwrap_err();
        assert!(error.contains("carries 2 units but accounts 1"), "{error}");

        let mut value = sample_v2();
        value.offenders[0].truncation.units_emitted = 99;
        assert!(value.encode().is_err());

        // On the wire too: flip units_emitted under a valid record.
        let wire = sample_v2().encode().unwrap();
        let mut bad = wire.clone();
        bad[FRAME_REFUSAL_HEADER_LEN + 24] = 1;
        assert!(validate_frame_refusal_v2_payload(&bad).is_err());
    }

    /// The reason is the field consumers log. A record that carries units and
    /// does not name the highest-priority one in its reason is the 192-byte
    /// channel this lane replaced, and it must not be encodable.
    ///
    /// Production change that makes this fail: deleting the
    /// `reason.contains(&first.render())` arm from `encode`, or letting a
    /// producer build the reason without calling `render_units_bounded`.
    #[test]
    fn a_reason_that_names_none_of_its_carried_units_cannot_be_encoded() {
        let mut value = sample_v2();
        value.offenders[0].reason = "region walk refused".to_string();
        let error = value.encode().unwrap_err();
        assert!(error.contains("does not contain unit 0"), "{error}");

        // Claiming zero named while carrying units is refused as its own shape,
        // so a producer cannot opt out by setting the count honestly to zero.
        let mut value = sample_v2();
        value.offenders[0].reason_units_named = 0;
        let error = value.encode().unwrap_err();
        assert!(error.contains("names none of them"), "{error}");

        // And a count above what is carried is refused rather than clamped.
        let mut value = sample_v2();
        value.offenders[0].reason_units_named = 99;
        let error = value.encode().unwrap_err();
        assert!(error.contains("but carries 2"), "{error}");

        // The mirror case: an offender carrying NO units cannot claim to have
        // named one. Without this, "names 1" and "carries 0" coexist and a
        // consumer counting named units across a stream over-counts. The same
        // count check catches it, which is why the contract needs no separate
        // empty-units arm.
        let mut value = sample_v2();
        value.offenders[0].units.clear();
        value.offenders[0].truncation = TruncationAccount::complete(0);
        value.offenders[0].reason_units_named = 1;
        let error = value.encode().unwrap_err();
        assert!(error.contains("names 1 units but carries 0"), "{error}");
    }

    /// The decoder enforces the same rule, so a corrupted or hand-built payload
    /// cannot hand a consumer a reason it has no way to tell is incomplete.
    ///
    /// Production change that makes this fail: removing the `reason_units_named`
    /// arms from `validate_frame_refusal_v2_payload`.
    #[test]
    fn a_forged_reason_unit_count_is_refused_on_the_way_in() {
        let wire = sample_v2().encode().unwrap();
        // The count sits at entry offset +17, the byte version 2 spends on it.
        const AT: usize = FRAME_REFUSAL_HEADER_LEN + 17;
        assert_eq!(wire[AT], 2, "the fixture names both of its units");
        FrameRefusalRecordV2::decode(&wire).unwrap();

        let mut bad = wire.clone();
        bad[AT] = 3; // above units_emitted
        let error = validate_frame_refusal_v2_payload(&bad).unwrap_err();
        assert!(error.contains("but carries 2"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());

        let mut bad = wire.clone();
        bad[AT] = 0; // carries units, names none
        let error = validate_frame_refusal_v2_payload(&bad).unwrap_err();
        assert!(error.contains("names none of them"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());

        // And the count survives the round trip as a value.
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        assert_eq!(back.offenders[0].reason_units_named, 2);
        assert!(back.offenders[0].render().contains("reason_names=2/2"));
    }

    /// The reader enforces the reason rule too. An invariant the writer applies
    /// and the reader does not is not enforced: a hand-built or corrupted
    /// payload could carry a `reason_units_named` naming units the reason never
    /// mentions, and no consumer could tell.
    ///
    /// Production change that makes this fail: dropping the
    /// `check_reason_names_its_units` call from
    /// `validate_frame_refusal_v2_payload`.
    #[test]
    fn a_payload_whose_reason_names_units_it_does_not_contain_is_refused_on_the_way_in() {
        // Build a legal record, then overwrite the reason bytes in place with a
        // same-length string that names nothing. Length-preserving, so every
        // other offset and every other check still passes.
        let record = sample_v2();
        let wire = record.encode().unwrap();
        let original = &record.offenders[0].reason;
        let at = wire
            .windows(original.len())
            .position(|window| window == original.as_bytes())
            .expect("the reason is on the wire verbatim");
        let mut bad = wire.clone();
        let filler = "x".repeat(original.len());
        bad[at..at + original.len()].copy_from_slice(filler.as_bytes());

        let error = validate_frame_refusal_v2_payload(&bad).unwrap_err();
        assert!(error.contains("does not contain unit 0"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());
    }

    /// `contains` is not the right test, and the collision is ordinary rather
    /// than contrived: any counter at 1 renders as a prefix of the same counter
    /// at 10. A reason that named only the larger reading would pass a
    /// substring check for the smaller one and claim a fact it never stated.
    ///
    /// Production change that makes this fail: replacing
    /// `contains_as_a_field` with `str::contains`.
    #[test]
    fn a_unit_named_only_as_the_head_of_a_longer_one_does_not_count_as_named() {
        let short = DiagUnit::deciding("d44_stage", 1);
        let long = DiagUnit::deciding("d44_stage", 10);
        // The long unit's rendering literally contains the short one's.
        assert!(long.render().contains(&short.render()));

        // A reason that names ONLY the long unit must not satisfy a claim to
        // have named the short one.
        assert!(check_reason_names_its_units(
            &format!("domain 44: {}", long.render()),
            core::slice::from_ref(&short),
            1,
            "test",
        )
        .is_err());
        // And the honest reason does satisfy it, at either end and in the middle.
        for reason in [
            format!("domain 44: {}", short.render()),
            format!("domain 44: {},{}", short.render(), long.render()),
            format!("domain 44: {},{}", long.render(), short.render()),
        ] {
            check_reason_names_its_units(&reason, core::slice::from_ref(&short), 1, "test")
                .unwrap_or_else(|error| panic!("{reason:?}: {error}"));
        }
    }

    /// A claim to name N units is checked against all N, not only the first.
    ///
    /// Production change that makes this fail: checking only `units[0]` in
    /// `check_reason_names_its_units`.
    #[test]
    fn every_unit_the_count_claims_must_actually_appear() {
        let units = vec![
            DiagUnit::deciding("d44_stage", 0),
            DiagUnit::deciding("d44_branch", 0),
            DiagUnit::counter("d44_rows", 10_272),
        ];
        let honest = format!(
            "domain 44: {},{}",
            units[0].render(),
            units[1].render()
        );
        check_reason_names_its_units(&honest, &units, 2, "test").unwrap();
        // Claiming three while naming two is refused, and the message says
        // WHICH one is missing rather than just that something is.
        let error = check_reason_names_its_units(&honest, &units, 3, "test").unwrap_err();
        assert!(error.contains("does not contain unit 2"), "{error}");
    }

    /// Production offender reasons append the producer's original diagnostic
    /// after the rendered unit prefix. The final named unit is therefore
    /// followed by ` | reason=`, not by a comma or end-of-string, and remains a
    /// complete field rather than a substring of a longer unit.
    #[test]
    fn a_final_named_unit_before_the_producer_reason_suffix_counts_as_named() {
        let units = vec![
            DiagUnit::deciding("read_completion", 1),
            DiagUnit::deciding("answer_extent", 1),
        ];
        let reason = format!(
            "domain 88: {},{} | reason=D88 source refused",
            units[0].render(),
            units[1].render()
        );

        check_reason_names_its_units(&reason, &units, 2, "test").unwrap();

        let longer = format!(
            "domain 88: {},{}_suffix=2 | reason=D88 source refused",
            units[0].render(),
            units[1].render()
        );
        assert!(check_reason_names_its_units(&longer, &units, 2, "test").is_err());
    }

    /// The offender entry's reserved `u16` must be zero, so a later version can
    /// still spend it. The finding walk already refused a nonzero reserved
    /// field; the offender walk did not, which meant retained streams could
    /// have accumulated arbitrary values there.
    ///
    /// Production change that makes this fail: dropping the `offset + 2` check
    /// from `validate_frame_refusal_v2_payload`.
    #[test]
    fn a_nonzero_reserved_field_in_an_offender_entry_is_refused() {
        let wire = sample_v2().encode().unwrap();
        validate_frame_refusal_v2_payload(&wire).unwrap();
        let mut bad = wire.clone();
        bad[FRAME_REFUSAL_HEADER_LEN + 2] = 1;
        let error = validate_frame_refusal_v2_payload(&bad).unwrap_err();
        assert!(error.contains("nonzero reserved field"), "{error}");
        assert!(FrameRefusalRecordV2::decode(&bad).is_err());
    }

    /// A domain far more verbose than the reason bound still encodes, still
    /// names its deciding unit first, and states the shortfall as a number.
    ///
    /// This is the pair of failures that used to be mutually exclusive: joining
    /// every unit blew `FRAME_REFUSAL_MAX_REASON_LEN` and made the record
    /// unencodable, and dropping the units made the reason say nothing.
    ///
    /// Production change that makes this fail: removing the length test from
    /// `render_units_bounded`, or dropping the role-ordered sort from
    /// `allocate_unit_budget` so the deciding unit is no longer first.
    #[test]
    fn a_verbose_domain_names_its_deciding_unit_and_states_what_the_reason_left_out() {
        let mut units: Vec<DiagUnit> = (0..600u64)
            .map(|index| DiagUnit::counter(format!("d44_counter_{index:04}"), index))
            .collect();
        // Deciding unit built LAST, as D44's was, and zero-valued.
        units.push(DiagUnit::deciding("d44_country_supply_failure_stage", 0));
        let grants = allocate_unit_budget(
            &[BudgetRequest {
                domain_id: 44,
                units,
            }],
            4_096,
            FRAME_REFUSAL_MAX_UNITS_LEN,
        );
        let grant = &grants[0];
        let prefix = "domain 44: ";
        let (rendered, named) = render_units_bounded(
            &grant.units,
            FRAME_REFUSAL_MAX_REASON_LEN.saturating_sub(prefix.len()),
        );
        let reason = format!("{prefix}{rendered}");

        assert!(
            reason.len() <= FRAME_REFUSAL_MAX_REASON_LEN,
            "reason is {} bytes",
            reason.len()
        );
        assert!(
            reason.contains("d44_country_supply_failure_stage=0"),
            "the deciding unit must be named first: {reason}"
        );
        assert!(named >= 1 && (named as usize) < grant.units.len());
        assert!(!reason.contains("..."), "no literal ellipsis: {reason}");

        let record = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders: vec![FrameRefusalOffenderV2 {
                domain_id: 44,
                status_flags: 0x0000_0200,
                non_tolerated_flags: 0x0000_0200,
                predicate: Predicate::new(4401, "country_supply_slot_is_readable").unwrap(),
                authority: Authority::SelfDomain,
                locator: Vec::new(),
                units: grant.units.clone(),
                truncation: grant.account,
                off_frame: None,
                reason,
                reason_units_named: named,
            }],
            frame_findings: Vec::new(),
        };
        // The whole point: it ENCODES. A reason above the bound refuses the
        // record, and a refused record is an hour that publishes nothing.
        let wire = record.encode().unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        let offender = &back.offenders[0];
        assert_eq!(offender.reason_units_named, named);
        assert!(offender
            .reason
            .contains("d44_country_supply_failure_stage=0"));
        assert_eq!(offender.units[0].name, "d44_country_supply_failure_stage");
        // The shortfall is readable as a number from the record alone.
        assert!(
            (offender.reason_units_named as usize) < offender.units.len(),
            "this fixture is only meaningful while the reason cannot name everything"
        );
        assert!(offender.render().contains(&format!(
            "reason_names={}/{}",
            offender.reason_units_named,
            offender.units.len()
        )));
    }

    /// PRODUCTION CHANGE THAT MAKES THIS FAIL: reverting the lift to stamp
    /// `Authority::SelfDomain` or `TruncationAccount::complete(0)` (each is an
    /// assertion no v1 producer made), rendering either as anything but an
    /// explicit `undeclared`, or letting `encode()` serialize a lifted record.
    #[test]
    fn a_v1_record_lifts_to_v2_as_explicitly_undeclared_rather_than_as_a_default() {
        let legacy = sample().encode().unwrap();
        validate_frame_refusal_payload_any_version(&legacy).unwrap();
        let lifted = decode_frame_refusal_any_version(&legacy).unwrap();
        assert_eq!(lifted.refused_hour, sample().refused_hour);
        for (index, offender) in lifted.offenders.iter().enumerate() {
            assert!(
                offender.predicate.is_undeclared(),
                "a v1 record carried no predicate and must say so, not invent one"
            );
            assert_eq!(
                offender.authority,
                Authority::Undeclared,
                "a v1 record carried no authority and must say so, not claim self"
            );
            assert!(offender.locator.is_empty());
            assert!(offender.units.is_empty());
            assert_eq!(
                offender.truncation.kind,
                TruncationKind::Undeclared,
                "a v1 record carried no truncation accounting and must say so, not claim complete"
            );
            assert_eq!(offender.reason, sample().offenders[index].reason);

            // The render a log line or a consumer sees states each absence and
            // asserts nothing a v1 producer never said.
            let rendered = offender.render();
            assert!(rendered.contains("authority=undeclared"), "{rendered}");
            assert!(rendered.contains("units=undeclared"), "{rendered}");
            assert!(!rendered.contains("authority=self"), "{rendered}");
            assert!(!rendered.contains("complete"), "{rendered}");
        }

        // A lifted record is a read-side view. Re-encoding it as v2 would
        // manufacture declarations, so the encoder refuses it by name.
        let error = lifted.encode().unwrap_err();
        assert!(
            error.contains("cannot be re-encoded as version 2"),
            "{error}"
        );
    }

    /// A version this build was never taught is REFUSED, and the refusal says
    /// which fact it is refusing on.
    ///
    /// The dispatcher used to fall through to the v1 validator for anything
    /// that was not v2, so a version 3 record was reported as `version 3 is
    /// not 1` -- which blames the record for being corrupt when what actually
    /// happened is that the producer is newer than the reader, and which does
    /// not say that version 2 is also decodable here.
    ///
    /// PRODUCTION CHANGE THAT MAKES THIS FAIL: adding a silent fall-through
    /// (accepting an unknown version as v1, or returning `Ok`) to either
    /// version-dispatching entry point, or dropping the known-version list
    /// from the message.
    #[test]
    fn a_version_neither_entry_point_knows_is_refused_and_names_the_versions_it_does_know() {
        for known in [sample().encode().unwrap(), sample_v2().encode().unwrap()] {
            for version in [0u16, 3, 4, u16::MAX] {
                let mut future = known.clone();
                future[4..6].copy_from_slice(&version.to_le_bytes());

                let error = validate_frame_refusal_payload_any_version(&future).unwrap_err();
                assert!(
                    error.contains(&format!("frame refusal version {version}")),
                    "the refusal must name the version it could not read: {error}"
                );
                assert!(
                    error.contains("not a version this build decodes"),
                    "{error}"
                );
                assert!(
                    error.contains(&format!(
                        "({FRAME_REFUSAL_VERSION} or {FRAME_REFUSAL_VERSION_V2})"
                    )),
                    "the refusal must name every version this build does decode: {error}"
                );

                let decode_error = decode_frame_refusal_any_version(&future).unwrap_err();
                assert_eq!(
                    decode_error, error,
                    "the validator and the decoder must refuse an unknown version identically"
                );
            }
        }
    }

    /// The versions this build DOES know stay accepted by the same dispatcher
    /// that refuses the ones it does not. Without this, "refuse everything"
    /// would pass the test above.
    #[test]
    fn both_known_versions_pass_the_dispatcher_that_refuses_unknown_ones() {
        validate_frame_refusal_payload_any_version(&sample().encode().unwrap()).unwrap();
        validate_frame_refusal_payload_any_version(&sample_v2().encode().unwrap()).unwrap();
        assert_eq!(
            decode_frame_refusal_any_version(&sample_v2().encode().unwrap()).unwrap(),
            sample_v2()
        );
        assert_eq!(
            decode_frame_refusal_any_version(&sample().encode().unwrap())
                .unwrap()
                .refused_hour,
            sample().refused_hour
        );
    }

    /// Bytes that are not a refusal record at all must still be diagnosed as
    /// that, not as an unknown version: the magic check is what fires, and the
    /// version word of a non-record is meaningless.
    ///
    /// PRODUCTION CHANGE THAT MAKES THIS FAIL: reading the version word before
    /// checking the magic in `declared_frame_refusal_version`.
    ///
    /// The fixture carries an UNKNOWN version under the broken magic, and that
    /// is load-bearing. With version 2 stamped, a reader that skipped the magic
    /// check would dispatch to the v2 validator, which checks the magic itself
    /// and produces the identical "magic is not H4FR" -- the test would pass
    /// under its own mutation. Version 3 makes the two paths say different
    /// things: magic-first says the bytes are not a record, version-first says
    /// the build is too old.
    #[test]
    fn bytes_that_are_not_a_refusal_are_diagnosed_by_magic_not_by_version() {
        let mut not_a_record = sample_v2().encode().unwrap();
        not_a_record[0] = b'X';
        not_a_record[4..6].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(
            declared_frame_refusal_version(&not_a_record),
            None,
            "a payload without the magic declares no version, whatever is in its version word"
        );
        let error = validate_frame_refusal_payload_any_version(&not_a_record).unwrap_err();
        assert!(error.contains("magic is not H4FR"), "{error}");
        let error = decode_frame_refusal_any_version(&not_a_record).unwrap_err();
        assert!(error.contains("magic is not H4FR"), "{error}");
        // And a payload too short to hold a version reports its length rather
        // than reading one -- through BOTH entry points, and at every length
        // below the header, so no index can be taken that is not there.
        for len in 0..FRAME_REFUSAL_HEADER_LEN {
            let short = &not_a_record[..len.min(not_a_record.len())];
            let error = validate_frame_refusal_payload_any_version(short).unwrap_err();
            assert!(!error.contains("this build decodes"), "{len}: {error}");
            let error = decode_frame_refusal_any_version(short).unwrap_err();
            assert!(!error.contains("this build decodes"), "{len}: {error}");
        }
        let error = validate_frame_refusal_payload_any_version(&[]).unwrap_err();
        assert!(error.contains("below the"), "{error}");
        let error = decode_frame_refusal_any_version(&[]).unwrap_err();
        assert!(error.contains("below the"), "{error}");
    }

    #[test]
    fn v2_is_wired_into_the_canonical_record_dispatch() {
        use crate::record_semantics::{
            validate_compact_record_semantics, COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
        };
        let wire = sample_v2().encode().unwrap();
        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &wire,
        )
        .expect("version 2 must be accepted through the canonical dispatch");
        // The legacy version still is.
        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &sample().encode().unwrap(),
        )
        .expect("version 1 must stay readable");
        // A version nobody has defined is still refused.
        let mut future = wire.clone();
        future[4] = 9;
        assert!(validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &future
        )
        .is_err());
    }

    /// The gap this closes: an hour refused for a reason that belongs to NO
    /// domain used to publish nothing at all, because the record could not
    /// carry a non-attributable reason and the producer's guard was
    /// `!offenders.is_empty()`. The hour went entirely unaccounted.
    #[test]
    fn a_refusal_with_no_attributable_domain_is_still_a_complete_record() {
        let frame_only = FrameRefusalRecordV2 {
            refused_hour: 60_831_500,
            offenders: Vec::new(),
            frame_findings: vec![FrameRefusalFinding {
                predicate: Predicate::new(0x0001_0003, "frame_end_present_and_singular").unwrap(),
                authority: Authority::Host,
                locator: Vec::new(),
                reason: "framing: frame carries no FRAME_END record".to_string(),
            }],
        };
        let wire = frame_only
            .encode()
            .expect("a frame-level-only refusal must be encodable");
        validate_frame_refusal_v2_payload(&wire).unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        assert_eq!(back, frame_only);
        assert!(back.offenders.is_empty());
        assert_eq!(back.frame_findings.len(), 1);
        assert!(back.frame_findings[0]
            .render()
            .contains("predicate=frame_end_present_and_singular#65539"));
        assert!(back.frame_findings[0].render().contains("authority=host"));
        // And through the canonical dispatch, which is what a consumer calls.
        use crate::record_semantics::{
            validate_compact_record_semantics, COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
        };
        validate_compact_record_semantics(
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_REFUSED,
            0,
            &wire,
        )
        .expect("a frame-level-only refusal must pass the canonical dispatch");
    }

    /// A refusal must still substantiate itself. Widening what counts as
    /// substantiation must not become "anything goes".
    #[test]
    fn a_refusal_that_substantiates_nothing_is_still_refused() {
        let empty = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders: Vec::new(),
            frame_findings: Vec::new(),
        };
        let error = empty.encode().unwrap_err();
        assert!(error.contains("neither an offending domain nor a frame-level finding"));

        // On the wire: zero both counts under an otherwise valid record.
        let mut wire = sample_v2().encode().unwrap();
        wire[12] = 0;
        wire[13] = 0;
        wire[14] = 0;
        wire[15] = 0;
        assert!(validate_frame_refusal_v2_payload(&wire).is_err());

        // A finding with a zero predicate code is refused, exactly as an
        // offender with one is.
        let mut wire = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders: Vec::new(),
            frame_findings: vec![FrameRefusalFinding {
                predicate: Predicate::new(7, "x").unwrap(),
                authority: Authority::Host,
                locator: Vec::new(),
                reason: "y".to_string(),
            }],
        }
        .encode()
        .unwrap();
        for byte in 0..4 {
            wire[FRAME_REFUSAL_HEADER_LEN + byte] = 0;
        }
        let error = validate_frame_refusal_v2_payload(&wire).unwrap_err();
        assert!(error.contains("declares no predicate"), "{error}");
    }

    #[test]
    fn every_v2_truncation_and_corruption_fails_closed() {
        let wire = sample_v2().encode().unwrap();
        for len in 0..wire.len() {
            assert!(
                validate_frame_refusal_v2_payload(&wire[..len]).is_err(),
                "prefix of {len} byte(s) must not validate"
            );
        }
        let mut trailing = wire.clone();
        trailing.push(0);
        assert!(validate_frame_refusal_v2_payload(&trailing).is_err());
        for (index, label) in [(0usize, "magic"), (4, "version"), (6, "header_len")] {
            let mut bad = wire.clone();
            bad[index] ^= 0xFF;
            assert!(
                validate_frame_refusal_v2_payload(&bad).is_err(),
                "corrupt {label} must fail"
            );
        }
        // Every single-byte flip in the fixed entry head is either rejected or
        // decodes to a self-consistent record; nothing may panic.
        for index in FRAME_REFUSAL_HEADER_LEN..wire.len() {
            let mut bad = wire.clone();
            bad[index] ^= 0xFF;
            let _ = validate_frame_refusal_v2_payload(&bad);
            let _ = FrameRefusalRecordV2::decode(&bad);
        }
    }

    /// The end-to-end property this whole lane exists for: under a budget that
    /// cannot carry everything, the deciding unit of EVERY offender still
    /// reaches the wire, and what was dropped is stated numerically.
    #[test]
    fn under_budget_pressure_every_offenders_deciding_unit_still_reaches_the_wire() {
        let requests: Vec<BudgetRequest> = (40..=52u16)
            .map(|domain_id| {
                let mut units = vec![DiagUnit::new(
                    format!("d{domain_id}_deciding"),
                    0,
                    UnitRole::Deciding,
                    UnitPresence::Observed,
                )];
                // A wildly verbose neighbour: 120 counters each.
                for index in 0..120u64 {
                    units.push(DiagUnit::counter(
                        format!("d{domain_id}_counter_{index:03}"),
                        index,
                    ));
                }
                // Deciding unit last in producer order, as D44's was.
                units.rotate_left(1);
                BudgetRequest { domain_id, units }
            })
            .collect();
        let grants = allocate_unit_budget(&requests, 512, FRAME_REFUSAL_MAX_UNITS_LEN);
        let offenders: Vec<FrameRefusalOffenderV2> = grants
            .iter()
            .map(|grant| FrameRefusalOffenderV2 {
                domain_id: grant.domain_id,
                status_flags: 0x0000_0100,
                non_tolerated_flags: 0x0000_0100,
                predicate: Predicate::new(u32::from(grant.domain_id) * 100 + 1, "walk_predicate")
                    .unwrap(),
                authority: Authority::SelfDomain,
                locator: Vec::new(),
                units: grant.units.clone(),
                truncation: grant.account,
                off_frame: None,
                reason: format!(
                    "refused: {}",
                    render_units_bounded(
                        &grant.units,
                        FRAME_REFUSAL_MAX_REASON_LEN.saturating_sub("refused: ".len())
                    )
                    .0
                ),
                reason_units_named: render_units_bounded(
                    &grant.units,
                    FRAME_REFUSAL_MAX_REASON_LEN.saturating_sub("refused: ".len()),
                )
                .1,
            })
            .collect();
        let record = FrameRefusalRecordV2 {
            refused_hour: 1,
            offenders,
            frame_findings: Vec::new(),
        };
        let wire = record.encode().unwrap();
        let back = FrameRefusalRecordV2::decode(&wire).unwrap();
        assert_eq!(back.offenders.len(), 13);
        for offender in &back.offenders {
            assert!(
                offender
                    .units
                    .iter()
                    .any(|unit| unit.role == UnitRole::Deciding),
                "domain {} lost its deciding unit under budget pressure",
                offender.domain_id
            );
            assert_eq!(offender.truncation.kind, TruncationKind::BudgetShare);
            assert!(offender.truncation.units_dropped() > 0);
            assert!(offender.truncation.bytes_dropped > 0);
            assert!(
                !offender.render().contains("..."),
                "truncation must be stated, not elided"
            );
        }
    }
}
