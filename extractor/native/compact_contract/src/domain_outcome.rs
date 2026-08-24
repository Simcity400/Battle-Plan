use crate::diagnostic_channel::{Authority, DiagUnit, LocatorLevel, Predicate};
use crate::status_flags_base::{
    COMPACT_STATUS_CONTROL_INVALID, COMPACT_STATUS_NO_CLOCK, COMPACT_STATUS_PARTIAL_DATA,
    COMPACT_STATUS_SOURCE_EMPTY, COMPACT_STATUS_SOURCE_UNAVAILABLE, COMPACT_STATUS_WORKER_PANIC,
};

// A predicate code is a DIAGNOSTIC ABI. Retained telemetry is read by build-

pub const READ_PREDICATE_WORKER_COMPLETED: u32 = 0x0002_0001;
pub const READ_PREDICATE_PAYLOAD_ENCODED: u32 = 0x0002_0002;
pub const READ_PREDICATE_PAYLOAD_DECODED_EXACT: u32 = 0x0002_0003;
pub const READ_PREDICATE_READER_RETURNED_ROWS: u32 = 0x0002_0004;
pub const READ_PREDICATE_READ_COUNTER_PROBE: u32 = 0x0002_0005;
pub const READ_PREDICATE_REJECTED_RECORDS_RESEQUENCED: u32 = 0x0002_0006;
pub const READ_PREDICATE_FRAME_HOUR_READABLE: u32 = 0x0002_0007;
pub const READ_PREDICATE_READ_AUTHORITY_VALID: u32 = 0x0002_0008;
pub const READ_PREDICATE_ATTRIBUTION_PROBE_APPENDED: u32 = 0x0002_000A;
pub const READ_PREDICATE_CAPTURED_FRAME_ADMISSIBLE: u32 = 0x0002_000B;
pub const READ_PREDICATE_HOOK_RESERVATION_CUSTODY_TAKEN: u32 = 0x0002_000C;
pub const READ_PREDICATE_PUBLISHED_PAYLOAD_CARRIES_ANSWER: u32 = 0x0002_000D;
pub const READ_PREDICATE_CAPTURED_PAYLOAD_OFFERED: u32 = 0x0002_000E;
pub const READ_PREDICATE_FRAME_STATUS_PERMITTED_READ: u32 = 0x0002_000F;
pub const READ_PREDICATE_SOURCE_ANSWERED_EMPTY: u32 = 0x0002_0010;
pub const READ_PREDICATE_OUTCOME_SUBSTANTIATED_FROM_RECORD: u32 = 0x0002_0011;

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
    (
        READ_PREDICATE_SOURCE_ANSWERED_EMPTY,
        "source_answered_empty",
    ),
    (
        READ_PREDICATE_OUTCOME_SUBSTANTIATED_FROM_RECORD,
        "outcome_substantiated_from_record",
    ),
];

pub fn read_predicate_name(code: u32) -> Option<&'static str> {
    READ_PREDICATES
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, name)| *name)
}

pub fn read_predicate(code: u32) -> Result<Predicate, String> {
    match read_predicate_name(code) {
        Some(name) => Predicate::new(code, name),
        None => Err(format!(
            "read predicate {code:#010x} is not declared in READ_PREDICATES"
        )),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ReadCompletion {
    Completed = 1,
    DidNotComplete = 2,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AnswerExtent {
    Whole = 1,
    Incomplete = 2,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum AnswerPresence {
    #[default]
    Present = 1,
    Empty = 2,
}

impl AnswerPresence {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Present => "present",
            Self::Empty => "empty",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClockPresence {
    Readable,
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadAuthority {
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DerivedStatus(u32);

impl DerivedStatus {
    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn added(self, base: u32) -> u32 {
        self.0 & !base
    }
}

pub const DERIVED_STATUS_MASK: u32 = COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_NO_CLOCK
    | COMPACT_STATUS_CONTROL_INVALID
    | COMPACT_STATUS_SOURCE_EMPTY;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainOutcome {
    pub domain_id: u16,
    pub predicate: Predicate,
    pub authority: Authority,
    pub locator: Vec<LocatorLevel>,
    pub units: Vec<DiagUnit>,
    pub reason: String,
    pub completion: ReadCompletion,
    pub extent: AnswerExtent,
    pub presence: AnswerPresence,
    pub clock: ClockPresence,
    pub authority_state: ReadAuthority,
    pub derived_status: Option<u32>,
}

impl DomainOutcome {
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
            presence: AnswerPresence::Present,
            clock: ClockPresence::Readable,
            authority_state: ReadAuthority::Valid,
            derived_status: None,
        }
    }

    pub fn measured_empty(domain_id: u16, predicate: Predicate, reason: impl Into<String>) -> Self {
        Self {
            domain_id,
            predicate,
            authority: Authority::SelfDomain,
            locator: Vec::new(),
            units: Vec::new(),
            reason: reason.into(),
            completion: ReadCompletion::Completed,
            extent: AnswerExtent::Whole,
            presence: AnswerPresence::Empty,
            clock: ClockPresence::Readable,
            authority_state: ReadAuthority::Valid,
            derived_status: None,
        }
    }

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
            presence: AnswerPresence::Present,
            clock: ClockPresence::Readable,
            authority_state: ReadAuthority::Valid,
            derived_status: None,
        }
    }

    pub fn with_derived_status(mut self, derived: DerivedStatus) -> Self {
        self.derived_status = Some(derived.bits());
        self
    }

    pub fn own_bits(&self) -> u32 {
        self.derive_status(0).bits()
    }

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
        if matches!(self.presence, AnswerPresence::Empty)
            && matches!(self.completion, ReadCompletion::Completed)
            && matches!(self.extent, AnswerExtent::Whole)
        {
            bits |= COMPACT_STATUS_SOURCE_EMPTY;
        }
        if matches!(self.clock, ClockPresence::Unreadable) {
            bits |= COMPACT_STATUS_NO_CLOCK;
        }
        if matches!(self.authority_state, ReadAuthority::Invalid) {
            bits |= COMPACT_STATUS_CONTROL_INVALID;
        }
        DerivedStatus(bits)
    }

    pub fn render(&self) -> String {
        format!(
            "d{} {}#{} {} {}/{}/{} reason={}",
            self.domain_id,
            self.predicate.name,
            self.predicate.code.get(),
            self.authority.render(),
            self.completion.label(),
            self.extent.label(),
            self.presence.label(),
            self.reason
        )
    }
}

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

    pub fn record_frame_finding(&mut self, outcome: DomainOutcome) {
        self.frame_findings.push(outcome);
    }

    pub fn frame_findings(&self) -> &[DomainOutcome] {
        &self.frame_findings
    }

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

    /// Replace one domain's entry unconditionally -- including a success over
    /// an existing failure, which `record` deliberately refuses.
    ///
    /// `record`'s rule is right for READS: several sites can speak for one
    /// domain in a frame, and a later success must not erase an earlier
    /// failure. A terminal hook-custody merge is not another read of the same
    /// thing. It rolls the hourly reservation back, re-seals the lane over an
    /// interval that CONTAINS the hourly one, and REPLACES the domain's DATA
    /// and PERF records in the frame with that drain -- so the hourly outcome
    /// no longer describes anything the frame carries. Leaving it in place
    /// ships a clean PERF word beside a failing diagnosis, which is the
    /// disagreement `audit_published_status` exists to catch and which nothing
    /// catches in that direction.
    ///
    /// Callers must own the record they are superseding. The one caller is the
    /// D90 terminal custody merge, after `replace_terminal_hook_domain_records`
    /// has actually succeeded.
    pub fn record_superseding(&mut self, outcome: DomainOutcome) {
        match self
            .entries
            .iter_mut()
            .find(|entry| entry.domain_id == outcome.domain_id)
        {
            Some(slot) => *slot = outcome,
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

    pub fn declared_domain_ids(&self) -> Vec<u16> {
        let mut ids: Vec<u16> = self.entries.iter().map(|entry| entry.domain_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeCoverage {
    Declared,
    NotMigrated,
    MigratedButSilent,
}

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

pub fn failure_diagnosis(ledger: &DomainOutcomeLedger, domain_id: u16) -> Option<&DomainOutcome> {
    ledger.get(domain_id).filter(|outcome| outcome.is_failure())
}

pub const READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME: u32 = 0x0002_0009;
pub const READ_PREDICATE_MIGRATED_SITE_PUBLISHED_OUTCOME_NAME: &str =
    "migrated_site_published_outcome";

#[cfg(test)]
mod tests {
    use super::*;

    /// Law: `record_superseding` REPLACES, including a success over a failure,
    /// and `record` still refuses that direction.
    ///
    /// The two rules are opposites on purpose. Within one frame's reads a later
    /// success must not erase an earlier failure for the same domain, which is
    /// `record`. A terminal hook-custody merge is not another read: it replaces
    /// the domain's DATA and PERF records in the frame with a drain that
    /// supersedes the hourly one, so its entry has to move with them or the
    /// frame ships a clean status word beside a failing diagnosis.
    ///
    /// What production change makes this fail: `record_superseding` delegating
    /// to `record`, which would silently keep the obsolete failure -- the exact
    /// regression a source pin on the CALLER cannot see.
    #[test]
    fn a_superseding_success_replaces_a_failure_and_an_ordinary_one_does_not() {
        let predicate = read_predicate(READ_PREDICATE_PAYLOAD_DECODED_EXACT)
            .expect("the D90 exactness predicate is declared");
        // `Predicate` is Clone, not Copy: every use below takes its own.
        let failure = || {
            DomainOutcome::failed(
                90,
                predicate.clone(),
                ReadCompletion::DidNotComplete,
                AnswerExtent::Unprovable,
                "the hourly peek could not prove its interval",
            )
        };
        let success = || DomainOutcome::succeeded(90, predicate.clone());

        // `record`: a later success leaves the failure standing.
        let mut ledger = DomainOutcomeLedger::new();
        ledger.record(failure());
        assert!(ledger.get(90).expect("recorded").is_failure());
        ledger.record(success());
        assert!(
            ledger.get(90).expect("still recorded").is_failure(),
            "an ordinary success must not erase an earlier failure for the same              domain in the same frame"
        );

        // `record_superseding`: the entry moves with the records it describes.
        ledger.record_superseding(success());
        let superseded = ledger.get(90).expect("still recorded");
        assert!(
            !superseded.is_failure(),
            "the terminal drain replaced the frame's D90 records, so its entry              must replace theirs"
        );
        assert_eq!(superseded.completion, ReadCompletion::Completed);
        assert_eq!(superseded.extent, AnswerExtent::Whole);
        assert_eq!(
            ledger.len(),
            1,
            "superseding must replace the domain's entry, not add a second one"
        );

        // And it inserts when there is nothing to supersede.
        let mut empty = DomainOutcomeLedger::new();
        empty.record_superseding(success());
        assert_eq!(empty.len(), 1);
        assert!(!empty.get(90).expect("inserted").is_failure());

        // It speaks for ONE domain: a neighbour's failure is untouched.
        let mut two = DomainOutcomeLedger::new();
        two.record(failure());
        let mut other = failure();
        other.domain_id = 86;
        two.record(other);
        two.record_superseding(success());
        assert!(!two.get(90).expect("superseded").is_failure());
        assert!(
            two.get(86).expect("neighbour").is_failure(),
            "superseding one domain must not touch another's entry"
        );
    }

    #[test]
    fn every_read_predicate_code_is_non_zero_and_unique() {
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
    fn a_measured_empty_read_derives_empty_and_never_unavailable() {
        let empty = DomainOutcome::measured_empty(
            36,
            read_predicate(READ_PREDICATE_SOURCE_ANSWERED_EMPTY).unwrap(),
            "the authenticated air-variant container held no variants",
        );
        let bits = empty.derive_status(0).bits();
        assert_eq!(bits, COMPACT_STATUS_SOURCE_EMPTY);
        assert_eq!(bits & COMPACT_STATUS_SOURCE_UNAVAILABLE, 0);
        assert_eq!(bits & COMPACT_STATUS_PARTIAL_DATA, 0);
        assert_eq!(bits & COMPACT_STATUS_WORKER_PANIC, 0);
        assert!(
            !empty.is_failure(),
            "a measured zero is an answer, not a failure"
        );
        assert_eq!(
            bits & crate::COMPACT_STATUS_FAILURE_MASK,
            0,
            "SOURCE_EMPTY must not reach any consumer's failure classification"
        );
    }

    #[test]
    fn a_read_that_did_not_complete_can_never_be_published_as_empty() {
        for (completion, extent) in [
            (ReadCompletion::DidNotComplete, AnswerExtent::Whole),
            (ReadCompletion::WorkerPanicked, AnswerExtent::Whole),
            (ReadCompletion::Completed, AnswerExtent::Incomplete),
            (ReadCompletion::Completed, AnswerExtent::Unprovable),
            (ReadCompletion::DidNotComplete, AnswerExtent::Unprovable),
        ] {
            let mut forged = outcome(completion, extent);
            forged.presence = AnswerPresence::Empty;
            let bits = forged.derive_status(0).bits();
            assert_eq!(
                bits & COMPACT_STATUS_SOURCE_EMPTY,
                0,
                "{}/{} derived a measured-zero badge it cannot substantiate",
                completion.label(),
                extent.label()
            );
        }
    }

    #[test]
    fn every_failure_constructor_leaves_presence_alone() {
        let failed = DomainOutcome::failed(
            5,
            read_predicate(READ_PREDICATE_PAYLOAD_ENCODED).unwrap(),
            ReadCompletion::DidNotComplete,
            AnswerExtent::Whole,
            "encoder refused",
        );
        assert_eq!(failed.presence, AnswerPresence::Present);
        assert_eq!(
            failed.derive_status(0).bits(),
            COMPACT_STATUS_SOURCE_UNAVAILABLE
        );
        let ok =
            DomainOutcome::succeeded(5, read_predicate(READ_PREDICATE_WORKER_COMPLETED).unwrap());
        assert_eq!(ok.presence, AnswerPresence::Present);
        assert_eq!(ok.derive_status(0).bits(), 0);
    }

    #[test]
    fn a_measured_empty_outcome_never_erases_an_earlier_failure() {
        let mut ledger = DomainOutcomeLedger::new();
        ledger.record(DomainOutcome::failed(
            36,
            read_predicate(READ_PREDICATE_WORKER_COMPLETED).unwrap(),
            ReadCompletion::WorkerPanicked,
            AnswerExtent::Whole,
            "air variant worker panicked",
        ));
        ledger.record(DomainOutcome::measured_empty(
            36,
            read_predicate(READ_PREDICATE_SOURCE_ANSWERED_EMPTY).unwrap(),
            "container held no variants",
        ));
        let entry = ledger.get(36).expect("domain 36 has an outcome");
        assert_eq!(entry.completion, ReadCompletion::WorkerPanicked);
        assert!(entry.is_failure());
    }

    #[test]
    fn a_derivation_never_reaches_outside_its_own_mask() {
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
                for presence in [AnswerPresence::Present, AnswerPresence::Empty] {
                    let mut candidate = outcome(completion, extent)
                        .with_clock(ClockPresence::Unreadable)
                        .with_read_authority(ReadAuthority::Invalid);
                    candidate.presence = presence;
                    let bits = candidate.derive_status(0).bits();
                    assert_eq!(
                        bits & !DERIVED_STATUS_MASK,
                        0,
                        "{}/{}/{} derived a bit outside DERIVED_STATUS_MASK",
                        completion.label(),
                        extent.label(),
                        presence.label()
                    );
                }
            }
        }

        assert_eq!(
            DERIVED_STATUS_MASK.count_ones(),
            6,
            "DERIVED_STATUS_MASK gained or lost a bit; an outcome speaks for \
             exactly the facts it carries"
        );
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
        assert!(
            entry.is_failure(),
            "a success erased the panic that explains it"
        );
        assert!(entry
            .reason
            .contains("tech-sharing roster pointer was null"));
        assert_eq!(entry.completion, ReadCompletion::WorkerPanicked);
    }

    #[test]
    fn a_later_failure_still_refines_an_earlier_one() {
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
        let mut ledger = DomainOutcomeLedger::new();
        ledger.record(DomainOutcome::succeeded(
            10,
            read_predicate(READ_PREDICATE_WORKER_COMPLETED).unwrap(),
        ));
        let migrated = [10u16, 11u16, 13u16];
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
