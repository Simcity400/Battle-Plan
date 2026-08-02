//! What a domain read DID, as one value — and the only place a status bit is
//! decided from it.
//!
//! # The defect this replaces
//!
//! Every failing domain read in the collector already computes a named reason.
//! It builds a `String`, returns `Err(that_string)`, and the caller then writes
//!
//! ```text
//! Err(_) => status |= COMPACT_STATUS_SOURCE_UNAVAILABLE,
//! ```
//!
//! The reason is constructed and thrown away in the same expression. Measured
//! on `build_compact_extraction_gate_records` alone there are 49 such arms,
//! including the worker-join block where the panic payload of every panicked
//! reader — the one artefact that says WHY a domain vanished — is discarded by
//! an `Err(_)` pattern that never binds it.
//!
//! Downstream, [`crate::frame_refusal::FrameRefusalRecordV2`] has fields for
//! exactly that information: a predicate, an authority, a locator path, typed
//! units. With no producer to fill them, every offender is stamped
//! [`Predicate::undeclared`]. The consumer half of the contract was built and
//! then fed nothing.
//!
//! # The shape
//!
//! A migrated site returns ONE [`DomainOutcome`]. It carries the diagnosis
//! (predicate / authority / locator / units / the reason that used to be
//! discarded) and the two orthogonal facts the status bits are derived from:
//!
//! * [`ReadCompletion`] — did the read finish, and if not, how did it stop.
//! * [`AnswerExtent`] — does what was published stand for the whole answer.
//!
//! They are separate on purpose. "The read did not complete" and "what we
//! published is not everything" are different observations that co-occur, and a
//! single enum forced the producer to pick one, which is how a domain that both
//! failed and truncated ended up reporting only one of the two.
//!
//! # Deriving the bits, not writing them
//!
//! [`DomainOutcome::derive_status`] is the ONLY place a migrated site's status
//! word is computed. It returns [`DerivedStatus`], a newtype that deliberately
//! implements no `BitOr`, `BitOrAssign` or `From<u32>`: at a migrated site,
//!
//! ```text
//! outcome.derive_status(base) | COMPACT_STATUS_PARTIAL_DATA
//! ```
//!
//! does not compile. That is the point. A hand-written bit is a claim the
//! record cannot substantiate, because nothing else in the outcome changed to
//! explain it.
//!
//! The derivation reproduces the pre-migration bit semantics EXACTLY — see
//! `derive_status` for the mapping and `tests` for the table that pins it. A
//! migration that changed which bits a consumer sees would not be a migration.
//!
//! # `Incomplete` is not `Unprovable`
//!
//! Both derive [`COMPACT_STATUS_PARTIAL_DATA`], and they must still be
//! distinguishable in the record. "We reached the whole extent and lost rows
//! inside it" is a defect in the read; "we cannot prove we saw everything" is a
//! defect in the construction. The first is actionable against the reader, the
//! second against the roster walk, and collapsing them into one bit is why the
//! roster family's failures were never separable.

use crate::diagnostic_channel::{Authority, DiagUnit, LocatorLevel, Predicate};
use crate::status_flags_base::{
    COMPACT_STATUS_CONTROL_INVALID, COMPACT_STATUS_NO_CLOCK, COMPACT_STATUS_PARTIAL_DATA,
    COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_WORKER_PANIC,
};

// ---------------------------------------------------------------------------
// Predicate codes
// ---------------------------------------------------------------------------

// A predicate code is a DIAGNOSTIC ABI. Retained telemetry is read by build-
// independent tooling, so a code that changes meaning between builds silently
// rewrites history. Codes are therefore assigned once, never reused, and never
// renumbered — a retired predicate leaves a hole.
//
// Ranges:
//   0x0001_xxxx  frame-level predicates (collector, `FRAME_PREDICATE_*`)
//   0x0002_xxxx  domain read predicates (this table)
//   0xFFFF_FF01  reserved: the producer declared none
//
// A predicate names WHICH CHECK failed, not which line failed. Six checks cover
// all 49 arms of tranche 1 because the arms are six shapes repeated, which is
// itself the finding: the reasons differed, the checks did not. Tranche 2 added
// one (the frame's attribution probe) and tranche 3 five, for the sites where
// the CONDITION was the diagnosis: 0x0002_000B..0x0002_000F. Several tranche-3
// sites share a predicate on purpose -- three domains re-check the caller's
// capture identically, and giving one fact three names would be three claims.
// WHERE is carried separately, by the collector's `MigratedStatusSite`.

/// The reader's worker joined without panicking.
pub const READ_PREDICATE_WORKER_COMPLETED: u32 = 0x0002_0001;
/// The domain's payload encoder accepted the read.
pub const READ_PREDICATE_PAYLOAD_ENCODED: u32 = 0x0002_0002;
/// The encoded payload decoded, and the decode proved the event set exhaustive.
pub const READ_PREDICATE_PAYLOAD_DECODED_EXACT: u32 = 0x0002_0003;
/// The reader returned its rows.
pub const READ_PREDICATE_READER_RETURNED_ROWS: u32 = 0x0002_0004;
/// The frame's read-counter probe was available.
pub const READ_PREDICATE_READ_COUNTER_PROBE: u32 = 0x0002_0005;
/// Rejected domain records were replaced with gaps and the frame resequenced.
pub const READ_PREDICATE_REJECTED_RECORDS_RESEQUENCED: u32 = 0x0002_0006;
/// The frame carried a readable game hour for this domain to label against.
pub const READ_PREDICATE_FRAME_HOUR_READABLE: u32 = 0x0002_0007;
/// The domain's read authority (control validity / gating) held.
pub const READ_PREDICATE_READ_AUTHORITY_VALID: u32 = 0x0002_0008;
// 0x0002_0009 is [`READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME`] (declared
// at the bottom of this file). It is the completeness walk's OWN predicate, not
// a check a producer runs, which is why it is not in `READ_PREDICATES` -- and
// why the next producer code is 0x0002_000A rather than 0x0002_0009.
/// The frame's attribution probe record (bound trips + status origins) was
/// appended.
///
/// Frame-scoped, like the read-counter probe beside it: no domain owns those
/// registries, and an hour that fails to drain them loses the attribution for
/// every domain in the frame rather than for one.
pub const READ_PREDICATE_ATTRIBUTION_PROBE_APPENDED: u32 = 0x0002_000A;
/// The payload the CALLER captured for this domain was admissible for this
/// frame: it decoded, its own status word was clean, it was labelled with this
/// frame's hour, and (where the domain has one) its authority was the exact
/// receipt the contract requires.
///
/// Four domains have their payload sealed by the caller before the gate
/// function is entered. The gate re-checks it and, when the check fails,
/// republishes an explicit unavailable payload instead. That check is the
/// diagnosis; there was never a `Result` for it to discard.
pub const READ_PREDICATE_CAPTURED_FRAME_ADMISSIBLE: u32 = 0x0002_000B;
/// The frame took custody of the hook batch reservation this read staged.
///
/// A reader stages a drain against a process-global hook and hands the frame a
/// reservation id. If the frame's slot for that domain is already occupied,
/// custody cannot pass: the batch is rolled back and the rows it carried are
/// dropped. Nothing failed to READ -- the read landed and then lost its rows to
/// a custody conflict, which is why the extent, not just the completion, has
/// something to say at these sites.
pub const READ_PREDICATE_HOOK_RESERVATION_CUSTODY_TAKEN: u32 = 0x0002_000C;
/// The payload this frame is about to publish actually carries the domain's
/// answer -- a clean status word, and the snapshot the domain exists to carry.
///
/// Distinct from [`READ_PREDICATE_CAPTURED_FRAME_ADMISSIBLE`]: that one judges
/// the caller's capture, this one judges the bytes the frame publishes, which
/// may be a stand-in the gate itself encoded.
pub const READ_PREDICATE_PUBLISHED_PAYLOAD_CARRIES_ANSWER: u32 = 0x0002_000D;
/// The caller captured a payload for this domain at all, and the frame's own
/// status word permitted using it.
///
/// The weaker precondition beneath [`READ_PREDICATE_CAPTURED_FRAME_ADMISSIBLE`]:
/// there is nothing to judge admissible when nothing was captured, or when the
/// frame is already refusing on its own account.
pub const READ_PREDICATE_CAPTURED_PAYLOAD_OFFERED: u32 = 0x0002_000E;
/// The FRAME's own status word permitted this domain's read to run.
///
/// Some domains refuse to read at all once the frame is already failing and
/// publish an explicit unavailable payload instead. The refusal belongs to this
/// domain's record — its hour produced no answer — but the reason is the
/// frame's state, not a fault of the domain, which is what the outcome's
/// [`Authority`] field is there to say.
pub const READ_PREDICATE_FRAME_STATUS_PERMITTED_READ: u32 = 0x0002_000F;

/// Every domain read predicate, for the uniqueness and non-zero checks.
///
/// A zero code is not a weaker predicate, it is a DELETED one: zero is reserved
/// for "not declared" and the encoder drops it, so a producer that shipped a
/// zero would publish a refusal that names no check while appearing to.
pub const READ_PREDICATES: &[(u32, &str)] = &[
    (READ_PREDICATE_WORKER_COMPLETED, "reader_worker_completed"),
    (READ_PREDICATE_PAYLOAD_ENCODED, "domain_payload_encoded"),
    (
        READ_PREDICATE_PAYLOAD_DECODED_EXACT,
        "domain_payload_decoded_exact",
    ),
    (READ_PREDICATE_READER_RETURNED_ROWS, "reader_returned_rows"),
    (READ_PREDICATE_READ_COUNTER_PROBE, "read_counter_probe"),
    (
        READ_PREDICATE_REJECTED_RECORDS_RESEQUENCED,
        "rejected_records_resequenced",
    ),
    (READ_PREDICATE_FRAME_HOUR_READABLE, "frame_hour_readable"),
    (READ_PREDICATE_READ_AUTHORITY_VALID, "read_authority_valid"),
    (
        READ_PREDICATE_ATTRIBUTION_PROBE_APPENDED,
        "attribution_probe_appended",
    ),
    (
        READ_PREDICATE_CAPTURED_FRAME_ADMISSIBLE,
        "captured_frame_admissible",
    ),
    (
        READ_PREDICATE_HOOK_RESERVATION_CUSTODY_TAKEN,
        "hook_reservation_custody_taken",
    ),
    (
        READ_PREDICATE_PUBLISHED_PAYLOAD_CARRIES_ANSWER,
        "published_payload_carries_answer",
    ),
    (
        READ_PREDICATE_CAPTURED_PAYLOAD_OFFERED,
        "captured_payload_offered",
    ),
    (
        READ_PREDICATE_FRAME_STATUS_PERMITTED_READ,
        "frame_status_permitted_read",
    ),
];

/// The name declared for a code in [`READ_PREDICATES`], or `None`.
pub fn read_predicate_name(code: u32) -> Option<&'static str> {
    READ_PREDICATES
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, name)| *name)
}

/// Build a predicate from this table.
///
/// Refuses a code that is not in the table rather than inventing a name for it:
/// an undeclared code on the wire is unreadable by the tooling that reads the
/// table, and a silently-named one is worse because it looks readable.
pub fn read_predicate(code: u32) -> Result<Predicate, String> {
    match read_predicate_name(code) {
        Some(name) => Predicate::new(code, name),
        None => Err(format!(
            "read predicate {code:#010x} is not declared in READ_PREDICATES"
        )),
    }
}

// ---------------------------------------------------------------------------
// The two orthogonal facts
// ---------------------------------------------------------------------------

/// Did the read finish, and if not, how did it stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ReadCompletion {
    /// The read ran to its own end. It may still have published a partial
    /// answer — that is [`AnswerExtent`]'s question, not this one.
    Completed = 1,
    /// The read did not complete. This is what
    /// [`COMPACT_STATUS_SOURCE_UNAVAILABLE`] has always meant: NOT "there is no
    /// source", but "we did not get through the read".
    DidNotComplete = 2,
    /// The read did not complete because the worker carrying it panicked. A
    /// strictly stronger statement than `DidNotComplete`, and the panic payload
    /// belongs in the outcome's units.
    WorkerPanicked = 3,
}

impl ReadCompletion {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::DidNotComplete => "did_not_complete",
            Self::WorkerPanicked => "worker_panicked",
        }
    }
}

/// Does what was published stand for the whole answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AnswerExtent {
    /// The construction proves the published answer is everything.
    Whole = 1,
    /// Rows were lost INSIDE an extent the read did reach. The extent is known;
    /// the contents of it are not all here.
    Incomplete = 2,
    /// The construction cannot prove exhaustiveness. Nothing is known to be
    /// missing; nothing is known not to be.
    Unprovable = 3,
}

impl AnswerExtent {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Whole => "whole",
            Self::Incomplete => "incomplete",
            Self::Unprovable => "unprovable",
        }
    }
}

/// Was the frame's game hour readable for this domain's label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockPresence {
    Readable,
    Unreadable,
}

/// Did this domain's read authority hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadAuthority {
    Valid,
    Invalid,
}

// ---------------------------------------------------------------------------
// Derived status
// ---------------------------------------------------------------------------

/// A status word that was DERIVED from an outcome.
///
/// Implements no `BitOr`, no `BitOrAssign`, no `From<u32>` and no `Deref`, so
/// the shape this migration exists to delete —
/// `status |= COMPACT_STATUS_SOURCE_UNAVAILABLE` next to a discarded reason —
/// does not compile at a migrated site. The bits leave only through
/// [`DerivedStatus::bits`], at the single hand-off to the record builder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivedStatus(u32);

impl DerivedStatus {
    /// The status word to publish.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The bits this derivation added to the frame-scoped base.
    pub const fn added(self, base: u32) -> u32 {
        self.0 & !base
    }
}

/// The bits [`DomainOutcome::derive_status`] is authoritative over.
///
/// A published status may carry other bits (the frame-scoped ones fan out into
/// every domain, and unmigrated siblings still set their own); this mask is the
/// subset an outcome is allowed to speak for, and the subset an audit compares.
///
/// Deliberately NOT the whole word. Auditing beyond this mask would accuse a
/// site of a fact its outcome never claimed: WAR_SEGMENT_ABSENT, for one, is a
/// real observation the country bundle's panic constructor makes and no
/// `DomainOutcome` field carries it. The rule the audit enforces is "the bits
/// this outcome speaks for are exactly what the record shows", not "the outcome
/// explains the whole word" -- the second is only true once every producer that
/// touches a domain's status has migrated, which is what the remaining tranches
/// are for. A hand-written bit OUTSIDE this mask is therefore not caught by the
/// runtime audit, and the source check
/// `no_migrated_site_ors_a_status_bit_onto_a_derived_word` is what covers it.
pub const DERIVED_STATUS_MASK: u32 = COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_NO_CLOCK
    | COMPACT_STATUS_CONTROL_INVALID;

// ---------------------------------------------------------------------------
// The outcome
// ---------------------------------------------------------------------------

/// One domain read's result, diagnosis included.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainOutcome {
    pub domain_id: u16,
    /// WHICH check this outcome is about. Non-zero by construction
    /// ([`Predicate`] holds a `NonZeroU32`).
    pub predicate: Predicate,
    /// WHO decided. Frequently not this domain — a reader blocked on another
    /// domain's roster must not be reported as this domain's defect.
    pub authority: Authority,
    /// Where, as `(ordinal, extent)` levels from outermost in.
    pub locator: Vec<LocatorLevel>,
    /// Typed units. A zero here is a zero, because presence says so.
    pub units: Vec<DiagUnit>,
    /// The producer's own text — the `Err(String)` the old arms discarded.
    pub reason: String,
    pub completion: ReadCompletion,
    pub extent: AnswerExtent,
    pub clock: ClockPresence,
    pub authority_state: ReadAuthority,
    /// The status word this outcome's derivation produced AND the site
    /// published, when the site published a derived word.
    ///
    /// This is what makes "no migrated site may write a bit by hand" checkable
    /// at runtime rather than by reading the source. `DerivedStatus` blocks the
    /// obvious shape at compile time -- it implements no `BitOr` -- but a site
    /// could still call `.bits()` and then `|` a constant onto the `u32`.
    /// Recording what the derivation said lets the completeness walk compare it
    /// with what the record actually carries, and disagree loudly.
    ///
    /// `None` where the published word is not the derivation's to own: a
    /// panicked worker's status comes from the domain's own
    /// `..._worker_panic_read` constructor and may legitimately carry more (the
    /// country bundle adds WAR_SEGMENT_ABSENT). Those are audited by the weaker
    /// rule that every bit the outcome claims is present.
    pub derived_status: Option<u32>,
}

impl DomainOutcome {
    /// The outcome of a read that completed and can prove it saw everything.
    ///
    /// Recorded on the SUCCESS path too, and that is load-bearing rather than
    /// bookkeeping: a migrated domain that offends with no outcome in the
    /// ledger has been BYPASSED, and only a success entry makes that
    /// distinguishable from a domain that simply did not fail.
    pub fn succeeded(domain_id: u16, predicate: Predicate) -> Self {
        Self {
            domain_id,
            predicate,
            authority: Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            reason: String::new(),
            completion: ReadCompletion::Completed,
            extent: AnswerExtent::Whole,
            clock: ClockPresence::Readable,
            authority_state: ReadAuthority::Valid,
            derived_status: None,
        }
    }

    /// The outcome of a read that failed, carrying the reason it computed.
    pub fn failed(
        domain_id: u16,
        predicate: Predicate,
        completion: ReadCompletion,
        extent: AnswerExtent,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            domain_id,
            predicate,
            authority: Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            reason: reason.into(),
            completion,
            extent,
            clock: ClockPresence::Readable,
            authority_state: ReadAuthority::Valid,
            derived_status: None,
        }
    }

    /// Stamp the derived word the site is about to publish, so the completeness
    /// walk can check the record against it.
    pub fn with_derived_status(mut self, derived: DerivedStatus) -> Self {
        self.derived_status = Some(derived.bits());
        self
    }

    /// The bits this outcome adds on its own, with no frame-scoped base.
    pub fn own_bits(&self) -> u32 {
        self.derive_status(0).bits()
    }

    /// Does the status word the record actually carries agree with the outcome?
    ///
    /// Two rules, because two kinds of site.
    ///
    /// * A site that published a DERIVED word must publish exactly it, over the
    ///   bits the derivation owns. Anything else means a bit was added after
    ///   `.bits()`, which is a claim the outcome does not substantiate -- and
    ///   the whole point of the migration is that the record and the diagnosis
    ///   cannot drift apart.
    /// * A site whose word is built elsewhere (a panicked worker's
    ///   `..._worker_panic_read`) must at least carry every bit the outcome
    ///   claims. It may carry more: the country bundle's constructor adds
    ///   WAR_SEGMENT_ABSENT, which is a real fact and not this outcome's to own.
    pub fn audit_published_status(&self, published: u32) -> Result<(), String> {
        match self.derived_status {
            Some(expected) => {
                if published & DERIVED_STATUS_MASK == expected & DERIVED_STATUS_MASK {
                    Ok(())
                } else {
                    Err(format!(
                        "domain {} published status {published:#010x} but its outcome derived                          {expected:#010x} (over mask {DERIVED_STATUS_MASK:#010x}); a bit was                          written by hand after the derivation",
                        self.domain_id
                    ))
                }
            }
            None => {
                let claimed = self.own_bits();
                if published & claimed == claimed {
                    Ok(())
                } else {
                    Err(format!(
                        "domain {} published status {published:#010x}, which is missing                          {:#010x} that its outcome claims",
                        self.domain_id,
                        claimed & !published
                    ))
                }
            }
        }
    }

    pub fn with_authority(mut self, authority: Authority) -> Self {
        self.authority = authority;
        self
    }

    pub fn with_locator(mut self, locator: Vec<LocatorLevel>) -> Self {
        self.locator = locator;
        self
    }

    pub fn with_level(mut self, level: LocatorLevel) -> Self {
        self.locator.push(level);
        self
    }

    pub fn with_units(mut self, units: Vec<DiagUnit>) -> Self {
        self.units = units;
        self
    }

    pub fn with_unit(mut self, unit: DiagUnit) -> Self {
        self.units.push(unit);
        self
    }

    pub fn with_clock(mut self, clock: ClockPresence) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_read_authority(mut self, authority_state: ReadAuthority) -> Self {
        self.authority_state = authority_state;
        self
    }

    /// Is this outcome a failure at all?
    pub const fn is_failure(&self) -> bool {
        !matches!(
            (
                self.completion,
                self.extent,
                self.clock,
                self.authority_state
            ),
            (
                ReadCompletion::Completed,
                AnswerExtent::Whole,
                ClockPresence::Readable,
                ReadAuthority::Valid,
            )
        )
    }

    /// THE derivation. The only place a migrated site's status word is decided.
    ///
    /// `base` is the frame-scoped status the domain inherits (perf gate,
    /// frame-level CONTROL_INVALID / NO_CLOCK); the outcome's own bits are
    /// added to it. The mapping reproduces the pre-migration semantics exactly:
    ///
    /// | outcome fact                     | bit                        |
    /// |----------------------------------|----------------------------|
    /// | `Completed`                      | none                       |
    /// | `DidNotComplete`                 | `SOURCE_UNAVAILABLE` 0x100 |
    /// | `WorkerPanicked`                 | `WORKER_PANIC` 0x40        |
    /// | `Whole`                          | none                       |
    /// | `Incomplete`                     | `PARTIAL_DATA` 0x200       |
    /// | `Unprovable`                     | `PARTIAL_DATA` 0x200       |
    /// | `ClockPresence::Unreadable`      | `NO_CLOCK` 0x4             |
    /// | `ReadAuthority::Invalid`         | `CONTROL_INVALID` 0x1000   |
    ///
    /// `SOURCE_UNAVAILABLE` means the read did not complete. It has never meant
    /// "there is no source", and a migration that quietly gave it the intuitive
    /// meaning would relabel every retained capture.
    pub fn derive_status(&self, base: u32) -> DerivedStatus {
        let mut bits = base;
        match self.completion {
            ReadCompletion::Completed => {}
            ReadCompletion::DidNotComplete => bits |= COMPACT_STATUS_SOURCE_UNAVAILABLE,
            ReadCompletion::WorkerPanicked => bits |= COMPACT_STATUS_WORKER_PANIC,
        }
        match self.extent {
            AnswerExtent::Whole => {}
            AnswerExtent::Incomplete | AnswerExtent::Unprovable => {
                bits |= COMPACT_STATUS_PARTIAL_DATA
            }
        }
        if matches!(self.clock, ClockPresence::Unreadable) {
            bits |= COMPACT_STATUS_NO_CLOCK;
        }
        if matches!(self.authority_state, ReadAuthority::Invalid) {
            bits |= COMPACT_STATUS_CONTROL_INVALID;
        }
        DerivedStatus(bits)
    }

    /// A one-line rendering for a log or a refusal text.
    pub fn render(&self) -> String {
        format!(
            "d{} {}#{} {} {}/{} reason={}",
            self.domain_id,
            self.predicate.name,
            self.predicate.code.get(),
            self.authority.render(),
            self.completion.label(),
            self.extent.label(),
            self.reason
        )
    }
}

// ---------------------------------------------------------------------------
// The ledger
// ---------------------------------------------------------------------------

/// Every migrated site's outcome for ONE frame.
///
/// The transport between the producer (which knows why) and
/// [`crate::frame_refusal`] (which has the fields but was being fed nothing).
/// Frame-scoped and cleared per frame; a stale entry would attribute one hour's
/// failure to another's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainOutcomeLedger {
    entries: Vec<DomainOutcome>,
    frame_findings: Vec<DomainOutcome>,
}

impl DomainOutcomeLedger {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            frame_findings: Vec::new(),
        }
    }

    /// Record an outcome that belongs to the FRAME, not to any domain.
    ///
    /// Appended rather than keyed, because the frame is one subject with many
    /// independent faults: its read-counter probe and its record resequencing
    /// can both fail in the same hour, and keying them on a single pseudo-domain
    /// would silently keep whichever ran last. `frame_findings` is also how
    /// these reach [`crate::frame_refusal::FrameRefusalFinding`], which exists
    /// precisely for a refusal that belongs to no domain.
    pub fn record_frame_finding(&mut self, outcome: DomainOutcome) {
        self.frame_findings.push(outcome);
    }

    pub fn frame_findings(&self) -> &[DomainOutcome] {
        &self.frame_findings
    }

    /// Record an outcome for a domain.
    ///
    /// A later FAILURE replaces an earlier outcome: a domain's emission site is
    /// downstream of its read, and a site that refines its diagnosis (a decode
    /// that then failed to prove exhaustiveness) is stating the final answer.
    ///
    /// A later SUCCESS does NOT replace an earlier failure, and that asymmetry
    /// is load-bearing rather than defensive. One domain is read by a worker
    /// and then encoded by a separate site: when the worker panics and the
    /// encoder subsequently succeeds on the fallback read, plain last-write-
    /// wins published "this domain succeeded" over the panic that is the whole
    /// story. The offender then reached the record `Declared`, with a success
    /// predicate and an empty reason -- which reads as ANSWERED and is strictly
    /// worse than the `undeclared` it replaced. A check that passed later never
    /// unmakes a check that failed earlier in the same frame.
    pub fn record(&mut self, outcome: DomainOutcome) {
        match self
            .entries
            .iter_mut()
            .find(|entry| entry.domain_id == outcome.domain_id)
        {
            Some(slot) => {
                if outcome.is_failure() || !slot.is_failure() {
                    *slot = outcome;
                }
            }
            None => self.entries.push(outcome),
        }
    }

    pub fn get(&self, domain_id: u16) -> Option<&DomainOutcome> {
        self.entries
            .iter()
            .find(|entry| entry.domain_id == domain_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &DomainOutcome> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.frame_findings.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.frame_findings.clear();
    }

    /// Domains that recorded an outcome this frame, whether it failed or not.
    pub fn declared_domain_ids(&self) -> Vec<u16> {
        let mut ids: Vec<u16> = self.entries.iter().map(|entry| entry.domain_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

// ---------------------------------------------------------------------------
// The boundary between migrated and unmigrated producers
// ---------------------------------------------------------------------------

/// Where a domain sits with respect to this migration.
///
/// The contract fails if 30 domains carry outcomes and 43 bypass the machinery
/// invisibly, so "not migrated" is a NAMED state with its own reserved
/// predicate rather than an absence, and "migrated but produced no outcome" is
/// a third state distinct from both. A single pass over a capture counts all
/// three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeCoverage {
    /// The site recorded an outcome. The refusal names a real predicate.
    Declared,
    /// The site is not migrated. The refusal names
    /// [`crate::diagnostic_channel::PREDICATE_UNDECLARED`] — a statement, not a
    /// silence.
    NotMigrated,
    /// The site IS migrated and still published nothing for a failing domain.
    /// That is a regression in the producer, not a property of the domain, and
    /// it gets its own loud finding rather than falling back to `NotMigrated`
    /// where it would be indistinguishable from work never done.
    MigratedButSilent,
}

/// Classify one offending domain against the ledger and the declared set.
/// Classify one OFFENDING domain against the ledger and the declared set.
///
/// Only a FAILING outcome counts as `Declared`, and that is the load-bearing
/// part. A site records success too -- that is what makes a bypass detectable
/// at all -- but a success is not a diagnosis of a failure. A domain whose
/// worker completed and which then failed somewhere that has NOT been migrated
/// would otherwise reach the record `Declared`, carrying the predicate of a
/// check that PASSED and an empty reason, which reads as answered and is
/// strictly worse than the `undeclared` it replaced. It is
/// `MigratedButSilent`: something failed this domain and did not say so.
pub fn classify_coverage(
    domain_id: u16,
    ledger: &DomainOutcomeLedger,
    migrated_domains: &[u16],
) -> OutcomeCoverage {
    match ledger.get(domain_id) {
        Some(outcome) if outcome.is_failure() => OutcomeCoverage::Declared,
        Some(_) | None => {
            if migrated_domains.contains(&domain_id) {
                OutcomeCoverage::MigratedButSilent
            } else {
                OutcomeCoverage::NotMigrated
            }
        }
    }
}

/// The outcome that DIAGNOSES a failure for this domain, if the producer
/// recorded one. A success is not a diagnosis; see [`classify_coverage`].
pub fn failure_diagnosis(ledger: &DomainOutcomeLedger, domain_id: u16) -> Option<&DomainOutcome> {
    ledger.get(domain_id).filter(|outcome| outcome.is_failure())
}

/// Predicate code for the `MigratedButSilent` finding.
pub const READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME: u32 = 0x0002_0009;
pub const READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME_NAME: &str =
    "migrated_site_published_outcome";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_read_predicate_code_is_non_zero_and_unique() {
        // Zero is reserved for "not declared" and `Predicate::new` refuses it,
        // so a zero in this table is a predicate that cannot reach the wire at
        // all while looking, in source, exactly like one that can.
        let mut seen: Vec<u32> = Vec::new();
        for (code, name) in READ_PREDICATES {
            assert_ne!(
                *code, 0,
                "read predicate {name} has code 0, which is reserved for 'not declared'"
            );
            assert!(
                !seen.contains(code),
                "read predicate code {code:#010x} ({name}) is used twice"
            );
            seen.push(*code);
            assert!(
                Predicate::new(*code, *name).is_ok(),
                "read predicate {name} cannot be constructed"
            );
        }
        assert_eq!(
            seen.len(),
            READ_PREDICATES.len(),
            "READ_PREDICATES lost an entry to deduplication"
        );
    }

    #[test]
    fn read_predicate_codes_are_in_the_reserved_domain_range() {
        // The range is the ABI's namespace split: 0x0001_xxxx is the frame's,
        // 0xFFFF_FF01 is reserved. A domain predicate outside 0x0002_xxxx would
        // collide with one of those as the tables grow.
        for (code, name) in READ_PREDICATES {
            assert_eq!(
                code >> 16,
                0x0002,
                "read predicate {name} ({code:#010x}) is outside the 0x0002_xxxx range"
            );
        }
        assert_eq!(
            READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME >> 16,
            0x0002,
            "the migrated-but-silent predicate is outside the 0x0002_xxxx range"
        );
    }

    #[test]
    fn read_predicate_refuses_a_code_that_is_not_declared() {
        assert!(read_predicate(0x0002_FFFF).is_err());
        assert!(read_predicate(0).is_err());
        assert!(read_predicate(READ_PREDICATE_WORKER_COMPLETED).is_ok());
    }

    fn outcome(completion: ReadCompletion, extent: AnswerExtent) -> DomainOutcome {
        DomainOutcome::failed(
            7,
            read_predicate(READ_PREDICATE_PAYLOAD_ENCODED).unwrap(),
            completion,
            extent,
            "reason",
        )
    }

    #[test]
    fn derived_bits_reproduce_the_pre_migration_semantics_exactly() {
        // This table IS the compatibility statement. Each row is a shape that
        // existed as a hand-written `status |=` before the migration, and the
        // right-hand side is the word that site produced. A consumer must not
        // be able to tell that the producer changed.
        let cases: &[(ReadCompletion, AnswerExtent, u32)] = &[
            (ReadCompletion::Completed, AnswerExtent::Whole, 0),
            (
                ReadCompletion::Completed,
                AnswerExtent::Incomplete,
                COMPACT_STATUS_PARTIAL_DATA,
            ),
            (
                ReadCompletion::Completed,
                AnswerExtent::Unprovable,
                COMPACT_STATUS_PARTIAL_DATA,
            ),
            (
                ReadCompletion::DidNotComplete,
                AnswerExtent::Whole,
                COMPACT_STATUS_SOURCE_UNAVAILABLE,
            ),
            (
                ReadCompletion::DidNotComplete,
                AnswerExtent::Incomplete,
                COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
            ),
            (
                ReadCompletion::DidNotComplete,
                AnswerExtent::Unprovable,
                COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA,
            ),
            (
                ReadCompletion::WorkerPanicked,
                AnswerExtent::Whole,
                COMPACT_STATUS_WORKER_PANIC,
            ),
        ];
        for (completion, extent, expected) in cases {
            let derived = outcome(*completion, *extent).derive_status(0);
            assert_eq!(
                derived.bits(),
                *expected,
                "{}/{} derived {:#010x}, expected {expected:#010x}",
                completion.label(),
                extent.label(),
                derived.bits()
            );
        }
    }

    #[test]
    fn source_unavailable_is_not_derived_for_a_merely_partial_answer() {
        // 0x100 means "the read did not complete". A read that completed and
        // published part of the answer is 0x200 and NOT 0x100; conflating them
        // is the single most consequential way this derivation could be wrong,
        // because a consumer counts 0x100 as a lost hour.
        let derived = outcome(ReadCompletion::Completed, AnswerExtent::Incomplete).derive_status(0);
        assert_eq!(derived.bits() & COMPACT_STATUS_SOURCE_UNAVAILABLE, 0);
        assert_eq!(
            derived.bits() & COMPACT_STATUS_PARTIAL_DATA,
            COMPACT_STATUS_PARTIAL_DATA
        );
    }

    #[test]
    fn partial_data_is_not_derived_for_a_read_that_merely_did_not_complete() {
        let derived = outcome(ReadCompletion::DidNotComplete, AnswerExtent::Whole).derive_status(0);
        assert_eq!(derived.bits() & COMPACT_STATUS_PARTIAL_DATA, 0);
        assert_eq!(
            derived.bits() & COMPACT_STATUS_SOURCE_UNAVAILABLE,
            COMPACT_STATUS_SOURCE_UNAVAILABLE
        );
    }

    #[test]
    fn incomplete_and_unprovable_share_a_bit_and_stay_distinguishable() {
        // Same bit downstream, different fact in the record. Losing rows inside
        // a reached extent is a defect in the reader; being unable to prove
        // exhaustiveness is a defect in the construction, and the roster family
        // needs to tell them apart.
        let incomplete = outcome(ReadCompletion::Completed, AnswerExtent::Incomplete);
        let unprovable = outcome(ReadCompletion::Completed, AnswerExtent::Unprovable);
        assert_eq!(
            incomplete.derive_status(0).bits(),
            unprovable.derive_status(0).bits()
        );
        assert_ne!(incomplete.extent, unprovable.extent);
        assert_ne!(incomplete.extent.code(), unprovable.extent.code());
    }

    #[test]
    fn the_frame_scoped_base_survives_the_derivation() {
        let base = COMPACT_STATUS_CONTROL_INVALID | COMPACT_STATUS_NO_CLOCK;
        let derived =
            outcome(ReadCompletion::DidNotComplete, AnswerExtent::Whole).derive_status(base);
        assert_eq!(derived.bits() & base, base);
        assert_eq!(
            derived.added(base),
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            "the outcome added exactly the bit it is authoritative over"
        );
    }

    #[test]
    fn clock_and_authority_facts_derive_their_own_bits() {
        let unreadable = outcome(ReadCompletion::DidNotComplete, AnswerExtent::Whole)
            .with_clock(ClockPresence::Unreadable);
        assert_eq!(
            unreadable.derive_status(0).bits(),
            COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_NO_CLOCK
        );
        let invalid = outcome(ReadCompletion::DidNotComplete, AnswerExtent::Incomplete)
            .with_read_authority(ReadAuthority::Invalid);
        assert_eq!(
            invalid.derive_status(0).bits(),
            COMPACT_STATUS_SOURCE_UNAVAILABLE
                | COMPACT_STATUS_PARTIAL_DATA
                | COMPACT_STATUS_CONTROL_INVALID
        );
    }

    #[test]
    fn a_derivation_never_reaches_outside_its_own_mask() {
        // The outcome speaks for five bits. If it ever set a sixth, a migrated
        // site would be making a claim about a fact it does not carry.
        for completion in [
            ReadCompletion::Completed,
            ReadCompletion::DidNotComplete,
            ReadCompletion::WorkerPanicked,
        ] {
            for extent in [
                AnswerExtent::Whole,
                AnswerExtent::Incomplete,
                AnswerExtent::Unprovable,
            ] {
                let bits = outcome(completion, extent)
                    .with_clock(ClockPresence::Unreadable)
                    .with_read_authority(ReadAuthority::Invalid)
                    .derive_status(0)
                    .bits();
                assert_eq!(
                    bits & !DERIVED_STATUS_MASK,
                    0,
                    "{}/{} derived a bit outside DERIVED_STATUS_MASK",
                    completion.label(),
                    extent.label()
                );
            }
        }
    }

    #[test]
    fn a_successful_outcome_is_not_a_failure_and_adds_no_bit() {
        let ok =
            DomainOutcome::succeeded(3, read_predicate(READ_PREDICATE_WORKER_COMPLETED).unwrap());
        assert!(!ok.is_failure());
        assert_eq!(ok.derive_status(0).bits(), 0);
        assert!(outcome(ReadCompletion::DidNotComplete, AnswerExtent::Whole).is_failure());
        assert!(outcome(ReadCompletion::Completed, AnswerExtent::Incomplete).is_failure());
    }

    #[test]
    fn the_ledger_keeps_one_outcome_per_domain_and_the_last_one_wins() {
        let mut ledger = DomainOutcomeLedger::new();
        ledger.record(DomainOutcome::succeeded(
            4,
            read_predicate(READ_PREDICATE_WORKER_COMPLETED).unwrap(),
        ));
        ledger.record(DomainOutcome::failed(
            4,
            read_predicate(READ_PREDICATE_PAYLOAD_ENCODED).unwrap(),
            ReadCompletion::DidNotComplete,
            AnswerExtent::Whole,
            "encoder refused",
        ));
        assert_eq!(ledger.len(), 1);
        let entry = ledger.get(4).expect("domain 4 has an outcome");
        assert_eq!(entry.reason, "encoder refused");
        assert!(entry.is_failure());
        assert_eq!(ledger.declared_domain_ids(), vec![4]);
    }

    #[test]
    fn a_later_success_never_erases_an_earlier_failure_for_the_same_domain() {
        // The measured defect: D69 and D70 are read by a worker and encoded by
        // a separate site. When the worker panicked and the encoder then
        // succeeded on the fallback read, plain last-write-wins published "this
        // domain succeeded" over the panic that is the entire explanation, and
        // the offender reached the record `Declared` with a success predicate
        // and an empty reason -- which reads as ANSWERED and is strictly worse
        // than the `undeclared` it replaced.
        //
        // Production change that makes this fail: restoring unconditional
        // last-write-wins in `record`.
        let mut ledger = DomainOutcomeLedger::new();
        ledger.record(DomainOutcome::failed(
            69,
            read_predicate(READ_PREDICATE_WORKER_COMPLETED).unwrap(),
            ReadCompletion::WorkerPanicked,
            AnswerExtent::Whole,
            "reader worker panicked: tech-sharing roster pointer was null",
        ));
        ledger.record(DomainOutcome::succeeded(
            69,
            read_predicate(READ_PREDICATE_PAYLOAD_ENCODED).unwrap(),
        ));
        let entry = ledger.get(69).expect("domain 69 has an outcome");
        assert!(entry.is_failure(), "a success erased the panic that explains it");
        assert!(entry.reason.contains("tech-sharing roster pointer was null"));
        assert_eq!(entry.completion, ReadCompletion::WorkerPanicked);
    }

    #[test]
    fn a_later_failure_still_refines_an_earlier_one() {
        // The asymmetry must not become "first write wins": a site that
        // discovers a WORSE fact downstream is stating the final answer, and
        // freezing the first would publish a diagnosis the record contradicts.
        let mut ledger = DomainOutcomeLedger::new();
        ledger.record(DomainOutcome::failed(
            90,
            read_predicate(READ_PREDICATE_PAYLOAD_DECODED_EXACT).unwrap(),
            ReadCompletion::Completed,
            AnswerExtent::Incomplete,
            "first",
        ));
        ledger.record(DomainOutcome::failed(
            90,
            read_predicate(READ_PREDICATE_PAYLOAD_DECODED_EXACT).unwrap(),
            ReadCompletion::DidNotComplete,
            AnswerExtent::Unprovable,
            "second",
        ));
        let entry = ledger.get(90).expect("domain 90 has an outcome");
        assert_eq!(entry.reason, "second");
        assert_eq!(entry.extent, AnswerExtent::Unprovable);
    }

    #[test]
    fn a_success_still_records_when_nothing_failed_first() {
        // Success recording is what makes `MigratedButSilent` detectable at
        // all; the asymmetry must not suppress it on the clean path.
        let mut ledger = DomainOutcomeLedger::new();
        ledger.record(DomainOutcome::succeeded(
            20,
            read_predicate(READ_PREDICATE_WORKER_COMPLETED).unwrap(),
        ));
        ledger.record(DomainOutcome::succeeded(
            20,
            read_predicate(READ_PREDICATE_PAYLOAD_ENCODED).unwrap(),
        ));
        let entry = ledger.get(20).expect("domain 20 has an outcome");
        assert!(!entry.is_failure());
        assert_eq!(
            entry.predicate.code.get(),
            READ_PREDICATE_PAYLOAD_ENCODED,
            "a later clean check is still the latest word when nothing failed"
        );
    }

    #[test]
    fn coverage_separates_not_migrated_from_migrated_but_silent() {
        // The whole point of the declared set. Without it a migrated site that
        // regressed to discarding its reason would fall back to the SAME
        // `undeclared` predicate as a domain nobody has touched yet, and the
        // regression would be invisible in exactly the measurement built to
        // find it.
        let mut ledger = DomainOutcomeLedger::new();
        ledger.record(DomainOutcome::succeeded(
            10,
            read_predicate(READ_PREDICATE_WORKER_COMPLETED).unwrap(),
        ));
        let migrated = [10u16, 11u16, 13u16];
        // A SUCCESS is not a diagnosis. Domain 10's site ran and recorded that
        // its worker completed; if 10 is nonetheless an offender, something
        // else failed it and did not say so. Reporting that as `Declared` puts
        // the predicate of a check that PASSED on the record with no reason --
        // which reads as answered, and is strictly worse than `undeclared`.
        assert_eq!(
            classify_coverage(10, &ledger, &migrated),
            OutcomeCoverage::MigratedButSilent
        );
        assert!(failure_diagnosis(&ledger, 10).is_none());
        ledger.record(DomainOutcome::failed(
            13,
            read_predicate(READ_PREDICATE_PAYLOAD_ENCODED).unwrap(),
            ReadCompletion::DidNotComplete,
            AnswerExtent::Whole,
            "encoder refused",
        ));
        assert_eq!(
            classify_coverage(13, &ledger, &migrated),
            OutcomeCoverage::Declared
        );
        assert_eq!(
            failure_diagnosis(&ledger, 13).map(|outcome| outcome.reason.as_str()),
            Some("encoder refused")
        );
        assert_eq!(
            classify_coverage(11, &ledger, &migrated),
            OutcomeCoverage::MigratedButSilent
        );
        assert_eq!(
            classify_coverage(12, &ledger, &migrated),
            OutcomeCoverage::NotMigrated
        );
    }
}
