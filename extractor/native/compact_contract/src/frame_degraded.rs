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
//! # WHO said this domain is unknown, and on what evidence
//!
//! Version 1 carried the offender's `reason` and nothing structured. That was
//! enough while every degradation came from the producer itself. It stopped
//! being enough when the collector gained a frame-final pass that
//! SUBSTANTIATES a diagnosis for a producer that recorded none — reading the
//! completion and extent back out of the published bits, because the
//! alternative was refusing all 73 domains for one silent one.
//!
//! Those two are different facts about an hour. "The reader said its source was
//! unavailable" and "nobody said anything, and this was reconstructed from the
//! status word" are worth different amounts to a consumer deciding whether to
//! trust a neighbouring domain, and to anyone counting how much of the producer
//! migration is left. In version 1 the difference existed only inside the
//! collector and in the PROSE of `reason`, so replay could recover it by string
//! matching or not at all. Under the single-file law that is a hole: the replay
//! file is the only connection between the two halves of this system, so a
//! distinction that matters has to be ON THE WIRE.
//!
//! So an entry carries the same two typed fields `H4FR`'s offenders already do,
//! and for the same reasons:
//!
//! * a **predicate**, non-zero code plus name, naming WHICH check failed — and
//!   in particular whether that check is
//!   `outcome_substantiated_from_record`, the sweep's own code, or
//!   `producer_declared_no_predicate`, the reserved stand-in. Both are
//!   countable in one pass over a stream by code, without parsing text;
//! * an **authority**, so "a different domain's roster was incomplete" or "the
//!   frame's own clock was unreadable" is not read as this domain's defect.
//!
//! They are not new information. The collector builds both for the offender it
//! would have put in `H4FR`; version 1 simply discarded them on the degraded
//! path.
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
//!   reserved        u16   MUST BE ZERO, so a later version can spend it
//!   status_flags    u32   raw PERF status for that domain
//!   non_tolerated   u32   the subset that made it unknown
//!   predicate_code  u32   NON-ZERO; PREDICATE_UNDECLARED if the producer
//!                         declared none
//!   authority       [12]  kind u16 | domain_id u16 | rva u64
//!   reason_len      u16
//!   name_len        u16   version 1's trailing `reserved`, spent
//!   -- 32 byte fixed part --
//!   reason          [reason_len]     UTF-8, the reader's own text
//!   predicate_name  [name_len]       UTF-8
//! ```

use crate::diagnostic_channel::{Authority, Predicate, AUTHORITY_LEN, MAX_PREDICATE_NAME_LEN};
use crate::{
    compact_status_failure_flags_after_expected_partial_tolerance, expected_partial_tolerance,
    expected_partial_tolerance_applies, CompactDomainId,
};

pub const FRAME_DEGRADED_MAGIC: &[u8; 4] = b"H4FD";
/// Version 2 added the predicate and the authority. There is no dual-read path
/// and no migration: pre-production captures are disposable, so a version this
/// decoder does not know is refused rather than guessed at.
pub const FRAME_DEGRADED_VERSION: u16 = 2;
pub const FRAME_DEGRADED_HEADER_LEN: usize = 16;
pub const FRAME_DEGRADED_ENTRY_FIXED_LEN: usize = 32;
/// Same bound `H4FR` puts on a predicate name, from the same constant.
pub const FRAME_DEGRADED_MAX_PREDICATE_NAME_LEN: usize = MAX_PREDICATE_NAME_LEN;
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

/// WHO said this domain failed, and on the strength of what check.
///
/// One aggregate rather than two more arguments, and MANDATORY rather than a
/// builder step, for the reason the whole record exists: the version-1 entry
/// let a caller state a degradation without stating who decided it, and the
/// only field that could have carried the difference was free text. A caller
/// that has nothing to say has one thing to pass —
/// [`FrameDegradedDiagnosis::undeclared`] — and that says so on the wire in a
/// form a census counts by code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameDegradedDiagnosis {
    pub predicate: Predicate,
    pub authority: Authority,
    /// The producer's own text.
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

    /// The reserved stand-in for a producer that declared no predicate.
    ///
    /// Deliberately NOT a `Default`: reaching for this has to be a decision a
    /// reader of the call site can see, because the count of entries carrying
    /// it is the exact size of the remaining producer migration.
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
    /// Build an entry for a domain, or `None` when the domain is not degraded.
    ///
    /// This is the only sanctioned way to produce an entry: `non_tolerated_flags`
    /// is DERIVED from the shared classifier rather than supplied, so a caller
    /// cannot list a complete or expected-partial domain here even by mistake.
    ///
    /// The diagnosis is an argument and not a later `with_` step, so a site that
    /// has not decided who refused cannot compile.
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

    /// A one-line rendering, the same shape `FrameRefusalOffenderV2::render`
    /// produces for the same three fields.
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

/// Everything an entry has to be self-consistent about, in ONE function shared
/// by `encode` and `validate_frame_degraded_payload`.
///
/// The sharing is the point: a record this codec can write and cannot read
/// back, or the reverse, is a producer and a consumer disagreeing about a
/// format neither can be talked out of. Every rule below is therefore stated
/// once and applied on both sides.
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
        // Version 2 spent one of version 1's two spare `u16`s and left this
        // one. A non-zero value in it is either corruption or a producer
        // writing a field this format has not defined, and admitting it now
        // would spend the field on nothing.
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) sets its reserved field to \
             {reserved:#06x}, which this format does not define"
        ));
    }
    if domain_id == crate::record_semantics::COMPACT_DOMAIN_FRAME {
        // This record's entries are PER-DOMAIN. The frame's own faults are what
        // `H4FR`'s frame findings carry, and a frame fault refuses the hour
        // rather than degrading one domain of it, so an entry naming the frame
        // envelope is a category error however it got here.
        return Err(format!(
            "frame degraded entry {index} names the frame envelope domain \
             {domain_id}, which is not a per-domain degradation"
        ));
    }
    if crate::assigned_descriptor_by_id(domain_id).is_none() {
        // Rejecting only the frame envelope above left every OTHER non-domain
        // id admissible: a one-entry record naming domain 250 -- an id the fold
        // uses as an unassigned probe -- validated on both sides and named a
        // degradation of nothing. The status word is already checked against
        // this domain's vocabulary two lines down, but that check reads the
        // LANE-LOCAL badge mask for the id, which an unassigned id simply does
        // not have, so a word of nothing but shared bits sailed past it.
        //
        // An entry is a claim about a domain the reader swept. There is no such
        // sweep for an id the contract never assigned, so the record is not a
        // degradation this consumer can account for and the whole notice is
        // refused rather than one entry dropped.
        return Err(format!(
            "frame degraded entry {index} names domain {domain_id}, which this contract \
             does not assign"
        ));
    }
    let unassigned = crate::compact_status_unassigned_flags(domain_id, status_flags);
    if unassigned != 0 {
        // The DLL and the host ship together, so a bit neither the v1
        // vocabulary nor this domain's lane-local badge mask defines is not
        // version skew -- it is wire disagreement, and the shared contract says
        // a consumer must refuse the record rather than classify the rest of
        // the word around it.
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) status {status_flags:#010x} sets \
             unassigned bit(s) {:?}",
            crate::compact_status_unassigned_bit_indices(unassigned)
        ));
    }
    if status_flags & crate::COMPACT_STATUS_BASELINE_CLEAR != 0 {
        // A baseline clear asks the consumer to REPLACE everything it holds for
        // the domain, on the strength of the record being a complete claim
        // about its whole extent. An entry in this record is failure-class by
        // construction (`non_tolerated_flags` is non-zero below), so it is the
        // one thing that can never make that claim.
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) requests BASELINE_CLEAR on a \
             failure-class record: a read that did not complete cannot describe the whole \
             extent it asks the consumer to replace"
        ));
    }
    if predicate_code == 0 {
        // Zero is what an uninitialised slot holds, and `Predicate` refuses it
        // in the type system. A record carrying it would say "no check failed"
        // about an entry that exists only because one did.
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
        // The reserved code and the reserved name travel together or not at
        // all. Letting them disagree would let a producer stamp the reserved
        // code under a name that reads like a real check -- or claim the
        // reserved name while carrying a code a census counts as declared --
        // and the count of undeclared producers is the measurement this pair
        // exists to make.
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
        // CANONICAL BYTES, not merely decodable ones. `Authority::decode` reads
        // the domain and rva words for every kind and discards the ones that
        // kind does not use, so two different byte strings decode to one
        // authority -- and a record that means one thing in two spellings is a
        // record two consumers can disagree about while both being right.
        return Err(format!(
            "frame degraded entry {index} (domain {domain_id}) authority bytes are not canonical \
             for {}",
            authority.render()
        ));
    }
    if let Authority::OtherDomain(other) = authority {
        // The canonicality rule above proves the BYTES spell one authority; it
        // says nothing about whether the domain those bytes name exists. Kind 2
        // takes any `u16`, so `02 00 FA 00 ...` decoded and re-encoded happily
        // as "domain 250 was the authority" -- a blame assignment pointing at
        // nothing, which is worse than `Undeclared` because it reads as an
        // answer. The whole point of this field is that "the thing that refused
        // is not me" names WHO, so a name that resolves to no domain fails the
        // field's only purpose.
        //
        // Checked here rather than in `Authority::decode` because decode is
        // shared with `H4FR` and its offenders; widening the shared decoder is
        // a change to a record this lane did not review.
        if crate::assigned_descriptor_by_id(other).is_none() {
            return Err(format!(
                "frame degraded entry {index} (domain {domain_id}) names domain {other} as its \
                 authority, which this contract does not assign"
            ));
        }
        if other == crate::record_semantics::COMPACT_DOMAIN_FRAME {
            // The frame envelope IS an assigned descriptor (id 1000,
            // `Infrastructure`), so the rule above passes it. It is still not a
            // peer whose published state another domain can have consulted: a
            // frame-owned fault refuses the hour through `H4FR` rather than
            // degrading one domain, which is the same category error the
            // subject-domain rule rejects at the top of this function.
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

impl FrameDegradedRecordV2 {
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
            let name_len = u16::from_le_bytes([payload[offset + 30], payload[offset + 31]]) as usize;
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
    let header_reserved = u16::from_le_bytes([payload[14], payload[15]]);
    if header_reserved != 0 {
        // The header's spare `u16` is still spare, and it stays reservable only
        // while nothing is admitted in it. The encoder writes zero; a decoder
        // that shrugged at anything else would let a future field be smuggled
        // into a record this version claims to understand.
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
        // The two variable fields are bounded and read BEFORE `check_entry`
        // sees the name, because the name is one of the things it checks. The
        // arithmetic is checked in one place so neither length can wrap into a
        // shorter slice than it claims.
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

    /// A domain that MEASURED an empty source is complete, and the frame it
    /// sits in is not degraded by it.
    ///
    /// This classifier is shared by the collector (what to publish), the fold
    /// (what to apply) and the certifier (what to accept), so getting it wrong
    /// here would make a measured zero unusable in all three at once — the
    /// domain's rows would be discarded and its accumulator retained, which is
    /// the exact behaviour the badge exists to stop.
    ///
    /// Production change that makes this fail: adding SOURCE_EMPTY to
    /// `COMPACT_STATUS_FAILURE_MASK`.
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
        // ... and the badge does not launder a real failure sitting beside it.
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
            // A DIFFERENT domain was the authority. Version 1 could not say
            // this, and reporting another domain's incomplete roster as this
            // domain's defect is the exact mistake the field exists to stop.
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
                // The frame-final sweep's own diagnosis. Telling these two
                // apart from the wire alone is what version 2 is for.
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
            compact_domain_completeness(29, COMPACT_STATUS_PARTIAL_DATA),
            CompactDomainCompleteness::Degraded {
                non_tolerated_flags: COMPACT_STATUS_PARTIAL_DATA
            }
        );
        // d47 used to be that example. It has a tolerance now — for a payload
        // that DECLARES which row families it did not observe — so the shape
        // that must stay degraded is the one carrying a second bit. Every d47
        // reader that publishes nothing sets `SOURCE_UNAVAILABLE` for exactly
        // this reason: it is what keeps an undeclared loss outside a tolerance
        // written for a declared one.
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
                    producer_diagnosis(),
                )
                .is_none(),
                "domain {} tolerated status must not build a degraded entry",
                tolerance.domain_id
            );
            // And a hand-built entry that lies about it is refused by the codec,
            // not merely discouraged by the constructor.
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

    /// An entry may not understate which bits made it unknown: the record must
    /// agree with the shared classifier exactly.
    #[test]
    fn an_entry_that_understates_its_non_tolerated_bits_is_refused() {
        let mut value = sample();
        value.domains[0].non_tolerated_flags = COMPACT_STATUS_SOURCE_UNAVAILABLE;
        let error = value.encode().expect_err("understated bits must be refused");
        assert!(error.contains("classifies as"), "{error}");
    }

    /// THE BYTE GOLDEN.
    ///
    /// Round-tripping proves the encoder and the decoder agree with each other,
    /// which is exactly what a layout drift preserves: move a field and both
    /// halves move with it, and every round-trip test still passes while every
    /// capture on disk becomes unreadable by the version that wrote it. This
    /// states the bytes independently, so the LAYOUT is the thing under test.
    ///
    /// One entry, spelled out field by field.
    ///
    /// kills: reordering any entry field, changing a width, writing the
    /// predicate name before the reason, dropping the entry's reserved `u16`,
    /// changing `FRAME_DEGRADED_VERSION` or `FRAME_DEGRADED_ENTRY_FIXED_LEN`,
    /// or switching any field to big-endian.
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
        expected.extend_from_slice(&2u16.to_le_bytes()); // version
        expected.extend_from_slice(&16u16.to_le_bytes()); // header_len
        expected.extend_from_slice(&0x0102_0304i32.to_le_bytes()); // degraded_hour
        expected.extend_from_slice(&1u16.to_le_bytes()); // domain_count
        expected.extend_from_slice(&0u16.to_le_bytes()); // header reserved
        expected.extend_from_slice(&91u16.to_le_bytes()); // domain_id
        expected.extend_from_slice(&0u16.to_le_bytes()); // entry reserved
        expected.extend_from_slice(&COMPACT_STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        expected.extend_from_slice(&COMPACT_STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        expected.extend_from_slice(&0x0002_0011u32.to_le_bytes()); // predicate_code
        expected.extend_from_slice(&2u16.to_le_bytes()); // authority kind = OtherDomain
        expected.extend_from_slice(&18u16.to_le_bytes()); // authority domain
        expected.extend_from_slice(&0u64.to_le_bytes()); // authority rva
        expected.extend_from_slice(&3u16.to_le_bytes()); // reason_len
        expected.extend_from_slice(&33u16.to_le_bytes()); // predicate name_len
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

    /// The predicate is what makes "the producer said so" and "the frame-final
    /// sweep reconstructed this from the record" different facts on the wire,
    /// and every way of stating that badly is refused.
    ///
    /// kills: dropping the zero-code check, dropping the name bound, or letting
    /// the reserved undeclared code travel under a name that reads like a real
    /// check (which is how a census of unmigrated producers gets quietly
    /// wrong).
    #[test]
    fn a_predicate_that_cannot_be_counted_is_refused() {
        let wire = sample().encode().unwrap();
        let code_at = FRAME_DEGRADED_HEADER_LEN + 12;

        // Zero is what an uninitialised slot holds.
        let mut zero_code = wire.clone();
        zero_code[code_at..code_at + 4].copy_from_slice(&0u32.to_le_bytes());
        let error = validate_frame_degraded_payload(&zero_code)
            .expect_err("predicate code 0 must be refused");
        assert!(error.contains("reserved for 'not declared'"), "{error}");

        // The reserved code and the reserved name travel together.
        let mut lying_code = wire.clone();
        lying_code[code_at..code_at + 4]
            .copy_from_slice(&crate::diagnostic_channel::PREDICATE_UNDECLARED.to_le_bytes());
        let error = validate_frame_degraded_payload(&lying_code)
            .expect_err("the reserved code under a real check's name must be refused");
        assert!(error.contains("only valid together"), "{error}");

        // ... and the reverse: the reserved NAME under a declared code.
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

    /// The authority has to mean exactly one thing per byte string.
    ///
    /// `Authority::decode` reads the domain and rva words whatever the kind is
    /// and discards the ones that kind does not use, so without a canonicality
    /// rule the same authority has many spellings — and two consumers diffing
    /// two captures would see a difference that is not a difference.
    ///
    /// kills: dropping the `authority.encode() != bytes` comparison, and
    /// admitting kind 0 (the in-memory-only `Undeclared`).
    #[test]
    fn a_non_canonical_or_undeclared_authority_is_refused() {
        let wire = sample().encode().unwrap();
        let authority_at = FRAME_DEGRADED_HEADER_LEN + 16;

        // Kind 0 exists in memory and has no wire encoding.
        let mut undeclared = wire.clone();
        undeclared[authority_at..authority_at + 2].copy_from_slice(&0u16.to_le_bytes());
        assert!(validate_frame_degraded_payload(&undeclared).is_err());

        // An unknown kind is refused by the decoder itself.
        let mut unknown = wire.clone();
        unknown[authority_at..authority_at + 2].copy_from_slice(&9u16.to_le_bytes());
        assert!(validate_frame_degraded_payload(&unknown).is_err());

        // The first entry is `Authority::OtherDomain(18)`, which does not use
        // the rva word. A non-zero one decodes to the SAME authority, and that
        // is precisely why it has to be refused rather than normalised away.
        let mut noisy_rva = wire.clone();
        noisy_rva[authority_at + 4..authority_at + 12].copy_from_slice(&0xDEADu64.to_le_bytes());
        let error = validate_frame_degraded_payload(&noisy_rva)
            .expect_err("non-canonical authority bytes must be refused");
        assert!(error.contains("not canonical"), "{error}");
    }

    /// Reserved means reserved, in the header and in every entry.
    ///
    /// kills: dropping either zero check — after which the field can never be
    /// spent, because captures already in existence would be carrying values
    /// in it.
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

    /// A status word carrying a bit no side of the contract defines is wire
    /// disagreement, and a per-domain degradation cannot request a baseline
    /// clear it is failure-class by construction.
    ///
    /// kills: dropping either check from `check_entry`.
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

    /// An entry is PER-DOMAIN. The frame's own faults refuse the hour and are
    /// carried by `H4FR`'s frame findings, so an entry naming the frame
    /// envelope is a category error whatever produced it.
    ///
    /// kills: dropping the `COMPACT_DOMAIN_FRAME` check.
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

    /// Rejecting only the frame envelope left every other non-domain id
    /// admissible. Domain 250 is the id `fold_check` uses as its unassigned
    /// probe, and a one-entry notice naming it validated on both sides: the
    /// status word here is nothing but shared vocabulary bits, so the
    /// per-domain badge check has nothing to object to.
    ///
    /// kills: dropping the `assigned_descriptor_by_id` check on the entry's
    /// subject domain, or narrowing it back to `COMPACT_DOMAIN_FRAME`.
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
        // And the decoder refuses it too, not just the encoder: a producer that
        // never called `encode` is exactly the case a validator exists for.
        let mut wire = sample().encode().unwrap();
        wire[16] = 250u16.to_le_bytes()[0];
        wire[17] = 250u16.to_le_bytes()[1];
        assert!(validate_frame_degraded_payload(&wire).is_err());
    }

    /// `Authority::OtherDomain` carries a bare `u16`, so canonical bytes could
    /// still name a domain that does not exist -- blame pointing at nothing,
    /// which reads as an answer and is therefore worse than `Undeclared`.
    ///
    /// kills: dropping the assigned check on `OtherDomain`. The bytes below are
    /// canonical, so the canonicality rule above cannot catch this.
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
        // The frame envelope needs its OWN branch here, unlike the subject
        // domain: 1000 is an assigned `Infrastructure` descriptor, so the
        // unassigned rule above passes it straight through. It is still not a
        // peer whose state a domain can have consulted.
        assert!(crate::assigned_descriptor_by_id(
            crate::record_semantics::COMPACT_DOMAIN_FRAME
        )
        .is_some());
        let framed = FrameDegradedRecordV2 {
            degraded_hour: 7,
            domains: vec![FrameDegradedDomain {
                domain_id: 47,
                status_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                non_tolerated_flags: COMPACT_STATUS_SOURCE_UNAVAILABLE,
                predicate: producer_diagnosis().predicate,
                authority: Authority::OtherDomain(
                    crate::record_semantics::COMPACT_DOMAIN_FRAME,
                ),
                reason: "blamed the frame as a peer".to_string(),
            }],
        };
        let error = framed
            .encode()
            .expect_err("the frame envelope is not a peer domain");
        assert!(error.contains("frame envelope"), "{error}");
        // A real peer domain still passes, so the rule is not simply refusing
        // every `OtherDomain`.
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
        let wire = ok.encode().expect("an assigned peer domain is a real authority");
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
